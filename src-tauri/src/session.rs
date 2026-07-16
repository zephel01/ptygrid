// Session management (Phase 1): session slots keyed by a stable u32 id,
// same-id restart with generation-based stale-reader suppression,
// autorestart policies, and session-state event emission per CONTRACT.md.
//
// Locking rules (concurrency-bug fixes, dogfood review):
// - The global sessions map lock is only ever held for short, non-blocking
//   critical sections. Blocking operations (PTY writes, child.wait()) happen
//   OUTSIDE it.
// - Every state write performed by a detached/delayed thread is guarded by a
//   generation check taken under the same lock as the write.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::config::{self, AgentDef, AutoRestart};
use crate::pty;
use crate::worktree::{self, WorktreeInfo};

/// Give up after this many consecutive automatic restarts.
const MAX_AUTORESTARTS: u32 = 5;
/// Delay before an automatic restart.
const AUTORESTART_DELAY: Duration = Duration::from_secs(1);
/// A process that stayed up at least this long resets the consecutive
/// autorestart counter (this is what makes the 5-restart cap "consecutive").
const STABLE_RUN: Duration = Duration::from_secs(10);
/// Per-session output ring buffer cap (drop oldest beyond this).
const OUTPUT_CAP: usize = 256 * 1024;

// ---------- wire types (contract) ----------

/// Payload for the `pty-output` event.
#[derive(Clone, Serialize)]
struct OutputPayload {
    id: u32,
    data: String,
}

/// Payload for the `pty-exit` event.
#[derive(Clone, Serialize)]
struct ExitPayload {
    id: u32,
    code: Option<i32>,
}

/// `"starting" | "running" | "exited" | "restarting"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Starting,
    Running,
    Exited,
    Restarting,
}

/// Session flavor (Phase 4.1). `pty` is an ordinary PTY-backed session; a
/// `transcript` session is a PTY-less logical session that tails a read-only
/// teammate/subagent transcript. Existing sessions are always `pty` on the
/// wire, so this is an additive field that does not break any prior consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Pty,
    Transcript,
}

/// Teammate metadata carried on a teammate session's `SessionInfo`. Present on
/// `transcript` sessions (mode `"observe"`, Phase 4.1) and on host-teammate
/// `pty` sessions (mode `"host"`, Phase 4.2).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeammateInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub lead_id: u32,
    pub mode: &'static str,
}

/// Payload for `session-state` and item type of `list_sessions`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub cmd: String,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    /// Foreground process name on the PTY (Phase 2.1). Computed LAZILY in
    /// list_sessions / resolve_agent only — the fg process changes
    /// constantly, so caching at spawn would be wrong, and recomputing on
    /// every hot-path state emit would be waste: `session-state` events
    /// deliberately carry None (field omitted). Failure to resolve -> None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeInfo>,
    /// Phase 4.1: `pty` (default) or `transcript`.
    pub kind: SessionKind,
    /// Phase 4.1: present only on `transcript` sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teammate: Option<TeammateInfo>,
}

// ---------- internal types ----------

/// Everything needed to (re)spawn a session. Captured at spawn time so that
/// restart_session relaunches agents with their definition and adhoc shells
/// with the same cmd/cwd.
#[derive(Clone)]
struct LaunchSpec {
    name: Option<String>,
    /// Display command; also what gets executed (via shell when shell_wrap).
    cmd: String,
    /// Extra argv passed to `cmd` when `!shell_wrap` (host teammate spawns use
    /// pre-split argv from the pane-backend protocol). Empty for shell-wrapped
    /// definitions and single-program shells.
    args: Vec<String>,
    /// true: run via `/bin/sh -c` (Windows: `powershell -Command`).
    shell_wrap: bool,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    autorestart: AutoRestart,
    worktree: Option<WorktreeInfo>,
}

/// The live PTY half of a slot; replaced wholesale on restart.
struct LivePty {
    master: Box<dyn MasterPty + Send>,
    /// Shared per-session writer: write_pty clones this Arc under the map
    /// lock, then performs the (potentially blocking) write under this
    /// per-session lock ONLY — never under the global map lock (fix #1).
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn Child + Send + Sync>,
}

/// Internal teammate metadata for a teammate slot (transcript or host PTY).
/// `agent_id` lets `SubagentStop` locate a transcript slot again; it is not
/// serialized on the wire. `mode` is `"observe"` for transcript slots and
/// `"host"` for host-teammate PTY slots.
#[derive(Clone)]
struct TeammateSlotMeta {
    agent_id: String,
    role: Option<String>,
    lead_id: u32,
    mode: &'static str,
}

/// A session slot. The id is stable across restarts; only the contents
/// (live PTY + generation) are swapped.
struct SessionSlot {
    spec: LaunchSpec,
    /// Current generation. Reader threads carry the generation they were
    /// spawned with and go silent when the slot has moved on.
    generation: u64,
    state: SessionState,
    code: Option<i32>,
    restart_count: u32,
    /// Set by kill_pty; suppresses autorestart.
    manual_kill: bool,
    live: Option<LivePty>,
    cols: u16,
    rows: u16,
    spawned_at: Instant,
    /// Rolling output buffer (cap OUTPUT_CAP, oldest bytes dropped).
    /// Deliberately NOT cleared on restart: it spans generations.
    output: Vec<u8>,
    /// Phase 4.1: `Pty` for ordinary sessions, `Transcript` for PTY-less
    /// read-only teammate transcript sessions.
    kind: SessionKind,
    /// Phase 4.1: teammate metadata, present only on transcript slots.
    teammate: Option<TeammateSlotMeta>,
}

/// Append `chunk` to `buf`, dropping the oldest bytes beyond `cap`.
fn append_capped(buf: &mut Vec<u8>, chunk: &[u8], cap: usize) {
    if chunk.len() >= cap {
        buf.clear();
        buf.extend_from_slice(&chunk[chunk.len() - cap..]);
        return;
    }
    buf.extend_from_slice(chunk);
    if buf.len() > cap {
        let overflow = buf.len() - cap;
        buf.drain(..overflow);
    }
}

/// Decode a freshly-read PTY chunk into an emittable UTF-8 string, carrying an
/// incomplete trailing multibyte sequence (at most 3 bytes) across calls so a
/// codepoint split at an 8192-byte read boundary is not mangled into U+FFFD.
/// Genuinely invalid bytes mid-stream are flushed lossily so `carry` can never
/// grow without bound. The session ring buffer stays byte-based; only the
/// `pty-output` emit stream uses this (M1).
fn decode_stream_chunk(carry: &mut Vec<u8>, chunk: &[u8]) -> String {
    carry.extend_from_slice(chunk);
    let keep_from = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        // `error_len() == None` means the invalid bytes are an incomplete
        // trailing sequence at end of input: emit the valid prefix, keep the
        // tail (<=3 bytes) for the next read.
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        // A genuine invalid byte not at the tail: flush everything lossily so
        // we never stall / accumulate on real garbage.
        Err(_) => carry.len(),
    };
    // `keep_from` is a char boundary, so this prefix is valid UTF-8 (lossy
    // performs no substitution) unless we deliberately flushed garbage above.
    let text = String::from_utf8_lossy(&carry[..keep_from]).into_owned();
    carry.drain(..keep_from);
    text
}

/// Whether a `kill()` error means the process has already exited (ESRCH). Such
/// a "failure" is benign: the reader thread still reaps and removes the slot,
/// so `kill_pty` should report success rather than a spurious error (L5).
fn is_process_gone(err: &std::io::Error) -> bool {
    // ESRCH is 3 on both Linux and macOS; fall back to the message so the
    // check survives platforms that surface it differently.
    err.raw_os_error() == Some(3)
        || err
            .to_string()
            .to_ascii_lowercase()
            .contains("no such process")
}

fn session_info(id: u32, slot: &SessionSlot) -> SessionInfo {
    SessionInfo {
        id,
        name: slot.spec.name.clone(),
        cmd: slot.spec.cmd.clone(),
        state: slot.state,
        code: slot.code,
        // Hot emit paths never resolve the fg process; see SessionInfo docs.
        foreground: None,
        worktree: slot.spec.worktree.clone(),
        kind: slot.kind,
        teammate: slot.teammate.as_ref().map(|m| TeammateInfo {
            role: m.role.clone(),
            lead_id: m.lead_id,
            mode: m.mode,
        }),
    }
}

/// Foreground process-group leader pid of a slot's PTY, if any.
/// portable-pty implements this as tcgetpgrp(master fd) — a non-blocking
/// ioctl, safe to call under the map lock. (unix only)
fn foreground_pid(slot: &SessionSlot) -> Option<i32> {
    #[cfg(unix)]
    {
        slot.live.as_ref()?.master.process_group_leader()
    }
    #[cfg(not(unix))]
    {
        None
    }
}

type SharedSessions = Arc<Mutex<HashMap<u32, SessionSlot>>>;

fn lock_map(map: &SharedSessions) -> MutexGuard<'_, HashMap<u32, SessionSlot>> {
    match map.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_writer(
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
) -> MutexGuard<'_, Box<dyn Write + Send>> {
    match writer.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Build the CommandBuilder for a spec. TERM is always set (a definition's
/// env may deliberately override it). QUEEN_URL is injected into every
/// session when the Queen MCP server is enabled.
fn command_for_spec(spec: &LaunchSpec, queen_url: Option<&str>) -> CommandBuilder {
    let mut cmd = if spec.shell_wrap {
        #[cfg(not(windows))]
        {
            let mut c = CommandBuilder::new("/bin/sh");
            c.arg("-c");
            c.arg(&spec.cmd);
            c
        }
        #[cfg(windows)]
        {
            let mut c = CommandBuilder::new("powershell.exe");
            c.arg("-Command");
            c.arg(&spec.cmd);
            c
        }
    } else {
        let mut c = CommandBuilder::new(&spec.cmd);
        for arg in &spec.args {
            c.arg(arg);
        }
        c
    };
    cmd.env("TERM", "xterm-256color");
    if let Some(url) = queen_url {
        cmd.env("QUEEN_URL", url);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &spec.cwd {
        cmd.cwd(cwd);
    }
    cmd
}

fn command_for_definition(def: &AgentDef, logical_resume: bool) -> &str {
    if logical_resume {
        def.resume.as_deref().unwrap_or(&def.cmd)
    } else {
        &def.cmd
    }
}

// ---------- pure exit/autorestart decision (unit-tested) ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EofAction {
    /// Autorestart; the slot's restart_count becomes new_count.
    Restart { new_count: u32 },
    /// Final exit; remove=true drops the slot (manual kill semantics).
    Exit { remove: bool },
}

/// Decide what happens when a session's PTY reaches EOF.
/// `stable_run` = the process stayed up at least STABLE_RUN, which resets the
/// consecutive-restart counter before the cap check.
fn decide_eof(
    policy: AutoRestart,
    manual_kill: bool,
    code: Option<i32>,
    restart_count: u32,
    stable_run: bool,
) -> EofAction {
    let count = if stable_run { 0 } else { restart_count };
    let wants_restart = match policy {
        AutoRestart::Never => false,
        AutoRestart::Always => true,
        AutoRestart::OnFailure => code != Some(0),
    };
    if !manual_kill && wants_restart && count < MAX_AUTORESTARTS {
        EofAction::Restart {
            new_count: count + 1,
        }
    } else {
        EofAction::Exit {
            remove: manual_kill,
        }
    }
}

// ---------- manager ----------

pub struct PtyManager {
    sessions: SharedSessions,
    next_id: AtomicU32,
    /// Global generation source; shared with detached threads.
    generations: Arc<AtomicU64>,
}

impl PtyManager {
    pub fn new() -> Self {
        PtyManager {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU32::new(1),
            generations: Arc::new(AtomicU64::new(1)),
        }
    }

    fn lock_sessions(&self) -> MutexGuard<'_, HashMap<u32, SessionSlot>> {
        lock_map(&self.sessions)
    }

    /// Phase 0 `spawn_shell` (+ optional cwd). cmd is exec'd directly
    /// (it is a shell/program path), not shell-wrapped.
    pub fn spawn_shell<R: Runtime>(
        &self,
        app: AppHandle<R>,
        cols: u16,
        rows: u16,
        cmd: Option<String>,
        cwd: Option<String>,
    ) -> Result<u32, String> {
        let spec = LaunchSpec {
            name: None,
            cmd: cmd.unwrap_or_else(pty::default_shell),
            args: Vec::new(),
            shell_wrap: false,
            cwd: cwd
                .map(PathBuf::from)
                .or_else(|| pty::home_dir().map(PathBuf::from)),
            env: Vec::new(),
            autorestart: AutoRestart::Never,
            worktree: None,
        };
        self.create_session(app, spec, cols, rows)
    }

    /// `spawn_agent`: launch a loaded agent/process definition.
    pub fn spawn_agent<R: Runtime>(
        &self,
        app: AppHandle<R>,
        def: &AgentDef,
        config_dir: &Path,
        cols: u16,
        rows: u16,
    ) -> Result<u32, String> {
        self.launch_agent(app, def, config_dir, (cols, rows), false, None)
    }

    /// Logical resume: resolve the current definition/env again and use its
    /// optional resume command. Persisted worktree metadata is validated by
    /// the worktree service before reuse.
    pub fn resume_agent<R: Runtime>(
        &self,
        app: AppHandle<R>,
        def: &AgentDef,
        config_dir: &Path,
        cols: u16,
        rows: u16,
        saved_worktree: Option<WorktreeInfo>,
    ) -> Result<u32, String> {
        self.launch_agent(app, def, config_dir, (cols, rows), true, saved_worktree)
    }

    fn launch_agent<R: Runtime>(
        &self,
        app: AppHandle<R>,
        def: &AgentDef,
        config_dir: &Path,
        size: (u16, u16),
        logical_resume: bool,
        saved_worktree: Option<WorktreeInfo>,
    ) -> Result<u32, String> {
        let (cols, rows) = size;
        let env = config::expanded_env(def);
        let prepared = if logical_resume {
            worktree::prepare_for_resume(&app, def, config_dir, &env, saved_worktree)?
        } else {
            worktree::prepare_for_agent(&app, def, config_dir, &env)?
        };
        let (cwd, worktree) = match prepared {
            Some(prepared) => (prepared.cwd, Some(prepared.info)),
            None => (config::resolve_cwd(config_dir, def.cwd.as_deref()), None),
        };
        // Reserve the id up front so a Phase 4.2 host lead can bind its per-lead
        // socket at teams/run/lead-<id>.sock and inject the matching env before
        // the PTY is spawned. Opt-in only: nothing runs unless `teams.mode:
        // host`. All host orchestration lives in teams_host, off this path.
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut env = env;
        if def.teams.as_ref().is_some_and(|t| t.is_host()) {
            env.extend(crate::teams_host::setup_lead(&app, id, def, &cwd, &env));
        }
        let spec = LaunchSpec {
            name: Some(def.name.clone()),
            cmd: command_for_definition(def, logical_resume).to_string(),
            args: Vec::new(),
            shell_wrap: true,
            cwd: Some(cwd),
            env,
            autorestart: def.autorestart.unwrap_or_default(),
            worktree,
        };
        let preserved_worktree = spec.worktree.as_ref().map(|info| info.path.clone());
        self.create_session_with_id(app, spec, cols, rows, id)
            .map_err(|error| {
                if let Some(path) = preserved_worktree {
                    format!("{error}; locked worktree was kept at {path}")
                } else {
                    error
                }
            })
    }

    fn create_session<R: Runtime>(
        &self,
        app: AppHandle<R>,
        spec: LaunchSpec,
        cols: u16,
        rows: u16,
    ) -> Result<u32, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.create_session_with_id(app, spec, cols, rows, id)
    }

    /// Like [`create_session`] but for a caller-reserved id (host leads reserve
    /// their id before spawn to set up the socket + env).
    fn create_session_with_id<R: Runtime>(
        &self,
        app: AppHandle<R>,
        spec: LaunchSpec,
        cols: u16,
        rows: u16,
        id: u32,
    ) -> Result<u32, String> {
        {
            let mut sessions = self.lock_sessions();
            sessions.insert(
                id,
                SessionSlot {
                    spec,
                    generation: 0,
                    state: SessionState::Starting,
                    code: None,
                    restart_count: 0,
                    manual_kill: false,
                    live: None,
                    cols,
                    rows,
                    spawned_at: Instant::now(),
                    output: Vec::new(),
                    kind: SessionKind::Pty,
                    teammate: None,
                },
            );
        }
        // expected_gen 0: nothing else can have re-generated a brand new slot.
        match spawn_into_slot(&app, &self.sessions, &self.generations, id, 0) {
            Ok(()) => Ok(id),
            Err(e) => {
                self.lock_sessions().remove(&id);
                Err(e)
            }
        }
    }

    pub fn write_pty(&self, id: u32, data: String) -> Result<(), String> {
        self.write_pty_bytes(id, data.as_bytes())
    }

    /// Raw-bytes stdin write (Phase 4.2 host teammate `write`, whose payload is
    /// arbitrary decoded key bytes rather than a UTF-8 string).
    pub fn write_pty_bytes(&self, id: u32, data: &[u8]) -> Result<(), String> {
        // Fix #1: clone the per-session writer handle under the map lock,
        // then DROP the map lock before the blocking write. A stalled PTY
        // input side must not wedge other sessions, and must not deadlock
        // against reader threads (which take the map lock per chunk).
        let writer = {
            let sessions = self.lock_sessions();
            let slot = sessions
                .get(&id)
                .ok_or_else(|| format!("session {id} not found"))?;
            let live = slot
                .live
                .as_ref()
                .ok_or_else(|| format!("session {id} is not running"))?;
            Arc::clone(&live.writer)
        };
        let mut w = lock_writer(&writer);
        w.write_all(data).map_err(|e| format!("write failed: {e}"))?;
        w.flush().map_err(|e| format!("flush failed: {e}"))?;
        Ok(())
    }

    /// Resize stays under the map lock: portable-pty's unix resize is a
    /// single TIOCSWINSZ ioctl on the master fd — a non-blocking syscall
    /// that cannot stall on child behavior, so it does not need the
    /// clone-and-release pattern used by write_pty.
    pub fn resize_pty(&self, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let mut sessions = self.lock_sessions();
        let slot = sessions
            .get_mut(&id)
            .ok_or_else(|| format!("session {id} not found"))?;
        // Remember the size so restarts reopen the PTY at the current size.
        slot.cols = cols;
        slot.rows = rows;
        if let Some(live) = slot.live.as_ref() {
            live.master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| format!("resize failed: {e}"))?;
        }
        Ok(())
    }

    /// Manual kill: never autorestarts. The reader thread reaps the child,
    /// emits `pty-exit` + `session-state`(exited) and removes the slot.
    /// (child.kill() is a signal send — non-blocking, safe under the lock.)
    pub fn kill_pty(&self, id: u32) -> Result<(), String> {
        let mut sessions = self.lock_sessions();
        let slot = sessions
            .get_mut(&id)
            .ok_or_else(|| format!("session {id} not found"))?;
        slot.manual_kill = true;
        match slot.live.as_mut() {
            Some(live) => {
                // A child that has already exited but not yet been reaped by
                // the reader thread makes kill() fail with ESRCH ("no such
                // process"). That is not a real failure: the reader will still
                // observe EOF and remove the slot, so report success (L5).
                if let Err(e) = live.child.kill() {
                    if !is_process_gone(&e) {
                        return Err(format!("kill failed: {e}"));
                    }
                }
                // reader thread handles reaping/eventing/removal
            }
            None => {
                // Already exited (or pending restart): just drop the slot.
                sessions.remove(&id);
            }
        }
        Ok(())
    }

    /// Manual restart: kill and respawn keeping the SAME id. The old reader
    /// thread is staled via the generation bump and emits nothing.
    pub fn restart_session<R: Runtime>(&self, app: AppHandle<R>, id: u32) -> Result<(), String> {
        let (old_live, info, new_gen) = {
            let mut sessions = self.lock_sessions();
            let slot = sessions
                .get_mut(&id)
                .ok_or_else(|| format!("session {id} not found"))?;
            if slot.kind == SessionKind::Transcript {
                return Err(format!(
                    "session {id} is a read-only transcript and cannot be restarted"
                ));
            }
            // Stale the current reader thread immediately.
            let new_gen = self.generations.fetch_add(1, Ordering::SeqCst);
            slot.generation = new_gen;
            slot.state = SessionState::Restarting;
            slot.code = None;
            slot.manual_kill = false;
            slot.restart_count = 0;
            (slot.live.take(), session_info(id, slot), new_gen)
        };
        // Reap the old child outside the lock. Dropping old_live (master,
        // writer) EOFs the stale reader, which then goes silent.
        if let Some(mut live) = old_live {
            let _ = live.child.kill();
            let _ = live.child.wait();
        }
        let _ = app.emit("session-state", &info);
        // Fix #3: a failed respawn must not strand the slot in `restarting`
        // with no event — mark it exited (generation-guarded) and still
        // surface the error to the caller.
        match spawn_into_slot(&app, &self.sessions, &self.generations, id, new_gen) {
            Ok(()) => Ok(()),
            Err(e) => {
                mark_exited_if_current(&app, &self.sessions, id, new_gen);
                Err(e)
            }
        }
    }

    /// All sessions, with the foreground process name resolved lazily here
    /// (pids are read under the lock — cheap ioctl — but the pid->name
    /// lookup runs after the lock is dropped: on macOS it shells out to ps).
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.list_sessions_with(pty::process_name)
    }

    /// Snapshot running PTY child roots for the shared resource sampler.
    /// `process_id` is an in-memory child handle lookup; no OS query or
    /// blocking work occurs while the sessions lock is held.
    pub(crate) fn resource_roots(&self) -> Vec<(u32, u32)> {
        let sessions = self.lock_sessions();
        sessions
            .iter()
            .filter(|(_, slot)| slot.state == SessionState::Running)
            .filter_map(|(id, slot)| {
                slot.live
                    .as_ref()
                    .and_then(|live| live.child.process_id())
                    .map(|pid| (*id, pid))
            })
            .collect()
    }

    /// Resolver-injected variant used to keep process lookup independently
    /// testable on restricted hosts where `ps` or `/proc` is unavailable.
    fn list_sessions_with<F>(&self, resolve_process: F) -> Vec<SessionInfo>
    where
        F: Fn(i32) -> Option<String>,
    {
        let mut list: Vec<(SessionInfo, Option<i32>)> = {
            let sessions = self.lock_sessions();
            sessions
                .iter()
                .map(|(id, slot)| (session_info(*id, slot), foreground_pid(slot)))
                .collect()
        };
        list.sort_by_key(|(s, _)| s.id);
        list.into_iter()
            .map(|(mut info, pid)| {
                info.foreground = pid.and_then(&resolve_process);
                info
            })
            .collect()
    }

    /// Full ring-buffer contents and current terminal dimensions (Queen
    /// read_output; spans restarts). Dimensions let the text renderer apply
    /// full-screen TUI cursor movements with the same bounds as the pane.
    pub fn output_snapshot(&self, id: u32) -> Result<(String, u16, u16), String> {
        let sessions = self.lock_sessions();
        let slot = sessions
            .get(&id)
            .ok_or_else(|| format!("session {id} not found"))?;
        Ok((
            String::from_utf8_lossy(&slot.output).to_string(),
            slot.rows,
            slot.cols,
        ))
    }

    /// Queen resolves `"#<id>"` first, then an exact unique definition/session
    /// name, and finally an exact unique foreground process name.
    /// Duplicate names are rejected with candidate ids instead of silently
    /// selecting one and risking delivery to the wrong terminal.
    /// Errors list the running sessions including their foreground names,
    /// so the message is self-documenting.
    pub fn resolve_agent(&self, agent: &str) -> Result<u32, String> {
        self.resolve_agent_with(agent, pty::process_name)
    }

    /// Resolver-injected variant for deterministic tests. Name resolution
    /// itself must not depend on permission to invoke the host process API.
    fn resolve_agent_with<F>(&self, agent: &str, resolve_process: F) -> Result<u32, String>
    where
        F: Fn(i32) -> Option<String>,
    {
        // Exact ids identify one session and are never ambiguous.
        if let Some(id_str) = agent.strip_prefix('#') {
            let id: u32 = id_str
                .trim()
                .parse()
                .map_err(|_| format!("invalid session id '{agent}'"))?;
            if self.lock_sessions().contains_key(&id) {
                return Ok(id);
            }
            return Err(format!("session {agent} not found"));
        }

        // Snapshot running sessions (id, name, fg pid) under the lock; the
        // pid->name lookup happens after the lock is dropped (macOS shells
        // out to ps).
        let (mut name_matches, mut running) = {
            let sessions = self.lock_sessions();
            let name_matches = sessions
                .iter()
                .filter(|(_, s)| {
                    s.state == SessionState::Running && s.spec.name.as_deref() == Some(agent)
                })
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            let running = sessions
                .iter()
                .filter(|(_, s)| s.state == SessionState::Running)
                .map(|(id, s)| (*id, s.spec.name.clone(), foreground_pid(s)))
                .collect::<Vec<_>>();
            (name_matches, running)
        };

        // 2. definition/session name
        name_matches.sort_unstable();
        match name_matches.as_slice() {
            [id] => return Ok(*id),
            [] => {}
            ids => {
                return Err(format!(
                    "ambiguous session name '{agent}'; use one of: {}",
                    format_ids(ids)
                ));
            }
        }

        running.sort_by_key(|(id, _, _)| *id);
        let with_fg: Vec<(u32, Option<String>, Option<String>)> = running
            .into_iter()
            .map(|(id, name, pid)| (id, name, pid.and_then(&resolve_process)))
            .collect();

        // 3. foreground process name (exact, case-sensitive, unique only)
        let fg_matches: Vec<u32> = with_fg
            .iter()
            .filter(|(_, _, fg)| fg.as_deref() == Some(agent))
            .map(|(id, _, _)| *id)
            .collect();
        match fg_matches.as_slice() {
            [id] => return Ok(*id),
            [] => {}
            ids => {
                return Err(format!(
                    "ambiguous foreground process '{agent}'; use one of: {}",
                    format_ids(ids)
                ));
            }
        }

        let running_list = if with_fg.is_empty() {
            "(none)".to_string()
        } else {
            with_fg
                .iter()
                .map(|(id, name, fg)| {
                    let label = name.as_deref().unwrap_or("shell");
                    match fg {
                        Some(f) => format!("#{id} {label} (fg: {f})"),
                        None => format!("#{id} {label}"),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        Err(format!(
            "no running session or foreground process named '{agent}'. \
             running sessions: {running_list}"
        ))
    }

    // ---------- transcript (read-only teammate) sessions, Phase 4.1 ----------

    /// The kind of a session, if it exists. Queen's `send_message` uses this to
    /// reject transcript destinations with a clear error.
    pub fn session_kind(&self, id: u32) -> Option<SessionKind> {
        self.lock_sessions().get(&id).map(|slot| slot.kind)
    }

    /// Running, named, PTY sessions with their resolved cwd. Used by the
    /// teammate hook receiver to resolve which lead a `SubagentStart` belongs
    /// to (transcript sessions are excluded — a lead is always a real PTY).
    pub fn running_named_sessions(&self) -> Vec<(u32, Option<String>, Option<PathBuf>)> {
        let sessions = self.lock_sessions();
        sessions
            .iter()
            .filter(|(_, s)| s.state == SessionState::Running && s.kind == SessionKind::Pty)
            .map(|(id, s)| (*id, s.spec.name.clone(), s.spec.cwd.clone()))
            .collect()
    }

    /// (total sessions, total transcript sessions, transcript-count-per-lead).
    /// Cheap snapshot under the lock for the pane-limit checks.
    pub fn transcript_stats(&self) -> (usize, usize, HashMap<u32, usize>) {
        let sessions = self.lock_sessions();
        let total = sessions.len();
        let mut transcripts = 0usize;
        let mut per_lead: HashMap<u32, usize> = HashMap::new();
        for slot in sessions.values() {
            if slot.kind == SessionKind::Transcript {
                transcripts += 1;
                if let Some(meta) = &slot.teammate {
                    *per_lead.entry(meta.lead_id).or_insert(0) += 1;
                }
            }
        }
        (total, transcripts, per_lead)
    }

    /// Create a PTY-less transcript session and return its stable id. Shares the
    /// existing id + generation counters. When `transcript_path` is Some (it is
    /// caller-validated to live under `$HOME/.claude/`), a background tail is
    /// started; otherwise the pane shows status only. Emits `session-state`.
    pub fn create_transcript_session<R: Runtime>(
        &self,
        app: AppHandle<R>,
        agent_id: String,
        role: Option<String>,
        lead_id: u32,
        transcript_path: Option<PathBuf>,
    ) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let generation = self.generations.fetch_add(1, Ordering::SeqCst);
        let info = {
            let mut sessions = self.lock_sessions();
            sessions.insert(
                id,
                SessionSlot {
                    spec: LaunchSpec {
                        name: role.clone(),
                        cmd: "transcript".to_string(),
                        args: Vec::new(),
                        shell_wrap: false,
                        cwd: None,
                        env: Vec::new(),
                        autorestart: AutoRestart::Never,
                        worktree: None,
                    },
                    generation,
                    // A transcript is "active" (Running) until SubagentStop.
                    state: SessionState::Running,
                    code: None,
                    restart_count: 0,
                    manual_kill: false,
                    live: None,
                    cols: 0,
                    rows: 0,
                    spawned_at: Instant::now(),
                    output: Vec::new(),
                    kind: SessionKind::Transcript,
                    teammate: Some(TeammateSlotMeta {
                        agent_id,
                        role,
                        lead_id,
                        mode: "observe",
                    }),
                },
            );
            session_info(id, sessions.get(&id).expect("just inserted"))
        };
        let _ = app.emit("session-state", &info);

        if let Some(path) = transcript_path {
            let sessions = Arc::clone(&self.sessions);
            let append: crate::transcript::AppendFn =
                Arc::new(move |gen: u64, text: &str| transcript_append(&sessions, id, gen, text));
            crate::transcript::spawn_tail(app, id, generation, path, append);
        }
        id
    }

    /// Transition the transcript session owning `agent_id` to `stopped`
    /// (Exited). The pane stays; the tail thread stops (generation bump). No-op
    /// when no such transcript session exists. Returns the affected id.
    pub fn stop_transcript_session<R: Runtime>(
        &self,
        app: AppHandle<R>,
        agent_id: &str,
    ) -> Option<u32> {
        let info = {
            let mut sessions = self.lock_sessions();
            let entry = sessions.iter_mut().find(|(_, s)| {
                s.kind == SessionKind::Transcript
                    && s.teammate.as_ref().is_some_and(|m| m.agent_id == agent_id)
            });
            let (id, slot) = entry?;
            let id = *id;
            slot.state = SessionState::Exited;
            // Bump the generation so the tail thread stops emitting.
            slot.generation = self.generations.fetch_add(1, Ordering::SeqCst);
            Some(session_info(id, slot))
        }?;
        let id = info.id;
        let _ = app.emit("session-state", &info);
        Some(id)
    }

    // ---------- host teammate (real PTY) sessions, Phase 4.2 ----------

    /// Spawn a split-window teammate as a real PTY session owned by `lead_id`.
    /// `argv` is pre-split (argv0 already allowlist-checked by the caller);
    /// runs with no shell wrap, no autorestart, and carries host teammate meta
    /// (`mode: "host"`). Returns the new session id (its context id is
    /// `%<id>`). Uses the same PTY spawn machinery as ordinary sessions.
    #[allow(clippy::too_many_arguments)] // mirrors the pane-backend spawn shape
    pub fn spawn_teammate<R: Runtime>(
        &self,
        app: AppHandle<R>,
        argv: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        lead_id: u32,
        role: Option<String>,
        cols: u16,
        rows: u16,
    ) -> Result<u32, String> {
        if argv.is_empty() {
            return Err("empty teammate command".to_string());
        }
        let cmd = argv[0].clone();
        let args = argv[1..].to_vec();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        {
            let mut sessions = self.lock_sessions();
            sessions.insert(
                id,
                SessionSlot {
                    spec: LaunchSpec {
                        name: role.clone(),
                        cmd,
                        args,
                        shell_wrap: false,
                        cwd,
                        env,
                        autorestart: AutoRestart::Never,
                        worktree: None,
                    },
                    generation: 0,
                    state: SessionState::Starting,
                    code: None,
                    restart_count: 0,
                    manual_kill: false,
                    live: None,
                    cols,
                    rows,
                    spawned_at: Instant::now(),
                    output: Vec::new(),
                    kind: SessionKind::Pty,
                    teammate: Some(TeammateSlotMeta {
                        agent_id: String::new(),
                        role,
                        lead_id,
                        mode: "host",
                    }),
                },
            );
        }
        match spawn_into_slot(&app, &self.sessions, &self.generations, id, 0) {
            Ok(()) => Ok(id),
            Err(e) => {
                self.lock_sessions().remove(&id);
                Err(e)
            }
        }
    }

    /// Live host-teammate PTY session ids owned by `lead_id`, sorted ascending.
    pub fn host_teammate_ids(&self, lead_id: u32) -> Vec<u32> {
        let sessions = self.lock_sessions();
        let mut ids: Vec<u32> = sessions
            .iter()
            .filter(|(_, s)| {
                s.kind == SessionKind::Pty
                    && s.teammate
                        .as_ref()
                        .is_some_and(|m| m.lead_id == lead_id && m.mode == "host")
            })
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Current state of a session, or None when it no longer exists. The host
    /// socket monitor uses this to detect that a lead has exited so it can tear
    /// down the per-lead server + socket file.
    pub fn session_state(&self, id: u32) -> Option<SessionState> {
        self.lock_sessions().get(&id).map(|slot| slot.state)
    }

    /// `(id, state, code)` for every host-teammate PTY session of `lead_id`.
    /// The host socket monitor diffs these across ticks to broadcast
    /// `context_exited` for teammates that exit or are removed.
    pub fn host_teammate_states(&self, lead_id: u32) -> Vec<(u32, SessionState, Option<i32>)> {
        let sessions = self.lock_sessions();
        let mut v: Vec<(u32, SessionState, Option<i32>)> = sessions
            .iter()
            .filter(|(_, s)| {
                s.kind == SessionKind::Pty
                    && s.teammate
                        .as_ref()
                        .is_some_and(|m| m.lead_id == lead_id && m.mode == "host")
            })
            .map(|(id, s)| (*id, s.state, s.code))
            .collect();
        v.sort_by_key(|t| t.0);
        v
    }

    /// Pane-limit inputs for a host lead's `split-window`: (this lead's live
    /// host-teammate count, total teammate sessions incl. transcripts, total
    /// sessions). Cheap snapshot under the lock.
    pub fn host_limit_inputs(&self, lead_id: u32) -> (usize, usize, usize) {
        let sessions = self.lock_sessions();
        let total = sessions.len();
        let mut per_lead_host = 0usize;
        let mut total_teammates = 0usize;
        for slot in sessions.values() {
            if let Some(meta) = &slot.teammate {
                total_teammates += 1;
                if meta.mode == "host" && meta.lead_id == lead_id {
                    per_lead_host += 1;
                }
            }
        }
        (per_lead_host, total_teammates, total)
    }
}

/// Generation-guarded append into a transcript slot's output ring. Returns
/// whether the slot is still current (same generation, still a transcript);
/// the tail thread uses the return value to detect that it has gone stale.
/// Empty text is a pure liveness check (no mutation).
fn transcript_append(sessions: &SharedSessions, id: u32, generation: u64, text: &str) -> bool {
    let mut guard = lock_map(sessions);
    match guard.get_mut(&id) {
        Some(slot) if slot.generation == generation && slot.kind == SessionKind::Transcript => {
            if !text.is_empty() {
                append_capped(&mut slot.output, text.as_bytes(), OUTPUT_CAP);
            }
            true
        }
        _ => false,
    }
}

fn format_ids(ids: &[u32]) -> String {
    ids.iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------- spawn / reader / autorestart machinery (detached-thread safe) ----------

/// Generation-guarded "give up as exited" transition used by failure paths
/// of delayed/spawning threads (fixes #3/#4): only writes state if the slot
/// still belongs to `expected_gen`.
fn mark_exited_if_current<R: Runtime>(
    app: &AppHandle<R>,
    sessions: &SharedSessions,
    id: u32,
    expected_gen: u64,
) {
    let info = {
        let mut sessions_guard = lock_map(sessions);
        match sessions_guard.get_mut(&id) {
            Some(slot) if slot.generation == expected_gen => {
                slot.state = SessionState::Exited;
                slot.code = None;
                Some(session_info(id, slot))
            }
            _ => None, // superseded or removed: do not touch state
        }
    };
    if let Some(info) = info {
        let _ = app.emit("session-state", &info);
    }
}

/// (Re)spawn the slot's command into a fresh PTY under a new generation and
/// start its reader thread. Emits `session-state`(running) on success.
///
/// `expected_gen` guards the whole operation (fix #4): if the slot's
/// generation no longer matches — a manual restart or kill won a race with a
/// delayed autorestart — the spawn is abandoned (any already-spawned child is
/// reaped) and NOTHING is written to the slot.
fn spawn_into_slot<R: Runtime>(
    app: &AppHandle<R>,
    sessions: &SharedSessions,
    generations: &Arc<AtomicU64>,
    id: u32,
    expected_gen: u64,
) -> Result<(), String> {
    let (spec, cols, rows) = {
        let sessions_guard = lock_map(sessions);
        let slot = sessions_guard
            .get(&id)
            .ok_or_else(|| format!("session {id} not found"))?;
        if slot.generation != expected_gen {
            return Err(format!("session {id} was superseded during spawn"));
        }
        (slot.spec.clone(), slot.cols, slot.rows)
    };

    let queen_url = crate::queen::current_env_url(app);
    let cmd = command_for_spec(&spec, queen_url.as_deref());
    let parts = pty::open_and_spawn(cmd, cols, rows)?;
    let generation = generations.fetch_add(1, Ordering::SeqCst);

    let info = {
        let mut sessions_guard = lock_map(sessions);
        match sessions_guard.get_mut(&id) {
            Some(slot) if slot.generation == expected_gen => {
                slot.generation = generation;
                slot.live = Some(LivePty {
                    master: parts.master,
                    writer: Arc::new(Mutex::new(parts.writer)),
                    child: parts.child,
                });
                slot.state = SessionState::Running;
                slot.code = None;
                slot.spawned_at = Instant::now();
                session_info(id, slot)
            }
            _ => {
                // Slot vanished (killed) or was re-generated (manual restart
                // won the race) while we were spawning: reap our orphan child
                // and bail WITHOUT touching the slot.
                drop(sessions_guard);
                let mut child = parts.child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "session {id} was removed or superseded during spawn"
                ));
            }
        }
    };
    let _ = app.emit("session-state", &info);

    spawn_reader_thread(
        app.clone(),
        Arc::clone(sessions),
        Arc::clone(generations),
        id,
        generation,
        parts.reader,
    );
    Ok(())
}

/// Dedicated blocking reader thread (contract: std::thread + AppHandle::emit).
/// Emits `pty-output` chunks while its generation is current; on EOF it
/// dispatches the exit/autorestart state machine.
fn spawn_reader_thread<R: Runtime>(
    app: AppHandle<R>,
    sessions: SharedSessions,
    generations: Arc<AtomicU64>,
    id: u32,
    generation: u64,
    mut reader: Box<dyn Read + Send>,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        // Holds an incomplete trailing multibyte UTF-8 sequence between reads
        // so a codepoint split at the read boundary emits intact (M1). The
        // ring buffer below stays byte-based and is unaffected.
        let mut carry: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    // Suppress output from stale generations (post-restart);
                    // current chunks also go into the slot's ring buffer.
                    let is_current = {
                        let mut sessions_guard = lock_map(&sessions);
                        match sessions_guard.get_mut(&id) {
                            Some(slot) if slot.generation == generation => {
                                append_capped(&mut slot.output, &buf[..n], OUTPUT_CAP);
                                true
                            }
                            _ => false,
                        }
                    };
                    if !is_current {
                        return; // stale: no EOF handling either
                    }
                    let data = decode_stream_chunk(&mut carry, &buf[..n]);
                    if !data.is_empty() {
                        let _ = app.emit("pty-output", OutputPayload { id, data });
                    }
                }
            }
        }
        handle_eof(&app, &sessions, &generations, id, generation);
    });
}

enum EofOutcome {
    Stale,
    Exited(SessionInfo, Option<i32>),
    Restarting(SessionInfo, Option<i32>),
}

/// Reader-thread EOF: reap the child, then either mark exited or schedule an
/// autorestart per policy.
///
/// Fix #2: child.wait() can linger after PTY EOF, so it must not run under
/// the map lock. Three phases:
///   1. under the lock: verify generation, take() the LivePty out;
///   2. outside the lock: wait() (reap) the child;
///   3. re-lock and RE-CHECK the generation before writing any state — a
///      manual restart may have won the race during phase 2.
fn handle_eof<R: Runtime>(
    app: &AppHandle<R>,
    sessions: &SharedSessions,
    generations: &Arc<AtomicU64>,
    id: u32,
    generation: u64,
) {
    // Phase 1: take the live parts under the lock (generation-checked).
    let live = {
        let mut sessions_guard = lock_map(sessions);
        match sessions_guard.get_mut(&id) {
            Some(slot) if slot.generation == generation => slot.live.take(),
            _ => return, // stale
        }
    };

    // Phase 2: reap OUTSIDE the lock. The child belongs to our generation
    // even if the slot moves on meanwhile, so reaping is always correct.
    let code = live
        .and_then(|mut l| l.child.wait().ok())
        .map(|status| status.exit_code() as i32);

    // Phase 3: re-lock, re-check generation, then decide and write state.
    let outcome = {
        let mut sessions_guard = lock_map(sessions);
        match sessions_guard.get_mut(&id) {
            None => EofOutcome::Stale,
            Some(slot) if slot.generation != generation => EofOutcome::Stale,
            Some(slot) => {
                slot.code = code;
                let stable_run = slot.spawned_at.elapsed() >= STABLE_RUN;
                match decide_eof(
                    slot.spec.autorestart,
                    slot.manual_kill,
                    code,
                    slot.restart_count,
                    stable_run,
                ) {
                    EofAction::Restart { new_count } => {
                        slot.restart_count = new_count;
                        slot.state = SessionState::Restarting;
                        EofOutcome::Restarting(session_info(id, slot), code)
                    }
                    EofAction::Exit { remove } => {
                        slot.state = SessionState::Exited;
                        let info = session_info(id, slot);
                        if remove {
                            // Phase 0 semantics: a killed session is removed.
                            sessions_guard.remove(&id);
                        }
                        EofOutcome::Exited(info, code)
                    }
                }
            }
        }
    };

    match outcome {
        EofOutcome::Stale => {}
        EofOutcome::Exited(info, code) => {
            let _ = app.emit("pty-exit", ExitPayload { id, code });
            let _ = app.emit("session-state", &info);
        }
        EofOutcome::Restarting(info, code) => {
            let _ = app.emit("pty-exit", ExitPayload { id, code });
            let _ = app.emit("session-state", &info);
            schedule_autorestart(
                app.clone(),
                Arc::clone(sessions),
                Arc::clone(generations),
                id,
                generation,
            );
        }
    }
}

/// Respawn after AUTORESTART_DELAY, unless the slot was killed, removed or
/// manually restarted (generation change) in the meantime. Both the spawn
/// itself and the failure transition are generation-guarded (fix #4).
fn schedule_autorestart<R: Runtime>(
    app: AppHandle<R>,
    sessions: SharedSessions,
    generations: Arc<AtomicU64>,
    id: u32,
    generation: u64,
) {
    std::thread::spawn(move || {
        std::thread::sleep(AUTORESTART_DELAY);

        let proceed = {
            let sessions_guard = lock_map(&sessions);
            matches!(
                sessions_guard.get(&id),
                Some(slot)
                    if slot.generation == generation
                        && slot.state == SessionState::Restarting
                        && !slot.manual_kill
            )
        };
        if !proceed {
            return;
        }

        if spawn_into_slot(&app, &sessions, &generations, id, generation).is_err() {
            // Respawn failed: give up as exited — but ONLY if the slot still
            // belongs to our generation (fix #4: a manual restart that won
            // the race must not be stomped to Exited).
            mark_exited_if_current(&app, &sessions, id, generation);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_resume_uses_optional_resume_command_only_for_resume() {
        let cfg = config::parse_config(
            "agents:\n  - name: codex\n    cmd: codex\n    resume: codex resume --last\n",
        )
        .unwrap();
        let def = &cfg.agents[0];
        assert_eq!(command_for_definition(def, false), "codex");
        assert_eq!(command_for_definition(def, true), "codex resume --last");

        let cfg = config::parse_config("agents:\n  - name: web\n    cmd: npm run dev\n").unwrap();
        assert_eq!(command_for_definition(&cfg.agents[0], true), "npm run dev");
    }

    // ----- ring buffer -----

    #[test]
    fn ring_buffer_appends_below_cap() {
        let mut buf = Vec::new();
        append_capped(&mut buf, b"hello ", 32);
        append_capped(&mut buf, b"world", 32);
        assert_eq!(buf, b"hello world");
    }

    #[test]
    fn ring_buffer_drops_oldest_beyond_cap() {
        let mut buf = Vec::new();
        append_capped(&mut buf, b"0123456789", 8);
        // single oversized chunk keeps only its tail
        assert_eq!(buf, b"23456789");

        append_capped(&mut buf, b"AB", 8);
        // oldest two bytes dropped, order preserved
        assert_eq!(buf, b"456789AB");
    }

    #[test]
    fn ring_buffer_exact_cap_boundary() {
        let mut buf = Vec::new();
        append_capped(&mut buf, b"12345678", 8);
        assert_eq!(buf, b"12345678");
        append_capped(&mut buf, b"", 8);
        assert_eq!(buf, b"12345678");
        append_capped(&mut buf, b"9", 8);
        assert_eq!(buf, b"23456789");
    }

    // ----- M1: multibyte-safe output streaming -----

    #[test]
    fn decode_stream_carries_split_multibyte_across_8k_reads() {
        // Japanese + emoji text well over 8KB, fed in 8192-byte PTY reads.
        // Every read boundary that lands mid-codepoint must round-trip without
        // a single U+FFFD replacement character.
        let text = "こんにちは世界🌍テスト".repeat(600);
        assert!(text.len() > 8192, "fixture must exceed one read");
        let bytes = text.as_bytes();
        let mut carry = Vec::new();
        let mut out = String::new();
        for chunk in bytes.chunks(8192) {
            out.push_str(&decode_stream_chunk(&mut carry, chunk));
        }
        assert!(carry.is_empty(), "complete input leaves no carry");
        assert_eq!(out, text);
        assert!(!out.contains('\u{FFFD}'), "no mojibake at any seam");
    }

    #[test]
    fn decode_stream_holds_incomplete_tail_until_completed() {
        // A 3-byte codepoint split 1/2 across two reads.
        let ch = "あ"; // 3 bytes: E3 81 82
        let bytes = ch.as_bytes();
        let mut carry = Vec::new();
        let first = decode_stream_chunk(&mut carry, &bytes[..1]);
        assert_eq!(first, "", "incomplete lead byte emits nothing");
        assert_eq!(carry.len(), 1);
        let second = decode_stream_chunk(&mut carry, &bytes[1..]);
        assert_eq!(second, ch, "completed codepoint emits intact");
        assert!(carry.is_empty());
    }

    #[test]
    fn decode_stream_flushes_genuine_garbage_lossily() {
        // A genuinely invalid byte (not an incomplete tail) must not stall the
        // stream; it is flushed lossily and the carry is drained.
        let mut carry = Vec::new();
        let out = decode_stream_chunk(&mut carry, &[b'a', 0xFF, b'b']);
        assert!(carry.is_empty());
        assert!(out.starts_with('a') && out.ends_with('b'));
        assert!(out.contains('\u{FFFD}'));
    }

    // ----- L5: kill() ESRCH classification -----

    #[test]
    fn process_gone_recognizes_esrch() {
        assert!(is_process_gone(&std::io::Error::from_raw_os_error(3)));
        assert!(is_process_gone(&std::io::Error::other(
            "No such process (os error 3)",
        )));
        assert!(!is_process_gone(&std::io::Error::from_raw_os_error(1)));
    }

    // ----- pure EOF/autorestart decision -----

    #[test]
    fn decide_never_exits() {
        assert_eq!(
            decide_eof(AutoRestart::Never, false, Some(1), 0, false),
            EofAction::Exit { remove: false }
        );
    }

    #[test]
    fn decide_always_restarts_and_counts() {
        assert_eq!(
            decide_eof(AutoRestart::Always, false, Some(0), 0, false),
            EofAction::Restart { new_count: 1 }
        );
        assert_eq!(
            decide_eof(AutoRestart::Always, false, Some(0), 3, false),
            EofAction::Restart { new_count: 4 }
        );
    }

    #[test]
    fn decide_on_failure_checks_code() {
        // non-zero code -> restart
        assert_eq!(
            decide_eof(AutoRestart::OnFailure, false, Some(2), 0, false),
            EofAction::Restart { new_count: 1 }
        );
        // zero code -> exit
        assert_eq!(
            decide_eof(AutoRestart::OnFailure, false, Some(0), 0, false),
            EofAction::Exit { remove: false }
        );
        // unknown code -> treated as failure
        assert_eq!(
            decide_eof(AutoRestart::OnFailure, false, None, 0, false),
            EofAction::Restart { new_count: 1 }
        );
    }

    #[test]
    fn decide_manual_kill_suppresses_restart_and_removes() {
        assert_eq!(
            decide_eof(AutoRestart::Always, true, Some(1), 0, false),
            EofAction::Exit { remove: true }
        );
    }

    #[test]
    fn decide_cap_five_consecutive_restarts() {
        assert_eq!(
            decide_eof(AutoRestart::Always, false, Some(1), 4, false),
            EofAction::Restart { new_count: 5 }
        );
        // at the cap -> give up as exited
        assert_eq!(
            decide_eof(AutoRestart::Always, false, Some(1), 5, false),
            EofAction::Exit { remove: false }
        );
        // ...unless the last run was stable, which resets the counter
        assert_eq!(
            decide_eof(AutoRestart::Always, false, Some(1), 5, true),
            EofAction::Restart { new_count: 1 }
        );
    }

    // ----- integration: real PTY + tauri mock runtime -----

    fn mock_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.handle().clone()
    }

    /// Regression for fix #3: restart_session whose respawn fails must
    /// transition the slot to exited (code None), not strand it restarting.
    #[test]
    fn restart_failure_marks_session_exited() {
        let handle = mock_handle();
        let manager = PtyManager::new();

        // Spawn a session whose binary we can delete afterwards.
        let dir = std::env::temp_dir().join(format!("mterm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("cat-copy");
        std::fs::copy("/bin/cat", &bin).unwrap();

        let id = manager
            .spawn_shell(
                handle.clone(),
                80,
                24,
                Some(bin.display().to_string()),
                None,
            )
            .expect("initial spawn should succeed");
        assert_eq!(
            manager.list_sessions()[0].state,
            SessionState::Running,
            "session should be running after spawn"
        );

        // Make the respawn fail, then restart.
        std::fs::remove_file(&bin).unwrap();
        let err = manager
            .restart_session(handle.clone(), id)
            .expect_err("restart must fail for a deleted binary");
        assert!(!err.is_empty());

        // The slot must be exited (not stuck restarting), code None.
        let sessions = manager.list_sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].state, SessionState::Exited);
        assert_eq!(sessions[0].code, None);

        // And it can be cleaned up like any exited session.
        manager.kill_pty(id).unwrap();
        assert!(manager.list_sessions().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression for fix #4: spawn_into_slot with a stale expected
    /// generation must not touch the slot.
    #[test]
    fn stale_spawn_does_not_stomp_slot() {
        let handle = mock_handle();
        let manager = PtyManager::new();

        let id = manager
            .spawn_shell(handle.clone(), 80, 24, Some("/bin/cat".to_string()), None)
            .expect("spawn should succeed");

        // Simulate a delayed autorestart thread holding an outdated
        // generation: it must refuse to spawn and must not alter state.
        let stale_gen = 999_999;
        let err = spawn_into_slot(
            &handle,
            &manager.sessions,
            &manager.generations,
            id,
            stale_gen,
        )
        .expect_err("stale-generation spawn must fail");
        assert!(err.contains("superseded"), "unexpected error: {err}");

        // The healthy session is untouched.
        assert_eq!(manager.list_sessions()[0].state, SessionState::Running);

        // ...and the generation-guarded failure transition is a no-op too.
        mark_exited_if_current(&handle, &manager.sessions, id, stale_gen);
        assert_eq!(manager.list_sessions()[0].state, SessionState::Running);

        manager.kill_pty(id).unwrap();
    }

    /// Phase 2.1: foreground process name shows up lazily in list_sessions
    /// and resolves as an agent name (order: name -> foreground -> #id).
    #[test]
    fn foreground_name_listed_and_resolvable() {
        let handle = mock_handle();
        let manager = PtyManager::new();

        let id1 = manager
            .spawn_shell(handle.clone(), 80, 24, Some("/bin/cat".to_string()), None)
            .unwrap();
        let id2 = manager
            .spawn_shell(handle.clone(), 80, 24, Some("/bin/cat".to_string()), None)
            .unwrap();

        // tcgetpgrp needs the child to be set up; poll briefly. Resolve the
        // returned pid deterministically so this test also works in sandboxed
        // environments where macOS `ps` is not permitted.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut fg = None;
        while Instant::now() < deadline {
            fg = manager
                .list_sessions_with(|_| Some("cat".to_string()))
                .iter()
                .find(|s| s.id == id1)
                .and_then(|s| s.foreground.clone());
            if fg.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            fg.as_deref(),
            Some("cat"),
            "foreground name in list_sessions"
        );

        // Duplicate foreground names are never guessed: callers must use #id.
        // (poll: id2's child may become the fg pgrp slightly after id1's)
        let deadline = Instant::now() + Duration::from_secs(5);
        let duplicate_error = loop {
            let result = manager.resolve_agent_with("cat", |_| Some("cat".to_string()));
            if matches!(&result, Err(error) if error.contains("ambiguous foreground")) {
                break result.unwrap_err();
            }
            if Instant::now() >= deadline {
                panic!("expected ambiguous foreground error, got {result:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(duplicate_error.contains(&format!("#{id1}")));
        assert!(duplicate_error.contains(&format!("#{id2}")));
        // Case-sensitive: no match, and the error lists sessions with fg names.
        let err = manager
            .resolve_agent_with("CAT", |_| Some("cat".to_string()))
            .unwrap_err();
        assert!(err.contains("fg: cat"), "error should list fg names: {err}");
        // "#<id>" still resolves (tried last).
        assert_eq!(
            manager.resolve_agent_with(&format!("#{id1}"), |_| Some("cat".to_string())),
            Ok(id1)
        );

        // session-state style info (hot path) must NOT carry foreground.
        // (list_sessions computes it; session_info itself leaves it None —
        // verified indirectly: an exited session has no live PTY.)
        manager.kill_pty(id1).unwrap();
        manager.kill_pty(id2).unwrap();
    }

    #[test]
    fn duplicate_definition_names_require_an_exact_session_id() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        let config =
            config::parse_config("agents:\n  - name: worker\n    cmd: exec /bin/cat\n").unwrap();
        let cwd = std::env::current_dir().unwrap();
        let first = manager
            .spawn_agent(handle.clone(), &config.agents[0], &cwd, 80, 24)
            .unwrap();
        let second = manager
            .spawn_agent(handle, &config.agents[0], &cwd, 80, 24)
            .unwrap();

        let error = manager.resolve_agent_with("worker", |_| None).unwrap_err();
        assert!(error.contains("ambiguous session name"));
        assert!(error.contains(&format!("#{first}")));
        assert!(error.contains(&format!("#{second}")));
        assert_eq!(
            manager.resolve_agent_with(&format!("#{first}"), |_| None),
            Ok(first)
        );

        manager.kill_pty(first).unwrap();
        manager.kill_pty(second).unwrap();
    }

    /// End-to-end sanity for fix #1: write_pty round-trips through a real
    /// PTY (echo from `cat`) into the ring buffer without holding the map
    /// lock across the write.
    #[test]
    fn write_pty_roundtrip_via_ring_buffer() {
        let handle = mock_handle();
        let manager = PtyManager::new();

        let id = manager
            .spawn_shell(handle.clone(), 80, 24, Some("/bin/cat".to_string()), None)
            .expect("spawn should succeed");

        manager
            .write_pty(id, "ping-42\r".to_string())
            .expect("write should succeed");

        // cat echoes stdin back; the PTY reader thread appends it to the
        // ring buffer. Poll briefly.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = String::new();
        while Instant::now() < deadline {
            seen = manager.output_snapshot(id).unwrap().0;
            if seen.contains("ping-42") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            seen.contains("ping-42"),
            "expected echoed output, got: {seen:?}"
        );

        manager.kill_pty(id).unwrap();
    }

    // ----- Phase 4.1: transcript sessions -----

    #[test]
    fn transcript_session_is_pty_less_and_stops_on_subagent_stop() {
        let handle = mock_handle();
        let manager = PtyManager::new();

        let id = manager.create_transcript_session(
            handle.clone(),
            "agent-1".to_string(),
            Some("reviewer".to_string()),
            3,
            None, // no path -> status only, no tail thread
        );

        let sessions = manager.list_sessions();
        assert_eq!(sessions.len(), 1);
        let info = &sessions[0];
        assert_eq!(info.id, id);
        assert_eq!(info.kind, SessionKind::Transcript);
        assert_eq!(info.state, SessionState::Running); // "active"
        let teammate = info.teammate.as_ref().expect("teammate meta present");
        assert_eq!(teammate.role.as_deref(), Some("reviewer"));
        assert_eq!(teammate.lead_id, 3);
        assert_eq!(teammate.mode, "observe");
        assert_eq!(manager.session_kind(id), Some(SessionKind::Transcript));

        // ids are shared with the PTY counter: a following PTY session is id+1.
        let shell = manager
            .spawn_shell(handle.clone(), 80, 24, Some("/bin/cat".to_string()), None)
            .unwrap();
        assert_eq!(shell, id + 1);
        assert_eq!(manager.session_kind(shell), Some(SessionKind::Pty));
        // Killing a live PTY reaps asynchronously; wait for the slot to drop.
        manager.kill_pty(shell).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while manager.session_kind(shell).is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(manager.session_kind(shell), None);

        // SubagentStop transitions the matching transcript to exited (stopped).
        let stopped = manager.stop_transcript_session(handle.clone(), "agent-1");
        assert_eq!(stopped, Some(id));
        let info = manager.list_sessions().into_iter().find(|s| s.id == id).unwrap();
        assert_eq!(info.state, SessionState::Exited); // "stopped"

        // An unknown agent_id is a no-op.
        assert_eq!(manager.stop_transcript_session(handle.clone(), "nope"), None);

        // Pane close removes the transcript synchronously (no PTY to reap).
        manager.kill_pty(id).unwrap();
        assert!(manager.list_sessions().is_empty());
    }

    #[test]
    fn transcript_read_output_returns_appended_text_and_rejects_restart() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        let id = manager.create_transcript_session(
            handle.clone(),
            "agent-2".to_string(),
            None,
            1,
            None,
        );

        // Simulate a tail appending formatted text at the current generation.
        let gen = manager.lock_sessions().get(&id).unwrap().generation;
        assert!(transcript_append(&manager.sessions, id, gen, "user: hi\n"));
        let (text, _rows, _cols) = manager.output_snapshot(id).unwrap();
        assert_eq!(text, "user: hi\n");

        // A stale generation must neither append nor report current.
        assert!(!transcript_append(&manager.sessions, id, gen + 999, "assistant: stale\n"));
        assert_eq!(manager.output_snapshot(id).unwrap().0, "user: hi\n");

        // Transcript sessions can never be restarted (no PTY to respawn).
        let err = manager.restart_session(handle, id).unwrap_err();
        assert!(err.contains("transcript"), "unexpected error: {err}");

        manager.kill_pty(id).unwrap();
    }

    #[test]
    fn transcript_stats_and_lead_counts() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        manager.create_transcript_session(handle.clone(), "a".into(), None, 5, None);
        manager.create_transcript_session(handle.clone(), "b".into(), None, 5, None);
        manager.create_transcript_session(handle.clone(), "c".into(), None, 7, None);

        let (total, transcripts, per_lead) = manager.transcript_stats();
        assert_eq!(total, 3);
        assert_eq!(transcripts, 3);
        assert_eq!(per_lead.get(&5), Some(&2));
        assert_eq!(per_lead.get(&7), Some(&1));

        // Transcript sessions are never treated as leads.
        assert!(manager.running_named_sessions().is_empty());
    }

    #[test]
    fn resource_roots_expose_each_running_pty_child() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        let id = manager
            .spawn_shell(handle, 80, 24, Some("/bin/cat".to_string()), None)
            .unwrap();
        let roots = manager.resource_roots();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].0, id);
        assert!(roots[0].1 > 0);
        manager.kill_pty(id).unwrap();
    }
}
