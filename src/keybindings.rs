//! User-customizable keybindings.
//!
//! Bindings live in `fasty.toml` under `[keybindings]` as a flat
//! `combo -> action` map (both strings). On startup and on every
//! config reload we re-parse the map and merge on top of the defaults.

use parking_lot::RwLock;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
    pub key: NamedKey,
}

impl std::fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl { write!(f, "ctrl+")?; }
        if self.shift { write!(f, "shift+")?; }
        if self.alt { write!(f, "alt+")?; }
        if self.logo { write!(f, "super+")?; }
        match self.key {
            NamedKey::Char(c) => write!(f, "{}", c),
            NamedKey::F1 => write!(f, "f1"),
            NamedKey::F2 => write!(f, "f2"),
            NamedKey::F3 => write!(f, "f3"),
            NamedKey::F4 => write!(f, "f4"),
            NamedKey::F5 => write!(f, "f5"),
            NamedKey::F6 => write!(f, "f6"),
            NamedKey::F7 => write!(f, "f7"),
            NamedKey::F8 => write!(f, "f8"),
            NamedKey::F9 => write!(f, "f9"),
            NamedKey::F10 => write!(f, "f10"),
            NamedKey::F11 => write!(f, "f11"),
            NamedKey::F12 => write!(f, "f12"),
            NamedKey::Up => write!(f, "up"),
            NamedKey::Down => write!(f, "down"),
            NamedKey::Left => write!(f, "left"),
            NamedKey::Right => write!(f, "right"),
            NamedKey::Return => write!(f, "return"),
            NamedKey::Tab => write!(f, "tab"),
            NamedKey::Escape => write!(f, "escape"),
            NamedKey::Backspace => write!(f, "backspace"),
            NamedKey::Delete => write!(f, "delete"),
            NamedKey::Insert => write!(f, "insert"),
            NamedKey::Home => write!(f, "home"),
            NamedKey::End => write!(f, "end"),
            NamedKey::PageUp => write!(f, "pageup"),
            NamedKey::PageDown => write!(f, "pagedown"),
            NamedKey::Space => write!(f, "space"),
            NamedKey::Plus => write!(f, "+"),
            NamedKey::Minus => write!(f, "-"),
            NamedKey::Equal => write!(f, "="),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum NamedKey {
    Char(char),
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Up, Down, Left, Right,
    Return, Tab, Escape, Backspace, Delete, Insert, Home, End, PageUp, PageDown,
    Space,
    Plus, Minus, Equal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    CloseTab,
    NewWindow,
    Copy,
    Paste,
    OpenSearch,
    OpenSettings,
    ReloadConfig,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    NextTab,
    PrevTab,
    SelectTab(u8),
    CommandPalette,
    SshManager,
    ProjectJumper,
    WorktreePicker,
    PrevPrompt,
    NextPrompt,
}

pub struct KeyBindingResolver {
    bindings: HashMap<KeyCombo, Action>,
}

impl KeyBindingResolver {
    pub fn with_defaults() -> Self {
        let mut b = HashMap::new();
        let mut insert = |s: &str, a: Action| {
            if let Some(c) = parse_combo(s) {
                b.insert(c, a);
            }
        };
        insert("ctrl+shift+t", Action::NewTab);
        insert("ctrl+shift+w", Action::CloseTab);
        insert("ctrl+shift+n", Action::NewWindow);
        insert("ctrl+shift+c", Action::Copy);
        insert("ctrl+shift+v", Action::Paste);
        insert("ctrl+f", Action::OpenSearch);
        insert("ctrl+shift+s", Action::OpenSettings);
        insert("ctrl+shift+r", Action::ReloadConfig);
        insert("ctrl+shift+p", Action::CommandPalette);
        insert("ctrl+shift+o", Action::SshManager);
        insert("ctrl+shift+j", Action::ProjectJumper);
        insert("ctrl+alt+w", Action::WorktreePicker);
        insert("f5", Action::ReloadConfig);
        insert("f10", Action::ReloadConfig);
        insert("ctrl+equal", Action::IncreaseFontSize);
        insert("ctrl+plus", Action::IncreaseFontSize);
        insert("ctrl+shift+plus", Action::IncreaseFontSize);
        insert("ctrl+shift+equal", Action::IncreaseFontSize);
        insert("ctrl+minus", Action::DecreaseFontSize);
        insert("ctrl+0", Action::ResetFontSize);
        insert("ctrl+tab", Action::NextTab);
        insert("ctrl+page_down", Action::NextTab);
        insert("ctrl+shift+tab", Action::PrevTab);
        insert("ctrl+page_up", Action::PrevTab);
        insert("ctrl+shift+up", Action::PrevPrompt);
        insert("ctrl+shift+h", Action::PrevPrompt);
        insert("ctrl+shift+down", Action::NextPrompt);
        for n in 1..=9u8 {
            insert(&format!("alt+{n}"), Action::SelectTab(n));
        }
        Self { bindings: b }
    }

    pub fn apply_user(&mut self, user: HashMap<String, String>) {
        for (combo_str, action_str) in user {
            let Some(combo) = parse_combo(&combo_str) else {
                tracing::warn!("keybindings: cannot parse combo {:?}", combo_str);
                continue;
            };
            let Some(action) = parse_action(&action_str) else {
                tracing::warn!("keybindings: unknown action {:?} (combo={:?})", action_str, combo_str);
                continue;
            };
            self.bindings.insert(combo, action);
        }
    }

    pub fn resolve(&self, combo: &KeyCombo) -> Option<Action> {
        self.bindings.get(combo).copied()
    }
}

pub fn parse_combo(s: &str) -> Option<KeyCombo> {
    let parts: Vec<&str> = s.split('+').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() { return None; }
    let key_str = parts.last()?;
    let key = parse_key(key_str)?;
    let mut combo = KeyCombo { ctrl: false, shift: false, alt: false, logo: false, key };
    for mod_str in &parts[..parts.len() - 1] {
        match mod_str.to_lowercase().as_str() {
            "ctrl" | "control" => combo.ctrl = true,
            "shift" => combo.shift = true,
            "alt" | "option" => combo.alt = true,
            "super" | "cmd" | "meta" | "win" => combo.logo = true,
            _ => return None,
        }
    }
    Some(combo)
}

fn parse_key(s: &str) -> Option<NamedKey> {
    if s.chars().count() == 1 {
        return s.chars().next().map(NamedKey::Char);
    }
    match s.to_lowercase().as_str() {
        "f1" => Some(NamedKey::F1),
        "f2" => Some(NamedKey::F2),
        "f3" => Some(NamedKey::F3),
        "f4" => Some(NamedKey::F4),
        "f5" => Some(NamedKey::F5),
        "f6" => Some(NamedKey::F6),
        "f7" => Some(NamedKey::F7),
        "f8" => Some(NamedKey::F8),
        "f9" => Some(NamedKey::F9),
        "f10" => Some(NamedKey::F10),
        "f11" => Some(NamedKey::F11),
        "f12" => Some(NamedKey::F12),
        "up" => Some(NamedKey::Up),
        "down" => Some(NamedKey::Down),
        "left" => Some(NamedKey::Left),
        "right" => Some(NamedKey::Right),
        "return" | "enter" => Some(NamedKey::Return),
        "tab" => Some(NamedKey::Tab),
        "escape" | "esc" => Some(NamedKey::Escape),
        "backspace" => Some(NamedKey::Backspace),
        "delete" | "del" => Some(NamedKey::Delete),
        "insert" | "ins" => Some(NamedKey::Insert),
        "home" => Some(NamedKey::Home),
        "end" => Some(NamedKey::End),
        "pageup" | "page_up" => Some(NamedKey::PageUp),
        "pagedown" | "page_down" => Some(NamedKey::PageDown),
        "space" => Some(NamedKey::Space),
        "plus" => Some(NamedKey::Char('+')),
        "minus" => Some(NamedKey::Char('-')),
        "equal" | "equals" => Some(NamedKey::Char('=')),
        _ => None,
    }
}

pub fn parse_action(s: &str) -> Option<Action> {
    if let Some(n) = s.strip_prefix("select_tab_").and_then(|x| x.parse::<u8>().ok()) {
        if (1..=9).contains(&n) { return Some(Action::SelectTab(n)); }
    }
    match s {
        "new_tab" => Some(Action::NewTab),
        "close_tab" => Some(Action::CloseTab),
        "new_window" => Some(Action::NewWindow),
        "copy" => Some(Action::Copy),
        "paste" => Some(Action::Paste),
        "open_search" => Some(Action::OpenSearch),
        "open_settings" => Some(Action::OpenSettings),
        "reload_config" => Some(Action::ReloadConfig),
        "command_palette" => Some(Action::CommandPalette),
        "ssh_manager" => Some(Action::SshManager),
        "project_jumper" => Some(Action::ProjectJumper),
        "worktree_picker" => Some(Action::WorktreePicker),
        "increase_font_size" => Some(Action::IncreaseFontSize),
        "decrease_font_size" => Some(Action::DecreaseFontSize),
        "reset_font_size" => Some(Action::ResetFontSize),
        "next_tab" => Some(Action::NextTab),
        "prev_tab" => Some(Action::PrevTab),
        "prev_prompt" => Some(Action::PrevPrompt),
        "next_prompt" => Some(Action::NextPrompt),
        _ => None,
    }
}

pub static RESOLVER: std::sync::OnceLock<RwLock<KeyBindingResolver>> = std::sync::OnceLock::new();

pub fn init_resolver(user: HashMap<String, String>) {
    let mut r = KeyBindingResolver::with_defaults();
    r.apply_user(user);
    let lock = RESOLVER.get_or_init(|| RwLock::new(KeyBindingResolver { bindings: HashMap::new() }));
    *lock.write() = r;
}

pub fn combo_from_event(
    event: &winit::event::KeyEvent,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<KeyCombo> {
    use winit::keyboard::{Key, NamedKey as WNamed};
    let key = match &event.logical_key {
        Key::Character(s) => NamedKey::Char(s.chars().next()?.to_ascii_lowercase()),
        Key::Named(WNamed::F1) => NamedKey::F1,
        Key::Named(WNamed::F2) => NamedKey::F2,
        Key::Named(WNamed::F3) => NamedKey::F3,
        Key::Named(WNamed::F4) => NamedKey::F4,
        Key::Named(WNamed::F5) => NamedKey::F5,
        Key::Named(WNamed::F6) => NamedKey::F6,
        Key::Named(WNamed::F7) => NamedKey::F7,
        Key::Named(WNamed::F8) => NamedKey::F8,
        Key::Named(WNamed::F9) => NamedKey::F9,
        Key::Named(WNamed::F10) => NamedKey::F10,
        Key::Named(WNamed::F11) => NamedKey::F11,
        Key::Named(WNamed::F12) => NamedKey::F12,
        Key::Named(WNamed::ArrowUp) => NamedKey::Up,
        Key::Named(WNamed::ArrowDown) => NamedKey::Down,
        Key::Named(WNamed::ArrowLeft) => NamedKey::Left,
        Key::Named(WNamed::ArrowRight) => NamedKey::Right,
        Key::Named(WNamed::Enter) => NamedKey::Return,
        Key::Named(WNamed::Tab) => NamedKey::Tab,
        Key::Named(WNamed::Escape) => NamedKey::Escape,
        Key::Named(WNamed::Backspace) => NamedKey::Backspace,
        Key::Named(WNamed::Delete) => NamedKey::Delete,
        Key::Named(WNamed::Insert) => NamedKey::Insert,
        Key::Named(WNamed::Home) => NamedKey::Home,
        Key::Named(WNamed::End) => NamedKey::End,
        Key::Named(WNamed::PageUp) => NamedKey::PageUp,
        Key::Named(WNamed::PageDown) => NamedKey::PageDown,
        Key::Named(WNamed::Space) => NamedKey::Space,
        _ => return None,
    };
    Some(KeyCombo { ctrl, shift, alt, logo: false, key })
}
