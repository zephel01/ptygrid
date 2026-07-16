//! Optional per-agent Git linked-worktree isolation (Phase 3.3).

use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

use crate::config::{self, AgentDef};

static NEXT_WORKTREE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub name: String,
    pub repo_root: String,
    pub path: String,
    pub branch: String,
    pub base: String,
    pub locked: bool,
}

pub struct PreparedWorktree {
    pub cwd: PathBuf,
    pub info: WorktreeInfo,
}

fn git_output<I, S>(dir: &Path, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("git executable failed: {e}"))
}

fn checked_git<I, S>(dir: &Path, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = git_output(dir, args)?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("git exited with status {}", output.status)
    };
    Err(detail)
}

fn common_git_dir(repo_root: &Path) -> Result<PathBuf, String> {
    let output = checked_git(repo_root, ["rev-parse", "--git-common-dir"])?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        return Err("git returned an empty common directory".to_string());
    }
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    };
    path.canonicalize()
        .map_err(|e| format!("cannot resolve git common directory: {e}"))
}

fn slug(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut previous_dash = false;
    for ch in input.chars() {
        let valid = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_');
        if valid {
            result.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash && !result.is_empty() {
            result.push('-');
            previous_dash = true;
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "agent".to_string()
    } else {
        result.chars().take(48).collect()
    }
}

fn project_key(common_dir: &Path) -> String {
    // Stable, dependency-free FNV-1a over the canonical common git-dir path.
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
    hasher.write(common_dir.to_string_lossy().as_bytes());
    format!("{:016x}", hasher.finish())
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = NEXT_WORKTREE.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{counter:x}")
}

fn run_setup(
    setup: &str,
    cwd: &Path,
    env: &[(String, String)],
    worktree_path: &Path,
) -> Result<(), String> {
    if setup.trim().is_empty() {
        return Ok(());
    }
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", setup]);
        command
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("powershell.exe");
        command.args(["-Command", setup]);
        command
    };
    command.current_dir(cwd);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().map_err(|e| {
        format!(
            "worktree created at {} but setup failed to start: {e}",
            worktree_path.display()
        )
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(format!(
        "worktree created and kept at {}, but setup failed: {}",
        worktree_path.display(),
        if detail.is_empty() {
            output.status.to_string()
        } else {
            detail
        }
    ))
}

pub fn prepare_for_agent<R: Runtime>(
    app: &AppHandle<R>,
    def: &AgentDef,
    config_dir: &Path,
    env: &[(String, String)],
) -> Result<Option<PreparedWorktree>, String> {
    let Some(options) = def.worktree.as_ref() else {
        return Ok(None);
    };
    if !options.effective_enabled() {
        return Ok(None);
    }
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot determine app data directory: {e}"))?;
    prepare_at(&app_data, def, config_dir, env)
}

fn prepare_at(
    app_data: &Path,
    def: &AgentDef,
    config_dir: &Path,
    env: &[(String, String)],
) -> Result<Option<PreparedWorktree>, String> {
    let Some(options) = def.worktree.as_ref() else {
        return Ok(None);
    };
    if !options.effective_enabled() {
        return Ok(None);
    }

    let original_cwd = config::resolve_cwd(config_dir, def.cwd.as_deref())
        .canonicalize()
        .map_err(|e| format!("cannot resolve agent cwd: {e}"))?;
    let repo_root = crate::git_service::repository_root(&original_cwd)?
        .canonicalize()
        .map_err(|e| format!("cannot resolve repository root: {e}"))?;
    let relative_cwd = original_cwd.strip_prefix(&repo_root).map_err(|_| {
        format!(
            "agent cwd {} is outside repository {}",
            original_cwd.display(),
            repo_root.display()
        )
    })?;
    let common_dir = common_git_dir(&repo_root)?;
    let agent_slug = slug(&def.name);
    let suffix = unique_suffix();
    let name = format!("{agent_slug}-{suffix}");
    let branch = format!("ptygrid/{agent_slug}/{suffix}");
    let base = options.effective_base().to_string();
    let worktree_root = app_data
        .join("worktrees")
        .join(project_key(&common_dir))
        .join(&name);
    let parent = worktree_root
        .parent()
        .ok_or_else(|| "worktree path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create worktree directory: {e}"))?;

    checked_git(
        &repo_root,
        [
            "worktree",
            "add",
            "--lock",
            "--reason",
            "ptygrid active session",
            "-b",
            &branch,
            worktree_root.to_string_lossy().as_ref(),
            &base,
        ],
    )
    .map_err(|e| format!("worktree creation failed: {e}"))?;

    let worktree_root = worktree_root
        .canonicalize()
        .map_err(|e| format!("worktree was created but its path cannot be resolved: {e}"))?;

    let cwd = worktree_root.join(relative_cwd);
    if !cwd.is_dir() {
        return Err(format!(
            "worktree created and kept at {}, but cwd {} does not exist in it",
            worktree_root.display(),
            cwd.display()
        ));
    }
    if let Some(setup) = options.setup.as_deref() {
        run_setup(setup, &cwd, env, &worktree_root)?;
    }

    Ok(Some(PreparedWorktree {
        cwd,
        info: WorktreeInfo {
            name,
            repo_root: repo_root.display().to_string(),
            path: worktree_root.display().to_string(),
            branch,
            base,
            locked: true,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_config;

    #[test]
    fn slug_is_safe_and_bounded() {
        assert_eq!(slug("Codex Review #1"), "codex-review-1");
        assert_eq!(slug("日本語"), "agent");
        assert!(slug(&"x".repeat(100)).len() <= 48);
    }

    #[test]
    fn creates_a_locked_worktree_and_preserves_relative_cwd() {
        let suffix = unique_suffix();
        let root = std::env::temp_dir().join(format!("ptygrid-worktree-test-{suffix}"));
        let repo = root.join("repo");
        let app_data = root.join("app-data");
        std::fs::create_dir_all(repo.join("subdir")).unwrap();
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.name", "ptygrid test"]);
        run(&["config", "user.email", "ptygrid@example.invalid"]);
        std::fs::write(repo.join("subdir/tracked.txt"), "tracked\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "initial"]);

        let cfg = parse_config(
            r#"
agents:
  - name: Codex Review
    cmd: codex
    cwd: subdir
    worktree:
      enabled: true
      base: HEAD
      setup: "printf \"$MARKER\" > setup.marker"
"#,
        )
        .unwrap();
        let prepared = prepare_at(
            &app_data,
            &cfg.agents[0],
            &repo,
            &[("MARKER".to_string(), "ready".to_string())],
        )
        .unwrap()
        .unwrap();
        assert!(prepared.cwd.ends_with("subdir"));
        assert!(prepared.cwd.join("tracked.txt").is_file());
        assert_eq!(
            std::fs::read_to_string(prepared.cwd.join("setup.marker")).unwrap(),
            "ready"
        );
        assert!(prepared.info.branch.starts_with("ptygrid/codex-review/"));
        assert!(Path::new(&prepared.info.path).starts_with(app_data.canonicalize().unwrap()));

        let listed = checked_git(&repo, ["worktree", "list", "--porcelain"]).unwrap();
        let listed = String::from_utf8_lossy(&listed.stdout);
        assert!(listed.contains(&format!("worktree {}", prepared.info.path)));
        assert!(listed.contains("locked ptygrid active session"));

        run(&["worktree", "unlock", &prepared.info.path]);
        run(&["worktree", "remove", "--force", &prepared.info.path]);
        run(&["branch", "-D", &prepared.info.branch]);
        let _ = std::fs::remove_dir_all(root);
    }
}
