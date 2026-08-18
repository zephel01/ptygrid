//! Phase 4.4.4: per-pane **connection context** — "where is this pane pointed?".
//!
//! One passive sampler that rides the Phase 3.5 resource-monitor thread at a
//! slower cadence and answers, per running PTY pane:
//!   * which directory / git branch the pane is working in,
//!   * which AWS profile + region its next command would hit,
//!   * which LLM endpoint / model an agent CLI in it is talking to.
//!
//! The remaining "where" — the destination of an `ssh` / `kubectl` foreground —
//! is already resolved by the Phase 4.4.3 foreground `detail` that rides the
//! 1s resource batch, so it is deliberately NOT duplicated here; the frontend
//! composes the remote chip from `ui.foregroundDetail`.
//!
//! ## Secrets
//!
//! Environment values are read through a fixed [`ENV_ALLOWLIST`] and every
//! candidate key additionally has to pass [`is_secretish`], so no `*_KEY` /
//! `*_TOKEN` / `*_SECRET` value can reach the frontend even if a future edit
//! adds one to the allowlist by mistake (`allowlist_has_no_secretish_key`
//! fails the build's test run in that case). URL userinfo is stripped from
//! endpoints for the same reason (`https://tok@host` → `host`).
//!
//! ## Freshness caveat (documented in CONTRACT.md)
//!
//! Both `/proc/<pid>/environ` (Linux) and `KERN_PROCARGS2` (macOS) expose the
//! environment a process was *started* with — the kernel keeps no live view.
//! So a variable exported inside an already-running shell (direnv, `export`)
//! shows up as soon as that shell runs a command (the child carries it), not
//! while the shell sits idle at its prompt. The working directory has no such
//! limitation: it is read live.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Manager, Runtime};

use crate::config::{Config, PaneContextConfig};

// ---------------------------------------------------------------------------
// wire types
// ---------------------------------------------------------------------------

/// One pane's connection context. Every field is optional: a pane whose
/// context resolved to nothing at all is dropped by the sampler rather than
/// emitted as an empty object.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneContext {
    pub id: u32,
    /// Display form of the live working directory (`~` collapsed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Directory name of the repository root containing `cwd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Branch name, or `@<short-sha>` when HEAD is detached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws: Option<AwsContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelContext>,
}

impl PaneContext {
    /// True when nothing at all resolved (not worth putting on the wire).
    fn is_empty(&self) -> bool {
        self.cwd.is_none() && self.repo.is_none() && self.branch.is_none()
            && self.aws.is_none() && self.model.is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AwsContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelContext {
    /// `anthropic` | `openai` | `ollama` — which env family matched.
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `host:port` of the base URL, userinfo and path stripped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// The endpoint resolves to this machine (loopback / `*.local`) — i.e. a
    /// local model server such as llama.cpp / ollama / claude-code-router.
    pub local: bool,
}

/// The batch emitted as the `session-context` event.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBatch {
    pub sampled_at_ms: u64,
    pub sessions: Vec<PaneContext>,
}

// ---------------------------------------------------------------------------
// env allowlist (pure)
// ---------------------------------------------------------------------------

/// The only environment variables ever read off a pane's process. Adding a key
/// here is the single place a new context field gets its input from — and the
/// [`is_secretish`] gate below still applies to every one of them.
pub(crate) const ENV_ALLOWLIST: [&str; 10] = [
    "AWS_PROFILE",
    "AWS_DEFAULT_PROFILE",
    "AWS_VAULT",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "OPENAI_BASE_URL",
    "OPENAI_MODEL",
    "OLLAMA_HOST",
];

/// Belt-and-braces guard: any key that *looks* like it carries a credential is
/// refused regardless of the allowlist. Deliberately over-broad — a false
/// positive costs one missing chip, a false negative leaks a secret into the
/// UI (and into any screenshot of it).
pub(crate) fn is_secretish(key: &str) -> bool {
    const NEEDLES: [&str; 7] = [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "SESSION",
    ];
    let upper = key.to_ascii_uppercase();
    NEEDLES.iter().any(|needle| upper.contains(needle))
}

/// Filter raw `KEY=VALUE` strings down to the allowlisted, non-secret, non-empty
/// ones. Input order wins for duplicate keys (the first occurrence is kept),
/// matching how a process' environment block is read.
pub(crate) fn collect_env<I, S>(entries: I) -> BTreeMap<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = BTreeMap::new();
    for entry in entries {
        let Some((key, value)) = entry.as_ref().split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() || is_secretish(key) || !ENV_ALLOWLIST.contains(&key) {
            continue;
        }
        out.entry(key.to_string()).or_insert_with(|| value.to_string());
    }
    out
}

/// `AWS_PROFILE` wins over `AWS_VAULT` (set by aws-vault for the *same*
/// profile) which wins over the legacy `AWS_DEFAULT_PROFILE`.
pub(crate) fn aws_context(env: &BTreeMap<String, String>) -> Option<AwsContext> {
    let pick = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|key| env.get(*key).cloned())
    };
    let ctx = AwsContext {
        profile: pick(&["AWS_PROFILE", "AWS_VAULT", "AWS_DEFAULT_PROFILE"]),
        region: pick(&["AWS_REGION", "AWS_DEFAULT_REGION"]),
    };
    (ctx != AwsContext::default()).then_some(ctx)
}

/// First matching family wins: a pane routed through claude-code-router sets
/// `ANTHROPIC_BASE_URL`, so anthropic is checked before the OpenAI-compatible
/// and ollama shapes.
pub(crate) fn model_context(env: &BTreeMap<String, String>) -> Option<ModelContext> {
    const FAMILIES: [(&str, &str, &str); 3] = [
        ("anthropic", "ANTHROPIC_BASE_URL", "ANTHROPIC_MODEL"),
        ("openai", "OPENAI_BASE_URL", "OPENAI_MODEL"),
        ("ollama", "OLLAMA_HOST", ""),
    ];
    for (provider, base_key, model_key) in FAMILIES {
        let base = env.get(base_key);
        let model = (!model_key.is_empty()).then(|| env.get(model_key)).flatten();
        if base.is_none() && model.is_none() {
            continue;
        }
        let endpoint = base.and_then(|raw| endpoint_label(raw));
        let local = endpoint
            .as_deref()
            .map(|authority| is_local_host(host_of(authority)))
            .unwrap_or(false);
        return Some(ModelContext {
            provider: provider.to_string(),
            model: model.cloned(),
            endpoint,
            local,
        });
    }
    None
}

/// `https://user:tok@host:8080/v1?x=1` → `host:8080`. Userinfo is dropped on
/// purpose: base URLs in the wild sometimes carry a token there.
pub(crate) fn endpoint_label(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    (!authority.is_empty()).then(|| authority.to_string())
}

/// Host part of an authority, IPv6 literals included (`[::1]:8080` → `::1`).
pub(crate) fn host_of(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    authority.split(':').next().unwrap_or(authority)
}

pub(crate) fn is_local_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
        || host.ends_with(".local")
        || host.starts_with("127.")
}

// ---------------------------------------------------------------------------
// git + path (pure parts)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GitInfo {
    pub repo: String,
    pub branch: String,
}

/// `.git/HEAD` → branch name, or `@<short sha>` for a detached HEAD.
/// A full ref path is kept below `refs/heads/` so `feature/x` stays `feature/x`.
pub(crate) fn parse_head(content: &str) -> Option<String> {
    let line = content.lines().next()?.trim();
    if let Some(reference) = line.strip_prefix("ref:") {
        let reference = reference.trim();
        let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return (!name.is_empty()).then(|| name.to_string());
    }
    let is_sha = line.len() >= 7 && line.chars().all(|c| c.is_ascii_hexdigit());
    is_sha.then(|| format!("@{}", &line[..7]))
}

/// `gitdir: <path>` out of a `.git` **file** (linked worktrees / submodules).
pub(crate) fn parse_gitdir_file(content: &str) -> Option<&str> {
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// Collapse the home prefix so a pane header shows `~/works/project/ptygrid`
/// instead of the full absolute path.
pub(crate) fn display_path(path: &Path, home: Option<&Path>) -> String {
    let full = path.to_string_lossy().into_owned();
    let Some(home) = home else { return full };
    let home = home.to_string_lossy();
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return full;
    }
    if full == home {
        return "~".to_string();
    }
    match full.strip_prefix(&format!("{home}/")) {
        Some(rest) => format!("~/{rest}"),
        None => full,
    }
}

/// Resolve the `.git` directory for `entry`, following the `gitdir:` pointer of
/// a linked worktree. Returns `None` when `entry` has no `.git` at all.
fn git_dir(entry: &Path) -> Option<PathBuf> {
    let dot = entry.join(".git");
    let meta = std::fs::metadata(&dot).ok()?;
    if meta.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let path = PathBuf::from(parse_gitdir_file(&text)?);
    Some(if path.is_absolute() {
        path
    } else {
        entry.join(path)
    })
}

/// Walk up from `start` to the first directory holding a `.git`, and read its
/// HEAD. Pure filesystem reads — no `git` subprocess, so this stays cheap
/// enough to run on every sampler tick.
pub(crate) fn git_info(start: &Path) -> Option<GitInfo> {
    for dir in start.ancestors() {
        let Some(git_dir) = git_dir(dir) else { continue };
        let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) else {
            continue;
        };
        let Some(branch) = parse_head(&head) else { continue };
        let repo = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());
        return Some(GitInfo { repo, branch });
    }
    None
}

// ---------------------------------------------------------------------------
// sampling
// ---------------------------------------------------------------------------

/// Per-session inputs snapshotted under the sessions lock by
/// [`crate::session::PtyManager::context_probes`].
pub(crate) struct Probe {
    pub id: u32,
    /// Foreground process group leader (what the user is actually looking at).
    pub foreground_pid: Option<i32>,
    /// The PTY's direct child (the pane's shell / agent), used as the fallback
    /// when the foreground pid is gone by the time we sample it.
    pub root_pid: Option<u32>,
    /// The cwd the session was spawned with — last-resort fallback.
    pub spawn_cwd: Option<PathBuf>,
}

/// Sample every probe. Uses its own short-lived `System` (rather than the
/// resource sampler's long-lived one) for two reasons: `with_environ` /
/// `with_cwd` are the expensive refresh kinds and must not be folded into the
/// 1s CPU/memory tick, and a fresh instance cannot serve a stale entry for a
/// pid that died between ticks.
pub(crate) fn sample(probes: &[Probe]) -> Vec<PaneContext> {
    if probes.is_empty() {
        return Vec::new();
    }
    let mut pids: Vec<Pid> = Vec::with_capacity(probes.len() * 2);
    for probe in probes {
        if let Some(pid) = probe.foreground_pid.filter(|pid| *pid > 0) {
            pids.push(Pid::from_u32(pid as u32));
        }
        if let Some(pid) = probe.root_pid {
            pids.push(Pid::from_u32(pid));
        }
    }
    pids.sort_unstable();
    pids.dedup();

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        false,
        ProcessRefreshKind::nothing()
            .with_cwd(UpdateKind::Always)
            .with_environ(UpdateKind::Always),
    );

    let home = crate::pty::home_dir().map(PathBuf::from);
    probes
        .iter()
        .map(|probe| build(&system, probe, home.as_deref()))
        .filter(|ctx| !ctx.is_empty())
        .collect()
}

fn proc_cwd(system: &System, pid: u32) -> Option<PathBuf> {
    let cwd = system.process(Pid::from_u32(pid))?.cwd()?;
    (!cwd.as_os_str().is_empty()).then(|| cwd.to_path_buf())
}

fn proc_env(system: &System, pid: u32) -> BTreeMap<String, String> {
    let Some(process) = system.process(Pid::from_u32(pid)) else {
        return BTreeMap::new();
    };
    collect_env(
        process
            .environ()
            .iter()
            .map(|entry| entry.to_string_lossy().into_owned()),
    )
}

fn build(system: &System, probe: &Probe, home: Option<&Path>) -> PaneContext {
    let foreground = probe.foreground_pid.filter(|pid| *pid > 0);
    let cwd = foreground
        .and_then(|pid| proc_cwd(system, pid as u32))
        .or_else(|| probe.root_pid.and_then(|pid| proc_cwd(system, pid)))
        .or_else(|| foreground.and_then(cwd_fallback))
        .or_else(|| probe.spawn_cwd.clone());

    // Prefer the foreground process' environment: it is the one started most
    // recently, so it carries anything direnv exported after the pane opened.
    let env = foreground
        .map(|pid| proc_env(system, pid as u32))
        .filter(|env| !env.is_empty())
        .or_else(|| probe.root_pid.map(|pid| proc_env(system, pid)))
        .unwrap_or_default();

    let git = cwd.as_deref().and_then(git_info);
    PaneContext {
        id: probe.id,
        cwd: cwd.as_deref().map(|path| display_path(path, home)),
        repo: git.as_ref().map(|info| info.repo.clone()),
        branch: git.map(|info| info.branch),
        aws: aws_context(&env),
        model: model_context(&env),
    }
}

/// macOS fallback for the working directory. `proc_pidinfo` (what sysinfo
/// uses) is refused for some processes even at the same uid; `lsof` reports the
/// same fd through a different path and is present on every macOS install.
/// Only runs when sysinfo returned nothing, so the common case costs nothing.
#[cfg(all(unix, not(target_os = "linux")))]
fn cwd_fallback(pid: i32) -> Option<PathBuf> {
    let out = std::process::Command::new("lsof")
        .args(["-a", "-w", "-d", "cwd", "-Fn", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_lsof_cwd(&String::from_utf8_lossy(&out.stdout)).map(PathBuf::from)
}

#[cfg(not(all(unix, not(target_os = "linux"))))]
fn cwd_fallback(_pid: i32) -> Option<PathBuf> {
    None
}

/// `lsof -Fn` field output: one `n<path>` line per reported fd. Only the macOS
/// fallback above calls this; `test` keeps it compiled (and covered) elsewhere.
#[cfg(any(all(unix, not(target_os = "linux")), test))]
pub(crate) fn parse_lsof_cwd(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix('n'))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// settings (managed state, mirrors AgentStatusManager's shape)
// ---------------------------------------------------------------------------

/// Live `pane_context:` settings. Read by the sampler thread every tick, so a
/// config reload takes effect without restarting anything.
pub struct PaneContextSettings {
    enabled: AtomicBool,
    interval_ms: AtomicU64,
}

impl Default for PaneContextSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl PaneContextSettings {
    pub fn new() -> Self {
        let defaults = PaneContextConfig::default();
        PaneContextSettings {
            enabled: AtomicBool::new(defaults.effective_enabled()),
            interval_ms: AtomicU64::new(defaults.effective_interval_ms()),
        }
    }

    fn reconfigure(&self, cfg: Option<&PaneContextConfig>) {
        let owned;
        let effective = match cfg {
            Some(cfg) => cfg,
            None => {
                owned = PaneContextConfig::default();
                &owned
            }
        };
        self.enabled
            .store(effective.effective_enabled(), Ordering::Relaxed);
        self.interval_ms
            .store(effective.effective_interval_ms(), Ordering::Relaxed);
    }

    /// `(enabled, interval_ms)`.
    pub(crate) fn snapshot(&self) -> (bool, u64) {
        (
            self.enabled.load(Ordering::Relaxed),
            self.interval_ms.load(Ordering::Relaxed),
        )
    }
}

/// Fold the loaded config's `pane_context:` block into the managed settings.
/// Called from `load_config`, next to `agent_status::apply`.
pub fn apply<R: Runtime>(app: &AppHandle<R>, config: &Config) {
    if let Some(state) = app.try_state::<PaneContextSettings>() {
        state.reconfigure(config.pane_context.as_ref());
    }
}

/// Sampler-side read of the managed settings. Defaults (enabled, 5s) when the
/// state is unmanaged, which is the case in session unit tests.
pub(crate) fn settings<R: Runtime>(app: &AppHandle<R>) -> (bool, u64) {
    match app.try_state::<PaneContextSettings>() {
        Some(state) => state.snapshot(),
        None => {
            let defaults = PaneContextConfig::default();
            (
                defaults.effective_enabled(),
                defaults.effective_interval_ms(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_has_no_secretish_key() {
        for key in ENV_ALLOWLIST {
            assert!(
                !is_secretish(key),
                "{key} would be filtered by the secret guard; it must not be allowlisted"
            );
        }
    }

    #[test]
    fn credential_shaped_variables_never_survive_collection() {
        let env = collect_env([
            "AWS_PROFILE=ptygrid-dev",
            "AWS_SECRET_ACCESS_KEY=super-secret",
            "AWS_ACCESS_KEY_ID=AKIA...",
            "AWS_SESSION_TOKEN=tok",
            "ANTHROPIC_API_KEY=sk-ant-123",
            "GITHUB_TOKEN=ghp_1",
        ]);
        assert_eq!(env.len(), 1);
        assert_eq!(env.get("AWS_PROFILE").map(String::as_str), Some("ptygrid-dev"));
    }

    #[test]
    fn collect_env_skips_empty_values_and_keeps_the_first_duplicate() {
        let env = collect_env(["AWS_REGION=", "AWS_PROFILE=first", "AWS_PROFILE=second"]);
        assert!(!env.contains_key("AWS_REGION"));
        assert_eq!(env.get("AWS_PROFILE").map(String::as_str), Some("first"));
    }

    #[test]
    fn collect_env_ignores_entries_without_a_separator() {
        assert!(collect_env(["AWS_PROFILE", ""]).is_empty());
    }

    #[test]
    fn aws_profile_precedence_and_region_fallback() {
        let env = collect_env([
            "AWS_VAULT=vaulted",
            "AWS_DEFAULT_PROFILE=legacy",
            "AWS_DEFAULT_REGION=us-east-1",
        ]);
        let aws = aws_context(&env).expect("aws context");
        assert_eq!(aws.profile.as_deref(), Some("vaulted"));
        assert_eq!(aws.region.as_deref(), Some("us-east-1"));

        let env = collect_env(["AWS_PROFILE=explicit", "AWS_VAULT=vaulted"]);
        assert_eq!(
            aws_context(&env).and_then(|aws| aws.profile).as_deref(),
            Some("explicit")
        );
    }

    #[test]
    fn no_aws_variables_means_no_aws_context() {
        assert_eq!(aws_context(&collect_env(["OPENAI_MODEL=gpt"])), None);
    }

    #[test]
    fn router_base_url_reads_as_a_local_anthropic_endpoint() {
        let env = collect_env([
            "ANTHROPIC_BASE_URL=http://127.0.0.1:3456",
            "ANTHROPIC_MODEL=qwen3-coder",
        ]);
        let model = model_context(&env).expect("model context");
        assert_eq!(model.provider, "anthropic");
        assert_eq!(model.endpoint.as_deref(), Some("127.0.0.1:3456"));
        assert_eq!(model.model.as_deref(), Some("qwen3-coder"));
        assert!(model.local);
    }

    #[test]
    fn a_remote_endpoint_is_not_marked_local() {
        let env = collect_env(["ANTHROPIC_BASE_URL=https://api.anthropic.com"]);
        let model = model_context(&env).expect("model context");
        assert_eq!(model.endpoint.as_deref(), Some("api.anthropic.com"));
        assert!(!model.local);
        assert_eq!(model.model, None);
    }

    #[test]
    fn anthropic_family_wins_over_openai_and_ollama() {
        let env = collect_env([
            "ANTHROPIC_MODEL=claude",
            "OPENAI_MODEL=gpt",
            "OLLAMA_HOST=http://127.0.0.1:11434",
        ]);
        assert_eq!(model_context(&env).unwrap().provider, "anthropic");

        let env = collect_env(["OPENAI_MODEL=gpt", "OLLAMA_HOST=http://127.0.0.1:11434"]);
        assert_eq!(model_context(&env).unwrap().provider, "openai");

        let env = collect_env(["OLLAMA_HOST=http://127.0.0.1:11434"]);
        let ollama = model_context(&env).unwrap();
        assert_eq!(ollama.provider, "ollama");
        assert!(ollama.local);
    }

    #[test]
    fn endpoint_label_strips_scheme_userinfo_and_path() {
        assert_eq!(
            endpoint_label("https://user:tok@proxy.internal:8443/v1?x=1").as_deref(),
            Some("proxy.internal:8443")
        );
        assert_eq!(endpoint_label("127.0.0.1:3456").as_deref(), Some("127.0.0.1:3456"));
        assert_eq!(endpoint_label("   ").as_deref(), None);
        assert_eq!(endpoint_label("https:///v1").as_deref(), None);
    }

    #[test]
    fn ipv6_authority_resolves_to_its_host() {
        assert_eq!(host_of("[::1]:8080"), "::1");
        assert_eq!(host_of("api.example.com"), "api.example.com");
        assert!(is_local_host(host_of("[::1]:8080")));
        assert!(is_local_host("127.0.0.53"));
        assert!(!is_local_host("10.0.0.1"));
    }

    #[test]
    fn head_parses_branches_detached_shas_and_nested_refs() {
        assert_eq!(parse_head("ref: refs/heads/main\n").as_deref(), Some("main"));
        assert_eq!(
            parse_head("ref: refs/heads/feat/pane-context\n").as_deref(),
            Some("feat/pane-context")
        );
        assert_eq!(
            parse_head("53b9e9a1c0ffee00deadbeef1234567890abcdef\n").as_deref(),
            Some("@53b9e9a")
        );
        assert_eq!(parse_head("").as_deref(), None);
        assert_eq!(parse_head("not a ref line").as_deref(), None);
    }

    #[test]
    fn gitdir_pointer_file_is_parsed() {
        assert_eq!(
            parse_gitdir_file("gitdir: /repo/.git/worktrees/codex\n"),
            Some("/repo/.git/worktrees/codex")
        );
        assert_eq!(parse_gitdir_file("gitdir:\n"), None);
        assert_eq!(parse_gitdir_file("something else"), None);
    }

    #[test]
    fn display_path_collapses_home_only_at_a_boundary() {
        let home = Path::new("/Users/h.yamamoto");
        assert_eq!(
            display_path(Path::new("/Users/h.yamamoto/works/ptygrid"), Some(home)),
            "~/works/ptygrid"
        );
        assert_eq!(display_path(Path::new("/Users/h.yamamoto"), Some(home)), "~");
        // A sibling that merely starts with the same characters must not collapse.
        assert_eq!(
            display_path(Path::new("/Users/h.yamamoto2/x"), Some(home)),
            "/Users/h.yamamoto2/x"
        );
        assert_eq!(display_path(Path::new("/tmp/x"), None), "/tmp/x");
    }

    #[test]
    fn lsof_field_output_yields_the_cwd_line() {
        assert_eq!(
            parse_lsof_cwd("p4321\nfcwd\nn/Users/h.yamamoto/works/project/ptygrid\n").as_deref(),
            Some("/Users/h.yamamoto/works/project/ptygrid")
        );
        assert_eq!(parse_lsof_cwd("p4321\nfcwd\n"), None);
    }

    #[test]
    fn git_info_reads_a_real_repository_layout() {
        let root = std::env::temp_dir().join(format!("ptygrid-ctx-{}", std::process::id()));
        let nested = root.join("src").join("lib");
        std::fs::create_dir_all(nested.join("deep")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let info = git_info(&nested).expect("git info from a nested directory");
        assert_eq!(info.branch, "main");
        assert_eq!(
            info.repo,
            root.file_name().unwrap().to_string_lossy().into_owned()
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// End-to-end over the real sysinfo path: spawn a child with a known
    /// environment, sample it, and assert both halves of the contract — the
    /// allowlisted value comes through, and the credential-shaped one next to
    /// it does not appear anywhere in the serialized payload.
    ///
    /// Hosts that refuse process introspection (sandboxed CI, hardened macOS)
    /// resolve nothing at all; the assertions are about what we expose *when*
    /// resolution works, so an empty result is not a failure here. The pure
    /// filtering rules are covered unconditionally by the tests above.
    #[cfg(unix)]
    #[test]
    fn sampling_a_child_exposes_the_allowlisted_value_and_nothing_else() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .env("AWS_PROFILE", "ptygrid-test-profile")
            .env("AWS_SECRET_ACCESS_KEY", "must-not-appear")
            .env("ANTHROPIC_API_KEY", "must-not-appear-either")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id() as i32;
        let sampled = sample(&[Probe {
            id: 7,
            foreground_pid: Some(pid),
            root_pid: None,
            spawn_cwd: None,
        }]);
        let _ = child.kill();
        let _ = child.wait();

        let Some(ctx) = sampled.iter().find(|ctx| ctx.id == 7) else {
            return; // introspection unavailable on this host
        };
        if let Some(aws) = &ctx.aws {
            assert_eq!(aws.profile.as_deref(), Some("ptygrid-test-profile"));
        }
        let json = serde_json::to_string(ctx).expect("PaneContext serializes");
        assert!(
            !json.contains("must-not-appear"),
            "a credential-shaped variable reached the wire: {json}"
        );
    }

    /// The sampler must survive a pid that is gone (or was never valid) and
    /// simply resolve nothing for it, rather than panicking on the sampler
    /// thread and taking the resource monitor down with it.
    #[test]
    fn sampling_a_dead_pid_yields_only_the_spawn_cwd_fallback() {
        let sampled = sample(&[
            Probe {
                id: 1,
                foreground_pid: Some(-1),
                root_pid: None,
                spawn_cwd: None,
            },
            Probe {
                id: 2,
                foreground_pid: None,
                root_pid: None,
                spawn_cwd: Some(PathBuf::from("/tmp")),
            },
        ]);
        // #1 resolved nothing at all and is dropped from the batch entirely.
        assert!(sampled.iter().all(|ctx| ctx.id != 1));
        let fallback = sampled.iter().find(|ctx| ctx.id == 2).expect("id 2");
        assert_eq!(fallback.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn empty_context_is_dropped_from_the_batch() {
        assert!(PaneContext { id: 1, ..Default::default() }.is_empty());
        assert!(!PaneContext {
            id: 1,
            branch: Some("main".into()),
            ..Default::default()
        }
        .is_empty());
    }
}
