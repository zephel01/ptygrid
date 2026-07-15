// mterm.yml configuration: parsing (serde_norway), ${VAR} expansion,
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
/// Phase 2 adds the optional `queen:` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub agents: Vec<AgentDef>,
    #[serde(default)]
    pub processes: Vec<AgentDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queen: Option<QueenConfig>,
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
    pub config: Config,
}

/// Parse mterm.yml text. Errors are passed through as strings (serde_norway
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

/// Resolve a definition's cwd against the directory containing mterm.yml.
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
/// active mterm.yml watcher.
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

    /// Implements the `load_config` command: read <dir>/mterm.yml
    /// (dir omitted -> previous dir, first time -> current dir), store the
    /// config, and (re)start the file watcher.
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

        let path = dir_path.join("mterm.yml");
        if !path.is_file() {
            return Err(format!("not_found: {}", path.display()));
        }

        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read failed: {e}"))?;
        let config = parse_config(&text)?;

        // Replace any existing watcher (dropping the old one stops it and
        // ends its throttle thread via channel disconnect).
        let watcher = start_watcher(app.clone(), &dir_path, &path)?;
        inner.dir = Some(dir_path);
        inner.config = Some(config.clone());
        inner.watcher = Some(watcher);

        Ok(ConfigInfo {
            path: path.display().to_string(),
            config,
        })
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

/// Watch the config directory (non-recursive) and emit `config-changed`
/// for events touching mterm.yml. Raw notify events are coalesced by a
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
}
