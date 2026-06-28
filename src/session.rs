//! Persisted tab/window state across restarts.
//!
//! On graceful exit we write `~/.config/fasty/session.json`. On startup, if
//! `config.session_restore` is true and the file parses, we restore each
//! window and its tabs.
//!
//! The schema is multi-window: `Session { windows: Vec<WindowSession> }`. An
//! older `tabs: Vec<TabInfo>` field is still accepted and migrated to one
//! window on load.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
        Err(e) => {
            tracing::warn!("session: failed to parse {}: {e}", path.display());
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
