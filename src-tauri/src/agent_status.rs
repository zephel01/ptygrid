// Semantic agent-status detection (Phase 4.4.0).
//
// This module estimates a per-session semantic status — working / blocked /
// done / idle (+ unknown) — for LIVE (`running`) PTY sessions, on top of and
// SEPARATE FROM `SessionState` (process liveness). It is an OPINION derived
// from terminal-output heuristics, never a fact, and it never mutates
// `SessionState`.
//
// Design constraints (see docs/spec-agent-status.md §7 and docs/design.md §11):
// - The session reader hot path NEVER runs regex/render here. It only marks a
//   session dirty (`mark_dirty`, an atomic + unbounded channel send).
// - A single debounced task (`start`) wakes every `debounce_ms` (default
//   250ms), takes the dirty set, and for each dirty running session does
//   `output_snapshot` -> `ansi::render_terminal` -> tail N lines -> `classify`.
// - Regexes are compiled ONCE (at startup and on config reload) and cached;
//   never compiled on the hot path or per evaluation.
// - `agent-status` is emitted ONLY when a session's status actually changes.
//
// Built-in default patterns live in `agent_status_defaults.yml` (embedded via
// `include_str!`) — a single place to update as CLI wording drifts.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::config::{AgentStatusConfig, Config};
use crate::session::PtyManager;

/// Semantic status of a session. Serialized to the lowercase wire strings the
/// frontend expects (`"working"`, `"blocked"`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Working,
    Blocked,
    Done,
    Idle,
    Unknown,
}

// ---------- built-in defaults (embedded, single source of truth) ----------

/// Raw (uncompiled) pattern lists for one ruleset key.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawRuleSet {
    #[serde(default)]
    blocked: Vec<String>,
    #[serde(default)]
    working: Vec<String>,
    #[serde(default)]
    done: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawDefaults {
    #[serde(default)]
    patterns: HashMap<String, RawRuleSet>,
}

/// The embedded built-in patterns, parsed once. NOTE: these go stale as CLI UIs
/// change — see the header of `agent_status_defaults.yml`.
static DEFAULT_RAW: LazyLock<RawDefaults> = LazyLock::new(|| {
    serde_norway::from_str(include_str!("agent_status_defaults.yml"))
        .expect("built-in agent_status_defaults.yml must parse")
});

// ---------- compiled rules ----------

/// A single compiled pattern; `source` doubles as the `matchedRule` id.
struct CompiledRule {
    source: String,
    re: regex::Regex,
}

/// Compiled patterns for one ruleset key, grouped by category.
pub struct CompiledRuleSet {
    blocked: Vec<CompiledRule>,
    working: Vec<CompiledRule>,
    done: Vec<CompiledRule>,
}

/// All rulesets, keyed by agent-definition / foreground-process name (+ the
/// opt-in `"*"` generic key). Rebuilt wholesale on config reload.
pub struct CompiledRules {
    sets: HashMap<String, CompiledRuleSet>,
}

impl CompiledRules {
    fn get(&self, key: &str) -> Option<&CompiledRuleSet> {
        self.sets.get(key)
    }
}

/// Compile one category's patterns. Each pattern is case-insensitive + multiline
/// by default (partial match); inline flags (e.g. `(?-i)`) can override per
/// pattern. An invalid regex is skipped (with a warning) so a single bad line
/// never disables the rest of the config (config-reload non-destructiveness).
fn compile_list(key: &str, category: &str, patterns: &[String]) -> Vec<CompiledRule> {
    let mut out = Vec::with_capacity(patterns.len());
    for source in patterns {
        match RegexBuilder::new(source)
            .case_insensitive(true)
            .multi_line(true)
            .build()
        {
            Ok(re) => out.push(CompiledRule {
                source: source.clone(),
                re,
            }),
            Err(e) => eprintln!(
                "agent_status: skipping invalid regex in {key}.{category} {source:?}: {e}"
            ),
        }
    }
    out
}

/// Merge the user's `agent_status.patterns` onto the built-in defaults and
/// compile the result. Merge semantics (spec §4.2):
/// - default is MERGE: each category is `built-in ++ user` (order: built-in
///   then user); a category the user omits keeps the built-in list.
/// - `replace: true` discards the built-in ruleset for that key; omitted
///   categories are then empty.
/// - keys with no built-in counterpart are added as new rulesets.
/// - `"*"` (generic opt-in) is compiled like any other key; ruleset SELECTION
///   only falls back to it when nothing else matched (`select_ruleset`).
fn build_rules(user: Option<&AgentStatusConfig>) -> CompiledRules {
    // Start from the built-in raw lists.
    let mut merged: HashMap<String, RawRuleSet> = DEFAULT_RAW.patterns.clone();

    if let Some(user_patterns) = user.and_then(|c| c.patterns.as_ref()) {
        for (key, set) in user_patterns {
            let replace = set.replace.unwrap_or(false);
            let mut base = if replace {
                RawRuleSet::default()
            } else {
                merged.get(key).cloned().unwrap_or_default()
            };
            // A provided category is appended (merge) or set (replace, base is
            // empty); an omitted category keeps `base` (built-in on merge,
            // empty on replace).
            if let Some(blocked) = &set.blocked {
                base.blocked.extend(blocked.iter().cloned());
            }
            if let Some(working) = &set.working {
                base.working.extend(working.iter().cloned());
            }
            if let Some(done) = &set.done {
                base.done.extend(done.iter().cloned());
            }
            merged.insert(key.clone(), base);
        }
    }

    let sets = merged
        .into_iter()
        .map(|(key, raw)| {
            let compiled = CompiledRuleSet {
                blocked: compile_list(&key, "blocked", &raw.blocked),
                working: compile_list(&key, "working", &raw.working),
                done: compile_list(&key, "done", &raw.done),
            };
            (key, compiled)
        })
        .collect();
    CompiledRules { sets }
}

// ---------- classify (pure) ----------

/// Classify reconstructed terminal `text` against one ruleset.
///
/// Decision order is **blocked > working > done > idle** (spec §3.3). `blocked`
/// is intentionally listed FIRST but its patterns are the narrowest (only known
/// approval / permission / choice UIs) — that asymmetry (highest priority, most
/// conservative firing condition) is how blocked-conservatism is enforced: an
/// unknown prompt, a bare shell return, or empty output matches no blocked
/// pattern and therefore falls through to `idle`, never `blocked` (spec §2.3).
///
/// A ruleset is always present here (unknown-ruleset sessions are handled by
/// `select_ruleset` returning None and are never classified); with a ruleset,
/// the fallthrough is `idle`. Returns the first matching pattern's source as
/// the `matchedRule` id (debug / tooltip).
pub fn classify(text: &str, rules: &CompiledRuleSet) -> (AgentStatus, Option<String>) {
    if let Some(rule) = rules.blocked.iter().find(|r| r.re.is_match(text)) {
        return (AgentStatus::Blocked, Some(rule.source.clone()));
    }
    if let Some(rule) = rules.working.iter().find(|r| r.re.is_match(text)) {
        return (AgentStatus::Working, Some(rule.source.clone()));
    }
    if let Some(rule) = rules.done.iter().find(|r| r.re.is_match(text)) {
        return (AgentStatus::Done, Some(rule.source.clone()));
    }
    (AgentStatus::Idle, None)
}

/// Select the ruleset for a session (spec §3.1), resolved lazily on EVERY
/// evaluation (never fixed at spawn — the foreground process changes):
/// 1. agent-definition name, else
/// 2. foreground process name (only resolved when 1 misses — on macOS this
///    shells out to `ps`, so we skip it for named agents), else
/// 3. the opt-in generic `"*"` key, else
/// 4. None -> the session is `unknown` (no badge).
fn select_ruleset<'a>(
    rules: &'a CompiledRules,
    name: Option<&str>,
    resolve_fg: impl FnOnce() -> Option<String>,
) -> (Option<String>, Option<&'a CompiledRuleSet>) {
    if let Some(name) = name {
        if let Some(set) = rules.get(name) {
            return (Some(name.to_string()), Some(set));
        }
    }
    if let Some(fg) = resolve_fg() {
        if let Some(set) = rules.get(&fg) {
            return (Some(fg), Some(set));
        }
    }
    if let Some(set) = rules.get("*") {
        return (Some("*".to_string()), Some(set));
    }
    (None, None)
}

// ---------- per-session state machine ----------

/// Per-session transition state: the last emitted status, when `done` began
/// (for `done_linger` decay), and the ruleset key in effect (carried onto the
/// linger->idle emit).
#[derive(Default)]
struct Tracker {
    current: Option<AgentStatus>,
    done_since: Option<Instant>,
    rule_set: Option<String>,
}

impl Tracker {
    /// Apply a fresh classification result, returning the new status IFF it
    /// changed (i.e. an `agent-status` event should be emitted).
    ///
    /// `done` transition rules (spec §3.4):
    /// - a `done` match, OR a working->idle transition (working pattern gone,
    ///   prompt returned), becomes `done`;
    /// - a stale `done` match while already `idle` does NOT resurrect `done`
    ///   (prevents a lingering `✓` from flickering done<->idle);
    /// - `done_linger` is measured from first entry into `done`; a re-match does
    ///   not extend it (so `done` reliably decays to `idle`);
    /// - `done_linger == 0` disables `done` (transitions go straight to `idle`).
    fn observe(&mut self, raw: AgentStatus, linger: Duration) -> Option<AgentStatus> {
        let prev = self.current;
        let mut next = match raw {
            AgentStatus::Blocked => AgentStatus::Blocked,
            AgentStatus::Working => AgentStatus::Working,
            AgentStatus::Unknown => AgentStatus::Unknown,
            AgentStatus::Done => {
                // Do not resurrect done from a decayed idle.
                if prev == Some(AgentStatus::Idle) {
                    AgentStatus::Idle
                } else {
                    AgentStatus::Done
                }
            }
            AgentStatus::Idle => match prev {
                Some(AgentStatus::Working) => AgentStatus::Done, // working -> prompt return
                Some(AgentStatus::Done) => AgentStatus::Done,    // hold through linger
                _ => AgentStatus::Idle,
            },
        };
        // done_linger == 0 means "never show done".
        if next == AgentStatus::Done && linger.is_zero() {
            next = AgentStatus::Idle;
        }

        if next == AgentStatus::Done {
            if prev != Some(AgentStatus::Done) {
                self.done_since = Some(Instant::now());
            }
        } else {
            self.done_since = None;
        }

        self.current = Some(next);
        if prev != Some(next) {
            Some(next)
        } else {
            None
        }
    }

    /// Decay `done` to `idle` once `done_linger` has elapsed. Called every tick
    /// (even for sessions with no new output), returning the new status IFF it
    /// changed.
    fn tick_linger(&mut self, linger: Duration) -> Option<AgentStatus> {
        if self.current == Some(AgentStatus::Done) {
            if let Some(since) = self.done_since {
                if since.elapsed() >= linger {
                    self.current = Some(AgentStatus::Idle);
                    self.done_since = None;
                    return Some(AgentStatus::Idle);
                }
            }
        }
        None
    }
}

// ---------- managed state ----------

/// Live detector settings, swapped wholesale on config reload.
struct Settings {
    rules: Arc<CompiledRules>,
    tail_lines: usize,
    debounce: Duration,
    done_linger: Duration,
}

/// Tauri-managed state for the agent-status detector. Holds the compiled rules,
/// the enabled flag, and the dirty-notification channel. The receiver is taken
/// once by `start`.
pub struct AgentStatusManager {
    enabled: AtomicBool,
    settings: Mutex<Settings>,
    dirty_tx: UnboundedSender<u32>,
    dirty_rx: Mutex<Option<UnboundedReceiver<u32>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Default for AgentStatusManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentStatusManager {
    /// Build with the built-in defaults (enabled). `load_config` later calls
    /// [`apply`] to fold in the user's `agent_status` block.
    pub fn new() -> Self {
        let (dirty_tx, dirty_rx) = unbounded_channel();
        let defaults = AgentStatusConfig::default();
        AgentStatusManager {
            enabled: AtomicBool::new(true),
            settings: Mutex::new(Settings {
                rules: Arc::new(build_rules(None)),
                tail_lines: defaults.effective_tail_lines(),
                debounce: Duration::from_millis(defaults.effective_debounce_ms()),
                done_linger: Duration::from_millis(defaults.effective_done_linger_ms()),
            }),
            dirty_tx,
            dirty_rx: Mutex::new(Some(dirty_rx)),
        }
    }

    /// Rebuild rules + scalars from a (possibly absent) config block. Called on
    /// every `load_config` (including config-file reloads), so pattern edits
    /// recompile and take effect immediately.
    fn reconfigure(&self, cfg: Option<&AgentStatusConfig>) {
        let owned;
        let effective = match cfg {
            Some(c) => c,
            None => {
                owned = AgentStatusConfig::default();
                &owned
            }
        };
        self.enabled
            .store(effective.effective_enabled(), Ordering::Relaxed);
        let mut settings = lock(&self.settings);
        settings.rules = Arc::new(build_rules(cfg));
        settings.tail_lines = effective.effective_tail_lines();
        settings.debounce = Duration::from_millis(effective.effective_debounce_ms());
        settings.done_linger = Duration::from_millis(effective.effective_done_linger_ms());
    }

    fn snapshot(&self) -> (bool, Arc<CompiledRules>, usize, Duration, Duration) {
        let enabled = self.enabled.load(Ordering::Relaxed);
        let settings = lock(&self.settings);
        (
            enabled,
            Arc::clone(&settings.rules),
            settings.tail_lines,
            settings.debounce,
            settings.done_linger,
        )
    }
}

/// Mark a session dirty from the reader hot path (spec §7.1). Cheap and
/// lock-free relative to the sessions map: an enabled-flag load plus an
/// unbounded channel send. A no-op when the manager is unmanaged (session unit
/// tests) or detection is disabled.
pub fn mark_dirty<R: Runtime>(app: &AppHandle<R>, id: u32) {
    if let Some(state) = app.try_state::<AgentStatusManager>() {
        if state.enabled.load(Ordering::Relaxed) {
            let _ = state.dirty_tx.send(id);
        }
    }
}

/// Fold the loaded config's `agent_status` block into the managed detector
/// (rules recompiled, enabled/tail/debounce/linger updated). Called from
/// `load_config`.
pub fn apply<R: Runtime>(app: &AppHandle<R>, config: &Config) {
    if let Some(state) = app.try_state::<AgentStatusManager>() {
        state.reconfigure(config.agent_status.as_ref());
    }
}

// ---------- emit ----------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentStatusPayload {
    id: u32,
    status: AgentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched_rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_set: Option<String>,
}

fn emit<R: Runtime>(
    app: &AppHandle<R>,
    id: u32,
    status: AgentStatus,
    matched_rule: Option<String>,
    rule_set: Option<String>,
) {
    let _ = app.emit(
        "agent-status",
        AgentStatusPayload {
            id,
            status,
            matched_rule,
            rule_set,
        },
    );
}

// ---------- tail reconstruction ----------

/// Last `n` lines of `text` (matches queen.rs `read_output` tail semantics).
fn last_n_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Build the classifier input for a session snapshot: reconstruct the current
/// terminal screen from the raw ring (transcript sessions are already formatted
/// and pass through), then take the tail N lines. Detection is ALWAYS on this
/// reconstructed text, never the raw byte stream (spec §3.2).
fn reconstruct_tail(snapshot: &crate::session::AgentStatusSnapshot, tail_lines: usize) -> String {
    let text = String::from_utf8_lossy(&snapshot.output);
    let rendered = if snapshot.kind == crate::session::SessionKind::Transcript {
        text.into_owned()
    } else {
        crate::ansi::render_terminal(&text, snapshot.rows, snapshot.cols)
    };
    last_n_lines(&rendered, tail_lines)
}

// ---------- evaluation loop ----------

/// Spawn the single debounced evaluation task. Takes the dirty receiver out of
/// the managed state (idempotent: a second call finds it already taken and
/// returns). Runs on `tauri::async_runtime`.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AgentStatusManager>() else {
        return;
    };
    let Some(mut dirty_rx) = lock(&state.dirty_rx).take() else {
        return; // already started
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut trackers: HashMap<u32, Tracker> = HashMap::new();
        loop {
            // Read the current settings in a tight scope so the managed-state
            // guard is dropped BEFORE the await below (it is not held across
            // suspension points). Ends when the state is gone (app teardown).
            let (enabled, rules, tail_lines, debounce, done_linger) = {
                let Some(state) = app.try_state::<AgentStatusManager>() else {
                    break;
                };
                state.snapshot()
            };
            tokio::time::sleep(debounce).await;

            // Drain the burst accumulated during the debounce window into a set
            // (one evaluation per session per tick -> <= 1000/debounce_ms Hz).
            let mut dirty: HashSet<u32> = HashSet::new();
            while let Ok(id) = dirty_rx.try_recv() {
                dirty.insert(id);
            }

            if !enabled {
                // Detection off: swallow the drained ids, keep trackers as-is,
                // emit nothing.
                continue;
            }

            let manager = app.state::<PtyManager>();
            evaluate_tick(
                &app,
                &manager,
                &rules,
                tail_lines,
                done_linger,
                &dirty,
                &mut trackers,
            );
        }
    });
}

/// Map a semantic-status *change* to the out-of-app notification event it should
/// raise (Phase 4.4.2): `blocked` needs a human, `done` is a completion. Other
/// statuses (working/idle/unknown) never notify.
fn notify_event_for(status: AgentStatus) -> Option<crate::notifications::NotifyEvent> {
    match status {
        AgentStatus::Blocked => Some(crate::notifications::NotifyEvent::NeedsAttention),
        AgentStatus::Done => Some(crate::notifications::NotifyEvent::Complete),
        _ => None,
    }
}

fn evaluate_tick<R: Runtime>(
    app: &AppHandle<R>,
    manager: &PtyManager,
    rules: &CompiledRules,
    tail_lines: usize,
    done_linger: Duration,
    dirty: &HashSet<u32>,
    trackers: &mut HashMap<u32, Tracker>,
) {
    // 1. Classify dirty, still-running sessions.
    for &id in dirty {
        let Some(snapshot) = manager.agent_status_snapshot(id) else {
            continue; // vanished
        };
        if snapshot.state != crate::session::SessionState::Running {
            continue; // semantic status is only for live PTYs (spec §2.2)
        }
        let (rule_set, ruleset) = select_ruleset(rules, snapshot.name.as_deref(), || {
            snapshot.foreground_pid.and_then(crate::pty::process_name)
        });
        let (raw, matched) = match ruleset {
            Some(set) => classify(&reconstruct_tail(&snapshot, tail_lines), set),
            None => (AgentStatus::Unknown, None),
        };
        let tracker = trackers.entry(id).or_default();
        tracker.rule_set = rule_set.clone();
        if let Some(status) = tracker.observe(raw, done_linger) {
            // Phase 4.4.2: mirror blocked/done edges to out-of-app notifications
            // (clone what `emit` is about to consume). A no-op when notifications
            // are disabled or no channel subscribes to this event.
            if let Some(event) = notify_event_for(status) {
                crate::notifications::dispatch(app, event, id, snapshot.name.clone(), matched.clone());
            }
            emit(app, id, status, matched, rule_set);
        }
    }

    // 2. Linger decay + cleanup for tracked sessions (runs even without new
    //    output). Exited/removed sessions drop their tracker WITHOUT an emit:
    //    the frontend clears `ui.agentStatus[id]` on `session-state: exited`
    //    (spec §7.2), so no dedicated clear event is needed.
    let mut drop_ids: Vec<u32> = Vec::new();
    for (&id, tracker) in trackers.iter_mut() {
        match manager.session_state(id) {
            Some(crate::session::SessionState::Running) => {
                if let Some(status) = tracker.tick_linger(done_linger) {
                    emit(app, id, status, None, tracker.rule_set.clone());
                }
            }
            _ => drop_ids.push(id),
        }
    }
    for id in drop_ids {
        trackers.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(user: Option<&AgentStatusConfig>) -> CompiledRules {
        build_rules(user)
    }

    fn parse(yaml: &str) -> AgentStatusConfig {
        crate::config::parse_config(yaml)
            .unwrap()
            .agent_status
            .unwrap()
    }

    // ----- built-in defaults integrity -----

    #[test]
    fn builtin_defaults_compile_and_cover_expected_keys() {
        let compiled = rules(None);
        for key in ["claude", "codex", "grok", "aider"] {
            assert!(compiled.get(key).is_some(), "missing built-in key {key}");
        }
        // No generic key ships by default (opt-in only).
        assert!(compiled.get("*").is_none());
        // Every built-in pattern compiled (no silent drops).
        for (key, raw) in &DEFAULT_RAW.patterns {
            let set = compiled.get(key).unwrap();
            assert_eq!(set.blocked.len(), raw.blocked.len(), "{key}.blocked");
            assert_eq!(set.working.len(), raw.working.len(), "{key}.working");
            assert_eq!(set.done.len(), raw.done.len(), "{key}.done");
        }
    }

    // ----- classify decision order + conservatism -----

    #[test]
    fn classify_prefers_blocked_over_working_over_done() {
        let compiled = rules(None);
        let claude = compiled.get("claude").unwrap();
        // claude: blocked wins over working (its done list is empty).
        let text = "esc to interrupt\nDo you want to proceed?";
        let (status, matched) = classify(text, claude);
        assert_eq!(status, AgentStatus::Blocked);
        assert!(matched.unwrap().contains("proceed"));

        // aider has all three categories; use it for working>done and done-alone.
        let aider = compiled.get("aider").unwrap();
        // working beats done.
        let (status, _) = classify(
            "Tokens: 2.3k sent, 391 received.\nWaiting for gpt-4o",
            aider,
        );
        assert_eq!(status, AgentStatus::Working);
        // done alone.
        let (status, _) = classify("Tokens: 2.3k sent, 391 received.", aider);
        assert_eq!(status, AgentStatus::Done);
    }

    #[test]
    fn classify_is_conservative_about_blocked() {
        let compiled = rules(None);
        let claude = compiled.get("claude").unwrap();
        // Unknown prompt, bare shell return, and empty output must be idle,
        // never blocked (blocked-conservatism, spec §2.3).
        for text in ["some random log line", "$ ", "user@host:~$ ", ""] {
            let (status, matched) = classify(text, claude);
            assert_eq!(status, AgentStatus::Idle, "text {text:?} should be idle");
            assert!(matched.is_none());
        }
    }

    #[test]
    fn classify_case_insensitive_and_multiline_by_default() {
        let compiled = rules(None);
        let claude = compiled.get("claude").unwrap();
        // Case-insensitive: uppercase still matches the lowercase built-in pattern.
        let (status, _) = classify("line one\nESC TO INTERRUPT\nline three", claude);
        assert_eq!(status, AgentStatus::Working);
        // Multiline: a `^`-anchored pattern matches a non-first line.
        let aider = compiled.get("aider").unwrap();
        let (status, _) = classify("some log line\nTokens: 5 sent, 6 received.", aider);
        assert_eq!(status, AgentStatus::Done);
    }

    // ----- ruleset selection -----

    #[test]
    fn select_ruleset_precedence_name_then_fg_then_generic_then_none() {
        let user = parse("agent_status:\n  patterns:\n    \"*\":\n      blocked:\n        - 'GENERIC'\n");
        let compiled = rules(Some(&user));

        // 1. definition name wins (no fg resolution needed).
        let (key, set) = select_ruleset(&compiled, Some("claude"), || {
            panic!("fg must not be resolved when name matches")
        });
        assert_eq!(key.as_deref(), Some("claude"));
        assert!(set.is_some());

        // 2. foreground process name when name misses.
        let (key, _) = select_ruleset(&compiled, None, || Some("codex".to_string()));
        assert_eq!(key.as_deref(), Some("codex"));

        // 3. opt-in generic "*" when neither matches.
        let (key, _) = select_ruleset(&compiled, Some("nope"), || Some("also-nope".to_string()));
        assert_eq!(key.as_deref(), Some("*"));

        // 4. None -> unknown when there is no "*" ruleset.
        let plain = rules(None);
        let (key, set) = select_ruleset(&plain, None, || None);
        assert!(key.is_none() && set.is_none());
    }

    // ----- merge / replace / invalid-skip -----

    #[test]
    fn merge_appends_to_builtin_and_replace_discards() {
        let base = rules(None);
        let base_claude_blocked = base.get("claude").unwrap().blocked.len();

        let user = parse(
            "agent_status:\n  patterns:\n    claude:\n      blocked:\n        - 'MY_EXTRA_PROMPT'\n    codex:\n      replace: true\n      blocked:\n        - 'ONLY_THIS'\n",
        );
        let compiled = rules(Some(&user));

        // merge: built-in + user, user pattern present.
        let claude = compiled.get("claude").unwrap();
        assert_eq!(claude.blocked.len(), base_claude_blocked + 1);
        assert!(claude.blocked.iter().any(|r| r.source == "MY_EXTRA_PROMPT"));

        // replace: only the user's pattern, built-in codex discarded; omitted
        // categories become empty.
        let codex = compiled.get("codex").unwrap();
        assert_eq!(codex.blocked.len(), 1);
        assert_eq!(codex.blocked[0].source, "ONLY_THIS");
        assert!(codex.working.is_empty());
        assert!(codex.done.is_empty());
    }

    #[test]
    fn invalid_regex_is_skipped_and_the_rest_survive() {
        let user = parse(
            "agent_status:\n  patterns:\n    my-agent:\n      blocked:\n        - 'good one'\n        - '(unterminated'\n        - 'another good'\n",
        );
        let compiled = rules(Some(&user));
        let set = compiled.get("my-agent").unwrap();
        // The one bad pattern is dropped; the two good ones remain active.
        assert_eq!(set.blocked.len(), 2);
        assert!(classify("here is a good one", set).0 == AgentStatus::Blocked);
    }

    // ----- tracker / done_linger -----

    #[test]
    fn tracker_working_then_prompt_return_becomes_done_then_idle() {
        let linger = Duration::from_millis(50);
        let mut t = Tracker::default();
        assert_eq!(t.observe(AgentStatus::Working, linger), Some(AgentStatus::Working));
        // working pattern gone (idle raw) -> done (completion edge).
        assert_eq!(t.observe(AgentStatus::Idle, linger), Some(AgentStatus::Done));
        // still idle within linger -> no change.
        assert_eq!(t.observe(AgentStatus::Idle, linger), None);
        // linger elapses -> decays to idle.
        std::thread::sleep(Duration::from_millis(70));
        assert_eq!(t.tick_linger(linger), Some(AgentStatus::Idle));
        // a lingering ✓ (done raw) while idle must NOT resurrect done.
        assert_eq!(t.observe(AgentStatus::Done, linger), None);
    }

    #[test]
    fn tracker_working_interrupts_done_linger() {
        let linger = Duration::from_secs(60);
        let mut t = Tracker::default();
        t.observe(AgentStatus::Working, linger);
        assert_eq!(t.observe(AgentStatus::Done, linger), Some(AgentStatus::Done));
        // New work arrives during linger -> straight back to working, done dropped.
        assert_eq!(t.observe(AgentStatus::Working, linger), Some(AgentStatus::Working));
        assert!(t.done_since.is_none());
        // Blocked also interrupts.
        assert_eq!(t.observe(AgentStatus::Blocked, linger), Some(AgentStatus::Blocked));
    }

    #[test]
    fn tracker_done_linger_zero_skips_done() {
        let zero = Duration::ZERO;
        let mut t = Tracker::default();
        t.observe(AgentStatus::Working, zero);
        // working -> idle with done disabled goes straight to idle.
        assert_eq!(t.observe(AgentStatus::Idle, zero), Some(AgentStatus::Idle));
        // explicit done match with done disabled also becomes idle.
        let mut t2 = Tracker::default();
        t2.observe(AgentStatus::Blocked, zero);
        assert_eq!(t2.observe(AgentStatus::Done, zero), Some(AgentStatus::Idle));
    }

    #[test]
    fn tracker_emits_only_on_change() {
        let linger = Duration::from_secs(60);
        let mut t = Tracker::default();
        assert_eq!(t.observe(AgentStatus::Blocked, linger), Some(AgentStatus::Blocked));
        assert_eq!(t.observe(AgentStatus::Blocked, linger), None);
        assert_eq!(t.observe(AgentStatus::Blocked, linger), None);
    }

    // ----- ANSI reconstruction integration -----

    #[test]
    fn spinner_bytes_reconstruct_then_classify_working() {
        // Raw TUI bytes with in-place spinner redraws: folded to the final
        // frame by render_terminal, then classified. Matching the raw stream
        // directly would be brittle; the reconstructed screen is stable.
        let raw = "\x1b[H\x1b[2K⠋ Thinking\x1b[H\x1b[2K⠙ Thinking (esc to interrupt)";
        let snapshot = crate::session::AgentStatusSnapshot {
            state: crate::session::SessionState::Running,
            kind: crate::session::SessionKind::Pty,
            name: Some("claude".to_string()),
            foreground_pid: None,
            output: raw.as_bytes().to_vec(),
            rows: 4,
            cols: 40,
        };
        let text = reconstruct_tail(&snapshot, 24);
        let compiled = rules(None);
        let (status, _) = classify(&text, compiled.get("claude").unwrap());
        assert_eq!(status, AgentStatus::Working);
    }
}
