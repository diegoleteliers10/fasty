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

/// Name of the newest direct child of each shell. The UI reads this on every
/// paint, so a background thread keeps the table warm and the render path only
/// takes the lock for a map lookup.
///
/// A full process scan costs milliseconds, so it runs once a second. A shell
/// that reports a command start or end calls
/// [`request_foreground_child_name_refresh`] to get the new name at once.
pub fn foreground_child_name(shell_pid: u32) -> Option<String> {
    static SNAPSHOT: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<u32, String>>,
    > = std::sync::OnceLock::new();
    static STARTED: std::sync::Once = std::sync::Once::new();
    let snapshot: &'static std::sync::Mutex<std::collections::HashMap<u32, String>> =
        SNAPSHOT.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    STARTED.call_once(|| {
        std::thread::Builder::new()
            .name("fastty-process-names".into())
            .spawn(move || {
                let mut system = System::new();
                let mut last_refresh: Option<std::time::Instant> = None;
                loop {
                    let demanded = REFRESH_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed);
                    let due = last_refresh.is_none_or(|last: std::time::Instant| {
                        last.elapsed() >= std::time::Duration::from_secs(1)
                    });
                    if demanded {
                        // The shell has just started a command: the child may
                        // not exist yet, so read once now and once more after
                        // it has had time to exec.
                        publish_foreground_child_names(&mut system, snapshot);
                        std::thread::sleep(std::time::Duration::from_millis(150));
                        publish_foreground_child_names(&mut system, snapshot);
                        last_refresh = Some(std::time::Instant::now());
                        continue;
                    }
                    if due {
                        publish_foreground_child_names(&mut system, snapshot);
                        last_refresh = Some(std::time::Instant::now());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
            .map(|_| ())
            .unwrap_or_else(|error| {
                eprintln!("Fastty process-name sampler did not start: {error}");
            });
    });
    snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&shell_pid)
        .cloned()
}

/// Ask the sampler to read the child names now instead of at its next pass.
pub fn request_foreground_child_name_refresh() {
    REFRESH_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
}

static REFRESH_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A running process seen as a possible child of another one.
struct ChildProcess {
    parent_pid: u32,
    pid: u32,
    start_time_secs: u64,
    name: String,
}

/// The newest direct child of each parent, by start time and then by pid. A
/// newer pid reads as the more recent process when both start in the same
/// second, which a pipeline often does.
fn newest_child_per_parent(
    children: Vec<ChildProcess>,
) -> std::collections::HashMap<u32, String> {
    let mut newest: std::collections::HashMap<u32, ChildProcess> =
        std::collections::HashMap::new();
    for child in children {
        if child.name.is_empty() {
            continue;
        }
        let replaces = newest.get(&child.parent_pid).is_none_or(|current| {
            (child.start_time_secs, child.pid) > (current.start_time_secs, current.pid)
        });
        if replaces {
            newest.insert(child.parent_pid, child);
        }
    }
    newest
        .into_iter()
        .map(|(parent, child)| (parent, child.name))
        .collect()
}

fn publish_foreground_child_names(
    system: &mut System,
    snapshot: &std::sync::Mutex<std::collections::HashMap<u32, String>>,
) {
    system.refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
    let children: Vec<ChildProcess> = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let parent = process.parent()?;
            Some(ChildProcess {
                parent_pid: parent.as_u32(),
                pid: pid.as_u32(),
                start_time_secs: process.start_time(),
                name: process.name().to_string_lossy().into_owned(),
            })
        })
        .collect();
    let names = newest_child_per_parent(children);
    // Swap the finished map in under the lock, so a paint never waits for a
    // rebuild.
    *snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = names;
}

#[cfg(test)]
mod child_name_tests {
    use super::{newest_child_per_parent, ChildProcess};
    use std::collections::HashMap;

    fn child(parent_pid: u32, pid: u32, start_time_secs: u64, name: &str) -> ChildProcess {
        ChildProcess {
            parent_pid,
            pid,
            start_time_secs,
            name: name.to_string(),
        }
    }

    #[test]
    fn the_newest_child_of_each_parent_wins() {
        let names = newest_child_per_parent(vec![
            child(10, 20, 100, "zsh"),
            child(10, 21, 200, "cargo"),
            child(30, 40, 150, "nvim"),
        ]);
        assert_eq!(names.get(&10).map(String::as_str), Some("cargo"));
        assert_eq!(names.get(&30).map(String::as_str), Some("nvim"));
    }

    #[test]
    fn a_newer_pid_breaks_a_start_time_tie() {
        let names = newest_child_per_parent(vec![
            child(10, 22, 100, "second"),
            child(10, 21, 100, "first"),
        ]);
        assert_eq!(names.get(&10).map(String::as_str), Some("second"));
    }

    #[test]
    fn a_nameless_child_is_skipped() {
        let names = newest_child_per_parent(vec![
            child(10, 20, 100, "zsh"),
            child(10, 21, 200, ""),
            child(30, 40, 100, ""),
        ]);
        assert_eq!(names, HashMap::from([(10u32, "zsh".to_string())]));
    }
}
