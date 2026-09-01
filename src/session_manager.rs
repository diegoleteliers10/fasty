use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedPane {
    pub id: usize,
    pub title: String,
    pub custom_title: Option<String>,
    pub cwd: Option<String>,
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
    pub title: String,
    pub custom_title: Option<String>,
    pub cwd: Option<String>,
    pub layout: Option<PersistedPaneNode>,
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
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let dir = PathBuf::from(home).join(".config/fastty/sessions");
    let _ = fs::create_dir_all(&dir);
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
    fs::write(path, json)?;
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
                title: "Terminal".to_string(),
                custom_title: Some("Dev Split".to_string()),
                cwd: Some("/tmp".to_string()),
                layout: Some(PersistedPaneNode::Split {
                    direction: "Horizontal".to_string(),
                    ratio: 0.5,
                    first: Box::new(PersistedPaneNode::Leaf(PersistedPane {
                        id: 1,
                        title: "Left".to_string(),
                        custom_title: None,
                        cwd: Some("/tmp/left".to_string()),
                    })),
                    second: Box::new(PersistedPaneNode::Leaf(PersistedPane {
                        id: 2,
                        title: "Right".to_string(),
                        custom_title: None,
                        cwd: Some("/tmp/right".to_string()),
                    })),
                }),
            }],
        };

        let json = serde_json::to_string(&session).unwrap();
        let deserialized: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "split-workspace");
        assert!(matches!(deserialized.tabs[0].layout, Some(PersistedPaneNode::Split { .. })));
    }
}
