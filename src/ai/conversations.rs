use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const CONVERSATION_STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub scope: &'static str,
}

pub fn discover_skills(cwd: &std::path::Path) -> Vec<SkillEntry> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        roots.push((PathBuf::from(home).join(".agents/skills"), "global"));
    }
    let project_root = git2::Repository::discover(cwd).ok()
        .and_then(|repo| repo.workdir().map(PathBuf::from))
        .unwrap_or_else(|| cwd.to_path_buf());
    roots.push((project_root.join(".agents/skills"), "project"));

    let mut skills = Vec::<SkillEntry>::new();
    for (root, scope) in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            let Ok(contents) = std::fs::read_to_string(&path) else { continue };
            let (name, description) = skill_frontmatter(&contents);
            let Some(name) = name.filter(|name| !name.trim().is_empty()) else { continue };
            let description = description.unwrap_or_default();
            if let Some(existing) = skills.iter_mut().find(|skill| skill.name == name) {
                if scope == "project" { *existing = SkillEntry { name, description, path, scope }; }
            } else {
                skills.push(SkillEntry { name, description, path, scope });
            }
        }
    }
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    skills
}

pub fn load_skill(name: &str, cwd: &std::path::Path) -> Option<(SkillEntry, String)> {
    let entry = discover_skills(cwd).into_iter().find(|skill| skill.name == name)?;
    let contents = std::fs::read_to_string(&entry.path).ok()?;
    let body = contents.strip_prefix("---")?.split_once("---")?.1.trim().to_string();
    Some((entry, body))
}

fn skill_frontmatter(contents: &str) -> (Option<String>, Option<String>) {
    let Some(frontmatter) = contents.strip_prefix("---")
        .and_then(|rest| rest.split_once("---").map(|(frontmatter, _)| frontmatter)) else {
        return (None, None);
    };
    let mut name = None;
    let mut description = None;
    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches(['"', '\'']).to_string();
        match key.trim() {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    }
    (name, description)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub tab_key: String,
    pub cwd: PathBuf,
    pub title: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    pub messages: Vec<crate::ui::ai_sidebar::AiUiMessage>,
    pub context_window: u64,
    pub used_tokens: Option<u64>,
    pub acp_session_id: Option<String>,
    #[serde(default)]
    pub available_commands: Vec<AvailableCommand>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConversationFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    conversations: Vec<Conversation>,
}

pub fn load() -> Vec<Conversation> {
    let path = store_path();
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<ConversationFile>(&contents)
        .map(|file| file.conversations)
        .unwrap_or_default()
}

pub fn save(conversations: &[Conversation]) -> anyhow::Result<()> {
    let path = store_path();
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("conversation store has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let contents = serde_json::to_vec(&ConversationFile {
        version: CONVERSATION_STORE_VERSION,
        conversations: conversations.to_vec(),
    })?;
    std::fs::write(&temp, contents)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

pub fn new_id() -> String {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let serial = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{serial}", now_millis())
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn store_path() -> PathBuf {
    crate::paths::get().state_dir.join("ai-conversations.json")
}
