use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::daemon::SessionIdentity;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time_unix_secs: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ForegroundProcesses {
    pub group_id: u32,
    pub members: Vec<ProcessIdentity>,
    pub ancestors: Vec<ProcessIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProcessUnavailableReason {
    PendingObservation,
    CoordinatesUnavailable,
    ProcessTableUnavailable,
    ShellUnavailable,
    ForegroundUnavailable,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProcessObservation {
    Available {
        shell: ProcessIdentity,
        foreground: ForegroundProcesses,
    },
    Unavailable {
        reason: ProcessUnavailableReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionProcessState {
    pub session: SessionIdentity,
    pub generation: u64,
    pub observation: ProcessObservation,
}

pub(crate) struct ProcessTable {
    system: System,
    records: BTreeMap<u32, ProcessRecord>,
}

struct ProcessRecord {
    identity: ProcessIdentity,
    parent: Option<u32>,
    group: Option<u32>,
}

impl ProcessTable {
    pub(crate) fn new() -> Self {
        Self {
            system: System::new(),
            records: BTreeMap::new(),
        }
    }

    pub(crate) fn refresh(&mut self) {
        self.system
            .refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
        self.records = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| {
                let pid = pid.as_u32();
                (
                    pid,
                    ProcessRecord {
                        identity: ProcessIdentity {
                            pid,
                            start_time_unix_secs: process.start_time(),
                            name: process.name().to_string_lossy().into_owned(),
                        },
                        parent: process.parent().map(|pid| pid.as_u32()),
                        group: process_group(pid),
                    },
                )
            })
            .collect();
    }

    pub(crate) fn observe(
        &self,
        shell_pid: Option<u32>,
        foreground_group: Option<u32>,
    ) -> ProcessObservation {
        if !cfg!(unix) {
            return unavailable(ProcessUnavailableReason::UnsupportedPlatform);
        }
        let (Some(shell_pid), Some(group_id)) = (shell_pid, foreground_group) else {
            return unavailable(ProcessUnavailableReason::CoordinatesUnavailable);
        };
        if self.records.is_empty() {
            return unavailable(ProcessUnavailableReason::ProcessTableUnavailable);
        }
        let Some(shell) = self.records.get(&shell_pid) else {
            return unavailable(ProcessUnavailableReason::ShellUnavailable);
        };
        if shell.group.is_none()
            || shell.identity.start_time_unix_secs == 0
            || shell.identity.name.is_empty()
        {
            return unavailable(ProcessUnavailableReason::ShellUnavailable);
        }
        let members: Vec<_> = self
            .records
            .values()
            .filter(|record| record.group == Some(group_id))
            .map(|record| record.identity.clone())
            .collect();
        if members.is_empty()
            || members
                .iter()
                .any(|member| member.start_time_unix_secs == 0 || member.name.is_empty())
        {
            return unavailable(ProcessUnavailableReason::ForegroundUnavailable);
        }
        let member_ids: BTreeSet<_> = members.iter().map(|member| member.pid).collect();
        let mut ancestor_ids = BTreeSet::new();
        for member in &members {
            let mut visited = BTreeSet::new();
            let mut current = member.pid;
            for _ in 0..64 {
                if !visited.insert(current) {
                    break;
                }
                let Some(parent) = self.records.get(&current).and_then(|record| record.parent)
                else {
                    break;
                };
                let Some(record) = self.records.get(&parent) else {
                    break;
                };
                if record.identity.start_time_unix_secs
                    > self.records[&current].identity.start_time_unix_secs
                {
                    return unavailable(ProcessUnavailableReason::ProcessTableUnavailable);
                }
                if record.identity.start_time_unix_secs > 0 && !member_ids.contains(&parent) {
                    ancestor_ids.insert(parent);
                }
                current = parent;
            }
        }
        let ancestors = ancestor_ids
            .into_iter()
            .filter_map(|pid| self.records.get(&pid).map(|record| record.identity.clone()))
            .collect();
        ProcessObservation::Available {
            shell: shell.identity.clone(),
            foreground: ForegroundProcesses {
                group_id,
                members,
                ancestors,
            },
        }
    }
}

fn unavailable(reason: ProcessUnavailableReason) -> ProcessObservation {
    ProcessObservation::Unavailable { reason }
}

#[cfg(unix)]
fn process_group(pid: u32) -> Option<u32> {
    let pid = libc::pid_t::try_from(pid).ok()?;
    let group = unsafe { libc::getpgid(pid) };
    (group > 0).then_some(group as u32)
}

#[cfg(not(unix))]
fn process_group(_pid: u32) -> Option<u32> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn record(pid: u32, start: u64, parent: Option<u32>, group: u32) -> ProcessRecord {
        ProcessRecord {
            identity: ProcessIdentity {
                pid,
                start_time_unix_secs: start,
                name: "worker".into(),
            },
            parent,
            group: Some(group),
        }
    }

    #[test]
    fn native_process_refresh_observes_the_current_process() {
        let pid = std::process::id();
        let group = process_group(pid).expect("The current process has a process group");
        let mut table = ProcessTable::new();
        table.refresh();
        let ProcessObservation::Available { shell, foreground } =
            table.observe(Some(pid), Some(group))
        else {
            panic!("The current process must have an available observation");
        };
        assert_eq!(shell.pid, pid);
        assert!(shell.start_time_unix_secs > 0);
        assert!(!shell.name.is_empty());
        assert_eq!(foreground.group_id, group);
        assert!(foreground.members.iter().any(|member| member == &shell));
    }

    #[test]
    fn group_members_and_ancestors_are_sorted_and_cycles_stop() {
        let mut table = ProcessTable::new();
        table.records.insert(1, record(1, 20, Some(2), 1));
        table.records.insert(2, record(2, 20, Some(1), 2));
        table.records.insert(3, record(3, 30, Some(2), 3));
        table.records.insert(4, record(4, 40, Some(3), 3));
        let ProcessObservation::Available { foreground, .. } = table.observe(Some(2), Some(3))
        else {
            panic!("Expected available processes");
        };
        assert_eq!(
            foreground.members.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert_eq!(
            foreground
                .ancestors
                .iter()
                .map(|p| p.pid)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn missing_foreground_and_coordinates_are_unavailable() {
        let mut table = ProcessTable::new();
        table.records.insert(2, record(2, 20, None, 2));
        assert_eq!(
            table.observe(None, None),
            unavailable(ProcessUnavailableReason::CoordinatesUnavailable)
        );
        assert_eq!(
            table.observe(Some(2), Some(3)),
            unavailable(ProcessUnavailableReason::ForegroundUnavailable)
        );
    }

    #[test]
    fn failed_shell_group_observation_is_unavailable() {
        let mut table = ProcessTable::new();
        let mut shell = record(2, 20, None, 2);
        shell.group = None;
        table.records.insert(2, shell);
        assert_eq!(
            table.observe(Some(2), Some(2)),
            unavailable(ProcessUnavailableReason::ShellUnavailable)
        );
    }

    #[test]
    fn a_reused_parent_pid_is_unavailable() {
        let mut table = ProcessTable::new();
        table.records.insert(2, record(2, 30, None, 2));
        table.records.insert(3, record(3, 20, Some(2), 3));
        assert_eq!(
            table.observe(Some(2), Some(3)),
            unavailable(ProcessUnavailableReason::ProcessTableUnavailable)
        );
    }

    #[test]
    fn process_start_time_detects_pid_reuse_and_wire_has_no_arguments() {
        let mut table = ProcessTable::new();
        table.records.insert(2, record(2, 20, None, 2));
        let first = table.observe(Some(2), Some(2));
        table.records.insert(2, record(2, 21, None, 2));
        let second = table.observe(Some(2), Some(2));
        assert_ne!(first, second);
        let json = serde_json::to_string(&second).unwrap();
        assert!(!json.contains("argv"));
        assert!(!json.contains("command"));
    }
}
