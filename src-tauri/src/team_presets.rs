// Phase 4.3: Queen team presets — one-shot launch of a config-declared team.
//
// This module is a thin orchestration wrapper over the EXISTING allowlist
// spawn path (`ConfigManager::resolve_def` + `PtyManager::spawn_agent`, i.e.
// exactly what the `spawn_agent` command / Queen tool use) plus durable-inbox
// delivery of role instructions. It introduces no new trust boundary and no
// new protocol. Wire shapes and launch semantics are specified in
// CONTRACT.md "Phase 4.3 追加契約"; design background in
// docs/spec-team-presets.md.
//
// Kept outside lib.rs and outside the session hot path per the release
// discipline (phase3.md).

use serde::Serialize;
use tauri::{AppHandle, Runtime};

use crate::config::ConfigManager;
use crate::queen_store::QueenStore;
use crate::session::{PtyManager, SessionState};

/// Grid-occupancy cap for partial launch. Mirrors the frontend `MAX_PANES`
/// (9-pane grid): a member whose launch would exceed this is reported as
/// `failed` instead of spawning a paneless session. The count approximates
/// pane occupancy with the total session count (any state, any kind), which
/// matches how the grid keeps exited panes visible until closed.
pub const TEAM_SESSION_CAP: usize = 9;

/// Launch outcome of one preset member. Wire values are lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberStatus {
    /// Spawned by this call.
    Started,
    /// A live (starting/running/restarting) session with this definition name
    /// already existed; nothing was spawned (idempotent re-invocation).
    Skipped,
    /// Not spawned: pane cap reached or the spawn itself failed (`error` set).
    Failed,
    /// Declared `standby: true`; never launched by team start.
    Standby,
}

/// One row of the [`TeamStartReport`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberOutcome {
    pub agent: String,
    pub standby: bool,
    pub status: MemberStatus,
    /// `started`: the new session id. `skipped`: the existing live session id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// `failed`: why. Also set when a member's instructions delivery failed
    /// (the launch itself is not rolled back).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Return value of `start_team` (Tauri command and Queen tool alike).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStartReport {
    pub preset: String,
    /// Effective lead (explicit `lead:`, else first non-standby member).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    pub members: Vec<MemberOutcome>,
    /// True only when a kickoff exists, the team actually activated
    /// (>=1 `started`) and the inbox send succeeded.
    pub kickoff_delivered: bool,
}

/// The live session id for a definition name, if one exists. "Live" is any
/// non-exited state — a member spawned milliseconds ago is `starting` and must
/// already count for the idempotent skip.
fn live_session_id(manager: &PtyManager, name: &str) -> Option<u32> {
    manager
        .list_sessions()
        .into_iter()
        .find(|s| s.state != SessionState::Exited && s.name.as_deref() == Some(name))
        .map(|s| s.id)
}

/// Launch a named team preset. See CONTRACT.md Phase 4.3 for the exact
/// semantics implemented here:
///
/// 1. non-standby members launch sequentially in declaration order through
///    the existing allowlist spawn path;
/// 2. members with a live same-name session are skipped (idempotent);
/// 3. members beyond the session cap fail with "pane limit" (partial launch);
/// 4. when at least one member started, instructions (started + standby
///    members) and the kickoff (to the effective lead) are delivered via the
///    durable Queen inbox; an all-skip call delivers nothing.
pub fn start_team<R: Runtime>(
    app: &AppHandle<R>,
    manager: &PtyManager,
    config: &ConfigManager,
    store: &QueenStore,
    preset_name: &str,
    cols: u16,
    rows: u16,
) -> Result<TeamStartReport, String> {
    let (cfg, project_dir) = config
        .current()
        .ok_or_else(|| "no config loaded (call load_config first)".to_string())?;
    let preset = cfg
        .team_presets
        .as_ref()
        .and_then(|presets| presets.get(preset_name))
        .ok_or_else(|| format!("team preset '{preset_name}' not found in config"))?
        .clone();
    let lead = preset.effective_lead().map(str::to_string);

    let mut members: Vec<MemberOutcome> = Vec::with_capacity(preset.members.len());
    let mut started_any = false;

    for member in &preset.members {
        let agent = member.agent.clone();
        if member.effective_standby() {
            members.push(MemberOutcome {
                agent,
                standby: true,
                status: MemberStatus::Standby,
                id: None,
                error: None,
            });
            continue;
        }
        if let Some(existing) = live_session_id(manager, &agent) {
            members.push(MemberOutcome {
                agent,
                standby: false,
                status: MemberStatus::Skipped,
                id: Some(existing),
                error: None,
            });
            continue;
        }
        if manager.list_sessions().len() >= TEAM_SESSION_CAP {
            members.push(MemberOutcome {
                agent,
                standby: false,
                status: MemberStatus::Failed,
                id: None,
                error: format!("pane limit ({TEAM_SESSION_CAP}) reached").into(),
            });
            continue;
        }
        // Same allowlist + spawn path as the `spawn_agent` command/tool.
        let spawn = config
            .resolve_def(&agent)
            .and_then(|(def, dir)| manager.spawn_agent(app.clone(), &def, &dir, cols, rows));
        match spawn {
            Ok(id) => {
                started_any = true;
                members.push(MemberOutcome {
                    agent,
                    standby: false,
                    status: MemberStatus::Started,
                    id: Some(id),
                    error: None,
                });
            }
            Err(error) => members.push(MemberOutcome {
                agent,
                standby: false,
                status: MemberStatus::Failed,
                id: None,
                error: Some(error),
            }),
        }
    }

    // Delivery only when the team actually activated this call. Standby
    // members receive their instructions too — the inbox is durable, so a
    // standby agent spawned later still reads them. An all-skip call is an
    // idempotent no-op and must not re-deliver anything.
    let mut kickoff_delivered = false;
    if started_any {
        let sender = format!("queen:preset/{preset_name}");
        for (member, outcome) in preset.members.iter().zip(members.iter_mut()) {
            let deliver = matches!(
                outcome.status,
                MemberStatus::Started | MemberStatus::Standby
            );
            let Some(text) = member.instructions.as_ref().filter(|_| deliver) else {
                continue;
            };
            if let Err(error) = store.send_inbox(
                &project_dir,
                sender.clone(),
                member.agent.clone(),
                format!("preset:{preset_name} instructions"),
                text.clone(),
            ) {
                // Delivery failure never rolls back the launch; surface it.
                outcome.error = Some(format!("instructions delivery failed: {error}"));
            }
        }
        if let (Some(kickoff), Some(lead_name)) = (preset.kickoff.as_ref(), lead.as_deref()) {
            kickoff_delivered = store
                .send_inbox(
                    &project_dir,
                    sender,
                    lead_name.to_string(),
                    format!("preset:{preset_name} kickoff"),
                    kickoff.clone(),
                )
                .is_ok();
        }
    }

    Ok(TeamStartReport {
        preset: preset_name.to_string(),
        lead,
        members,
        kickoff_delivered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{parse_config, ConfigManager};

    fn mock_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.handle().clone()
    }

    /// Config with three /bin/cat agents (long-lived, quiet) and one preset.
    /// `project_dir` must exist because the Queen store canonicalizes it.
    fn harness(yaml: &str) -> (ConfigManager, QueenStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "ptygrid-team-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = ConfigManager::new();
        config.set_for_test(dir.clone(), parse_config(yaml).unwrap());
        let store = QueenStore::open_in_memory().unwrap();
        (config, store, dir)
    }

    const TEAM_YAML: &str = "agents:\n  - name: local\n    cmd: /bin/cat\n  - name: opus\n    cmd: /bin/cat\n  - name: grok\n    cmd: /bin/cat\nteam_presets:\n  daily:\n    lead: local\n    members:\n      - agent: local\n        instructions: primary worker\n      - agent: opus\n        standby: true\n        instructions: hard problems only\n      - agent: grok\n    kickoff: start the day\n";

    fn inbox_bodies(store: &QueenStore, dir: &std::path::Path, mailbox: &str) -> Vec<String> {
        store
            .list_inbox(dir, mailbox.to_string(), 0, true, 50)
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect()
    }

    #[test]
    fn start_team_launches_non_standby_and_delivers_inbox() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        let (config, store, dir) = harness(TEAM_YAML);

        let report = start_team(&handle, &manager, &config, &store, "daily", 80, 24)
            .expect("start_team should succeed");

        assert_eq!(report.preset, "daily");
        assert_eq!(report.lead.as_deref(), Some("local"));
        assert_eq!(report.members.len(), 3);
        // local: started with an id; opus: standby, never launched; grok: started.
        assert_eq!(report.members[0].status, MemberStatus::Started);
        assert!(report.members[0].id.is_some());
        assert_eq!(report.members[1].status, MemberStatus::Standby);
        assert_eq!(report.members[1].id, None);
        assert_eq!(report.members[2].status, MemberStatus::Started);
        // Exactly the two non-standby members have live sessions.
        let sessions = manager.list_sessions();
        assert_eq!(sessions.len(), 2);
        let names: Vec<_> = sessions.iter().filter_map(|s| s.name.clone()).collect();
        assert!(names.contains(&"local".to_string()));
        assert!(names.contains(&"grok".to_string()));

        // Inbox: started member + standby member instructions, kickoff to lead.
        assert_eq!(inbox_bodies(&store, &dir, "local").len(), 2); // instructions + kickoff
        assert_eq!(
            inbox_bodies(&store, &dir, "opus"),
            vec!["hard problems only".to_string()]
        );
        assert!(inbox_bodies(&store, &dir, "grok").is_empty()); // no instructions declared
        assert!(report.kickoff_delivered);

        for s in manager.list_sessions() {
            let _ = manager.kill_pty(s.id);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_call_is_idempotent_and_delivers_nothing() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        let (config, store, dir) = harness(TEAM_YAML);

        let first = start_team(&handle, &manager, &config, &store, "daily", 80, 24).unwrap();
        let first_ids: Vec<_> = first.members.iter().filter_map(|m| m.id).collect();
        let local_before = inbox_bodies(&store, &dir, "local").len();
        let opus_before = inbox_bodies(&store, &dir, "opus").len();

        let second = start_team(&handle, &manager, &config, &store, "daily", 80, 24).unwrap();
        // Non-standby members are skipped and report their EXISTING ids.
        assert_eq!(second.members[0].status, MemberStatus::Skipped);
        assert_eq!(second.members[2].status, MemberStatus::Skipped);
        let second_ids: Vec<_> = second.members.iter().filter_map(|m| m.id).collect();
        assert_eq!(first_ids, second_ids);
        assert_eq!(manager.list_sessions().len(), 2, "no duplicate sessions");
        // An all-skip call re-delivers nothing — not even the kickoff.
        assert!(!second.kickoff_delivered);
        assert_eq!(inbox_bodies(&store, &dir, "local").len(), local_before);
        assert_eq!(inbox_bodies(&store, &dir, "opus").len(), opus_before);

        for s in manager.list_sessions() {
            let _ = manager.kill_pty(s.id);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pane_cap_yields_partial_launch_with_explicit_failures() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        let (config, store, dir) = harness(TEAM_YAML);

        // Occupy the grid up to one slot below the cap.
        for _ in 0..(TEAM_SESSION_CAP - 1) {
            manager
                .spawn_shell(handle.clone(), 80, 24, Some("/bin/cat".to_string()), None)
                .unwrap();
        }

        let report = start_team(&handle, &manager, &config, &store, "daily", 80, 24).unwrap();
        // local fills the last slot; grok is over the cap -> failed, not spawned.
        assert_eq!(report.members[0].status, MemberStatus::Started);
        assert_eq!(report.members[2].status, MemberStatus::Failed);
        assert!(report.members[2]
            .error
            .as_deref()
            .unwrap()
            .contains("pane limit"));
        assert_eq!(manager.list_sessions().len(), TEAM_SESSION_CAP);
        // The team still activated (local started), so kickoff is delivered.
        assert!(report.kickoff_delivered);

        for s in manager.list_sessions() {
            let _ = manager.kill_pty(s.id);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn member_launch_failure_is_recorded_and_later_members_still_launch() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        // PTY spawn of a missing binary fails ASYNCHRONOUSLY (fork/exec), so a
        // bad cmd is not a deterministic sync failure. Exercise the sync error
        // path instead: simulate config drift where a preset member no longer
        // resolves to a definition (set_for_test bypasses parse validation,
        // like a reload race would).
        let yaml = "agents:\n  - name: local\n    cmd: /bin/cat\nteam_presets:\n  t:\n    members:\n      - agent: local\n";
        let mut cfg = parse_config(yaml).unwrap();
        cfg.team_presets.as_mut().unwrap().get_mut("t").unwrap().members.insert(
            0,
            crate::config::TeamMember {
                agent: "ghost".to_string(),
                standby: None,
                instructions: None,
            },
        );
        let dir = std::env::temp_dir().join(format!(
            "ptygrid-team-test-drift-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = ConfigManager::new();
        config.set_for_test(dir.clone(), cfg);
        let store = QueenStore::open_in_memory().unwrap();

        let report = start_team(&handle, &manager, &config, &store, "t", 80, 24).unwrap();
        assert_eq!(report.members[0].status, MemberStatus::Failed);
        assert!(report.members[0].error.as_deref().unwrap().contains("ghost"));
        assert_eq!(report.members[1].status, MemberStatus::Started);
        assert_eq!(
            report.lead.as_deref(),
            Some("ghost"),
            "effective lead is declaration-order first non-standby regardless of outcome"
        );

        for s in manager.list_sessions() {
            let _ = manager.kill_pty(s.id);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_preset_and_missing_config_error() {
        let handle = mock_handle();
        let manager = PtyManager::new();
        let (config, store, dir) = harness(TEAM_YAML);

        let err = start_team(&handle, &manager, &config, &store, "nope", 80, 24).unwrap_err();
        assert!(err.contains("'nope'"), "error was: {err}");

        let empty = ConfigManager::new();
        let err = start_team(&handle, &manager, &empty, &store, "daily", 80, 24).unwrap_err();
        assert!(err.contains("no config loaded"), "error was: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
