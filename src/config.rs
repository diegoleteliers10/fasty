//! Configuration for fasty terminal emulator.
//!
//! Loads from config.json in the same directory as the binary.

use serde::{Deserialize, Serialize};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::OnceLock;

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

/// In-memory cache of user-supplied themes, loaded once at startup.
pub static CUSTOM_THEMES: OnceLock<RwLock<HashMap<String, ThemeFile>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeFile {
    pub background: String,
    pub foreground: String,
    #[serde(default = "default_color")]
    pub black: String,
    #[serde(default = "default_color")]
    pub red: String,
    #[serde(default = "default_color")]
    pub green: String,
    #[serde(default = "default_color")]
    pub yellow: String,
    #[serde(default = "default_color")]
    pub blue: String,
    #[serde(default = "default_color")]
    pub magenta: String,
    #[serde(default = "default_color")]
    pub cyan: String,
    #[serde(default = "default_color")]
    pub white: String,
    #[serde(default = "default_color")]
    pub bright_black: String,
    #[serde(default = "default_color")]
    pub bright_red: String,
    #[serde(default = "default_color")]
    pub bright_green: String,
    #[serde(default = "default_color")]
    pub bright_yellow: String,
    #[serde(default = "default_color")]
    pub bright_blue: String,
    #[serde(default = "default_color")]
    pub bright_magenta: String,
    #[serde(default = "default_color")]
    pub bright_cyan: String,
    #[serde(default = "default_color")]
    pub bright_white: String,
}

fn default_color() -> String { "#000000".to_string() }

/// Parse "#rrggbb" (case-insensitive, with or without leading #) into (u8, u8, u8).
pub fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 { return None; }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

fn themes_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    std::path::Path::new(&home).join(".config/fasty/themes")
}

pub fn load_custom_themes() {
    let dir = themes_dir();
    let mut map: HashMap<String, ThemeFile> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if let Ok(content) = std::fs::read_to_string(&path) {
                match serde_json::from_str::<ThemeFile>(&content) {
                    Ok(theme) => { map.insert(name, theme); }
                    Err(e) => tracing::warn!("Failed to parse theme {:?}: {:?}", path, e),
                }
            }
        }
    }
    let _ = CUSTOM_THEMES.set(RwLock::new(map));
}

pub fn all_theme_names() -> Vec<String> {
    let mut names: Vec<String> = BUILTIN_THEMES.iter().map(|s| s.to_string()).collect();
    if let Some(custom) = CUSTOM_THEMES.get() {
        let c = custom.read();
        for k in c.keys() {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    names
}

pub fn try_get_custom_theme(name: &str) -> Option<(u8, u8, u8)> {
    let custom = CUSTOM_THEMES.get()?.read();
    let theme = custom.get(name)?;
    parse_hex_color(&theme.background)
}

/// Returns the full 18-color palette for a custom theme (background, foreground, then the
/// 16 ANSI colors in standard order). Returns None if the theme doesn't exist or
/// any required field is unparseable. Falls back to bg/fg for missing color slots.
pub fn try_get_custom_theme_full(name: &str) -> Option<[Option<(u8, u8, u8)>; 18]> {
    let custom = CUSTOM_THEMES.get()?.read();
    let t = custom.get(name)?;
    let bg = parse_hex_color(&t.background)?;
    let fg = parse_hex_color(&t.foreground)?;
    let slots = [
        t.black.as_str(),
        t.red.as_str(),
        t.green.as_str(),
        t.yellow.as_str(),
        t.blue.as_str(),
        t.magenta.as_str(),
        t.cyan.as_str(),
        t.white.as_str(),
        t.bright_black.as_str(),
        t.bright_red.as_str(),
        t.bright_green.as_str(),
        t.bright_yellow.as_str(),
        t.bright_blue.as_str(),
        t.bright_magenta.as_str(),
        t.bright_cyan.as_str(),
        t.bright_white.as_str(),
    ];
    let mut out: [Option<(u8, u8, u8)>; 18] = [None; 18];
    out[0] = Some(bg);
    out[1] = Some(fg);
    for (i, s) in slots.iter().enumerate() {
        out[i + 2] = parse_hex_color(s);
    }
    Some(out)
}

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
