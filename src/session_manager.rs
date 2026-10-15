use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static NEXT_SAVE_REVISION: AtomicU64 = AtomicU64::new(1);
static SAVED_REVISIONS: OnceLock<Mutex<std::collections::HashMap<String, u64>>> = OnceLock::new();

pub fn next_save_revision() -> u64 {
    NEXT_SAVE_REVISION.fetch_add(1, Ordering::Relaxed)
}

#[cfg(windows)]
#[link(name = "Kernel32")]
extern "system" {
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedPane {
    pub id: usize,
    pub title: String,
    pub custom_title: Option<String>,
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_session_id: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_session_identity: Option<crate::daemon::SessionIdentity>,
    /// Compressed terminal snapshot. Restore normal-screen history into a new shell.
    /// Do not replay an old TUI screen as a live process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(default)]
    pub snapshot_activity: SnapshotActivity,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SnapshotActivity {
    #[default]
    Unknown,
    Shell,
    ForegroundProcess,
}

impl PersistedPaneNode {
    fn attach_snapshot(&mut self, pane_id: usize, encoded: &str) -> bool {
        match self {
            Self::Leaf(pane) if pane.id == pane_id => {
                pane.snapshot = Some(encoded.to_string());
                pane.snapshot_activity = pane
                    .host_session_id
                    .and_then(|id| crate::session_host::session_process_status(id).ok())
                    .map(|(running, _)| {
                        if running {
                            SnapshotActivity::ForegroundProcess
                        } else {
                            SnapshotActivity::Shell
                        }
                    })
                    .unwrap_or(SnapshotActivity::Unknown);
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.attach_snapshot(pane_id, encoded) || second.attach_snapshot(pane_id, encoded)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PersistedPaneNode {
    Leaf(PersistedPane),
    Split {
        direction: String, // "Horizontal" or "Vertical"
        ratio: f32,
        first: Box<PersistedPaneNode>,
        second: Box<PersistedPaneNode>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTab {
    pub id: usize,
    #[serde(default)]
    pub ai_tab_key: Option<String>,
    #[serde(default)]
    pub ai_conversation_id: Option<String>,
    pub title: String,
    pub custom_title: Option<String>,
    pub cwd: Option<String>,
    pub layout: Option<PersistedPaneNode>,
    #[serde(default)]
    pub active_pane_id: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub active_tab_idx: usize,
    pub tabs: Vec<PersistedTab>,
}

pub fn sessions_dir() -> PathBuf {
    let dir = crate::paths::get().data_dir.join("sessions");
    let _ = fs::create_dir_all(&dir);

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let legacy = PathBuf::from(home).join(".config/fastty/sessions");
    if legacy.exists() && legacy != dir {
        if let Ok(entries) = fs::read_dir(&legacy) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name() {
                    let dest = dir.join(name);
                    if !dest.exists() {
                        let _ = fs::copy(&p, &dest);
                    }
                }
            }
        }
    }
    dir
}

pub fn list_sessions() -> Vec<SessionData> {
    let dir = sessions_dir();
    let mut sessions = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return sessions,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(session) = serde_json::from_str::<SessionData>(&content) {
                    sessions.push(session);
                }
            }
        }
    }

    sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
    sessions
}

pub fn save_session(session: &SessionData) -> anyhow::Result<()> {
    let dir = sessions_dir();
    let safe_name = session.name.replace(['/', '\\'], "_");
    let path = dir.join(format!("{safe_name}.json"));
    let json = serde_json::to_string_pretty(session)?;
    let tmp = dir.join(format!("{safe_name}.json.tmp"));
    let result = fs::write(&tmp, json).and_then(|_| replace_file(&tmp, &path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}

pub fn save_session_with_snapshots(
    mut session: SessionData,
    snapshots: Vec<(usize, Vec<u8>)>,
    revision: u64,
) -> anyhow::Result<()> {
    use base64::prelude::*;

    let mut saved_revisions = SAVED_REVISIONS
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if saved_revisions
        .get(&session.name)
        .is_some_and(|saved| *saved > revision)
    {
        return Ok(());
    }

    for (pane_id, raw) in snapshots {
        let compressed = crate::server::binary_snapshot::compress_snapshot(&raw)
            .ok_or_else(|| anyhow::anyhow!("invalid snapshot for pane {pane_id}"))?;
        let encoded = BASE64_STANDARD.encode(compressed);
        if !session.tabs.iter_mut().any(|tab| {
            tab.layout
                .as_mut()
                .is_some_and(|layout| layout.attach_snapshot(pane_id, &encoded))
        }) {
            return Err(anyhow::anyhow!("missing pane {pane_id} in workspace"));
        }
    }
    save_session(&session)?;
    saved_revisions.insert(session.name, revision);
    Ok(())
}

pub fn load_session(name: &str) -> Option<SessionData> {
    let dir = sessions_dir();
    let safe_name = name.replace(['/', '\\'], "_");
    let path = dir.join(format!("{safe_name}.json"));
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<SessionData>(&content).ok()
}

pub fn delete_session(name: &str) -> anyhow::Result<()> {
    let dir = sessions_dir();
    let safe_name = name.replace(['/', '\\'], "_");
    let path = dir.join(format!("{safe_name}.json"));
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_session_serialization() {
        let session = SessionData {
            name: "split-workspace".to_string(),
            created_at: 1000,
            updated_at: 2000,
            active_tab_idx: 0,
            tabs: vec![PersistedTab {
                id: 1,
                ai_tab_key: None,
                ai_conversation_id: None,
                title: "Terminal".to_string(),
                custom_title: Some("Dev Split".to_string()),
                cwd: Some("/tmp".to_string()),
                active_pane_id: Some(2),
                layout: Some(PersistedPaneNode::Split {
                    direction: "Horizontal".to_string(),
                    ratio: 0.5,
                    first: Box::new(PersistedPaneNode::Leaf(PersistedPane {
                        id: 1,
                        title: "Left".to_string(),
                        custom_title: None,
                        cwd: Some("/tmp/left".to_string()),
                        host_session_id: None,
                        host_session_identity: None,
                        snapshot: None,
                        snapshot_activity: SnapshotActivity::Unknown,
                    })),
                    second: Box::new(PersistedPaneNode::Leaf(PersistedPane {
                        id: 2,
                        title: "Right".to_string(),
                        custom_title: None,
                        cwd: Some("/tmp/right".to_string()),
                        host_session_id: None,
                        host_session_identity: None,
                        snapshot: None,
                        snapshot_activity: SnapshotActivity::Unknown,
                    })),
                }),
            }],
        };

        let json = serde_json::to_string(&session).unwrap();
        let deserialized: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "split-workspace");
        assert!(matches!(
            deserialized.tabs[0].layout,
            Some(PersistedPaneNode::Split { .. })
        ));
    }
}
