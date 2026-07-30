// ptygrid.yml (legacy: mterm.yml) configuration: parsing (serde_norway), ${VAR} expansion,
// relative-cwd resolution, and the file watcher (notify) that emits
// `config-changed` events per the Phase 1 contract.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::pty::home_dir;

/// `Config { project?, agents, processes }` — processes defaults to empty Vec.
/// Phase 2 adds the optional `queen:` block; Phase 4.0 the `teammates:` block.
///
/// `Config::default()` is the **built-in default config** used as the no-config
/// fallback (see [`ConfigManager::load`]): `project: None`, empty `agents` /
/// `processes`, `queen: None` (Queen enabled with defaults), `teammates: None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    // `agents` is optional (M3): a config with only `queen:` / `processes:` /
    // `teammates:` is valid and defaults to an empty agent list.
    #[serde(default)]
    pub agents: Vec<AgentDef>,
    #[serde(default)]
    pub processes: Vec<AgentDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queen: Option<QueenConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teammates: Option<TeammatesConfig>,
    /// Phase 4.4.0 global `agent_status:` block (semantic-status detection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_status: Option<AgentStatusConfig>,
    /// Phase 4.4.2 global `notifications:` block (out-of-app alerting).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notifications: Option<NotificationsConfig>,
    /// Phase 4.3 top-level `team_presets:` block (named one-shot team launch).
    /// Validated at parse time (see [`validate_team_presets`]); unlike the
    /// other 4.x blocks a broken preset declaration FAILS the config load,
    /// because presets are launch declarations tied to the spawn allowlist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_presets: Option<BTreeMap<String, TeamPreset>>,
    /// Phase 5.0 top-level `workflows:` block (declarative DAG orchestration).
    /// Validated at parse time (see [`validate_workflows`]); a broken workflow
    /// declaration FAILS the config load, same as `team_presets:`. Members
    /// reference `agents:` definition names only (allowlist integrity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflows: Option<BTreeMap<String, WorkflowDef>>,
    /// Phase 5.5.0 top-level `mcp:` block (the `/mcp` RC-compat router).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpConfig>,
}

/// `queen: { enabled?: bool (default true), port?: u16 (default 39237) }`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct QueenConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

impl QueenConfig {
    pub fn effective_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    pub fn effective_port(&self) -> u16 {
        // `port: 0` would bind an arbitrary OS-assigned ephemeral port rather
        // than the documented default, so treat it like a missing value and
        // fall back to DEFAULT_PORT (L9).
        match self.port {
            Some(p) if p != 0 => p,
            _ => crate::queen::DEFAULT_PORT,
        }
    }
}

/// Phase 5.5.0 `mcp:` block — raw ptygrid.yml shape for the MCP RC-compat
/// router flags (spec-phase5-5.md §4.1). Resolution to plain values lives in
/// `queen_compat::config::McpCompatConfig` (the `QueenConfig` ->
/// `effective_*()` split, same pattern).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct McpConfig {
    /// Accept the 2026-07-28 RC route (`Mcp-Method` / `Mcp-Name` headers,
    /// no session id issuance). Default true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rc_2026_07_28: Option<bool>,
    /// Keep accepting the 2025-06 legacy route during the deprecation
    /// window. Default true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_2025_06: Option<bool>,
    /// Per-request body cap for the compat router, bytes. Default 1 MiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_body_bytes: Option<usize>,
    /// Deprecated-capability no-op policy (sampling/roots/logging). Default:
    /// sampling/roots off (real `-32601`), logging on (200 no-op).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_capabilities: Option<McpLegacyCapabilitiesConfig>,
}

impl McpConfig {
    pub fn effective_rc_2026_07_28(&self) -> bool {
        self.rc_2026_07_28.unwrap_or(true)
    }
    pub fn effective_legacy_2025_06(&self) -> bool {
        self.legacy_2025_06.unwrap_or(true)
    }
    pub fn effective_max_body_bytes(&self) -> usize {
        // 0 would reject every request; treat it like a missing value
        // (same posture as QueenConfig::effective_port for port 0).
        match self.max_body_bytes {
            Some(n) if n > 0 => n,
            _ => 1_048_576,
        }
    }
    pub fn effective_legacy_capabilities_sampling(&self) -> bool {
        self.legacy_capabilities
            .and_then(|c| c.sampling)
            .unwrap_or(false)
    }
    pub fn effective_legacy_capabilities_roots(&self) -> bool {
        self.legacy_capabilities
            .and_then(|c| c.roots)
            .unwrap_or(false)
    }
    pub fn effective_legacy_capabilities_logging(&self) -> bool {
        self.legacy_capabilities
            .and_then(|c| c.logging)
            .unwrap_or(true)
    }
}

/// `mcp.legacy_capabilities:` — per-capability opt-in to keep answering
/// `sampling/*`, `resources/roots`, `logging/setLevel` with a 200 no-op
/// instead of a real `-32601 method_not_found` during the 12-month
/// deprecation window (spec-phase5-5.md §3.1, §10 edge cases).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct McpLegacyCapabilitiesConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<bool>,
}

/// Phase 4.0 global `teammates:` block. Governs whether teammate hook events
/// are emitted/toasted and where `register_teammate_hooks` writes by default.
/// `agents[].teams` (per-agent teammate config) is Phase 4.1, not parsed here.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeammatesConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_notifications: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_max_panes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks_scope: Option<HooksScope>,
    /// argv0 basenames treated as teammate leads when a `claude` (or compatible)
    /// CLI is started by hand in a shell pane. Used by the Phase 4.1 implicit
    /// observe fallback (a foreground process match becomes a lead when no
    /// explicit `teams.enabled` named lead exists). Default `["claude"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teammate_binaries: Option<Vec<String>>,
}

impl TeammatesConfig {
    /// Default false: hook events are received (token still checked) but not
    /// emitted/toasted until the user opts in.
    pub fn effective_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }
    pub fn effective_hook_notifications(&self) -> bool {
        self.hook_notifications.unwrap_or(true)
    }
    /// Default 6, clamped into the 1..=9 pane range. Consumed by the Phase 4.1
    /// pane-limit logic.
    pub fn effective_global_max_panes(&self) -> u32 {
        self.global_max_panes.unwrap_or(6).clamp(1, 9)
    }
    pub fn effective_hooks_scope(&self) -> HooksScope {
        self.hooks_scope.unwrap_or_default()
    }
    /// argv0 basenames that count as an implicit observe lead when started by
    /// hand. Default `["claude"]`. Empty lists collapse to the default so a
    /// `teammate_binaries: []` never silently disables the fallback.
    pub fn effective_teammate_binaries(&self) -> Vec<String> {
        match &self.teammate_binaries {
            Some(list) if !list.is_empty() => list.clone(),
            _ => vec!["claude".to_string()],
        }
    }
}

/// Phase 4.4.2 global `notifications:` block. Routes the two edge-triggered
/// event sources — session lifecycle (`session::handle_eof`) and agent-status
/// changes (`agent_status::emit`) — to channels OUTSIDE the ptygrid window: the
/// desktop OS toast and chat webhooks (Slack / Mattermost / Discord / Telegram).
/// The routing model + dispatch live in [`crate::notifications`]; this struct
/// only carries the parsed values and the scalar defaults.
///
/// Opt-in: omitting the block, or `enabled: false`, sends nothing. Unknown keys
/// are ignored (forward compat), like the other 4.x blocks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Global default preset for channels that omit their own `level`. Default
    /// `critical` (errors / abnormal exits only) — the least-noisy useful level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<NotifyLevel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<ChannelConfig>,
}

impl NotificationsConfig {
    /// Default false: the feature is opt-in. No block / `enabled: false` sends
    /// nothing regardless of channels.
    pub fn effective_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }
    /// Global default level applied to any channel that omits its own `level`.
    /// Default `Critical` (errors + abnormal termination only).
    pub fn effective_level(&self) -> NotifyLevel {
        self.level.unwrap_or(NotifyLevel::Critical)
    }
}

/// Notification preset: a named bundle of event severities (see the design
/// matrix). Least → most noisy. `needs-attention` is the kebab wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NotifyLevel {
    /// Nothing is sent.
    Silent,
    /// Errors / abnormal termination only. The default.
    #[default]
    Critical,
    /// Critical + "needs attention" (a live agent blocked on approval / input /
    /// permission).
    NeedsAttention,
    /// Everything, including normal completion and progress.
    All,
}

/// Notification transport for one `notifications.channels` entry. `os` is the
/// local desktop toast; the rest are outbound chat webhooks. An unknown/mistyped
/// kind fails the field parse with a clear serde message (the block itself still
/// ignores unknown *keys*; only this closed enum is strict).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    #[default]
    Os,
    Slack,
    Mattermost,
    Discord,
    Telegram,
}

/// One entry under `notifications.channels`. `type` selects the transport; the
/// remaining fields are transport-specific and validated at DISPATCH, not parse,
/// so a half-filled channel (e.g. a `slack` entry missing its `webhook`) never
/// fails the whole config load — it is skipped with a warning at send time.
///
/// `webhook` / `bot_token` / `chat_id` are stored verbatim and `${VAR}`-expanded
/// only when a message is actually sent (mirrors how `env` values are handled).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelConfig {
    #[serde(rename = "type")]
    pub kind: ChannelKind,
    /// Per-channel override of the global `level`. Omitted -> the channel uses
    /// `notifications.level` (see [`NotificationsConfig::effective_level`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<NotifyLevel>,
    /// Slack / Mattermost / Discord incoming-webhook URL. `${VAR}` expanded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook: Option<String>,
    /// Telegram Bot API token. `${VAR}` expanded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    /// Telegram destination chat id. `${VAR}` expanded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    /// Optional cosmetic label shown in the message prefix (e.g. to tell two
    /// Slack channels apart).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl ChannelConfig {
    /// The level in force for this channel: its own override, else the global
    /// default handed in by the caller.
    pub fn effective_level(&self, global: NotifyLevel) -> NotifyLevel {
        self.level.unwrap_or(global)
    }
}

/// Phase 4.4.0 global `agent_status:` block. Governs the semantic-status
/// detector (working/blocked/done/idle) that runs on top of live `running`
/// PTY sessions. Everything is optional; omitting the block leaves detection
/// enabled with built-in defaults. This is a **separate layer** from
/// `SessionState` (process liveness) and never changes it.
///
/// The pattern compilation + built-in-default merge lives in
/// [`crate::agent_status`]; this struct only carries the parsed user values and
/// the scalar defaults/clamps.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentStatusConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_lines: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debounce_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_linger_ms: Option<u64>,
    /// Ruleset overrides keyed by agent-definition name or foreground process
    /// name (plus the opt-in `"*"` generic key). Merged onto the built-in
    /// defaults by [`crate::agent_status`] (merge by default, `replace: true`
    /// discards the built-in ruleset for that key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patterns: Option<HashMap<String, AgentStatusPatternSet>>,
}

/// One ruleset override under `agent_status.patterns.<key>`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentStatusPatternSet {
    /// Default false (merge onto the built-in ruleset of the same key). `true`
    /// discards the built-in ruleset and uses only these patterns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<Vec<String>>,
}

impl AgentStatusConfig {
    /// Default true: detection + `agent-status` events run unless disabled.
    pub fn effective_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    /// Default 24, clamped into 4..=200. Reconstructed-tail line count fed to
    /// the classifier.
    pub fn effective_tail_lines(&self) -> usize {
        self.tail_lines.unwrap_or(24).clamp(4, 200) as usize
    }
    /// Default 250ms, clamped into 100..=2000. Evaluation debounce interval.
    pub fn effective_debounce_ms(&self) -> u64 {
        self.debounce_ms.unwrap_or(250).clamp(100, 2000)
    }
    /// Default 6000ms, clamped into 0..=60000. How long `done` is held before
    /// decaying to `idle`; `0` disables `done` (transitions go straight to idle).
    pub fn effective_done_linger_ms(&self) -> u64 {
        self.done_linger_ms.unwrap_or(6000).clamp(0, 60000)
    }
}

/// Where `register_teammate_hooks` writes the Claude Code hooks by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HooksScope {
    #[default]
    User,
    Project,
}

impl HooksScope {
    pub fn as_str(self) -> &'static str {
        match self {
            HooksScope::User => "user",
            HooksScope::Project => "project",
        }
    }
}

/// Phase 4.1/4.2 per-agent `teams:` block. Governs whether this lead's
/// teammates / subagents get panes auto-generated. In `observe` mode a
/// read-only transcript pane is created on `SubagentStart` (Phase 4.1). In
/// `host` mode (Phase 4.2) the lead is started with the tmux shim + a per-lead
/// socket server so split-pane teammates are hosted as real interactive PTY
/// panes; `teammate_binaries` and `fallback_to_observe` apply only to host.
/// Everything is optional; omitting the block leaves the agent unchanged.
///
/// Not `Copy`: `teammate_binaries` carries an owned `Vec<String>`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentTeamsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<TeamsMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_panes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_tail: Option<bool>,
    /// Host mode only: argv0 basenames allowed to be spawned as split-window
    /// teammates. Default `["claude"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teammate_binaries: Option<Vec<String>>,
    /// Host mode only: fall back to a read-only observe transcript pane when a
    /// teammate is detected via hook but the shim never drives a spawn (the
    /// #6447-style breakage). Default true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_to_observe: Option<bool>,
}

impl AgentTeamsConfig {
    /// Default false: this lead does not produce teammate panes.
    pub fn effective_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }
    /// Default `observe`. Phase 4.2 makes `host` a real behavior branch (see
    /// [`crate::teams_host`]): a host lead is spawned with the tmux shim and a
    /// per-lead socket server, and hosts split-pane teammates as real PTYs.
    pub fn effective_mode(&self) -> TeamsMode {
        self.mode.unwrap_or_default()
    }
    /// Default 3, clamped into the 1..=9 pane range.
    pub fn effective_max_panes(&self) -> u32 {
        self.max_panes.unwrap_or(3).clamp(1, 9)
    }
    /// Default true: create a read-only transcript pane. When false the lead's
    /// subagents only surface as lifecycle events / status, no pane.
    pub fn effective_transcript_tail(&self) -> bool {
        self.transcript_tail.unwrap_or(true)
    }
    /// Host mode only. Default `["claude"]`. Empty lists collapse to the
    /// default so a `teammate_binaries: []` never disables all spawns silently.
    pub fn effective_teammate_binaries(&self) -> Vec<String> {
        match &self.teammate_binaries {
            Some(list) if !list.is_empty() => list.clone(),
            _ => vec!["claude".to_string()],
        }
    }
    /// Host mode only. Default true.
    pub fn effective_fallback_to_observe(&self) -> bool {
        self.fallback_to_observe.unwrap_or(true)
    }
    /// Whether this lead should run the Phase 4.2 real-PTY host path.
    pub fn is_host(&self) -> bool {
        self.effective_enabled() && self.effective_mode() == TeamsMode::Host
    }
}

/// `observe | host` (default observe). Phase 4.2 makes `host` a real behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TeamsMode {
    #[default]
    Observe,
    Host,
}

/// Phase 4.3: one named team preset under `team_presets:`. Members reference
/// `agents:` definitions by name only (allowlist integrity — the preset can
/// never launch anything `spawn_agent` could not). Validation rules live in
/// [`validate_team_presets`] and run at parse time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamPreset {
    /// Kickoff recipient. Must name a non-standby member. Omitted -> the
    /// first non-standby member ([`TeamPreset::effective_lead`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    #[serde(default)]
    pub members: Vec<TeamMember>,
    /// Optional first message, delivered to the effective lead's inbox after
    /// the non-standby members have been launched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kickoff: Option<String>,
}

impl TeamPreset {
    /// The kickoff recipient: the explicit `lead`, else the first non-standby
    /// member. `None` only for invalid presets (validation rejects those).
    pub fn effective_lead(&self) -> Option<&str> {
        match self.lead.as_deref() {
            Some(lead) => Some(lead),
            None => self
                .members
                .iter()
                .find(|m| !m.effective_standby())
                .map(|m| m.agent.as_str()),
        }
    }
}

/// One entry under `team_presets.<name>.members`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    /// Reference to an `agents:` definition name (never `processes:`).
    pub agent: String,
    /// Default false. `true` declares the member without launching it at team
    /// start; it is spawned later on demand (`spawn_agent` / UI) when needed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standby: Option<bool>,
    /// Optional role instructions, delivered to the member's inbox mailbox
    /// (= definition name) when the team is activated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

impl TeamMember {
    pub fn effective_standby(&self) -> bool {
        self.standby.unwrap_or(false)
    }
}

/// Phase 5.0: DAG orchestration pattern. `pipeline` and `fan-out` were the MVO
/// bootstrap (5.0.0); `supervisor` and `handoff` became executable in 5.0.4 and
/// all four now run. Two distinct things key off this value:
///
/// - SHAPE VALIDATION in `validate_workflows` below: `supervisor` demands
///   exactly one root that every other step depends on, `handoff` demands a
///   single linear chain, `fan-out` demands at least one step with `fanOut`
///   (anywhere in the graph — a dependent satisfies it, not just a root), and
///   `pipeline` forbids `fanOut` entirely.
/// - COPY EXPANSION at runtime, in `orchestrator::copies_for`, which is the
///   only place the orchestrator matches on the pattern: `fan-out` expands a
///   step into `fanOut` parallel sessions, every other pattern yields exactly
///   one. Everything else in the driver — dependency readiness, joins,
///   `condition`, `handoffTo`, retry, timeouts — is driven purely by the step
///   graph and is pattern-agnostic.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowPattern {
    #[default]
    Pipeline,
    FanOut,
    Supervisor,
    Handoff,
}

/// Phase 5.0: workflow-level failure policy. `fail-fast` cancels remaining
/// steps on the first FAILED step; `continue` keeps independent branches
/// running. Default: fail-fast (documented in spec-phase5-0.md §2.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum OnFailure {
    #[default]
    FailFast,
    Continue,
}

/// Phase 5.0: fan-out join rule, consumed by `orchestrator::dep_satisfied`.
/// `all` needs every parallel copy to SUCCEED; `any` is satisfied by the first
/// success; numeric `n` by the n-th; `reply` completes the step itself on an
/// inbox reply to its own kickoff thread. MVO (5.0.0) shipped all/any only;
/// `n` and `reply` became executable in 5.0.4 and all four now run. Untagged
/// serde so YAML `joinOn: all` and `joinOn: 3` both work.
///
/// NOTE, correcting the pre-5.0.4 wording here: `any` and `n` do NOT cancel the
/// copies that have not finished. The join only decides when DEPENDENTS may
/// spawn; the losing copies keep running to their own natural end and the run
/// does not finalize until every copy is terminal (`orchestrator::all_terminal`).
/// Cooperative cancellation of in-flight siblings is still unimplemented —
/// `StepState::Cancelled` is written only by an explicit `cancel_workflow`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum JoinOn {
    Named(JoinOnName),
    Count(u32),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JoinOnName {
    All,
    Any,
    /// Complete when an `inbox` reply to the step's kickoff thread arrives,
    /// sent by the step's own agent. Implemented in 5.0.4
    /// (`orchestrator::detect_reply_completions`). Requires a non-empty
    /// `kickoff:` on the same step — validated below, because without a thread
    /// to reply on the step could never complete.
    Reply,
}

/// Phase 5.0: one workflow declaration. See docs/spec/spec-phase5-0.md §2.1.
/// Field naming is camelCase in YAML (aligned with existing `spawn_agent` and
/// `spawn_team` conventions) — `depends_on` here maps to `dependsOn` in YAML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDef {
    #[serde(default)]
    pub pattern: WorkflowPattern,
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_failure: Option<OnFailure>,
    /// Optional: fan-out workflow opens the Arena drawer on launch (Phase
    /// 5.0.5). Parses today; the frontend hookup lands in 5.0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arena: Option<bool>,
    /// Phase 5.0.0 追補: auto-close a step's pane a few seconds after that
    /// step reaches a terminal state — `success` closes only `succeeded`
    /// steps, `always` also closes `failed`/`cancelled` steps. Default
    /// `never` (opt-in, existing behavior unchanged). Parsed only; the
    /// frontend evaluates and performs the close (mirrors `close_on_exit`
    /// above). Wire: `autoClose` (WorkflowDef is `rename_all = "camelCase"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_close: Option<AutoCloseMode>,
}

/// Phase 5.0.4: per-step retry policy. `max` bounds the number of restart
/// attempts after the first failed attempt; `backoff_ms` (default 0 when
/// omitted) is the delay before the next attempt spawns. See
/// `orchestrator::retry` for the pure attempts/backoff calculation and
/// `orchestrator::arm_retry_backoff`/`orchestrator::fire_due_retries` for the
/// driver wiring.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetryPolicy {
    pub max: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_ms: Option<u64>,
}

/// One entry under `workflows.<name>.steps`. Members reference `agents:`
/// definition names only (allowlist integrity — same as team_presets).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    /// Step identifier — unique within the workflow's `steps` list.
    pub id: String,
    /// Reference to an `agents:` definition name (never `processes:`).
    pub agent: String,
    /// Predecessor step ids. Empty/None means "root of the DAG".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
    /// Parallel spawn count (fan-out pattern only). Must be >= 2 when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_out: Option<u32>,
    /// Fan-out completion rule. Defaults to `all` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_on: Option<JoinOn>,
    /// Per-step upper time bound in milliseconds (5.0.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Optional first message delivered to the spawned pane's inbox mailbox
    /// (= definition name) — the same durable inbox path team_presets uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kickoff: Option<String>,
    /// Phase 5.0.4: restart policy applied when this step's spawn attempt
    /// fails (route1/route2/timeout). `None` means no retry — a single
    /// failed attempt is terminal, same as pre-5.0.4 behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
    /// Phase 5.0.4: regex evaluated once against the single dependency's
    /// `reply_body`, at the moment that dependency becomes satisfied and before
    /// this step is spawned (`orchestrator::condition_targets`). Three outcomes,
    /// and the third is the one to read carefully:
    ///
    /// - Match => the step spawns normally.
    /// - No match => `StepState::Skipped`. A cleanly declined branch, NOT a
    ///   failure: `Skipped` is neutral for run finalization, so the run can
    ///   still report `Succeeded`.
    /// - Dependency completed with NO reply at all — it produced no
    ///   `reply_body`, either because it declares no `kickoff:` to reply on or
    ///   because it declares one its agent never answered before completing via
    ///   route 1 (PTY exit) or route 2 (semantic `done`)
    ///   => `StepState::Failed`, not `Skipped`. There is nothing to match
    ///   against, so treating it as a decline would report the run green while
    ///   having silently dropped a branch the operator's config could never
    ///   have gated. This is the common mistake: a `condition:` is only
    ///   meaningful when its dependency produces a reply.
    ///
    /// Requires exactly one `dependsOn` entry; rejected on a `fanOut` step or
    /// a step depending on one (validated in `validate_workflows`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Phase 5.0.4 (handoff pattern): the next step in the chain. When this
    /// step completes via a durable inbox reply, that reply's body is
    /// prepended to the target step's `kickoff` (see `orchestrator::handoff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_to: Option<String>,
}

/// Longest workflow name whose run mailbox still fits `queen_store`'s
/// `MAX_MAILBOX_BYTES` (128).
///
/// `orchestrator::workflow_mailbox` builds `"queen:workflow/{name}/{run_id}"`.
/// The fixed cost is 44 bytes: 15 for `"queen:workflow/"`, 1 for the `/`
/// separator, and 28 for a `new_run_id` (`"wfr_"` + 16 hex nanos + 8 hex
/// counter). 128 - 44 = 84 bytes left for the name.
///
/// Byte length, not chars: `MAX_MAILBOX_BYTES` bounds the encoded string, so a
/// multi-byte name is measured the same way the store measures it.
const WORKFLOW_NAME_MAX_BYTES: usize = 84;

/// Phase 5.0 parse-time validation of the `workflows:` block. Every error
/// names the offending workflow and step so multi-workflow configs stay
/// debuggable (same principle as `validate_team_presets`).
///
/// Rejected: empty workflow name, empty `steps`, duplicate step id, unknown
/// step id in `dependsOn`, DAG cycle, unknown `agent` reference (must be in
/// `agents:`), `fanOut < 2`, `fanOut` on a non-`fan-out` pattern, `fanOut`
/// missing on a `fan-out` pattern's roots, `pipeline` with a step that has
/// more than one `dependsOn` (pipelines are linear by construction), and a
/// workflow name too long for the run's inbox mailbox
/// (`WORKFLOW_NAME_MAX_BYTES`).
fn validate_workflows(config: &Config) -> Result<(), String> {
    let Some(workflows) = &config.workflows else {
        return Ok(());
    };
    for (name, wf) in workflows {
        let ctx = format!("workflows.{name}");
        if name.trim().is_empty() {
            return Err("workflows: workflow name must not be empty".to_string());
        }
        // Checked here rather than at `send_inbox` time so an over-long name
        // fails loudly at load instead of as an inscrutable per-step kickoff
        // failure once the run is already on the grid. See
        // `WORKFLOW_NAME_MAX_BYTES` for the arithmetic.
        if name.trim().len() > WORKFLOW_NAME_MAX_BYTES {
            return Err(format!(
                "{ctx}: workflow name is {} bytes; must be <= {WORKFLOW_NAME_MAX_BYTES} \
                 (the run's inbox mailbox is 'queen:workflow/<name>/<run_id>', whose \
                 44 bytes of fixed cost — 15 for the prefix, 1 separator, 28 for the \
                 run id — leave that much of queen_store's 128-byte mailbox budget)",
                name.trim().len()
            ));
        }
        if wf.steps.is_empty() {
            return Err(format!("{ctx}: steps must not be empty"));
        }
        // Step id uniqueness + agent allowlist.
        let mut ids: Vec<&str> = Vec::with_capacity(wf.steps.len());
        for step in &wf.steps {
            if step.id.trim().is_empty() {
                return Err(format!("{ctx}: step id must not be empty"));
            }
            if ids.contains(&step.id.as_str()) {
                return Err(format!(
                    "{ctx}: step id '{}' is declared more than once",
                    step.id
                ));
            }
            ids.push(step.id.as_str());
            if !config.agents.iter().any(|a| a.name == step.agent) {
                return Err(format!(
                    "{ctx}: step '{}' references agent '{}' not defined under agents:                      (processes: entries cannot be workflow steps)",
                    step.id, step.agent
                ));
            }
            // Phase 5.0.4 field-level checks (apply regardless of pattern).
            if let Some(retry) = step.retry {
                if retry.max < 1 || retry.max > 10 {
                    return Err(format!(
                        "{ctx}: step '{}' retry.max ({}) must be between 1 and 10",
                        step.id, retry.max
                    ));
                }
                if let Some(backoff) = retry.backoff_ms {
                    if backoff > 60_000 {
                        return Err(format!(
                            "{ctx}: step '{}' retry.backoffMs ({}) must be <= 60000",
                            step.id, backoff
                        ));
                    }
                }
            }
            if let Some(timeout) = step.timeout_ms {
                if !(100..=86_400_000).contains(&timeout) {
                    return Err(format!(
                        "{ctx}: step '{}' timeoutMs ({}) must be between 100 and 86400000",
                        step.id, timeout
                    ));
                }
            }
            if let Some(JoinOn::Count(n)) = step.join_on {
                // `fanOut` only multiplies copies under `pattern: fan-out`
                // (see the `copies_for` match in orchestrator.rs); every other
                // pattern always spawns exactly one copy per step regardless
                // of what `fanOut` says, so the bound must track that, not
                // the raw field.
                let max = match wf.pattern {
                    WorkflowPattern::FanOut => step.fan_out.unwrap_or(1),
                    WorkflowPattern::Pipeline
                    | WorkflowPattern::Supervisor
                    | WorkflowPattern::Handoff => 1,
                };
                if n < 1 || n > max {
                    return Err(format!(
                        "{ctx}: step '{}' joinOn ({}) must be between 1 and the pattern's effective copy count ({})",
                        step.id, n, max
                    ));
                }
            }
            if let Some(pattern) = &step.condition {
                if Regex::new(pattern).is_err() {
                    return Err(format!(
                        "{ctx}: step '{}' condition is not a valid regex: '{}'",
                        step.id, pattern
                    ));
                }
                let deps = step.depends_on.as_deref().unwrap_or(&[]);
                if deps.len() != 1 {
                    return Err(format!(
                        "{ctx}: step '{}' condition requires exactly one dependsOn (found {})",
                        step.id, deps.len()
                    ));
                }
                if step.fan_out.is_some() {
                    return Err(format!(
                        "{ctx}: step '{}' declares both fanOut and condition; not supported",
                        step.id
                    ));
                }
                if let Some(dep_step) = wf.steps.iter().find(|s| s.id == deps[0]) {
                    if dep_step.fan_out.is_some() {
                        return Err(format!(
                            "{ctx}: step '{}' condition depends on fan-out step '{}'; not supported",
                            step.id, dep_step.id
                        ));
                    }
                }
            }
            // Phase 5.0.4: `joinOn: reply` completes ONLY on an inbox reply to
            // the step's own kickoff thread (`orchestrator::
            // detect_reply_completions`). With no `kickoff:` there is no thread
            // to reply on, so the step would sit Running forever and the run
            // would never finalize. Reject it at parse time rather than ship a
            // config shape whose only possible outcome is a wedged run.
            if matches!(step.join_on, Some(JoinOn::Named(JoinOnName::Reply)))
                && !step
                    .kickoff
                    .as_deref()
                    .is_some_and(|k| !k.trim().is_empty())
            {
                return Err(format!(
                    "{ctx}: step '{}' declares joinOn: reply but has no kickoff; \
                     a reply join completes on a reply to its own kickoff thread, \
                     so without one the step can never complete",
                    step.id
                ));
            }
            if let Some(target) = &step.handoff_to {
                if target == &step.id {
                    return Err(format!(
                        "{ctx}: step '{}' handoffTo references itself",
                        step.id
                    ));
                }
                if !wf.steps.iter().any(|s| &s.id == target) {
                    return Err(format!(
                        "{ctx}: step '{}' handoffTo '{}' which is not a step id",
                        step.id, target
                    ));
                }
                // A fan-out SOURCE would leave `orchestrator::handoff_bodies`
                // choosing between N copies' `reply_body`s, and it resolves
                // first-writer-wins over `run.steps` order — so the carried
                // kickoff would depend on spawn ordering rather than on
                // anything the author wrote. Reject it, mirroring the same
                // rule already applied to `condition`.
                if step.fan_out.is_some() {
                    return Err(format!(
                        "{ctx}: step '{}' declares both fanOut and handoffTo; not supported \
                         (which copy's reply would be carried is undefined)",
                        step.id
                    ));
                }
                // `handoffTo` only does anything where the DAG makes the target
                // spawn AFTER the source. `orchestrator::spawn_ready` looks the
                // carried body up at the instant it spawns the target, and
                // `ready_steps` is driven purely by `dependsOn` — so without the
                // back-edge the target is spawnable immediately, usually on the
                // very first tick, long before the source has replied, and the
                // carry is silently dropped. Pre-5.0.4 only `pattern: handoff`
                // required this edge (in a stricter 1:1 form, enforced below);
                // under every other pattern `handoffTo` validated clean and then
                // did nothing, which is the worst of both worlds — the author
                // sees no error and gets no chaining.
                let target_depends_on_source = wf
                    .steps
                    .iter()
                    .find(|s| &s.id == target)
                    .and_then(|s| s.depends_on.as_deref())
                    .is_some_and(|deps| deps.iter().any(|dep| dep == &step.id));
                if !target_depends_on_source {
                    return Err(format!(
                        "{ctx}: step '{}' handoffTo '{}' but '{}' does not dependOn '{}'; \
                         the carried reply is only read when the target spawns, so without \
                         that edge the target can start first and the handoff is lost",
                        step.id, target, target, step.id
                    ));
                }
            }
        }
        // dependsOn references must be known step ids.
        for step in &wf.steps {
            for dep in step.depends_on.as_deref().unwrap_or(&[]) {
                if !ids.contains(&dep.as_str()) {
                    return Err(format!(
                        "{ctx}: step '{}' depends_on '{}' which is not a step id",
                        step.id, dep
                    ));
                }
                if dep == &step.id {
                    return Err(format!(
                        "{ctx}: step '{}' depends on itself",
                        step.id
                    ));
                }
            }
        }
        // DAG cycle detection via DFS from each node.
        detect_cycle(&ctx, &wf.steps)?;
        // Pattern-specific rules.
        match wf.pattern {
            WorkflowPattern::Pipeline => {
                for step in &wf.steps {
                    let deps = step.depends_on.as_deref().unwrap_or(&[]);
                    if deps.len() > 1 {
                        return Err(format!(
                            "{ctx}: pipeline step '{}' has {} dependencies;                              pipeline is linear (max 1 dependsOn per step)",
                            step.id, deps.len()
                        ));
                    }
                    if step.fan_out.is_some() {
                        return Err(format!(
                            "{ctx}: pipeline step '{}' declares fanOut;                              use pattern: fan-out instead",
                            step.id
                        ));
                    }
                }
            }
            WorkflowPattern::FanOut => {
                let has_any_fan_out = wf.steps.iter().any(|s| s.fan_out.is_some());
                if !has_any_fan_out {
                    return Err(format!(
                        "{ctx}: fan-out pattern requires at least one step with fanOut set"
                    ));
                }
                for step in &wf.steps {
                    if let Some(count) = step.fan_out {
                        if count < 2 {
                            return Err(format!(
                                "{ctx}: step '{}' fanOut ({}) must be >= 2",
                                step.id, count
                            ));
                        }
                    }
                }
            }
            WorkflowPattern::Supervisor => {
                // `copies_for` never expands a step under `supervisor` (only
                // `fan-out` multiplies copies), so a declared `fanOut` here is
                // silently ignored at runtime — a `joinOn: n` built against it
                // can then demand more successes than will ever spawn. Reject
                // outright rather than let the shape pass and wedge later.
                for step in &wf.steps {
                    if step.fan_out.is_some() {
                        return Err(format!(
                            "{ctx}: supervisor step '{}' declares fanOut; fanOut only has \
                             meaning under pattern: fan-out — express supervisor \
                             parallelism via sibling steps instead",
                            step.id
                        ));
                    }
                }
                let roots: Vec<&WorkflowStep> = wf
                    .steps
                    .iter()
                    .filter(|s| s.depends_on.as_deref().unwrap_or(&[]).is_empty())
                    .collect();
                if roots.len() != 1 {
                    return Err(format!(
                        "{ctx}: supervisor pattern requires exactly one root step (found {})",
                        roots.len()
                    ));
                }
                let root_id = roots[0].id.clone();
                for step in &wf.steps {
                    if step.id == root_id {
                        continue;
                    }
                    if !step
                        .depends_on
                        .as_deref()
                        .unwrap_or(&[])
                        .contains(&root_id)
                    {
                        return Err(format!(
                            "{ctx}: supervisor step '{}' must dependOn root step '{}'",
                            step.id, root_id
                        ));
                    }
                }
            }
            WorkflowPattern::Handoff => {
                // Handoff is a strict linear chain: `depends_on` still drives
                // the generic driver (`ready_steps`/`spawn_ready`), so it must
                // mirror the `handoff_to` chain 1:1 rather than being a second,
                // independent graph.
                for step in &wf.steps {
                    let deps = step.depends_on.as_deref().unwrap_or(&[]);
                    if deps.len() > 1 {
                        return Err(format!(
                            "{ctx}: handoff step '{}' has {} dependencies;                              handoff is linear (max 1 dependsOn per step)",
                            step.id, deps.len()
                        ));
                    }
                }
                // `copies_for` never expands a step under `handoff` (only
                // `fan-out` multiplies copies), so a declared `fanOut` here is
                // silently ignored at runtime. The field-level loop above
                // already rejects `fanOut` combined with `handoffTo`, but that
                // only constrains steps that themselves hand off — the chain's
                // *last* step has no `handoffTo` and would otherwise slip
                // through with an ignored `fanOut`. Reject it outright here,
                // mirroring the `supervisor` rule above.
                for step in &wf.steps {
                    if step.fan_out.is_some() {
                        return Err(format!(
                            "{ctx}: handoff step '{}' declares fanOut; fanOut only has \
                             meaning under pattern: fan-out — express handoff \
                             parallelism via a different pattern instead",
                            step.id
                        ));
                    }
                }
                // SUBSUMED as of 5.0.4, kept as a narrower assertion. The
                // back-edge requirement moved to the field-level loop above,
                // where it now applies under EVERY pattern and runs before this
                // match — so a missing back-edge already errors with the
                // specific message there and can no longer masquerade as a
                // second root here. What survives is the stricter *exactly one*
                // dependency form: the loop above accepts extra dependencies on
                // the target, and this block does not. Under `handoff` that
                // difference is itself already covered by the `deps.len() > 1`
                // check directly above, which makes this provably unreachable
                // today; it is retained because it is the only place the linear
                // 1:1 chain shape is stated outright.
                for step in &wf.steps {
                    if let Some(next) = &step.handoff_to {
                        let next_step = wf
                            .steps
                            .iter()
                            .find(|s| &s.id == next)
                            .expect("handoffTo target existence already validated above");
                        let next_deps = next_step.depends_on.as_deref().unwrap_or(&[]);
                        if next_deps.len() != 1 || next_deps[0] != step.id {
                            return Err(format!(
                                "{ctx}: handoff step '{}' handoffTo '{}' but '{}' does not dependOn '{}'",
                                step.id, next, next, step.id
                            ));
                        }
                    }
                }
                let roots: Vec<&WorkflowStep> = wf
                    .steps
                    .iter()
                    .filter(|s| s.depends_on.as_deref().unwrap_or(&[]).is_empty())
                    .collect();
                if roots.len() != 1 {
                    return Err(format!(
                        "{ctx}: handoff pattern requires exactly one root step (found {})",
                        roots.len()
                    ));
                }
                let mut visited: HashSet<&str> = HashSet::new();
                let mut cur = roots[0].id.as_str();
                loop {
                    if !visited.insert(cur) {
                        return Err(format!(
                            "{ctx}: handoff chain revisits step '{}' (cycle)",
                            cur
                        ));
                    }
                    let step = wf.steps.iter().find(|s| s.id == cur).expect(
                        "handoff chain walk only visits ids already known to be valid step ids",
                    );
                    match &step.handoff_to {
                        Some(next) => cur = next.as_str(),
                        None => break,
                    }
                }
                if visited.len() != wf.steps.len() {
                    return Err(format!(
                        "{ctx}: handoff chain covers {} of {} declared steps;                              every step must be part of the single root->terminal chain",
                        visited.len(),
                        wf.steps.len()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Kahn/DFS-style cycle detection over the DAG defined by `steps[].depends_on`.
/// Returns an error naming a member of the first detected cycle.
fn detect_cycle(ctx: &str, steps: &[WorkflowStep]) -> Result<(), String> {
    use std::collections::HashMap;
    #[derive(Copy, Clone, PartialEq)]
    enum Color { White, Gray, Black }
    let mut color: HashMap<&str, Color> =
        steps.iter().map(|s| (s.id.as_str(), Color::White)).collect();

    fn visit<'a>(
        node: &'a str,
        steps: &'a [WorkflowStep],
        color: &mut HashMap<&'a str, Color>,
    ) -> Result<(), &'a str> {
        match color.get(node).copied() {
            Some(Color::Gray) => return Err(node), // back edge -> cycle
            Some(Color::Black) => return Ok(()),
            _ => {}
        }
        color.insert(node, Color::Gray);
        if let Some(step) = steps.iter().find(|s| s.id == node) {
            for dep in step.depends_on.as_deref().unwrap_or(&[]) {
                visit(dep.as_str(), steps, color)?;
            }
        }
        color.insert(node, Color::Black);
        Ok(())
    }

    for step in steps {
        if visit(step.id.as_str(), steps, &mut color).is_err() {
            return Err(format!("{ctx}: dependency cycle detected involving step '{}'", step.id));
        }
    }
    Ok(())
}

/// Phase 4.3 parse-time validation of the `team_presets:` block. Every error
/// names the offending preset so multi-preset configs stay debuggable.
///
/// Rejected: empty preset name, empty `members`, zero non-standby members, a
/// member referencing a name not defined under `agents:` (`processes:` names
/// are deliberately NOT accepted — a team is a set of interactive agents), a
/// duplicate `agent` within one preset, and a `lead` that is missing from the
/// members or points at a standby member.
fn validate_team_presets(config: &Config) -> Result<(), String> {
    let Some(presets) = &config.team_presets else {
        return Ok(());
    };
    for (name, preset) in presets {
        let ctx = format!("team_presets.{name}");
        if name.trim().is_empty() {
            return Err("team_presets: preset name must not be empty".to_string());
        }
        if preset.members.is_empty() {
            return Err(format!("{ctx}: members must not be empty"));
        }
        let mut seen: Vec<&str> = Vec::new();
        for member in &preset.members {
            if seen.contains(&member.agent.as_str()) {
                return Err(format!(
                    "{ctx}: member '{}' is declared more than once",
                    member.agent
                ));
            }
            seen.push(member.agent.as_str());
            if !config.agents.iter().any(|a| a.name == member.agent) {
                return Err(format!(
                    "{ctx}: member '{}' is not defined under agents: \
                     (processes: entries cannot join a team preset)",
                    member.agent
                ));
            }
        }
        if !preset.members.iter().any(|m| !m.effective_standby()) {
            return Err(format!(
                "{ctx}: at least one member must not be standby"
            ));
        }
        if let Some(lead) = &preset.lead {
            match preset.members.iter().find(|m| &m.agent == lead) {
                None => {
                    return Err(format!("{ctx}: lead '{lead}' is not a member"));
                }
                Some(m) if m.effective_standby() => {
                    return Err(format!(
                        "{ctx}: lead '{lead}' must not be a standby member"
                    ));
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

/// One agent or process definition (processes use the same shape, no
/// `instructions` in practice but the field is simply optional).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub name: String,
    pub cmd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autostart: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autorestart: Option<AutoRestart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Optional command used for a Phase 3.4 logical resume. The normal
    /// `cmd` remains the fallback when this is omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeConfig>,
    /// Phase 4.1 per-agent teammate/observe config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teams: Option<AgentTeamsConfig>,
    /// Phase 5.0.0 追補: auto-close this pane a few seconds after the
    /// session exits — `success` only when the exit code is 0, `always`
    /// regardless. Default `never` (existing behavior unchanged). Parsed
    /// only; the close itself is scheduled/performed entirely by the
    /// frontend (see stores.svelte.ts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_on_exit: Option<AutoCloseMode>,
}

/// Optional per-definition linked-worktree isolation (Phase 3.3).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

impl WorktreeConfig {
    pub fn effective_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    pub fn effective_base(&self) -> &str {
        self.base.as_deref().unwrap_or("HEAD")
    }
}

/// `never | on-failure | always` (default never).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AutoRestart {
    #[default]
    Never,
    OnFailure,
    Always,
}

/// `success | always | never` (default `never`). Phase 5.0.0 追補（終了ペイン
/// の自動クローズ）: shared by `AgentDef.close_on_exit` (session exit) and
/// `WorkflowDef.auto_close` (workflow step completion, wire `autoClose`).
/// Parsed here only — deciding WHEN to close and performing the close is
/// entirely a frontend concern (stores.svelte.ts `shouldAutoClose` /
/// `scheduleAutoClose`), mirroring how `autorestart` is parsed here but
/// enacted in session.rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AutoCloseMode {
    #[default]
    Never,
    Success,
    Always,
}

/// Where the config file that was actually loaded came from, relative to the
/// working folder passed to `load_config`:
/// - `Project`: inside the working folder (`<work>/ptygrid.yml` or legacy `mterm.yml`)
/// - `Launch`:  the app launch folder (`<launch>/ptygrid.yml`)
/// - `Global`:  the per-user global config (`~/.ptygrid/ptygrid.yml`)
/// - `Default`: no config file was found in any of the three locations and the
///   built-in default config was used ([`Config::default`]); the `path` reported
///   alongside is the first candidate `<work>/ptygrid.yml` that *would* have been
///   read, so a later-created file there is detected by the watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigOrigin {
    Project,
    Launch,
    Global,
    Default,
}

/// Return type of the `load_config` command.
///
/// `path` is the config file that was actually read; `dir` is the **working
/// folder** (the project boundary — cwd/Queen/Git/project-state base), which is
/// independent of where the config file lives; `origin` names which of the
/// three search locations `path` came from.
///
/// `trusted` (security Finding S2, additive) reports whether this config may be
/// used to *automatically* run commands (autostart / `worktree.setup`). It is
/// always true for `origin` `Global` (`~/.ptygrid`) and `Default` (the built-in
/// config); for `Project`/`Launch` it is true only when the working folder has
/// been explicitly trusted (see [`crate::trust`]). Loading always succeeds; the
/// frontend uses this flag to gate the autostart loop, not to block viewing the
/// config or manual, user-initiated launches.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigInfo {
    pub path: String,
    pub dir: String,
    pub origin: ConfigOrigin,
    pub trusted: bool,
    pub config: Config,
}

/// Parse ptygrid.yml text. Errors are passed through as strings (serde_norway
/// messages include line/column information). Phase 4.3: a present
/// `team_presets:` block is validated here too, so every load path (file,
/// reload, tests) gets the same guarantees.
pub fn parse_config(text: &str) -> Result<Config, String> {
    let config: Config = serde_norway::from_str(text).map_err(|e| e.to_string())?;
    validate_team_presets(&config)?;
    validate_workflows(&config)?;
    Ok(config)
}

/// Expand `${VAR}` occurrences in a value using the host environment.
/// A missing variable expands to the empty string.
pub fn expand_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find('}') {
            Some(end) => {
                let var = &rest[start + 2..start + 2 + end];
                out.push_str(&std::env::var(var).unwrap_or_default());
                rest = &rest[start + 2 + end + 1..];
            }
            None => {
                // Unterminated "${" — keep literally.
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Resolve a definition's cwd against the directory containing the config file.
/// Relative paths are joined onto `base`; absolute paths win; None -> base.
pub fn resolve_cwd(base: &Path, cwd: Option<&str>) -> PathBuf {
    match cwd {
        None => base.to_path_buf(),
        Some(".") | Some("") => base.to_path_buf(),
        Some(c) => {
            let p = Path::new(c);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                base.join(p)
            }
        }
    }
}

/// Env map of a definition with all values `${VAR}`-expanded.
pub fn expanded_env(def: &AgentDef) -> Vec<(String, String)> {
    def.env
        .as_ref()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), expand_vars(v)))
                .collect()
        })
        .unwrap_or_default()
}

/// Process launch directory (the folder ptygrid was started from, e.g. where
/// `npm run tauri dev` was invoked). Captured once at the very start of `main`
/// — before `fix_path_env::fix()` or any Tauri setup — because later startup
/// steps could in principle change the process cwd. Used as the ② candidate in
/// config resolution. `None` if the cwd could not be read.
static LAUNCH_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Capture the current directory as the launch folder. Idempotent: only the
/// first call wins. Call as early as possible in `main`.
pub fn capture_launch_dir() {
    let _ = LAUNCH_DIR.set(std::env::current_dir().ok());
}

/// The captured launch folder, if any. Returns `None` before `capture_launch_dir`
/// has run (e.g. in unit tests, which inject the launch folder explicitly).
pub(crate) fn launch_dir() -> Option<PathBuf> {
    LAUNCH_DIR.get().cloned().flatten()
}

#[derive(Default)]
struct ConfigStateInner {
    dir: Option<PathBuf>,
    config: Option<Config>,
    /// Kept alive so the notify watcher keeps running; replaced on reload.
    watcher: Option<RecommendedWatcher>,
}

/// Managed Tauri state holding the loaded config, its directory and the
/// active config-file watcher.
pub struct ConfigManager {
    inner: Mutex<ConfigStateInner>,
}

impl ConfigManager {
    pub fn new() -> Self {
        ConfigManager {
            inner: Mutex::new(ConfigStateInner::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ConfigStateInner> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Implements the `load_config` command.
    ///
    /// `dir` is the **working folder** (the project boundary). A leading `~` is
    /// expanded to the home directory; the folder must exist and be a directory.
    /// When omitted, the previous working folder is reused (first time: the
    /// current dir). The config file itself is resolved separately (working
    /// folder → launch folder → `~/.ptygrid`; see [`resolve_config_path`]), so
    /// the working folder need not contain a config file. The config + working
    /// folder are stored and the file watcher is (re)started on the folder that
    /// holds the file that was actually loaded.
    ///
    /// When `allow_default` is true and no config file is found in any of the
    /// three search locations, the built-in default config ([`Config::default`])
    /// is used with `origin: Default` instead of erroring; the watcher is started
    /// on `<work>/ptygrid.yml` so a file the user creates there afterwards emits
    /// `config-changed`. When `allow_default` is false (the startup auto-load
    /// path), a missing config still yields the `not_found:` error so the
    /// frontend's startup fallback keeps its previous behavior.
    pub fn load(
        &self,
        app: &AppHandle,
        dir: Option<String>,
        allow_default: bool,
    ) -> Result<ConfigInfo, String> {
        let mut inner = self.lock();

        let dir_path = match dir {
            Some(d) => expand_working_dir(&d)?,
            None => match inner.dir.clone() {
                Some(prev) => prev,
                None => std::env::current_dir()
                    .map_err(|e| format!("cannot determine current dir: {e}"))?,
            },
        };

        // The working folder must exist and be a directory (clear error otherwise);
        // it is the project boundary regardless of where the config file lives.
        let meta = std::fs::metadata(&dir_path).map_err(|e| {
            format!(
                "working folder {} is not accessible: {e}",
                dir_path.display()
            )
        })?;
        if !meta.is_dir() {
            return Err(format!(
                "working folder {} is not a directory",
                dir_path.display()
            ));
        }

        let home = home_dir().map(PathBuf::from);
        let (path, origin) = resolve_config_source(
            &dir_path,
            launch_dir().as_deref(),
            home.as_deref(),
            allow_default,
        )?;

        // `origin == Default` means no file was found and the built-in default is
        // used; `path` is the `<work>/ptygrid.yml` we would have read (watched
        // below). Any other origin points at a real file to read + parse.
        let config = if origin == ConfigOrigin::Default {
            Config::default()
        } else {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("read failed: {e}"))?;
            parse_config(&text)?
        };

        // Watch the parent dir of the file that was ACTUALLY loaded. When a
        // launch-folder or global (~/.ptygrid) config is used, this watches that
        // folder — NOT the working folder — so edits to the loaded file are
        // detected. Replace any existing watcher (dropping the old one stops it
        // and ends its throttle thread via channel disconnect).
        let watch_dir = path.parent().unwrap_or(dir_path.as_path()).to_path_buf();
        let watcher = start_watcher(app.clone(), &watch_dir, &path)?;
        // Security Finding S2: is this config trusted for autostart /
        // worktree.setup? Global/Default are always trusted; project/launch
        // require an explicit trust decision for the working folder. Loading
        // itself is never blocked — only the frontend autostart loop is gated.
        let trusted = crate::trust::is_trusted(app, origin, &dir_path);
        let dir = dir_path.display().to_string();
        inner.dir = Some(dir_path);
        inner.config = Some(config.clone());
        inner.watcher = Some(watcher);

        Ok(ConfigInfo {
            path: path.display().to_string(),
            dir,
            origin,
            trusted,
            config,
        })
    }

    /// Inject a config + directory directly, bypassing file IO and the
    /// watcher. Test-only; `load` requires a concrete Wry `AppHandle`.
    #[cfg(test)]
    pub(crate) fn set_for_test(&self, dir: PathBuf, config: Config) {
        let mut inner = self.lock();
        inner.dir = Some(dir);
        inner.config = Some(config);
    }

    /// Current loaded config + its directory (Queen list_agents).
    pub fn current(&self) -> Option<(Config, PathBuf)> {
        let inner = self.lock();
        match (&inner.config, &inner.dir) {
            (Some(c), Some(d)) => Some((c.clone(), d.clone())),
            _ => None,
        }
    }

    /// Look up an agent (then process) definition by name, together with the
    /// config directory used for cwd resolution.
    pub fn resolve_def(&self, name: &str) -> Result<(AgentDef, PathBuf), String> {
        let inner = self.lock();
        let config = inner
            .config
            .as_ref()
            .ok_or_else(|| "no config loaded (call load_config first)".to_string())?;
        let def = config
            .agents
            .iter()
            .chain(config.processes.iter())
            .find(|d| d.name == name)
            .cloned()
            .ok_or_else(|| format!("agent or process '{name}' not found in config"))?;
        let dir = inner
            .dir
            .clone()
            .ok_or_else(|| "config dir missing".to_string())?;
        Ok((def, dir))
    }
}

/// Preferred config filename (since the multi-terminal -> ptygrid rename).
pub const CONFIG_FILE_NAME: &str = "ptygrid.yml";
/// Legacy filename, still accepted so existing projects keep loading.
pub const LEGACY_CONFIG_FILE_NAME: &str = "mterm.yml";
/// Directory (under `$HOME`) holding the per-user global config.
pub const GLOBAL_CONFIG_DIR: &str = ".ptygrid";

/// Expand a leading `~` / `~/` in a working-folder input to the home directory.
/// A `~name` form (named home) is not special-cased. Non-tilde paths pass
/// through unchanged. Mirrors `app_settings::expand_tilde`.
fn expand_working_dir(input: &str) -> Result<PathBuf, String> {
    if input == "~" {
        return home_dir()
            .map(PathBuf::from)
            .ok_or_else(|| "cannot determine home directory".to_string());
    }
    if let Some(rest) = input.strip_prefix("~/") {
        let home = home_dir().ok_or_else(|| "cannot determine home directory".to_string())?;
        return Ok(Path::new(&home).join(rest));
    }
    Ok(PathBuf::from(input))
}

/// Pure config-file resolution shared by [`resolve_config_path`] and unit
/// tests. Search order (first existing file wins):
///
/// 1. `<work>/ptygrid.yml`, then legacy `<work>/mterm.yml` (legacy fallback is
///    the **working folder only**) — origin `Project`.
/// 2. `<launch>/ptygrid.yml` (launch folder; skipped when it equals the working
///    folder to avoid a duplicate try) — origin `Launch`.
/// 3. `<home>/.ptygrid/ptygrid.yml` — origin `Global`.
///
/// `is_file` is injected so the order can be tested without touching the disk.
/// On failure returns the full ordered list of candidates that were tried.
pub(crate) fn resolve_config_path_pure(
    work: &Path,
    launch: Option<&Path>,
    home: Option<&Path>,
    is_file: &dyn Fn(&Path) -> bool,
) -> Result<(PathBuf, ConfigOrigin), Vec<PathBuf>> {
    let mut tried: Vec<PathBuf> = Vec::new();

    // ① working folder: ptygrid.yml, then legacy mterm.yml.
    let work_preferred = work.join(CONFIG_FILE_NAME);
    if is_file(&work_preferred) {
        return Ok((work_preferred, ConfigOrigin::Project));
    }
    tried.push(work_preferred);
    let work_legacy = work.join(LEGACY_CONFIG_FILE_NAME);
    if is_file(&work_legacy) {
        return Ok((work_legacy, ConfigOrigin::Project));
    }
    tried.push(work_legacy);

    // ② launch folder: ptygrid.yml only (no legacy). Skip when it is the same
    // path as the working folder (already tried above).
    if let Some(launch) = launch {
        if launch != work {
            let launch_preferred = launch.join(CONFIG_FILE_NAME);
            if is_file(&launch_preferred) {
                return Ok((launch_preferred, ConfigOrigin::Launch));
            }
            tried.push(launch_preferred);
        }
    }

    // ③ global ~/.ptygrid/ptygrid.yml.
    if let Some(home) = home {
        let global = home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME);
        if is_file(&global) {
            return Ok((global, ConfigOrigin::Global));
        }
        tried.push(global);
    }

    Err(tried)
}

/// Pure config-*source* resolution: [`resolve_config_path_pure`] plus the
/// built-in default fallback. When a file is found it is returned as-is; when
/// none is found and `allow_default` is true, `(<work>/ptygrid.yml, Default)` is
/// returned (the caller uses [`Config::default`] and watches that path); when
/// none is found and `allow_default` is false, the tried-candidate list is
/// returned as `Err` for the caller to format. `is_file` is injected for tests.
fn resolve_config_source_pure(
    work: &Path,
    launch: Option<&Path>,
    home: Option<&Path>,
    is_file: &dyn Fn(&Path) -> bool,
    allow_default: bool,
) -> Result<(PathBuf, ConfigOrigin), Vec<PathBuf>> {
    match resolve_config_path_pure(work, launch, home, is_file) {
        Ok(found) => Ok(found),
        Err(_) if allow_default => Ok((work.join(CONFIG_FILE_NAME), ConfigOrigin::Default)),
        Err(tried) => Err(tried),
    }
}

/// Resolve the config source for a `load` call using the real filesystem. See
/// [`resolve_config_source_pure`] for the search order and the default fallback.
/// On failure (no file and `allow_default` is false) the error begins with
/// `not_found:` (matched by the frontend startup fallback) and lists every
/// candidate that was tried.
fn resolve_config_source(
    work: &Path,
    launch: Option<&Path>,
    home: Option<&Path>,
    allow_default: bool,
) -> Result<(PathBuf, ConfigOrigin), String> {
    resolve_config_source_pure(work, launch, home, &|p| p.is_file(), allow_default).map_err(
        |tried| {
            let list = tried
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!("not_found: no ptygrid.yml found; tried {list}")
        },
    )
}

/// Watch the config directory (non-recursive) and emit `config-changed`
/// for events touching the loaded config file. Raw notify events are coalesced by a
/// 300ms thread-side throttle so one editor save emits a single event.
/// Watching the parent dir (not the file) keeps working across editors
/// that save via rename/replace.
fn start_watcher(
    app: AppHandle,
    dir: &Path,
    file: &Path,
) -> Result<RecommendedWatcher, String> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let file_name = file.file_name().map(|n| n.to_os_string());

    let mut watcher = notify::recommended_watcher(
        move |res: Result<notify::Event, notify::Error>| match res {
            Ok(event) => {
                let relevant = event
                    .paths
                    .iter()
                    .any(|p| p.file_name() == file_name.as_deref())
                    || event.paths.is_empty();
                if relevant {
                    let _ = tx.send(());
                }
            }
            // A watcher error (e.g. the watched directory was removed/renamed)
            // silently stops config reloads; surface it instead of dropping it (L8).
            Err(error) => eprintln!("config watcher error: {error}"),
        },
    )
    .map_err(|e| format!("watcher create failed: {e}"))?;

    watcher
        .watch(dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("watch failed: {e}"))?;

    let path_str = file.display().to_string();
    std::thread::spawn(move || {
        // recv() errors (sender dropped == watcher replaced/dropped) end the thread.
        while rx.recv().is_ok() {
            std::thread::sleep(Duration::from_millis(300));
            while rx.try_recv().is_ok() {} // drain the burst
            let _ = app.emit(
                "config-changed",
                serde_json::json!({ "path": path_str }),
            );
        }
    });

    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
project: my-app
agents:
  - name: claude
    cmd: "claude"
    cwd: "sub/dir"
    env:
      ANTHROPIC_API_KEY: "${MTERM_TEST_KEY}"
      MIXED: "pre-${MTERM_TEST_KEY}-post"
      MISSING: "${MTERM_TEST_DOES_NOT_EXIST_XYZ}"
    autostart: true
    autorestart: on-failure
    instructions: "be nice"
  - name: codex
    cmd: "codex --full-auto"
"#;

    #[test]
    fn parses_yaml_with_defaults() {
        let cfg = parse_config(SAMPLE).expect("parse failed");
        assert_eq!(cfg.project.as_deref(), Some("my-app"));
        assert_eq!(cfg.agents.len(), 2);
        // processes omitted -> empty Vec
        assert!(cfg.processes.is_empty());

        let claude = &cfg.agents[0];
        assert_eq!(claude.name, "claude");
        assert_eq!(claude.cmd, "claude");
        assert_eq!(claude.autostart, Some(true));
        assert_eq!(claude.autorestart, Some(AutoRestart::OnFailure));
        assert_eq!(claude.instructions.as_deref(), Some("be nice"));

        let codex = &cfg.agents[1];
        assert_eq!(codex.autostart, None);
        assert_eq!(codex.autorestart, None);
        assert_eq!(codex.cwd, None);
    }

    #[test]
    fn parses_autorestart_variants_and_processes() {
        let yaml = r#"
agents:
  - name: a
    cmd: "x"
    autorestart: always
  - name: b
    cmd: "y"
    autorestart: never
processes:
  - name: web
    cmd: "npm run dev"
    autostart: false
"#;
        let cfg = parse_config(yaml).unwrap();
        assert_eq!(cfg.agents[0].autorestart, Some(AutoRestart::Always));
        assert_eq!(cfg.agents[1].autorestart, Some(AutoRestart::Never));
        assert_eq!(cfg.processes.len(), 1);
        assert_eq!(cfg.processes[0].cmd, "npm run dev");
        assert_eq!(cfg.project, None);
    }

    #[test]
    fn parses_opt_in_worktree_config() {
        let yaml = r#"
agents:
  - name: isolated
    cmd: codex
    resume: codex resume --last
    worktree:
      enabled: true
      base: main
      setup: npm install
  - name: shared
    cmd: claude
"#;
        let cfg = parse_config(yaml).unwrap();
        let isolated = cfg.agents[0].worktree.as_ref().unwrap();
        assert!(isolated.effective_enabled());
        assert_eq!(isolated.effective_base(), "main");
        assert_eq!(isolated.setup.as_deref(), Some("npm install"));
        assert_eq!(cfg.agents[0].resume.as_deref(), Some("codex resume --last"));
        assert!(cfg.agents[1].worktree.is_none());

        let defaults = WorktreeConfig::default();
        assert!(!defaults.effective_enabled());
        assert_eq!(defaults.effective_base(), "HEAD");
    }

    #[test]
    fn parse_error_is_string_with_location() {
        let err = parse_config("agents: [ { name: x } ]").unwrap_err();
        // missing `cmd` -> serde error mentioning the field, with location info
        assert!(err.contains("cmd"), "error was: {err}");
    }

    #[test]
    fn expands_vars_from_host_env() {
        std::env::set_var("MTERM_TEST_KEY", "sekrit");
        assert_eq!(expand_vars("${MTERM_TEST_KEY}"), "sekrit");
        assert_eq!(expand_vars("a-${MTERM_TEST_KEY}-b"), "a-sekrit-b");
        // missing var -> empty string
        assert_eq!(expand_vars("x${MTERM_TEST_DOES_NOT_EXIST_XYZ}y"), "xy");
        // no markers -> unchanged; unterminated -> literal
        assert_eq!(expand_vars("plain $HOME text"), "plain $HOME text");
        assert_eq!(expand_vars("oops ${UNTERMINATED"), "oops ${UNTERMINATED");
    }

    #[test]
    fn expanded_env_expands_all_values() {
        std::env::set_var("MTERM_TEST_KEY", "sekrit");
        let cfg = parse_config(SAMPLE).unwrap();
        let env = expanded_env(&cfg.agents[0]);
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("ANTHROPIC_API_KEY"), "sekrit");
        assert_eq!(get("MIXED"), "pre-sekrit-post");
        assert_eq!(get("MISSING"), "");
    }

    #[test]
    fn queen_block_defaults_and_overrides() {
        // No queen block at all
        let cfg = parse_config("agents: []").unwrap();
        assert!(cfg.queen.is_none());

        // Empty queen block -> effective defaults true / 39237
        let cfg = parse_config("agents: []\nqueen: {}").unwrap();
        let q = cfg.queen.unwrap();
        assert_eq!(q.enabled, None);
        assert_eq!(q.port, None);
        assert!(q.effective_enabled());
        assert_eq!(q.effective_port(), 39237);

        // Explicit values
        let cfg = parse_config("agents: []\nqueen:\n  enabled: false\n  port: 40100").unwrap();
        let q = cfg.queen.unwrap();
        assert!(!q.effective_enabled());
        assert_eq!(q.effective_port(), 40100);

        // L9: `port: 0` is not an ephemeral-port request; fall back to default.
        let cfg = parse_config("agents: []\nqueen:\n  port: 0").unwrap();
        let q = cfg.queen.unwrap();
        assert_eq!(q.port, Some(0));
        assert_eq!(q.effective_port(), crate::queen::DEFAULT_PORT);
    }

    #[test]
    fn agents_field_is_optional() {
        // M3: a config with only `queen:` (no `agents:`) must parse, with
        // `agents` defaulting to empty.
        let cfg = parse_config("queen: {}").unwrap();
        assert!(cfg.agents.is_empty());
        assert!(cfg.queen.is_some());

        // Only `processes:` is likewise valid without `agents:`.
        let cfg = parse_config("processes:\n  - name: web\n    cmd: npm run dev\n").unwrap();
        assert!(cfg.agents.is_empty());
        assert_eq!(cfg.processes.len(), 1);
    }

    #[test]
    fn teammates_block_defaults_and_clamp() {
        // No teammates block at all -> None; effective defaults on Default.
        let cfg = parse_config("agents: []").unwrap();
        assert!(cfg.teammates.is_none());
        let defaults = TeammatesConfig::default();
        assert!(!defaults.effective_enabled());
        assert!(defaults.effective_hook_notifications());
        assert_eq!(defaults.effective_global_max_panes(), 6);
        assert_eq!(defaults.effective_hooks_scope(), HooksScope::User);

        // Empty block -> same effective defaults.
        let cfg = parse_config("agents: []\nteammates: {}").unwrap();
        let t = cfg.teammates.unwrap();
        assert_eq!(t.enabled, None);
        assert!(!t.effective_enabled());
        assert!(t.effective_hook_notifications());
        assert_eq!(t.effective_global_max_panes(), 6);
        assert_eq!(t.effective_hooks_scope(), HooksScope::User);

        // Explicit values, including out-of-range panes that clamp to 1..=9.
        let cfg = parse_config(
            "agents: []\nteammates:\n  enabled: true\n  hook_notifications: false\n  global_max_panes: 42\n  hooks_scope: project",
        )
        .unwrap();
        let t = cfg.teammates.unwrap();
        assert!(t.effective_enabled());
        assert!(!t.effective_hook_notifications());
        assert_eq!(t.effective_global_max_panes(), 9);
        assert_eq!(t.effective_hooks_scope(), HooksScope::Project);

        // Below the range clamps up to 1.
        let cfg = parse_config("agents: []\nteammates:\n  global_max_panes: 0").unwrap();
        assert_eq!(cfg.teammates.unwrap().effective_global_max_panes(), 1);
    }

    #[test]
    fn teammates_block_ignores_unknown_fields() {
        // Unknown keys are ignored (no deny_unknown_fields), known ones parse.
        let cfg = parse_config(
            "agents: []\nteammates:\n  enabled: true\n  future_option: 123\n  nested:\n    a: b",
        )
        .unwrap();
        let t = cfg.teammates.unwrap();
        assert!(t.effective_enabled());
    }

    #[test]
    fn agent_teams_block_defaults_and_overrides() {
        // No teams block -> None; Default gives the documented effective values.
        let cfg = parse_config("agents:\n  - name: claude\n    cmd: claude\n").unwrap();
        assert!(cfg.agents[0].teams.is_none());
        let d = AgentTeamsConfig::default();
        assert!(!d.effective_enabled());
        assert_eq!(d.effective_mode(), TeamsMode::Observe);
        assert_eq!(d.effective_max_panes(), 3);
        assert!(d.effective_transcript_tail());

        // Empty block -> same effective defaults.
        let cfg =
            parse_config("agents:\n  - name: claude\n    cmd: claude\n    teams: {}\n").unwrap();
        let t = cfg.agents[0].teams.clone().unwrap();
        assert!(!t.effective_enabled());
        assert_eq!(t.effective_mode(), TeamsMode::Observe);
        assert_eq!(t.effective_max_panes(), 3);
        assert!(t.effective_transcript_tail());
        assert!(!t.is_host());

        // Explicit values incl. host mode and an out-of-range max_panes clamp.
        let cfg = parse_config(
            "agents:\n  - name: claude\n    cmd: claude\n    teams:\n      enabled: true\n      mode: host\n      max_panes: 99\n      transcript_tail: false\n",
        )
        .unwrap();
        let t = cfg.agents[0].teams.clone().unwrap();
        assert!(t.effective_enabled());
        assert_eq!(t.effective_mode(), TeamsMode::Host);
        assert_eq!(t.effective_max_panes(), 9);
        assert!(!t.effective_transcript_tail());
        assert!(t.is_host());
    }

    #[test]
    fn agent_teams_host_only_fields_default_and_parse() {
        // Defaults: allowlist is ["claude"], fallback_to_observe is true.
        let d = AgentTeamsConfig::default();
        assert_eq!(d.effective_teammate_binaries(), vec!["claude".to_string()]);
        assert!(d.effective_fallback_to_observe());

        // Explicit host fields parse.
        let cfg = parse_config(
            "agents:\n  - name: claude\n    cmd: claude\n    teams:\n      enabled: true\n      mode: host\n      teammate_binaries: [claude, claude-next]\n      fallback_to_observe: false\n",
        )
        .unwrap();
        let t = cfg.agents[0].teams.clone().unwrap();
        assert_eq!(
            t.effective_teammate_binaries(),
            vec!["claude".to_string(), "claude-next".to_string()]
        );
        assert!(!t.effective_fallback_to_observe());
        assert!(t.is_host());

        // An empty allowlist never disables all spawns: it collapses to default.
        let cfg = parse_config(
            "agents:\n  - name: claude\n    cmd: claude\n    teams:\n      teammate_binaries: []\n",
        )
        .unwrap();
        assert_eq!(
            cfg.agents[0].teams.clone().unwrap().effective_teammate_binaries(),
            vec!["claude".to_string()]
        );
    }

    #[test]
    fn is_host_requires_both_enabled_and_mode_host() {
        // enabled + host => host path.
        let host = parse_config(
            "agents:\n  - name: c\n    cmd: c\n    teams:\n      enabled: true\n      mode: host\n",
        )
        .unwrap();
        assert!(host.agents[0].teams.clone().unwrap().is_host());
        // host mode but disabled => not a host lead (opt-in gate).
        let disabled = parse_config(
            "agents:\n  - name: c\n    cmd: c\n    teams:\n      enabled: false\n      mode: host\n",
        )
        .unwrap();
        assert!(!disabled.agents[0].teams.clone().unwrap().is_host());
        // enabled but observe => not host.
        let observe = parse_config(
            "agents:\n  - name: c\n    cmd: c\n    teams:\n      enabled: true\n      mode: observe\n",
        )
        .unwrap();
        assert!(!observe.agents[0].teams.clone().unwrap().is_host());
    }

    #[test]
    fn agent_teams_block_ignores_unknown_fields() {
        // Unknown keys are ignored; known ones still parse.
        let cfg = parse_config(
            "agents:\n  - name: claude\n    cmd: claude\n    teams:\n      enabled: true\n      teammate_binaries: [claude]\n      future_flag: 7\n",
        )
        .unwrap();
        assert!(cfg.agents[0].teams.clone().unwrap().effective_enabled());
    }

    #[test]
    fn agent_status_block_defaults_and_clamp() {
        // No block at all -> None; effective defaults come from Default.
        let cfg = parse_config("agents: []").unwrap();
        assert!(cfg.agent_status.is_none());
        let d = AgentStatusConfig::default();
        assert!(d.effective_enabled()); // default TRUE (4.1)
        assert_eq!(d.effective_tail_lines(), 24);
        assert_eq!(d.effective_debounce_ms(), 250);
        assert_eq!(d.effective_done_linger_ms(), 6000);

        // Empty block -> same effective defaults.
        let cfg = parse_config("agents: []\nagent_status: {}").unwrap();
        let a = cfg.agent_status.unwrap();
        assert_eq!(a.enabled, None);
        assert!(a.effective_enabled());

        // Out-of-range values clamp; enabled: false is honored.
        let cfg = parse_config(
            "agents: []\nagent_status:\n  enabled: false\n  tail_lines: 9999\n  debounce_ms: 1\n  done_linger_ms: 999999\n",
        )
        .unwrap();
        let a = cfg.agent_status.unwrap();
        assert!(!a.effective_enabled());
        assert_eq!(a.effective_tail_lines(), 200);
        assert_eq!(a.effective_debounce_ms(), 100);
        assert_eq!(a.effective_done_linger_ms(), 60000);

        // Below-range clamps up.
        let cfg = parse_config("agents: []\nagent_status:\n  tail_lines: 0\n  debounce_ms: 5").unwrap();
        let a = cfg.agent_status.unwrap();
        assert_eq!(a.effective_tail_lines(), 4);
        assert_eq!(a.effective_debounce_ms(), 100);
    }

    #[test]
    fn agent_status_patterns_parse_with_merge_and_replace() {
        let cfg = parse_config(
            "agent_status:\n  patterns:\n    claude:\n      blocked:\n        - 'Do you want to proceed\\?'\n      working:\n        - 'esc to interrupt'\n    codex:\n      replace: true\n      blocked:\n        - '\\[y/N\\]'\n    \"*\":\n      blocked:\n        - '\\[y/N\\]\\s*$'\n",
        )
        .unwrap();
        let pats = cfg.agent_status.unwrap().patterns.unwrap();
        // merge is the default (replace unset -> None).
        assert_eq!(pats["claude"].replace, None);
        assert_eq!(pats["claude"].blocked.as_ref().unwrap().len(), 1);
        assert_eq!(pats["claude"].working.as_ref().unwrap().len(), 1);
        // replace: true is captured.
        assert_eq!(pats["codex"].replace, Some(true));
        // the opt-in generic "*" key parses like any other.
        assert!(pats.contains_key("*"));
    }

    #[test]
    fn agent_status_block_ignores_unknown_fields() {
        // Forward-compat: 4.4.2 fields (notify, etc.) are ignored today.
        let cfg = parse_config(
            "agent_status:\n  enabled: true\n  notify: true\n  notify_sound: false\n  renotify_ms: 5000\n",
        )
        .unwrap();
        assert!(cfg.agent_status.unwrap().effective_enabled());
    }

    // ---- notifications: block (Phase 4.4.2) ----

    #[test]
    fn notifications_block_defaults_and_overrides() {
        // No block at all -> None; Default gives the documented effective values.
        let cfg = parse_config("agents: []").unwrap();
        assert!(cfg.notifications.is_none());
        let d = NotificationsConfig::default();
        assert!(!d.effective_enabled()); // opt-in
        assert_eq!(d.effective_level(), NotifyLevel::Critical);
        assert!(d.channels.is_empty());

        // Empty block -> same effective defaults.
        let cfg = parse_config("notifications: {}").unwrap();
        let n = cfg.notifications.unwrap();
        assert_eq!(n.enabled, None);
        assert!(!n.effective_enabled());
        assert_eq!(n.effective_level(), NotifyLevel::Critical);

        // Explicit enabled + global level (kebab wire form parses).
        let cfg =
            parse_config("notifications:\n  enabled: true\n  level: needs-attention\n").unwrap();
        let n = cfg.notifications.unwrap();
        assert!(n.effective_enabled());
        assert_eq!(n.effective_level(), NotifyLevel::NeedsAttention);
    }

    #[test]
    fn notify_level_parses_all_kebab_variants() {
        for (wire, want) in [
            ("silent", NotifyLevel::Silent),
            ("critical", NotifyLevel::Critical),
            ("needs-attention", NotifyLevel::NeedsAttention),
            ("all", NotifyLevel::All),
        ] {
            let cfg =
                parse_config(&format!("notifications:\n  level: {wire}\n")).unwrap();
            assert_eq!(cfg.notifications.unwrap().effective_level(), want, "wire {wire}");
        }
        // An unknown level is a hard field error (closed enum).
        assert!(parse_config("notifications:\n  level: loud\n").is_err());
    }

    #[test]
    fn notification_channels_parse_with_per_channel_level_and_type() {
        let cfg = parse_config(
            "notifications:\n  enabled: true\n  level: critical\n  channels:\n    - type: os\n      level: all\n    - type: slack\n      webhook: \"${SLACK_WEBHOOK}\"\n    - type: telegram\n      bot_token: \"${TG_TOKEN}\"\n      chat_id: \"12345\"\n      level: needs-attention\n      label: mobile\n",
        )
        .unwrap();
        let n = cfg.notifications.unwrap();
        assert_eq!(n.channels.len(), 3);

        // os channel: explicit level override wins over the global.
        assert_eq!(n.channels[0].kind, ChannelKind::Os);
        assert_eq!(n.channels[0].effective_level(n.effective_level()), NotifyLevel::All);

        // slack channel: no own level -> falls back to the global (critical).
        assert_eq!(n.channels[1].kind, ChannelKind::Slack);
        assert_eq!(n.channels[1].level, None);
        assert_eq!(
            n.channels[1].effective_level(n.effective_level()),
            NotifyLevel::Critical
        );
        assert_eq!(n.channels[1].webhook.as_deref(), Some("${SLACK_WEBHOOK}")); // verbatim

        // telegram channel: bot_token + chat_id + own level + label.
        let tg = &n.channels[2];
        assert_eq!(tg.kind, ChannelKind::Telegram);
        assert_eq!(tg.bot_token.as_deref(), Some("${TG_TOKEN}"));
        assert_eq!(tg.chat_id.as_deref(), Some("12345"));
        assert_eq!(tg.effective_level(n.effective_level()), NotifyLevel::NeedsAttention);
        assert_eq!(tg.label.as_deref(), Some("mobile"));
    }

    #[test]
    fn channel_kind_parses_all_variants_and_rejects_unknown() {
        for (wire, want) in [
            ("os", ChannelKind::Os),
            ("slack", ChannelKind::Slack),
            ("mattermost", ChannelKind::Mattermost),
            ("discord", ChannelKind::Discord),
            ("telegram", ChannelKind::Telegram),
        ] {
            let cfg = parse_config(&format!(
                "notifications:\n  channels:\n    - type: {wire}\n"
            ))
            .unwrap();
            assert_eq!(cfg.notifications.unwrap().channels[0].kind, want, "wire {wire}");
        }
        // Unknown transport is a field error.
        assert!(parse_config("notifications:\n  channels:\n    - type: carrier-pigeon\n").is_err());
    }

    #[test]
    fn notifications_block_ignores_unknown_fields() {
        // Forward-compat: future keys (throttling, digest, ...) are ignored.
        let cfg = parse_config(
            "notifications:\n  enabled: true\n  throttle_ms: 5000\n  digest: true\n  channels:\n    - type: os\n      renotify: false\n",
        )
        .unwrap();
        let n = cfg.notifications.unwrap();
        assert!(n.effective_enabled());
        assert_eq!(n.channels[0].kind, ChannelKind::Os);
    }

    #[test]
    fn resolves_cwd_against_config_dir() {
        let base = Path::new("/proj/root");
        assert_eq!(resolve_cwd(base, None), PathBuf::from("/proj/root"));
        assert_eq!(resolve_cwd(base, Some(".")), PathBuf::from("/proj/root"));
        assert_eq!(
            resolve_cwd(base, Some("sub/dir")),
            PathBuf::from("/proj/root/sub/dir")
        );
        assert_eq!(resolve_cwd(base, Some("/abs")), PathBuf::from("/abs"));
    }

    #[test]
    fn prefers_ptygrid_yml_and_falls_back_to_legacy_mterm_yml() {
        let dir = std::env::temp_dir().join(format!(
            "ptygrid-config-name-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // Neither file (no launch/global, allow_default=false): clear error
        // naming both candidates.
        let err = resolve_config_source(&dir, None, None, false).unwrap_err();
        assert!(err.starts_with("not_found:"));
        assert!(err.contains("ptygrid.yml") && err.contains("mterm.yml"));

        // Legacy only: mterm.yml is accepted (origin Project).
        std::fs::write(dir.join(LEGACY_CONFIG_FILE_NAME), "agents: []\n").unwrap();
        assert_eq!(
            resolve_config_source(&dir, None, None, false).unwrap(),
            (dir.join(LEGACY_CONFIG_FILE_NAME), ConfigOrigin::Project)
        );

        // Both present: ptygrid.yml wins.
        std::fs::write(dir.join(CONFIG_FILE_NAME), "agents: []\n").unwrap();
        assert_eq!(
            resolve_config_source(&dir, None, None, false).unwrap(),
            (dir.join(CONFIG_FILE_NAME), ConfigOrigin::Project)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- pure config-file resolution (work -> launch -> ~/.ptygrid) ----

    /// Build an `is_file` predicate from a fixed set of "existing" paths.
    fn present(paths: &[PathBuf]) -> impl Fn(&Path) -> bool + '_ {
        move |p: &Path| paths.iter().any(|x| x == p)
    }

    #[test]
    fn resolves_config_in_working_folder_first() {
        let work = Path::new("/work");
        let launch = Path::new("/launch");
        let home = Path::new("/home/user");

        // ptygrid.yml in the working folder wins over launch and global.
        let existing = vec![
            work.join(CONFIG_FILE_NAME),
            launch.join(CONFIG_FILE_NAME),
            home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME),
        ];
        let (path, origin) =
            resolve_config_path_pure(work, Some(launch), Some(home), &present(&existing)).unwrap();
        assert_eq!(path, work.join(CONFIG_FILE_NAME));
        assert_eq!(origin, ConfigOrigin::Project);
    }

    #[test]
    fn working_folder_prefers_ptygrid_over_mterm() {
        let work = Path::new("/work");
        let existing = vec![work.join(CONFIG_FILE_NAME), work.join(LEGACY_CONFIG_FILE_NAME)];
        let (path, origin) =
            resolve_config_path_pure(work, None, None, &present(&existing)).unwrap();
        assert_eq!(path, work.join(CONFIG_FILE_NAME));
        assert_eq!(origin, ConfigOrigin::Project);

        // Legacy-only still resolves inside the working folder.
        let legacy_only = vec![work.join(LEGACY_CONFIG_FILE_NAME)];
        let (path, origin) =
            resolve_config_path_pure(work, None, None, &present(&legacy_only)).unwrap();
        assert_eq!(path, work.join(LEGACY_CONFIG_FILE_NAME));
        assert_eq!(origin, ConfigOrigin::Project);
    }

    #[test]
    fn falls_back_to_launch_folder() {
        let work = Path::new("/work");
        let launch = Path::new("/launch");
        let home = Path::new("/home/user");

        // Nothing in the working folder; launch has ptygrid.yml (global also has
        // one, but launch is tried first).
        let existing = vec![
            launch.join(CONFIG_FILE_NAME),
            home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME),
        ];
        let (path, origin) =
            resolve_config_path_pure(work, Some(launch), Some(home), &present(&existing)).unwrap();
        assert_eq!(path, launch.join(CONFIG_FILE_NAME));
        assert_eq!(origin, ConfigOrigin::Launch);

        // The launch folder does NOT honor the legacy mterm.yml name.
        let legacy_launch = vec![launch.join(LEGACY_CONFIG_FILE_NAME)];
        let err = resolve_config_path_pure(work, Some(launch), Some(home), &present(&legacy_launch))
            .unwrap_err();
        assert!(err.contains(&launch.join(CONFIG_FILE_NAME)));
    }

    #[test]
    fn falls_back_to_global_ptygrid_dir() {
        let work = Path::new("/work");
        let launch = Path::new("/launch");
        let home = Path::new("/home/user");
        let global = home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME);

        let existing = vec![global.clone()];
        let (path, origin) =
            resolve_config_path_pure(work, Some(launch), Some(home), &present(&existing)).unwrap();
        assert_eq!(path, global);
        assert_eq!(origin, ConfigOrigin::Global);
    }

    #[test]
    fn dedups_launch_when_equal_to_working_folder() {
        // Launch == working folder: the launch candidate must not appear a
        // second time in the tried list, and the (missing) global is last.
        let work = Path::new("/work");
        let home = Path::new("/home/user");
        let tried =
            resolve_config_path_pure(work, Some(work), Some(home), &present(&[])).unwrap_err();
        assert_eq!(
            tried,
            vec![
                work.join(CONFIG_FILE_NAME),
                work.join(LEGACY_CONFIG_FILE_NAME),
                home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME),
            ]
        );
    }

    #[test]
    fn error_lists_every_tried_candidate() {
        let work = Path::new("/work");
        let launch = Path::new("/launch");
        let home = Path::new("/home/user");
        let tried =
            resolve_config_path_pure(work, Some(launch), Some(home), &present(&[])).unwrap_err();
        assert_eq!(
            tried,
            vec![
                work.join(CONFIG_FILE_NAME),
                work.join(LEGACY_CONFIG_FILE_NAME),
                launch.join(CONFIG_FILE_NAME),
                home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME),
            ]
        );
    }

    // ---- built-in default fallback (no config file anywhere) ----

    #[test]
    fn default_config_is_empty_with_queen_enabled() {
        // The no-config fallback: no project, no agents/processes, no queen /
        // teammates blocks (so Queen defaults to enabled on its default port).
        let cfg = Config::default();
        assert_eq!(cfg.project, None);
        assert!(cfg.agents.is_empty());
        assert!(cfg.processes.is_empty());
        assert!(cfg.queen.is_none());
        assert!(cfg.teammates.is_none());
        // queen: None means "enabled with default port" via the effective helpers.
        let q = cfg.queen.unwrap_or_default();
        assert!(q.effective_enabled());
        assert_eq!(q.effective_port(), crate::queen::DEFAULT_PORT);
    }

    #[test]
    fn config_origin_default_serializes_to_lowercase() {
        // Wire value of the new origin is "default".
        assert_eq!(
            serde_json::to_string(&ConfigOrigin::Default).unwrap(),
            "\"default\""
        );
    }

    #[test]
    fn falls_back_to_default_when_nothing_found_and_allowed() {
        // No file in any of the three locations + allow_default=true: resolve to
        // the built-in default, reporting the first candidate <work>/ptygrid.yml
        // as the path (what a later-created file there would be).
        let work = Path::new("/work");
        let launch = Path::new("/launch");
        let home = Path::new("/home/user");
        let (path, origin) =
            resolve_config_source_pure(work, Some(launch), Some(home), &present(&[]), true).unwrap();
        assert_eq!(path, work.join(CONFIG_FILE_NAME));
        assert_eq!(origin, ConfigOrigin::Default);
    }

    #[test]
    fn no_default_when_nothing_found_and_not_allowed() {
        // Same absence, allow_default=false: still the not_found candidate list
        // (startup auto-load path keeps its previous behavior).
        let work = Path::new("/work");
        let launch = Path::new("/launch");
        let home = Path::new("/home/user");
        let tried =
            resolve_config_source_pure(work, Some(launch), Some(home), &present(&[]), false)
                .unwrap_err();
        assert_eq!(
            tried,
            vec![
                work.join(CONFIG_FILE_NAME),
                work.join(LEGACY_CONFIG_FILE_NAME),
                launch.join(CONFIG_FILE_NAME),
                home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME),
            ]
        );
    }

    #[test]
    fn does_not_fall_back_to_default_when_a_file_exists() {
        // A real file anywhere in the search order wins over the default, even
        // when allow_default=true — the fallback only fires when all three miss.
        let work = Path::new("/work");
        let launch = Path::new("/launch");
        let home = Path::new("/home/user");

        // Working-folder file wins.
        let in_work = vec![work.join(CONFIG_FILE_NAME)];
        let (path, origin) =
            resolve_config_source_pure(work, Some(launch), Some(home), &present(&in_work), true)
                .unwrap();
        assert_eq!(path, work.join(CONFIG_FILE_NAME));
        assert_eq!(origin, ConfigOrigin::Project);

        // Global-only file also wins over the default (origin Global, not Default).
        let in_global = vec![home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME)];
        let (path, origin) =
            resolve_config_source_pure(work, Some(launch), Some(home), &present(&in_global), true)
                .unwrap();
        assert_eq!(path, home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE_NAME));
        assert_eq!(origin, ConfigOrigin::Global);
    }

    #[test]
    fn expands_leading_tilde_in_working_folder() {
        // Point HOME at a known dir; `~` and `~/x` expand, others pass through.
        let prev = std::env::var("HOME").ok();
        // SAFETY: single-threaded test manipulating process env.
        unsafe {
            std::env::set_var("HOME", "/home/tester");
        }
        assert_eq!(expand_working_dir("~").unwrap(), PathBuf::from("/home/tester"));
        assert_eq!(
            expand_working_dir("~/works/hoge").unwrap(),
            PathBuf::from("/home/tester/works/hoge")
        );
        assert_eq!(
            expand_working_dir("/abs/path").unwrap(),
            PathBuf::from("/abs/path")
        );
        // `~name` is not special-cased.
        assert_eq!(
            expand_working_dir("~alice/x").unwrap(),
            PathBuf::from("~alice/x")
        );
        unsafe {
            match prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    // ---- team_presets: block (Phase 4.3) ----

    const TEAM_AGENTS: &str = "agents:\n  - name: local\n    cmd: claude\n  - name: opus\n    cmd: claude\n  - name: grok\n    cmd: grok\n";

    #[test]
    fn team_presets_parse_with_defaults_and_lead() {
        let yaml = format!(
            "{TEAM_AGENTS}team_presets:\n  daily:\n    lead: local\n    members:\n      - agent: local\n        instructions: primary\n      - agent: opus\n        standby: true\n      - agent: grok\n    kickoff: go\n"
        );
        let cfg = parse_config(&yaml).unwrap();
        let presets = cfg.team_presets.as_ref().unwrap();
        let daily = &presets["daily"];
        assert_eq!(daily.lead.as_deref(), Some("local"));
        assert_eq!(daily.effective_lead(), Some("local"));
        assert_eq!(daily.kickoff.as_deref(), Some("go"));
        assert_eq!(daily.members.len(), 3);
        assert!(!daily.members[0].effective_standby());
        assert!(daily.members[1].effective_standby());
        assert_eq!(daily.members[0].instructions.as_deref(), Some("primary"));
        assert_eq!(daily.members[2].instructions, None);
    }

    #[test]
    fn team_presets_effective_lead_defaults_to_first_non_standby() {
        let yaml = format!(
            "{TEAM_AGENTS}team_presets:\n  t:\n    members:\n      - agent: opus\n        standby: true\n      - agent: local\n      - agent: grok\n"
        );
        let cfg = parse_config(&yaml).unwrap();
        let t = &cfg.team_presets.as_ref().unwrap()["t"];
        assert_eq!(t.effective_lead(), Some("local"));
    }

    #[test]
    fn team_presets_block_is_optional_and_absent_by_default() {
        let cfg = parse_config(TEAM_AGENTS).unwrap();
        assert!(cfg.team_presets.is_none());
    }

    #[test]
    fn team_presets_reject_unknown_member() {
        let yaml = format!(
            "{TEAM_AGENTS}team_presets:\n  t:\n    members:\n      - agent: nope\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("team_presets.t"), "error was: {err}");
        assert!(err.contains("'nope'"), "error was: {err}");
    }

    #[test]
    fn team_presets_reject_process_member() {
        // processes: names must not join a team preset.
        let yaml = format!(
            "{TEAM_AGENTS}processes:\n  - name: web\n    cmd: npm run dev\nteam_presets:\n  t:\n    members:\n      - agent: web\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("processes"), "error was: {err}");
    }

    #[test]
    fn team_presets_reject_empty_members_and_all_standby() {
        let yaml = format!("{TEAM_AGENTS}team_presets:\n  t:\n    members: []\n");
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("must not be empty"), "error was: {err}");

        let yaml = format!(
            "{TEAM_AGENTS}team_presets:\n  t:\n    members:\n      - agent: local\n        standby: true\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("not be standby"), "error was: {err}");
    }

    #[test]
    fn team_presets_reject_bad_lead() {
        // Lead not a member.
        let yaml = format!(
            "{TEAM_AGENTS}team_presets:\n  t:\n    lead: opus\n    members:\n      - agent: local\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("lead 'opus'"), "error was: {err}");

        // Lead is a standby member.
        let yaml = format!(
            "{TEAM_AGENTS}team_presets:\n  t:\n    lead: opus\n    members:\n      - agent: local\n      - agent: opus\n        standby: true\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("standby"), "error was: {err}");
    }

    #[test]
    fn team_presets_reject_duplicate_member() {
        let yaml = format!(
            "{TEAM_AGENTS}team_presets:\n  t:\n    members:\n      - agent: local\n      - agent: local\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("more than once"), "error was: {err}");
    }

    #[test]
    fn team_presets_ignore_unknown_fields() {
        // Unknown keys inside preset/member are ignored (forward compat),
        // matching the other 4.x blocks.
        let yaml = format!(
            "{TEAM_AGENTS}team_presets:\n  t:\n    future_flag: 1\n    members:\n      - agent: local\n        future_member_flag: 2\n"
        );
        let cfg = parse_config(&yaml).unwrap();
        assert_eq!(cfg.team_presets.unwrap()["t"].members.len(), 1);
    }

    // ---- Phase 5.0.0 追補: close_on_exit / workflows.<name>.autoClose ----

    #[test]
    fn close_on_exit_and_workflow_auto_close_parse_with_defaults() {
        // Agent-level close_on_exit: omitted -> None (never, unchanged).
        let cfg = parse_config("agents:\n  - name: a\n    cmd: x\n").unwrap();
        assert_eq!(cfg.agents[0].close_on_exit, None);

        // Explicit values on agents AND processes (shared AgentDef shape).
        let cfg = parse_config(
            "agents:\n  - name: a\n    cmd: x\n    close_on_exit: success\n  - name: b\n    cmd: y\n    close_on_exit: always\nprocesses:\n  - name: web\n    cmd: npm run dev\n    close_on_exit: always\n",
        )
        .unwrap();
        assert_eq!(cfg.agents[0].close_on_exit, Some(AutoCloseMode::Success));
        assert_eq!(cfg.agents[1].close_on_exit, Some(AutoCloseMode::Always));
        assert_eq!(cfg.processes[0].close_on_exit, Some(AutoCloseMode::Always));

        // Workflow-level autoClose (camelCase wire, WorkflowDef convention).
        let yaml = "agents:\n  - name: a\n    cmd: x\n  - name: b\n    cmd: y\nworkflows:\n  demo:\n    pattern: pipeline\n    autoClose: always\n    steps:\n      - id: first\n        agent: a\n      - id: second\n        agent: b\n        dependsOn: [first]\n  demo2:\n    pattern: pipeline\n    steps:\n      - id: only\n        agent: a\n";
        let cfg = parse_config(yaml).unwrap();
        let workflows = cfg.workflows.unwrap();
        assert_eq!(workflows["demo"].auto_close, Some(AutoCloseMode::Always));
        // Omitted on demo2 -> None (never, unchanged default).
        assert_eq!(workflows["demo2"].auto_close, None);

        // Unknown value is a hard field error (closed enum, same posture as
        // AutoRestart / NotifyLevel / ChannelKind).
        assert!(parse_config(
            "agents:\n  - name: a\n    cmd: x\n    close_on_exit: sometimes\n"
        )
        .is_err());
    }

    // ---- Phase 5.0.4: retry / timeout / condition / handoffTo / supervisor
    // field-level and pattern-shape validation (validate_workflows). ----

    const WF_AGENTS: &str =
        "agents:\n  - name: a\n    cmd: \"a\"\n  - name: b\n    cmd: \"b\"\n  - name: c\n    cmd: \"c\"\n";

    #[test]
    fn workflow_retry_max_range_is_validated() {
        let wf = |max: i32| {
            format!(
                "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        retry:\n          max: {max}\n"
            )
        };
        let err = parse_config(&wf(0)).unwrap_err();
        assert!(err.contains("retry.max"), "error was: {err}");
        let err = parse_config(&wf(11)).unwrap_err();
        assert!(err.contains("retry.max"), "error was: {err}");
        assert!(parse_config(&wf(1)).is_ok());
        assert!(parse_config(&wf(10)).is_ok());
    }

    #[test]
    fn workflow_retry_backoff_ms_range_is_validated() {
        let wf = |backoff: u64| {
            format!(
                "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        retry:\n          max: 3\n          backoffMs: {backoff}\n"
            )
        };
        assert!(parse_config(&wf(60_000)).is_ok());
        let err = parse_config(&wf(60_001)).unwrap_err();
        assert!(err.contains("backoffMs"), "error was: {err}");
    }

    #[test]
    fn workflow_timeout_ms_range_is_validated() {
        let wf = |timeout: u64| {
            format!(
                "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        timeoutMs: {timeout}\n"
            )
        };
        assert!(parse_config(&wf(100)).is_ok());
        assert!(parse_config(&wf(86_400_000)).is_ok());
        let err = parse_config(&wf(99)).unwrap_err();
        assert!(err.contains("timeoutMs"), "error was: {err}");
        let err = parse_config(&wf(86_400_001)).unwrap_err();
        assert!(err.contains("timeoutMs"), "error was: {err}");
    }

    /// `joinOn: n` must land in `1..=fanOut`, where an undeclared `fanOut` is 1.
    ///
    /// Both ends of that range used to be reachable, and each broke the run in
    /// its own way. `joinOn: 0` was fail-open: the dependent became ready with
    /// zero successes and spawned alongside its own unfinished dependency. A
    /// `joinOn` above the copy count was the opposite — unsatisfiable, so the
    /// dependent never became ready, fail-fast had nothing to cancel, and the
    /// run sat in `Running` forever. Neither shape is representable now; this
    /// test is what keeps them that way.
    ///
    /// Note what is still legal, and therefore still broken: an IN-range
    /// `joinOn` can also become unreachable once enough copies fail
    /// terminally. That defect is pinned by
    /// `count_join_partial_success_wedges_run_known_defect` in orchestrator.rs
    /// and cannot be closed here — the config is fine, the tick is not.
    #[test]
    fn workflow_join_on_count_range_is_validated() {
        // No `fanOut`: exactly one copy, so 1 is the only legal count.
        let plain = |join: u32| {
            format!(
                "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: first\n        agent: a\n        joinOn: {join}\n      - id: second\n        agent: b\n        dependsOn: [first]\n"
            )
        };
        assert!(parse_config(&plain(1)).is_ok());
        let err = parse_config(&plain(0)).unwrap_err();
        assert!(err.contains("joinOn"), "error was: {err}");
        let err = parse_config(&plain(2)).unwrap_err();
        assert!(err.contains("joinOn"), "error was: {err}");

        // `fanOut: 3`: 1..=3 legal, 4 is not.
        let fanned = |join: u32| {
            format!(
                "{WF_AGENTS}workflows:\n  wf:\n    pattern: fan-out\n    steps:\n      - id: candidate\n        agent: a\n        fanOut: 3\n        joinOn: {join}\n      - id: reduce\n        agent: b\n        dependsOn: [candidate]\n"
            )
        };
        assert!(parse_config(&fanned(1)).is_ok());
        assert!(parse_config(&fanned(3)).is_ok());
        let err = parse_config(&fanned(4)).unwrap_err();
        assert!(err.contains("joinOn"), "error was: {err}");

        // `joinOn: all` is the other untagged variant and carries no count to
        // range-check, so the check above must not reject it.
        let named = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: fan-out\n    steps:\n      - id: candidate\n        agent: a\n        fanOut: 3\n        joinOn: all\n      - id: reduce\n        agent: b\n        dependsOn: [candidate]\n"
        );
        assert!(parse_config(&named).is_ok());
    }

    #[test]
    fn workflow_condition_requires_valid_regex() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: first\n        agent: a\n      - id: second\n        agent: b\n        dependsOn: [first]\n        condition: \"(\"\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("not a valid regex"), "error was: {err}");
    }

    #[test]
    fn workflow_condition_requires_exactly_one_dependency() {
        let no_deps = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        condition: \"^ACCEPT\"\n"
        );
        let err = parse_config(&no_deps).unwrap_err();
        assert!(err.contains("exactly one dependsOn (found 0)"), "error was: {err}");

        let two_deps = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: first\n        agent: a\n      - id: second\n        agent: b\n      - id: third\n        agent: c\n        dependsOn: [first, second]\n        condition: \"^ACCEPT\"\n"
        );
        let err = parse_config(&two_deps).unwrap_err();
        assert!(err.contains("exactly one dependsOn (found 2)"), "error was: {err}");

        let one_dep = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: first\n        agent: a\n      - id: second\n        agent: b\n        dependsOn: [first]\n        condition: \"^ACCEPT\"\n"
        );
        assert!(parse_config(&one_dep).is_ok());
    }

    #[test]
    fn workflow_condition_rejects_fan_out_on_conditional_step() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: fan-out\n    steps:\n      - id: build\n        agent: a\n        fanOut: 2\n      - id: gate\n        agent: b\n        dependsOn: [build]\n        fanOut: 2\n        condition: \"^ACCEPT\"\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("declares both fanOut and condition"), "error was: {err}");
    }

    #[test]
    fn workflow_condition_rejects_dependency_with_fan_out() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: fan-out\n    steps:\n      - id: build\n        agent: a\n        fanOut: 3\n      - id: gate\n        agent: b\n        dependsOn: [build]\n        condition: \"^ACCEPT\"\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("condition depends on fan-out step"), "error was: {err}");
    }

    #[test]
    fn workflow_reply_join_requires_a_kickoff() {
        // Without a kickoff there is no inbox thread for the agent to reply on,
        // so `detect_reply_completions` could never fire and the run would hang
        // in Running forever. Parse time is the only place to catch it.
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        joinOn: reply\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(
            err.contains("joinOn: reply but has no kickoff"),
            "error was: {err}"
        );

        // Whitespace is not a kickoff either — `send_inbox` trims and then
        // rejects an empty body, so this would fail delivery and leave the
        // same wedge.
        let blank = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        joinOn: reply\n        kickoff: \"   \"\n"
        );
        let err = parse_config(&blank).unwrap_err();
        assert!(
            err.contains("joinOn: reply but has no kickoff"),
            "error was: {err}"
        );

        // With a real kickoff it parses.
        let ok = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        joinOn: reply\n        kickoff: answer me\n"
        );
        assert!(parse_config(&ok).is_ok(), "a reply join with a kickoff is valid");
    }

    #[test]
    fn workflow_handoff_to_rejects_a_fan_out_source() {
        // Which of the N copies' reply bodies would be carried is undefined,
        // so the config shape is rejected rather than resolved arbitrarily.
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: fan-out\n    steps:\n      - id: build\n        agent: a\n        fanOut: 3\n        handoffTo: gate\n      - id: gate\n        agent: b\n        dependsOn: [build]\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(
            err.contains("both fanOut and handoffTo"),
            "error was: {err}"
        );
    }

    #[test]
    fn workflow_handoff_to_rejects_self_reference() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        handoffTo: only\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("handoffTo references itself"), "error was: {err}");
    }

    #[test]
    fn workflow_handoff_to_rejects_unknown_target() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    steps:\n      - id: only\n        agent: a\n        handoffTo: nope\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("not a step id"), "error was: {err}");
    }

    #[test]
    fn workflow_supervisor_requires_exactly_one_root() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: supervisor\n    steps:\n      - id: root1\n        agent: a\n      - id: root2\n        agent: b\n      - id: child\n        agent: c\n        dependsOn: [root1]\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("requires exactly one root step"), "error was: {err}");
    }

    #[test]
    fn workflow_supervisor_children_must_depend_on_root() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: supervisor\n    steps:\n      - id: root\n        agent: a\n      - id: mid\n        agent: b\n        dependsOn: [root]\n      - id: stray\n        agent: c\n        dependsOn: [mid]\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("must dependOn root step"), "error was: {err}");
    }

    #[test]
    fn workflow_supervisor_valid_shape_is_accepted() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: supervisor\n    steps:\n      - id: root\n        agent: a\n      - id: child1\n        agent: b\n        dependsOn: [root]\n      - id: child2\n        agent: c\n        dependsOn: [root]\n"
        );
        assert!(parse_config(&yaml).is_ok());
    }

    #[test]
    fn workflow_handoff_rejects_step_with_multiple_dependencies() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: handoff\n    steps:\n      - id: a1\n        agent: a\n      - id: b1\n        agent: b\n      - id: c1\n        agent: c\n        dependsOn: [a1, b1]\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("handoff is linear (max 1 dependsOn per step)"), "error was: {err}");
    }

    #[test]
    fn workflow_handoff_requires_exactly_one_root() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: handoff\n    steps:\n      - id: a1\n        agent: a\n      - id: b1\n        agent: b\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("requires exactly one root step"), "error was: {err}");
    }

    /// Phase 5.0.4: the back-edge requirement is no longer `pattern: handoff`
    /// only. Under supervisor/fan-out/pipeline a `handoffTo` with no matching
    /// `dependsOn` used to load clean and then carry nothing, because
    /// `spawn_ready` reads the carried body at spawn time and `ready_steps`
    /// only looks at `dependsOn`.
    #[test]
    fn workflow_handoff_to_requires_the_target_to_depend_on_the_source() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: supervisor\n    steps:\n      - id: lead\n        agent: a\n      - id: w1\n        agent: b\n        dependsOn: [lead]\n        handoffTo: w2\n      - id: w2\n        agent: c\n        dependsOn: [lead]\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(
            err.contains("'w2' does not dependOn 'w1'"),
            "error was: {err}"
        );

        // Adding the edge the carry actually needs makes it load.
        let fixed = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: supervisor\n    steps:\n      - id: lead\n        agent: a\n      - id: w1\n        agent: b\n        dependsOn: [lead]\n        handoffTo: w2\n      - id: w2\n        agent: c\n        dependsOn: [lead, w1]\n"
        );
        assert!(parse_config(&fixed).is_ok(), "{:?}", parse_config(&fixed));
    }

    #[test]
    fn workflow_handoff_chain_must_match_depends_on() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: handoff\n    steps:\n      - id: a1\n        agent: a\n        handoffTo: b1\n      - id: b1\n        agent: b\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("does not dependOn"), "error was: {err}");
    }

    #[test]
    fn workflow_handoff_chain_must_cover_all_steps() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: handoff\n    steps:\n      - id: a1\n        agent: a\n        handoffTo: b1\n      - id: b1\n        agent: b\n        dependsOn: [a1]\n      - id: orphan\n        agent: c\n        dependsOn: [a1]\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(err.contains("chain covers 2 of 3 declared steps"), "error was: {err}");
    }

    #[test]
    fn workflow_handoff_valid_chain_is_accepted() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: handoff\n    steps:\n      - id: a1\n        agent: a\n        handoffTo: b1\n      - id: b1\n        agent: b\n        dependsOn: [a1]\n        handoffTo: c1\n      - id: c1\n        agent: c\n        dependsOn: [b1]\n"
        );
        assert!(parse_config(&yaml).is_ok());
    }

    /// `copies_for` (orchestrator.rs) never expands a step under `supervisor`
    /// (only `fan-out` multiplies copies), so a `joinOn: n` built against a
    /// declared `fanOut` was demanding more successes than would ever spawn —
    /// the exact shape `probe_supervisor_fanout_count_join_reports_green_with_skipped_child`
    /// (deleted, orchestrator.rs) used to reach a false-green run through.
    /// Deliberately does NOT also declare `fanOut` on `lead`, so only this
    /// gate (not `workflow_supervisor_step_declaring_fan_out_is_rejected_at_load`
    /// below) fires — the two are meant to be independently testable.
    #[test]
    fn workflow_supervisor_count_join_beyond_effective_copies_is_rejected_at_load() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: supervisor\n    steps:\n      - id: lead\n        agent: a\n        joinOn: 3\n      - id: child\n        agent: b\n        dependsOn: [lead]\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(
            err.contains("effective copy count"),
            "error was: {err}"
        );
    }

    /// `fanOut` only has meaning under `pattern: fan-out`; a `supervisor` step
    /// that declares it would have the field silently ignored by `copies_for`
    /// at runtime. Rejected at load instead of letting the operator's
    /// intended parallelism quietly vanish.
    #[test]
    fn workflow_supervisor_step_declaring_fan_out_is_rejected_at_load() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: supervisor\n    steps:\n      - id: lead\n        agent: a\n        fanOut: 3\n      - id: child\n        agent: b\n        dependsOn: [lead]\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(
            err.contains("fanOut only has meaning under pattern: fan-out"),
            "error was: {err}"
        );
    }

    /// The field-level loop already rejects `fanOut` combined with
    /// `handoffTo` on the SAME step, but that leaves the chain's last step —
    /// which by definition has no `handoffTo` — free to declare `fanOut` and
    /// have it silently ignored by `copies_for`. Must be placed on the final
    /// step: declaring it anywhere else in the chain trips the
    /// `both fanOut and handoffTo` gate first and never reaches this one.
    #[test]
    fn workflow_handoff_step_declaring_fan_out_is_rejected_at_load() {
        let yaml = format!(
            "{WF_AGENTS}workflows:\n  wf:\n    pattern: handoff\n    steps:\n      - id: a1\n        agent: a\n        handoffTo: b1\n      - id: b1\n        agent: b\n        dependsOn: [a1]\n        fanOut: 3\n"
        );
        let err = parse_config(&yaml).unwrap_err();
        assert!(
            err.contains("fanOut only has meaning under pattern: fan-out"),
            "error was: {err}"
        );
    }

    /// The run mailbox `orchestrator::workflow_mailbox` builds from the
    /// workflow name has to fit `queen_store`'s 128-byte mailbox cap. Caught
    /// here, at load, rather than as a `send_inbox` rejection on every kickoff
    /// of a run that has already taken panes on the grid.
    ///
    /// The two cases are adjacent on purpose: 84 bytes is the exact budget, so
    /// accepting 84 and rejecting 85 is what pins the constant to the
    /// arithmetic instead of to a round number someone picked.
    #[test]
    fn workflow_name_longer_than_the_mailbox_budget_is_rejected_at_load() {
        let one_step = "    pattern: pipeline\n    steps:\n      - id: only\n        agent: a\n";

        let at_budget = "w".repeat(WORKFLOW_NAME_MAX_BYTES);
        let yaml = format!("{WF_AGENTS}workflows:\n  {at_budget}:\n{one_step}");
        assert!(
            parse_config(&yaml).is_ok(),
            "a name of exactly {WORKFLOW_NAME_MAX_BYTES} bytes still fits the mailbox"
        );

        let over_budget = "w".repeat(WORKFLOW_NAME_MAX_BYTES + 1);
        let yaml = format!("{WF_AGENTS}workflows:\n  {over_budget}:\n{one_step}");
        let err = parse_config(&yaml).unwrap_err();
        assert!(
            err.contains("workflow name is 85 bytes")
                && err.contains("queen:workflow/<name>/<run_id>"),
            "the error must say how long the name is and why the limit exists; \
             error was: {err}"
        );
    }

    // ---- example/ configs actually parse ----

    /// `example/measure-parallelism/ptygrid.yml` is the synthetic (sleep-only)
    /// fixture used to measure orchestration overhead by hand, so nothing but a
    /// real load exercises it — an operator finds out it is malformed only when
    /// the app refuses the config mid-measurement. `include_str!` pins the file
    /// into this test binary so `parse_config` (serde shape + the whole of
    /// `validate_workflows`: agent references, pattern shapes, `fanOut`/`joinOn`
    /// combinations, name length) runs against the exact bytes shipped in the
    /// repo.
    ///
    /// The assertions below are deliberately about the SHAPE the sample's
    /// comments promise a reader, not just "it parsed": the four workflows,
    /// their patterns, the fan-out copy counts the 9-pane cap arithmetic is
    /// built on (6 + 6 = 12 > `WORKFLOW_SESSION_CAP`), and the `joinOn: any`
    /// that makes the straggler-cancellation demo a straggler-cancellation
    /// demo. Editing the sample's numbers without editing its documentation
    /// fails here.
    #[test]
    fn example_measure_parallelism_config_parses() {
        let text = include_str!("../../example/measure-parallelism/ptygrid.yml");
        let cfg = parse_config(text).expect("example/measure-parallelism must parse");

        // Every workflow step references an `agents:` entry (validate_workflows
        // enforces it, so a parse success already proves it); assert the count
        // so a dropped definition is not silently compensated for elsewhere.
        assert_eq!(
            cfg.agents.len(),
            11,
            "6 work units + 1 gate + 2 waves + 2 race roles"
        );
        assert!(
            cfg.agents.iter().all(|a| a.autostart != Some(true)),
            "loading a measurement fixture must not fill the grid on its own"
        );

        let workflows = cfg.workflows.as_ref().expect("workflows: block");
        assert_eq!(workflows.len(), 4);

        // 1. serial chain: six steps, each depending on the previous one.
        let serial = &workflows["measure-1-serial"];
        assert_eq!(serial.pattern, WorkflowPattern::Pipeline);
        assert_eq!(serial.steps.len(), 6);
        assert_eq!(
            serial
                .steps
                .iter()
                .filter(|s| s.depends_on.as_deref().unwrap_or(&[]).is_empty())
                .count(),
            1,
            "a chain has exactly one root"
        );

        // 2. same six units, split into three independent two-step chains.
        let split = &workflows["measure-2-split"];
        assert_eq!(split.pattern, WorkflowPattern::Pipeline);
        assert_eq!(split.steps.len(), 6);
        assert_eq!(
            split
                .steps
                .iter()
                .filter(|s| s.depends_on.as_deref().unwrap_or(&[]).is_empty())
                .count(),
            3,
            "three items means three roots"
        );
        // The two workflows must move the SAME total amount of work, which is
        // what makes their ideal durations (30s vs 10s) comparable at all.
        let agents_of = |wf: &WorkflowDef| {
            let mut v: Vec<String> = wf.steps.iter().map(|s| s.agent.clone()).collect();
            v.sort();
            v
        };
        assert_eq!(agents_of(serial), agents_of(split));

        // Neither fan-out workflow may declare `fanOut` on a ROOT step:
        // `spawn_workflow`'s root loop gives every copy the bare step id
        // (see `orchestrator::base_id`), while `spawn_ready` mints `id#k`
        // per copy — only the latter is separately readable in the panel,
        // which is where every number this sample is about gets read.
        let root_fan_out = |wf: &WorkflowDef| {
            wf.steps.iter().any(|s| {
                s.fan_out.is_some() && s.depends_on.as_deref().unwrap_or(&[]).is_empty()
            })
        };

        // 3. pane-cap queue: two sibling fan-out steps, 6 + 6 > the 9-pane cap,
        // so the second one is deferred rather than spawned.
        let queue = &workflows["measure-3-pane-queue"];
        assert_eq!(queue.pattern, WorkflowPattern::FanOut);
        assert_eq!(queue.auto_close, Some(AutoCloseMode::Success));
        assert!(!root_fan_out(queue));
        let copies: u32 = queue.steps.iter().filter_map(|s| s.fan_out).sum();
        assert_eq!(copies, 12);
        assert!(
            copies as usize > crate::orchestrator::WORKFLOW_SESSION_CAP,
            "the queue only forms when the two waves cannot both fit on the grid"
        );
        let waves: Vec<&WorkflowStep> =
            queue.steps.iter().filter(|s| s.fan_out.is_some()).collect();
        assert!(
            waves
                .iter()
                .all(|s| s.depends_on.as_deref() == Some(["gate".to_string()].as_slice())),
            "the waves hang off the same gate and not off each other; a \
             dependency between them would serialise them for the wrong \
             reason and the measured wait would mean nothing"
        );

        // 4. joinOn: any + straggler cancellation.
        let race = &workflows["measure-4-join-any"];
        assert_eq!(race.pattern, WorkflowPattern::FanOut);
        assert_eq!(race.auto_close, None, "winner/report panes stay readable");
        assert!(!root_fan_out(race));
        let candidate = race
            .steps
            .iter()
            .find(|s| s.id == "race")
            .expect("the racing step");
        assert_eq!(candidate.fan_out, Some(3));
        assert_eq!(candidate.join_on, Some(JoinOn::Named(JoinOnName::Any)));
        assert!(
            race.steps
                .iter()
                .any(|s| s.depends_on.as_deref() == Some(["race".to_string()].as_slice())),
            "a dependent is what proves the join released downstream work"
        );
        // The losers are killed, not left to exit on their own, so the pane
        // must be closed by the AGENT-level rule: `cancel_stragglers` clears
        // the outcome's `session_id`, which takes the pane out of the run the
        // frontend's `autoCloseModeFor` looks up first.
        let loser = cfg
            .agents
            .iter()
            .find(|a| a.name == candidate.agent)
            .expect("the fan-out step's agent is defined");
        assert_eq!(loser.close_on_exit, Some(AutoCloseMode::Always));
    }

    /// `example/measure-coldstart/ptygrid.yml` — the v0.5.8 item-4 cold-start
    /// fixture — must load through the real `parse_config`, same as the
    /// parallelism sample above and for the same reason: it is committed to the
    /// repo and read by a human who then runs it against paid agents, so a
    /// config that only fails at load time would waste their run.
    ///
    /// The assertions are about the SHAPE the sample's comments promise, and in
    /// particular about the three properties the measurement is built on:
    ///
    /// 1. every step names the SAME agent — that is what makes step 1 a fresh
    ///    `spawn_step` and steps 2/3 a `live_session_id` reuse, and the
    ///    difference between their durations the cold start;
    /// 2. the steps form one linear chain — parallel siblings would trip
    ///    `orchestrator::agent_claimed_by_other_step` and every step would
    ///    spawn fresh, measuring nothing;
    /// 3. every step declares `joinOn: reply` WITH a non-empty `kickoff`. The
    ///    join is what lets a reused (still-running) pane complete at all, and
    ///    the kickoff is what `validate_workflows` demands alongside it — this
    ///    test is the actual proof that the sample clears that check rather
    ///    than a claim that it does.
    #[test]
    fn example_measure_coldstart_config_parses() {
        let text = include_str!("../../example/measure-coldstart/ptygrid.yml");
        let cfg = parse_config(text).expect("example/measure-coldstart must parse");

        // Exactly one definition: the measurement compares one agent against
        // itself, and a second live definition would only add a pane that the
        // "empty the grid first" precondition then has to talk about.
        assert_eq!(cfg.agents.len(), 1, "one agent, measured against itself");
        let agent = &cfg.agents[0];
        assert_ne!(
            agent.autostart,
            Some(true),
            "autostart would leave a live pane behind, and `spawn_workflow`'s \
             root loop reuses a live same-name pane — step 1 would be warm too \
             and the measurement would read ~0"
        );
        assert_eq!(
            agent.close_on_exit, None,
            "a killed pane must stay on the grid so a wedge can be read"
        );
        // The bootstrap prompt is not decoration: kickoffs are delivered to the
        // durable inbox only (`orchestrator::deliver_kickoff` just calls
        // `send_inbox`), nothing is typed into the pane, so a bare CLI would sit
        // at its prompt and step 1 would never complete. The loop instruction is
        // what makes steps 2/3 possible at all.
        for needle in ["await", "reply_inbox", "mailbox=coldstart"] {
            assert!(
                agent.cmd.contains(needle),
                "the bootstrap prompt must name {needle:?}: {}",
                agent.cmd
            );
        }

        let workflows = cfg.workflows.as_ref().expect("workflows: block");
        assert_eq!(workflows.len(), 1, "one workflow, run once and read");
        let wf = &workflows["measure-coldstart"];
        assert_eq!(
            wf.pattern,
            WorkflowPattern::Pipeline,
            "reuse only happens for a singular pipeline step; a fan-out copy \
             always takes a fresh pane"
        );
        assert_eq!(
            wf.auto_close, None,
            "the panes are the measurement's transcript and must survive it"
        );
        assert_eq!(wf.steps.len(), 3, "1 cold + 2 warm");

        assert!(
            wf.steps.iter().all(|s| s.agent == agent.name),
            "all three steps must name the one agent, or steps 2/3 are not warm"
        );

        // A single chain: one root, and every later step depends on exactly the
        // step before it.
        assert_eq!(
            wf.steps[0].depends_on.as_deref().unwrap_or(&[]).len(),
            0,
            "the first step is the chain's root"
        );
        for pair in wf.steps.windows(2) {
            assert_eq!(
                pair[1].depends_on.as_deref(),
                Some([pair[0].id.clone()].as_slice()),
                "step '{}' must hang off '{}' alone",
                pair[1].id,
                pair[0].id
            );
        }

        for step in &wf.steps {
            assert_eq!(
                step.join_on,
                Some(JoinOn::Named(JoinOnName::Reply)),
                "step '{}': a reused pane never exits, so route 1 can never \
                 complete it — only a reply join can",
                step.id
            );
            // The `joinOn: reply` + kickoff pairing rule lives in
            // `validate_workflows`; `parse_config` succeeding above is what
            // proves the sample satisfies it. Assert the kickoff is really
            // there so a future edit cannot quietly drop it and re-derive the
            // error this test exists to prevent.
            assert!(
                step.kickoff.as_deref().is_some_and(|k| !k.trim().is_empty()),
                "step '{}': joinOn: reply requires a non-empty kickoff",
                step.id
            );
            assert_eq!(
                step.timeout_ms,
                Some(120_000),
                "step '{}': the escape hatch is documented as 2 minutes",
                step.id
            );
            assert!(step.fan_out.is_none(), "step '{}': no fan-out", step.id);
        }

        // The two bounds the file's "why this number" section reasons about.
        // The step timeout must clear two full 55s `await` rounds (so a single
        // spurious re-await cannot fail a healthy step) and stay well under the
        // pane-wait bound (so a timeout in the panel is unambiguous).
        let step_timeout = wf.steps[0].timeout_ms.expect("checked above");
        assert!(
            step_timeout > 2 * 55_000,
            "a step timeout at or below two await rounds would false-fire"
        );
        assert!(
            step_timeout < crate::orchestrator::WORKFLOW_DEFER_MAX_MS,
            "the execution bound and the pane-wait bound must stay \
             distinguishable in the panel"
        );
    }
}
