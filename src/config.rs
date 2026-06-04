//! Configuration for fasty terminal emulator.
//!
//! Loads from config.json in the same directory as the binary.

use serde::{Deserialize, Serialize};
use parking_lot::RwLock;

pub static ACTIVE_THEME: RwLock<String> = RwLock::new(String::new());

pub fn set_active_theme(theme: &str) {
    *ACTIVE_THEME.write() = theme.to_string();
}

pub const BUILTIN_THEMES: &[&str] = &[
    "default",
    "catppuccin",
    "one-dark",
    "solarized-dark",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub font: FontConfig,
    pub shell: Option<String>,
    pub scrollback: usize,
    pub theme: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontConfig {
    pub family: String,
    pub size: f32,
    pub weight: f32,
    pub ligatures: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontConfig {
                family: "monospace".to_string(),
                size: 14.0,
                weight: 400.0,
                ligatures: true,
            },
            shell: None,
            scrollback: 3000,
            theme: Some("default".to_string()),
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "monospace".to_string(),
            size: 14.0,
            weight: 400.0,
            ligatures: true,
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
        let config_paths: Vec<std::path::PathBuf> = vec![
            std::path::PathBuf::from("config.json"),
            std::path::PathBuf::from("/etc/fasty/config.json"),
            std::path::Path::new(&home)
                .join(".config/fasty/config.json"),
        ];

        for path in &config_paths {
            let p: &std::path::Path = path.as_ref();
            if p.exists() {
                let content = std::fs::read_to_string(path)?;
                let mut config: Config = serde_json::from_str(&content)?;
                config.scrollback = config.scrollback.min(3000);
                *ACTIVE_THEME.write() = config.theme.clone().unwrap_or("default".to_string());
                return Ok(config);
            }
        }

        let def = Config::default();
        *ACTIVE_THEME.write() = def.theme.clone().unwrap_or("default".to_string());
        Ok(def)
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn config_path() -> std::path::PathBuf {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
        std::path::Path::new(&home)
            .join(".config/fasty/config.json")
    }

    pub fn get_active_config_path() -> std::path::PathBuf {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
        let config_paths: Vec<std::path::PathBuf> = vec![
            std::path::PathBuf::from("config.json"),
            std::path::PathBuf::from("/etc/fasty/config.json"),
            std::path::Path::new(&home)
                .join(".config/fasty/config.json"),
        ];

        for path in &config_paths {
            if path.exists() {
                return path.clone();
            }
        }

        Self::config_path()
    }
}
