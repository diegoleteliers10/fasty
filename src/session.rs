//! Persisted tab/window state across restarts.
//!
//! On graceful exit we write `~/.config/fastty/session.json`. On startup, if
//! `config.session_restore` is true and the file parses, we restore each
//! window and its tabs.
//!
//! The schema is multi-window: `Session { windows: Vec<WindowSession> }`. An
//! older `tabs: Vec<TabInfo>` field is still accepted and migrated to one
//! window on load.

use gpui::WindowId;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

#[derive(Default)]
struct WindowSnapshots {
    windows: Vec<(Option<WindowId>, WindowSession)>,
    active_window: usize,
    last_closed: Option<WindowSession>,
    quitting: bool,
}

impl WindowSnapshots {
    fn session(&self) -> Option<Session> {
        let windows: Vec<_> = self
            .windows
            .iter()
            .map(|(_, window)| window.clone())
            .collect();
        let windows = if windows.is_empty() {
            vec![self.last_closed.clone()?]
        } else {
            windows
        };
        Some(Session {
            active_window: self.active_window.min(windows.len() - 1),
            windows,
            legacy_tabs: Vec::new(),
            legacy_active_tab: 0,
        })
    }
}

static WINDOW_SNAPSHOTS: OnceLock<parking_lot::Mutex<WindowSnapshots>> = OnceLock::new();

fn window_snapshots() -> &'static parking_lot::Mutex<WindowSnapshots> {
    WINDOW_SNAPSHOTS.get_or_init(Default::default)
}

/// Keep all saved windows until each restored window has a live WindowId.
pub fn begin_restore(session: &Session) {
    let mut snapshots = window_snapshots().lock();
    snapshots.windows = session
        .windows
        .iter()
        .cloned()
        .map(|window| (None, window))
        .collect();
    snapshots.active_window = session
        .active_window
        .min(snapshots.windows.len().saturating_sub(1));
    snapshots.last_closed = None;
    snapshots.quitting = false;
}

pub fn bind_restored_window(index: usize, window_id: WindowId) {
    if let Some((id, _)) = window_snapshots().lock().windows.get_mut(index) {
        *id = Some(window_id);
    }
}

/// Save a full snapshot under one lock so two windows cannot overwrite each other.
pub fn register_window(window_id: WindowId, mut window: WindowSession) -> anyhow::Result<()> {
    let mut snapshots = window_snapshots().lock();
    if let Some(index) = snapshots
        .windows
        .iter()
        .position(|(id, _)| *id == Some(window_id))
    {
        if window.position.is_none() {
            window.position = snapshots.windows[index].1.position;
        }
        if window.size.is_none() {
            window.size = snapshots.windows[index].1.size;
        }
        snapshots.windows[index].1 = window;
        snapshots.active_window = index;
    } else {
        snapshots.windows.push((Some(window_id), window));
        snapshots.active_window = snapshots.windows.len() - 1;
    }
    snapshots.last_closed = None;
    if let Some(session) = snapshots.session() {
        save(&session)?;
    }
    Ok(())
}

/// Remove a closed window. Keep the final window as the restart target.
pub fn persist_window(window_id: WindowId) -> anyhow::Result<()> {
    let mut snapshots = window_snapshots().lock();
    if snapshots.quitting {
        return Ok(());
    }
    let Some(index) = snapshots
        .windows
        .iter()
        .position(|(id, _)| *id == Some(window_id))
    else {
        return Ok(());
    };
    let (_, closed) = snapshots.windows.remove(index);
    snapshots.active_window = snapshots
        .active_window
        .min(snapshots.windows.len().saturating_sub(1));
    if snapshots.windows.is_empty() {
        snapshots.last_closed = Some(closed);
    }
    if let Some(session) = snapshots.session() {
        save(&session)?;
    }
    Ok(())
}

pub fn persist_before_quit(active_window: Option<WindowId>) -> anyhow::Result<()> {
    let mut snapshots = window_snapshots().lock();
    if let Some(window_id) = active_window {
        if let Some(index) = snapshots
            .windows
            .iter()
            .position(|(id, _)| *id == Some(window_id))
        {
            snapshots.active_window = index;
        }
    }
    snapshots.quitting = true;
    if let Some(session) = snapshots.session() {
        save(&session)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub windows: Vec<WindowSession>,
    /// Active window index. `0` if there is only one window.
    #[serde(default)]
    pub active_window: usize,
    /// Legacy single-window field. Loaded if `windows` is empty.
    #[serde(default, rename = "tabs")]
    pub legacy_tabs: Vec<TabInfo>,
    /// Legacy single-window field. Used when migrating from old session.
    #[serde(default, rename = "active_tab")]
    pub legacy_active_tab: usize,
}

impl Session {
    pub fn migrate(self) -> Session {
        if !self.windows.is_empty() {
            return self;
        }
        if self.legacy_tabs.is_empty() {
            return self;
        }
        Session {
            windows: vec![WindowSession {
                tabs: self.legacy_tabs.clone(),
                active_tab: self.legacy_active_tab,
                position: None,
                size: None,
                ai_sidebar_open: false,
            }],
            active_window: 0,
            legacy_tabs: self.legacy_tabs,
            legacy_active_tab: self.legacy_active_tab,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowSession {
    pub tabs: Vec<TabInfo>,
    #[serde(default)]
    pub active_tab: usize,
    #[serde(default)]
    pub position: Option<(i32, i32)>,
    #[serde(default)]
    pub size: Option<(u32, u32)>,
    #[serde(default)]
    pub ai_sidebar_open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub ai_tab_key: Option<String>,
    #[serde(default)]
    pub ai_conversation_id: Option<String>,
    #[serde(default)]
    pub custom_name: Option<String>,
    #[serde(default)]
    pub title_override: Option<String>,
    #[serde(default)]
    pub pane_tree: Option<PersistedPaneTree>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedPaneTree {
    pub root: PersistedPaneNode,
    pub active_pane_id: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PersistedPaneNode {
    Leaf(PersistedTerminalPane),
    Split {
        direction: PersistedSplitDirection,
        ratio: f32,
        first: Box<PersistedPaneNode>,
        second: Box<PersistedPaneNode>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PersistedSplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTerminalPane {
    pub pane_id: usize,
    /// Absent in old session files and when a pane has no live host session.
    #[serde(default)]
    pub host_session_id: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_session_identity: Option<crate::daemon::SessionIdentity>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    pub title: String,
    #[serde(default)]
    pub custom_title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_tab_json_loads_without_pane_tree() {
        let json = r#"{"cwd":"/tmp","ai_tab_key":"tab","ai_conversation_id":"chat","custom_name":null,"title_override":"Terminal"}"#;

        let tab: TabInfo = serde_json::from_str(json).unwrap();

        assert!(tab.pane_tree.is_none());
        assert_eq!(tab.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(tab.title_override.as_deref(), Some("Terminal"));
    }

    #[test]
    fn legacy_session_migrates_tabs_without_host_session_ids() {
        let json = r#"{"tabs":[{"cwd":"/tmp","ai_tab_key":"tab","ai_conversation_id":"chat","custom_name":null,"title_override":"Terminal"}],"active_tab":0}"#;

        let session: Session = serde_json::from_str(json).unwrap();
        let migrated = session.migrate();

        assert_eq!(migrated.windows.len(), 1);
        assert_eq!(migrated.windows[0].tabs.len(), 1);
        assert!(migrated.windows[0].tabs[0].pane_tree.is_none());
    }

    #[test]
    fn pane_tree_round_trip_keeps_each_host_session_id() {
        let tab = TabInfo {
            cwd: Some(PathBuf::from("/work")),
            ai_tab_key: None,
            ai_conversation_id: None,
            custom_name: None,
            title_override: None,
            pane_tree: Some(PersistedPaneTree {
                root: PersistedPaneNode::Split {
                    direction: PersistedSplitDirection::Horizontal,
                    ratio: 0.4,
                    first: Box::new(PersistedPaneNode::Leaf(PersistedTerminalPane {
                        pane_id: 10,
                        host_session_id: Some(41),
                        host_session_identity: Some(crate::daemon::SessionIdentity {
                            host_instance: crate::daemon::HostInstanceId("original-host".into()),
                            session_id: 41,
                        }),
                        cwd: Some(PathBuf::from("/work/one")),
                        title: "one".into(),
                        custom_title: Some("first".into()),
                    })),
                    second: Box::new(PersistedPaneNode::Leaf(PersistedTerminalPane {
                        pane_id: 11,
                        host_session_id: Some(42),
                        host_session_identity: None,
                        cwd: Some(PathBuf::from("/work/two")),
                        title: "two".into(),
                        custom_title: None,
                    })),
                },
                active_pane_id: 11,
            }),
        };

        let encoded = serde_json::to_string(&tab).unwrap();
        let decoded: TabInfo = serde_json::from_str(&encoded).unwrap();

        let Some(PersistedPaneTree {
            root: PersistedPaneNode::Split { first, second, .. },
            active_pane_id,
        }) = decoded.pane_tree
        else {
            panic!("persisted split must round-trip");
        };
        let PersistedPaneNode::Leaf(first) = *first else {
            panic!("first pane must be a leaf");
        };
        let PersistedPaneNode::Leaf(second) = *second else {
            panic!("second pane must be a leaf");
        };
        assert_eq!(first.host_session_id, Some(41));
        assert_eq!(
            first
                .host_session_identity
                .as_ref()
                .map(|identity| identity.session_id),
            Some(41)
        );
        assert_eq!(
            first
                .host_session_identity
                .as_ref()
                .map(|identity| identity.host_instance.0.as_str()),
            Some("original-host")
        );
        assert_eq!(second.host_session_id, Some(42));
        assert_eq!(active_pane_id, 11);
    }
}

pub fn session_path() -> PathBuf {
    crate::paths::get().state_dir.join("session.json")
}

pub fn load_result() -> anyhow::Result<Option<Session>> {
    let path = session_path();
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let session: Session = serde_json::from_str(&content)?;
    Ok(Some(session.migrate()))
}

pub fn load() -> Option<Session> {
    load_result().ok().flatten()
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

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

pub fn save(session: &Session) -> anyhow::Result<()> {
    let path = session_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp_os);
    let content = serde_json::to_vec_pretty(session)?;
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(&content)?;
        file.sync_all()?;
        drop(file);
        replace_file(&tmp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Ok(())
}
