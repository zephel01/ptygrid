// Phase 4.1: read-only teammate/subagent transcript tailing.
//
// A `transcript` session (see session.rs `SessionKind::Transcript`) has no
// PTY. Instead, this module tails the subagent's JSONL transcript file, folds
// each appended line into a simple `role: text` view, and streams the delta to
// the frontend via the `transcript-output` event. Everything here is kept off
// the PTY session hot path: session.rs only calls `spawn_tail` and provides a
// generation-guarded append closure.
//
// Security: the caller (teams_hooks) must have validated the path with
// `validate_transcript_path` first — only files under `$HOME/.claude/` are ever
// opened, so a hostile `transcript_path` cannot turn ptygrid into an arbitrary
// file reader.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Runtime};

/// Poll interval used both as the notify fallback and as a liveness re-check
/// tick (spec 6.1.4: 200ms..500ms polling fallback).
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Generation-guarded append + liveness check, provided by session.rs. Returns
/// false once the transcript slot is gone or superseded (stale generation), at
/// which point the tail thread exits. An empty `text` is a pure liveness probe.
pub type AppendFn = Arc<dyn Fn(u64, &str) -> bool + Send + Sync>;

/// Payload for the `transcript-output` event (append-only deltas).
#[derive(Clone, Serialize)]
struct TranscriptOutput {
    id: u32,
    text: String,
}

/// Whether `path` is safe to tail: an absolute path, with no `..` components,
/// under `$HOME/.claude/`. Anything else is rejected so the pane shows status
/// only rather than reading an arbitrary file.
pub fn validate_transcript_path(path: &Path, home: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return false;
    }
    path.starts_with(home.join(".claude"))
}

/// Pull the human-readable text out of a transcript `content` value. Strings
/// are used verbatim; arrays fold text blocks and summarize tool calls to a
/// single line each. Returns None when nothing displayable is present.
fn extract_text(content: &Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    let arr = content.as_array()?;
    let mut parts: Vec<String> = Vec::new();
    for block in arr {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    if !t.is_empty() {
                        parts.push(t.to_string());
                    }
                }
            }
            Some("tool_use") => {
                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                parts.push(format!("[tool_use: {name}]"));
            }
            Some("tool_result") => parts.push("[tool_result]".to_string()),
            _ => {}
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Format a single parsed JSONL object into `role: text`, or None when the
/// line carries no displayable message (system/summary/meta lines, empty
/// content, ...). Handles both the `{message:{role,content}}` envelope and a
/// flat `{role,content}` shape.
fn format_value(v: &Value) -> Option<String> {
    let message = v.get("message");
    let role = message
        .and_then(|m| m.get("role"))
        .or_else(|| v.get("role"))
        .and_then(|r| r.as_str());
    let content = message
        .and_then(|m| m.get("content"))
        .or_else(|| v.get("content"))?;
    let text = extract_text(content)?;
    if text.trim().is_empty() {
        return None;
    }
    match role {
        Some(role) => Some(format!("{role}: {text}")),
        None => Some(text),
    }
}

/// Format a chunk of newly appended JSONL text. Returns the formatted view
/// (each message a `role: text` line) plus the number of lines that could not
/// be parsed or held nothing to show (skipped rather than dumped raw).
pub fn format_new_lines(chunk: &str) -> (String, usize) {
    let mut out = String::new();
    let mut skipped = 0usize;
    for line in chunk.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(v) => match format_value(&v) {
                Some(formatted) => {
                    out.push_str(&formatted);
                    out.push('\n');
                }
                None => skipped += 1,
            },
            Err(_) => skipped += 1,
        }
    }
    (out, skipped)
}

/// Read bytes appended past `offset`, returning only the complete lines (up to
/// the last newline) and advancing `offset` past them. A partial trailing line
/// is left for the next read. A file shorter than `offset` (truncation) resets
/// to the start.
fn read_complete_lines(path: &Path, offset: &mut u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len < *offset {
        *offset = 0; // rotated/truncated
    }
    if len == *offset {
        return Some(String::new());
    }
    file.seek(SeekFrom::Start(*offset)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    match buf.iter().rposition(|&b| b == b'\n') {
        Some(pos) => {
            *offset += (pos + 1) as u64;
            Some(String::from_utf8_lossy(&buf[..=pos]).into_owned())
        }
        None => Some(String::new()), // no complete line yet
    }
}

/// Start the background tail for a transcript session. The thread stops as soon
/// as `append` reports the slot is stale (closed or generation-bumped by
/// SubagentStop), so no explicit shutdown handle is needed.
pub fn spawn_tail<R: Runtime>(
    app: AppHandle<R>,
    id: u32,
    generation: u64,
    path: PathBuf,
    append: AppendFn,
) {
    std::thread::spawn(move || {
        // Bail immediately if the slot was already superseded.
        if !append(generation, "") {
            return;
        }

        // notify watcher on the parent dir; failures degrade to pure polling
        // since `recv_timeout` fires every POLL_INTERVAL regardless. The kept
        // sender guarantees the channel never disconnects (which would busy
        // loop) when the watcher could not be created.
        let (tx, rx) = mpsc::channel::<()>();
        let _keepalive = tx.clone();
        let mut watcher = notify::recommended_watcher(move |_res| {
            let _ = tx.send(());
        })
        .ok();
        if let (Some(w), Some(parent)) = (watcher.as_mut(), path.parent()) {
            let _ = w.watch(parent, RecursiveMode::NonRecursive);
        }

        let mut offset: u64 = 0;
        loop {
            let chunk = read_complete_lines(&path, &mut offset).unwrap_or_default();
            let formatted = if chunk.is_empty() {
                String::new()
            } else {
                format_new_lines(&chunk).0
            };
            if formatted.is_empty() {
                // Liveness probe; stop if the slot is gone.
                if !append(generation, "") {
                    return;
                }
            } else {
                if !append(generation, &formatted) {
                    return;
                }
                let _ = app.emit(
                    "transcript-output",
                    TranscriptOutput {
                        id,
                        text: formatted,
                    },
                );
            }
            // Wait for a change event or the poll tick. Disconnect can't happen
            // (we hold `_keepalive`), so this always resolves to Ok/Timeout.
            match rx.recv_timeout(POLL_INTERVAL) {
                Ok(()) => {
                    // Drain a burst so one save is one scan.
                    while rx.try_recv().is_ok() {}
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_path_only_allows_home_claude() {
        let home = Path::new("/home/alice");
        assert!(validate_transcript_path(
            Path::new("/home/alice/.claude/projects/x/s/subagents/agent-1.jsonl"),
            home
        ));
        // outside ~/.claude
        assert!(!validate_transcript_path(
            Path::new("/home/alice/notes/secret.txt"),
            home
        ));
        assert!(!validate_transcript_path(Path::new("/etc/passwd"), home));
        // relative path
        assert!(!validate_transcript_path(Path::new(".claude/x.jsonl"), home));
        // traversal escape is rejected even though it lexically starts with
        // the allowed prefix.
        assert!(!validate_transcript_path(
            Path::new("/home/alice/.claude/../../etc/passwd"),
            home
        ));
    }

    #[test]
    fn format_flat_and_enveloped_messages() {
        let flat = r#"{"role":"user","content":"hello there"}"#;
        assert_eq!(format_new_lines(flat), ("user: hello there\n".to_string(), 0));

        let enveloped =
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]}}"#;
        assert_eq!(
            format_new_lines(enveloped),
            ("assistant: working on it\n".to_string(), 0)
        );
    }

    #[test]
    fn format_summarizes_tool_calls_to_one_line() {
        let line = r#"{"message":{"role":"assistant","content":[{"type":"text","text":"let me check"},{"type":"tool_use","name":"Bash","input":{"command":"ls -la /very/long/path"}}]}}"#;
        let (out, skipped) = format_new_lines(line);
        assert_eq!(out, "assistant: let me check [tool_use: Bash]\n");
        assert_eq!(skipped, 0);
    }

    #[test]
    fn format_skips_unparseable_and_empty_content() {
        let chunk = concat!(
            "not json at all\n",
            r#"{"type":"summary","summary":"…"}"#,
            "\n",
            r#"{"role":"assistant","content":[]}"#,
            "\n",
            r#"{"role":"user","content":"real"}"#,
            "\n",
            "\n" // trailing blank line ignored (not counted)
        );
        let (out, skipped) = format_new_lines(chunk);
        assert_eq!(out, "user: real\n");
        // unparseable + summary(no content) + empty-content array = 3 skipped
        assert_eq!(skipped, 3);
    }

    #[test]
    fn read_complete_lines_only_returns_whole_lines() {
        let dir = std::env::temp_dir().join(format!(
            "ptygrid-transcript-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\npartial").unwrap();

        let mut offset = 0u64;
        let chunk = read_complete_lines(&path, &mut offset).unwrap();
        assert_eq!(chunk, "{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(offset, 16); // "partial" (7 bytes) left unread

        // No new complete line yet.
        assert_eq!(read_complete_lines(&path, &mut offset).unwrap(), "");
        // Finish the partial line + a new one.
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\npartial-done\n{\"c\":3}\n").unwrap();
        let chunk = read_complete_lines(&path, &mut offset).unwrap();
        assert_eq!(chunk, "partial-done\n{\"c\":3}\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
