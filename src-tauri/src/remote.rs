// Phase 4.4.5: ssh persistence + reconnect.
//
// tmux already solves "keep the remote process alive across a dropped link";
// ptygrid does not re-implement that. What it adds, for a definition whose
// `cmd` is `ssh …` and that carries an `.ssh` block, is:
//
//   1. a command rewrite (`wrap_ssh_command`, pure) that attaches the pane to
//      a named tmux / screen session on the remote host (`new-session -A` /
//      `-D -R`: create on first connect, re-attach afterwards) and adds a
//      ServerAlive probe so a dead link is noticed within seconds instead of
//      the TCP timeout;
//   2. a reconnect policy (`decide_reconnect`, pure) evaluated by the session
//      EOF state machine: ssh's exit status 255 ("connection error") triggers
//      a delayed respawn of the SAME command with exponential backoff, which
//      lands back inside the surviving multiplexer session.
//
// Everything with a side effect stays in session.rs; this module is pure and
// unit-tested on its own.

use std::time::Duration;

use serde::Serialize;

use crate::config::{SshConfig, SshPersist, SSH_KEEPALIVE_COUNT};
use crate::pty;

/// ssh's exit status for "connection error" (also: link dropped, host
/// unreachable, auth failure). Anything else came from the remote command.
pub const SSH_CONNECTION_ERROR: i32 = 255;
/// Reconnect backoff: 1s, 2s, 4s … capped here.
pub const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);
pub const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);

/// Resolved, immutable per-launch remote settings carried on the session slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSpec {
    /// Display destination (`user@host`, alias, or URI authority).
    pub destination: String,
    pub persist: SshPersist,
    /// Multiplexer session name (meaningless for `persist: none`).
    pub session: String,
    pub reconnect: bool,
    /// 0 = unlimited.
    pub max_reconnects: u32,
}

/// Additive wire field on `SessionInfo` (`remote`), so the pane header can
/// show `tmux:<session>` and the reconnect badge knows the destination.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    pub destination: String,
    pub persist: SshPersist,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub reconnect: bool,
}

impl RemoteSpec {
    pub fn info(&self) -> RemoteInfo {
        RemoteInfo {
            destination: self.destination.clone(),
            persist: self.persist,
            session: (self.persist != SshPersist::None).then(|| self.session.clone()),
            reconnect: self.reconnect,
        }
    }
}

/// Result of [`wrap_ssh_command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedCommand {
    /// The rewritten shell command line (still run via `/bin/sh -c`).
    pub command: String,
    pub spec: RemoteSpec,
}

/// Rewrite `cmd` (an `ssh …` command line as written in ptygrid.yml) into the
/// persistent, keepalive-probed form:
///
/// ```text
/// ssh <user opts> -t -o ServerAliveInterval=K -o ServerAliveCountMax=3 -- <dest> \
///     'tmux new-session -A -s <session> [<remote_cmd>]'
/// ```
///
/// Rules:
/// - the user's own options stay in place and come FIRST, so a `-o
///   ServerAliveInterval=…` written by hand wins (ssh keeps the first value);
/// - a remote command already present after the destination in `cmd` is used
///   as the command run inside the multiplexer; combining that with
///   `ssh.remote_cmd` is an error rather than a silent choice;
/// - `persist: none` only adds the keepalive probe (no multiplexer, no `-t`).
pub fn wrap_ssh_command(
    cmd: &str,
    def_name: &str,
    ssh: &SshConfig,
) -> Result<WrappedCommand, String> {
    let words = split_shell_words_spanned(cmd)?;
    let tokens: Vec<String> = words.iter().map(|w| w.value.clone()).collect();
    let first = tokens.first().map(String::as_str).unwrap_or("");
    if first.rsplit('/').next().unwrap_or(first) != "ssh" {
        return Err(format!("ssh: `cmd` must start with ssh (got '{first}')"));
    }
    let dest_idx = pty::ssh_destination_index(&tokens)
        .ok_or_else(|| "ssh: no destination (user@host) found in `cmd`".to_string())?;
    let destination = pty::ssh_destination(&tokens)
        .ok_or_else(|| "ssh: no destination (user@host) found in `cmd`".to_string())?;
    // The user's own text (ssh, its options, the destination) is re-emitted
    // VERBATIM — never re-quoted — so `~`/`$VAR` keep expanding under `sh -c`.
    let head: Vec<String> = words[..dest_idx].iter().map(|w| w.raw.clone()).collect();
    let dest = words[dest_idx].raw.clone();
    let inline_remote: Vec<String> = tokens[dest_idx + 1..].to_vec();

    let remote_cmd: Option<String> = match (ssh.remote_cmd.as_deref(), inline_remote.is_empty()) {
        (Some(_), false) => {
            return Err(
                "ssh: `cmd` already carries a remote command after the destination; \
                 remove it or drop `ssh.remote_cmd` (use one, not both)"
                    .to_string(),
            )
        }
        (Some(rc), true) => Some(rc.trim().to_string()),
        (None, false) => Some(
            inline_remote
                .iter()
                .map(|t| shell_quote(t))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        (None, true) => None,
    };

    let persist = ssh.effective_persist();
    let session = ssh.effective_session(def_name);
    let keepalive = ssh.effective_keepalive();

    let mut out: Vec<String> = head; // ssh + user options, verbatim
    if persist != SshPersist::None {
        out.push("-t".into());
    }
    out.push("-o".into());
    out.push(format!("ServerAliveInterval={keepalive}"));
    out.push("-o".into());
    out.push(format!("ServerAliveCountMax={SSH_KEEPALIVE_COUNT}"));
    out.push("--".into());
    out.push(dest); // verbatim

    let remote = match persist {
        SshPersist::Tmux => {
            let mut r = format!("tmux new-session -A -s {}", shell_quote(&session));
            if let Some(rc) = &remote_cmd {
                r.push(' ');
                r.push_str(&shell_quote(rc));
            }
            Some(r)
        }
        SshPersist::Screen => {
            let mut r = format!("screen -D -R -S {}", shell_quote(&session));
            if let Some(rc) = &remote_cmd {
                r.push_str(" sh -c ");
                r.push_str(&shell_quote(rc));
            }
            Some(r)
        }
        SshPersist::None => None,
    };
    match (remote, persist) {
        // The one token that must reach ssh as a single argv element.
        (Some(r), _) => out.push(shell_quote(&r)),
        // No multiplexer: the user's inline remote command stays verbatim;
        // an `ssh.remote_cmd` string is handed to the remote shell whole.
        (None, SshPersist::None) => {
            if !inline_remote.is_empty() {
                out.extend(words[dest_idx + 1..].iter().map(|w| w.raw.clone()));
            } else if let Some(rc) = &remote_cmd {
                out.push(shell_quote(rc));
            }
        }
        (None, _) => {}
    }

    let command = out.join(" ");
    Ok(WrappedCommand {
        command,
        spec: RemoteSpec {
            destination,
            persist,
            session,
            reconnect: ssh.effective_reconnect(),
            max_reconnects: ssh.effective_max_reconnects(),
        },
    })
}

/// Pure reconnect decision, evaluated by the EOF state machine BEFORE the
/// ordinary autorestart policy. Returns the new consecutive-reconnect count and
/// the delay to wait, or None when this exit is not a reconnect case (then
/// `autorestart` applies as before).
///
/// - only ssh exit 255 qualifies; a clean 0 (operator detached / `exit`) and
///   anything from the remote command (e.g. 127: tmux not installed) do not;
/// - a link that stayed up at least `stable_run` resets the counter, so the
///   cap only ever bites on back-to-back failures (auth errors, host down);
/// - `max_reconnects == 0` means unlimited.
pub fn decide_reconnect(
    spec: Option<&RemoteSpec>,
    manual_kill: bool,
    code: Option<i32>,
    reconnect_count: u32,
    stable_run: bool,
) -> Option<(u32, Duration)> {
    let spec = spec?;
    if manual_kill || !spec.reconnect || code != Some(SSH_CONNECTION_ERROR) {
        return None;
    }
    let count = if stable_run { 0 } else { reconnect_count };
    if spec.max_reconnects != 0 && count >= spec.max_reconnects {
        return None;
    }
    Some((count + 1, backoff_delay(count)))
}

/// 1s · 2^n, capped at [`RECONNECT_MAX_DELAY`].
pub fn backoff_delay(consecutive_failures: u32) -> Duration {
    let factor = 1u64 << consecutive_failures.min(16);
    RECONNECT_BASE_DELAY
        .saturating_mul(factor as u32)
        .min(RECONNECT_MAX_DELAY)
}

/// POSIX-sh quoting: bare when safe, single-quoted otherwise.
pub fn shell_quote(token: &str) -> String {
    if !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c))
    {
        return token.to_string();
    }
    format!("'{}'", token.replace('\'', "'\\''"))
}

/// A parsed word plus the exact source text it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// Unquoted value (what the shell would pass as one argv element).
    pub value: String,
    /// Verbatim source slice, quotes included. Re-emitted as-is for the parts
    /// of the user's command we keep, so `~/.ssh/id` and `$VAR` still expand
    /// exactly as they did before the rewrite.
    pub raw: String,
}

/// Minimal POSIX-ish word splitter: whitespace separation, single quotes
/// (literal), double quotes (backslash escapes `"` `\` `$` `` ` ``), and
/// backslash escapes outside quotes. Enough for an ssh command line; anything
/// exotic (`$(…)`, pipes) is rejected by the caller's ssh-shape checks anyway.
#[cfg(test)]
pub fn split_shell_words(input: &str) -> Result<Vec<String>, String> {
    Ok(split_shell_words_spanned(input)?
        .into_iter()
        .map(|w| w.value)
        .collect())
}

pub fn split_shell_words_spanned(input: &str) -> Result<Vec<Word>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut raw_start: Option<usize> = None;
    let mut chars = input.char_indices().peekable();
    let mut last_end = 0usize;
    let flush = |words: &mut Vec<Word>, cur: &mut String, start: Option<usize>, end: usize| {
        if let Some(start) = start {
            words.push(Word {
                value: std::mem::take(cur),
                raw: input[start..end].to_string(),
            });
        }
    };
    while let Some((i, c)) = chars.next() {
        last_end = i + c.len_utf8();
        match c {
            c if c.is_whitespace() => {
                flush(&mut words, &mut cur, raw_start.take(), i);
            }
            '\'' => {
                raw_start.get_or_insert(i);
                loop {
                    match chars.next() {
                        Some((j, '\'')) => {
                            last_end = j + 1;
                            break;
                        }
                        Some((_, ch)) => cur.push(ch),
                        None => return Err("unterminated single quote in `cmd`".into()),
                    }
                }
            }
            '"' => {
                raw_start.get_or_insert(i);
                loop {
                    match chars.next() {
                        Some((j, '"')) => {
                            last_end = j + 1;
                            break;
                        }
                        Some((_, '\\')) => match chars.next() {
                            Some((_, e @ ('"' | '\\' | '$' | '`'))) => cur.push(e),
                            Some((_, other)) => {
                                cur.push('\\');
                                cur.push(other);
                            }
                            None => return Err("unterminated double quote in `cmd`".into()),
                        },
                        Some((_, ch)) => cur.push(ch),
                        None => return Err("unterminated double quote in `cmd`".into()),
                    }
                }
            }
            '\\' => {
                raw_start.get_or_insert(i);
                match chars.next() {
                    Some((j, ch)) => {
                        last_end = j + ch.len_utf8();
                        cur.push(ch)
                    }
                    None => return Err("trailing backslash in `cmd`".into()),
                }
            }
            other => {
                raw_start.get_or_insert(i);
                cur.push(other);
            }
        }
    }
    flush(&mut words, &mut cur, raw_start.take(), last_end);
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(yaml_fields: &str) -> SshConfig {
        serde_norway::from_str(if yaml_fields.is_empty() { "{}" } else { yaml_fields })
            .unwrap()
    }

    #[test]
    fn wraps_plain_ssh_into_tmux_attach_with_keepalive() {
        let w = wrap_ssh_command("ssh me@gpu", "gpu box", &ssh("")).unwrap();
        assert_eq!(
            w.command,
            "ssh -t -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -- me@gpu \
             'tmux new-session -A -s ptygrid-gpu-box'"
        );
        assert_eq!(w.spec.destination, "me@gpu");
        assert_eq!(w.spec.persist, SshPersist::Tmux);
        assert_eq!(w.spec.session, "ptygrid-gpu-box");
        assert!(w.spec.reconnect);
        assert_eq!(w.spec.max_reconnects, 0);
    }

    #[test]
    fn keeps_user_options_first_and_folds_login() {
        let w = wrap_ssh_command(
            "ssh -p 2222 -i ~/.ssh/id -l root web01",
            "w",
            &ssh("keepalive: 5\nsession: s1"),
        )
        .unwrap();
        assert_eq!(
            w.command,
            "ssh -p 2222 -i ~/.ssh/id -l root -t -o ServerAliveInterval=5 \
             -o ServerAliveCountMax=3 -- web01 'tmux new-session -A -s s1'"
        );
        assert_eq!(w.spec.destination, "root@web01");
    }

    #[test]
    fn remote_cmd_runs_inside_the_multiplexer_quoted() {
        let w = wrap_ssh_command("ssh h", "a", &ssh("remote_cmd: claude --continue")).unwrap();
        assert!(w.command.ends_with(
            "-- h 'tmux new-session -A -s ptygrid-a '\\''claude --continue'\\'''"
        ), "{}", w.command);
        // Inline remote command after the destination is picked up too.
        let w = wrap_ssh_command("ssh h claude --continue", "a", &ssh("")).unwrap();
        assert!(w.command.contains("'claude --continue'"), "{}", w.command);
        // Both at once is an error, not a guess.
        let err = wrap_ssh_command("ssh h claude", "a", &ssh("remote_cmd: codex")).unwrap_err();
        assert!(err.contains("one, not both"), "{err}");
    }

    #[test]
    fn screen_and_none_shapes() {
        let w = wrap_ssh_command("ssh h", "a", &ssh("persist: screen\nremote_cmd: top")).unwrap();
        assert!(w.command.ends_with("-- h 'screen -D -R -S ptygrid-a sh -c top'"), "{}", w.command);
        let w = wrap_ssh_command("ssh h", "a", &ssh("persist: none")).unwrap();
        assert_eq!(
            w.command,
            "ssh -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -- h"
        );
        assert_eq!(w.spec.info().session, None);
        let w = wrap_ssh_command("ssh h uptime", "a", &ssh("persist: none")).unwrap();
        assert!(w.command.ends_with("-- h uptime"));
    }

    #[test]
    fn rejects_non_ssh_and_missing_destination() {
        assert!(wrap_ssh_command("mosh h", "a", &ssh("")).is_err());
        assert!(wrap_ssh_command("ssh -p 22", "a", &ssh("")).is_err());
        assert!(wrap_ssh_command("ssh 'unterminated", "a", &ssh("")).is_err());
    }

    #[test]
    fn user_text_is_kept_verbatim_so_shell_expansion_survives() {
        let w = wrap_ssh_command(
            "ssh -i \"$HOME/.ssh/id\" -o 'ProxyCommand=nc %h %p' $DEST",
            "a",
            &ssh("persist: none"),
        )
        .unwrap();
        assert_eq!(
            w.command,
            "ssh -i \"$HOME/.ssh/id\" -o 'ProxyCommand=nc %h %p' -o ServerAliveInterval=15 \
             -o ServerAliveCountMax=3 -- $DEST"
        );
        assert_eq!(w.spec.destination, "$DEST");
        let w = wrap_ssh_command("ssh h", "a", &ssh("persist: none\nremote_cmd: claude --continue")).unwrap();
        assert!(w.command.ends_with("-- h 'claude --continue'"), "{}", w.command);
    }

    #[test]
    fn split_and_quote_roundtrip() {
        let spanned = split_shell_words_spanned("ssh  'a b'\tc\\ d ").unwrap();
        assert_eq!(
            spanned.iter().map(|w| w.raw.as_str()).collect::<Vec<_>>(),
            vec!["ssh", "'a b'", "c\\ d"]
        );
        assert_eq!(
            spanned.iter().map(|w| w.value.as_str()).collect::<Vec<_>>(),
            vec!["ssh", "a b", "c d"]
        );
        assert_eq!(
            split_shell_words(r#"ssh -o "ProxyCommand=nc %h %p" 'a b' c\ d"#).unwrap(),
            vec!["ssh", "-o", "ProxyCommand=nc %h %p", "a b", "c d"]
        );
        assert_eq!(shell_quote("user@host"), "user@host");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn reconnect_only_on_255_with_backoff_and_cap() {
        let spec = RemoteSpec {
            destination: "h".into(),
            persist: SshPersist::Tmux,
            session: "s".into(),
            reconnect: true,
            max_reconnects: 3,
        };
        let s = Some(&spec);
        assert_eq!(decide_reconnect(s, false, Some(255), 0, false), Some((1, Duration::from_secs(1))));
        assert_eq!(decide_reconnect(s, false, Some(255), 1, false), Some((2, Duration::from_secs(2))));
        assert_eq!(decide_reconnect(s, false, Some(255), 2, false), Some((3, Duration::from_secs(4))));
        assert_eq!(decide_reconnect(s, false, Some(255), 3, false), None, "cap reached");
        // A stable link resets the counter even at the cap.
        assert_eq!(decide_reconnect(s, false, Some(255), 3, true), Some((1, Duration::from_secs(1))));
        // Not reconnect cases.
        assert_eq!(decide_reconnect(s, true, Some(255), 0, false), None, "manual kill");
        assert_eq!(decide_reconnect(s, false, Some(0), 0, false), None, "clean exit");
        assert_eq!(decide_reconnect(s, false, Some(127), 0, false), None, "remote cmd missing");
        assert_eq!(decide_reconnect(s, false, None, 0, false), None);
        assert_eq!(decide_reconnect(None, false, Some(255), 0, false), None, "no ssh block");
        let off = RemoteSpec { reconnect: false, ..spec.clone() };
        assert_eq!(decide_reconnect(Some(&off), false, Some(255), 0, false), None);
        let unlimited = RemoteSpec { max_reconnects: 0, ..spec };
        assert_eq!(decide_reconnect(Some(&unlimited), false, Some(255), 500, false).map(|r| r.0), Some(501));
    }

    #[test]
    fn backoff_caps_at_30s() {
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(4), Duration::from_secs(16));
        assert_eq!(backoff_delay(5), Duration::from_secs(30));
        assert_eq!(backoff_delay(40), Duration::from_secs(30));
    }
}
