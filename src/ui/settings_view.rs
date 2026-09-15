use gpui::{
    canvas, Context, CursorStyle, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Render, ScrollHandle, ScrollStrategy, SharedString, UniformListScrollHandle, Window,
    WindowControlArea, WindowHandle, div, prelude::*, px, uniform_list,
};
use icons::common::IconType;
use parking_lot::Mutex;
use crate::config::{self, Config};
use crate::ui::icons::{render_app_logo, render_icon};
use crate::ui::root_view::{available_system_fonts, open_path_or_url};
use crate::ui::theme::Theme;

pub static SETTINGS_WINDOW_HANDLE: Mutex<Option<WindowHandle<SettingsView>>> = Mutex::new(None);

#[derive(Clone)]
pub struct ThemeCardInfo {
    pub name: String,
    pub label: String,
    pub bg_color: gpui::Hsla,
    pub surf_color: gpui::Hsla,
    pub acc_color: gpui::Hsla,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Appearance,
    Keyboard,
    Ai,
    Migration,
    Advanced,
}

impl SettingsTab {
    pub fn label(&self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Keyboard => "Keyboard",
            Self::Ai => "AI Assistant",
            Self::Migration => "Migration",
            Self::Advanced => "Advanced",
        }
    }

    pub fn icon(&self) -> IconType {
        match self {
            Self::General => IconType::Settings,
            Self::Appearance => IconType::Palette,
            Self::Keyboard => IconType::Layers,
            Self::Ai => IconType::Sparkles,
            Self::Migration => IconType::FolderOpen,
            Self::Advanced => IconType::FileCode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveInputField {
    Search,
    AiBaseUrl,
    AiApiKey,
    AiModel,
    FontSearch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiTestStatus {
    Idle,
    Testing,
    Success(std::time::Duration),
    Failed(String),
}

pub struct SettingsView {
    pub config: Config,
    pub theme: Theme,
    pub current_theme_name: String,
    pub font_size: f32,
    pub font_family: String,
    pub opacity: f32,
    pub cursor_blink: bool,
    pub copy_on_select: bool,
    pub tab_layout: crate::config::TabLayout,
    pub scrollback: usize,
    pub keybinding_preset: crate::keybindings::KeybindingPreset,
    pub ai_permission_mode: crate::ai::PermissionMode,
    pub ai_context_window: u32,
    pub ai_provider: String,
    pub ai_model: Option<String>,
    pub detected_external_configs: Vec<crate::importer::DetectedConfig>,
    pub import_status_message: Option<String>,
    pub focus_handle: gpui::FocusHandle,
    pub window_scroll_handle: ScrollHandle,
    pub system_fonts: Vec<String>,
    pub theme_cards: Vec<ThemeCardInfo>,
    pub active_tab: SettingsTab,
    pub search_query: String,
    pub search_input: crate::ui::TextInputState,
    pub active_input_field: Option<ActiveInputField>,
    pub font_combobox_open: bool,
    pub font_search_query: String,
    pub font_search_state: crate::ui::TextInputState,
    pub font_highlighted_index: usize,
    pub font_scroll_handle: UniformListScrollHandle,
    pub font_trigger_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    pub font_search_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    pub ai_base_url_input: String,
    pub ai_base_url_state: crate::ui::TextInputState,
    pub ai_api_key_input: String,
    pub ai_api_key_state: crate::ui::TextInputState,
    pub ai_model_input: String,
    pub ai_model_state: crate::ui::TextInputState,
    pub detected_ollama_models: Vec<String>,
    pub ai_test_status: AiTestStatus,
    pub search_input_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    pub ai_model_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    pub ai_base_url_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    pub ai_api_key_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    pub is_dragging_input: bool,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        config::load_custom_themes();
        let loaded_config = crate::config::load_lenient();
        let theme_name = loaded_config.theme.as_deref().unwrap_or("default").to_string();
        let theme = Theme::from_name(&theme_name).with_opacity(loaded_config.opacity);
        let tab_layout = loaded_config.tab_layout;
        let scrollback = loaded_config.scrollback;
        let keybinding_preset = loaded_config.keybinding_preset.unwrap_or_default();
        let detected_external_configs = crate::importer::detect_all_external_configs();

        let font_size = loaded_config.font.size;
        let font_family = if loaded_config.font.family.is_empty() || loaded_config.font.family == "monospace" {
            #[cfg(target_os = "macos")]
            {
                "Menlo".to_string()
            }
            #[cfg(target_os = "windows")]
            {
                "Cascadia Code".to_string()
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                "monospace".to_string()
            }
        } else {
            loaded_config.font.family.clone()
        };

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let mut raw_fonts = cx.text_system().all_font_names();
        for f in available_system_fonts() {
            if !raw_fonts.iter().any(|existing| existing.eq_ignore_ascii_case(&f)) {
                raw_fonts.push(f);
            }
        }

        let mut system_fonts: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if !font_family.is_empty() {
            seen.insert(font_family.to_lowercase());
            system_fonts.push(font_family.clone());
        }

        let mut other_fonts: Vec<String> = Vec::new();
        for font in raw_fonts {
            let trimmed = font.trim().to_string();
            let lower = trimmed.to_lowercase();
            if !trimmed.is_empty() && !trimmed.starts_with('.') && !seen.contains(&lower) {
                seen.insert(lower);
                other_fonts.push(trimmed);
            }
        }
        other_fonts.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        system_fonts.extend(other_fonts);

        let theme_cards: Vec<ThemeCardInfo> = crate::config::all_theme_names()
            .into_iter()
            .map(|name| {
                let t = Theme::from_name(&name);
                let label = match name.as_str() {
                    "default" => "Fastty Default".to_string(),
                    "catppuccin" => "Catppuccin Mocha".to_string(),
                    "one-dark" => "One Dark Pro".to_string(),
                    "solarized-dark" => "Solarized Dark".to_string(),
                    "high-contrast" => "High Contrast".to_string(),
                    other => other.to_string(),
                };
                ThemeCardInfo {
                    name,
                    label,
                    bg_color: t.background,
                    surf_color: t.surface_raised,
                    acc_color: t.accent,
                }
            })
            .collect();

        let ai_base_url_input = loaded_config
            .ai
            .providers
            .get(&loaded_config.ai.default)
            .map(|p| p.base_url().to_string())
            .unwrap_or_else(|| "http://localhost:11434/v1".to_string());
        let ai_api_key_input = loaded_config
            .ai
            .providers
            .get(&loaded_config.ai.default)
            .and_then(|p| p.api_key().map(|s| s.to_string()))
            .unwrap_or_default();

        let detected_ollama_models = if let Some(prov) = loaded_config.ai.providers.get("ollama") {
            crate::ai::detect_installed_ollama_models(Some(prov.base_url()))
        } else {
            crate::ai::detect_installed_ollama_models(None)
        };

        let mut ai_model = loaded_config.ai.default_model.clone();
        if loaded_config.ai.default == "ollama" && ai_model.is_none() && !detected_ollama_models.is_empty() {
            ai_model = detected_ollama_models.first().cloned();
        } else if ai_model.is_none() {
            if let Some(prov) = loaded_config.ai.providers.get(&loaded_config.ai.default) {
                ai_model = prov.models().first().cloned();
            }
        }
        let ai_model_input = ai_model.clone().unwrap_or_default();

        Self {
            opacity: loaded_config.opacity,
            cursor_blink: loaded_config.cursor.blink,
            copy_on_select: loaded_config.copy_on_select,
            tab_layout,
            scrollback,
            keybinding_preset,
            ai_permission_mode: loaded_config.ai.permission_mode,
            ai_context_window: loaded_config.ai.context_window,
            ai_provider: loaded_config.ai.default.clone(),
            ai_model,
            detected_external_configs,
            import_status_message: None,
            config: loaded_config,
            theme,
            current_theme_name: theme_name,
            font_size,
            font_family,
            focus_handle,
            window_scroll_handle: ScrollHandle::new(),
            system_fonts,
            theme_cards,
            active_tab: SettingsTab::General,
            search_query: String::new(),
            search_input: crate::ui::TextInputState::default(),
            active_input_field: None,
            font_combobox_open: false,
            font_search_query: String::new(),
            font_search_state: crate::ui::TextInputState::default(),
            font_highlighted_index: 0,
            font_scroll_handle: UniformListScrollHandle::new(),
            font_trigger_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            font_search_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_base_url_state: crate::ui::TextInputState::new(ai_base_url_input.clone()),
            ai_base_url_input,
            ai_api_key_state: crate::ui::TextInputState::new(ai_api_key_input.clone()),
            ai_api_key_input,
            ai_model_state: crate::ui::TextInputState::new(ai_model_input.clone()),
            ai_model_input,
            detected_ollama_models,
            ai_test_status: AiTestStatus::Idle,
            search_input_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_model_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_base_url_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_api_key_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            is_dragging_input: false,
        }
    }

    pub fn filtered_font_indices(&self) -> Vec<usize> {
        let q = self.font_search_query.trim().to_lowercase();
        if q.is_empty() {
            (0..self.system_fonts.len()).collect()
        } else {
            self.system_fonts
                .iter()
                .enumerate()
                .filter(|(_, name)| name.to_lowercase().contains(&q))
                .map(|(idx, _)| idx)
                .collect()
        }
    }

    pub fn toggle_font_combobox(&mut self, cx: &mut Context<Self>) {
        self.font_combobox_open = !self.font_combobox_open;
        if self.font_combobox_open {
            self.active_input_field = Some(ActiveInputField::FontSearch);
            self.font_search_state.clear();
            self.font_search_query.clear();
            let indices = self.filtered_font_indices();
            if let Some(pos) = indices.iter().position(|&idx| self.system_fonts.get(idx) == Some(&self.font_family)) {
                self.font_highlighted_index = pos;
                self.font_scroll_handle.scroll_to_item(pos, ScrollStrategy::Center);
            } else {
                self.font_highlighted_index = 0;
            }
        } else if self.active_input_field == Some(ActiveInputField::FontSearch) {
            self.active_input_field = None;
        }
        cx.notify();
    }

    pub fn new_with_config(window: &mut Window, active_config: &Config, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new(window, cx);
        view.sync_from_config(active_config, cx);
        view
    }

    pub fn sync_from_config(&mut self, cfg: &Config, cx: &mut Context<Self>) {
        self.config = cfg.clone();
        self.ai_test_status = AiTestStatus::Idle;
        self.opacity = cfg.opacity;
        self.scrollback = cfg.scrollback;
        self.cursor_blink = cfg.cursor.blink;
        self.copy_on_select = cfg.copy_on_select;
        self.tab_layout = cfg.tab_layout;
        self.font_size = cfg.font.size;
        if !cfg.font.family.is_empty() && cfg.font.family != "monospace" {
            self.font_family = cfg.font.family.clone();
        }
        self.keybinding_preset = cfg.keybinding_preset.unwrap_or_default();
        self.ai_permission_mode = cfg.ai.permission_mode;
        self.ai_context_window = cfg.ai.context_window;
        self.ai_provider = cfg.ai.default.clone();
        self.ai_model = cfg.ai.default_model.clone();
        if let Some(ref t) = cfg.theme {
            self.current_theme_name = t.clone();
        }
        self.theme = Theme::from_name(&self.current_theme_name).with_opacity(self.opacity);

        if let Some(prov) = cfg.ai.providers.get(&self.ai_provider) {
            self.ai_base_url_input = prov.base_url().to_string();
            self.ai_api_key_input = prov.api_key().unwrap_or("").to_string();
        }

        self.ai_model_input = self.ai_model.clone().unwrap_or_else(|| {
            cfg.ai.providers
                .get(&self.ai_provider)
                .and_then(|p| p.models().first().cloned())
                .unwrap_or_default()
        });

        self.ai_base_url_state = crate::ui::TextInputState::new(self.ai_base_url_input.clone());
        self.ai_api_key_state = crate::ui::TextInputState::new(self.ai_api_key_input.clone());
        self.ai_model_state = crate::ui::TextInputState::new(self.ai_model_input.clone());

        cx.notify();
    }

    fn apply_search_filter(&mut self, cx: &mut Context<Self>) {
        let q = self.search_query.to_lowercase();
        if q.contains("ai")
            || q.contains("model")
            || q.contains("llm")
            || q.contains("prov")
            || q.contains("token")
            || q.contains("key")
            || q.contains("url")
            || q.contains("endpoint")
        {
            self.active_tab = SettingsTab::Ai;
        } else if q.contains("theme")
            || q.contains("color")
            || q.contains("font")
            || q.contains("size")
            || q.contains("opac")
            || q.contains("appear")
        {
            self.active_tab = SettingsTab::Appearance;
        } else if q.contains("key")
            || q.contains("short")
            || q.contains("bind")
            || q.contains("tmux")
            || q.contains("ghost")
        {
            self.active_tab = SettingsTab::Keyboard;
        } else if q.contains("tab")
            || q.contains("cursor")
            || q.contains("blink")
            || q.contains("scroll")
            || q.contains("copy")
            || q.contains("gen")
        {
            self.active_tab = SettingsTab::General;
        } else if q.contains("migr")
            || q.contains("warp")
            || q.contains("iterm")
            || q.contains("alacr")
        {
            self.active_tab = SettingsTab::Migration;
        } else if q.contains("file")
            || q.contains("toml")
            || q.contains("fold")
            || q.contains("adv")
        {
            self.active_tab = SettingsTab::Advanced;
        }
        cx.notify();
    }

    fn save_ai_provider_config(&mut self) {
        self.config.ai.default = self.ai_provider.clone();
        if let Some(prov) = self.config.ai.providers.get_mut(&self.ai_provider) {
            prov.set_base_url(self.ai_base_url_input.clone());
            let key = if self.ai_api_key_input.trim().is_empty() {
                None
            } else {
                Some(self.ai_api_key_input.trim().to_string())
            };
            prov.set_api_key(key);
        }
        if !self.ai_model_input.trim().is_empty() {
            self.config.ai.default_model = Some(self.ai_model_input.trim().to_string());
            self.ai_model = Some(self.ai_model_input.trim().to_string());
        } else {
            self.config.ai.default_model = None;
            self.ai_model = None;
        }
        self.ai_test_status = AiTestStatus::Idle;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
    }

    fn sync_active_field_text(&mut self, field: ActiveInputField, cx: &mut Context<Self>) {
        match field {
            ActiveInputField::Search => {
                self.search_query = self.search_input.text.clone();
                self.apply_search_filter(cx);
            }
            ActiveInputField::AiBaseUrl => {
                self.ai_base_url_input = self.ai_base_url_state.text.clone();
                self.save_ai_provider_config();
                cx.notify();
            }
            ActiveInputField::AiApiKey => {
                self.ai_api_key_input = self.ai_api_key_state.text.clone();
                self.save_ai_provider_config();
                cx.notify();
            }
            ActiveInputField::AiModel => {
                self.ai_model_input = self.ai_model_state.text.clone();
                self.save_ai_provider_config();
                cx.notify();
            }
            ActiveInputField::FontSearch => {
                self.font_search_query = self.font_search_state.text.clone();
                self.font_highlighted_index = 0;
                cx.notify();
            }
        }
    }

    fn handle_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            if self.font_combobox_open {
                self.font_combobox_open = false;
                if self.active_input_field == Some(ActiveInputField::FontSearch) {
                    self.active_input_field = None;
                }
                cx.notify();
                return;
            }
            if self.active_input_field.is_some() {
                if self.active_input_field == Some(ActiveInputField::Search) && !self.search_query.is_empty() {
                    self.search_query.clear();
                    self.search_input.clear();
                } else {
                    self.active_input_field = None;
                }
                cx.notify();
                return;
            }
            *SETTINGS_WINDOW_HANDLE.lock() = None;
            window.remove_window();
            return;
        }

        // Text input handling for active field
        if let Some(field) = self.active_input_field {
            let key = ev.keystroke.key.as_str();
            let key_lower = key.to_lowercase();
            let is_ctrl = ev.keystroke.modifiers.control;
            let is_plat = ev.keystroke.modifiers.platform;
            let shift = ev.keystroke.modifiers.shift;

            if field == ActiveInputField::FontSearch {
                if key_lower == "down" || key_lower == "arrowdown" {
                    let indices = self.filtered_font_indices();
                    if !indices.is_empty() {
                        self.font_highlighted_index = (self.font_highlighted_index + 1) % indices.len();
                        self.font_scroll_handle.scroll_to_item(self.font_highlighted_index, ScrollStrategy::Nearest);
                        cx.notify();
                    }
                    return;
                }
                if key_lower == "up" || key_lower == "arrowup" {
                    let indices = self.filtered_font_indices();
                    if !indices.is_empty() {
                        self.font_highlighted_index = if self.font_highlighted_index == 0 {
                            indices.len() - 1
                        } else {
                            self.font_highlighted_index - 1
                        };
                        self.font_scroll_handle.scroll_to_item(self.font_highlighted_index, ScrollStrategy::Nearest);
                        cx.notify();
                    }
                    return;
                }
                if key_lower == "enter" || key_lower == "return" {
                    let indices = self.filtered_font_indices();
                    if let Some(&font_idx) = indices.get(self.font_highlighted_index) {
                        if let Some(name) = self.system_fonts.get(font_idx).cloned() {
                            self.set_font_family(&name, cx);
                        }
                    }
                    self.font_combobox_open = false;
                    self.active_input_field = None;
                    cx.notify();
                    return;
                }
            }

            // Select All: Cmd+A (macOS) or Ctrl+A (Linux/Windows)
            let is_select_all = (cfg!(target_os = "macos") && is_plat && key_lower == "a")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "a");
            if is_select_all {
                let state = match field {
                    ActiveInputField::Search => &mut self.search_input,
                    ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                    ActiveInputField::AiModel => &mut self.ai_model_state,
                    ActiveInputField::FontSearch => &mut self.font_search_state,
                };
                state.select_all();
                cx.notify();
                return;
            }

            // Copy: Cmd+C (macOS) or Ctrl+C (Linux/Windows)
            let is_copy = (cfg!(target_os = "macos") && is_plat && key_lower == "c")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "c");
            if is_copy {
                let state = match field {
                    ActiveInputField::Search => &self.search_input,
                    ActiveInputField::AiBaseUrl => &self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &self.ai_api_key_state,
                    ActiveInputField::AiModel => &self.ai_model_state,
                    ActiveInputField::FontSearch => &self.font_search_state,
                };
                if let Some(sel) = state.selected_text() {
                    if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                        let _ = clip.set_text(sel);
                    }
                }
                return;
            }

            // Cut: Cmd+X (macOS) or Ctrl+X (Linux/Windows)
            let is_cut = (cfg!(target_os = "macos") && is_plat && key_lower == "x")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "x");
            if is_cut {
                let state = match field {
                    ActiveInputField::Search => &mut self.search_input,
                    ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                    ActiveInputField::AiModel => &mut self.ai_model_state,
                    ActiveInputField::FontSearch => &mut self.font_search_state,
                };
                if let Some(sel) = state.selected_text() {
                    if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                        let _ = clip.set_text(sel);
                    }
                    state.delete_selection();
                    self.sync_active_field_text(field, cx);
                }
                return;
            }

            // Paste: Cmd+V (macOS) or Ctrl+V (Linux/Windows)
            let is_paste = (cfg!(target_os = "macos") && is_plat && key_lower == "v")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "v");
            if is_paste {
                if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                    if let Ok(text) = clip.get_text() {
                        let clean = text.trim();
                        let state = match field {
                            ActiveInputField::Search => &mut self.search_input,
                            ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                            ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                            ActiveInputField::AiModel => &mut self.ai_model_state,
                            ActiveInputField::FontSearch => &mut self.font_search_state,
                        };
                        state.insert_str(clean);
                        self.sync_active_field_text(field, cx);
                        return;
                    }
                }
            }

            if key_lower == "enter" || key_lower == "return" {
                self.active_input_field = None;
                cx.notify();
                return;
            }

            if key_lower == "backspace" {
                let state = match field {
                    ActiveInputField::Search => &mut self.search_input,
                    ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                    ActiveInputField::AiModel => &mut self.ai_model_state,
                    ActiveInputField::FontSearch => &mut self.font_search_state,
                };
                state.backspace();
                self.sync_active_field_text(field, cx);
                return;
            }

            if key_lower == "delete" {
                let state = match field {
                    ActiveInputField::Search => &mut self.search_input,
                    ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                    ActiveInputField::AiModel => &mut self.ai_model_state,
                    ActiveInputField::FontSearch => &mut self.font_search_state,
                };
                state.delete_forward();
                self.sync_active_field_text(field, cx);
                return;
            }

            if key_lower == "left" || key_lower == "arrowleft" {
                let state = match field {
                    ActiveInputField::Search => &mut self.search_input,
                    ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                    ActiveInputField::AiModel => &mut self.ai_model_state,
                    ActiveInputField::FontSearch => &mut self.font_search_state,
                };
                state.move_left(shift);
                cx.notify();
                return;
            }

            if key_lower == "right" || key_lower == "arrowright" {
                let state = match field {
                    ActiveInputField::Search => &mut self.search_input,
                    ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                    ActiveInputField::AiModel => &mut self.ai_model_state,
                    ActiveInputField::FontSearch => &mut self.font_search_state,
                };
                state.move_right(shift);
                cx.notify();
                return;
            }

            if key_lower == "home" {
                let state = match field {
                    ActiveInputField::Search => &mut self.search_input,
                    ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                    ActiveInputField::AiModel => &mut self.ai_model_state,
                    ActiveInputField::FontSearch => &mut self.font_search_state,
                };
                state.move_home(shift);
                cx.notify();
                return;
            }

            if key_lower == "end" {
                let state = match field {
                    ActiveInputField::Search => &mut self.search_input,
                    ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                    ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                    ActiveInputField::AiModel => &mut self.ai_model_state,
                    ActiveInputField::FontSearch => &mut self.font_search_state,
                };
                state.move_end(shift);
                cx.notify();
                return;
            }

            if !is_plat && !is_ctrl {
                let text = ev.keystroke.key_char.as_deref().or_else(|| {
                    if key.chars().count() == 1 {
                        Some(key)
                    } else {
                        None
                    }
                });
                if let Some(txt) = text {
                    if !txt.chars().any(|c| c.is_control()) {
                        let state = match field {
                            ActiveInputField::Search => &mut self.search_input,
                            ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
                            ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
                            ActiveInputField::AiModel => &mut self.ai_model_state,
                            ActiveInputField::FontSearch => &mut self.font_search_state,
                        };
                        state.insert_str(txt);
                        self.sync_active_field_text(field, cx);
                        return;
                    }
                }
            }
        }

        // Fast tab switching shortcuts: Cmd+1..6
        if ev.keystroke.modifiers.platform {
            match ev.keystroke.key.as_str() {
                "1" => {
                    self.active_tab = SettingsTab::General;
                    self.active_input_field = None;
                    cx.notify();
                }
                "2" => {
                    self.active_tab = SettingsTab::Appearance;
                    self.active_input_field = None;
                    cx.notify();
                }
                "3" => {
                    self.active_tab = SettingsTab::Keyboard;
                    self.active_input_field = None;
                    cx.notify();
                }
                "4" => {
                    self.active_tab = SettingsTab::Ai;
                    self.active_input_field = None;
                    cx.notify();
                }
                "5" => {
                    self.active_tab = SettingsTab::Migration;
                    self.active_input_field = None;
                    cx.notify();
                }
                "6" => {
                    self.active_tab = SettingsTab::Advanced;
                    self.active_input_field = None;
                    cx.notify();
                }
                _ => {}
            }
        }
    }

    pub fn set_theme(&mut self, theme_name: &str, cx: &mut Context<Self>) {
        self.current_theme_name = theme_name.to_string();
        self.config.theme = Some(self.current_theme_name.clone());
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        self.theme = Theme::from_name(theme_name).with_opacity(self.config.opacity);
        cx.notify();
    }

    pub fn adjust_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.font_size = (self.font_size + delta).clamp(8.0, 36.0);
        self.config.font.size = self.font_size;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn set_font_family(&mut self, family: &str, cx: &mut Context<Self>) {
        self.font_family = family.to_string();
        self.config.font.family = family.to_string();
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn adjust_opacity(&mut self, opacity: f32, cx: &mut Context<Self>) {
        self.opacity = opacity.clamp(0.2, 1.0);
        self.config.opacity = self.opacity;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        self.theme = Theme::from_name(&self.current_theme_name).with_opacity(self.config.opacity);
        cx.notify();
    }

    pub fn toggle_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink = !self.cursor_blink;
        self.config.cursor.blink = self.cursor_blink;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn toggle_copy_on_select(&mut self, cx: &mut Context<Self>) {
        self.copy_on_select = !self.copy_on_select;
        self.config.copy_on_select = self.copy_on_select;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn set_tab_layout(&mut self, layout: crate::config::TabLayout, cx: &mut Context<Self>) {
        self.tab_layout = layout;
        self.config.tab_layout = layout;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn set_scrollback(&mut self, scrollback: usize, cx: &mut Context<Self>) {
        self.scrollback = scrollback;
        self.config.scrollback = scrollback;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn set_keybinding_preset(&mut self, preset: crate::keybindings::KeybindingPreset, cx: &mut Context<Self>) {
        self.keybinding_preset = preset;
        self.config.keybinding_preset = Some(preset);
        let _ = self.config.save_default();
        crate::keybindings::init_resolver(self.config.keybindings.clone(), Some(preset));
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn set_ai_permission_mode(&mut self, mode: crate::ai::PermissionMode, cx: &mut Context<Self>) {
        self.ai_permission_mode = mode;
        self.config.ai.permission_mode = mode;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn set_ai_context_window(&mut self, tokens: u32, cx: &mut Context<Self>) {
        let clamped = tokens.clamp(4_000, 2_000_000);
        self.ai_context_window = clamped;
        self.config.ai.context_window = clamped;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn set_ai_provider(&mut self, provider: &str, cx: &mut Context<Self>) {
        self.ai_provider = provider.to_string();
        self.config.ai.default = self.ai_provider.clone();
        if let Some(prov) = self.config.ai.providers.get(provider) {
            self.ai_base_url_input = prov.base_url().to_string();
            self.ai_base_url_state.set_text(self.ai_base_url_input.clone());
            self.ai_api_key_input = prov.api_key().unwrap_or("").to_string();
            self.ai_api_key_state.set_text(self.ai_api_key_input.clone());

            if provider == "ollama" {
                if self.detected_ollama_models.is_empty() {
                    self.detected_ollama_models = crate::ai::detect_installed_ollama_models(Some(&self.ai_base_url_input));
                }
                let chosen = self.detected_ollama_models.first().cloned().or_else(|| prov.models().first().cloned());
                self.ai_model = chosen.clone();
                self.ai_model_input = chosen.clone().unwrap_or_default();
                self.ai_model_state.set_text(self.ai_model_input.clone());
                self.config.ai.default_model = chosen;
            } else {
                let first_model = prov.models().first().cloned();
                self.ai_model = first_model.clone();
                self.ai_model_input = first_model.clone().unwrap_or_default();
                self.ai_model_state.set_text(self.ai_model_input.clone());
                self.config.ai.default_model = first_model;
            }
        }
        self.ai_test_status = AiTestStatus::Idle;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn reset_ai_base_url(&mut self, cx: &mut Context<Self>) {
        let default_url = match self.ai_provider.as_str() {
            "ollama" => "http://localhost:11434/v1",
            "anthropic" => "https://api.anthropic.com/v1",
            "openai" => "https://api.openai.com/v1",
            "openrouter" => "https://openrouter.ai/api/v1",
            "lm-studio" => "http://localhost:1234/v1",
            _ => "http://localhost:11434/v1",
        };
        self.ai_base_url_input = default_url.to_string();
        self.ai_base_url_state.set_text(self.ai_base_url_input.clone());
        self.save_ai_provider_config();
        cx.notify();
    }

    pub fn clear_ai_api_key(&mut self, cx: &mut Context<Self>) {
        self.ai_api_key_input.clear();
        self.ai_api_key_state.clear();
        self.save_ai_provider_config();
        cx.notify();
    }

    pub fn set_ai_model(&mut self, model: &str, cx: &mut Context<Self>) {
        let trimmed = model.trim().to_string();
        self.ai_model = Some(trimmed.clone());
        self.ai_model_input = trimmed.clone();
        self.ai_model_state.set_text(trimmed.clone());
        self.config.ai.default_model = Some(trimmed);
        self.config.ai.default = self.ai_provider.clone();
        self.ai_test_status = AiTestStatus::Idle;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn refresh_ollama_models(&mut self, cx: &mut Context<Self>) {
        self.detected_ollama_models = crate::ai::detect_installed_ollama_models(Some(&self.ai_base_url_input));
        if self.ai_provider == "ollama" {
            let first_opt = self.detected_ollama_models.first().cloned();
            let should_switch = self.ai_model_input.trim().is_empty()
                || !self.detected_ollama_models.contains(&self.ai_model_input);
            if let Some(first) = first_opt {
                if should_switch {
                    self.set_ai_model(&first, cx);
                    return;
                }
            }
        }
        cx.notify();
    }

    pub fn clear_ai_model(&mut self, cx: &mut Context<Self>) {
        self.ai_model_input.clear();
        self.ai_model_state.clear();
        self.ai_model = None;
        self.config.ai.default_model = None;
        self.ai_test_status = AiTestStatus::Idle;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        cx.notify();
    }

    pub fn test_ai_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_test_status == AiTestStatus::Testing {
            return;
        }
        self.ai_test_status = AiTestStatus::Testing;
        cx.notify();

        let cfg = self.config.ai.clone();
        let provider = self.ai_provider.clone();
        let model = self.ai_model_input.trim().to_string();

        let (tx, rx) = async_channel::bounded::<Result<std::time::Duration, String>>(1);

        std::thread::Builder::new()
            .name("fastty-ai-test".into())
            .spawn(move || {
                let res = crate::ai::test_provider_connection(&cfg, &provider, &model);
                let _ = tx.send_blocking(res);
            })
            .ok();

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(res) = rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    match res {
                        Ok(dur) => this.ai_test_status = AiTestStatus::Success(dur),
                        Err(e) => this.ai_test_status = AiTestStatus::Failed(e),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn import_external(&mut self, app: crate::importer::ExternalApp, path: std::path::PathBuf, cx: &mut Context<Self>) {
        match crate::importer::import_external_config(app, &path, &mut self.config) {
            Ok(msg) => {
                self.import_status_message = Some(msg);
                self.scrollback = self.config.scrollback;
                self.font_size = self.config.font.size;
                self.font_family = self.config.font.family.clone();
                self.opacity = self.config.opacity;
                if let Some(ref t) = self.config.theme {
                    self.current_theme_name = t.clone();
                    self.theme = Theme::from_name(t).with_opacity(self.config.opacity);
                }
                if let Some(p) = self.config.keybinding_preset {
                    self.keybinding_preset = p;
                }
                crate::config::increment_config_version();
                cx.notify();
            }
            Err(e) => {
                self.import_status_message = Some(format!("Error importing: {e}"));
                cx.notify();
            }
        }
    }

    pub fn open_settings_file() {
        let path = crate::config::Config::get_active_config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if !path.exists() {
            let _ = std::fs::write(&path, "# Fastty configuration\n");
        }
        open_path_or_url(&path);
    }

    pub fn open_config_folder() {
        let config_dir = dirs::home_dir().map(|h| h.join(".config/fastty")).unwrap_or_default();
        let _ = std::fs::create_dir_all(&config_dir);
        if let Some(path_str) = config_dir.to_str() {
            open_path_or_url(path_str);
        }
    }
}

fn render_toggle_switch(is_on: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(if is_on { "toggle-on" } else { "toggle-off" })
        .flex()
        .items_center()
        .w(px(38.))
        .h(px(22.))
        .rounded_full()
        .bg(if is_on { theme.accent } else { theme.surface_raised })
        .border_1()
        .border_color(if is_on { theme.accent } else { theme.border })
        .p(px(2.))
        .cursor(CursorStyle::PointingHand)
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .rounded_full()
                .bg(if is_on { theme.white } else { theme.bright_black })
                .when(is_on, |this| this.ml(px(16.))),
        )
}

fn render_section_header(title: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.muted_strong)
        .mb(px(8.))
        .child(title.into())
}

fn render_group_card(theme: &Theme) -> gpui::Div {
    div()
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .rounded(px(10.))
        .overflow_hidden()
        .flex()
        .flex_col()
}

fn render_card_row(
    title: impl Into<SharedString>,
    subtitle: Option<impl Into<SharedString>>,
    control: impl IntoElement,
    is_last: bool,
    theme: &Theme,
) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(16.))
        .py(px(12.))
        .when(!is_last, |this| this.border_b_1().border_color(theme.border))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .mr(px(12.))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child(title.into()),
                )
                .when_some(subtitle, |this, sub| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .child(sub.into()),
                    )
                }),
        )
        .child(
            div()
                .flex_shrink_0()
                .child(control),
        )
}

impl SettingsView {
    fn render_sidebar(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = [
            (SettingsTab::General, None),
            (SettingsTab::Appearance, None),
            (SettingsTab::Keyboard, None),
            (SettingsTab::Ai, Some("✦")),
            (
                SettingsTab::Migration,
                if !self.detected_external_configs.is_empty() {
                    Some("New")
                } else {
                    None
                },
            ),
            (SettingsTab::Advanced, None),
        ];

        div()
            .id("settings-sidebar")
            .w(px(230.))
            .min_w(px(210.))
            .max_w(px(250.))
            .h_full()
            .bg(theme.sidebar_bg)
            .border_r_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .overflow_hidden()
            // Top draggable spacer (clears macOS traffic lights)
            .child(
                div()
                    .h(if cfg!(target_os = "macos") { px(38.) } else { px(16.) })
                    .w_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |ev, window, _cx| {
                        if ev.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
            // Search box
            .child({
                let is_search_active = self.active_input_field == Some(ActiveInputField::Search);
                let query = self.search_query.clone();
                div()
                    .id("settings-search-bar")
                    .mx(px(12.))
                    .mb(px(10.))
                    .h(px(32.))
                    .rounded(px(8.))
                    .bg(theme.surface)
                    .border_1()
                    .border_color(if is_search_active { theme.accent } else { theme.border })
                    .px(px(10.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        this.active_input_field = Some(ActiveInputField::Search);
                        this.search_input.last_bounds = this.search_input_bounds.get();
                        let col = this.search_input.index_for_position(ev.position, 11.5, 0.0, window);
                        this.search_input.start_drag(col);
                        this.is_dragging_input = true;
                        cx.notify();
                    }))
                    .child(render_icon(IconType::Search, if is_search_active { theme.accent } else { theme.muted }, 12.0))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w(px(0.))
                            .child({
                                let bounds_cell = self.search_input_bounds.clone();
                                canvas(
                                    |_, _, _| {},
                                    move |bounds, _, _, _| {
                                        bounds_cell.set(Some(bounds));
                                    },
                                )
                                .absolute()
                                .size_full()
                            })
                            .child(if query.is_empty() {
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .when(is_search_active, |this| {
                                        this.child(
                                            div()
                                                .w(px(2.))
                                                .h(px(13.))
                                                .rounded(px(1.))
                                                .bg(theme.accent)
                                                .mr(px(2.)),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(theme.muted)
                                            .child("Search settings..."),
                                    )
                            } else {
                                crate::ui::text_input::render_line_spans(
                                    &query,
                                    0,
                                    if is_search_active { Some(self.search_input.cursor) } else { None },
                                    self.search_input.selection,
                                    11.5,
                                    theme,
                                    Some(window),
                                )
                            })
                    )
            })
            // App Branding / Profile Card
            .child(
                div()
                    .mx(px(10.))
                    .mb(px(12.))
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(theme.surface_raised.opacity(0.6))
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2p5()
                    .child(render_app_logo(22.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .child("Fastty Terminal"),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(theme.muted)
                                    .child(concat!("v", env!("CARGO_PKG_VERSION"), " • Preferences")),
                            ),
                    ),
            )
            // Tab list
            .child(
                div()
                    .id("settings-tabs-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(8.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(tabs.into_iter().map(|(tab, badge)| {
                        let is_active = self.active_tab == tab;
                        div()
                            .id(SharedString::from(format!("tab-item-{:?}", tab)))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .px(px(10.))
                            .py(px(7.))
                            .rounded(px(7.))
                            .bg(if is_active { theme.surface_raised } else { theme.sidebar_bg })
                            .border_1()
                            .border_color(if is_active { theme.border } else { theme.sidebar_bg })
                            .hover(move |s| if !is_active { s.bg(theme.hover) } else { s })
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                this.active_tab = tab;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2p5()
                                    .child(render_icon(
                                        tab.icon(),
                                        if is_active { theme.accent } else { theme.muted_strong },
                                        13.0,
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                            .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                            .child(tab.label()),
                                    ),
                            )
                            .when_some(badge, |this, badge_text| {
                                this.child(
                                    div()
                                        .px(px(5.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .bg(if is_active { theme.selected } else { theme.surface })
                                        .text_size(px(10.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(if is_active { theme.accent } else { theme.muted })
                                        .child(badge_text),
                                )
                            })
                    })),
            )
    }

    fn render_content_pane(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("settings-content-pane")
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.main_bg)
            .overflow_hidden()
            // Header
            .child(
                div()
                    .id("settings-header")
                    .h(px(44.))
                    .w_full()
                    .bg(theme.main_bg)
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(24.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(render_icon(self.active_tab.icon(), theme.accent, 14.0))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .child(self.active_tab.label()),
                            ),
                    )
                    // Drag region to move window
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag)
                            .on_mouse_down(MouseButton::Left, |ev, window, _cx| {
                                if ev.click_count == 2 {
                                    window.zoom_window();
                                } else {
                                    window.start_window_move();
                                }
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme.muted)
                                    .child("Press Esc to close"),
                            )
                            .when(!cfg!(target_os = "macos"), |this| {
                                this.child(
                                    div()
                                        .id("settings-win-close")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(26.))
                                        .h(px(22.))
                                        .rounded(px(4.))
                                        .when(cfg!(target_os = "windows"), |this| {
                                            this.window_control_area(WindowControlArea::Close)
                                        })
                                        .hover(move |s| s.bg(gpui::Hsla { h: 0.0, s: 0.7, l: 0.45, a: 1.0 }))
                                        .cursor(CursorStyle::PointingHand)
                                        .on_mouse_down(MouseButton::Left, |_ev, window, _cx| {
                                            *SETTINGS_WINDOW_HANDLE.lock() = None;
                                            window.remove_window();
                                        })
                                        .child(render_icon(IconType::X, theme.foreground, 10.0)),
                                )
                            }),
                    ),
            )
            // Scrollable Content
            .child(
                div()
                    .id("settings-scroll-container")
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("settings-scroll-content")
                            .track_scroll(&self.window_scroll_handle)
                            .size_full()
                            .overflow_y_scroll()
                            .p(px(24.))
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(match self.active_tab {
                                SettingsTab::General => self.render_tab_general(theme, cx),
                                SettingsTab::Appearance => self.render_tab_appearance(theme, window, cx),
                                SettingsTab::Keyboard => self.render_tab_keyboard(theme, cx),
                                SettingsTab::Ai => self.render_tab_ai(theme, window, cx),
                                SettingsTab::Migration => self.render_tab_migration(theme, cx),
                                SettingsTab::Advanced => self.render_tab_advanced(theme, cx),
                            }),
                    ),
            )
    }

    fn render_tab_general(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let is_horiz = self.tab_layout == crate::config::TabLayout::Horizontal;
        let is_vert = self.tab_layout == crate::config::TabLayout::Vertical;

        div()
            .flex()
            .flex_col()
            .gap_6()
            // Section 1: Tab Bar Layout (Visual Mode Selector)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Tab Bar Layout", theme))
                    .child(
                        render_group_card(theme)
                            .p(px(14.))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_4()
                                    // Horizontal Card
                                    .child(
                                        div()
                                            .id("layout-horizontal-card")
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap_2p5()
                                            .p(px(10.))
                                            .rounded(px(8.))
                                            .bg(if is_horiz { theme.surface_raised } else { theme.surface })
                                            .border_1()
                                            .border_color(if is_horiz { theme.accent } else { theme.border })
                                            .hover(move |s| s.bg(theme.surface_raised))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.set_tab_layout(crate::config::TabLayout::Horizontal, cx);
                                            }))
                                            // Mini mockup
                                            .child(
                                                div()
                                                    .w_full()
                                                    .h(px(72.))
                                                    .rounded(px(6.))
                                                    .bg(theme.main_bg)
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .flex()
                                                    .flex_col()
                                                    .overflow_hidden()
                                                    .child(
                                                        div()
                                                            .h(px(16.))
                                                            .w_full()
                                                            .bg(theme.tab_bar_bg)
                                                            .border_b_1()
                                                            .border_color(theme.border)
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .px(px(6.))
                                                            .gap_1()
                                                            .child(div().w(px(28.)).h(px(9.)).rounded(px(2.)).bg(theme.accent))
                                                            .child(div().w(px(22.)).h(px(9.)).rounded(px(2.)).bg(theme.surface_raised)),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .p(px(8.))
                                                            .flex()
                                                            .flex_col()
                                                            .gap_1()
                                                            .child(div().w(px(32.)).h(px(4.)).rounded_full().bg(theme.accent.opacity(0.7)))
                                                            .child(div().w(px(64.)).h(px(3.)).rounded_full().bg(theme.muted.opacity(0.5))),
                                                    ),
                                            )
                                            // Pill label
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .px(px(10.))
                                                    .py(px(4.))
                                                    .rounded_full()
                                                    .bg(if is_horiz { theme.selected } else { theme.surface })
                                                    .border_1()
                                                    .border_color(if is_horiz { theme.accent } else { theme.border })
                                                    .child(
                                                        div()
                                                            .text_size(px(11.5))
                                                            .font_weight(if is_horiz { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                            .text_color(if is_horiz { theme.foreground } else { theme.muted })
                                                            .child("Horizontal (Top Bar)"),
                                                    )
                                                    .when(is_horiz, |this| {
                                                        this.child(render_icon(IconType::Check, theme.accent, 11.0))
                                                    }),
                                            ),
                                    )
                                    // Vertical Card
                                    .child(
                                        div()
                                            .id("layout-vertical-card")
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap_2p5()
                                            .p(px(10.))
                                            .rounded(px(8.))
                                            .bg(if is_vert { theme.surface_raised } else { theme.surface })
                                            .border_1()
                                            .border_color(if is_vert { theme.accent } else { theme.border })
                                            .hover(move |s| s.bg(theme.surface_raised))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.set_tab_layout(crate::config::TabLayout::Vertical, cx);
                                            }))
                                            // Mini mockup
                                            .child(
                                                div()
                                                    .w_full()
                                                    .h(px(72.))
                                                    .rounded(px(6.))
                                                    .bg(theme.main_bg)
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .flex()
                                                    .flex_row()
                                                    .overflow_hidden()
                                                    .child(
                                                        div()
                                                            .w(px(24.))
                                                            .h_full()
                                                            .bg(theme.sidebar_bg)
                                                            .border_r_1()
                                                            .border_color(theme.border)
                                                            .flex()
                                                            .flex_col()
                                                            .items_center()
                                                            .pt(px(6.))
                                                            .gap_1()
                                                            .child(div().w(px(14.)).h(px(6.)).rounded(px(2.)).bg(theme.accent))
                                                            .child(div().w(px(14.)).h(px(6.)).rounded(px(2.)).bg(theme.surface_raised))
                                                            .child(div().w(px(14.)).h(px(6.)).rounded(px(2.)).bg(theme.surface_raised)),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .p(px(8.))
                                                            .flex()
                                                            .flex_col()
                                                            .gap_1()
                                                            .child(div().w(px(32.)).h(px(4.)).rounded_full().bg(theme.accent.opacity(0.7)))
                                                            .child(div().w(px(64.)).h(px(3.)).rounded_full().bg(theme.muted.opacity(0.5))),
                                                    ),
                                            )
                                            // Pill label
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .px(px(10.))
                                                    .py(px(4.))
                                                    .rounded_full()
                                                    .bg(if is_vert { theme.selected } else { theme.surface })
                                                    .border_1()
                                                    .border_color(if is_vert { theme.accent } else { theme.border })
                                                    .child(
                                                        div()
                                                            .text_size(px(11.5))
                                                            .font_weight(if is_vert { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                            .text_color(if is_vert { theme.foreground } else { theme.muted })
                                                            .child("Vertical (Sidebar)"),
                                                    )
                                                    .when(is_vert, |this| {
                                                        this.child(render_icon(IconType::Check, theme.accent, 11.0))
                                                    }),
                                            ),
                                    ),
                            ),
                    ),
            )
            // Section 2: Terminal Behavior (Grouped Card)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Terminal Behavior", theme))
                    .child(
                        render_group_card(theme)
                            .child(render_card_row(
                                "Cursor Blinking",
                                Some("Smooth animated cursor in terminal buffer"),
                                div()
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                        this.toggle_cursor_blink(cx);
                                    }))
                                    .child(render_toggle_switch(self.cursor_blink, theme)),
                                false,
                                theme,
                            ))
                            .child(render_card_row(
                                "Copy on Select",
                                Some("Automatically copy highlighted text to system clipboard"),
                                div()
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                        this.toggle_copy_on_select(cx);
                                    }))
                                    .child(render_toggle_switch(self.copy_on_select, theme)),
                                false,
                                theme,
                            ))
                            .child({
                                let scrollback_pills = [
                                    (5000, "5,000"),
                                    (10000, "10,000"),
                                    (20000, "20,000"),
                                    (50000, "50,000"),
                                    (100000, "100,000"),
                                ];
                                render_card_row(
                                    "Scrollback History",
                                    Some("Maximum lines preserved in terminal buffer"),
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap_1p5()
                                        .children(scrollback_pills.into_iter().map(|(lines, label)| {
                                            let is_active = self.scrollback == lines;
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .px(px(8.))
                                                .py(px(4.))
                                                .rounded(px(5.))
                                                .border_1()
                                                .border_color(if is_active { theme.accent } else { theme.border })
                                                .bg(if is_active { theme.accent } else { theme.surface_raised })
                                                .hover(move |s| if !is_active { s.bg(theme.hover) } else { s })
                                                .text_color(if is_active { theme.background } else { theme.foreground })
                                                .text_size(px(11.))
                                                .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                    this.set_scrollback(lines, cx);
                                                }))
                                                .child(label)
                                                .when(is_active, |el| el.child(render_icon(IconType::Check, theme.background, 10.0)))
                                        })),
                                    true,
                                    theme,
                                )
                            }),
                    ),
            )
    }

    fn render_tab_appearance(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_6()
            // Section 1: Color Theme
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Color Theme", theme))
                    .child(
                        render_group_card(theme)
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .px(px(16.))
                                    .py(px(12.))
                                    .border_b_1()
                                    .border_color(theme.border)
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .text_size(px(12.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(theme.foreground)
                                                    .child("Active Theme"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(theme.muted)
                                                    .child(SharedString::from(format!("Currently applied: {}", self.current_theme_name))),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1p5()
                                            .px(px(8.))
                                            .py(px(4.))
                                            .rounded(px(5.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(theme.border)
                                            .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(theme.background))
                                            .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(theme.surface))
                                            .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(theme.accent))
                                            .child(
                                                div()
                                                    .ml(px(2.))
                                                    .text_size(px(11.))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(theme.foreground)
                                                    .child(SharedString::from(self.current_theme_name.clone())),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .p(px(14.))
                                    .flex()
                                    .flex_row()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(self.theme_cards.iter().map(|card| {
                                        let is_active = self.current_theme_name == card.name;
                                        let theme_name_str = card.name.clone();
                                        let bg_color = card.bg_color;
                                        let surf_color = card.surf_color;
                                        let acc_color = card.acc_color;
                                        let label = card.label.clone();
                                        div()
                                            .id(SharedString::from(format!("theme-card-{}", card.name)))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .px(px(12.))
                                            .py(px(8.))
                                            .rounded(px(8.))
                                            .bg(if is_active { theme.surface_raised } else { theme.surface })
                                            .border_1()
                                            .border_color(if is_active { theme.accent } else { theme.border })
                                            .hover(move |s| s.bg(theme.surface_raised))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                this.set_theme(&theme_name_str, cx);
                                            }))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_2p5()
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_1()
                                                            .p(px(2.))
                                                            .rounded(px(4.))
                                                            .bg(bg_color)
                                                            .border_1()
                                                            .border_color(surf_color)
                                                            .child(div().w(px(7.)).h(px(7.)).rounded_full().bg(bg_color))
                                                            .child(div().w(px(7.)).h(px(7.)).rounded_full().bg(surf_color))
                                                            .child(div().w(px(7.)).h(px(7.)).rounded_full().bg(acc_color)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(11.5))
                                                            .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                            .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                                            .child(label),
                                                    ),
                                            )
                                            .when(is_active, |this| {
                                                this.child(render_icon(IconType::Check, theme.accent, 12.0))
                                            })
                                    })),
                            ),
                    ),
            )
            // Section 2: Window Transparency
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Window Transparency", theme))
                    .child(
                        render_group_card(theme).child({
                            let opacity_pills = [1.0, 0.95, 0.90, 0.85, 0.80, 0.75];
                            render_card_row(
                                "Window Opacity",
                                Some("Background transparency and macOS vibrancy blur"),
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_1p5()
                                    .children(opacity_pills.into_iter().map(|val| {
                                        let is_active = (self.opacity - val).abs() < 0.025;
                                        let label = format!("{:.0}%", val * 100.0);
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .px(px(8.))
                                            .py(px(4.))
                                            .rounded(px(5.))
                                            .border_1()
                                            .border_color(if is_active { theme.accent } else { theme.border })
                                            .bg(if is_active { theme.accent } else { theme.surface_raised })
                                            .hover(move |s| if !is_active { s.bg(theme.hover) } else { s })
                                            .text_color(if is_active { theme.background } else { theme.foreground })
                                            .text_size(px(11.))
                                            .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                this.adjust_opacity(val, cx);
                                            }))
                                            .child(label)
                                            .when(is_active, |el| el.child(render_icon(IconType::Check, theme.background, 10.0)))
                                    })),
                                true,
                                theme,
                            )
                        }),
                    ),
            )
            // Section 3: Typography
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Typography", theme))
                    .child(
                        div()
                            .bg(theme.surface)
                            .border_1()
                            .border_color(theme.border)
                            .rounded(px(10.))
                            .flex()
                            .flex_col()
                            .child(render_card_row(
                                "Font Size",
                                Some("Terminal text grid cell point size"),
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(28.))
                                            .h(px(28.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(6.))
                                            .bg(theme.surface_raised)
                                            .hover(|s| s.bg(theme.hover))
                                            .border_1()
                                            .border_color(theme.border)
                                            .text_color(theme.foreground)
                                            .font_weight(FontWeight::BOLD)
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.adjust_font_size(-1.0, cx);
                                            }))
                                            .child("−"),
                                    )
                                    .child(
                                        div()
                                            .min_w(px(46.))
                                            .flex()
                                            .justify_center()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.accent)
                                            .child(format!("{:.0}px", self.font_size)),
                                    )
                                    .child(
                                        div()
                                            .w(px(28.))
                                            .h(px(28.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(6.))
                                            .bg(theme.surface_raised)
                                            .hover(|s| s.bg(theme.hover))
                                            .border_1()
                                            .border_color(theme.border)
                                            .text_color(theme.foreground)
                                            .font_weight(FontWeight::BOLD)
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.adjust_font_size(1.0, cx);
                                            }))
                                            .child("+"),
                                    ),
                                false,
                                theme,
                            ))
                            .child(
                                div()
                                    .p(px(16.))
                                    .child(self.render_font_combobox(theme, window, cx)),
                            ),
                    ),
            )
    }

    fn render_font_combobox(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let is_open = self.font_combobox_open;
        let is_search_active = self.active_input_field == Some(ActiveInputField::FontSearch);
        let query = self.font_search_query.clone();
        let query_lower = query.trim().to_lowercase();
        let filtered_indices: Vec<usize> = if query_lower.is_empty() {
            (0..self.system_fonts.len()).collect()
        } else {
            self.system_fonts
                .iter()
                .enumerate()
                .filter(|(_, f)| f.to_lowercase().contains(&query_lower))
                .map(|(idx, _)| idx)
                .collect()
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("Font Family"),
                    )
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(theme.surface_raised)
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.accent)
                            .child(self.font_family.clone()),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(36.))
                            .px(px(12.))
                            .rounded(px(6.))
                            .bg(if is_open { theme.surface_raised } else { theme.surface })
                            .border_1()
                            .border_color(if is_open { theme.accent } else { theme.border })
                            .hover(move |s| s.bg(theme.surface_raised).border_color(theme.accent))
                            .cursor(CursorStyle::PointingHand)
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                this.toggle_font_combobox(cx);
                            }))
                            .child({
                                let bounds_cell = self.font_trigger_bounds.clone();
                                canvas(
                                    |_, _, _| {},
                                    move |bounds, _, _, _| {
                                        bounds_cell.set(Some(bounds));
                                    },
                                )
                                .absolute()
                                .size_full()
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(22.))
                                            .h(px(22.))
                                            .rounded(px(4.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(theme.border)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.accent)
                                            .font_family(SharedString::from(self.font_family.clone()))
                                            .child("Aa"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.foreground)
                                            .child(self.font_family.clone()),
                                    ),
                            )
                            .child(render_icon(
                                if is_open { IconType::ChevronUp } else { IconType::ChevronDown },
                                if is_open { theme.accent } else { theme.muted_strong },
                                12.0,
                            )),
                    )
                    .when(is_open, |combobox| {
                        let total_fonts = self.system_fonts.len();
                        let shown_fonts = filtered_indices.len();

                        let open_upward = if let Some(bounds) = self.font_trigger_bounds.get() {
                            let window_h = window.viewport_size().height;
                            let space_below = window_h - bounds.bottom();
                            let space_above = bounds.top();
                            space_below < px(290.) && space_above > space_below
                        } else {
                            false
                        };

                        combobox.child(
                            div()
                                .id("font-combobox-dropdown")
                                .absolute()
                                .when(open_upward, |el| el.bottom(px(42.)))
                                .when(!open_upward, |el| el.top(px(42.)))
                                .left_0()
                                .right_0()
                                .occlude()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_scroll_wheel(|_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .rounded(px(8.))
                                .bg({
                                    let mut bg = theme.surface_raised;
                                    bg.a = 1.0;
                                    bg
                                })
                                .border_1()
                                .border_color(theme.border)
                                .shadow_lg()
                                .p(px(6.))
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(
                                    div()
                                        .h(px(32.))
                                        .rounded(px(6.))
                                        .bg(theme.surface)
                                        .border_1()
                                        .border_color(if is_search_active { theme.accent } else { theme.border })
                                        .px(px(8.))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .cursor(CursorStyle::IBeam)
                                        .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                            cx.stop_propagation();
                                            this.active_input_field = Some(ActiveInputField::FontSearch);
                                            this.font_search_state.last_bounds = this.font_search_bounds.get();
                                            let col = this.font_search_state.index_for_position(ev.position, 11.5, 0.0, window);
                                            this.font_search_state.start_drag(col);
                                            this.is_dragging_input = true;
                                            cx.notify();
                                        }))
                                        .child(render_icon(
                                            IconType::Search,
                                            if is_search_active { theme.accent } else { theme.muted },
                                            12.0,
                                        ))
                                        .child(
                                            div()
                                                .relative()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .child({
                                                    let bounds_cell = self.font_search_bounds.clone();
                                                    canvas(
                                                        |_, _, _| {},
                                                        move |bounds, _, _, _| {
                                                            bounds_cell.set(Some(bounds));
                                                        },
                                                    )
                                                    .absolute()
                                                    .size_full()
                                                })
                                                .child(if query.is_empty() {
                                                    div()
                                                        .flex()
                                                        .flex_row()
                                                        .items_center()
                                                        .when(is_search_active, |this| {
                                                            this.child(
                                                                div()
                                                                    .w(px(2.))
                                                                    .h(px(13.))
                                                                    .rounded(px(1.))
                                                                    .bg(theme.accent)
                                                                    .mr(px(2.)),
                                                            )
                                                        })
                                                        .child(
                                                            div()
                                                                .text_size(px(11.5))
                                                                .text_color(theme.muted)
                                                                .child("Search font family..."),
                                                        )
                                                } else {
                                                    crate::ui::text_input::render_line_spans(
                                                        &query,
                                                        0,
                                                        if is_search_active { Some(self.font_search_state.cursor) } else { None },
                                                        self.font_search_state.selection,
                                                        11.5,
                                                        theme,
                                                        Some(window),
                                                    )
                                                }),
                                        )
                                        .when(!query.is_empty(), |el| {
                                            el.child(
                                                div()
                                                    .px(px(4.))
                                                    .py(px(2.))
                                                    .rounded(px(4.))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .hover(move |s| s.bg(theme.surface_raised))
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                        cx.stop_propagation();
                                                        this.font_search_state.clear();
                                                        this.font_search_query.clear();
                                                        this.font_highlighted_index = 0;
                                                        cx.notify();
                                                    }))
                                                    .child(render_icon(IconType::X, theme.muted, 11.0)),
                                            )
                                        }),
                                )
                                .child(div().h(px(1.)).bg(theme.border).my(px(2.)))
                                .child(
                                    div()
                                        .id("font-combobox-list")
                                        .h(px(230.))
                                        .overflow_hidden()
                                        .when(filtered_indices.is_empty(), |el| {
                                            el.child(
                                                div()
                                                    .py(px(24.))
                                                    .flex()
                                                    .flex_col()
                                                    .items_center()
                                                    .justify_center()
                                                    .gap_1p5()
                                                    .child(render_icon(IconType::Search, theme.muted, 16.0))
                                                    .child(
                                                        div()
                                                            .text_size(px(11.5))
                                                            .text_color(theme.muted)
                                                            .child(format!("No fonts matching \"{}\"", query)),
                                                    ),
                                            )
                                        })
                                        .when(!filtered_indices.is_empty(), |el| {
                                            let filtered = filtered_indices.clone();
                                            el.child(
                                                uniform_list(
                                                    "font-uniform-list",
                                                    filtered.len(),
                                                    cx.processor(move |this: &mut SettingsView, range: std::ops::Range<usize>, _window, cx| {
                                                        let theme = this.theme;
                                                        let font_family_curr = this.font_family.clone();
                                                        let highlighted = this.font_highlighted_index;
                                                        let mut items = Vec::with_capacity(range.len());

                                                        for pos in range {
                                                            let font_idx = filtered[pos];
                                                            let font_name = this.system_fonts[font_idx].clone();
                                                            let is_selected = font_family_curr == font_name;
                                                            let is_focused = pos == highlighted;
                                                            let name_for_click = font_name.clone();

                                                            items.push(
                                                                div()
                                                                    .id(pos)
                                                                    .h(px(28.))
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .justify_between()
                                                                    .px(px(8.))
                                                                    .py(px(4.))
                                                                    .rounded(px(5.))
                                                                    .bg(if is_selected {
                                                                        theme.accent.opacity(0.15)
                                                                    } else if is_focused {
                                                                        theme.hover
                                                                    } else {
                                                                        theme.surface_raised
                                                                    })
                                                                    .hover(move |s| {
                                                                        s.bg(if is_selected {
                                                                            theme.accent.opacity(0.2)
                                                                        } else {
                                                                            theme.hover
                                                                        })
                                                                    })
                                                                    .cursor(CursorStyle::PointingHand)
                                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                        cx.stop_propagation();
                                                                        this.set_font_family(&name_for_click, cx);
                                                                        this.font_combobox_open = false;
                                                                        this.active_input_field = None;
                                                                        cx.notify();
                                                                    }))
                                                                    .child(
                                                                        div()
                                                                            .flex()
                                                                            .flex_row()
                                                                            .items_center()
                                                                            .gap_2()
                                                                            .child(
                                                                                div()
                                                                                    .text_size(px(12.))
                                                                                    .text_color(if is_selected {
                                                                                        theme.foreground
                                                                                    } else if is_focused {
                                                                                        theme.foreground
                                                                                    } else {
                                                                                        theme.muted_strong
                                                                                    })
                                                                                    .font_weight(if is_selected {
                                                                                        FontWeight::BOLD
                                                                                    } else {
                                                                                        FontWeight::NORMAL
                                                                                    })
                                                                                    .child(font_name.clone()),
                                                                            )
                                                                            .child(
                                                                                div()
                                                                                    .px(px(4.))
                                                                                    .py(px(0.5))
                                                                                    .rounded(px(3.))
                                                                                    .bg(theme.surface)
                                                                                    .text_size(px(9.5))
                                                                                    .text_color(theme.muted)
                                                                                    .font_family(SharedString::from(font_name.clone()))
                                                                                    .child("Aa"),
                                                                            ),
                                                                    )
                                                                    .when(is_selected, |row| {
                                                                        row.child(render_icon(IconType::Check, theme.accent, 11.0))
                                                                    }),
                                                            );
                                                        }
                                                        items
                                                    }),
                                                )
                                                .track_scroll(&self.font_scroll_handle)
                                                .h(px(230.))
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .pt(px(6.))
                                        .pb(px(2.))
                                        .px(px(4.))
                                        .border_t_1()
                                        .border_color(theme.border)
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_size(px(10.5))
                                                .text_color(theme.muted)
                                                .child(format!("{} of {} fonts", shown_fonts, total_fonts)),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("↑↓ navigate • ↵ select • esc close"),
                                        ),
                                 ),
                        )
                    }),
            )
    }

    fn render_tab_keyboard(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let presets = [
            (crate::keybindings::KeybindingPreset::Default, "Default (Fastty)", "Standard modern shortcuts (⌘T, ⌘W, ⌘D, ⌘⌥Arrows)"),
            (crate::keybindings::KeybindingPreset::Ghostty, "Ghostty", "Ghostty-compatible splits, tabs & nav"),
            (crate::keybindings::KeybindingPreset::Tmux, "tmux", "tmux-compatible Ctrl+B leader & Alt arrows"),
            (crate::keybindings::KeybindingPreset::ITerm2, "iTerm2", "macOS iTerm2 muscle memory shortcuts"),
        ];

        let shortcuts_ref = [
            ("Toggle AI Assistant Sidebar", if cfg!(target_os = "macos") { "⌘L" } else { "Ctrl+Shift+L" }),
            ("Command Palette", if cfg!(target_os = "macos") { "⌘P" } else { "Ctrl+Shift+P" }),
            ("Mission Control / Tab Peek", if cfg!(target_os = "macos") { "⌘⇧O" } else { "Ctrl+Shift+M" }),
            ("Find in All Tabs (Global Search)", if cfg!(target_os = "macos") { "⌘⇧F" } else { "Ctrl+Shift+F" }),
            ("Split Pane Right", if cfg!(target_os = "macos") { "⌘D" } else { "Ctrl+Shift+D" }),
            ("Split Pane Down", if cfg!(target_os = "macos") { "⌘⇧D" } else { "Ctrl+Shift+E" }),
        ];

        div()
            .flex()
            .flex_col()
            .gap_6()
            // Section 1: Presets
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Keybinding Presets", theme))
                    .child(
                        render_group_card(theme).children(presets.into_iter().enumerate().map(|(idx, (preset, name, desc))| {
                            let is_active = self.keybinding_preset == preset;
                            let is_last = idx == presets.len() - 1;
                            div()
                                .id(SharedString::from(format!("preset-row-{}", preset)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .px(px(16.))
                                .py(px(12.))
                                .when(!is_last, |this| this.border_b_1().border_color(theme.border))
                                .hover(move |s| s.bg(theme.surface_raised))
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                    this.set_keybinding_preset(preset, cx);
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                                .child(name),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme.muted)
                                                .child(desc),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(20.))
                                        .h(px(20.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(if is_active { theme.accent } else { theme.border })
                                        .bg(if is_active { theme.accent } else { theme.surface })
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(is_active, |this| {
                                            this.child(render_icon(IconType::Check, theme.background, 12.0))
                                        }),
                                )
                        })),
                    ),
            )
            // Section 2: Shortcuts Quick Reference
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Quick Shortcuts Reference", theme))
                    .child(
                        render_group_card(theme).children(shortcuts_ref.into_iter().enumerate().map(|(idx, (name, sc))| {
                            let is_last = idx == shortcuts_ref.len() - 1;
                            render_card_row(
                                name,
                                None::<&str>,
                                div()
                                    .px(px(7.))
                                    .py(px(3.))
                                    .rounded(px(5.))
                                    .bg(theme.surface_raised)
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .child(sc),
                                is_last,
                                theme,
                            )
                        })),
                    ),
            )
    }

    fn render_tab_ai(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let providers = [
            ("ollama", "Ollama", "Local / Free", "Offline private models on localhost:11434"),
            ("anthropic", "Anthropic", "Cloud API", "Claude 3.7 Sonnet & 3.5 Haiku"),
            ("openai", "OpenAI", "Cloud API", "GPT-4o, GPT-4o-mini & o3-mini"),
            ("openrouter", "OpenRouter", "Unified API", "Hundreds of open & proprietary models"),
            ("lm-studio", "LM Studio", "Local Server", "Local LLM server on localhost:1234"),
        ];

        let is_ollama = self.ai_provider == "ollama";
        let default_presets = crate::ai::AiConfig::default_preset_models(&self.ai_provider);
        let chips_to_render: Vec<String> = if is_ollama {
            if !self.detected_ollama_models.is_empty() {
                self.detected_ollama_models.clone()
            } else {
                default_presets
            }
        } else {
            default_presets
        };

        let api_key_env = self
            .config
            .ai
            .providers
            .get(&self.ai_provider)
            .and_then(|p| p.api_key_env().map(|s| s.to_string()));

        let (auth_desc, auth_color, auth_badge) = if !self.ai_api_key_input.trim().is_empty() {
            ("Direct API key configured and stored in fastty.toml".to_string(), theme.green, "Custom Key")
        } else if let Some(ref env_var) = api_key_env {
            if std::env::var(env_var).is_ok() {
                (format!("Using ${} from environment", env_var), theme.green, "Environment")
            } else {
                (format!("Missing API key (${} not found in environment)", env_var), theme.yellow, "Missing")
            }
        } else {
            ("No API key required (local inference)".to_string(), theme.muted, "Local")
        };

        let permissions = [
            (crate::ai::PermissionMode::ConfirmWrites, "Confirm Writes", "Prompt before running commands or editing files (Recommended)"),
            (crate::ai::PermissionMode::ConfirmAll, "Confirm All", "Prompt for all tool actions including file reads"),
            (crate::ai::PermissionMode::Yolo, "Autonomous (YOLO)", "Execute tool calls immediately without approval prompts"),
        ];

        div()
            .flex()
            .flex_col()
            .gap_6()
            // Section 1: AI Provider
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("AI Provider", theme))
                    .child(
                        render_group_card(theme).children(providers.into_iter().enumerate().map(|(idx, (id, name, badge, desc))| {
                            let is_active = self.ai_provider == id;
                            let is_last = idx == providers.len() - 1;
                            div()
                                .id(SharedString::from(format!("ai-prov-row-{}", id)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .px(px(16.))
                                .py(px(12.))
                                .when(!is_last, |this| this.border_b_1().border_color(theme.border))
                                .hover(move |s| s.bg(theme.surface_raised))
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                    this.set_ai_provider(id, cx);
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .text_size(px(12.5))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                                        .child(name),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(9.5))
                                                        .px(px(5.))
                                                        .py(px(1.))
                                                        .rounded(px(3.))
                                                        .bg(if is_active { theme.selected } else { theme.surface_raised })
                                                        .text_color(if is_active { theme.accent } else { theme.muted })
                                                        .child(badge),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme.muted)
                                                .child(desc),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(20.))
                                        .h(px(20.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(if is_active { theme.accent } else { theme.border })
                                        .bg(if is_active { theme.accent } else { theme.surface })
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(is_active, |this| {
                                            this.child(render_icon(IconType::Check, theme.background, 12.0))
                                        }),
                                )
                        })),
                    ),
            )
            // Section 2: Model Configuration (BYOK)
            .child({
                let is_model_focused = self.active_input_field == Some(ActiveInputField::AiModel);
                let model_val = self.ai_model_input.clone();

                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Model Configuration (BYOK)", theme))
                    .child(
                        render_group_card(theme)
                            .child(render_card_row(
                                "Model Identifier",
                                Some("Model name or alias (e.g. GLM-5.7, Opus 5)"),
                                div()
                                    .w(px(210.))
                                    .h(px(30.))
                                    .relative()
                                    .child({
                                        let bounds_cell = self.ai_model_bounds.clone();
                                        canvas(
                                            |_, _, _| {},
                                            move |bounds, _, _, _| {
                                                bounds_cell.set(Some(bounds));
                                            },
                                        )
                                        .absolute()
                                        .size_full()
                                    })
                                    .px(px(8.))
                                    .rounded(px(6.))
                                    .bg(theme.surface_raised)
                                    .border_1()
                                    .border_color(if is_model_focused { theme.accent } else { theme.border })
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .cursor(CursorStyle::IBeam)
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                        this.active_input_field = Some(ActiveInputField::AiModel);
                                        this.ai_model_state.last_bounds = this.ai_model_bounds.get();
                                        let col = this.ai_model_state.index_for_position(ev.position, 11.0, 8.0, window);
                                        this.ai_model_state.start_drag(col);
                                        this.is_dragging_input = true;
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .overflow_hidden()
                                            .child(if model_val.is_empty() {
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .when(is_model_focused, |this| {
                                                        this.child(
                                                            div()
                                                                .w(px(2.))
                                                                .h(px(13.))
                                                                .rounded(px(1.))
                                                                .bg(theme.accent)
                                                                .mr(px(2.)),
                                                        )
                                                    })
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted)
                                                            .child(match self.ai_provider.as_str() {
                                                                "ollama" => "e.g. gemma4:12b",
                                                                "anthropic" => "e.g. claude-3-7-sonnet",
                                                                "openai" => "e.g. gpt-4o",
                                                                _ => "Custom model identifier...",
                                                            }),
                                                    )
                                            } else {
                                                crate::ui::text_input::render_line_spans(
                                                    &model_val,
                                                    0,
                                                    if is_model_focused { Some(self.ai_model_state.cursor) } else { None },
                                                    self.ai_model_state.selection,
                                                    11.0,
                                                    theme,
                                                    Some(window),
                                                )
                                            }),
                                    )
                                    .when(!model_val.is_empty(), |this| {
                                        this.child(
                                            div()
                                                .flex_shrink_0()
                                                .ml(px(4.))
                                                .px(px(4.))
                                                .py(px(1.))
                                                .rounded(px(3.))
                                                .hover(move |s| s.bg(theme.hover))
                                                .cursor(CursorStyle::PointingHand)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("✕")
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.clear_ai_model(cx);
                                                })),
                                        )
                                    }),
                                false,
                                theme,
                            ))
                            .child(
                                div()
                                    .px(px(16.))
                                    .py(px(12.))
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_size(px(12.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(theme.foreground)
                                                    .child(if is_ollama {
                                                        "Installed Local Models"
                                                    } else {
                                                        "Preset & Recent Models"
                                                    }),
                                            )
                                            .when(is_ollama, |this| {
                                                this.child(
                                                    div()
                                                        .px(px(7.))
                                                        .py(px(3.))
                                                        .rounded(px(5.))
                                                        .bg(theme.surface_raised)
                                                        .border_1()
                                                        .border_color(theme.border)
                                                        .cursor(CursorStyle::PointingHand)
                                                        .hover(move |s| s.bg(theme.hover))
                                                        .text_size(px(10.5))
                                                        .text_color(theme.accent)
                                                        .child("Refresh ↺")
                                                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                            this.refresh_ollama_models(cx);
                                                        })),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .flex_wrap()
                                            .gap_1p5()
                                            .children(chips_to_render.into_iter().map(|model_name| {
                                                let is_active = self.ai_model_input.trim() == model_name;
                                                let m_name = model_name.clone();
                                                div()
                                                    .id(SharedString::from(format!("ai-model-pill-{}", model_name)))
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .px(px(8.))
                                                    .py(px(5.))
                                                    .rounded(px(6.))
                                                    .bg(if is_active { theme.surface_raised } else { theme.surface })
                                                    .border_1()
                                                    .border_color(if is_active { theme.accent } else { theme.border })
                                                    .hover(move |s| s.bg(theme.surface_raised))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                        this.set_ai_model(&m_name, cx);
                                                    }))
                                                    .when(is_ollama && !self.detected_ollama_models.is_empty(), |el| {
                                                        el.child(div().w(px(6.)).h(px(6.)).rounded_full().bg(theme.green))
                                                    })
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                            .text_color(if is_active { theme.foreground } else { theme.muted })
                                                            .child(SharedString::from(model_name)),
                                                    )
                                                    .when(is_active, |this| {
                                                        this.child(render_icon(IconType::Check, theme.accent, 11.0))
                                                    })
                                            })),
                                    ),
                            ),
                    )
            })
            // Section 3: Endpoint & Auth Status
            .child({
                let is_url_focused = self.active_input_field == Some(ActiveInputField::AiBaseUrl);
                let is_key_focused = self.active_input_field == Some(ActiveInputField::AiApiKey);
                let url_val = self.ai_base_url_input.clone();
                let key_val = self.ai_api_key_input.clone();

                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Endpoint & Authentication", theme))
                    .child(
                        render_group_card(theme)
                            .child(render_card_row(
                                "Endpoint URL",
                                Some("HTTP API base address for model requests"),
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(210.))
                                            .h(px(30.))
                                            .relative()
                                            .child({
                                                let bounds_cell = self.ai_base_url_bounds.clone();
                                                canvas(
                                                    |_, _, _| {},
                                                    move |bounds, _, _, _| {
                                                        bounds_cell.set(Some(bounds));
                                                    },
                                                )
                                                .absolute()
                                                .size_full()
                                            })
                                            .px(px(8.))
                                            .rounded(px(6.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(if is_url_focused { theme.accent } else { theme.border })
                                            .flex()
                                            .items_center()
                                            .cursor(CursorStyle::IBeam)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                                this.active_input_field = Some(ActiveInputField::AiBaseUrl);
                                                this.ai_base_url_state.last_bounds = this.ai_base_url_bounds.get();
                                                let col = this.ai_base_url_state.index_for_position(ev.position, 11.0, 8.0, window);
                                                this.ai_base_url_state.start_drag(col);
                                                this.is_dragging_input = true;
                                                cx.notify();
                                            }))
                                            .child(if url_val.is_empty() {
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .when(is_url_focused, |this| {
                                                        this.child(
                                                            div()
                                                                .w(px(2.))
                                                                .h(px(13.))
                                                                .rounded(px(1.))
                                                                .bg(theme.accent)
                                                                .mr(px(2.)),
                                                        )
                                                    })
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted)
                                                            .child("e.g. http://localhost:11434/v1"),
                                                    )
                                            } else {
                                                crate::ui::text_input::render_line_spans(
                                                    &url_val,
                                                    0,
                                                    if is_url_focused { Some(self.ai_base_url_state.cursor) } else { None },
                                                    self.ai_base_url_state.selection,
                                                    11.0,
                                                    theme,
                                                    Some(window),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .px(px(7.))
                                            .py(px(4.))
                                            .rounded(px(5.))
                                            .bg(theme.surface)
                                            .border_1()
                                            .border_color(theme.border)
                                            .hover(move |s| s.bg(theme.surface_raised))
                                            .cursor(CursorStyle::PointingHand)
                                            .text_size(px(10.5))
                                            .text_color(theme.muted_strong)
                                            .child("Reset")
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.reset_ai_base_url(cx);
                                            })),
                                    ),
                                false,
                                theme,
                            ))
                            .child(render_card_row(
                                "API Secret Key",
                                Some("Custom key stored in fastty.toml (overrides env)"),
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(210.))
                                            .h(px(30.))
                                            .relative()
                                            .child({
                                                let bounds_cell = self.ai_api_key_bounds.clone();
                                                canvas(
                                                    |_, _, _| {},
                                                    move |bounds, _, _, _| {
                                                        bounds_cell.set(Some(bounds));
                                                    },
                                                )
                                                .absolute()
                                                .size_full()
                                            })
                                            .px(px(8.))
                                            .rounded(px(6.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(if is_key_focused { theme.accent } else { theme.border })
                                            .flex()
                                            .items_center()
                                            .cursor(CursorStyle::IBeam)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                                this.active_input_field = Some(ActiveInputField::AiApiKey);
                                                this.ai_api_key_state.last_bounds = this.ai_api_key_bounds.get();
                                                let col = this.ai_api_key_state.index_for_position(ev.position, 11.0, 8.0, window);
                                                this.ai_api_key_state.start_drag(col);
                                                this.is_dragging_input = true;
                                                cx.notify();
                                            }))
                                            .child(if key_val.is_empty() {
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .when(is_key_focused, |this| {
                                                        this.child(
                                                            div()
                                                                .w(px(2.))
                                                                .h(px(13.))
                                                                .rounded(px(1.))
                                                                .bg(theme.accent)
                                                                .mr(px(2.)),
                                                        )
                                                    })
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted)
                                                            .child("Paste key (⌘V)..."),
                                                    )
                                            } else if is_key_focused {
                                                crate::ui::text_input::render_line_spans(
                                                    &key_val,
                                                    0,
                                                    Some(self.ai_api_key_state.cursor),
                                                    self.ai_api_key_state.selection,
                                                    11.0,
                                                    theme,
                                                    Some(window),
                                                )
                                            } else {
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted_strong)
                                                            .child("••••••••••••••••"),
                                                    )
                                            }),
                                    )
                                    .when(!key_val.is_empty(), |this| {
                                        this.child(
                                            div()
                                                .px(px(7.))
                                                .py(px(4.))
                                                .rounded(px(5.))
                                                .bg(theme.surface)
                                                .border_1()
                                                .border_color(theme.border)
                                                .hover(move |s| s.bg(theme.surface_raised))
                                                .cursor(CursorStyle::PointingHand)
                                                .text_size(px(10.5))
                                                .text_color(theme.muted_strong)
                                                .child("Clear")
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.clear_ai_api_key(cx);
                                                })),
                                        )
                                    }),
                                false,
                                theme,
                            ))
                            .child(render_card_row(
                                "Authentication Status",
                                Some(SharedString::from(auth_desc)),
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .child(div().w(px(7.)).h(px(7.)).rounded_full().bg(auth_color))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(auth_color)
                                            .child(auth_badge),
                                    ),
                                false,
                                theme,
                            ))
                            .child(render_card_row(
                                "Test Connection",
                                Some("Test endpoint reachability and model response"),
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        match &self.ai_test_status {
                                            AiTestStatus::Idle => div(),
                                            AiTestStatus::Testing => {
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .px(px(7.))
                                                    .py(px(3.))
                                                    .rounded(px(5.))
                                                    .bg(theme.surface_raised)
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .child(
                                                        div()
                                                            .text_size(px(10.5))
                                                            .text_color(theme.yellow)
                                                            .child("Testing..."),
                                                    )
                                            }
                                            AiTestStatus::Success(dur) => {
                                                let ms = dur.as_millis();
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .px(px(7.))
                                                    .py(px(3.))
                                                    .rounded(px(5.))
                                                    .bg(theme.surface_raised)
                                                    .border_1()
                                                    .border_color(theme.green)
                                                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(theme.green))
                                                    .child(
                                                        div()
                                                            .text_size(px(10.5))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(theme.green)
                                                            .child(SharedString::from(format!("OK ({}ms)", ms))),
                                                    )
                                            }
                                            AiTestStatus::Failed(err) => {
                                                let short_err = if err.len() > 30 {
                                                    format!("{}...", &err[..27])
                                                } else {
                                                    err.clone()
                                                };
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .px(px(7.))
                                                    .py(px(3.))
                                                    .rounded(px(5.))
                                                    .bg(theme.surface_raised)
                                                    .border_1()
                                                    .border_color(theme.red)
                                                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(theme.red))
                                                    .child(
                                                        div()
                                                            .text_size(px(10.5))
                                                            .text_color(theme.red)
                                                            .child(SharedString::from(format!("Failed: {}", short_err))),
                                                    )
                                            }
                                        }
                                    )
                                    .child({
                                        let is_testing = self.ai_test_status == AiTestStatus::Testing;
                                        div()
                                            .id("ai-test-connection-btn")
                                            .px(px(10.))
                                            .py(px(4.))
                                            .rounded(px(5.))
                                            .bg(if is_testing { theme.surface_raised } else { theme.accent })
                                            .border_1()
                                            .border_color(if is_testing { theme.border } else { theme.accent })
                                            .hover(move |s| {
                                                if is_testing { s } else { s.opacity(0.88) }
                                            })
                                            .cursor(if is_testing { CursorStyle::Arrow } else { CursorStyle::PointingHand })
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(if is_testing { theme.muted } else { theme.background })
                                            .child(if is_testing { "Testing..." } else { "Test Connection" })
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, window, cx| {
                                                this.test_ai_connection(window, cx);
                                            }))
                                    }),
                                true,
                                theme,
                            )),
                    )
            })
            // Section 4: Tool Execution Permissions
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Tool Execution Permissions", theme))
                    .child(
                        render_group_card(theme).children(permissions.into_iter().enumerate().map(|(idx, (mode, name, desc))| {
                            let is_active = self.ai_permission_mode == mode;
                            let is_last = idx == permissions.len() - 1;
                            div()
                                .id(SharedString::from(format!("ai-mode-row-{:?}", mode)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .px(px(16.))
                                .py(px(12.))
                                .when(!is_last, |this| this.border_b_1().border_color(theme.border))
                                .hover(move |s| s.bg(theme.surface_raised))
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                    this.set_ai_permission_mode(mode, cx);
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                                .child(name),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme.muted)
                                                .child(desc),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(20.))
                                        .h(px(20.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(if is_active { theme.accent } else { theme.border })
                                        .bg(if is_active { theme.accent } else { theme.surface })
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(is_active, |this| {
                                            this.child(render_icon(IconType::Check, theme.background, 12.0))
                                        }),
                                )
                        })),
                    ),
            )
            // Section 5: Context Window
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Context Window", theme))
                    .child(
                        render_group_card(theme).child(
                            render_card_row(
                                "Token Budget",
                                Some("Max tokens the model context holds before compaction triggers."),
                                {
                                    let current = self.ai_context_window;
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_1()
                                        .children(
                                            [4_000u32, 32_000, 64_000, 128_000, 200_000, 1_000_000]
                                                .into_iter()
                                                .map(|size| {
                                                    let label = if size >= 1_000_000 {
                                                        format!("{}M", size / 1_000_000)
                                                    } else {
                                                        format!("{}k", size / 1_000)
                                                    };
                                                    let is_sel = current == size;
                                                    div()
                                                        .id(SharedString::from(format!("ctx-win-{}", size)))
                                                        .px(px(8.))
                                                        .py(px(4.))
                                                        .rounded(px(5.))
                                                        .cursor(CursorStyle::PointingHand)
                                                        .bg(if is_sel { theme.accent } else { theme.surface_raised })
                                                        .border_1()
                                                        .border_color(if is_sel { theme.accent } else { theme.border })
                                                        .text_size(px(11.))
                                                        .font_weight(if is_sel { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                        .text_color(if is_sel { theme.background } else { theme.muted_strong })
                                                        .hover(move |s| if is_sel { s } else { s.bg(theme.hover) })
                                                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                            this.set_ai_context_window(size, cx);
                                                        }))
                                                        .child(label)
                                                })
                                        )
                                },
                                true,
                                theme,
                            ),
                        ),
                    ),
            )
    }

    fn render_tab_migration(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Migrate from Other Terminals", theme))
                    .child(
                        render_group_card(theme)
                            .when(!self.detected_external_configs.is_empty(), |this| {
                                this.children(self.detected_external_configs.iter().enumerate().map(|(idx, detected)| {
                                    let app = detected.app;
                                    let path = detected.path.clone();
                                    let label = detected.label.clone();
                                    let details = detected.details.clone();
                                    let is_last = idx == self.detected_external_configs.len() - 1 && self.import_status_message.is_none();

                                    render_card_row(
                                        label,
                                        Some(details),
                                        div()
                                            .px(px(10.))
                                            .py(px(5.))
                                            .rounded(px(5.))
                                            .bg(theme.accent)
                                            .hover(|s| s.opacity(0.9))
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.black)
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                this.import_external(app, path.clone(), cx);
                                            }))
                                            .child("Import & Adopt"),
                                        is_last,
                                        theme,
                                    )
                                }))
                            })
                            .when(self.detected_external_configs.is_empty(), |this| {
                                this.child(
                                    div()
                                        .p(px(20.))
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(theme.foreground)
                                                .child("No external terminal configurations detected"),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme.muted)
                                                .child("Configurations for Ghostty, Warp, Alacritty, iTerm2, and WezTerm are checked automatically."),
                                        ),
                                )
                            })
                            .when_some(self.import_status_message.as_ref(), |this, msg| {
                                this.child(
                                    div()
                                        .p(px(12.))
                                        .border_t_1()
                                        .border_color(theme.border)
                                        .bg(theme.accent.opacity(0.12))
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme.accent)
                                        .child(msg.clone()),
                                )
                            }),
                    ),
            )
    }

    fn render_tab_advanced(&self, theme: &Theme, _cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_6()
            // Section 1: Configuration Files
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Configuration Files", theme))
                    .child(
                        render_group_card(theme)
                            .child(render_card_row(
                                "Configuration Directory",
                                Some("~/.config/fastty"),
                                div()
                                    .px(px(10.))
                                    .py(px(5.))
                                    .rounded(px(6.))
                                    .bg(theme.surface_raised)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.hover))
                                    .cursor(CursorStyle::PointingHand)
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                        Self::open_config_folder();
                                    })
                                    .child("Open Folder ↗"),
                                false,
                                theme,
                            ))
                            .child(render_card_row(
                                "fastty.toml",
                                Some("Directly edit raw configuration file"),
                                div()
                                    .px(px(10.))
                                    .py(px(5.))
                                    .rounded(px(6.))
                                    .bg(theme.surface_raised)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.hover))
                                    .cursor(CursorStyle::PointingHand)
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                        Self::open_settings_file();
                                    })
                                    .child("Edit File ↗"),
                                true,
                                theme,
                            )),
                    ),
            )
            // Section 2: Application Info
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Application Information", theme))
                    .child(
                        render_group_card(theme)
                            .child(render_card_row(
                                "Version",
                                None::<&str>,
                                div()
                                    .px(px(7.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(theme.surface_raised)
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.muted_strong)
                                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                                false,
                                theme,
                            ))
                            .child(render_card_row(
                                "Rendering Engine",
                                None::<&str>,
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.muted_strong)
                                    .child("GPUI Metal / DirectX hardware-accelerated"),
                                false,
                                theme,
                            ))
                            .child(render_card_row(
                                "Platform Target",
                                None::<&str>,
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.muted_strong)
                                    .child(format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)),
                                true,
                                theme,
                            )),
                    ),
            )
    }
    fn handle_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_dragging_input {
            return;
        }
        if let Some(field) = self.active_input_field {
            match field {
                ActiveInputField::Search => {
                    self.search_input.last_bounds = self.search_input_bounds.get();
                    let col = self.search_input.index_for_position(ev.position, 11.5, 0.0, window);
                    self.search_input.update_drag(col);
                    self.search_query = self.search_input.text.clone();
                    cx.notify();
                }
                ActiveInputField::AiModel => {
                    self.ai_model_state.last_bounds = self.ai_model_bounds.get();
                    let col = self.ai_model_state.index_for_position(ev.position, 11.0, 8.0, window);
                    self.ai_model_state.update_drag(col);
                    self.ai_model_input = self.ai_model_state.text.clone();
                    cx.notify();
                }
                ActiveInputField::AiBaseUrl => {
                    self.ai_base_url_state.last_bounds = self.ai_base_url_bounds.get();
                    let col = self.ai_base_url_state.index_for_position(ev.position, 11.0, 8.0, window);
                    self.ai_base_url_state.update_drag(col);
                    self.ai_base_url_input = self.ai_base_url_state.text.clone();
                    cx.notify();
                }
                ActiveInputField::AiApiKey => {
                    self.ai_api_key_state.last_bounds = self.ai_api_key_bounds.get();
                    let col = self.ai_api_key_state.index_for_position(ev.position, 11.0, 8.0, window);
                    self.ai_api_key_state.update_drag(col);
                    self.ai_api_key_input = self.ai_api_key_state.text.clone();
                    cx.notify();
                }
                ActiveInputField::FontSearch => {
                    self.font_search_state.last_bounds = self.font_search_bounds.get();
                    let col = self.font_search_state.index_for_position(ev.position, 11.5, 0.0, window);
                    self.font_search_state.update_drag(col);
                    self.font_search_query = self.font_search_state.text.clone();
                    cx.notify();
                }
            }
        }
    }

    fn handle_mouse_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dragging_input {
            self.is_dragging_input = false;
            self.search_input.end_drag();
            self.ai_model_state.end_drag();
            self.ai_base_url_state.end_drag();
            self.ai_api_key_state.end_drag();
            self.font_search_state.end_drag();
            cx.notify();
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let win_ref = &*window;

        div()
            .track_focus(&self.focus_handle)
            .key_context("SettingsView")
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .w_full()
            .h_full()
            .bg(theme.window_fill())
            .text_color(theme.foreground)
            .flex()
            .flex_row()
            .overflow_hidden()
            .child(self.render_sidebar(&theme, win_ref, cx))
            .child(self.render_content_pane(&theme, win_ref, cx))
    }
}
