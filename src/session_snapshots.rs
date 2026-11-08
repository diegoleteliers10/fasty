//! Scrollback snapshots that outlive the host that owned them.
//!
//! The window session file stores structure and host session ids. When the
//! host restarts, those ids refer to a dead host: the pane cannot attach and
//! the previous output is gone. This store keeps the last terminal snapshot
//! per host session so the next launch can rebuild the pane with its scrollback
//! instead of opening a blank shell.
//!
//! Records are keyed by `(host instance, session id)`. A session id is only
//! meaningful inside the host instance that issued it, so a snapshot taken
//! from another instance must never be replayed into a live one.

use base64::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Upper bound on stored snapshots.
const MAX_ENTRIES: usize = 48;
/// Largest snapshot this store accepts. The snapshot generator itself allows a
/// much larger payload, so the cap has to be enforced here: one oversized pane
/// would otherwise write tens of MiB and slow every later write.
const MAX_RECORD_BYTES: usize = 1_048_576;
/// Snapshots older than this are dropped on the next write.
const MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;
const FILE_NAME: &str = "session-snapshots.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub session_id: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_instance: Option<String>,
    /// A foreground program owned the pane when the snapshot was taken. Replaying
    /// that screen into a new shell would show a dead TUI, so restore skips it.
    #[serde(default)]
    pub foreground_running: bool,
    pub updated_at: u64,
    pub encoded: String,
}

impl SnapshotRecord {
    pub fn bytes(&self) -> Option<Vec<u8>> {
        BASE64_STANDARD.decode(self.encoded.as_bytes()).ok()
    }
}

/// Tests must not read or write the real user state file.
static DISK_DISABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Reported once per process so a runaway pane does not flood stderr.
static OVERSIZE_REPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn store() -> &'static parking_lot::Mutex<HashMap<String, SnapshotRecord>> {
    static STORE: OnceLock<parking_lot::Mutex<HashMap<String, SnapshotRecord>>> = OnceLock::new();
    STORE.get_or_init(|| parking_lot::Mutex::new(load()))
}

fn key(host_instance: Option<&str>, session_id: usize) -> String {
    format!("{}:{session_id}", host_instance.unwrap_or("-"))
}

pub fn path() -> Option<PathBuf> {
    if DISK_DISABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    crate::paths::get_opt().map(|dirs| dirs.state_dir.join(FILE_NAME))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load() -> HashMap<String, SnapshotRecord> {
    let Some(path) = path() else {
        return HashMap::new();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

fn prune(map: &mut HashMap<String, SnapshotRecord>) {
    let now = now_secs();
    map.retain(|_, record| now.saturating_sub(record.updated_at) <= MAX_AGE_SECS);
    if map.len() <= MAX_ENTRIES {
        return;
    }
    let mut by_age: Vec<(String, u64)> = map
        .iter()
        .map(|(key, record)| (key.clone(), record.updated_at))
        .collect();
    by_age.sort_by_key(|(_, updated_at)| *updated_at);
    for (key, _) in by_age.into_iter().take(map.len() - MAX_ENTRIES) {
        map.remove(&key);
    }
}

/// Store the snapshot for one host session. `bytes` come from
/// `TerminalState::snapshot_state_for_save`, so they may be FST1 or the legacy
/// binary format; the restore path decodes both.
pub fn record(
    session_id: usize,
    host_instance: Option<&str>,
    foreground_running: bool,
    bytes: Vec<u8>,
) -> Option<()> {
    record_many(&[(session_id, host_instance, foreground_running, bytes)]).then_some(())
}

/// Store several snapshots with one pass over the map and one file write.
///
/// The flush loop covers every live pane, so writing the whole JSON once per
/// pane made a cycle cost N serializations of N records. Entries above
/// [`MAX_RECORD_BYTES`] are dropped: a snapshot that large is not worth the
/// disk and IO, and the pane keeps working without it.
pub fn record_many(
    entries: &[(usize, Option<&str>, bool, Vec<u8>)],
) -> bool {
    let now = now_secs();
    let mut stored_any = false;
    let mut map = store().lock();
    for (session_id, host_instance, foreground_running, bytes) in entries {
        if bytes.is_empty() {
            continue;
        }
        if bytes.len() > MAX_RECORD_BYTES {
            if !OVERSIZE_REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!(
                    "fastty: session snapshot for {session_id} is {} bytes, above the {MAX_RECORD_BYTES} byte cap; keeping the previous record",
                    bytes.len()
                );
            }
            continue;
        }
        map.insert(
            key(*host_instance, *session_id),
            SnapshotRecord {
                session_id: *session_id,
                host_instance: host_instance.map(str::to_string),
                foreground_running: *foreground_running,
                updated_at: now,
                encoded: BASE64_STANDARD.encode(bytes),
            },
        );
        stored_any = true;
    }
    if !stored_any {
        return false;
    }
    prune(&mut map);
    write(&map);
    true
}

/// Snapshot for a session that belonged to `host_instance`. When the recorded
/// instance differs, the id is foreign and must not be replayed.
pub fn lookup(session_id: usize, host_instance: Option<&str>) -> Option<SnapshotRecord> {
    let map = store().lock();
    if let Some(record) = map.get(&key(host_instance, session_id)) {
        return Some(record.clone());
    }
    // Legacy session files carry no identity. Only then is a bare session id
    // allowed to match, and only when exactly one instance recorded it.
    if host_instance.is_some() {
        return None;
    }
    let mut matches = map
        .values()
        .filter(|record| record.session_id == session_id);
    let first = matches.next()?.clone();
    matches.next().is_none().then_some(first)
}

/// Drop the record for one session once its pane attached to a live host.
pub fn forget(session_id: usize, host_instance: Option<&str>) {
    let mut map = store().lock();
    if map.remove(&key(host_instance, session_id)).is_some() {
        write(&map);
    }
}

fn write(map: &HashMap<String, SnapshotRecord>) {
    let Some(path) = path() else {
        return;
    };
    let Ok(json) = serde_json::to_string_pretty(map) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, json).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

/// Test-only switch: keep every test off the real state file, even when another
/// test already initialized the app paths.
#[cfg(test)]
pub(crate) fn disable_disk_for_tests() {
    DISK_DISABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Test-only reset so cases do not see each other's records.
#[cfg(test)]
pub(crate) fn clear_for_tests() {
    let mut map = store().lock();
    map.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the cases and keeps every one off the real state file: the
    /// store is process-wide, so parallel cases would otherwise interleave and
    /// a test that runs after `paths::init` would write the user's directory.
    fn isolate() -> parking_lot::MutexGuard<'static, ()> {
        static LOCK: OnceLock<parking_lot::Mutex<()>> = OnceLock::new();
        let guard = LOCK.get_or_init(|| parking_lot::Mutex::new(())).lock();
        disable_disk_for_tests();
        clear_for_tests();
        guard
    }

    #[test]
    fn record_and_lookup_round_trip() {
        let _guard = isolate();
        let bytes = b"scrollback-bytes".to_vec();
        record(7, Some("host-a"), false, bytes.clone()).expect("record");
        let found = lookup(7, Some("host-a")).expect("lookup same host");
        assert_eq!(found.bytes().as_deref(), Some(bytes.as_slice()));
        assert!(!found.foreground_running);
    }

    #[test]
    fn a_foreign_host_instance_never_matches() {
        let _guard = isolate();
        record(9, Some("host-a"), false, b"old".to_vec()).expect("record");
        assert!(
            lookup(9, Some("host-b")).is_none(),
            "a session id from another host instance must not replay"
        );
    }

    #[test]
    fn legacy_lookup_without_identity_is_unambiguous_only() {
        let _guard = isolate();
        record(11, Some("host-a"), false, b"a".to_vec()).expect("record");
        assert!(lookup(11, None).is_some(), "single instance resolves");
        record(11, Some("host-b"), false, b"b".to_vec()).expect("record");
        assert!(
            lookup(11, None).is_none(),
            "two instances made the bare id ambiguous"
        );
    }

    #[test]
    fn foreground_state_is_preserved_for_restore_decisions() {
        let _guard = isolate();
        record(13, Some("host-a"), true, b"tui".to_vec()).expect("record");
        assert!(lookup(13, Some("host-a")).expect("lookup").foreground_running);
    }

    #[test]
    fn empty_snapshot_is_not_recorded() {
        let _guard = isolate();
        assert!(record(15, Some("host-a"), false, Vec::new()).is_none());
        assert!(lookup(15, Some("host-a")).is_none());
    }

    #[test]
    fn forget_removes_the_record() {
        let _guard = isolate();
        record(17, Some("host-a"), false, b"x".to_vec()).expect("record");
        forget(17, Some("host-a"));
        assert!(lookup(17, Some("host-a")).is_none());
    }

    #[test]
    fn a_snapshot_above_the_cap_keeps_the_previous_record() {
        let _guard = isolate();
        record(19, Some("host-a"), false, b"small".to_vec()).expect("record");
        let oversized = vec![0u8; MAX_RECORD_BYTES + 1];
        assert!(
            record(19, Some("host-a"), false, oversized).is_none(),
            "an oversized snapshot is not stored"
        );
        let kept = lookup(19, Some("host-a")).expect("previous record survives");
        assert_eq!(kept.bytes().as_deref(), Some(b"small".as_slice()));
    }

    #[test]
    fn record_many_stores_every_entry_in_one_pass() {
        let _guard = isolate();
        let entries = vec![
            (21usize, Some("host-a"), false, b"one".to_vec()),
            (22usize, Some("host-a"), false, b"two".to_vec()),
            (23usize, Some("host-a"), false, Vec::new()),
        ];
        assert!(record_many(&entries));
        assert!(lookup(21, Some("host-a")).is_some());
        assert!(lookup(22, Some("host-a")).is_some());
        assert!(lookup(23, Some("host-a")).is_none());
    }

    #[test]
    fn record_many_without_storable_entries_reports_nothing_stored() {
        let _guard = isolate();
        let entries = vec![(25usize, Some("host-a"), false, Vec::new())];
        assert!(!record_many(&entries));
        assert!(lookup(25, Some("host-a")).is_none());
    }
}
