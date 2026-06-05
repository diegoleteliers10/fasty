//! Persisted tab state across restarts.
//!
//! On graceful exit we write `~/.config/fasty/session.json` with the cwd
//! of each open tab and the active tab index. On startup, if
//! `config.session_restore` is true and the file parses, we spawn a
//! new tab in each saved cwd.
//!
//! We do not track the in-session cwd (no OSC 7 listener for v0.2.9),
//! so the restored cwd is the cwd at the time the tab was spawned.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub tabs: Vec<TabInfo>,
    pub active_tab: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub cwd: Option<PathBuf>,
}

pub fn session_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".config/fasty/session.json")
}

pub fn load() -> Option<Session> {
    let path = session_path();
    let content = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<Session>(&content) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("session: failed to parse {}: {e}", path.display());
            None
        }
    }
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
