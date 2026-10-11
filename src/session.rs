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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Latest session snapshot per open window. `on_window_closed` reads the
/// snapshot of the window that was just closed and writes it to disk, so the
/// path of the last window closed survives a restart even when several windows
/// are open at once.
static WINDOW_SNAPSHOTS: OnceLock<parking_lot::Mutex<HashMap<WindowId, Session>>> = OnceLock::new();

fn window_snapshots() -> &'static parking_lot::Mutex<HashMap<WindowId, Session>> {
    WINDOW_SNAPSHOTS.get_or_init(Default::default)
}

/// Remember the latest state of `window_id` so it can be persisted when the
/// window is closed and the `RootView` is no longer accessible.
pub fn register_window(window_id: WindowId, session: Session) {
    window_snapshots().lock().insert(window_id, session);
}

/// Persist the state of the window that was just closed.
pub fn persist_window(window_id: WindowId) {
    let snapshot = window_snapshots().lock().get(&window_id).cloned();
    if let Some(session) = snapshot {
        let _ = save(&session);
    }
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
                        cwd: Some(PathBuf::from("/work/one")),
                        title: "one".into(),
                        custom_title: Some("first".into()),
                    })),
                    second: Box::new(PersistedPaneNode::Leaf(PersistedTerminalPane {
                        pane_id: 11,
                        host_session_id: Some(42),
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

        let Some(PersistedPaneTree { root: PersistedPaneNode::Split { first, second, .. }, active_pane_id }) = decoded.pane_tree else {
            panic!("persisted split must round-trip");
        };
        let PersistedPaneNode::Leaf(first) = *first else { panic!("first pane must be a leaf"); };
        let PersistedPaneNode::Leaf(second) = *second else { panic!("second pane must be a leaf"); };
        assert_eq!(first.host_session_id, Some(41));
        assert_eq!(second.host_session_id, Some(42));
        assert_eq!(active_pane_id, 11);
    }
}

pub fn session_path() -> PathBuf {
    crate::paths::get().state_dir.join("session.json")
}

pub fn load() -> Option<Session> {
    let path = session_path();
    let content = std::fs::read_to_string(&path).ok()?;
    let s: Session = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(_) => {
            return None;
        }
    };
    Some(s.migrate())
}

pub fn save(session: &Session) -> anyhow::Result<()> {
    let path = session_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    let content = serde_json::to_string_pretty(session)?;
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
