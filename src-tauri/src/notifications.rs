// Out-of-app notifications (Phase 4.4.2) — routing + message model.
//
// Edge-triggered alerts to channels OUTSIDE the ptygrid window: the desktop OS
// toast and chat webhooks (Slack / Mattermost / Discord / Telegram). Three event
// sources feed this module, all ALREADY edge-triggered by the time they get here
// (so this layer never polls and never dedups a level):
//   - session lifecycle (`session::handle_eof`): a session EXITED — abnormally
//     (nonzero exit / signal) => `Error`, cleanly (code 0) => `Complete`.
//   - agent-status (`agent_status::emit`): a live session's semantic status
//     CHANGED — to `blocked` => `NeedsAttention`, to `done` => `Complete`.
//   - workflow escalation (`orchestrator::take_escalations`, Stage A-4): a
//     workflow step spent its whole `retry` budget => `Error`. That source sits
//     on the 200ms driver tick, which IS a poll, so the EDGE is made there, not
//     here: `take_escalations` flips a once-only `StepOutcome::escalated` flag
//     and only hands this module the steps that flipped on that tick. This
//     module's "never dedups" premise therefore still holds.
//
// This file is the PURE core: the event model, the (event × level) routing
// decision, channel selection, and message formatting — all unit-tested with no
// I/O. The managed state, `${VAR}` expansion, OS-toast + webhook dispatch, and
// the wiring from the three event sources land in the dispatch layer (a
// follow-up within this module) and never run on the reader hot path.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

use crate::config::{
    expand_vars, ChannelConfig, ChannelKind, Config, NotificationsConfig, NotifyLevel,
};

/// A notifiable moment, ordered by severity. Always derived from one of the two
/// edge sources above — never a polled/level state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyEvent {
    /// Abnormal termination: nonzero exit code or a signal. Highest severity.
    Error,
    /// A live agent needs a human: an approval / input / permission prompt
    /// (agent-status changed to `blocked`).
    NeedsAttention,
    /// Normal completion: a clean exit (code 0) or agent-status changed to
    /// `done`.
    Complete,
    /// Progress / informational. Reserved: only `all` receives it, and no event
    /// source constructs it yet (a future progress source will — spec §9). The
    /// `allow` keeps the reserved variant without tripping the 0-warning build.
    #[allow(dead_code)]
    Progress,
}

/// Whether a channel sitting at `level` receives `event` — the design matrix,
/// verbatim:
///
/// | level             | error | needs-attention | complete | progress |
/// |-------------------|:-----:|:---------------:|:--------:|:--------:|
/// | silent            |   —   |        —        |    —     |    —     |
/// | critical          |   ●   |        —        |    —     |    —     |
/// | needs-attention   |   ●   |        ●        |    —     |    —     |
/// | all               |   ●   |        ●        |    ●     |    ●     |
pub fn should_send(level: NotifyLevel, event: NotifyEvent) -> bool {
    match level {
        NotifyLevel::Silent => false,
        NotifyLevel::Critical => matches!(event, NotifyEvent::Error),
        NotifyLevel::NeedsAttention => {
            matches!(event, NotifyEvent::Error | NotifyEvent::NeedsAttention)
        }
        NotifyLevel::All => true,
    }
}

/// The channels that should fire for `event`, given the loaded block's channel
/// list and its global default level. Pure: each channel's *effective* level is
/// its own override else `global`. Preserves config order so dispatch is
/// deterministic.
pub fn channels_for(
    channels: &[ChannelConfig],
    global: NotifyLevel,
    event: NotifyEvent,
) -> impl Iterator<Item = &ChannelConfig> {
    channels
        .iter()
        .filter(move |c| should_send(c.effective_level(global), event))
}

/// Which workflow step a notification is about, when the event came from the
/// orchestrator rather than from a session (Stage A-4). `None` for the two
/// session-centric sources, which is what keeps their messages byte-identical
/// to what they were before this existed.
///
/// Deliberately additive rather than a change to `NotifyContext::session_id`:
/// an escalating step may have NO session at all (its spawn never got a pane,
/// or its pane was killed by `check_timeouts` before the budget ran out), and
/// making `session_id` an `Option` would have rippled through the two sources
/// that always do have one. Here the step is named by workflow + step id, so
/// the session id is never needed to name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowOrigin {
    /// Workflow name as declared under `workflows:`.
    pub workflow: String,
    /// Run id (`wfr_…`), so a message can be matched to a row in the panel.
    pub run_id: String,
    /// Step id, suffixed (`review#2`) for a fan-out / `onEach` copy.
    pub step_id: String,
    /// Spawn attempts made before the budget ran out (1 = the original spawn).
    pub attempts: u32,
}

/// Owned session context for a notification message. Owned strings so a
/// dispatch payload can outlive the session-map lock it was read under.
#[derive(Debug, Clone, Default)]
pub struct NotifyContext {
    pub session_id: u32,
    /// Agent/process definition name, when the session has one.
    pub name: Option<String>,
    /// Loaded project name (`config.project`), for multi-project disambiguation.
    pub project: Option<String>,
    /// Event-specific tail: exit-code text for `Error`/`Complete`, the matched
    /// rule / prompt hint for `NeedsAttention`, the step's own error text for a
    /// workflow escalation.
    pub detail: Option<String>,
    /// Set only by the workflow-escalation source; see `WorkflowOrigin`.
    pub origin: Option<WorkflowOrigin>,
}

impl NotifyContext {
    /// How the session is named in a message: its definition name, else `#id`.
    fn who(&self) -> String {
        match &self.name {
            Some(n) if !n.is_empty() => n.clone(),
            _ => format!("#{}", self.session_id),
        }
    }
}

/// A leading glyph per event (matches the in-app status vocabulary). Kept ASCII
/// on the sad path so plain webhooks stay readable.
fn glyph(event: NotifyEvent) -> &'static str {
    match event {
        NotifyEvent::Error => "⛔",
        NotifyEvent::NeedsAttention => "⏳",
        NotifyEvent::Complete => "✅",
        NotifyEvent::Progress => "…",
    }
}

/// One-line notification title: `<glyph> <who> <verb>`, optionally prefixed with
/// the project so a multi-project desktop can tell alerts apart.
///
/// A workflow escalation (Stage A-4) names the STEP rather than the session
/// (`demo/review#2`) and gets its own verb: "exited abnormally" would be a lie
/// twice over — the step may never have had a process to exit, and the point of
/// the message is that the automatic recovery is over, not that something ended.
pub fn format_title(event: NotifyEvent, ctx: &NotifyContext) -> String {
    if let Some(origin) = &ctx.origin {
        let mut title = format!(
            "{} {}/{} exhausted its retries",
            glyph(event),
            origin.workflow,
            origin.step_id
        );
        if let Some(project) = ctx.project.as_deref().filter(|p| !p.is_empty()) {
            title = format!("[{project}] {title}");
        }
        return title;
    }
    let verb = match event {
        NotifyEvent::Error => "exited abnormally",
        NotifyEvent::NeedsAttention => "needs attention",
        NotifyEvent::Complete => "finished",
        NotifyEvent::Progress => "update",
    };
    let mut title = format!("{} {} {}", glyph(event), ctx.who(), verb);
    if let Some(project) = ctx.project.as_deref().filter(|p| !p.is_empty()) {
        title = format!("[{project}] {title}");
    }
    title
}

/// Notification body: the event-specific `detail` when present, else a stable
/// fallback naming the session. Never empty (some transports reject empty text).
///
/// A workflow escalation composes instead of substituting: the operator has to
/// be able to act on the message alone, so it always states which workflow,
/// which run, which step, which agent and how many attempts, and appends the
/// step's own error text as the tail rather than letting it replace all of that.
pub fn format_body(event: NotifyEvent, ctx: &NotifyContext) -> String {
    if let Some(origin) = &ctx.origin {
        return escalation_body(origin, ctx);
    }
    if let Some(detail) = ctx.detail.as_deref().filter(|d| !d.is_empty()) {
        return detail.to_string();
    }
    match event {
        NotifyEvent::Error => format!("Session {} terminated abnormally.", ctx.who()),
        NotifyEvent::NeedsAttention => format!("Session {} is waiting for you.", ctx.who()),
        NotifyEvent::Complete => format!("Session {} completed.", ctx.who()),
        NotifyEvent::Progress => format!("Session {} update.", ctx.who()),
    }
}

/// Body for a retry-exhaustion escalation. Ends with what the operator is being
/// asked to do, because nothing in ptygrid will try this step again.
fn escalation_body(origin: &WorkflowOrigin, ctx: &NotifyContext) -> String {
    let plural = if origin.attempts == 1 { "" } else { "s" };
    let mut body = format!(
        "Workflow '{}' (run {}): step '{}' failed after {} attempt{} and has no retry budget left.",
        origin.workflow, origin.run_id, origin.step_id, origin.attempts, plural
    );
    if let Some(agent) = ctx.name.as_deref().filter(|n| !n.is_empty()) {
        body.push_str(&format!(" Agent: {agent}."));
    }
    if let Some(detail) = ctx.detail.as_deref().filter(|d| !d.is_empty()) {
        body.push_str(&format!(" Last error: {detail}"));
    }
    body.push_str(" No further automatic retry will happen.");
    body
}

/// Map a session exit code to the right lifecycle event: `Complete` on a clean
/// `Some(0)`, `Error` on any nonzero code or an unknown code (`None`, i.e. the
/// child could not be reaped / was signalled). Used by the `session::handle_eof`
/// wiring.
pub fn event_for_exit(code: Option<i32>) -> NotifyEvent {
    match code {
        Some(0) => NotifyEvent::Complete,
        _ => NotifyEvent::Error,
    }
}

/// Render a human-readable `detail` for an exit event (`"exit code 2"`,
/// `"exited cleanly"`, `"terminated (no exit code)"`).
pub fn exit_detail(code: Option<i32>) -> String {
    match code {
        Some(0) => "exited cleanly".to_string(),
        Some(c) => format!("exit code {c}"),
        None => "terminated (no exit code)".to_string(),
    }
}

// ---------- dispatch layer (managed state + I/O) ----------

/// The loaded `notifications` block plus the project name, or `None` when the
/// feature is off (no block / `enabled: false`). Swapped wholesale on every
/// `load_config`, mirroring how `agent_status` reconfigures.
#[derive(Clone)]
struct Loaded {
    config: NotificationsConfig,
    project: Option<String>,
}

/// Tauri-managed state for out-of-app notifications. The lock is held only long
/// enough to clone a snapshot — never across the actual OS/webhook send.
pub struct NotificationManager {
    inner: Mutex<Option<Loaded>>,
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationManager {
    pub fn new() -> Self {
        NotificationManager {
            inner: Mutex::new(None),
        }
    }
    fn snapshot(&self) -> Option<Loaded> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn set(&self, loaded: Option<Loaded>) {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner()) = loaded;
    }
}

/// Fold the loaded config's `notifications` block into managed state. Called from
/// `load_config` (including file reloads), so edits take effect immediately. A
/// disabled/absent block clears the state so nothing is sent until re-enabled.
pub fn apply<R: Runtime>(app: &AppHandle<R>, config: &Config) {
    let Some(state) = app.try_state::<NotificationManager>() else {
        return;
    };
    let loaded = match &config.notifications {
        Some(n) if n.effective_enabled() => Some(Loaded {
            config: n.clone(),
            project: config.project.clone(),
        }),
        _ => None,
    };
    state.set(loaded);
}

/// Route one edge event to every channel whose effective level includes it, and
/// fire each: the OS toast inline, chat webhooks on a detached thread so blocking
/// network I/O never touches the caller (the agent-status task, the PTY reader
/// thread, or the 200ms workflow driver). A no-op when the feature is off or no
/// channel matches. Session-sourced convenience wrapper over `dispatch_ctx`.
pub fn dispatch<R: Runtime>(
    app: &AppHandle<R>,
    event: NotifyEvent,
    id: u32,
    name: Option<String>,
    detail: Option<String>,
) {
    dispatch_ctx(
        app,
        event,
        NotifyContext {
            session_id: id,
            name,
            project: None, // filled in from the loaded config below
            detail,
            origin: None,
        },
    );
}

/// `dispatch` for a caller that has more than a session to say — today only the
/// workflow-escalation source (Stage A-4), which fills `NotifyContext::origin`.
/// Same routing, same channels, same threading rules; the only difference is
/// that the context is supplied whole instead of assembled from three arguments.
/// `ctx.project` is always overwritten from the loaded config, so no caller has
/// to know the project name.
pub fn dispatch_ctx<R: Runtime>(app: &AppHandle<R>, event: NotifyEvent, mut ctx: NotifyContext) {
    let Some(state) = app.try_state::<NotificationManager>() else {
        return;
    };
    let Some(loaded) = state.snapshot() else {
        return; // feature disabled
    };
    let global = loaded.config.effective_level();
    let targets: Vec<ChannelConfig> = channels_for(&loaded.config.channels, global, event)
        .cloned()
        .collect();
    if targets.is_empty() {
        return;
    }
    ctx.project = loaded.project.clone();
    let title = format_title(event, &ctx);
    let body = format_body(event, &ctx);
    for ch in &targets {
        send_channel(app, ch, &title, &body);
    }
}

fn send_channel<R: Runtime>(app: &AppHandle<R>, ch: &ChannelConfig, title: &str, body: &str) {
    match ch.kind {
        ChannelKind::Os => send_os(app, title, body),
        // Slack and Mattermost share the incoming-webhook `{"text": ...}` shape.
        ChannelKind::Slack | ChannelKind::Mattermost => {
            send_webhook(ch, WebhookFmt::Slack, title, body)
        }
        ChannelKind::Discord => send_webhook(ch, WebhookFmt::Discord, title, body),
        ChannelKind::Telegram => send_telegram(ch, title, body),
    }
}

/// Local desktop toast via tauri-plugin-notification. Failures (e.g. the OS has
/// not granted permission) are logged, never propagated.
fn send_os<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("notifications: OS toast failed: {e}");
    }
}

/// Slack / Mattermost / Discord incoming webhook. Missing or empty (post-expand)
/// URLs are skipped with a warning rather than failing the whole dispatch.
fn send_webhook(ch: &ChannelConfig, fmt: WebhookFmt, title: &str, body: &str) {
    let tag = kind_tag(ch.kind);
    let Some(raw) = ch.webhook.as_deref() else {
        eprintln!("notifications: {tag} channel missing `webhook`; skipped");
        return;
    };
    let url = expand_vars(raw);
    if url.is_empty() {
        eprintln!("notifications: {tag} webhook expanded to empty; skipped");
        return;
    }
    post_json(url, webhook_body(&fmt, title, body), tag);
}

/// Telegram Bot API `sendMessage`. Needs both `bot_token` and `chat_id`.
fn send_telegram(ch: &ChannelConfig, title: &str, body: &str) {
    let (Some(token), Some(chat)) = (ch.bot_token.as_deref(), ch.chat_id.as_deref()) else {
        eprintln!("notifications: telegram channel needs `bot_token` and `chat_id`; skipped");
        return;
    };
    let token = expand_vars(token);
    let chat = expand_vars(chat);
    if token.is_empty() || chat.is_empty() {
        eprintln!("notifications: telegram bot_token/chat_id expanded to empty; skipped");
        return;
    }
    post_json(telegram_url(&token), telegram_body(&chat, title, body), "telegram");
}

/// POST a JSON body on a detached thread (blocking ureq, 10s overall timeout).
/// Fire-and-forget: a failed webhook is logged, never retried or surfaced.
fn post_json(url: String, body: serde_json::Value, label: &'static str) {
    std::thread::spawn(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build();
        if let Err(e) = agent.post(&url).send_json(body) {
            eprintln!("notifications: {label} webhook failed: {e}");
        }
    });
}

/// Slack/Mattermost use `{"text": ...}`; Discord uses `{"content": ...}`. Both
/// carry the title + body as one combined message.
enum WebhookFmt {
    Slack,
    Discord,
}

fn webhook_body(fmt: &WebhookFmt, title: &str, body: &str) -> serde_json::Value {
    let text = format!("{title}\n{body}");
    match fmt {
        WebhookFmt::Slack => serde_json::json!({ "text": text }),
        WebhookFmt::Discord => serde_json::json!({ "content": text }),
    }
}

fn telegram_url(token: &str) -> String {
    format!("https://api.telegram.org/bot{token}/sendMessage")
}

fn telegram_body(chat_id: &str, title: &str, body: &str) -> serde_json::Value {
    serde_json::json!({ "chat_id": chat_id, "text": format!("{title}\n{body}") })
}

fn kind_tag(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Os => "os",
        ChannelKind::Slack => "slack",
        ChannelKind::Mattermost => "mattermost",
        ChannelKind::Discord => "discord",
        ChannelKind::Telegram => "telegram",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{parse_config, ChannelKind};

    fn channels(yaml: &str) -> (Vec<ChannelConfig>, NotifyLevel) {
        let n = parse_config(yaml).unwrap().notifications.unwrap();
        let global = n.effective_level();
        (n.channels, global)
    }

    // ---- routing matrix ----

    #[test]
    fn should_send_matches_the_design_matrix() {
        use NotifyEvent::*;
        use NotifyLevel as L;
        // silent: never.
        for e in [Error, NeedsAttention, Complete, Progress] {
            assert!(!should_send(L::Silent, e), "silent must drop {e:?}");
        }
        // critical: error only.
        assert!(should_send(L::Critical, Error));
        for e in [NeedsAttention, Complete, Progress] {
            assert!(!should_send(L::Critical, e), "critical must drop {e:?}");
        }
        // needs-attention: error + needs-attention.
        assert!(should_send(L::NeedsAttention, Error));
        assert!(should_send(L::NeedsAttention, NeedsAttention));
        assert!(!should_send(L::NeedsAttention, Complete));
        assert!(!should_send(L::NeedsAttention, Progress));
        // all: everything.
        for e in [Error, NeedsAttention, Complete, Progress] {
            assert!(should_send(L::All, e), "all must send {e:?}");
        }
    }

    // ---- channel selection (per-channel override vs global) ----

    #[test]
    fn channels_for_applies_per_channel_level_over_global() {
        // Global critical; Slack stays critical, Telegram bumped to
        // needs-attention, OS bumped to all.
        let (chans, global) = channels(
            "notifications:\n  level: critical\n  channels:\n    - type: slack\n      webhook: s\n    - type: telegram\n      bot_token: t\n      chat_id: c\n      level: needs-attention\n    - type: os\n      level: all\n",
        );

        // A NeedsAttention event: slack (critical) drops it; telegram + os keep it.
        let got: Vec<ChannelKind> = channels_for(&chans, global, NotifyEvent::NeedsAttention)
            .map(|c| c.kind)
            .collect();
        assert_eq!(got, vec![ChannelKind::Telegram, ChannelKind::Os]);

        // A Complete event: only os (all) receives it.
        let got: Vec<ChannelKind> = channels_for(&chans, global, NotifyEvent::Complete)
            .map(|c| c.kind)
            .collect();
        assert_eq!(got, vec![ChannelKind::Os]);

        // An Error event: everyone (all three levels include Error).
        let got: Vec<ChannelKind> = channels_for(&chans, global, NotifyEvent::Error)
            .map(|c| c.kind)
            .collect();
        assert_eq!(
            got,
            vec![ChannelKind::Slack, ChannelKind::Telegram, ChannelKind::Os]
        );
    }

    #[test]
    fn channels_for_silent_channel_receives_nothing() {
        let (chans, global) = channels(
            "notifications:\n  level: all\n  channels:\n    - type: os\n      level: silent\n",
        );
        for e in [
            NotifyEvent::Error,
            NotifyEvent::NeedsAttention,
            NotifyEvent::Complete,
            NotifyEvent::Progress,
        ] {
            assert_eq!(channels_for(&chans, global, e).count(), 0, "{e:?}");
        }
    }

    #[test]
    fn channels_for_preserves_config_order() {
        let (chans, global) = channels(
            "notifications:\n  level: all\n  channels:\n    - type: discord\n      webhook: d\n    - type: slack\n      webhook: s\n    - type: mattermost\n      webhook: m\n",
        );
        let got: Vec<ChannelKind> = channels_for(&chans, global, NotifyEvent::Error)
            .map(|c| c.kind)
            .collect();
        assert_eq!(
            got,
            vec![ChannelKind::Discord, ChannelKind::Slack, ChannelKind::Mattermost]
        );
    }

    // ---- exit-code mapping ----

    #[test]
    fn exit_code_maps_to_event_and_detail() {
        assert_eq!(event_for_exit(Some(0)), NotifyEvent::Complete);
        assert_eq!(event_for_exit(Some(1)), NotifyEvent::Error);
        assert_eq!(event_for_exit(Some(137)), NotifyEvent::Error);
        assert_eq!(event_for_exit(None), NotifyEvent::Error);

        assert_eq!(exit_detail(Some(0)), "exited cleanly");
        assert_eq!(exit_detail(Some(2)), "exit code 2");
        assert_eq!(exit_detail(None), "terminated (no exit code)");
    }

    // ---- message formatting ----

    #[test]
    fn title_uses_name_then_falls_back_to_hash_id() {
        let named = NotifyContext {
            session_id: 3,
            name: Some("codex".to_string()),
            ..Default::default()
        };
        assert_eq!(
            format_title(NotifyEvent::Error, &named),
            "⛔ codex exited abnormally"
        );

        let anon = NotifyContext {
            session_id: 7,
            name: None,
            ..Default::default()
        };
        assert_eq!(
            format_title(NotifyEvent::NeedsAttention, &anon),
            "⏳ #7 needs attention"
        );
    }

    #[test]
    fn title_prefixes_project_when_present() {
        let ctx = NotifyContext {
            session_id: 1,
            name: Some("claude".to_string()),
            project: Some("my-app".to_string()),
            detail: None,
            origin: None,
        };
        assert_eq!(
            format_title(NotifyEvent::Complete, &ctx),
            "[my-app] ✅ claude finished"
        );
        // An empty project string is treated as absent (no stray brackets).
        let ctx = NotifyContext {
            project: Some(String::new()),
            ..ctx
        };
        assert_eq!(format_title(NotifyEvent::Complete, &ctx), "✅ claude finished");
    }

    #[test]
    fn body_prefers_detail_then_falls_back() {
        let with_detail = NotifyContext {
            session_id: 2,
            name: Some("web".to_string()),
            project: None,
            detail: Some("exit code 2".to_string()),
            origin: None,
        };
        assert_eq!(format_body(NotifyEvent::Error, &with_detail), "exit code 2");

        let no_detail = NotifyContext {
            session_id: 2,
            name: Some("web".to_string()),
            ..Default::default()
        };
        assert_eq!(
            format_body(NotifyEvent::Error, &no_detail),
            "Session web terminated abnormally."
        );
        // Body is never empty even when detail is an empty string.
        let empty_detail = NotifyContext {
            detail: Some(String::new()),
            ..no_detail
        };
        assert!(!format_body(NotifyEvent::Complete, &empty_detail).is_empty());
    }

    // ---- workflow escalation (Stage A-4) ----

    fn escalation_ctx() -> NotifyContext {
        NotifyContext {
            session_id: 0, // an escalating step may have had no pane at all
            name: Some("reviewer".to_string()),
            project: Some("my-app".to_string()),
            detail: Some("timed out after 60000ms".to_string()),
            origin: Some(WorkflowOrigin {
                workflow: "demo".to_string(),
                run_id: "wfr_abc".to_string(),
                step_id: "review#2".to_string(),
                attempts: 3,
            }),
        }
    }

    #[test]
    fn escalation_names_the_workflow_run_step_and_attempt_count() {
        let ctx = escalation_ctx();
        // Title names the STEP, not `#0` — the step may never have had a session.
        assert_eq!(
            format_title(NotifyEvent::Error, &ctx),
            "[my-app] ⛔ demo/review#2 exhausted its retries"
        );
        let body = format_body(NotifyEvent::Error, &ctx);
        for needle in [
            "demo",
            "wfr_abc",
            "review#2",
            "3 attempts",
            "reviewer",
            "timed out after 60000ms",
            "No further automatic retry",
        ] {
            assert!(body.contains(needle), "body must mention {needle}: {body}");
        }
        // A single attempt is not pluralised, and a step that failed without an
        // error string still produces a usable, non-empty body.
        let bare = NotifyContext {
            name: None,
            detail: None,
            origin: Some(WorkflowOrigin {
                workflow: "demo".to_string(),
                run_id: "wfr_abc".to_string(),
                step_id: "first".to_string(),
                attempts: 1,
            }),
            ..escalation_ctx()
        };
        let body = format_body(NotifyEvent::Error, &bare);
        assert!(body.contains("1 attempt and"), "no plural for one: {body}");
        assert!(!body.contains("Agent:"), "no agent clause when unnamed: {body}");
        assert!(!body.contains("Last error:"), "no error clause when none: {body}");
    }

    #[test]
    fn escalation_reaches_a_channel_left_at_the_default_critical_level() {
        // Why `Error` and not `NeedsAttention`: `critical` is the DEFAULT level,
        // and it subscribes to `error` only. Mapping retry exhaustion to
        // `NeedsAttention` would make the escalation invisible to every operator
        // who never edited `level:` — i.e. to the default configuration.
        let (chans, global) =
            channels("notifications:\n  channels:\n    - type: slack\n      webhook: s\n");
        assert_eq!(global, NotifyLevel::Critical, "default level is critical");
        assert_eq!(channels_for(&chans, global, NotifyEvent::Error).count(), 1);
        assert_eq!(
            channels_for(&chans, global, NotifyEvent::NeedsAttention).count(),
            0,
            "a critical channel would have dropped a NeedsAttention escalation"
        );
    }

    // ---- webhook payload builders ----

    #[test]
    fn webhook_body_slack_vs_discord_shape() {
        let s = webhook_body(&WebhookFmt::Slack, "T", "B");
        assert_eq!(s["text"], "T\nB");
        assert!(s.get("content").is_none());

        let d = webhook_body(&WebhookFmt::Discord, "T", "B");
        assert_eq!(d["content"], "T\nB");
        assert!(d.get("text").is_none());
    }

    #[test]
    fn telegram_url_and_body_shape() {
        assert_eq!(
            telegram_url("123:abc"),
            "https://api.telegram.org/bot123:abc/sendMessage"
        );
        let b = telegram_body("42", "T", "B");
        assert_eq!(b["chat_id"], "42");
        assert_eq!(b["text"], "T\nB");
    }

    #[test]
    fn kind_tag_covers_every_variant() {
        assert_eq!(kind_tag(ChannelKind::Os), "os");
        assert_eq!(kind_tag(ChannelKind::Slack), "slack");
        assert_eq!(kind_tag(ChannelKind::Mattermost), "mattermost");
        assert_eq!(kind_tag(ChannelKind::Discord), "discord");
        assert_eq!(kind_tag(ChannelKind::Telegram), "telegram");
    }
}
