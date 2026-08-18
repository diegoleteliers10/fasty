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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub custom_name: Option<String>,
    #[serde(default)]
    pub title_override: Option<String>,
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
