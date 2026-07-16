//! tmux-compatibility shim logic: parse the tmux subcommand surface that
//! teammate-mode clients drive ([観測] split-window / send-keys /
//! capture-pane / select-pane / kill-pane plus presence checks) and map it
//! onto pane-backend JSON-RPC calls.
//!
//! Kept as library code so parsing and execution are unit-testable; the
//! `ptygrid-tmux-shim` binary is a thin wrapper that plugs in a blocking
//! Unix-socket client. Unknown subcommands succeed as no-ops so a missing
//! tmux feature never crashes the driving client; the embedding app treats
//! "hooks saw a teammate but no split-window arrived" as the breakage signal.

use base64::Engine as _;
use serde_json::{json, Value};

/// Minimal RPC surface the shim needs; the binary implements it over a Unix
/// socket, tests implement it over an in-memory fake.
pub trait RpcClient {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
}

/// What the process should do after handling one invocation.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub stdout: String,
    pub exit_code: i32,
}

impl Outcome {
    fn ok() -> Self {
        Self {
            stdout: String::new(),
            exit_code: 0,
        }
    }

    fn ok_with(stdout: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            exit_code: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TmuxCommand {
    SplitWindow {
        print_pane_id: bool,
        cwd: Option<String>,
        command: Vec<String>,
    },
    SendKeys {
        target: String,
        literal: bool,
        keys: Vec<String>,
    },
    CapturePane {
        target: String,
        print: bool,
        lines: Option<u32>,
    },
    SelectPane {
        target: String,
    },
    KillPane {
        target: String,
    },
    ListPanes,
    ListSessions,
    DisplayMessage {
        print: bool,
        format: Option<String>,
    },
    /// Presence checks and option juggling: succeed without RPC.
    NoOp(String),
}

/// Parse a full tmux argv (excluding argv0).
pub fn parse(args: &[String]) -> Result<TmuxCommand, String> {
    let mut it = args.iter().peekable();
    // Skip global tmux flags we can safely ignore (e.g. -u, -S <socket>).
    while let Some(a) = it.peek() {
        match a.as_str() {
            "-S" | "-L" | "-f" => {
                it.next();
                it.next();
            }
            s if s.starts_with('-') && s != "--" => {
                it.next();
            }
            _ => break,
        }
    }
    let Some(sub) = it.next() else {
        return Ok(TmuxCommand::NoOp("(none)".into()));
    };
    let rest: Vec<String> = it.cloned().collect();

    match sub.as_str() {
        "split-window" | "splitw" => parse_split_window(&rest),
        "send-keys" | "send" => parse_send_keys(&rest),
        "capture-pane" | "capturep" => parse_capture_pane(&rest),
        "select-pane" | "selectp" => {
            let target = take_target(&rest)?;
            Ok(TmuxCommand::SelectPane { target })
        }
        "kill-pane" | "killp" => {
            let target = take_target(&rest)?;
            Ok(TmuxCommand::KillPane { target })
        }
        "list-panes" | "lsp" => Ok(TmuxCommand::ListPanes),
        "list-sessions" | "ls" => Ok(TmuxCommand::ListSessions),
        "display-message" | "display" => {
            let print = rest.iter().any(|a| a == "-p");
            let format = rest.iter().rfind(|a| !a.starts_with('-')).cloned();
            Ok(TmuxCommand::DisplayMessage { print, format })
        }
        other => Ok(TmuxCommand::NoOp(other.to_string())),
    }
}

fn parse_split_window(args: &[String]) -> Result<TmuxCommand, String> {
    let mut print_pane_id = false;
    let mut cwd = None;
    let mut command: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-P" => print_pane_id = true,
            "-c" => {
                i += 1;
                cwd = args.get(i).cloned();
            }
            "-F" | "-t" | "-l" | "-p" | "-e" => {
                // Flags with a value we don't need (format, target, size, env).
                i += 1;
            }
            "-d" | "-h" | "-v" | "-b" | "-f" => {}
            "--" => {
                command.extend(args[i + 1..].iter().cloned());
                break;
            }
            _ => {
                command.extend(args[i..].iter().cloned());
                break;
            }
        }
        i += 1;
    }
    if command.is_empty() {
        return Err("split-window: missing command".into());
    }
    // tmux passes a single shell string; #26572 wants argv. Split it unless
    // the caller already provided pre-split argv after `--`.
    let command = if command.len() == 1 {
        split_shell_words(&command[0])?
    } else {
        command
    };
    Ok(TmuxCommand::SplitWindow {
        print_pane_id,
        cwd,
        command,
    })
}

fn parse_send_keys(args: &[String]) -> Result<TmuxCommand, String> {
    let mut target = None;
    let mut literal = false;
    let mut keys = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-t" => {
                i += 1;
                target = args.get(i).cloned();
            }
            "-l" => literal = true,
            "--" => {
                keys.extend(args[i + 1..].iter().cloned());
                break;
            }
            s if s.starts_with('-') && keys.is_empty() => {}
            _ => keys.push(args[i].clone()),
        }
        i += 1;
    }
    Ok(TmuxCommand::SendKeys {
        target: target.ok_or("send-keys: missing -t target")?,
        literal,
        keys,
    })
}

fn parse_capture_pane(args: &[String]) -> Result<TmuxCommand, String> {
    let mut target = None;
    let mut print = false;
    let mut lines = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-t" => {
                i += 1;
                target = args.get(i).cloned();
            }
            "-p" => print = true,
            "-S" => {
                i += 1;
                // `-S -200` means "start 200 lines back"; map to lines=200.
                if let Some(v) = args.get(i) {
                    lines = v.trim_start_matches('-').parse::<u32>().ok();
                }
            }
            _ => {}
        }
        i += 1;
    }
    Ok(TmuxCommand::CapturePane {
        target: target.ok_or("capture-pane: missing -t target")?,
        print,
        lines,
    })
}

fn take_target(args: &[String]) -> Result<String, String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-t" {
            return args.get(i + 1).cloned().ok_or("missing -t value".into());
        }
        i += 1;
    }
    Err("missing -t target".into())
}

/// Minimal shell-word splitting (spaces, single/double quotes, backslash in
/// unquoted/double-quoted context). No expansion; keeps argv0 intact so the
/// host-side teammate binary allowlist can check it.
pub fn split_shell_words(input: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => current.push(ch),
                        None => return Err("unterminated single quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(esc @ ('"' | '\\' | '$' | '`')) => current.push(esc),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err("unterminated escape".into()),
                        },
                        Some(ch) => current.push(ch),
                        None => return Err("unterminated double quote".into()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(ch) => current.push(ch),
                    None => return Err("trailing backslash".into()),
                }
            }
            _ => {
                in_word = true;
                current.push(c);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    if words.is_empty() {
        return Err("empty command".into());
    }
    Ok(words)
}

/// tmux key-name translation for non-literal send-keys tokens.
fn translate_key(token: &str) -> String {
    match token {
        "Enter" | "KPEnter" => "\r".into(),
        "Tab" => "\t".into(),
        "Escape" => "\x1b".into(),
        "Space" => " ".into(),
        "BSpace" => "\x7f".into(),
        "Up" => "\x1b[A".into(),
        "Down" => "\x1b[B".into(),
        "Right" => "\x1b[C".into(),
        "Left" => "\x1b[D".into(),
        t if t.len() == 3 && t.starts_with("C-") => {
            let c = t.as_bytes()[2].to_ascii_uppercase();
            if c.is_ascii_uppercase() {
                ((c - b'A' + 1) as char).to_string()
            } else {
                t.into()
            }
        }
        other => other.into(),
    }
}

/// Execute a parsed command against the backend. `self_id_env` carries
/// `TMUX_PANE` when present so `display-message -p '#{pane_id}'` can answer
/// without an RPC round-trip.
pub fn execute(
    cmd: TmuxCommand,
    client: &mut dyn RpcClient,
    self_id_env: Option<&str>,
) -> Result<Outcome, String> {
    match cmd {
        TmuxCommand::NoOp(_) => Ok(Outcome::ok()),
        TmuxCommand::SplitWindow {
            print_pane_id,
            cwd,
            command,
        } => {
            let mut params = json!({ "command": command });
            if let Some(cwd) = cwd {
                params["cwd"] = json!(cwd);
            }
            let result = client.call("spawn_agent", params)?;
            let id = result["context_id"].as_str().unwrap_or_default().to_string();
            if print_pane_id {
                Ok(Outcome::ok_with(format!("{id}\n")))
            } else {
                Ok(Outcome::ok())
            }
        }
        TmuxCommand::SendKeys {
            target,
            literal,
            keys,
        } => {
            let data: String = if literal {
                keys.concat()
            } else {
                keys.iter().map(|k| translate_key(k)).collect()
            };
            let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
            client.call("write", json!({ "context_id": target, "data": encoded }))?;
            Ok(Outcome::ok())
        }
        TmuxCommand::CapturePane {
            target,
            print,
            lines,
        } => {
            let mut params = json!({ "context_id": target });
            if let Some(lines) = lines {
                params["lines"] = json!(lines);
            }
            let result = client.call("capture", params)?;
            let text = result["text"].as_str().unwrap_or_default();
            if print {
                let mut out = text.to_string();
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                Ok(Outcome::ok_with(out))
            } else {
                Ok(Outcome::ok())
            }
        }
        TmuxCommand::SelectPane { target } => {
            client.call("ptygrid/focus", json!({ "context_id": target }))?;
            Ok(Outcome::ok())
        }
        TmuxCommand::KillPane { target } => {
            client.call("kill", json!({ "context_id": target }))?;
            Ok(Outcome::ok())
        }
        TmuxCommand::ListPanes => {
            let result = client.call("list", json!({}))?;
            let mut out = String::new();
            if let Some(items) = result["contexts"].as_array() {
                for item in items {
                    if let Some(id) = item.as_str() {
                        out.push_str(id);
                        out.push('\n');
                    }
                }
            }
            Ok(Outcome::ok_with(out))
        }
        TmuxCommand::ListSessions => {
            // One logical session; presence is what callers check.
            Ok(Outcome::ok_with("ptygrid: 1 windows\n"))
        }
        TmuxCommand::DisplayMessage { print, format } => {
            if !print {
                return Ok(Outcome::ok());
            }
            let wants_pane_id = format.as_deref().is_some_and(|f| f.contains("#{pane_id}"));
            if !wants_pane_id {
                return Ok(Outcome::ok_with("\n"));
            }
            if let Some(id) = self_id_env {
                return Ok(Outcome::ok_with(format!("{id}\n")));
            }
            let result = client.call("get_self_id", json!({}))?;
            let id = result["context_id"].as_str().unwrap_or_default();
            Ok(Outcome::ok_with(format!("{id}\n")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClient {
        pub calls: Vec<(String, Value)>,
        pub responses: Vec<Value>,
    }

    impl FakeClient {
        fn new(responses: Vec<Value>) -> Self {
            Self {
                calls: Vec::new(),
                responses,
            }
        }
    }

    impl RpcClient for FakeClient {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.calls.push((method.to_string(), params));
            if self.responses.is_empty() {
                Ok(json!({}))
            } else {
                Ok(self.responses.remove(0))
            }
        }
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_split_window_with_shell_string() {
        let cmd = parse(&argv(&[
            "split-window",
            "-d",
            "-h",
            "-P",
            "-F",
            "#{pane_id}",
            "claude --agent-id 'researcher@team one'",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            TmuxCommand::SplitWindow {
                print_pane_id: true,
                cwd: None,
                command: vec![
                    "claude".into(),
                    "--agent-id".into(),
                    "researcher@team one".into()
                ],
            }
        );
    }

    #[test]
    fn parses_split_window_with_explicit_argv() {
        let cmd = parse(&argv(&[
            "split-window", "-P", "-c", "/project", "--", "claude", "--foo",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            TmuxCommand::SplitWindow {
                print_pane_id: true,
                cwd: Some("/project".into()),
                command: vec!["claude".into(), "--foo".into()],
            }
        );
    }

    #[test]
    fn split_window_without_command_errors() {
        assert!(parse(&argv(&["split-window", "-d"])).is_err());
    }

    #[test]
    fn shell_word_splitting_handles_quotes() {
        assert_eq!(
            split_shell_words(r#"claude --arg "a b" 'c d' e\ f"#).unwrap(),
            vec!["claude", "--arg", "a b", "c d", "e f"]
        );
        assert!(split_shell_words("claude 'unterminated").is_err());
    }

    #[test]
    fn send_keys_translates_key_names() {
        let cmd = parse(&argv(&["send-keys", "-t", "%1", "hello", "Enter"])).unwrap();
        let mut client = FakeClient::new(vec![]);
        execute(cmd, &mut client, None).unwrap();
        let (method, params) = &client.calls[0];
        assert_eq!(method, "write");
        assert_eq!(params["context_id"], "%1");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(params["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, b"hello\r");
    }

    #[test]
    fn send_keys_literal_skips_translation() {
        let cmd = parse(&argv(&["send-keys", "-l", "-t", "%1", "Enter"])).unwrap();
        let mut client = FakeClient::new(vec![]);
        execute(cmd, &mut client, None).unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(client.calls[0].1["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, b"Enter");
    }

    #[test]
    fn ctrl_key_translation() {
        assert_eq!(translate_key("C-c"), "\x03");
        assert_eq!(translate_key("C-z"), "\x1a");
    }

    #[test]
    fn capture_pane_prints_text_with_lines() {
        let cmd = parse(&argv(&["capture-pane", "-p", "-t", "%1", "-S", "-200"])).unwrap();
        let mut client = FakeClient::new(vec![json!({"text": "line1\nline2"})]);
        let out = execute(cmd, &mut client, None).unwrap();
        assert_eq!(out.stdout, "line1\nline2\n");
        assert_eq!(client.calls[0].1["lines"], 200);
    }

    #[test]
    fn select_pane_maps_to_focus_extension() {
        let cmd = parse(&argv(&["select-pane", "-t", "%2"])).unwrap();
        let mut client = FakeClient::new(vec![]);
        execute(cmd, &mut client, None).unwrap();
        assert_eq!(client.calls[0].0, "ptygrid/focus");
    }

    #[test]
    fn display_message_prefers_env_pane_id() {
        let cmd = parse(&argv(&["display-message", "-p", "#{pane_id}"])).unwrap();
        let mut client = FakeClient::new(vec![]);
        let out = execute(cmd, &mut client, Some("%0")).unwrap();
        assert_eq!(out.stdout, "%0\n");
        assert!(client.calls.is_empty(), "no RPC when TMUX_PANE is set");
    }

    #[test]
    fn unknown_subcommand_is_silent_noop() {
        let cmd = parse(&argv(&["set-option", "-g", "status", "off"])).unwrap();
        assert_eq!(cmd, TmuxCommand::NoOp("set-option".into()));
        let mut client = FakeClient::new(vec![]);
        let out = execute(cmd, &mut client, None).unwrap();
        assert_eq!(out, Outcome::ok());
        assert!(client.calls.is_empty());
    }

    #[test]
    fn split_window_prints_context_id_with_p() {
        let cmd = parse(&argv(&["split-window", "-P", "claude"])).unwrap();
        let mut client = FakeClient::new(vec![json!({"context_id": "%3"})]);
        let out = execute(cmd, &mut client, None).unwrap();
        assert_eq!(out.stdout, "%3\n");
    }
}
