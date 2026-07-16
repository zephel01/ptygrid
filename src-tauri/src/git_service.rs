//! Git integration for Phase 3.
//!
//! The service invokes the user's installed `git` executable with structured
//! arguments and never runs through a shell. Read operations disable optional
//! locks; mutation operations are explicit and limited to index/commit actions.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde::Serialize;

const MAX_STATUS_FILES: usize = 10_000;
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_MUTATION_PATHS: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileStatus {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusInfo {
    pub repo_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub head: String,
    pub files: Vec<GitFileStatus>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffInfo {
    pub repo_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub staged: bool,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitInfo {
    pub repo_root: String,
    pub oid: String,
    pub summary: String,
    pub output: String,
}

fn base_command(dir: &Path, read_only: bool) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .env("GIT_PAGER", "cat")
        // A repository can legally contain names beginning with `:(`.
        // Treat every path supplied by the UI as a literal, never as Git's
        // pathspec magic syntax.
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C");
    if read_only {
        command.env("GIT_OPTIONAL_LOCKS", "0");
    }
    command
}

fn git_output<I, S>(dir: &Path, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    base_command(dir, true)
        .args(args)
        .output()
        .map_err(|e| format!("git executable failed: {e}"))
}

fn mutation_output<I, S>(dir: &Path, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    base_command(dir, false)
        .args(args)
        .output()
        .map_err(|e| format!("git executable failed: {e}"))
}

fn output_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        stdout
    } else {
        format!("git exited with status {}", output.status)
    }
}

fn checked_output<I, S>(dir: &Path, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_output(dir, args)?;
    if output.status.success() {
        return Ok(output);
    }
    Err(output_error(&output))
}

fn checked_mutation<I, S>(dir: &Path, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = mutation_output(dir, args)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(output_error(&output))
    }
}

fn trimmed_stdout(output: Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn discover_root(dir: &Path) -> Result<PathBuf, String> {
    let output = checked_output(dir, ["rev-parse", "--show-toplevel"])
        .map_err(|e| format!("not_a_git_repository: {e}"))?;
    let root = trimmed_stdout(output);
    if root.is_empty() {
        return Err("not_a_git_repository: git returned an empty root".to_string());
    }
    Ok(PathBuf::from(root))
}

pub(crate) fn repository_root(dir: &Path) -> Result<PathBuf, String> {
    discover_root(dir)
}

fn branch_name(root: &Path) -> Result<Option<String>, String> {
    let output = checked_output(root, ["branch", "--show-current"])?;
    let branch = trimmed_stdout(output);
    Ok((!branch.is_empty()).then_some(branch))
}

fn head_name(root: &Path) -> Result<String, String> {
    let output = git_output(root, ["rev-parse", "--verify", "--short=12", "HEAD"])?;
    if output.status.success() {
        Ok(trimmed_stdout(output))
    } else {
        // A repository with no first commit is valid and still has useful
        // untracked/staged status to display.
        Ok("unborn".to_string())
    }
}

/// Parse `git status --porcelain=v1 -z` records. Rename/copy records have a
/// second NUL-delimited path; in `-z` mode Git emits destination then source.
fn parse_porcelain_v1_z(bytes: &[u8]) -> (Vec<GitFileStatus>, bool) {
    let records: Vec<&[u8]> = bytes.split(|b| *b == 0).collect();
    let mut files = Vec::new();
    let mut i = 0;
    let mut truncated = false;

    while i < records.len() {
        let record = records[i];
        i += 1;
        if record.is_empty() || record.len() < 3 || record[2] != b' ' {
            continue;
        }
        if files.len() >= MAX_STATUS_FILES {
            truncated = true;
            break;
        }

        let index = record[0] as char;
        let worktree = record[1] as char;
        let path = String::from_utf8_lossy(&record[3..]).to_string();
        let is_rename_or_copy = matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C');
        let original_path = if is_rename_or_copy && i < records.len() {
            let original = records[i];
            i += 1;
            Some(String::from_utf8_lossy(original).to_string())
        } else {
            None
        };

        files.push(GitFileStatus {
            path,
            original_path,
            index_status: index.to_string(),
            worktree_status: worktree.to_string(),
        });
    }

    (files, truncated)
}

pub fn status(dir: &Path) -> Result<GitStatusInfo, String> {
    let root = discover_root(dir)?;
    let output = checked_output(
        &root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let (files, truncated) = parse_porcelain_v1_z(&output.stdout);

    Ok(GitStatusInfo {
        repo_root: root.display().to_string(),
        branch: branch_name(&root)?,
        head: head_name(&root)?,
        files,
        truncated,
    })
}

fn is_exact_untracked(root: &Path, path: &str) -> Result<bool, String> {
    let output = checked_output(
        root,
        [
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            path,
        ],
    )?;
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .any(|record| record == path.as_bytes()))
}

fn untracked_diff(root: &Path, path: &str) -> Result<Option<Output>, String> {
    if !is_exact_untracked(root, path)? {
        return Ok(None);
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve repository root: {e}"))?;
    let target = root
        .join(path)
        .canonicalize()
        .map_err(|e| format!("cannot resolve untracked file '{path}': {e}"))?;
    if !target.starts_with(&canonical_root) {
        return Err(format!("untracked path escapes repository: {path}"));
    }

    #[cfg(windows)]
    let null_path = "NUL";
    #[cfg(not(windows))]
    let null_path = "/dev/null";

    let output = git_output(
        root,
        [
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=3",
            "--",
            null_path,
            target.to_string_lossy().as_ref(),
        ],
    )?;
    // `git diff --no-index` returns 1 when it successfully found differences.
    if output.status.success() || output.status.code() == Some(1) {
        Ok(Some(output))
    } else {
        Err(output_error(&output))
    }
}

fn bounded_diff_text(output: &Output) -> (String, bool) {
    let truncated = output.stdout.len() > MAX_DIFF_BYTES;
    let visible = &output.stdout[..output.stdout.len().min(MAX_DIFF_BYTES)];
    let mut text = String::from_utf8_lossy(visible).to_string();
    if truncated {
        text.push_str("\n\n[ptygrid: diff truncated at 2 MiB]\n");
    }
    (text, truncated)
}

pub fn diff(dir: &Path, path: Option<String>, staged: bool) -> Result<GitDiffInfo, String> {
    let root = discover_root(dir)?;
    let mut args = vec![
        "diff".to_string(),
        "--no-ext-diff".to_string(),
        "--no-textconv".to_string(),
        "--src-prefix=a/".to_string(),
        "--dst-prefix=b/".to_string(),
        "--unified=3".to_string(),
    ];
    if staged {
        args.push("--cached".to_string());
    }
    if let Some(selected) = path.as_ref() {
        args.push("--".to_string());
        args.push(selected.clone());
    }

    let mut output = checked_output(&root, args)?;
    if !staged && output.stdout.is_empty() {
        if let Some(selected) = path.as_deref() {
            if let Some(untracked) = untracked_diff(&root, selected)? {
                output = untracked;
            }
        }
    }
    let (text, truncated) = bounded_diff_text(&output);

    Ok(GitDiffInfo {
        repo_root: root.display().to_string(),
        path,
        staged,
        text,
        truncated,
    })
}

fn validate_paths(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("at least one path is required".to_string());
    }
    if paths.len() > MAX_MUTATION_PATHS {
        return Err(format!(
            "too many paths: {} (maximum {MAX_MUTATION_PATHS})",
            paths.len()
        ));
    }
    if paths.iter().any(|path| path.is_empty()) {
        return Err("paths must not contain an empty string".to_string());
    }
    Ok(())
}

pub fn stage(dir: &Path, paths: Vec<String>) -> Result<GitStatusInfo, String> {
    validate_paths(&paths)?;
    let root = discover_root(dir)?;
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(paths);
    checked_mutation(&root, args)?;
    status(&root)
}

pub fn unstage(dir: &Path, paths: Vec<String>) -> Result<GitStatusInfo, String> {
    validate_paths(&paths)?;
    let root = discover_root(dir)?;
    let mut args = if head_name(&root)? == "unborn" {
        vec![
            "rm".to_string(),
            "--cached".to_string(),
            "--quiet".to_string(),
            "--ignore-unmatch".to_string(),
            "--".to_string(),
        ]
    } else {
        vec![
            "restore".to_string(),
            "--staged".to_string(),
            "--".to_string(),
        ]
    };
    args.extend(paths);
    checked_mutation(&root, args)?;
    status(&root)
}

pub fn commit(dir: &Path, message: String) -> Result<GitCommitInfo, String> {
    if message.trim().is_empty() {
        return Err("commit message must not be empty".to_string());
    }
    let root = discover_root(dir)?;
    let mut child = base_command(&root, false)
        // `whitespace` (not `strip`) so leading `#` lines like "#42 fix" are
        // preserved instead of being treated as comments and dropped, which
        // desynced the commit body from the reported summary (M2).
        .args(["commit", "--file=-", "--cleanup=whitespace"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git commit failed to start: {e}"))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "git commit stdin is unavailable".to_string())?;
        if let Err(error) = stdin
            .write_all(message.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
        {
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("cannot write commit message: {error}"));
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("git commit wait failed: {e}"))?;
    if !output.status.success() {
        return Err(output_error(&output));
    }

    let oid = trimmed_stdout(checked_output(&root, ["rev-parse", "HEAD"])?);
    let summary = message
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let output_text = [stdout, stderr]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(GitCommitInfo {
        repo_root: root.display().to_string(),
        oid,
        summary,
        output: output_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_modified_untracked_and_rename_records() {
        let input = b" M src/main.rs\0?? new file.txt\0R  new-name.rs\0old-name.rs\0";
        let (files, truncated) = parse_porcelain_v1_z(input);
        assert!(!truncated);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].index_status, " ");
        assert_eq!(files[0].worktree_status, "M");
        assert_eq!(files[1].path, "new file.txt");
        assert_eq!(files[1].index_status, "?");
        assert_eq!(files[2].path, "new-name.rs");
        assert_eq!(files[2].original_path.as_deref(), Some("old-name.rs"));
    }

    #[test]
    fn skips_malformed_records() {
        let (files, truncated) = parse_porcelain_v1_z(b"bad\0\0 M valid\0");
        assert!(!truncated);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "valid");
    }

    #[test]
    fn reads_status_and_diff_from_a_real_repository() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("ptygrid-git-test-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&dir)
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
        std::fs::write(dir.join("tracked.txt"), "before\n").unwrap();
        run(&["add", "tracked.txt"]);
        run(&["commit", "-q", "-m", "initial"]);
        std::fs::write(dir.join("tracked.txt"), "after\n").unwrap();

        let status_info = status(&dir).unwrap();
        assert_eq!(status_info.files.len(), 1);
        assert_eq!(status_info.files[0].path, "tracked.txt");
        assert_eq!(status_info.files[0].worktree_status, "M");

        let diff_info = diff(&dir, Some("tracked.txt".to_string()), false).unwrap();
        assert!(diff_info.text.contains("-before"));
        assert!(diff_info.text.contains("+after"));
        assert!(!diff_info.truncated);

        std::fs::write(dir.join("untracked.txt"), "new content\n").unwrap();
        let untracked = diff(&dir, Some("untracked.txt".to_string()), false).unwrap();
        assert!(untracked.text.contains("+new content"));

        let staged = stage(
            &dir,
            vec!["tracked.txt".to_string(), "untracked.txt".to_string()],
        )
        .unwrap();
        assert_eq!(staged.files.len(), 2);
        assert!(staged.files.iter().all(|file| file.index_status != " "));

        let unstaged = unstage(&dir, vec!["tracked.txt".to_string()]).unwrap();
        let tracked = unstaged
            .files
            .iter()
            .find(|file| file.path == "tracked.txt")
            .unwrap();
        assert_eq!(tracked.index_status, " ");
        assert_eq!(tracked.worktree_status, "M");

        stage(&dir, vec!["tracked.txt".to_string()]).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let hook = dir.join(".git/hooks/pre-commit");
            std::fs::write(&hook, "#!/bin/sh\necho hook-blocked >&2\nexit 1\n").unwrap();
            let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&hook, permissions).unwrap();
            let error = commit(&dir, "must be blocked".to_string()).unwrap_err();
            assert!(error.contains("hook-blocked"));
            std::fs::remove_file(hook).unwrap();
        }

        let committed = commit(&dir, "phase 3.2 integration\n\nbody".to_string()).unwrap();
        assert_eq!(committed.oid.len(), 40);
        assert_eq!(committed.summary, "phase 3.2 integration");
        assert!(status(&dir).unwrap().files.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unstages_a_file_in_an_unborn_repository() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ptygrid-git-unborn-test-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let output = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(output.status.success());
        std::fs::write(dir.join("first.txt"), "first\n").unwrap();

        assert_eq!(
            stage(&dir, vec!["first.txt".to_string()]).unwrap().head,
            "unborn"
        );
        let result = unstage(&dir, vec!["first.txt".to_string()]).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].index_status, "?");
        assert_eq!(result.files[0].worktree_status, "?");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_preserves_leading_hash_lines() {
        // M2: `#42 fix` must survive `--cleanup=whitespace` (with `strip` the
        // whole line would be dropped as a comment, and the summary would
        // disagree with the actual commit body / fail as "empty message").
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join(format!("ptygrid-git-hash-test-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "git {args:?}");
        };
        run(&["init", "-q"]);
        run(&["config", "user.name", "ptygrid test"]);
        run(&["config", "user.email", "ptygrid@example.invalid"]);
        std::fs::write(dir.join("f.txt"), "x\n").unwrap();
        stage(&dir, vec!["f.txt".to_string()]).unwrap();

        let committed = commit(&dir, "#42 fix login".to_string()).unwrap();
        assert_eq!(committed.summary, "#42 fix login");

        // The committed subject must match the summary (line preserved).
        let subject = trimmed_stdout(
            checked_output(&dir, ["log", "-1", "--pretty=%s"]).unwrap(),
        );
        assert_eq!(subject, "#42 fix login");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
