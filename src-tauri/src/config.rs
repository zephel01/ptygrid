// ptygrid.yml (legacy: mterm.yml) configuration: parsing (serde_norway), ${VAR} expansion,
// relative-cwd resolution, and the file watcher (notify) that emits
// `config-changed` events per the Phase 1 contract.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// `Config { project?, agents, processes }` — processes defaults to empty Vec.
/// Phase 2 adds the optional `queen:` block; Phase 4.0 the `teammates:` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub agents: Vec<AgentDef>,
    #[serde(default)]
    pub processes: Vec<AgentDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queen: Option<QueenConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teammates: Option<TeammatesConfig>,
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
        self.port.unwrap_or(crate::queen::DEFAULT_PORT)
    }
}

/// Phase 4.0 global `teammates:` block. Governs whether teammate hook events
/// are emitted/toasted and where `register_teammate_hooks` writes by default.
/// `agents[].teams` (per-agent teammate config) is Phase 4.1, not parsed here.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct TeammatesConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_notifications: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_max_panes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks_scope: Option<HooksScope>,
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

/// Phase 4.1 per-agent `teams:` block. Governs whether this lead's teammates /
/// subagents get read-only transcript panes auto-generated on `SubagentStart`.
/// Everything is optional; omitting the block leaves the agent unchanged.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct AgentTeamsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<TeamsMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_panes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_tail: Option<bool>,
}

impl AgentTeamsConfig {
    /// Default false: this lead does not produce teammate panes.
    pub fn effective_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }
    /// Default `observe`. In Phase 4.1 `host` behaves identically to `observe`
    /// (a read-only transcript pane); the real PTY host lands in Phase 4.2, so
    /// this accessor is part of the stable schema surface but not yet a
    /// behavior branch.
    #[allow(dead_code)]
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
}

/// `observe | host` (default observe). Phase 4.1 treats `host` as `observe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TeamsMode {
    #[default]
    Observe,
    Host,
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

/// Return type of the `load_config` command.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigInfo {
    pub path: String,
    pub dir: String,
    pub config: Config,
}

/// Parse ptygrid.yml text. Errors are passed through as strings (serde_norway
/// messages include line/column information).
pub fn parse_config(text: &str) -> Result<Config, String> {
    serde_norway::from_str(text).map_err(|e| e.to_string())
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

    /// Implements the `load_config` command: read <dir>/ptygrid.yml (falling
    /// back to the legacy <dir>/mterm.yml; dir omitted -> previous dir, first
    /// time -> current dir), store the config, and (re)start the file watcher.
    pub fn load(&self, app: &AppHandle, dir: Option<String>) -> Result<ConfigInfo, String> {
        let mut inner = self.lock();

        let dir_path = match dir {
            Some(d) => PathBuf::from(d),
            None => match inner.dir.clone() {
                Some(prev) => prev,
                None => std::env::current_dir()
                    .map_err(|e| format!("cannot determine current dir: {e}"))?,
            },
        };

        let path = resolve_config_path(&dir_path)?;

        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read failed: {e}"))?;
        let config = parse_config(&text)?;

        // Replace any existing watcher (dropping the old one stops it and
        // ends its throttle thread via channel disconnect).
        let watcher = start_watcher(app.clone(), &dir_path, &path)?;
        let dir = dir_path.display().to_string();
        inner.dir = Some(dir_path);
        inner.config = Some(config.clone());
        inner.watcher = Some(watcher);

        Ok(ConfigInfo {
            path: path.display().to_string(),
            dir,
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

/// Resolve the config file inside `dir`: prefer `ptygrid.yml`, fall back to
/// the legacy `mterm.yml`. When both exist, `ptygrid.yml` wins (the watcher
/// then only follows the file that was actually loaded).
fn resolve_config_path(dir: &Path) -> Result<PathBuf, String> {
    let preferred = dir.join(CONFIG_FILE_NAME);
    if preferred.is_file() {
        return Ok(preferred);
    }
    let legacy = dir.join(LEGACY_CONFIG_FILE_NAME);
    if legacy.is_file() {
        return Ok(legacy);
    }
    Err(format!(
        "not_found: {} (also tried legacy {})",
        preferred.display(),
        legacy.display()
    ))
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
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                let relevant = event
                    .paths
                    .iter()
                    .any(|p| p.file_name() == file_name.as_deref())
                    || event.paths.is_empty();
                if relevant {
                    let _ = tx.send(());
                }
            }
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
        let t = cfg.agents[0].teams.unwrap();
        assert!(!t.effective_enabled());
        assert_eq!(t.effective_mode(), TeamsMode::Observe);
        assert_eq!(t.effective_max_panes(), 3);
        assert!(t.effective_transcript_tail());

        // Explicit values incl. host mode and an out-of-range max_panes clamp.
        let cfg = parse_config(
            "agents:\n  - name: claude\n    cmd: claude\n    teams:\n      enabled: true\n      mode: host\n      max_panes: 99\n      transcript_tail: false\n",
        )
        .unwrap();
        let t = cfg.agents[0].teams.unwrap();
        assert!(t.effective_enabled());
        assert_eq!(t.effective_mode(), TeamsMode::Host);
        assert_eq!(t.effective_max_panes(), 9);
        assert!(!t.effective_transcript_tail());
    }

    #[test]
    fn agent_teams_block_ignores_unknown_fields() {
        // Unknown keys are ignored; known ones still parse.
        let cfg = parse_config(
            "agents:\n  - name: claude\n    cmd: claude\n    teams:\n      enabled: true\n      teammate_binaries: [claude]\n      future_flag: 7\n",
        )
        .unwrap();
        assert!(cfg.agents[0].teams.unwrap().effective_enabled());
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

        // Neither file: clear error naming both candidates.
        let err = resolve_config_path(&dir).unwrap_err();
        assert!(err.contains("ptygrid.yml") && err.contains("mterm.yml"));

        // Legacy only: mterm.yml is accepted.
        std::fs::write(dir.join(LEGACY_CONFIG_FILE_NAME), "agents: []\n").unwrap();
        assert_eq!(
            resolve_config_path(&dir).unwrap(),
            dir.join(LEGACY_CONFIG_FILE_NAME)
        );

        // Both present: ptygrid.yml wins.
        std::fs::write(dir.join(CONFIG_FILE_NAME), "agents: []\n").unwrap();
        assert_eq!(resolve_config_path(&dir).unwrap(), dir.join(CONFIG_FILE_NAME));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
