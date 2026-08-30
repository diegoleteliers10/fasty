//! User-customizable keybindings.
//!
//! Bindings live in `fastty.toml` under `[keybindings]` as a flat
//! `combo -> action` map (both strings). On startup and on every
//! config reload we re-parse the map and merge on top of the defaults.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeybindingPreset {
    #[default]
    Default,
    Ghostty,
    Tmux,
    ITerm2,
}

impl std::fmt::Display for KeybindingPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::Ghostty => write!(f, "ghostty"),
            Self::Tmux => write!(f, "tmux"),
            Self::ITerm2 => write!(f, "iterm2"),
        }
    }
}

impl std::str::FromStr for KeybindingPreset {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(Self::Default),
            "ghostty" => Ok(Self::Ghostty),
            "tmux" => Ok(Self::Tmux),
            "iterm2" | "iterm" => Ok(Self::ITerm2),
            _ => Ok(Self::Default),
        }
    }
}

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
    ClearScrollback,
    ToggleFullscreen,
    ToggleTabSidebar,
    SplitRight,
    SplitDown,
    SplitLeft,
    SplitTop,
    FocusRight,
    FocusDown,
    FocusLeft,
    FocusTop,
    ClosePane,
    GlobalSearch,
    TabOverview,
    Quit,
}

pub struct KeyBindingResolver {
    bindings: HashMap<KeyCombo, Action>,
}

impl KeyBindingResolver {
    pub fn for_preset(preset: KeybindingPreset) -> Self {
        let mut b = HashMap::new();
        let mut insert = |s: &str, a: Action| {
            if let Some(c) = parse_combo(s) {
                b.insert(c, a);
            }
        };

        if cfg!(target_os = "macos") {
            insert("super+c", Action::Copy);
            insert("super+v", Action::Paste);
            insert("shift+insert", Action::Paste);
            insert("super+equal", Action::IncreaseFontSize);
            insert("super+plus", Action::IncreaseFontSize);
            insert("super+shift+plus", Action::IncreaseFontSize);
            insert("super+shift+equal", Action::IncreaseFontSize);
            insert("super+minus", Action::DecreaseFontSize);
            insert("super+0", Action::ResetFontSize);
            insert("super+q", Action::Quit);
        } else {
            insert("ctrl+shift+c", Action::Copy);
            insert("ctrl+shift+v", Action::Paste);
            insert("shift+insert", Action::Paste);
            insert("ctrl+equal", Action::IncreaseFontSize);
            insert("ctrl+plus", Action::IncreaseFontSize);
            insert("ctrl+shift+plus", Action::IncreaseFontSize);
            insert("ctrl+shift+equal", Action::IncreaseFontSize);
            insert("ctrl+minus", Action::DecreaseFontSize);
            insert("ctrl+0", Action::ResetFontSize);
            insert("alt+f4", Action::Quit);
            insert("ctrl+q", Action::Quit);
        }

        match preset {
            KeybindingPreset::Default => {
                if cfg!(target_os = "macos") {
                    insert("super+t", Action::NewTab);
                    insert("super+w", Action::ClosePane);
                    insert("super+shift+w", Action::CloseTab);
                    insert("super+b", Action::ToggleTabSidebar);
                    insert("super+n", Action::NewWindow);
                    insert("super+f", Action::OpenSearch);
                    insert("super+s", Action::OpenSettings);
                    insert("super+comma", Action::OpenSettings);
                    insert("super+r", Action::ReloadConfig);
                    insert("super+p", Action::CommandPalette);
                    insert("super+o", Action::SshManager);
                    insert("super+j", Action::ProjectJumper);
                    insert("super+alt+w", Action::WorktreePicker);
                    insert("super+k", Action::ClearScrollback);
                    insert("ctrl+super+f", Action::ToggleFullscreen);
                    insert("f11", Action::ToggleFullscreen);
                    insert("super+shift+]", Action::NextTab);
                    insert("super+shift+[", Action::PrevTab);
                    insert("ctrl+tab", Action::NextTab);
                    insert("ctrl+shift+tab", Action::PrevTab);
                    insert("super+shift+up", Action::PrevPrompt);
                    insert("super+shift+h", Action::PrevPrompt);
                    insert("super+shift+down", Action::NextPrompt);
                    for n in 1..=9u8 {
                        insert(&format!("super+{n}"), Action::SelectTab(n));
                        insert(&format!("alt+{n}"), Action::SelectTab(n));
                    }
                    insert("ctrl+shift+c", Action::Copy);
                    insert("ctrl+shift+v", Action::Paste);
                    insert("ctrl+shift+t", Action::NewTab);
                    insert("ctrl+shift+w", Action::ClosePane);
                    insert("super+d", Action::SplitRight);
                    insert("super+shift+d", Action::SplitDown);
                    insert("super+alt+left", Action::FocusLeft);
                    insert("super+alt+right", Action::FocusRight);
                    insert("super+alt+up", Action::FocusTop);
                    insert("super+alt+down", Action::FocusDown);
                    insert("super+shift+f", Action::GlobalSearch);
                    insert("super+shift+o", Action::TabOverview);
                } else {
                    insert("ctrl+shift+t", Action::NewTab);
                    insert("ctrl+shift+w", Action::ClosePane);
                    insert("ctrl+shift+q", Action::CloseTab);
                    insert("ctrl+b", Action::ToggleTabSidebar);
                    insert("ctrl+shift+b", Action::ToggleTabSidebar);
                    insert("ctrl+shift+n", Action::NewWindow);
                    insert("ctrl+shift+e", Action::SplitRight);
                    insert("ctrl+shift+d", Action::SplitDown);
                    insert("alt+left", Action::FocusLeft);
                    insert("alt+right", Action::FocusRight);
                    insert("alt+up", Action::FocusTop);
                    insert("alt+down", Action::FocusDown);
                    insert("ctrl+f", Action::OpenSearch);
                    insert("ctrl+shift+f", Action::GlobalSearch);
                    insert("ctrl+shift+m", Action::TabOverview);
                    insert("ctrl+comma", Action::OpenSettings);
                    insert("ctrl+shift+s", Action::OpenSettings);
                    insert("ctrl+shift+r", Action::ReloadConfig);
                    insert("ctrl+shift+p", Action::CommandPalette);
                    insert("ctrl+shift+o", Action::SshManager);
                    insert("ctrl+shift+j", Action::ProjectJumper);
                    insert("ctrl+alt+w", Action::WorktreePicker);
                    insert("ctrl+shift+k", Action::ClearScrollback);
                    insert("f11", Action::ToggleFullscreen);
                    insert("f5", Action::ReloadConfig);
                    insert("f10", Action::ReloadConfig);
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
                }
            }
            KeybindingPreset::Ghostty => {
                if cfg!(target_os = "macos") {
                    insert("super+t", Action::NewTab);
                    insert("super+w", Action::ClosePane);
                    insert("super+shift+w", Action::CloseTab);
                    insert("super+n", Action::NewWindow);
                    insert("super+d", Action::SplitRight);
                    insert("super+shift+d", Action::SplitDown);
                    insert("super+alt+left", Action::FocusLeft);
                    insert("super+alt+right", Action::FocusRight);
                    insert("super+alt+up", Action::FocusTop);
                    insert("super+alt+down", Action::FocusDown);
                    insert("super+shift+j", Action::FocusDown);
                    insert("super+shift+k", Action::FocusTop);
                    insert("super+comma", Action::OpenSettings);
                    insert("super+shift+p", Action::CommandPalette);
                    insert("super+k", Action::ClearScrollback);
                    insert("super+f", Action::OpenSearch);
                    insert("super+shift+f", Action::GlobalSearch);
                    insert("super+shift+o", Action::TabOverview);
                    insert("ctrl+super+f", Action::ToggleFullscreen);
                    insert("super+enter", Action::ToggleFullscreen);
                    insert("super+shift+]", Action::NextTab);
                    insert("super+shift+[", Action::PrevTab);
                    for n in 1..=9u8 {
                        insert(&format!("super+{n}"), Action::SelectTab(n));
                    }
                } else {
                    insert("ctrl+shift+t", Action::NewTab);
                    insert("ctrl+shift+w", Action::ClosePane);
                    insert("ctrl+shift+q", Action::CloseTab);
                    insert("ctrl+shift+n", Action::NewWindow);
                    insert("ctrl+shift+o", Action::SplitRight);
                    insert("ctrl+shift+e", Action::SplitDown);
                    insert("ctrl+shift+left", Action::FocusLeft);
                    insert("ctrl+shift+right", Action::FocusRight);
                    insert("ctrl+shift+up", Action::FocusTop);
                    insert("ctrl+shift+down", Action::FocusDown);
                    insert("ctrl+comma", Action::OpenSettings);
                    insert("ctrl+shift+p", Action::CommandPalette);
                    insert("ctrl+shift+k", Action::ClearScrollback);
                    insert("ctrl+shift+f", Action::GlobalSearch);
                    insert("ctrl+shift+m", Action::TabOverview);
                    insert("f11", Action::ToggleFullscreen);
                    insert("ctrl+tab", Action::NextTab);
                    insert("ctrl+shift+tab", Action::PrevTab);
                }
            }
            KeybindingPreset::Tmux => {
                insert("ctrl+b", Action::ToggleTabSidebar);
                insert("ctrl+t", Action::NewTab);
                insert("ctrl+w", Action::TabOverview);
                insert("ctrl+d", Action::SplitRight);
                insert("ctrl+shift+d", Action::SplitDown);
                insert("alt+left", Action::FocusLeft);
                insert("alt+right", Action::FocusRight);
                insert("alt+up", Action::FocusTop);
                insert("alt+down", Action::FocusDown);
                insert("alt+h", Action::FocusLeft);
                insert("alt+l", Action::FocusRight);
                insert("alt+k", Action::FocusTop);
                insert("alt+j", Action::FocusDown);
                insert("ctrl+k", Action::ClearScrollback);
                insert("ctrl+f", Action::OpenSearch);
                insert("ctrl+shift+f", Action::GlobalSearch);
                insert("ctrl+p", Action::CommandPalette);
                insert("f11", Action::ToggleFullscreen);
                for n in 1..=9u8 {
                    insert(&format!("alt+{n}"), Action::SelectTab(n));
                }
            }
            KeybindingPreset::ITerm2 => {
                insert("super+t", Action::NewTab);
                insert("super+w", Action::ClosePane);
                insert("super+shift+w", Action::CloseTab);
                insert("super+d", Action::SplitRight);
                insert("super+shift+d", Action::SplitDown);
                insert("super+alt+left", Action::FocusLeft);
                insert("super+alt+right", Action::FocusRight);
                insert("super+alt+up", Action::FocusTop);
                insert("super+alt+down", Action::FocusDown);
                insert("super+]", Action::NextTab);
                insert("super+[", Action::PrevTab);
                insert("super+f", Action::OpenSearch);
                insert("super+shift+f", Action::GlobalSearch);
                insert("super+alt+o", Action::TabOverview);
                insert("super+k", Action::ClearScrollback);
                insert("super+comma", Action::OpenSettings);
                insert("super+shift+o", Action::SshManager);
                for n in 1..=9u8 {
                    insert(&format!("super+{n}"), Action::SelectTab(n));
                }
            }
        }

        Self { bindings: b }
    }

    pub fn with_defaults() -> Self {
        Self::for_preset(KeybindingPreset::Default)
    }

    pub fn apply_user(&mut self, user: HashMap<String, String>) {
        for (combo_str, action_str) in user {
            let Some(combo) = parse_combo(&combo_str) else {
                continue;
            };
            let Some(action) = parse_action(&action_str) else {
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
        "comma" => Some(NamedKey::Char(',')),
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
        "clear_scrollback" => Some(Action::ClearScrollback),
        "toggle_fullscreen" => Some(Action::ToggleFullscreen),
        "toggle_tab_sidebar" => Some(Action::ToggleTabSidebar),
        "split_right" => Some(Action::SplitRight),
        "split_down" => Some(Action::SplitDown),
        "split_left" => Some(Action::SplitLeft),
        "split_top" => Some(Action::SplitTop),
        "focus_right" => Some(Action::FocusRight),
        "focus_down" => Some(Action::FocusDown),
        "focus_left" => Some(Action::FocusLeft),
        "focus_top" => Some(Action::FocusTop),
        "close_pane" => Some(Action::ClosePane),
        "global_search" | "search_all" | "multi_tab_search" => Some(Action::GlobalSearch),
        "tab_overview" | "mission_control" | "tab_peek" => Some(Action::TabOverview),
        "quit" => Some(Action::Quit),
        _ => None,
    }
}

pub static RESOLVER: std::sync::OnceLock<RwLock<KeyBindingResolver>> = std::sync::OnceLock::new();

pub fn init_resolver(user: HashMap<String, String>, preset: Option<KeybindingPreset>) {
    let mut r = KeyBindingResolver::for_preset(preset.unwrap_or_default());
    r.apply_user(user);
    let lock = RESOLVER.get_or_init(|| RwLock::new(KeyBindingResolver { bindings: HashMap::new() }));
    *lock.write() = r;
}

pub fn combo_from_key(
    key_str: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
    logo: bool,
) -> Option<KeyCombo> {
    let lower = key_str.to_lowercase();
    let key = match lower.as_str() {
        "f1" => NamedKey::F1,
        "f2" => NamedKey::F2,
        "f3" => NamedKey::F3,
        "f4" => NamedKey::F4,
        "f5" => NamedKey::F5,
        "f6" => NamedKey::F6,
        "f7" => NamedKey::F7,
        "f8" => NamedKey::F8,
        "f9" => NamedKey::F9,
        "f10" => NamedKey::F10,
        "f11" => NamedKey::F11,
        "f12" => NamedKey::F12,
        "up" | "arrowup" => NamedKey::Up,
        "down" | "arrowdown" => NamedKey::Down,
        "left" | "arrowleft" => NamedKey::Left,
        "right" | "arrowright" => NamedKey::Right,
        "enter" | "return" => NamedKey::Return,
        "tab" => NamedKey::Tab,
        "escape" | "esc" => NamedKey::Escape,
        "backspace" => NamedKey::Backspace,
        "delete" => NamedKey::Delete,
        "insert" => NamedKey::Insert,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        "space" => NamedKey::Space,
        "+" => NamedKey::Plus,
        "-" => NamedKey::Minus,
        "=" => NamedKey::Equal,
        s if s.chars().count() == 1 => NamedKey::Char(s.chars().next()?),
        _ => return None,
    };
    Some(KeyCombo { ctrl, shift, alt, logo, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_resolvers() {
        let ghostty = KeyBindingResolver::for_preset(KeybindingPreset::Ghostty);
        assert!(!ghostty.bindings.is_empty());

        let tmux = KeyBindingResolver::for_preset(KeybindingPreset::Tmux);
        assert!(!tmux.bindings.is_empty());

        let iterm = KeyBindingResolver::for_preset(KeybindingPreset::ITerm2);
        assert!(!iterm.bindings.is_empty());
    }
}
