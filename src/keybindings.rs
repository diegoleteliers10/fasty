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
            NamedKey::Plus => write!(f, "plus"),
            NamedKey::Minus => write!(f, "minus"),
            NamedKey::Equal => write!(f, "equal"),
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
    ToggleAiSidebar,
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
    InsertFilePath,
    Quit,
}

/// Marker value in the user `[keybindings]` map that removes a preset binding.
/// e.g. `"super+t" = "none"` unbinds the preset combo without assigning it.
pub const UNBOUND_MARKERS: &[&str] = &["none", "unbound", "disabled"];

pub fn is_unbound_marker(s: &str) -> bool {
    UNBOUND_MARKERS.contains(&s.to_lowercase().as_str())
}

impl Action {
    /// Every configurable action. `SelectTab` expands to 1..=9.
    pub fn all() -> Vec<Action> {
        let mut v = vec![
            Action::NewTab,
            Action::CloseTab,
            Action::NextTab,
            Action::PrevTab,
        ];
        for n in 1..=9u8 {
            v.push(Action::SelectTab(n));
        }
        v.extend([
            Action::SplitRight,
            Action::SplitDown,
            Action::SplitLeft,
            Action::SplitTop,
            Action::FocusRight,
            Action::FocusDown,
            Action::FocusLeft,
            Action::FocusTop,
            Action::ClosePane,
            Action::PrevPrompt,
            Action::NextPrompt,
            Action::Copy,
            Action::Paste,
            Action::IncreaseFontSize,
            Action::DecreaseFontSize,
            Action::ResetFontSize,
            Action::ClearScrollback,
            Action::ToggleFullscreen,
            Action::ToggleTabSidebar,
            Action::ToggleAiSidebar,
            Action::OpenSearch,
            Action::GlobalSearch,
            Action::TabOverview,
            Action::InsertFilePath,
            Action::CommandPalette,
            Action::SshManager,
            Action::ProjectJumper,
            Action::WorktreePicker,
            Action::NewWindow,
            Action::OpenSettings,
            Action::ReloadConfig,
            Action::Quit,
        ]);
        v
    }

    /// Canonical id used in `fastty.toml` (`select_tab_1`..`select_tab_9`).
    /// Inverse of [`parse_action`] for canonical ids.
    pub fn binding_id(&self) -> String {
        match self {
            Action::SelectTab(n) => format!("select_tab_{n}"),
            Action::NewTab => "new_tab".to_string(),
            Action::CloseTab => "close_tab".to_string(),
            Action::NewWindow => "new_window".to_string(),
            Action::Copy => "copy".to_string(),
            Action::Paste => "paste".to_string(),
            Action::OpenSearch => "open_search".to_string(),
            Action::OpenSettings => "open_settings".to_string(),
            Action::ReloadConfig => "reload_config".to_string(),
            Action::IncreaseFontSize => "increase_font_size".to_string(),
            Action::DecreaseFontSize => "decrease_font_size".to_string(),
            Action::ResetFontSize => "reset_font_size".to_string(),
            Action::NextTab => "next_tab".to_string(),
            Action::PrevTab => "prev_tab".to_string(),
            Action::CommandPalette => "command_palette".to_string(),
            Action::SshManager => "ssh_manager".to_string(),
            Action::ProjectJumper => "project_jumper".to_string(),
            Action::WorktreePicker => "worktree_picker".to_string(),
            Action::PrevPrompt => "prev_prompt".to_string(),
            Action::NextPrompt => "next_prompt".to_string(),
            Action::ClearScrollback => "clear_scrollback".to_string(),
            Action::ToggleFullscreen => "toggle_fullscreen".to_string(),
            Action::ToggleTabSidebar => "toggle_tab_sidebar".to_string(),
            Action::ToggleAiSidebar => "toggle_ai_sidebar".to_string(),
            Action::SplitRight => "split_right".to_string(),
            Action::SplitDown => "split_down".to_string(),
            Action::SplitLeft => "split_left".to_string(),
            Action::SplitTop => "split_top".to_string(),
            Action::FocusRight => "focus_right".to_string(),
            Action::FocusDown => "focus_down".to_string(),
            Action::FocusLeft => "focus_left".to_string(),
            Action::FocusTop => "focus_top".to_string(),
            Action::ClosePane => "close_pane".to_string(),
            Action::GlobalSearch => "global_search".to_string(),
            Action::TabOverview => "tab_overview".to_string(),
            Action::InsertFilePath => "insert_file_path".to_string(),
            Action::Quit => "quit".to_string(),
        }
    }

    /// Short human label for Settings.
    pub fn display_name(&self) -> String {
        match self {
            Action::SelectTab(n) => format!("Select Tab {n}"),
            _ => match self {
                Action::NewTab => "New Tab",
                Action::CloseTab => "Close Tab",
                Action::NewWindow => "New Window",
                Action::Copy => "Copy",
                Action::Paste => "Paste",
                Action::OpenSearch => "Find in Tab",
                Action::OpenSettings => "Open Settings",
                Action::ReloadConfig => "Reload Config",
                Action::IncreaseFontSize => "Increase Font Size",
                Action::DecreaseFontSize => "Decrease Font Size",
                Action::ResetFontSize => "Reset Font Size",
                Action::NextTab => "Next Tab",
                Action::PrevTab => "Previous Tab",
                Action::SelectTab(_) => unreachable!("handled above"),
                Action::CommandPalette => "Command Palette",
                Action::SshManager => "SSH Manager",
                Action::ProjectJumper => "Project Jumper",
                Action::WorktreePicker => "Worktree Picker",
                Action::PrevPrompt => "Previous Prompt",
                Action::NextPrompt => "Next Prompt",
                Action::ClearScrollback => "Clear Scrollback",
                Action::ToggleFullscreen => "Toggle Fullscreen",
                Action::ToggleTabSidebar => "Toggle Tab Sidebar",
                Action::ToggleAiSidebar => "Toggle AI Sidebar",
                Action::SplitRight => "Split Right",
                Action::SplitDown => "Split Down",
                Action::SplitLeft => "Split Left",
                Action::SplitTop => "Split Top",
                Action::FocusRight => "Focus Right Pane",
                Action::FocusDown => "Focus Down Pane",
                Action::FocusLeft => "Focus Left Pane",
                Action::FocusTop => "Focus Top Pane",
                Action::ClosePane => "Close Pane",
                Action::GlobalSearch => "Global Search (All Tabs)",
                Action::TabOverview => "Tab Overview",
                Action::InsertFilePath => "Insert File Path",
                Action::Quit => "Quit",
            }
            .to_string(),
        }
    }

    /// Grouping for the Settings editor.
    pub fn category(&self) -> &'static str {
        match self {
            Action::NewTab | Action::CloseTab | Action::NextTab | Action::PrevTab
            | Action::SelectTab(_) => "Tabs",
            Action::SplitRight | Action::SplitDown | Action::SplitLeft | Action::SplitTop
            | Action::FocusRight | Action::FocusDown | Action::FocusLeft | Action::FocusTop
            | Action::ClosePane => "Panes",
            Action::PrevPrompt | Action::NextPrompt => "Navigation",
            Action::Copy | Action::Paste => "Clipboard",
            Action::IncreaseFontSize | Action::DecreaseFontSize | Action::ResetFontSize
            | Action::ClearScrollback | Action::ToggleFullscreen | Action::ToggleTabSidebar
            | Action::ToggleAiSidebar => "View",
            Action::OpenSearch | Action::GlobalSearch | Action::TabOverview
            | Action::CommandPalette => "Search",
            Action::SshManager | Action::ProjectJumper | Action::WorktreePicker
            | Action::InsertFilePath => "Tools",
            Action::NewWindow | Action::OpenSettings | Action::ReloadConfig | Action::Quit => {
                "Application"
            }
        }
    }

    pub fn categories() -> Vec<&'static str> {
        vec![
            "Tabs",
            "Panes",
            "Navigation",
            "Clipboard",
            "View",
            "Search",
            "Tools",
            "Application",
        ]
    }
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
            insert("super+equal", Action::IncreaseFontSize);
            insert("super+minus", Action::DecreaseFontSize);
            insert("super+0", Action::ResetFontSize);
            insert("super+q", Action::Quit);
        } else {
            insert("ctrl+shift+c", Action::Copy);
            insert("ctrl+shift+v", Action::Paste);
            insert("ctrl+equal", Action::IncreaseFontSize);
            insert("ctrl+minus", Action::DecreaseFontSize);
            insert("ctrl+0", Action::ResetFontSize);
            insert("ctrl+q", Action::Quit);
        }

        match preset {
            KeybindingPreset::Default => {
                if cfg!(target_os = "macos") {
                    insert("super+t", Action::NewTab);
                    insert("super+w", Action::ClosePane);
                    insert("super+shift+w", Action::CloseTab);
                    insert("super+b", Action::ToggleTabSidebar);
                    insert("super+l", Action::ToggleAiSidebar);
                    insert("super+n", Action::NewWindow);
                    insert("super+f", Action::OpenSearch);
                    insert("super+comma", Action::OpenSettings);
                    insert("super+r", Action::ReloadConfig);
                    insert("super+p", Action::CommandPalette);
                    insert("super+o", Action::SshManager);
                    insert("super+j", Action::ProjectJumper);
                    insert("super+alt+w", Action::WorktreePicker);
                    insert("super+k", Action::ClearScrollback);
                    insert("ctrl+super+f", Action::ToggleFullscreen);
                    insert("super+shift+]", Action::NextTab);
                    insert("super+shift+[", Action::PrevTab);
                    insert("super+shift+up", Action::PrevPrompt);
                    insert("super+shift+down", Action::NextPrompt);
                    for n in 1..=9u8 {
                        insert(&format!("super+{n}"), Action::SelectTab(n));
                    }
                    insert("super+d", Action::SplitRight);
                    insert("super+shift+d", Action::SplitDown);
                    insert("super+alt+left", Action::FocusLeft);
                    insert("super+alt+right", Action::FocusRight);
                    insert("super+alt+up", Action::FocusTop);
                    insert("super+alt+down", Action::FocusDown);
                    insert("super+shift+f", Action::GlobalSearch);
                    insert("super+shift+o", Action::TabOverview);
                    insert("ctrl+shift+comma", Action::InsertFilePath);
                } else {
                    insert("ctrl+shift+t", Action::NewTab);
                    insert("ctrl+shift+w", Action::ClosePane);
                    insert("ctrl+shift+q", Action::CloseTab);
                    insert("ctrl+shift+b", Action::ToggleTabSidebar);
                    insert("ctrl+shift+l", Action::ToggleAiSidebar);
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
                    insert("ctrl+shift+r", Action::ReloadConfig);
                    insert("ctrl+shift+p", Action::CommandPalette);
                    insert("ctrl+shift+o", Action::SshManager);
                    insert("ctrl+shift+j", Action::ProjectJumper);
                    insert("ctrl+alt+w", Action::WorktreePicker);
                    insert("ctrl+shift+k", Action::ClearScrollback);
                    insert("f11", Action::ToggleFullscreen);
                    insert("ctrl+tab", Action::NextTab);
                    insert("ctrl+shift+tab", Action::PrevTab);
                    insert("ctrl+shift+up", Action::PrevPrompt);
                    insert("ctrl+shift+down", Action::NextPrompt);
                    insert("ctrl+shift+comma", Action::InsertFilePath);
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
                    insert("super+comma", Action::OpenSettings);
                    insert("super+shift+p", Action::CommandPalette);
                    insert("super+k", Action::ClearScrollback);
                    insert("super+f", Action::OpenSearch);
                    insert("super+shift+f", Action::GlobalSearch);
                    insert("super+shift+o", Action::TabOverview);
                    insert("ctrl+super+f", Action::ToggleFullscreen);
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
            if is_unbound_marker(&action_str) {
                self.bindings.remove(&combo);
                continue;
            }
            let Some(action) = parse_action(&action_str) else {
                continue;
            };
            self.bindings.insert(combo, action);
        }
    }

    pub fn resolve(&self, combo: &KeyCombo) -> Option<Action> {
        self.bindings.get(combo).copied()
    }

    /// All combos currently bound to `action`, sorted for stable display.
    pub fn combos_for(&self, action: Action) -> Vec<KeyCombo> {
        let mut v: Vec<KeyCombo> = self
            .bindings
            .iter()
            .filter(|(_, a)| **a == action)
            .map(|(c, _)| *c)
            .collect();
        v.sort_by_key(|c| c.to_string());
        v
    }

    /// Preset combos for `action` before user overrides.
    pub fn preset_combos(preset: KeybindingPreset, action: Action) -> Vec<KeyCombo> {
        Self::for_preset(preset).combos_for(action)
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
    Some(normalize_combo(combo))
}

/// US-layout shifted punctuation folded back to its base key.
///
/// macOS GPUI folds Shift into the key for non-letter characters: pressing
/// shift+comma reports the key as `<` with `shift = false`. Normalizing both
/// event combos and config combos through this table keeps `ctrl+shift+,`
/// matching the `<` keystroke, and makes `super+shift+[` work too.
const SHIFTED_TO_BASE: &[(char, char)] = &[
    ('~', '`'), ('!', '1'), ('@', '2'), ('#', '3'), ('$', '4'), ('%', '5'),
    ('^', '6'), ('&', '7'), ('*', '8'), ('(', '9'), (')', '0'),
    ('_', '-'), ('+', '='), ('{', '['), ('}', ']'), ('|', '\\'),
    (':', ';'), ('"', '\''), ('<', ','), ('>', '.'), ('?', '/'),
];

/// Canonical form of a combo: uppercase and shifted punctuation fold into
/// the base key with `shift` set.
pub fn normalize_combo(mut combo: KeyCombo) -> KeyCombo {
    if let NamedKey::Char(c) = combo.key {
        if c.is_ascii_uppercase() {
            combo.key = NamedKey::Char(c.to_ascii_lowercase());
            combo.shift = true;
        } else if let Some(&(_, base)) = SHIFTED_TO_BASE.iter().find(|(s, _)| *s == c) {
            combo.key = NamedKey::Char(base);
            combo.shift = true;
        }
    }
    combo
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
        "plus" => Some(NamedKey::Plus),
        "minus" => Some(NamedKey::Minus),
        "equal" | "equals" => Some(NamedKey::Equal),
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
        "toggle_ai_sidebar" => Some(Action::ToggleAiSidebar),
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
        "insert_file_path" | "file_picker" => Some(Action::InsertFilePath),
        "quit" => Some(Action::Quit),
        _ => None,
    }
}

/// Merged preset + user bindings, as Settings and the key handler see them.
pub fn effective_resolver(
    preset: KeybindingPreset,
    user: &HashMap<String, String>,
) -> KeyBindingResolver {
    let mut r = KeyBindingResolver::for_preset(preset);
    r.apply_user(user.clone());
    r
}

/// Drop every user entry that affects `action`: overrides pointing at it,
/// unbind markers on its preset combos, and entries that steal one of its
/// preset combos for another action. Preset defaults resurface.
pub fn reset_action_bindings(
    user: &mut HashMap<String, String>,
    preset: KeybindingPreset,
    action: Action,
) {
    let preset_combos = KeyBindingResolver::preset_combos(preset, action);
    user.retain(|combo_str, action_str| {
        if let Some(a) = parse_action(action_str) {
            if a == action {
                return false;
            }
        }
        if let Some(c) = parse_combo(combo_str) {
            if preset_combos.contains(&c) {
                return false;
            }
        }
        true
    });
}

/// True when the user map changes anything for `action` vs the preset:
/// an override pointing at it (aliases included), a marker on its preset
/// combos, or a stolen preset combo.
pub fn is_action_customized(
    user: &HashMap<String, String>,
    preset: KeybindingPreset,
    action: Action,
) -> bool {
    let preset_combos = KeyBindingResolver::preset_combos(preset, action);
    user.iter().any(|(k, v)| {
        parse_action(v).is_some_and(|a| a == action)
            || parse_combo(k).is_some_and(|c| preset_combos.contains(&c))
    })
}

fn format_key(key: &NamedKey) -> String {
    if cfg!(target_os = "macos") {
        match key {
            NamedKey::Char(c) => c.to_uppercase().to_string(),
            NamedKey::Up => "↑".to_string(),
            NamedKey::Down => "↓".to_string(),
            NamedKey::Left => "←".to_string(),
            NamedKey::Right => "→".to_string(),
            NamedKey::Return => "↵".to_string(),
            NamedKey::Tab => "⇥".to_string(),
            NamedKey::Escape => "⎋".to_string(),
            NamedKey::Backspace => "⌫".to_string(),
            NamedKey::Delete => "⌦".to_string(),
            NamedKey::Space => "Space".to_string(),
            NamedKey::Plus => "+".to_string(),
            NamedKey::Minus => "-".to_string(),
            NamedKey::Equal => "=".to_string(),
            _ => format!("{key:?}"),
        }
    } else {
        match key {
            NamedKey::Char(c) => c.to_uppercase().to_string(),
            NamedKey::Space => "Space".to_string(),
            NamedKey::Plus => "+".to_string(),
            NamedKey::Minus => "-".to_string(),
            NamedKey::Equal => "=".to_string(),
            _ => format!("{key:?}"),
        }
    }
}

/// OS-aware combo label: `⌘⇧D` on macOS, `Ctrl+Shift+D` elsewhere.
/// `super` shows as `⌘` / `Super`.
pub fn format_combo(combo: &KeyCombo) -> String {
    if cfg!(target_os = "macos") {
        let mut s = String::new();
        if combo.ctrl {
            s.push('⌃');
        }
        if combo.alt {
            s.push('⌥');
        }
        if combo.shift {
            s.push('⇧');
        }
        if combo.logo {
            s.push('⌘');
        }
        s.push_str(&format_key(&combo.key));
        s
    } else {
        let mut parts = Vec::new();
        if combo.ctrl {
            parts.push("Ctrl".to_string());
        }
        if combo.shift {
            parts.push("Shift".to_string());
        }
        if combo.alt {
            parts.push("Alt".to_string());
        }
        if combo.logo {
            parts.push("Super".to_string());
        }
        parts.push(format_key(&combo.key));
        parts.join("+")
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
        // Key names some platform backends report instead of the character
        // (xkb on Linux uses "comma", not ",").
        "comma" => NamedKey::Char(','),
        "period" | "dot" => NamedKey::Char('.'),
        "semicolon" => NamedKey::Char(';'),
        "slash" => NamedKey::Char('/'),
        s if s.chars().count() == 1 => NamedKey::Char(s.chars().next()?),
        _ => return None,
    };
    Some(normalize_combo(KeyCombo { ctrl, shift, alt, logo, key }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binding_id_roundtrip() {
        for a in Action::all() {
            let id = a.binding_id();
            assert_eq!(parse_action(&id), Some(a), "roundtrip failed for {id}");
        }
    }

    #[test]
    fn test_unbind_marker_removes_preset() {
        let mut r = KeyBindingResolver::for_preset(KeybindingPreset::Default);
        let combo = parse_combo("super+t").unwrap();
        assert_eq!(r.resolve(&combo), Some(Action::NewTab));
        let mut user = HashMap::new();
        user.insert("super+t".to_string(), "none".to_string());
        r.apply_user(user);
        assert_eq!(r.resolve(&combo), None);
    }

    #[test]
    fn test_reset_action_restores_preset() {
        let mut user = HashMap::new();
        user.insert("ctrl+t".to_string(), "new_tab".to_string());
        user.insert("super+t".to_string(), "none".to_string());
        reset_action_bindings(&mut user, KeybindingPreset::Default, Action::NewTab);
        assert!(user.is_empty());
        let r = effective_resolver(KeybindingPreset::Default, &user);
        assert!(!r.combos_for(Action::NewTab).is_empty());
    }

    #[test]
    fn test_is_action_customized() {
        let user = HashMap::new();
        assert!(!is_action_customized(
            &user,
            KeybindingPreset::Default,
            Action::NewTab
        ));
        let mut user = HashMap::new();
        user.insert("ctrl+t".to_string(), "new_tab".to_string());
        assert!(is_action_customized(
            &user,
            KeybindingPreset::Default,
            Action::NewTab
        ));
        assert!(!is_action_customized(
            &user,
            KeybindingPreset::Default,
            Action::CloseTab
        ));
    }

    #[test]
    fn test_combo_string_roundtrip() {
        for s in [
            "super+t",
            "ctrl+shift+plus",
            "super+minus",
            "ctrl+equal",
            "alt+left",
            "super+shift+[",
            "f11",
            "ctrl+tab",
            "super+comma",
        ] {
            let combo = parse_combo(s).expect(s);
            assert_eq!(parse_combo(&combo.to_string()), Some(combo), "{s}");
        }
    }

    #[test]
    fn test_shifted_punctuation_normalizes_like_macos_gpui() {
        // macOS GPUI folds Shift into non-letter keys: ctrl+shift+comma
        // arrives as key "<" with shift = false.
        let event = combo_from_key("<", true, false, false, false);
        assert_eq!(event, parse_combo("ctrl+shift+,"));
        // The same fold applies to super+shift+] (tab switching).
        let brace = combo_from_key("}", false, false, false, true);
        assert_eq!(brace, parse_combo("super+shift+]"));
        // Base keys with an explicit shift modifier stay unchanged.
        assert_eq!(
            parse_combo("ctrl+shift+,"),
            Some(KeyCombo { ctrl: true, shift: true, alt: false, logo: false, key: NamedKey::Char(',') })
        );
    }

    #[test]
    fn test_alt_comma_works_on_any_layout() {
        // Alt never changes the reported character, so ctrl+alt+comma
        // resolves regardless of keyboard layout.
        let event = combo_from_key(",", true, false, true, false);
        assert_eq!(event, parse_combo("ctrl+alt+,"));
    }

    #[test]
    fn test_key_word_aliases() {
        assert_eq!(
            combo_from_key("comma", true, false, true, false),
            parse_combo("ctrl+alt+,")
        );
    }

    #[test]
    fn test_reset_action_reclaims_stolen_combo() {
        let mut user = HashMap::new();
        user.insert("super+t".to_string(), "copy".to_string());
        assert!(is_action_customized(
            &user,
            KeybindingPreset::Default,
            Action::NewTab
        ));
        reset_action_bindings(&mut user, KeybindingPreset::Default, Action::NewTab);
        assert!(user.is_empty());
        let r = effective_resolver(KeybindingPreset::Default, &user);
        assert!(r
            .combos_for(Action::NewTab)
            .contains(&parse_combo("super+t").unwrap()));
    }

    #[test]
    fn test_preset_resolvers() {
        let ghostty = KeyBindingResolver::for_preset(KeybindingPreset::Ghostty);
        assert!(!ghostty.bindings.is_empty());

        let tmux = KeyBindingResolver::for_preset(KeybindingPreset::Tmux);
        assert!(!tmux.bindings.is_empty());

        let iterm = KeyBindingResolver::for_preset(KeybindingPreset::ITerm2);
        assert!(!iterm.bindings.is_empty());
    }

    #[test]
    fn test_presets_have_at_most_one_combo_per_action() {
        for preset in [
            KeybindingPreset::Default,
            KeybindingPreset::Ghostty,
            KeybindingPreset::Tmux,
            KeybindingPreset::ITerm2,
        ] {
            let resolver = KeyBindingResolver::for_preset(preset);
            for action in Action::all() {
                let combos = resolver.combos_for(action);
                assert!(
                    combos.len() <= 1,
                    "Preset {preset} has {} combos for action {action:?}: {combos:?}",
                    combos.len()
                );
            }
        }
    }

    #[test]
    fn test_file_picker_has_only_ctrl_shift_comma() {
        let resolver = KeyBindingResolver::for_preset(KeybindingPreset::Default);
        let combos = resolver.combos_for(Action::InsertFilePath);
        assert_eq!(combos.len(), 1);
        assert_eq!(combos[0], parse_combo("ctrl+shift+,").unwrap());
    }
}
