//! Versioned, app-global settings (not project-scoped).
//!
//! Currently holds only `projectsRoot`: a fixed parent directory the toolbar
//! "bulk cd" popover uses to list project folders and to resolve bare relative
//! names (e.g. `notemake` -> `<root>/notemake`). The root is stored verbatim as
//! the user typed it (a leading `~` is kept, not expanded, so the file stays
//! portable across machines); `~` is expanded to the home directory only when a
//! path must actually be touched (validation / listing).
//!
//! Same on-disk discipline as `project_state`: a versioned JSON blob, unknown
//! future versions are refused rather than silently opened, and writes are
//! atomic (temp file + rename).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::pty::home_dir;

pub const SETTINGS_VERSION: u32 = 1;
/// Upper bound on directory names returned by `list_project_dirs`; anything
/// past this is dropped and `truncated` is set so the UI stays bounded.
const MAX_PROJECT_DIRS: usize = 200;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub version: u32,
    /// Verbatim user string (may start with `~`). `None` when never set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects_root: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsRoot {
    pub root: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDirs {
    pub root: String,
    pub dirs: Vec<String>,
    pub truncated: bool,
}

fn settings_path(app_data: &Path) -> PathBuf {
    app_data.join("app-settings.json")
}

/// Expand a leading `~` / `~/` to the home directory. A `~name` form (named
/// home) is not special-cased and is returned as-is. Non-tilde paths pass
/// through unchanged.
fn expand_tilde(input: &str) -> Result<PathBuf, String> {
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

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "app settings path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create app settings directory: {e}"))?;
    let suffix = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".settings-{}-{suffix}.tmp", std::process::id()));
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| format!("cannot encode app settings: {e}"))?;
    std::fs::write(&temp, bytes).map_err(|e| format!("cannot write app settings: {e}"))?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace app settings: {e}")
    })
}

fn read_settings(app_data: &Path) -> Result<AppSettings, String> {
    let path = settings_path(app_data);
    if !path.is_file() {
        return Ok(AppSettings {
            version: SETTINGS_VERSION,
            projects_root: None,
        });
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read app settings: {e}"))?;
    let settings: AppSettings =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid app settings: {e}"))?;
    if settings.version != SETTINGS_VERSION {
        return Err(format!(
            "unsupported app settings version {} (expected {SETTINGS_VERSION})",
            settings.version
        ));
    }
    Ok(settings)
}

fn get_root_at(app_data: &Path) -> Result<ProjectsRoot, String> {
    let settings = read_settings(app_data)?;
    Ok(ProjectsRoot {
        root: settings.projects_root,
    })
}

fn set_root_at(app_data: &Path, root: String) -> Result<ProjectsRoot, String> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return Err("projects root must not be empty".to_string());
    }
    let expanded = expand_tilde(trimmed)?;
    let meta = std::fs::metadata(&expanded)
        .map_err(|e| format!("projects root {} is not accessible: {e}", expanded.display()))?;
    if !meta.is_dir() {
        return Err(format!(
            "projects root {} is not a directory",
            expanded.display()
        ));
    }
    let settings = AppSettings {
        version: SETTINGS_VERSION,
        projects_root: Some(trimmed.to_string()),
    };
    write_json_atomic(&settings_path(app_data), &settings)?;
    Ok(ProjectsRoot {
        root: settings.projects_root,
    })
}

fn list_dirs_at(app_data: &Path) -> Result<ProjectDirs, String> {
    let settings = read_settings(app_data)?;
    let root = settings
        .projects_root
        .ok_or_else(|| "projects root is not set".to_string())?;
    let expanded = expand_tilde(&root)?;
    let meta = std::fs::metadata(&expanded)
        .map_err(|e| format!("projects root {} is not accessible: {e}", expanded.display()))?;
    if !meta.is_dir() {
        return Err(format!(
            "projects root {} is not a directory",
            expanded.display()
        ));
    }
    let entries =
        std::fs::read_dir(&expanded).map_err(|e| format!("cannot read projects root: {e}"))?;
    let mut dirs: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read projects root entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden (dot) entries.
        if name.starts_with('.') {
            continue;
        }
        // Only directories (follow symlinks via file_type()->is_dir fallback to metadata).
        let is_dir = match entry.file_type() {
            Ok(ft) if ft.is_dir() => true,
            Ok(ft) if ft.is_symlink() => entry.path().is_dir(),
            _ => false,
        };
        if is_dir {
            dirs.push(name);
        }
    }
    dirs.sort();
    let truncated = dirs.len() > MAX_PROJECT_DIRS;
    if truncated {
        dirs.truncate(MAX_PROJECT_DIRS);
    }
    Ok(ProjectDirs {
        root,
        dirs,
        truncated,
    })
}

fn app_data_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("cannot determine app data directory: {e}"))
}

pub fn get_root<R: Runtime>(app: &AppHandle<R>) -> Result<ProjectsRoot, String> {
    get_root_at(&app_data_dir(app)?)
}

pub fn set_root<R: Runtime>(app: &AppHandle<R>, root: String) -> Result<ProjectsRoot, String> {
    set_root_at(&app_data_dir(app)?, root)
}

pub fn list_dirs<R: Runtime>(app: &AppHandle<R>) -> Result<ProjectDirs, String> {
    list_dirs_at(&app_data_dir(app)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ptygrid-settings-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn round_trip_persists_verbatim_root() {
        let root = temp_root("roundtrip");
        let app_data = root.join("app-data");
        let projects = root.join("projects");
        std::fs::create_dir_all(&projects).unwrap();

        // Nothing saved yet.
        assert_eq!(get_root_at(&app_data).unwrap().root, None);

        let saved = set_root_at(&app_data, projects.display().to_string()).unwrap();
        assert_eq!(saved.root.as_deref(), Some(projects.display().to_string()).as_deref());
        assert_eq!(
            get_root_at(&app_data).unwrap().root.as_deref(),
            Some(projects.display().to_string().as_str())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unknown_settings_version() {
        let root = temp_root("version");
        let app_data = root.join("app-data");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::write(
            settings_path(&app_data),
            br#"{ "version": 2, "projectsRoot": "/tmp" }"#,
        )
        .unwrap();
        let err = read_settings(&app_data).unwrap_err();
        assert!(err.contains("version"), "got: {err}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn set_root_rejects_missing_or_non_dir() {
        let root = temp_root("nonexist");
        let app_data = root.join("app-data");
        std::fs::create_dir_all(&app_data).unwrap();

        // Missing directory.
        let missing = root.join("does-not-exist");
        assert!(set_root_at(&app_data, missing.display().to_string()).is_err());

        // A file, not a directory.
        let file = root.join("a-file");
        std::fs::write(&file, b"x").unwrap();
        let err = set_root_at(&app_data, file.display().to_string()).unwrap_err();
        assert!(err.contains("not a directory"), "got: {err}");

        // Empty string.
        assert!(set_root_at(&app_data, "   ".to_string()).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tilde_expands_to_home_for_validation() {
        // Point HOME at a temp dir so `~` resolves to a real, existing dir.
        let root = temp_root("tilde");
        let home = root.join("home");
        let project = home.join("works").join("project");
        std::fs::create_dir_all(&project).unwrap();
        let app_data = root.join("app-data");
        std::fs::create_dir_all(&app_data).unwrap();

        // SAFETY: single-threaded test manipulating process env.
        let prev = std::env::var("HOME").ok();
        unsafe {
            std::env::set_var("HOME", &home);
        }
        // `~/works/project` must validate and be stored verbatim (tilde kept).
        let saved = set_root_at(&app_data, "~/works/project".to_string()).unwrap();
        assert_eq!(saved.root.as_deref(), Some("~/works/project"));
        assert_eq!(
            expand_tilde("~/works/project").unwrap(),
            project,
            "tilde should expand to HOME"
        );
        // A tilde path that does not exist is rejected.
        assert!(set_root_at(&app_data, "~/nope-nope".to_string()).is_err());
        unsafe {
            match prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn list_returns_sorted_non_hidden_dirs_only() {
        let root = temp_root("list");
        let app_data = root.join("app-data");
        let projects = root.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        // dirs
        std::fs::create_dir_all(projects.join("zebra")).unwrap();
        std::fs::create_dir_all(projects.join("alpha")).unwrap();
        std::fs::create_dir_all(projects.join("notemake")).unwrap();
        // hidden dir (excluded)
        std::fs::create_dir_all(projects.join(".hidden")).unwrap();
        // files (excluded)
        std::fs::write(projects.join("README.md"), b"x").unwrap();
        std::fs::write(projects.join("alpha.txt"), b"x").unwrap();

        set_root_at(&app_data, projects.display().to_string()).unwrap();
        let listed = list_dirs_at(&app_data).unwrap();
        assert_eq!(listed.dirs, vec!["alpha", "notemake", "zebra"]);
        assert!(!listed.truncated);
        assert_eq!(listed.root, projects.display().to_string());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn list_caps_at_max_and_flags_truncated() {
        let root = temp_root("cap");
        let app_data = root.join("app-data");
        let projects = root.join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        for i in 0..(MAX_PROJECT_DIRS + 5) {
            std::fs::create_dir_all(projects.join(format!("d{i:04}"))).unwrap();
        }
        set_root_at(&app_data, projects.display().to_string()).unwrap();
        let listed = list_dirs_at(&app_data).unwrap();
        assert_eq!(listed.dirs.len(), MAX_PROJECT_DIRS);
        assert!(listed.truncated);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn list_errors_when_root_unset() {
        let root = temp_root("unset");
        let app_data = root.join("app-data");
        std::fs::create_dir_all(&app_data).unwrap();
        let err = list_dirs_at(&app_data).unwrap_err();
        assert!(err.contains("not set"), "got: {err}");
        let _ = std::fs::remove_dir_all(root);
    }
}
