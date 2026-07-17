// ptygrid.yml (legacy: mterm.yml) configuration: parsing (serde_norway), ${VAR} expansion,
// relative-cwd resolution, and the file watcher (notify) that emits
// `config-changed` events per the Phase 1 contract.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
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
fn launch_dir() -> Option<PathBuf> {
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
fn resolve_config_path_pure(
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
}
