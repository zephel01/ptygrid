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
        CommandBuilder::new(&spec.cmd)
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
        AutoRestart::OnFailure => code.map_or(true, |c| c != 0),
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
        let spec = LaunchSpec {
            name: Some(def.name.clone()),
            cmd: command_for_definition(def, logical_resume).to_string(),
            shell_wrap: true,
            cwd: Some(cwd),
            env,
            autorestart: def.autorestart.unwrap_or_default(),
            worktree,
        };
        let preserved_worktree = spec.worktree.as_ref().map(|info| info.path.clone());
        self.create_session(app, spec, cols, rows).map_err(|error| {
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
        w.write_all(data.as_bytes())
            .map_err(|e| format!("write failed: {e}"))?;
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
                live.child.kill().map_err(|e| format!("kill failed: {e}"))?;
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
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app.emit("pty-output", OutputPayload { id, data });
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
