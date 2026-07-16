//! Versioned, project-scoped UI/session state (Phase 3.4).
//!
//! Persist only logical references. Commands and expanded environment values
//! deliberately never cross this boundary; definitions are resolved again
//! from the current ptygrid.yml when a session is resumed.

use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::worktree::WorktreeInfo;

pub const STATE_VERSION: u32 = 1;
const MAX_SAVED_SESSIONS: usize = 9;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LogicalSession {
    Definition {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        worktree: Option<WorktreeInfo>,
    },
    Shell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    pub version: u32,
    pub config_dir: String,
    /// `auto | 1 | 2 | 3`; stored as text for a stable JSON wire shape.
    pub layout_mode: String,
    pub sessions: Vec<LogicalSession>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximized_index: Option<usize>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LastProject {
    version: u32,
    config_dir: String,
}

/// Phase 4.1/4.2: teammate panes are ephemeral and must never be persisted —
/// they are re-created by the lead on resume, never as a logical resume target.
/// This covers both observe transcript panes (Phase 4.1) and host-teammate PTY
/// panes (Phase 4.2): the exclusion is keyed on the teammate marker, NOT on the
/// session kind, so a host teammate (a `pty` session carrying teammate meta) is
/// excluded too. Given `(pane_id, is_teammate)` pairs, return the ids eligible
/// for persistence. The frontend applies the same rule (any pane whose
/// `SessionInfo.teammate` is set) before building a `ProjectState`; keeping it
/// here makes the exclusion invariant unit-testable and documents that
/// `LogicalSession` has no teammate variant by construction.
#[cfg_attr(not(test), allow(dead_code))]
pub fn persistable_pane_ids(panes: &[(u32, bool)]) -> Vec<u32> {
    panes
        .iter()
        .filter(|(_, is_teammate)| !is_teammate)
        .map(|(id, _)| *id)
        .collect()
}

fn project_key(path: &Path) -> String {
    struct Fnv64(u64);
    impl Hasher for Fnv64 {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(0x100000001b3);
            }
        }
    }
    let mut hasher = Fnv64(0xcbf29ce484222325);
    hasher.write(path.to_string_lossy().as_bytes());
    format!("{:016x}", hasher.finish())
}

fn state_root(app_data: &Path) -> PathBuf {
    app_data.join("project-state")
}

fn project_path(app_data: &Path, config_dir: &Path) -> PathBuf {
    state_root(app_data)
        .join("projects")
        .join(format!("{}.json", project_key(config_dir)))
}

fn canonical_dir(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|e| format!("cannot resolve project directory {}: {e}", path.display()))
}

fn validate(state: &ProjectState) -> Result<(), String> {
    if state.version != STATE_VERSION {
        return Err(format!(
            "unsupported project state version {} (expected {STATE_VERSION})",
            state.version
        ));
    }
    if !matches!(state.layout_mode.as_str(), "auto" | "1" | "2" | "3") {
        return Err(format!("invalid layout mode '{}'", state.layout_mode));
    }
    if state.sessions.len() > MAX_SAVED_SESSIONS {
        return Err(format!(
            "project state has too many sessions (max {MAX_SAVED_SESSIONS})"
        ));
    }
    if state
        .maximized_index
        .is_some_and(|index| index >= state.sessions.len())
    {
        return Err("maximized session index is out of range".to_string());
    }
    for session in &state.sessions {
        if let LogicalSession::Definition { name, .. } = session {
            if name.trim().is_empty() || name.len() > 256 {
                return Err("invalid saved definition name".to_string());
            }
        }
    }
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "project state path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create project state directory: {e}"))?;
    let suffix = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".state-{}-{suffix}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| format!("cannot encode project state: {e}"))?;
    std::fs::write(&temp, bytes).map_err(|e| format!("cannot write project state: {e}"))?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace project state: {e}")
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read project state: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid project state: {e}"))
}

fn save_at(app_data: &Path, mut state: ProjectState) -> Result<(), String> {
    validate(&state)?;
    let config_dir = canonical_dir(Path::new(&state.config_dir))?;
    state.config_dir = config_dir.display().to_string();
    write_json_atomic(&project_path(app_data, &config_dir), &state)?;
    write_json_atomic(
        &state_root(app_data).join("last-project.json"),
        &LastProject {
            version: STATE_VERSION,
            config_dir: state.config_dir,
        },
    )
}

fn load_at(app_data: &Path, dir: Option<&Path>) -> Result<Option<ProjectState>, String> {
    let config_dir = match dir {
        Some(dir) => canonical_dir(dir)?,
        None => {
            let pointer_path = state_root(app_data).join("last-project.json");
            if !pointer_path.is_file() {
                return Ok(None);
            }
            let pointer: LastProject = read_json(&pointer_path)?;
            if pointer.version != STATE_VERSION {
                return Err(format!(
                    "unsupported last-project version {} (expected {STATE_VERSION})",
                    pointer.version
                ));
            }
            canonical_dir(Path::new(&pointer.config_dir))?
        }
    };
    let path = project_path(app_data, &config_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let mut state: ProjectState = read_json(&path)?;
    validate(&state)?;
    state.config_dir = config_dir.display().to_string();
    Ok(Some(state))
}

pub fn save<R: Runtime>(app: &AppHandle<R>, state: ProjectState) -> Result<(), String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot determine app data directory: {e}"))?;
    save_at(&app_data, state)
}

pub fn load<R: Runtime>(
    app: &AppHandle<R>,
    dir: Option<String>,
) -> Result<Option<ProjectState>, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot determine app data directory: {e}"))?;
    load_at(&app_data, dir.as_deref().map(Path::new))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ptygrid-state-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn round_trip_contains_only_logical_session_references() {
        let root = temp_root("roundtrip");
        let app_data = root.join("app-data");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let state = ProjectState {
            version: STATE_VERSION,
            config_dir: project.display().to_string(),
            layout_mode: "2".to_string(),
            sessions: vec![
                LogicalSession::Definition {
                    name: "codex".to_string(),
                    worktree: None,
                },
                LogicalSession::Shell,
            ],
            maximized_index: Some(0),
        };
        save_at(&app_data, state).unwrap();
        let restored = load_at(&app_data, None).unwrap().unwrap();
        assert_eq!(restored.layout_mode, "2");
        assert_eq!(restored.sessions.len(), 2);

        let stored =
            std::fs::read_to_string(project_path(&app_data, &project.canonicalize().unwrap()))
                .unwrap();
        assert!(!stored.contains("cmd"));
        assert!(!stored.contains("env"));
        assert!(!stored.contains("secret"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn teammate_panes_are_excluded_from_persistence() {
        // A mix of ordinary panes (false) and teammate panes (true) — the
        // teammate marker covers both observe transcripts and host-teammate
        // PTYs. Only the ordinary panes survive, order preserved.
        let panes = [(1, false), (2, true), (3, false), (4, true)];
        assert_eq!(persistable_pane_ids(&panes), vec![1, 3]);
        // All teammate => nothing persisted.
        assert!(persistable_pane_ids(&[(7, true), (8, true)]).is_empty());
        // All ordinary => everything persisted.
        assert_eq!(persistable_pane_ids(&[(5, false), (6, false)]), vec![5, 6]);
    }

    #[test]
    fn rejects_unknown_versions_and_invalid_layouts() {
        let base = ProjectState {
            version: STATE_VERSION + 1,
            config_dir: ".".to_string(),
            layout_mode: "auto".to_string(),
            sessions: Vec::new(),
            maximized_index: None,
        };
        assert!(validate(&base).unwrap_err().contains("version"));
        let mut invalid_layout = base;
        invalid_layout.version = STATE_VERSION;
        invalid_layout.layout_mode = "4".to_string();
        assert!(validate(&invalid_layout).unwrap_err().contains("layout"));
    }
}
