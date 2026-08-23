//! Configuration for fastty terminal emulator.
//!
//! Primary source: `fastty.toml`. Search order:
//!   1. `./fastty.toml` (cwd, portable)
//!   2. `/etc/fastty/fastty.toml` (system-wide)
//!   3. `~/.config/fastty/fastty.toml` (user)
//!
//! Legacy `config.json` from <= v0.2.7 is auto-migrated on first launch
//! (user path only) and the original is renamed to `config.json.bak`.

use serde::{Deserialize, Serialize};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use toml_edit::{DocumentMut, Item, Table, value};

pub static ACTIVE_THEME: RwLock<String> = RwLock::new(String::new());

static CONFIG_VERSION: AtomicU64 = AtomicU64::new(1);

pub fn current_config_version() -> u64 {
    CONFIG_VERSION.load(Ordering::Relaxed)
}

pub fn increment_config_version() {
    CONFIG_VERSION.fetch_add(1, Ordering::SeqCst);
}

static LAST_APPLIED_HASH: RwLock<u64> = RwLock::new(0);

const WATCH_DEBOUNCE: Duration = Duration::from_millis(250);

pub fn content_hash(bytes: &[u8]) -> u64 {
    seahash::hash(bytes)
}

pub fn set_last_applied_hash(h: u64) {
    *LAST_APPLIED_HASH.write() = h;
}

pub fn last_applied_hash() -> u64 {
    *LAST_APPLIED_HASH.read()
}

pub fn set_active_theme(theme: &str) {
    *ACTIVE_THEME.write() = theme.to_string();
}

pub const BUILTIN_THEMES: &[&str] = &[
    "default",
    "catppuccin",
    "one-dark",
    "solarized-dark",
    "high-contrast",
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
    crate::paths::get().config_dir.join("themes")
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
                if let Ok(theme) = serde_json::from_str::<ThemeFile>(&content) {
                    map.insert(name, theme);
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
    #[serde(default)]
    pub font: FontConfig,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default = "default_scrollback")]
    pub scrollback: usize,
    #[serde(default = "default_theme")]
    pub theme: Option<String>,
    #[serde(default)]
    pub keybindings: std::collections::HashMap<String, String>,
    #[serde(default = "default_session_restore")]
    pub session_restore: bool,
    #[serde(default)]
    pub copy_on_select: bool,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_notify_on_command_finish")]
    pub notify_on_command_finish: bool,
    #[serde(default)]
    pub bottombar: BottombarConfig,
    #[serde(default)]
    pub cursor: CursorConfig,
    /// Treat the Option key as Alt (send `ESC` + key) instead of letting
    /// macOS compose the layout character. Mirrors zed's `option_as_meta`
    /// and ghostty's `macos-option-as-alt`; default is false so Option
    /// keys produce the real character (e.g. `~` via option+n on Latam).
    #[serde(default = "default_option_as_meta")]
    pub option_as_meta: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BottombarConfig {
    #[serde(default)]
    pub widgets: Vec<WidgetSpec>,
}

/// Configuration for one bottombar widget.
///
/// `kebab-case` `type` maps to a built-in widget id (`git`, `time`, `kube`,
/// `aws`, `command`). Unknown types are skipped at load time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum WidgetSpec {
    Git {
        #[serde(default)]
        align: Option<AlignSpec>,
        #[serde(default)]
        interval_ms: Option<u64>,
    },
    GitActions {
        #[serde(default)]
        align: Option<AlignSpec>,
        #[serde(default)]
        interval_ms: Option<u64>,
    },
    Time {
        #[serde(default)]
        format: Option<String>,
        #[serde(default)]
        align: Option<AlignSpec>,
        #[serde(default)]
        interval_ms: Option<u64>,
        #[serde(default)]
        timezone: Option<i32>,
    },
    Kube {
        #[serde(default)]
        align: Option<AlignSpec>,
        #[serde(default)]
        interval_ms: Option<u64>,
    },
    Aws {
        #[serde(default)]
        align: Option<AlignSpec>,
        #[serde(default)]
        interval_ms: Option<u64>,
    },
    Command {
        name: String,
        command: String,
        #[serde(default)]
        on_click: Option<String>,
        #[serde(default)]
        align: Option<AlignSpec>,
        #[serde(default)]
        interval_ms: Option<u64>,
    },
    GitSync {
        #[serde(default)]
        align: Option<AlignSpec>,
        #[serde(default)]
        interval_ms: Option<u64>,
    },
    GitPrs {
        #[serde(default)]
        align: Option<AlignSpec>,
        #[serde(default)]
        interval_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignSpec {
    Left,
    Right,
}

impl From<AlignSpec> for crate::widgets::Align {
    fn from(a: AlignSpec) -> Self {
        match a {
            AlignSpec::Left => crate::widgets::Align::Left,
            AlignSpec::Right => crate::widgets::Align::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorShapeConfig {
    Block,
    Beam,
    Underline,
    HollowBlock,
}

impl Default for CursorShapeConfig {
    fn default() -> Self {
        CursorShapeConfig::Beam
    }
}

impl From<CursorShapeConfig> for alacritty_terminal::vte::ansi::CursorShape {
    fn from(c: CursorShapeConfig) -> Self {
        match c {
            CursorShapeConfig::Block => alacritty_terminal::vte::ansi::CursorShape::Block,
            CursorShapeConfig::Beam => alacritty_terminal::vte::ansi::CursorShape::Beam,
            CursorShapeConfig::Underline => alacritty_terminal::vte::ansi::CursorShape::Underline,
            CursorShapeConfig::HollowBlock => alacritty_terminal::vte::ansi::CursorShape::HollowBlock,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorConfig {
    #[serde(default)]
    pub shape: CursorShapeConfig,
    #[serde(default = "default_true")]
    pub blink: bool,
    #[serde(default = "default_true")]
    pub smooth: bool,
    #[serde(default = "default_cursor_anim_duration")]
    pub animation_duration_ms: u64,
}

fn default_true() -> bool { true }
fn default_cursor_anim_duration() -> u64 { 80 }

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            shape: CursorShapeConfig::default(),
            blink: true,
            smooth: true,
            animation_duration_ms: default_cursor_anim_duration(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontConfig {
    #[serde(default = "default_font_family")]
    pub family: String,
    #[serde(default = "default_font_size")]
    pub size: f32,
    #[serde(default = "default_font_weight")]
    pub weight: f32,
    #[serde(default = "default_font_ligatures")]
    pub ligatures: bool,
}

fn default_scrollback() -> usize { 1000 }
fn default_theme() -> Option<String> { Some("default".to_string()) }
fn default_font_family() -> String { "monospace".to_string() }
fn default_font_size() -> f32 { 14.0 }
fn default_font_weight() -> f32 { 400.0 }
fn default_font_ligatures() -> bool { true }
fn default_session_restore() -> bool { true }
fn default_opacity() -> f32 { 1.0 }
fn default_notify_on_command_finish() -> bool { true }
fn default_option_as_meta() -> bool { false }

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            shell: None,
            scrollback: default_scrollback(),
            theme: default_theme(),
            keybindings: std::collections::HashMap::new(),
            session_restore: default_session_restore(),
            copy_on_select: false,
            opacity: default_opacity(),
            notify_on_command_finish: default_notify_on_command_finish(),
            bottombar: BottombarConfig::default(),
            cursor: CursorConfig::default(),
            option_as_meta: default_option_as_meta(),
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: default_font_family(),
            size: default_font_size(),
            weight: default_font_weight(),
            ligatures: default_font_ligatures(),
        }
    }
}

fn user_toml_path() -> PathBuf {
    crate::paths::get().config_dir.join("config.toml")
}

fn user_fastty_toml_path() -> PathBuf {
    crate::paths::get().config_dir.join("fastty.toml")
}

fn user_legacy_json_path() -> PathBuf {
    crate::paths::get().config_dir.join("config.json")
}

fn candidate_toml_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("config.toml"),
        PathBuf::from("fastty.toml"),
        PathBuf::from("/etc/fastty/config.toml"),
        PathBuf::from("/etc/fastty/fastty.toml"),
        user_toml_path(),
        user_fastty_toml_path(),
    ]
}

fn atomic_write(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    std::fs::write(&tmp, contents)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

fn ensure_table(doc: &mut DocumentMut, key: &str) {
    let needs_init = !doc.contains_key(key) || !doc[key].is_table();
    if needs_init {
        doc[key] = Item::Table(Table::new());
    }
}

fn apply_to_doc(doc: &mut DocumentMut, c: &Config) {
    doc["scrollback"] = value(c.scrollback as i64);
    doc["opacity"] = value(c.opacity as f64);
    doc["option_as_meta"] = value(c.option_as_meta);
    match &c.shell {
        Some(s) => doc["shell"] = value(s.as_str()),
        None => { doc.as_table_mut().remove("shell"); }
    }
    match &c.theme {
        Some(s) => doc["theme"] = value(s.as_str()),
        None => { doc.as_table_mut().remove("theme"); }
    }
    ensure_table(doc, "font");
    if let Some(font) = doc["font"].as_table_mut() {
        font["family"] = value(c.font.family.as_str());
        font["size"] = value(c.font.size as f64);
        font["weight"] = value(c.font.weight as f64);
        font["ligatures"] = value(c.font.ligatures);
    }
    ensure_table(doc, "cursor");
    if let Some(cursor) = doc["cursor"].as_table_mut() {
        let shape_str = match c.cursor.shape {
            CursorShapeConfig::Block => "block",
            CursorShapeConfig::Beam => "beam",
            CursorShapeConfig::Underline => "underline",
            CursorShapeConfig::HollowBlock => "hollow_block",
        };
        cursor["shape"] = value(shape_str);
        cursor["blink"] = value(c.cursor.blink);
        cursor["smooth"] = value(c.cursor.smooth);
        cursor["animation_duration_ms"] = value(c.cursor.animation_duration_ms as i64);
    }
    ensure_table(doc, "keybindings");
}

fn config_to_toml_string(c: &Config) -> String {
    let mut doc = DocumentMut::new();
    apply_to_doc(&mut doc, c);
    doc.to_string()
}

fn migrate_legacy_json_to_toml(toml_path: &Path, json_path: &Path) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(json_path)?;
    let mut parsed: Config = serde_json::from_str(&content)?;
    parsed.scrollback = parsed.scrollback.min(1000);
    atomic_write(toml_path, config_to_toml_string(&parsed).as_bytes())?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bak = {
        let mut p = json_path.as_os_str().to_owned();
        p.push(format!(".bak.{ts}"));
        PathBuf::from(p)
    };
    std::fs::rename(json_path, &bak)?;
    Ok(())
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let target = user_toml_path();
        let legacy = user_legacy_json_path();
        if !target.exists() && legacy.exists() {
            let _ = migrate_legacy_json_to_toml(&target, &legacy);
        }

        let mut any_existed = false;
        let mut last_err: Option<anyhow::Error> = None;
        for path in candidate_toml_paths() {
            if !path.exists() { continue; }
            any_existed = true;
            let content = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("read {}: {e}", path.display()));
                    continue;
                }
            };
            let h = content_hash(content.as_bytes());
            match toml_edit::de::from_str::<Config>(&content) {
                Ok(mut cfg) => {
                    cfg.scrollback = cfg.scrollback.min(1000);
                    *ACTIVE_THEME.write() = cfg.theme.clone().unwrap_or_else(|| "default".to_string());
                    set_last_applied_hash(h);
                    return Ok(cfg);
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("parse {}: {e}", path.display()));
                }
            }
        }

        if any_existed {
            if let Some(err) = last_err {
                return Err(err.context("all existing config files failed to load; keeping previous state"));
            }
        }

        let def = Config::default();
        *ACTIVE_THEME.write() = def.theme.clone().unwrap_or_else(|| "default".to_string());
        set_last_applied_hash(0);
        Ok(def)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut doc: DocumentMut = match std::fs::read_to_string(path) {
            Ok(s) => s.parse::<DocumentMut>().unwrap_or_else(|_| DocumentMut::new()),
            Err(_) => DocumentMut::new(),
        };
        apply_to_doc(&mut doc, self);
        let serialized = doc.to_string();
        let bytes = serialized.as_bytes();
        set_last_applied_hash(content_hash(bytes));
        atomic_write(path, bytes)?;
        Ok(())
    }

    pub fn config_path() -> PathBuf {
        user_toml_path()
    }

    pub fn get_active_config_path() -> PathBuf {
        for path in candidate_toml_paths() {
            if path.exists() {
                return path;
            }
        }
        Self::config_path()
    }

    pub fn save_default(&self) -> anyhow::Result<()> {
        let path = Self::get_active_config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let res = self.save(&path);
        increment_config_version();
        res
    }
}

pub fn start_config_watcher<F>(file_path: PathBuf, on_change: F) -> anyhow::Result<()>
where
    F: Fn() + Send + 'static,
{
    use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

    let parent = match file_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    if !parent.exists() {
        std::fs::create_dir_all(&parent)?;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(WATCH_DEBOUNCE, tx)?;
    debouncer.watcher().watch(&parent, RecursiveMode::NonRecursive)?;

    let watched_file = file_path.clone();
    let watched_name = watched_file.file_name().map(|n| n.to_owned());
    std::thread::Builder::new()
        .name("fastty-config-watch".into())
        .spawn(move || {
            let _debouncer = debouncer;
            for batch in rx {
                let events = match batch {
                    Ok(ev) => ev,
                    Err(_) => continue,
                };
                let touches_file = events.iter().any(|e| {
                    e.path.file_name() == watched_name.as_deref()
                });
                if !touches_file { continue; }
                let content = match std::fs::read(&watched_file) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let h = content_hash(&content);
                if h == last_applied_hash() {
                    continue;
                }
                on_change();
            }
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_config_toml_parsing() {
        let toml_str = r#"
            theme = "catppuccin"
            [cursor]
            shape = "beam"
            blink = false
            smooth = true
            animation_duration_ms = 120
        "#;
        let cfg: Config = toml_edit::de::from_str(toml_str).unwrap();
        assert_eq!(cfg.cursor.shape, CursorShapeConfig::Beam);
        assert_eq!(cfg.cursor.blink, false);
        assert_eq!(cfg.cursor.smooth, true);
        assert_eq!(cfg.cursor.animation_duration_ms, 120);
    }
}
