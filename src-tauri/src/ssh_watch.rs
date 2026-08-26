// Phase 4.4.5: ad-hoc ssh drop detection for shell panes.
//
// A definition with an `.ssh` block is owned by the reconnect state machine
// in session.rs (the pane's PTY child IS the ssh process, so its exit status
// is known). An `ssh host` typed into a shell pane is different: the shell
// stays alive, the pane never reaches EOF, and the child's exit status is
// invisible from outside the shell. What IS visible — through the same
// per-second foreground sampling that already labels the pane `ssh user@host`
// — is the transition "foreground was ssh → foreground is the shell again".
//
// This module turns that edge into an `ssh-disconnected` event carrying the
// exact command line that was running, so the frontend can offer a one-click
// "reconnect" (re-typing the command into the same pane). It is a proposal,
// never automatic: a deliberate `exit` looks identical from here.
//
// Pure state machine, unit-tested; the sampler feeds it and emits.

use std::collections::HashMap;

use crate::session::SshDisconnectedPayload;

/// One running PTY session as seen by a sampler tick.
#[derive(Debug, Clone)]
pub struct Observation {
    pub id: u32,
    /// Resolved foreground process name (None: lookup failed this tick).
    pub name: Option<String>,
    /// Destination detail when the foreground is a remote-session command.
    pub detail: Option<String>,
    /// Full argv of the foreground process, resolved by the caller ONLY when
    /// `name == "ssh"` (keeps the per-tick cost on non-ssh panes at zero).
    pub argv: Option<Vec<String>>,
    /// Definition-managed ssh (has an `.ssh` block): never watched here.
    pub managed: bool,
}

#[derive(Default)]
pub struct AdhocSshTracker {
    /// Per session id: destination + command line of the ssh last seen in
    /// the foreground.
    last: HashMap<u32, (String, String)>,
}

impl AdhocSshTracker {
    /// Feed one tick; returns the drops detected since the previous tick.
    ///
    /// - foreground `ssh` with a destination → remember (or refresh) it;
    /// - a resolved NON-ssh foreground on a remembered pane → the shell is
    ///   back: emit once and forget;
    /// - an unresolved foreground (`name == None`) is transient (the pgrp
    ///   leader may be mid-exec) → keep waiting;
    /// - a remembered pane that is absent from the tick (exited / not
    ///   running) is forgotten silently — a dead pane has nothing to reconnect.
    pub fn tick(&mut self, observations: &[Observation]) -> Vec<SshDisconnectedPayload> {
        let mut drops = Vec::new();
        let mut seen: Vec<u32> = Vec::with_capacity(observations.len());
        for obs in observations {
            seen.push(obs.id);
            if obs.managed {
                self.last.remove(&obs.id);
                continue;
            }
            match obs.name.as_deref() {
                Some("ssh") => {
                    if let (Some(dest), Some(argv)) = (obs.detail.as_deref(), obs.argv.as_ref()) {
                        if !argv.is_empty() {
                            self.last
                                .insert(obs.id, (dest.to_string(), argv.join(" ")));
                        }
                    }
                }
                Some(_) => {
                    if let Some((destination, command)) = self.last.remove(&obs.id) {
                        drops.push(SshDisconnectedPayload {
                            id: obs.id,
                            destination,
                            command,
                        });
                    }
                }
                None => {}
            }
        }
        self.last.retain(|id, _| seen.contains(id));
        drops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(id: u32, name: Option<&str>, detail: Option<&str>, argv: Option<&[&str]>) -> Observation {
        Observation {
            id,
            name: name.map(str::to_string),
            detail: detail.map(str::to_string),
            argv: argv.map(|a| a.iter().map(|s| s.to_string()).collect()),
            managed: false,
        }
    }

    #[test]
    fn ssh_then_shell_emits_one_proposal_with_the_command() {
        let mut t = AdhocSshTracker::default();
        let argv: &[&str] = &["ssh", "-p", "2222", "me@gpu"];
        assert!(t.tick(&[obs(1, Some("ssh"), Some("me@gpu"), Some(argv))]).is_empty());
        // Still connected: nothing.
        assert!(t.tick(&[obs(1, Some("ssh"), Some("me@gpu"), Some(argv))]).is_empty());
        let drops = t.tick(&[obs(1, Some("zsh"), None, None)]);
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].id, 1);
        assert_eq!(drops[0].destination, "me@gpu");
        assert_eq!(drops[0].command, "ssh -p 2222 me@gpu");
        // Emitted once, not every tick.
        assert!(t.tick(&[obs(1, Some("zsh"), None, None)]).is_empty());
    }

    #[test]
    fn unresolved_tick_is_transient_and_absent_pane_is_forgotten() {
        let mut t = AdhocSshTracker::default();
        let argv: &[&str] = &["ssh", "h"];
        t.tick(&[obs(1, Some("ssh"), Some("h"), Some(argv))]);
        assert!(t.tick(&[obs(1, None, None, None)]).is_empty(), "keep waiting");
        let drops = t.tick(&[obs(1, Some("bash"), None, None)]);
        assert_eq!(drops.len(), 1, "still remembered across the unresolved tick");

        t.tick(&[obs(2, Some("ssh"), Some("h"), Some(argv))]);
        assert!(t.tick(&[]).is_empty(), "pane gone: no proposal");
        assert!(t.tick(&[obs(2, Some("bash"), None, None)]).is_empty(), "and forgotten");
    }

    #[test]
    fn managed_sessions_and_ssh_without_detail_are_ignored() {
        let mut t = AdhocSshTracker::default();
        let argv: &[&str] = &["ssh", "h"];
        let mut managed = obs(1, Some("ssh"), Some("h"), Some(argv));
        managed.managed = true;
        t.tick(&[managed]);
        assert!(t.tick(&[obs(1, Some("bash"), None, None)]).is_empty());

        // `ssh -Q cipher` style: no destination → nothing to reconnect to.
        t.tick(&[obs(3, Some("ssh"), None, Some(argv))]);
        assert!(t.tick(&[obs(3, Some("bash"), None, None)]).is_empty());
        // A new ssh started before the shell was ever sampled in between
        // simply refreshes the remembered command.
        let hop2: &[&str] = &["ssh", "inner"];
        t.tick(&[obs(4, Some("ssh"), Some("outer"), Some(argv))]);
        t.tick(&[obs(4, Some("ssh"), Some("inner"), Some(hop2))]);
        let d = t.tick(&[obs(4, Some("bash"), None, None)]);
        assert_eq!(d[0].destination, "inner");
    }
}
