//! One shared process sampler for all PTY sessions (Phase 3.5).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::session::PtyManager;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
struct ProcessSample {
    pid: u32,
    parent: Option<u32>,
    cpu_percent: f32,
    memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionResourceUsage {
    pub id: u32,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: u32,
}

/// Foreground process name of one running PTY session (Phase 4.4.2). Rides on
/// the resource batch so a hand-started CLI in a shell pane gets a live display
/// name / badge without any extra polling.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionForeground {
    id: u32,
    name: String,
    /// Phase 4.4.3: optional display detail (currently the ssh destination,
    /// e.g. `user@host`). Omitted from the wire when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceBatch {
    sampled_at_ms: u64,
    sessions: Vec<SessionResourceUsage>,
    /// Per-session foreground process name (only sessions that resolved one).
    foreground: Vec<SessionForeground>,
}

fn aggregate_process_trees(
    samples: &[ProcessSample],
    roots: &[(u32, u32)],
) -> Vec<SessionResourceUsage> {
    let by_pid: HashMap<u32, &ProcessSample> =
        samples.iter().map(|sample| (sample.pid, sample)).collect();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for sample in samples {
        if let Some(parent) = sample.parent {
            children.entry(parent).or_default().push(sample.pid);
        }
    }

    let mut usages = Vec::with_capacity(roots.len());
    for &(id, root_pid) in roots {
        let mut cpu_percent = 0.0f32;
        let mut memory_bytes = 0u64;
        let mut process_count = 0u32;
        let mut stack = vec![root_pid];
        let mut visited = HashSet::new();
        while let Some(pid) = stack.pop() {
            if !visited.insert(pid) {
                continue;
            }
            let Some(sample) = by_pid.get(&pid) else {
                continue;
            };
            cpu_percent += sample.cpu_percent.max(0.0);
            memory_bytes = memory_bytes.saturating_add(sample.memory_bytes);
            process_count = process_count.saturating_add(1);
            if let Some(descendants) = children.get(&pid) {
                stack.extend(descendants);
            }
        }
        // A root that disappeared between the PTY snapshot and sysinfo refresh
        // is omitted; the next batch will naturally clear it in the frontend.
        if process_count > 0 {
            usages.push(SessionResourceUsage {
                id,
                cpu_percent,
                memory_bytes,
                process_count,
            });
        }
    }
    usages.sort_by_key(|usage| usage.id);
    usages
}

fn process_samples(system: &System) -> Vec<ProcessSample> {
    system
        .processes()
        .iter()
        .map(|(pid, process)| ProcessSample {
            pid: pid.as_u32(),
            parent: process.parent().map(|parent| parent.as_u32()),
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
        })
        .collect()
}

pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    std::thread::spawn(move || {
        let refresh_kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
        let mut system = System::new();

        // CPU usage is a delta and needs two refreshes. Prime the shared
        // System once, then leave a full sample interval before the first emit.
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);
        loop {
            std::thread::sleep(SAMPLE_INTERVAL);
            system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);
            let manager = app.state::<PtyManager>();
            let roots = manager.resource_roots();
            let sessions = aggregate_process_trees(&process_samples(&system), &roots);
            // Resolve foreground names for the same tick so the frontend can
            // label / badge hand-started CLIs live (rides this existing poll).
            let foreground = manager
                .foreground_names()
                .into_iter()
                .filter_map(|(id, name, detail)| {
                    name.map(|name| SessionForeground { id, name, detail })
                })
                .collect();
            let sampled_at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if app
                .emit(
                    "session-resources",
                    ResourceBatch {
                        sampled_at_ms,
                        sessions,
                        foreground,
                    },
                )
                .is_err()
            {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_each_root_with_all_descendants() {
        let samples = vec![
            ProcessSample {
                pid: 10,
                parent: Some(1),
                cpu_percent: 2.5,
                memory_bytes: 100,
            },
            ProcessSample {
                pid: 11,
                parent: Some(10),
                cpu_percent: 3.0,
                memory_bytes: 200,
            },
            ProcessSample {
                pid: 12,
                parent: Some(11),
                cpu_percent: 4.5,
                memory_bytes: 300,
            },
            ProcessSample {
                pid: 20,
                parent: Some(1),
                cpu_percent: 1.0,
                memory_bytes: 50,
            },
        ];
        let usages = aggregate_process_trees(&samples, &[(7, 10), (8, 20)]);
        assert_eq!(
            usages,
            vec![
                SessionResourceUsage {
                    id: 7,
                    cpu_percent: 10.0,
                    memory_bytes: 600,
                    process_count: 3,
                },
                SessionResourceUsage {
                    id: 8,
                    cpu_percent: 1.0,
                    memory_bytes: 50,
                    process_count: 1,
                },
            ]
        );
    }

    #[test]
    fn omits_missing_roots_and_does_not_loop_on_cycles() {
        let samples = vec![
            ProcessSample {
                pid: 30,
                parent: Some(31),
                cpu_percent: 1.0,
                memory_bytes: 10,
            },
            ProcessSample {
                pid: 31,
                parent: Some(30),
                cpu_percent: 2.0,
                memory_bytes: 20,
            },
        ];
        let usages = aggregate_process_trees(&samples, &[(1, 999), (2, 30)]);
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].id, 2);
        assert_eq!(usages[0].process_count, 2);
        assert_eq!(usages[0].memory_bytes, 30);
    }

    #[test]
    fn sysinfo_refresh_includes_the_sampler_process() {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        let current_pid = std::process::id();
        let sample = process_samples(&system)
            .into_iter()
            .find(|sample| sample.pid == current_pid)
            .expect("current process should be visible to sysinfo");
        assert!(sample.memory_bytes > 0);
    }
}
