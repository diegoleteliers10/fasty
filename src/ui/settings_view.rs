use gpui::{
    App, WeakEntity, canvas, Context, CursorStyle, FontWeight, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, ScrollHandle, ScrollStrategy,
    SharedString, UniformListScrollHandle, Window, WindowControlArea, WindowHandle, div, list,
    ListAlignment, ListState, prelude::*, px, uniform_list, Animation, AnimationExt,
    DefiniteLength,
};
use std::time::Duration;
use icons::common::IconType;
use parking_lot::Mutex;
use crate::config::{self, Config};
use crate::ui::icons::{render_app_logo, render_icon};
use crate::ui::root_view::{available_system_fonts, open_path_or_url};
use crate::ui::theme::Theme;

pub static SETTINGS_WINDOW_HANDLE: Mutex<Option<WindowHandle<SettingsView>>> = Mutex::new(None);

/// One confetti particle for the config-import celebration ("software
/// should be fun"). Positions are viewport fractions; the overlay's
/// animation interpolates them with simple projectile physics from a
/// center burst, so the whole thing is stateless per frame.
#[derive(Clone)]
pub(crate) struct CelebrationParticle {
    x0: f32,
    y0: f32,
    vx: f32,
    vy: f32,
    g: f32,
    w: f32,
    h: f32,
    round: bool,
    color: Hsla,
}

impl CelebrationParticle {
    fn burst(seed: u64, theme: &Theme) -> Vec<Self> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut rnd = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as f32 / (1u64 << 31) as f32
        };
        let palette = [
            theme.red,
            theme.green,
            theme.yellow,
            theme.blue,
            theme.magenta,
            theme.cyan,
            theme.accent,
        ];
        (0..90)
            .map(|_| {
                let round = rnd() > 0.5;
                let size = 5.0 + rnd() * 4.0;
                Self {
                    x0: 0.5 + (rnd() - 0.5) * 0.05,
                    y0: 0.42 + (rnd() - 0.5) * 0.05,
                    vx: (rnd() - 0.5) * 0.95,
                    vy: -(0.22 + rnd() * 0.42),
                    g: 1.35 + rnd() * 0.55,
                    w: size,
                    h: if round { size } else { size + rnd() * 6.0 },
                    round,
                    color: palette[(rnd() * palette.len() as f32) as usize % palette.len()],
                }
            })
            .collect()
    }
}

/// Full font list for the Appearance combobox. Enumerating fonts is
/// expensive (DirectWrite COM walk on Windows, `reg query` / `fc-list`
/// subprocess elsewhere), so it runs once per process on a background
/// thread and every settings window reuses the result.
static SYSTEM_FONTS_CACHE: Mutex<Option<Vec<String>>> = Mutex::new(None);
#[cfg(target_os = "macos")]
static MONOSPACE_FONTS_CACHE: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// Union of the text-system families and the OS-level list, deduped
/// case-insensitively and sorted.
fn merge_font_lists(gpui_fonts: Vec<String>, os_fonts: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();
    for font in gpui_fonts.into_iter().chain(os_fonts) {
        let trimmed = font.trim().to_string();
        let lower = trimmed.to_lowercase();
        if !trimmed.is_empty() && !trimmed.starts_with('.') && seen.insert(lower) {
            merged.push(trimmed);
        }
    }
    merged.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    merged
}

/// Moves `family` to the front so the combobox highlights a valid entry.
fn pin_family_first(mut list: Vec<String>, family: &str) -> Vec<String> {
    if let Some(pos) = list.iter().position(|f| f.eq_ignore_ascii_case(family)) {
        let entry = list.remove(pos);
        list.insert(0, entry);
    }
    list
}

#[derive(Clone)]
pub struct ThemeCardInfo {
    pub name: String,
    pub label: String,
    pub bg_color: gpui::Hsla,
    pub surf_color: gpui::Hsla,
    pub acc_color: gpui::Hsla,
    pub fg_color: gpui::Hsla,
    pub muted_color: gpui::Hsla,
    pub ansi: [gpui::Hsla; 8],
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

    pub fn subtitle(&self) -> &'static str {
        match self {
            Self::General => "Layout, terminal behavior, notifications and updates.",
            Self::Appearance => "Theme, transparency and the type your shell is set in.",
            Self::Keyboard => "Pick a preset, then make any shortcut your own.",
            Self::Ai => "Choose who powers Fastty AI and what it is allowed to do.",
            Self::Migration => "Bring your setup over from another terminal in one click.",
            Self::Advanced => "Raw configuration and build details.",
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

/// Pending reassign confirmation when a captured combo is already bound.
#[derive(Debug, Clone, Copy)]
pub struct BindingConflict {
    pub target: crate::keybindings::Action,
    pub combo: crate::keybindings::KeyCombo,
    pub owner: crate::keybindings::Action,
}

const KEYBOARD_PRESETS: [(crate::keybindings::KeybindingPreset, &str, &str); 4] = [
    (
        crate::keybindings::KeybindingPreset::Default,
        "Default (Fastty)",
        "Standard modern shortcuts (⌘T, ⌘W, ⌘D, ⌘⌥Arrows)",
    ),
    (
        crate::keybindings::KeybindingPreset::Ghostty,
        "Ghostty",
        "Ghostty-compatible splits, tabs & nav",
    ),
    (
        crate::keybindings::KeybindingPreset::Tmux,
        "tmux",
        "tmux-compatible Ctrl+B leader & Alt arrows",
    ),
    (
        crate::keybindings::KeybindingPreset::ITerm2,
        "iTerm2",
        "macOS iTerm2 muscle memory shortcuts",
    ),
];

#[derive(Clone, Copy)]
enum KeyboardRow {
    PresetsHeader,
    Preset(usize),
    CustomHeader,
    Category(&'static str),
    Action(crate::keybindings::Action),
    Conflict(crate::keybindings::Action),
}

struct BindingRenderData {
    combos: std::collections::HashMap<crate::keybindings::Action, Vec<Vec<String>>>,
    customized: std::collections::HashSet<crate::keybindings::Action>,
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
    pub contrast_correction: bool,
    /// Confetti generation counter: bumped on every successful config
    /// import so the overlay remounts and replays its one-shot animation.
    pub(crate) celebration_generation: u64,
    pub(crate) celebration_particles: Vec<CelebrationParticle>,
    pub update_channel: String,
    pub update_checking: bool,
    pub update_status_msg: Option<String>,
    pub update_found: Option<crate::updater::ReleaseInfo>,
    pub update_ready: bool,
    pub tab_layout: crate::config::TabLayout,
    pub scrollback: usize,
    pub keybinding_preset: crate::keybindings::KeybindingPreset,
    pub ai_permission_mode: crate::ai::PermissionMode,
    pub ai_context_window: u32,
    pub ai_context_window_error: Option<String>,
    pub ai_provider: String,
    pub ai_model: Option<String>,
    pub detected_external_configs: Vec<crate::importer::DetectedConfig>,
    pub import_status_message: Option<String>,
    config_error: Option<String>,
    pub focus_handle: gpui::FocusHandle,
    pub window_scroll_handle: ScrollHandle,
    pub system_fonts: Vec<String>,
    pub monospace_fonts_only: bool,
    pub font_ligatures: bool,
    pub theme_cards: Vec<ThemeCardInfo>,
    pub active_tab: SettingsTab,
    pub search_query: String,
    pub search_input: crate::ui::TextInputState,
    pub active_input_field: Option<ActiveInputField>,
    pub font_combobox_open: bool,
    pub font_search_query: String,
    pub font_search_state: crate::ui::TextInputState,
    pub font_highlighted_index: usize,
    pub ai_model_dropdown_open: bool,
    pub ai_model_scroll_handle: ScrollHandle,
    keyboard_list_state: ListState,
    keyboard_row_count: usize,
    settings_scroll_target: f32,
    settings_scroll_offset: f32,
    settings_scroll_animating: bool,
    settings_scroll_last_frame: Option<std::time::Instant>,
    theme_carousel_scroll: ScrollHandle,
    binding_render_cache: Option<std::rc::Rc<BindingRenderData>>,
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
    /// UX3: hash of the on-disk config our in-memory state was synced from.
    /// Our own saves re-baseline it; any other difference means an external
    /// edit (main window action, hand-edited TOML) that our next save would
    /// silently clobber.
    pub synced_file_hash: Option<u64>,
    /// File hash the user explicitly dismissed via the stale badge: the
    /// badge stays hidden until the file changes again.
    pub acknowledged_hash: Option<u64>,
    /// Whether the stale badge is shown (recomputed throttled in render).
    pub stale: bool,
    pub last_stale_check: std::time::Instant,
    /// Action currently waiting for a keypress (press-to-capture editing).
    pub capturing_action: Option<crate::keybindings::Action>,
    /// Reassign confirmation for a captured combo owned by another action.
    pub binding_conflict: Option<BindingConflict>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        config::load_custom_themes();
        let loaded_config = crate::config::load_lenient();
        let auto_discover_opencode = loaded_config.ai.providers.get(&loaded_config.ai.default)
            .is_some_and(crate::ai::ProviderConfig::is_acp);
        let theme_name = loaded_config.theme.as_deref().unwrap_or("default").to_string();
        let theme = Theme::from_name(&theme_name);
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

        // Full font enumeration used to run right here, before the window's
        // first paint, and was the largest cause of the settings window
        // opening slowly or freezing on Windows. Until background discovery
        // lands, the combobox shows only the configured family.
        let cached_fonts = SYSTEM_FONTS_CACHE.lock().clone();
        #[cfg(target_os = "macos")]
        let cached_monospace_fonts = MONOSPACE_FONTS_CACHE.lock().clone();
        let fonts_cached = cached_fonts.is_some();
        let monospace_fonts_only = loaded_config.font.monospace_only;
        let font_ligatures = loaded_config.font.ligatures;
        let filter_fonts = |list: Vec<String>| {
            #[cfg(target_os = "macos")]
            {
                if monospace_fonts_only {
                    cached_monospace_fonts.unwrap_or_default()
                } else {
                    list
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = monospace_fonts_only;
                list
            }
        };
        let system_fonts = match cached_fonts {
            Some(list) => pin_family_first(filter_fonts(list), &font_family),
            None => vec![font_family.clone()],
        };

        let theme_cards: Vec<ThemeCardInfo> = crate::config::all_theme_names()
            .into_iter()
            .map(|name| {
                let t = Theme::from_name(&name);
        let label = crate::config::pretty_theme_label(&name).to_string();
                ThemeCardInfo {
                    name,
                    label,
                    bg_color: t.background,
                    surf_color: t.surface_raised,
                    acc_color: t.accent,
                    fg_color: t.foreground,
                    muted_color: t.muted_strong,
                    ansi: [
                        t.black, t.red, t.green, t.yellow, t.blue, t.magenta, t.cyan, t.white,
                    ],
                }
            })
            .collect();

        let ai_base_url_input = loaded_config
            .ai
            .providers
            .get(&loaded_config.ai.default)
            .map(|p| p.command().unwrap_or_else(|| p.base_url()).to_string())
            .unwrap_or_else(|| if loaded_config.ai.default == "opencode" { "opencode".to_string() } else { "http://localhost:11434/v1".to_string() });
        let ai_api_key_input = loaded_config
            .ai
            .providers
            .get(&loaded_config.ai.default)
            .and_then(|p| p.api_key().map(|s| s.to_string()))
            .unwrap_or_default();

        // Ollama detection probes localhost HTTP (800ms timeout) and falls
        // back to an `ollama list` subprocess with no timeout: cheap to hang
        // a painting thread for seconds, so it runs on a background thread
        // and fills the list when it lands.
        let ollama_base_url = loaded_config
            .ai
            .providers
            .get("ollama")
            .map(|p| p.base_url().to_string());

        let mut ai_model = loaded_config.ai.default_model.clone();
        if ai_model.is_none() {
            if let Some(prov) = loaded_config.ai.providers.get(&loaded_config.ai.default) {
                ai_model = prov.models().first().cloned();
            }
        }
        let ai_model_input = ai_model.clone().unwrap_or_default();

        let mut view = Self {
            opacity: loaded_config.opacity,
            cursor_blink: loaded_config.cursor.blink,
            copy_on_select: loaded_config.copy_on_select,
            contrast_correction: loaded_config.contrast_correction,
            celebration_generation: 0,
            celebration_particles: Vec::new(),
            update_channel: crate::updater::UpdateChannel::parse(&loaded_config.update_channel).as_str().to_string(),
            update_checking: false,
            update_status_msg: None,
            update_found: None,
            update_ready: false,
            tab_layout,
            scrollback,
            keybinding_preset,
            ai_permission_mode: loaded_config.ai.permission_mode,
            ai_context_window: loaded_config.ai.context_window,
            ai_context_window_error: None,
            ai_provider: loaded_config.ai.default.clone(),
            ai_model,
            detected_external_configs,
            import_status_message: None,
            config_error: crate::config::load_error(),
            config: loaded_config,
            theme,
            current_theme_name: theme_name,
            font_size,
            font_family,
            focus_handle,
            window_scroll_handle: ScrollHandle::new(),
            system_fonts,
            monospace_fonts_only,
            font_ligatures,
            theme_cards,
            active_tab: SettingsTab::General,
            search_query: String::new(),
            search_input: crate::ui::TextInputState::default(),
            active_input_field: None,
            font_combobox_open: false,
            font_search_query: String::new(),
            font_search_state: crate::ui::TextInputState::default(),
            font_highlighted_index: 0,
            ai_model_dropdown_open: false,
            ai_model_scroll_handle: ScrollHandle::new(),
            keyboard_list_state: ListState::new(0, ListAlignment::Top, px(240.)),
            keyboard_row_count: 0,
            settings_scroll_target: 0.0,
            settings_scroll_offset: 0.0,
            settings_scroll_animating: false,
            settings_scroll_last_frame: None,
            theme_carousel_scroll: ScrollHandle::new(),
            binding_render_cache: None,
            font_scroll_handle: UniformListScrollHandle::new(),
            font_trigger_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            font_search_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_base_url_state: crate::ui::TextInputState::new(ai_base_url_input.clone()),
            ai_base_url_input,
            ai_api_key_state: crate::ui::TextInputState::new(ai_api_key_input.clone()),
            ai_api_key_input,
            ai_model_state: crate::ui::TextInputState::new(ai_model_input.clone()),
            ai_model_input,
            detected_ollama_models: Vec::new(),
            ai_test_status: AiTestStatus::Idle,
            search_input_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_model_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_base_url_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_api_key_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            is_dragging_input: false,
            synced_file_hash: Self::config_file_hash(),
            acknowledged_hash: None,
            stale: false,
            last_stale_check: std::time::Instant::now(),
            capturing_action: None,
            binding_conflict: None,
        };

        if !fonts_cached {
            view.spawn_font_discovery(window, cx);
        }
        view.spawn_ollama_detection(ollama_base_url, cx);
        if auto_discover_opencode {
            view.test_ai_connection(window, cx);
        }
        view
    }

    /// Enumerates fonts once per process, off the open path: the OS-level
    /// list (subprocess) on a background thread, the text-system families
    /// on the UI thread after the window is already visible.
    fn spawn_font_discovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::bounded::<Vec<String>>(1);
        std::thread::Builder::new()
            .name("fastty-font-discovery".into())
            .spawn(move || {
                let _ = tx.send_blocking(available_system_fonts());
            })
            .ok();

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(os_fonts) = rx.recv().await {
                let _ = this.update_in(cx, |_this, _window, cx| {
                    let merged = merge_font_lists(cx.text_system().all_font_names(), os_fonts);
                    // Cache the unfiltered list so toggling back to "all fonts"
                    // is instant instead of re-running discovery.
                    #[cfg(target_os = "macos")]
                    {
                        let (tx, rx) = async_channel::bounded::<Vec<String>>(1);
                        let all_fonts = merged.clone();
                        std::thread::Builder::new()
                            .name("fastty-monospace-font-filter".into())
                            .spawn(move || {
                                let filtered = crate::font_discovery_macos::filter_monospace(merged);
                                *MONOSPACE_FONTS_CACHE.lock() = Some(filtered.clone());
                                *SYSTEM_FONTS_CACHE.lock() = Some(all_fonts);
                                let _ = tx.send_blocking(filtered);
                            })
                            .ok();
                        cx.spawn(async move |this, cx| {
                            if let Ok(filtered) = rx.recv().await {
                                let _ = this.update(cx, |this, cx| {
                                    let shown = if this.monospace_fonts_only {
                                        filtered
                                    } else {
                                        SYSTEM_FONTS_CACHE.lock().clone().unwrap_or_default()
                                    };
                                    this.system_fonts = pin_family_first(shown, &this.font_family);
                                    cx.notify();
                                });
                            }
                        })
                        .detach();
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        *SYSTEM_FONTS_CACHE.lock() = Some(merged.clone());
                        _this.system_fonts = pin_family_first(merged, &_this.font_family);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Detects installed Ollama models off the UI thread.
    fn spawn_ollama_detection(&mut self, base_url: Option<String>, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::bounded::<Vec<String>>(1);
        std::thread::Builder::new()
            .name("fastty-ollama-detect".into())
            .spawn(move || {
                let _ = tx.send_blocking(crate::ai::detect_installed_ollama_models(base_url.as_deref()));
            })
            .ok();

        cx.spawn(async move |this, cx| {
            if let Ok(models) = rx.recv().await {
                let _ = this.update(cx, |this, cx| this.apply_ollama_models(models, cx));
            }
        })
        .detach();
    }

    fn apply_ollama_models(&mut self, models: Vec<String>, cx: &mut Context<Self>) {
        self.detected_ollama_models = models;
        if self.ai_provider == "ollama" {
            let should_switch = self.ai_model_input.trim().is_empty()
                || !self.detected_ollama_models.contains(&self.ai_model_input);
            if should_switch {
                if let Some(first) = self.detected_ollama_models.first().cloned() {
                    self.ai_model = Some(first.clone());
                    self.ai_model_input = first.clone();
                    self.ai_model_state.set_text(first);
                }
            }
        }
        cx.notify();
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
        self.binding_render_cache = None;
        self.config = cfg.clone();        self.ai_test_status = AiTestStatus::Idle;
        self.opacity = cfg.opacity;
        self.scrollback = cfg.scrollback;
        self.cursor_blink = cfg.cursor.blink;
        self.copy_on_select = cfg.copy_on_select;
        self.contrast_correction = cfg.contrast_correction;
        self.update_channel = crate::updater::UpdateChannel::parse(&cfg.update_channel).as_str().to_string();
        self.tab_layout = cfg.tab_layout;
        self.font_size = cfg.font.size;
        self.font_ligatures = cfg.font.ligatures;
        self.monospace_fonts_only = cfg.font.monospace_only;
        if !cfg.font.family.is_empty() && cfg.font.family != "monospace" {
            self.font_family = cfg.font.family.clone();
        }
        #[cfg(target_os = "macos")]
        {
            let fonts = if self.monospace_fonts_only {
                MONOSPACE_FONTS_CACHE.lock().clone()
            } else {
                SYSTEM_FONTS_CACHE.lock().clone()
            };
            if let Some(fonts) = fonts {
                self.system_fonts = pin_family_first(fonts, &self.font_family);
            }
        }
        self.keybinding_preset = cfg.keybinding_preset.unwrap_or_default();
        self.ai_permission_mode = cfg.ai.permission_mode;
        self.ai_context_window = cfg.ai.context_window;
        self.ai_context_window_error = None;
        self.ai_provider = cfg.ai.default.clone();
        self.ai_model = cfg.ai.default_model.clone();
        if let Some(ref t) = cfg.theme {
            self.current_theme_name = t.clone();
        }
        self.theme = Theme::from_name(&self.current_theme_name);

        if let Some(prov) = cfg.ai.providers.get(&self.ai_provider) {
            self.ai_base_url_input = prov.command().unwrap_or_else(|| prov.base_url()).to_string();
            self.ai_api_key_input = prov.api_key().unwrap_or("").to_string();
        } else if self.ai_provider == "opencode" {
            self.ai_base_url_input = "opencode".to_string();
            self.ai_api_key_input.clear();
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
        // Our state now mirrors the caller's config: whatever is on disk at
        // this point is the new baseline (covers construction, reopen, and
        // explicit Reload from the stale badge).
        self.refresh_baseline();
        self.capturing_action = None;
        self.binding_conflict = None;

        cx.notify();
    }

    /// Hash of the active config file, if it exists.
    fn config_file_hash() -> Option<u64> {
        std::fs::read(crate::config::Config::get_active_config_path())
            .ok()
            .map(|b| crate::config::content_hash(&b))
    }

    /// Re-baselines divergence tracking to the current on-disk content.
    fn refresh_baseline(&mut self) {
        self.synced_file_hash = Self::config_file_hash();
        self.acknowledged_hash = None;
        self.stale = false;
    }

    fn save_config(&mut self, edit: impl FnOnce(&mut Config)) -> bool {
        match Config::update_default(edit) {
            Ok(config) => {
                self.config = config;
                self.opacity = self.config.opacity;
                self.scrollback = self.config.scrollback;
                self.cursor_blink = self.config.cursor.blink;
                self.copy_on_select = self.config.copy_on_select;
                self.contrast_correction = self.config.contrast_correction;
                self.update_channel = self.config.update_channel.clone();
                self.tab_layout = self.config.tab_layout;
                self.font_size = self.config.font.size;
                self.font_family = self.config.font.family.clone();
                self.font_ligatures = self.config.font.ligatures;
                self.monospace_fonts_only = self.config.font.monospace_only;
                self.keybinding_preset = self.config.keybinding_preset.unwrap_or_default();
                self.ai_permission_mode = self.config.ai.permission_mode;
                self.ai_context_window = self.config.ai.context_window;
                if let Some(name) = &self.config.theme {
                    self.current_theme_name = name.clone();
                }
                self.theme = Theme::from_name(&self.current_theme_name);
                self.config_error = None;
                self.refresh_baseline();
                true
            }
            Err(error) => {
                self.config_error = Some(format!("Cannot save settings: {error}"));
                false
            }
        }
    }

    /// Recomputes `stale`, throttled: at most one file read+hash per second.
    /// Called from `render`, which only runs on interaction.
    fn refresh_stale_state(&mut self) {
        const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        if self.last_stale_check.elapsed() < CHECK_INTERVAL {
            return;
        }
        self.last_stale_check = std::time::Instant::now();
        self.stale = Self::is_stale_state(
            Self::config_file_hash(),
            self.synced_file_hash,
            self.acknowledged_hash,
        );
    }

    /// Pure divergence decision: the view is stale when the file differs
    /// from the synced baseline, unless the user dismissed exactly this
    /// file content (any newer change re-shows the badge).
    fn is_stale_state(
        current: Option<u64>,
        baseline: Option<u64>,
        acknowledged: Option<u64>,
    ) -> bool {
        let diverged = current != baseline;
        let acked = acknowledged.is_some() && acknowledged == current;
        diverged && !acked
    }

    /// Reloads the on-disk config into the view (stale badge action).
    /// Unsaved in-window edits are replaced by the file content.
    fn reload_from_disk(&mut self, cx: &mut Context<Self>) {
        match crate::config::load_result() {
            Ok(cfg) => {
                self.config_error = None;
                self.sync_from_config(&cfg, cx);
            }
            Err(error) => {
                self.config_error = Some(format!("Cannot reload settings: {error}"));
                cx.notify();
            }
        }
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
            self.sync_settings_scroll_target();
        } else if q.contains("theme")
            || q.contains("color")
            || q.contains("font")
            || q.contains("size")
            || q.contains("opac")
            || q.contains("appear")
        {
            self.active_tab = SettingsTab::Appearance;
            self.sync_settings_scroll_target();
        } else if q.contains("key")
            || q.contains("short")
            || q.contains("bind")
            || q.contains("tmux")
            || q.contains("ghost")
        {
            self.active_tab = SettingsTab::Keyboard;
            self.sync_settings_scroll_target();
        } else if q.contains("tab")
            || q.contains("cursor")
            || q.contains("blink")
            || q.contains("scroll")
            || q.contains("copy")
            || q.contains("gen")
            || q.contains("notif")
            || q.contains("status")
        {
            self.active_tab = SettingsTab::General;
            self.sync_settings_scroll_target();
        } else if q.contains("migr")
            || q.contains("warp")
            || q.contains("iterm")
            || q.contains("alacr")
        {
            self.active_tab = SettingsTab::Migration;
            self.sync_settings_scroll_target();
        } else if q.contains("file")
            || q.contains("toml")
            || q.contains("fold")
            || q.contains("adv")
        {
            self.active_tab = SettingsTab::Advanced;
            self.sync_settings_scroll_target();
        }
        cx.notify();
    }

    fn catalog_model_selection(
        current: Option<&str>,
        models: &[String],
        variants: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<String> {
        current.filter(|current| models.iter().any(|model| {
            *current == model.as_str() || current.strip_prefix(&format!("{model}/"))
                .is_some_and(|variant| variants.get(model)
                    .is_some_and(|values| values.iter().any(|value| value == variant)))
        })).map(str::to_string).or_else(|| models.first().cloned())
    }

    fn save_ai_provider_config(&mut self, field: ActiveInputField) {
        let provider = self.ai_provider.clone();
        let value = match field {
            ActiveInputField::AiBaseUrl => self.ai_base_url_input.clone(),
            ActiveInputField::AiApiKey => self.ai_api_key_input.trim().to_string(),
            ActiveInputField::AiModel => self.ai_model_input.trim().to_string(),
            _ => return,
        };
        if self.save_config(move |latest| {
            match field {
                ActiveInputField::AiBaseUrl => {
                    if let Some(prov) = latest.ai.providers.get_mut(&provider) {
                        if prov.command().is_some() {
                            prov.set_command(value);
                        } else {
                            prov.set_base_url(value);
                        }
                    }
                }
                ActiveInputField::AiApiKey => {
                    if let Some(prov) = latest.ai.providers.get_mut(&provider) {
                        prov.set_api_key((!value.is_empty()).then_some(value));
                    }
                }
                ActiveInputField::AiModel => {
                    latest.ai.default = provider;
                    latest.ai.default_model = (!value.is_empty()).then_some(value);
                }
                _ => {}
            }
        }) {
            self.ai_model = self.config.ai.default_model.clone();
            self.ai_test_status = AiTestStatus::Idle;
        }
    }

    fn sync_active_field_text(&mut self, field: ActiveInputField, cx: &mut Context<Self>) {
        match field {
            ActiveInputField::Search => {
                self.search_query = self.search_input.text.clone();
                self.apply_search_filter(cx);
            }
            ActiveInputField::AiBaseUrl => {
                self.ai_base_url_input = self.ai_base_url_state.text.clone();
                self.save_ai_provider_config(field);
                cx.notify();
            }
            ActiveInputField::AiApiKey => {
                self.ai_api_key_input = self.ai_api_key_state.text.clone();
                self.save_ai_provider_config(field);
                cx.notify();
            }
            ActiveInputField::AiModel => {
                self.ai_model_input = self.ai_model_state.text.clone();
                self.save_ai_provider_config(field);
                cx.notify();
            }
            ActiveInputField::FontSearch => {
                self.font_search_query = self.font_search_state.text.clone();
                self.font_highlighted_index = 0;
                cx.notify();
            }
        }
    }

    fn input_state_mut(&mut self, field: ActiveInputField) -> &mut crate::ui::TextInputState {
        match field {
            ActiveInputField::Search => &mut self.search_input,
            ActiveInputField::AiBaseUrl => &mut self.ai_base_url_state,
            ActiveInputField::AiApiKey => &mut self.ai_api_key_state,
            ActiveInputField::AiModel => &mut self.ai_model_state,
            ActiveInputField::FontSearch => &mut self.font_search_state,
        }
    }

    fn select_all_active_input(&mut self, cx: &mut Context<Self>) {
        if let Some(field) = self.active_input_field {
            self.input_state_mut(field).select_all();
            cx.notify();
        }
    }

    fn copy_active_input(&mut self) {
        let Some(field) = self.active_input_field else {
            return;
        };
        if let Some(text) = self.input_state_mut(field).selected_text() {
            if let Some(mut clipboard) = crate::event_listener::clipboard_helper() {
                let _ = clipboard.set_text(text);
            }
        }
    }

    fn cut_active_input(&mut self, cx: &mut Context<Self>) {
        let Some(field) = self.active_input_field else {
            return;
        };
        if self.input_state_mut(field).selected_text().is_some() {
            self.copy_active_input();
            self.input_state_mut(field).delete_selection();
            self.sync_active_field_text(field, cx);
        }
    }

    fn paste_active_input(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(field) = self.active_input_field else {
            return false;
        };
        let Some(mut clipboard) = crate::event_listener::clipboard_helper() else {
            return false;
        };
        let Ok(text) = clipboard.get_text() else {
            return false;
        };
        self.input_state_mut(field).insert_str(text.trim());
        self.sync_active_field_text(field, cx);
        true
    }

    #[cfg(target_os = "macos")]
    fn handle_native_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.capturing_action.is_some() {
            self.handle_key_down(ev, window, cx);
            cx.stop_propagation();
            return;
        }
        if crate::ui::native_menu::dispatch_settings_shortcut(
            ev,
            self.active_input_field.is_some(),
            window,
            cx,
        ) {
            cx.stop_propagation();
            return;
        }
        let modifiers = &ev.keystroke.modifiers;
        let plain_key = !modifiers.platform && !modifiers.control && !modifiers.alt;
        let alt_gr = modifiers.control && modifiers.alt && !modifiers.platform;
        let key = ev.keystroke.key.to_lowercase();
        let text_key = ev.keystroke.key_char.as_deref().is_some_and(|text| {
            !text.is_empty() && !text.chars().any(char::is_control)
        }) || (key.chars().count() == 1 && !key.chars().any(char::is_control));
        let edit_key = matches!(key.as_str(),
            "backspace" | "delete" | "left" | "arrowleft" | "right" | "arrowright"
                | "up" | "arrowup" | "down" | "arrowdown" | "home" | "end"
                | "enter" | "return" | "escape" | "esc" | "tab" | "space"
        );
        let consume_input = self.active_input_field.is_some()
            && (ev.prefer_character_input || alt_gr || (plain_key && (text_key || edit_key)));
        self.handle_key_down(ev, window, cx);
        if consume_input {
            cx.stop_propagation();
        }
    }

    fn handle_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Press-to-capture binding editing: consume every key for the capture.
        if self.capturing_action.is_some() {
            let key = ev.keystroke.key.as_str();
            let mods = &ev.keystroke.modifiers;
            let is_escape = key.eq_ignore_ascii_case("escape") || key.eq_ignore_ascii_case("esc");
            let is_return = key.eq_ignore_ascii_case("return") || key.eq_ignore_ascii_case("enter");
            if (is_escape || is_return)
                && !mods.control
                && !mods.platform
                && !mods.alt
                && !mods.shift
            {
                self.cancel_binding_capture(cx);
                return;
            }
            let is_alt_gr = mods.control && !mods.platform && mods.alt;
            let is_ctrl = mods.control && !is_alt_gr;
            if let Some(combo) = crate::keybindings::combo_from_key(
                key,
                is_ctrl,
                mods.shift,
                mods.alt && !is_alt_gr,
                mods.platform,
            ) {
                let is_f_key = matches!(
                    combo.key,
                    crate::keybindings::NamedKey::F1
                        | crate::keybindings::NamedKey::F2
                        | crate::keybindings::NamedKey::F3
                        | crate::keybindings::NamedKey::F4
                        | crate::keybindings::NamedKey::F5
                        | crate::keybindings::NamedKey::F6
                        | crate::keybindings::NamedKey::F7
                        | crate::keybindings::NamedKey::F8
                        | crate::keybindings::NamedKey::F9
                        | crate::keybindings::NamedKey::F10
                        | crate::keybindings::NamedKey::F11
                        | crate::keybindings::NamedKey::F12
                );
                if combo.ctrl || combo.alt || combo.logo || is_f_key {
                    self.finish_binding_capture(combo, cx);
                }
            }
            // Modifier-only or non-f-key bare keys: keep waiting.
            return;
        }
        if ev.keystroke.key.eq_ignore_ascii_case("escape")
            || ev.keystroke.key.eq_ignore_ascii_case("esc")
        {
            if self.ai_model_dropdown_open {
                self.ai_model_dropdown_open = false;
                cx.notify();
                return;
            }
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
                self.select_all_active_input(cx);
                return;
            }

            // Copy: Cmd+C (macOS) or Ctrl+C (Linux/Windows)
            let is_copy = (cfg!(target_os = "macos") && is_plat && key_lower == "c")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "c");
            if is_copy {
                self.copy_active_input();
                return;
            }

            // Cut: Cmd+X (macOS) or Ctrl+X (Linux/Windows)
            let is_cut = (cfg!(target_os = "macos") && is_plat && key_lower == "x")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "x");
            if is_cut {
                self.cut_active_input(cx);
                return;
            }

            // Paste: Cmd+V (macOS) or Ctrl+V (Linux/Windows)
            let is_paste = (cfg!(target_os = "macos") && is_plat && key_lower == "v")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "v");
            if is_paste && self.paste_active_input(cx) {
                return;
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
            self.sync_settings_scroll_target();
                    self.active_input_field = None;
                    cx.notify();
                }
                "2" => {
                    self.active_tab = SettingsTab::Appearance;
            self.sync_settings_scroll_target();
                    self.active_input_field = None;
                    cx.notify();
                }
                "3" => {
                    self.active_tab = SettingsTab::Keyboard;
            self.sync_settings_scroll_target();
                    self.active_input_field = None;
                    cx.notify();
                }
                "4" => {
                    self.active_tab = SettingsTab::Ai;
            self.sync_settings_scroll_target();
                    self.active_input_field = None;
                    cx.notify();
                }
                "5" => {
                    self.active_tab = SettingsTab::Migration;
            self.sync_settings_scroll_target();
                    self.active_input_field = None;
                    cx.notify();
                }
                "6" => {
                    self.active_tab = SettingsTab::Advanced;
            self.sync_settings_scroll_target();
                    self.active_input_field = None;
                    cx.notify();
                }
                _ => {}
            }
        }
    }

    pub fn set_theme(&mut self, theme_name: &str, cx: &mut Context<Self>) {
        let name = theme_name.to_string();
        if self.save_config(move |latest| latest.theme = Some(name)) {
            self.current_theme_name = self.config.theme.clone().unwrap_or_default();
            self.theme = Theme::from_name(&self.current_theme_name);
        }
        cx.notify();
    }

    pub fn adjust_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        if self.save_config(|latest| latest.font.size = (latest.font.size + delta).clamp(8.0, 36.0)) {
            self.font_size = self.config.font.size;
        }
        cx.notify();
    }

    pub fn set_font_family(&mut self, family: &str, cx: &mut Context<Self>) {
        let family = family.to_string();
        if self.save_config(move |latest| latest.font.family = family) {
            self.font_family = self.config.font.family.clone();
        }
        cx.notify();
    }

    pub fn adjust_opacity(&mut self, opacity: f32, cx: &mut Context<Self>) {
        if self.save_config(|latest| latest.opacity = opacity.clamp(0.2, 1.0)) {
            self.opacity = self.config.opacity;
            self.theme = Theme::from_name(&self.current_theme_name);
        }
        cx.notify();
    }

    pub fn toggle_font_ligatures(&mut self, cx: &mut Context<Self>) {
        if self.save_config(|latest| latest.font.ligatures = !latest.font.ligatures) {
            self.font_ligatures = self.config.font.ligatures;
        }
        cx.notify();
    }

    pub fn toggle_monospace_fonts_only(&mut self, cx: &mut Context<Self>) {
        if !self.save_config(|latest| latest.font.monospace_only = !latest.font.monospace_only) {
            cx.notify();
            return;
        }
        self.monospace_fonts_only = self.config.font.monospace_only;
        // Refilter the cached enumeration rather than rediscovering fonts.
        #[cfg(target_os = "macos")]
        {
            let shown = if self.monospace_fonts_only {
                MONOSPACE_FONTS_CACHE.lock().clone()
            } else {
                SYSTEM_FONTS_CACHE.lock().clone()
            };
            if let Some(shown) = shown {
                self.system_fonts = pin_family_first(shown, &self.font_family);
            }
        }
        cx.notify();
    }

    pub fn toggle_cursor_blink(&mut self, cx: &mut Context<Self>) {
        if self.save_config(|latest| latest.cursor.blink = !latest.cursor.blink) {
            self.cursor_blink = self.config.cursor.blink;
        }
        cx.notify();
    }

    pub fn toggle_copy_on_select(&mut self, cx: &mut Context<Self>) {
        if self.save_config(|latest| latest.copy_on_select = !latest.copy_on_select) {
            self.copy_on_select = self.config.copy_on_select;
        }
        cx.notify();
    }

    pub fn toggle_contrast_correction(&mut self, cx: &mut Context<Self>) {
        if self.save_config(|latest| latest.contrast_correction = !latest.contrast_correction) {
            self.contrast_correction = self.config.contrast_correction;
        }
        cx.notify();
    }

    pub fn toggle_notify_on_ai_status(&mut self, cx: &mut Context<Self>) {
        self.save_config(|latest| latest.notify_on_ai_status = !latest.notify_on_ai_status);
        cx.notify();
    }

    pub fn toggle_notify_on_program_status(&mut self, cx: &mut Context<Self>) {
        self.save_config(|latest| latest.notify_on_program_status = !latest.notify_on_program_status);
        cx.notify();
    }

    /// Fire the confetti: regenerate particles from the current theme and
    /// bump the generation so the animation element remounts and replays.
    fn start_celebration(&mut self) {
        self.celebration_generation += 1;
        let theme = self.theme;
        self.celebration_particles =
            CelebrationParticle::burst(self.celebration_generation, &theme);
    }

    /// Full-window, click-through confetti overlay. Stateless physics:
    /// every frame recomputes positions from the animation delta, and the
    /// particles fade out as `delta` approaches 1.
    fn render_celebration(&self) -> impl IntoElement {
        let particles = self.celebration_particles.clone();
        let gen = self.celebration_generation;
        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .with_animation(
                format!("confetti-{gen}"),
                Animation::new(Duration::from_millis(1800)),
                move |el, delta| {
                    let mut out = el;
                    for p in &particles {
                        let x = p.x0 + p.vx * delta;
                        let y = p.y0 + p.vy * delta + 0.5 * p.g * delta * delta;
                        let opacity = (1.0 - delta).max(0.0).powi(2);
                        let piece = div()
                            .absolute()
                            .top(DefiniteLength::Fraction(y))
                            .left(DefiniteLength::Fraction(x))
                            .opacity(opacity)
                            .bg(p.color)
                            .when(p.round, |piece| piece.rounded_full())
                            .when(!p.round, |piece| piece.rounded(px(1.)))
                            .w(px(p.w))
                            .h(px(p.h));
                        out = out.child(piece);
                    }
                    out
                },
            )
    }

    pub fn set_update_channel(&mut self, channel: &str, cx: &mut Context<Self>) {
        let normalized = crate::updater::UpdateChannel::parse(channel).as_str().to_string();
        if self.save_config(move |latest| latest.update_channel = normalized) {
            self.update_channel = self.config.update_channel.clone();
        }
        cx.notify();
    }

    /// Forced update check from the Settings Updates section. Runs the
    /// blocking network call off the UI thread and reports the result inline
    /// (available / up to date / error) instead of failing silently.
    pub fn check_updates_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_checking {
            return;
        }
        self.update_checking = true;
        self.update_ready = false;
        self.update_found = None;
        self.update_status_msg = Some("Checking for updates...".to_string());
        cx.notify();

        let channel = crate::updater::UpdateChannel::parse(&self.update_channel);
        let (tx, rx) = async_channel::bounded::<Result<Option<crate::updater::ReleaseInfo>, String>>(1);
        std::thread::Builder::new()
            .name("fastty-update-check".into())
            .spawn(move || {
                let res = crate::updater::check_for_update_sync_with(channel, true)
                    .map_err(|e| e.to_string());
                let _ = tx.send_blocking(res);
            })
            .ok();

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(res) = rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    this.update_checking = false;
                    match res {
                        Ok(Some(release)) => {
                            this.update_status_msg = if let Some(reason) = &release.self_update_blocked_reason {
                                Some(format!("Fastty v{} is available.\n{}", release.version, reason))
                            } else {
                                Some(format!("Fastty v{} is available.", release.version))
                            };
                            this.update_found = Some(release);
                        }
                        Ok(None) => {
                            this.update_status_msg = Some(format!(
                                "Fastty v{} is up to date.",
                                env!("CARGO_PKG_VERSION")
                            ));
                        }
                        Err(e) => {
                            this.update_status_msg = Some(format!("Update check failed: {e}"));
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Single entry point for the Settings update button. Branches on state:
    /// restart when staged, open the download page when self-update is
    /// blocked, download otherwise, or check when nothing is known yet.
    pub fn update_primary_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_checking {
            return;
        }
        if self.update_ready {
            crate::updater::relaunch_fastty();
            return;
        }
        if let Some(release) = self.update_found.clone() {
            if release.self_update_blocked_reason.is_some() {
                open_path_or_url(&release.release_url);
                return;
            }
            self.update_checking = true;
            self.update_status_msg = Some(format!("Downloading Fastty v{}...", release.version));
            cx.notify();
            let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
            std::thread::Builder::new()
                .name("fastty-update-download".into())
                .spawn(move || {
                    let res = crate::updater::apply_update_sync(&release).map_err(|e| e.to_string());
                    let _ = tx.send_blocking(res);
                })
                .ok();
            cx.spawn_in(window, async move |this, cx| {
                if let Ok(res) = rx.recv().await {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        this.update_checking = false;
                        match res {
                            Ok(()) => {
                                this.update_ready = true;
                                this.update_status_msg = Some("Update downloaded and verified. Restart Fastty to switch to the new version.".to_string());
                            }
                            Err(e) => {
                                this.update_status_msg = Some(format!("Update failed: {e}"));
                            }
                        }
                        cx.notify();
                    });
                }
            })
            .detach();
            return;
        }
        self.check_updates_now(window, cx);
    }

    /// Dismisses the release found by the Settings check so neither Settings
    /// nor the main window keeps offering it.
    pub fn skip_found_update(&mut self, cx: &mut Context<Self>) {
        if let Some(release) = self.update_found.take() {
            crate::updater::dismiss_version(&release.tag_name);
            self.update_status_msg = Some(format!(
                "Skipped v{}. Check again to see future releases.",
                release.version
            ));
        }
        self.update_ready = false;
        cx.notify();
    }

    pub fn set_tab_layout(&mut self, layout: crate::config::TabLayout, cx: &mut Context<Self>) {
        if self.save_config(move |latest| latest.tab_layout = layout) {
            self.tab_layout = self.config.tab_layout;
        }
        cx.notify();
    }

    pub fn set_scrollback(&mut self, scrollback: usize, cx: &mut Context<Self>) {
        if self.save_config(move |latest| latest.scrollback = scrollback) {
            self.scrollback = self.config.scrollback;
        }
        cx.notify();
    }

    pub fn set_keybinding_preset(&mut self, preset: crate::keybindings::KeybindingPreset, cx: &mut Context<Self>) {
        self.persist_bindings(|latest| latest.keybinding_preset = Some(preset), cx);
        self.capturing_action = None;
        self.binding_conflict = None;
    }

    fn persist_bindings(&mut self, edit: impl FnOnce(&mut Config), cx: &mut Context<Self>) {
        self.binding_render_cache = None;
        if self.save_config(edit) {
            self.keybinding_preset = self.config.keybinding_preset.unwrap_or_default();
            crate::keybindings::init_resolver(
                self.config.keybindings.clone(),
                self.config.keybinding_preset,
            );
            crate::keybindings::init_key_modes(self.config.key_modes.clone());
        }
        cx.notify();
    }

    fn binding_render_data(&mut self) -> std::rc::Rc<BindingRenderData> {
        if let Some(cache) = &self.binding_render_cache {
            return cache.clone();
        }
        let effective = crate::keybindings::effective_resolver(
            self.keybinding_preset,
            &self.config.keybindings,
        );
        let customized =
            crate::keybindings::customized_actions(&self.config.keybindings, self.keybinding_preset);
        let mut combos = std::collections::HashMap::new();
        for action in crate::keybindings::Action::all() {
            let keycaps = effective
                .combos_for(action)
                .iter()
                .map(crate::keybindings::combo_keycaps)
                .collect();
            combos.insert(action, keycaps);
        }
        let data = std::rc::Rc::new(BindingRenderData { combos, customized });
        self.binding_render_cache = Some(data.clone());
        data
    }

    pub fn start_binding_capture(
        &mut self,
        action: crate::keybindings::Action,
        cx: &mut Context<Self>,
    ) {
        if self.capturing_action == Some(action) {
            self.capturing_action = None;
            self.binding_conflict = None;
        } else {
            self.capturing_action = Some(action);
            self.binding_conflict = None;
            self.active_input_field = None;
            self.font_combobox_open = false;
        }
        cx.notify();
    }

    fn apply_action_binding(
        latest: &mut Config,
        target: crate::keybindings::Action,
        combo: crate::keybindings::KeyCombo,
    ) {
        let id = target.binding_id();
        let target_combo_str = combo.to_string();
        latest.keybindings.retain(|k, v| v != &id || k == &target_combo_str);
        let preset = latest.keybinding_preset.unwrap_or_default();
        for pc in crate::keybindings::KeyBindingResolver::preset_combos(preset, target) {
            if pc != combo {
                latest.keybindings.insert(pc.to_string(), "none".to_string());
            }
        }
        latest.keybindings.insert(target_combo_str, id);
    }

    fn finish_binding_capture(
        &mut self,
        combo: crate::keybindings::KeyCombo,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.capturing_action.take() else {
            return;
        };
        let mut conflict = None;
        self.persist_bindings(|latest| {
            let effective = crate::keybindings::effective_resolver(
                latest.keybinding_preset.unwrap_or_default(),
                &latest.keybindings,
            );
            match effective.resolve(&combo) {
                Some(owner) if owner != target => {
                    conflict = Some(BindingConflict { target, combo, owner });
                }
                _ => Self::apply_action_binding(latest, target, combo),
            }
        }, cx);
        self.binding_conflict = conflict;
    }

    pub fn confirm_binding_conflict(&mut self, cx: &mut Context<Self>) {
        if let Some(conflict) = self.binding_conflict.take() {
            self.persist_bindings(|latest| {
                Self::apply_action_binding(latest, conflict.target, conflict.combo);
            }, cx);
        }
    }

    pub fn cancel_binding_capture(&mut self, cx: &mut Context<Self>) {
        self.capturing_action = None;
        self.binding_conflict = None;
        cx.notify();
    }

    pub fn unbind_action(
        &mut self,
        action: crate::keybindings::Action,
        cx: &mut Context<Self>,
    ) {
        self.binding_conflict = None;
        self.persist_bindings(|latest| {
            let effective = crate::keybindings::effective_resolver(
                latest.keybinding_preset.unwrap_or_default(),
                &latest.keybindings,
            );
            let id = action.binding_id();
            latest.keybindings.retain(|_, value| value != &id);
            for combo in effective.combos_for(action) {
                latest.keybindings.insert(combo.to_string(), "none".to_string());
            }
        }, cx);
    }

    pub fn reset_action_bindings_ui(
        &mut self,
        action: crate::keybindings::Action,
        cx: &mut Context<Self>,
    ) {
        self.binding_conflict = None;
        self.persist_bindings(|latest| {
            crate::keybindings::reset_action_bindings(
                &mut latest.keybindings,
                latest.keybinding_preset.unwrap_or_default(),
                action,
            );
        }, cx);
    }

    pub fn reset_all_bindings(&mut self, cx: &mut Context<Self>) {
        self.capturing_action = None;
        self.binding_conflict = None;
        self.persist_bindings(|latest| latest.keybindings.clear(), cx);
    }

    pub fn set_ai_permission_mode(&mut self, mode: crate::ai::PermissionMode, cx: &mut Context<Self>) {
        if self.save_config(|latest| latest.ai.permission_mode = mode) {
            self.ai_permission_mode = self.config.ai.permission_mode;
        }
        cx.notify();
    }

    pub fn set_ai_context_window(&mut self, tokens: u32, cx: &mut Context<Self>) {
        let clamped = tokens.clamp(4_000, 2_000_000);
        if self.save_config(|latest| latest.ai.context_window = clamped) {
            self.ai_context_window = self.config.ai.context_window;
            self.ai_context_window_error = None;
            crate::config::set_ai_context_window(self.ai_context_window);
        } else {
            self.ai_context_window_error = self.config_error.clone();
        }
        cx.notify();
    }

    pub fn set_ai_provider(
        &mut self,
        provider: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detected_model = self.detected_ollama_models.first().cloned();
        if !self.save_config(|latest| {
            latest.ai.default = provider.to_string();
            if provider == "opencode" {
                latest.ai.providers.entry("opencode".to_string()).or_insert_with(|| {
                    crate::ai::ProviderConfig::Opencode {
                        command: "opencode".to_string(),
                        models: Vec::new(),
                        variants: std::collections::HashMap::new(),
                    }
                });
            } else if provider == "claude-acp" {
                latest.ai.providers.entry("claude-acp".to_string()).or_insert_with(|| {
                    crate::ai::ProviderConfig::Acp {
                        command: "claude-agent-acp".to_string(),
                        args: Vec::new(),
                        models: Vec::new(),
                        variants: std::collections::HashMap::new(),
                    }
                });
            } else if provider == "antigravity-acp" {
                latest.ai.providers.entry("antigravity-acp".to_string()).or_insert_with(|| {
                    crate::ai::ProviderConfig::Acp {
                        command: "agy_acp_server".to_string(),
                        args: Vec::new(),
                        models: Vec::new(),
                        variants: std::collections::HashMap::new(),
                    }
                });
            }
            latest.ai.default_model = latest.ai.providers.get(provider)
                .and_then(|entry| entry.models().first().cloned());
            if provider == "ollama" {
                latest.ai.default_model = detected_model.or_else(|| latest.ai.default_model.clone());
            }
        }) {
            cx.notify();
            return;
        }
        self.ai_provider = self.config.ai.default.clone();
        let prov_info = self
            .config
            .ai
            .providers
            .get(provider)
            .map(|p| (p.command().unwrap_or_else(|| p.base_url()).to_string(), p.api_key().unwrap_or("").to_string(), p.models().first().cloned()));
        if let Some((base_url, api_key, first_prov_model)) = prov_info {
            self.ai_base_url_input = base_url.clone();
            self.ai_base_url_state.set_text(self.ai_base_url_input.clone());
            self.ai_api_key_input = api_key;
            self.ai_api_key_state.set_text(self.ai_api_key_input.clone());

            if provider == "ollama" {
                if self.detected_ollama_models.is_empty() {
                    self.spawn_ollama_detection(Some(base_url), cx);
                }
                let chosen = self.detected_ollama_models.first().cloned().or(first_prov_model);
                self.ai_model = chosen.clone();
                self.ai_model_input = chosen.clone().unwrap_or_default();
                self.ai_model_state.set_text(self.ai_model_input.clone());
            } else {
                self.ai_model = first_prov_model.clone();
                self.ai_model_input = first_prov_model.clone().unwrap_or_default();
                self.ai_model_state.set_text(self.ai_model_input.clone());
            }
        }
        self.ai_test_status = AiTestStatus::Idle;
        cx.notify();
        if self.config.ai.providers.get(provider).is_some_and(crate::ai::ProviderConfig::is_acp) {
            self.test_ai_connection(window, cx);
        }
    }

    pub fn reset_ai_base_url(&mut self, cx: &mut Context<Self>) {
        let default_url = match self.ai_provider.as_str() {
            "ollama" => "http://localhost:11434/v1",
            "anthropic" => "https://api.anthropic.com/v1",
            "openai" => "https://api.openai.com/v1",
            "openrouter" => "https://openrouter.ai/api/v1",
            "lm-studio" => "http://localhost:1234/v1",
            "opencode" => "opencode",
            "claude-acp" => "claude-agent-acp",
            "antigravity-acp" => "agy_acp_server",
            _ => "http://localhost:11434/v1",
        };
        self.ai_base_url_input = default_url.to_string();
        self.ai_base_url_state.set_text(self.ai_base_url_input.clone());
        self.save_ai_provider_config(ActiveInputField::AiBaseUrl);
        cx.notify();
    }

    pub fn clear_ai_api_key(&mut self, cx: &mut Context<Self>) {
        self.ai_api_key_input.clear();
        self.ai_api_key_state.clear();
        self.save_ai_provider_config(ActiveInputField::AiApiKey);
        cx.notify();
    }

    pub fn set_ai_model(&mut self, model: &str, cx: &mut Context<Self>) {
        let trimmed = model.trim().to_string();
        let provider = self.ai_provider.clone();
        if self.save_config(|latest| {
            latest.ai.default_model = Some(trimmed.clone());
            latest.ai.default = provider;
        }) {
            self.ai_model = self.config.ai.default_model.clone();
            self.ai_model_input = trimmed.clone();
            self.ai_model_state.set_text(trimmed);
            self.ai_test_status = AiTestStatus::Idle;
        }
        cx.notify();
    }

    pub fn refresh_ollama_models(&mut self, cx: &mut Context<Self>) {
        self.spawn_ollama_detection(Some(self.ai_base_url_input.clone()), cx);
        cx.notify();
    }

    pub fn clear_ai_model(&mut self, cx: &mut Context<Self>) {
        if self.save_config(|latest| latest.ai.default_model = None) {
            self.ai_model_input.clear();
            self.ai_model_state.clear();
            self.ai_model = None;
            self.ai_test_status = AiTestStatus::Idle;
        }
        cx.notify();
    }

    pub fn test_ai_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_test_status == AiTestStatus::Testing {
            return;
        }
        self.ai_test_status = AiTestStatus::Testing;
        cx.notify();

        if let Some(provider) = self.config.ai.providers.get(&self.ai_provider).filter(|provider| provider.is_acp()) {
            let provider_name = self.ai_provider.clone();
            let command = provider.command().unwrap_or("opencode").to_string();
            let args = provider.acp_args().unwrap_or_else(|| vec!["acp".to_string()]);
            let args_for_check = args.clone();
            let opencode_config = matches!(provider, crate::ai::ProviderConfig::Opencode { .. });
            let command_for_check = command.clone();
            let (tx, rx) = async_channel::bounded::<Result<(std::time::Duration, Vec<String>, std::collections::HashMap<String, Vec<String>>), String>>(1);
            let worker = std::thread::Builder::new().name("fastty-acp-discovery".into()).spawn(move || {
                let start = std::time::Instant::now();
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let result = crate::ai::opencode::discover_catalog_with_args(command, &args, cwd, opencode_config)
                    .map(|(models, variants)| (start.elapsed(), models, variants))
                    .map_err(|error| format!("{error:#}"));
                let _ = tx.send_blocking(result);
            });
            if let Err(error) = worker {
                self.ai_test_status = AiTestStatus::Failed(error.to_string());
                cx.notify();
                return;
            }
            cx.spawn_in(window, async move |this, cx| {
                if let Ok(result) = rx.recv().await {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        let current_command = this.config.ai.providers.get(&provider_name)
                            .and_then(crate::ai::ProviderConfig::command).unwrap_or("opencode");
                        let current_args = this.config.ai.providers.get(&provider_name)
                            .and_then(crate::ai::ProviderConfig::acp_args);
                        if this.ai_provider != provider_name
                            || current_command != command_for_check
                            || current_args.as_deref() != Some(args_for_check.as_slice()) {
                            return;
                        }
                        match result {
                            Ok((duration, models, variants)) => {
                                let mut accepted = false;
                                let saved = this.save_config(|latest| {
                                    if latest.ai.default != provider_name {
                                        return;
                                    }
                                    let selected = Self::catalog_model_selection(
                                        latest.ai.default_model.as_deref(), &models, &variants,
                                    );
                                    let Some(provider) = latest.ai.providers.get_mut(&provider_name) else {
                                        return;
                                    };
                                    if provider.command() != Some(command_for_check.as_str())
                                        || provider.acp_args().as_deref() != Some(args_for_check.as_slice()) {
                                        return;
                                    }
                                    provider.set_acp_catalog(models, variants);
                                    if selected.is_some() {
                                        latest.ai.default_model = selected;
                                    }
                                    accepted = true;
                                });
                                if saved && accepted {
                                    this.ai_model = this.config.ai.default_model.clone();
                                    this.ai_model_input = this.ai_model.clone().unwrap_or_default();
                                    this.ai_model_state.set_text(this.ai_model_input.clone());
                                    this.ai_test_status = AiTestStatus::Success(duration);
                                } else if !saved {
                                    this.ai_test_status = AiTestStatus::Failed(
                                        this.config_error.clone().unwrap_or_else(|| "Cannot save settings".to_string()),
                                    );
                                } else {
                                    this.ai_test_status = AiTestStatus::Idle;
                                }
                            }
                            Err(error) => this.ai_test_status = AiTestStatus::Failed(error),
                        }
                        cx.notify();
                    });
                }
            }).detach();
            return;
        }

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
        let snapshot = self.config.clone();
        match crate::importer::import_external_config(app, &path, &mut self.config) {
            Ok(msg) => {
                let imported = std::mem::replace(&mut self.config, snapshot);
                if self.save_config(move |latest| *latest = imported) {
                    self.import_status_message = Some(msg);
                    crate::keybindings::init_resolver(
                        self.config.keybindings.clone(),
                        self.config.keybinding_preset,
                    );
                    crate::keybindings::init_key_modes(self.config.key_modes.clone());
                    // "Software should be fun": the migration moment gets its
                    // celebration, a nod to the config-import confetti that
                    // made the rounds in September.
                    self.start_celebration();
                } else {
                    self.import_status_message = self.config_error.clone();
                }
                cx.notify();
            }
            Err(e) => {
                self.config = snapshot;
                self.config_error = Some(format!("Cannot import settings: {e}"));
                self.import_status_message = self.config_error.clone();
                cx.notify();
            }
        }
    }

    pub fn open_settings_file() {
        let path = crate::config::Config::get_active_config_path();
        if let Err(error) = crate::config::seed_config_stub(&path) {
            eprintln!("Fastty could not seed the settings file: {error}");
        }
        open_path_or_url(&path);
    }

    pub fn open_config_folder() {
        let path = Config::get_active_config_path();
        if let Some(config_dir) = path.parent() {
            let _ = std::fs::create_dir_all(config_dir);
            open_path_or_url(config_dir);
        }
    }
}

fn render_toggle_switch(is_on: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(if is_on { "toggle-on" } else { "toggle-off" })
        .flex()
        .items_center()
        .w(px(46.))
        .h(px(26.))
        .rounded_full()
        .bg(if is_on { theme.accent } else { theme.surface_raised })
        .p(px(3.))
        .cursor(CursorStyle::PointingHand)
        .child(
            div()
                .w(px(20.))
                .h(px(20.))
                .rounded_full()
                .bg(if is_on { theme.white } else { theme.muted_strong })
                .when(is_on, |this| this.ml(px(20.))),
        )
}

fn render_section_header(title: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .text_size(px(13.))
        .text_color(theme.muted)
        .mb(px(10.))
        .child(format!("# {}", title.into()))
}

/// Plain group used by the General page: rows sit on the page background.
fn render_plain_group() -> gpui::Div {
    div().flex().flex_col()
}

fn render_plain_row(
    title: impl Into<SharedString>,
    subtitle: Option<impl Into<SharedString>>,
    control: impl IntoElement,
    _is_last: bool,
    theme: &Theme,
) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(0.))
        .py(px(16.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .mr(px(12.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(px(14.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child(title.into()),
                )
                .when_some(subtitle, |this, sub| {
                    this.child(
                        div()
                            .text_size(px(12.5))
                            .text_color(theme.muted)
                            .child(sub.into()),
                    )
                }),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w_full()
                .min_w_0()
                .child(control),
        )
}

fn render_plain_row_inset(
    title: impl Into<SharedString>,
    subtitle: Option<impl Into<SharedString>>,
    control: impl IntoElement,
    _is_last: bool,
    theme: &Theme,
) -> gpui::Div {
    render_plain_row(title, subtitle, control, false, theme).px(px(14.))
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
            .w(px(250.))
            .min_w(px(230.))
            .max_w(px(270.))
            .h_full()
            .bg(theme.main_bg)
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
            // Brand
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .mb(px(18.))
                    .child(render_app_logo(20.0))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.foreground)
                            .child("Fastty"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                    ),
            )
            // Search box
            .child({
                let is_search_active = self.active_input_field == Some(ActiveInputField::Search);
                let query = self.search_query.clone();
                div()
                    .id("settings-search-bar")
                    .mx(px(16.))
                    .mb(px(18.))
                    .h(px(36.))
                    .rounded(px(8.))
                    .bg(theme.surface_raised)
                    .border_1()
                    .border_color(if is_search_active { theme.accent } else { theme.border })
                    .px(px(12.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                        this.active_input_field = Some(ActiveInputField::Search);
                        this.search_input.last_bounds = this.search_input_bounds.get();
                        let col = this.search_input.index_for_position(ev.position, 12.5, 0.0, window);
                        this.search_input.start_drag(col);
                        this.is_dragging_input = true;
                        cx.notify();
                    }))
                    .child(render_icon(IconType::Search, if is_search_active { theme.accent } else { theme.muted }, 14.0))
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
                                            .text_size(px(12.5))
                                            .text_color(theme.muted)
                                            .child("Search settings"),
                                    )
                            } else {
                                crate::ui::text_input::render_line_spans(
                                    &query,
                                    0,
                                    if is_search_active { Some(self.search_input.cursor) } else { None },
                                    self.search_input.selection,
                                    12.5,
                                    theme,
                                    Some(window),
                                )
                            })
                    )
            })
            .child(
                div()
                    .px(px(16.))
                    .pb(px(10.))
                    .text_size(px(10.5))
                    .text_color(theme.muted)
                    .child("PREFERENCES"),
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
                    .gap(px(4.))
                    .children(tabs.into_iter().map(|(tab, badge)| {
                        let is_active = self.active_tab == tab;
                        div()
                            .id(SharedString::from(format!("tab-item-{:?}", tab)))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .h(px(38.))
                            .px(px(8.))
                            .rounded(px(8.))
                            .bg(if is_active { theme.surface_raised } else { theme.main_bg })
                            .hover(move |s| if !is_active { s.bg(theme.hover) } else { s })
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                this.active_tab = tab;
                                this.sync_settings_scroll_target();
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
                                        15.0,
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .font_weight(if is_active { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                                            .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                            .child(tab.label()),
                                    ),
                            )
                            .when_some(badge, |this, badge_text| {
                                if tab == SettingsTab::Ai {
                                    this.child(render_icon(
                                        IconType::Sparkles,
                                        theme.accent,
                                        13.0,
                                    ))
                                } else {
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
                                }
                            })
                    })),
            )
    }

    fn render_content_pane(&mut self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("settings-content-pane")
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.main_bg)
            .overflow_hidden()
            // Top bar: drag region and the Esc hint
            .child(
                div()
                    .id("settings-top-bar")
                    .h(px(42.))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(20.))
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
                            .gap(px(6.))
                            .child(
                                div()
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(theme.surface_raised)
                                    .text_size(px(10.5))
                                    .text_color(theme.muted_strong)
                                    .child("esc"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.muted)
                                    .child("to close"),
                            )
                            .when(!cfg!(target_os = "macos"), |this| {
                                this.child(
                                    div()
                                        .id("settings-win-close")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(24.))
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
            .when_some(self.config_error.as_ref(), |this, error| {
                this.child(
                    div()
                        .w_full()
                        .px(px(56.))
                        .py(px(8.))
                        .bg(theme.red.opacity(0.10))
                        .border_b_1()
                        .border_color(theme.red.opacity(0.35))
                        .text_size(px(11.5))
                        .text_color(theme.red)
                        .child(error.clone()),
                )
            })
            // Scrollable Content
            // UX3: the config file changed outside this window (main-window
            // action or hand-edited TOML). Offer Reload before the next save
            // here silently overwrites that edit.
            .when(self.stale, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .px(px(56.))
                        .py(px(8.))
                        .bg(theme.accent.opacity(0.10))
                        .border_b_1()
                        .border_color(theme.accent.opacity(0.35))
                        .child(div().text_size(px(12.)).child("⚠"))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.5))
                                .text_color(theme.foreground)
                                .overflow_hidden()
                                .child(
                                    "Settings on disk changed — this view may be outdated. Reload to see it.",
                                ),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .bg(theme.surface_raised)
                                .text_size(px(11.))
                                .text_color(theme.foreground)
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _ev, _window, cx| {
                                        this.reload_from_disk(cx);
                                    }),
                                )
                                .child("Reload"),
                        )
                        .child(
                            div()
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .text_size(px(11.))
                                .text_color(theme.muted)
                                .cursor(CursorStyle::PointingHand)
                                .hover(|s| s.text_color(theme.foreground))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _ev, _window, cx| {
                                        this.acknowledged_hash = Self::config_file_hash();
                                        this.stale = false;
                                        cx.notify();
                                    }),
                                )
                                .child("Dismiss"),
                        ),
                )
            })
            .child(
                div()
                    .id("settings-scroll-container")
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .child(if self.active_tab == SettingsTab::Keyboard {
                        self.render_keyboard_page(theme, window, cx).into_any_element()
                    } else {
                        div()
                            .id("settings-scroll-content")
                            .track_scroll(&self.window_scroll_handle)
                            .on_scroll_wheel(cx.listener(Self::handle_settings_scroll))
                            .size_full()
                            .overflow_y_scroll()
                            .px(px(56.))
                            .pt(px(28.))
                            .pb(px(48.))
                            .flex()
                            .flex_col()
                            .gap(px(24.))
                            .child(self.render_page_title(theme))
                            .child(match self.active_tab {
                                SettingsTab::General => self.render_tab_general(theme, cx),
                                SettingsTab::Appearance => self.render_tab_appearance(theme, window, cx),
                                SettingsTab::Keyboard => div(),
                                SettingsTab::Ai => self.render_tab_ai(theme, window, cx),
                                SettingsTab::Migration => self.render_tab_migration(theme, cx),
                                SettingsTab::Advanced => self.render_tab_advanced(theme, cx),
                            })
                            .into_any_element()
                    }),
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
                        render_plain_group()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(24.))
                                    // Horizontal Card
                                    .child(
                                        div()
                                            .id("layout-horizontal-card")
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .p(px(14.))
                                            .rounded(px(12.))
                                            .bg(if is_horiz { theme.surface_raised } else { theme.surface })
                                            .border_1()
                                            .border_color(if is_horiz { theme.accent } else { theme.border })
                                            .hover(move |style| style.bg(theme.surface_raised))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.set_tab_layout(crate::config::TabLayout::Horizontal, cx);
                                            }))
                                            // Mini mockup
                                            .child(
                                                div()
                                                    .w_full()
                                                    .h(px(110.))
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
                                            .child(
                                                div()
                                                    .w_full()
                                                    .pt(px(12.))
                                                    .text_center()
                                                    .text_size(px(13.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(if is_horiz { theme.foreground } else { theme.muted })
                                                    .child("Horizontal (Top Bar)"),
                                            )
                                    )
                                    // Vertical Card
                                    .child(
                                        div()
                                            .id("layout-vertical-card")
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .p(px(14.))
                                            .rounded(px(12.))
                                            .bg(if is_vert { theme.surface_raised } else { theme.surface })
                                            .border_1()
                                            .border_color(if is_vert { theme.accent } else { theme.border })
                                            .hover(move |style| style.bg(theme.surface_raised))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.set_tab_layout(crate::config::TabLayout::Vertical, cx);
                                            }))
                                            // Mini mockup
                                            .child(
                                                div()
                                                    .w_full()
                                                    .h(px(110.))
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
                                            .child(
                                                div()
                                                    .w_full()
                                                    .pt(px(12.))
                                                    .text_center()
                                                    .text_size(px(13.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(if is_vert { theme.foreground } else { theme.muted })
                                                    .child("Vertical (Sidebar)"),
                                            )
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
                        render_plain_group()
                            .child(render_plain_row(
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
                            .child(render_plain_row(
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
                            .child(render_plain_row(
                                "Automatic Contrast Correction",
                                Some("Fix program-requested colors that clash with the theme (Oklab lightness, hue preserved)"),
                                div()
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                        this.toggle_contrast_correction(cx);
                                    }))
                                    .child(render_toggle_switch(self.contrast_correction, theme)),
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
                                render_plain_row(
                                    "Scrollback History",
                                    Some("Maximum lines preserved in terminal buffer"),
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap(px(2.))
                                        .p(px(3.))
                                        .rounded(px(8.))
                                        .bg(theme.surface_raised)
                                        .children(scrollback_pills.into_iter().map(|(lines, label)| {
                                            let is_active = self.scrollback == lines;
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .px(px(10.))
                                                .py(px(5.))
                                                .rounded(px(6.))
                                                .when(is_active, |el| el.bg(theme.accent))
                                                .hover(move |s| if !is_active { s.bg(theme.hover) } else { s })
                                                .text_color(if is_active { theme.background } else { theme.muted })
                                                .text_size(px(12.))
                                                .font_weight(if is_active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                    this.set_scrollback(lines, cx);
                                                }))
                                                .child(label)
                                        })),
                                    true,
                                    theme,
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Notifications", theme))
                    .child(
                        render_plain_group()
                            .child(render_plain_row(
                                "AI Status",
                                None::<&str>,
                                div()
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                        this.toggle_notify_on_ai_status(cx);
                                    }))
                                    .child(render_toggle_switch(self.config.notify_on_ai_status, theme)),
                                false,
                                theme,
                            ))
                            .child(render_plain_row(
                                "Program Status",
                                None::<&str>,
                                div()
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                        this.toggle_notify_on_program_status(cx);
                                    }))
                                    .child(render_toggle_switch(self.config.notify_on_program_status, theme)),
                                true,
                                theme,
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Updates", theme))
                    .child(
                        render_plain_group()
                            .child({
                                let channels = [
                                    ("stable", "Stable"),
                                    ("beta", "Beta"),
                                ];
                                render_plain_row(
                                    "Update Channel",
                                    Some("Beta includes prereleases"),
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap(px(2.))
                                        .p(px(3.))
                                        .rounded(px(8.))
                                        .bg(theme.surface_raised)
                                        .children(channels.into_iter().map(|(id, label)| {
                                            let is_active = self.update_channel == id;
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .px(px(10.))
                                                .py(px(5.))
                                                .rounded(px(6.))
                                                .when(is_active, |el| el.bg(theme.accent))
                                                .hover(move |s| if !is_active { s.bg(theme.hover) } else { s })
                                                .text_color(if is_active { theme.background } else { theme.muted })
                                                .text_size(px(12.))
                                                .font_weight(if is_active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                    this.set_update_channel(id, cx);
                                                }))
                                                .child(label)
                                        })),
                                    false,
                                    theme,
                                )
                            })
                            .child(render_plain_row(
                                "Version",
                                Some("Current installed version"),
                                div()
                                    .px(px(9.))
                                    .py(px(4.))
                                    .rounded(px(6.))
                                    .bg(theme.surface_raised)
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.muted_strong)
                                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                                false,
                                theme,
                            ))
                            .child({
                                let status = self.update_status_msg.clone();
                                let primary_label = if self.update_checking {
                                    "Working..."
                                } else if self.update_ready {
                                    "Restart Now"
                                } else if let Some(ref rel) = self.update_found {
                                    if rel.self_update_blocked_reason.is_some() {
                                        "Open download page"
                                    } else {
                                        "Update Now"
                                    }
                                } else {
                                    "Check now"
                                };
                                let show_skip = self.update_found.is_some()
                                    && !self.update_ready
                                    && !self.update_checking;
                                render_plain_row(
                                    "Check for updates",
                                    status.as_deref(),
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap_1p5()
                                        .when(show_skip, |row| {
                                            row.child(
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
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(theme.foreground)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                        this.skip_found_update(cx);
                                                    }))
                                                    .child("Skip this version"),
                                            )
                                        })
                                        .child(
                                            div()
                                                .px(px(14.))
                                                .py(px(7.))
                                                .rounded(px(7.))
                                                .bg(theme.accent)
                                                .cursor(CursorStyle::PointingHand)
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(theme.background)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, window, cx| {
                                                    this.update_primary_action(window, cx);
                                                }))
                                                .child(primary_label),
                                        ),
                                    true,
                                    theme,
                                )
                            }),
                    ),
            )
    }

    fn render_tab_appearance(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let font_family = SharedString::from(self.font_family.clone());

        // Theme carousel: fixed-width cards scrolled horizontally.
        let mut theme_cards = div()
            .id("theme-carousel")
            .track_scroll(&self.theme_carousel_scroll)
            .flex()
            .flex_row()
            .gap(px(20.))
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .w_full();
        for card in &self.theme_cards {
            let is_active = self.current_theme_name == card.name;
            let name = card.name.clone();
            theme_cards = theme_cards.child(
                div()
                    .id(SharedString::from(format!("theme-card-{}", card.name)))
                    .w(px(300.))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if is_active { theme.accent } else { theme.border })
                    .bg(theme.surface)
                    .overflow_hidden()
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |style| style.border_color(theme.muted_strong))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                        this.set_theme(&name, cx);
                    }))
                    .child(
                        div()
                            .h(px(156.))
                            .p(px(14.))
                            .flex()
                            .flex_col()
                            .gap(px(7.))
                            .bg(card.bg_color)
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(4.))
                                    .pb(px(4.))
                                    .children((0..3).map(|_| {
                                        div()
                                            .w(px(6.))
                                            .h(px(6.))
                                            .rounded_full()
                                            .bg(card.surf_color)
                                    })),
                            )
                            .child(
                                div()
                                    .font_family(font_family.clone())
                                    .text_size(px(12.5))
                                    .text_color(card.fg_color)
                                    .child("~ > fastty --version"),
                            )
                            .child(
                                div()
                                    .font_family(font_family.clone())
                                    .text_size(px(12.5))
                                    .text_color(card.muted_color)
                                    .child(format!("fastty {}", env!("CARGO_PKG_VERSION"))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(4.))
                                    .font_family(font_family.clone())
                                    .text_size(px(12.5))
                                    .text_color(card.acc_color)
                                    .child("~ >")
                                    .child(
                                        div()
                                            .w(px(6.))
                                            .h(px(13.))
                                            .bg(card.fg_color),
                                    ),
                            )
                            .child(
                                div()
                                    .mt_auto()
                                    .w_full()
                                    .h(px(22.))
                                    .rounded(px(4.))
                                    .overflow_hidden()
                                    .flex()
                                    .flex_row()
                                    .children(card.ansi.into_iter().map(|color| {
                                        div().flex_1().h_full().bg(color)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .h(px(42.))
                            .px(px(14.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .bg(theme.surface_raised)
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(9.))
                                    .child(
                                        div()
                                            .w(px(10.))
                                            .h(px(10.))
                                            .rounded_full()
                                            .bg(card.acc_color),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.foreground)
                                            .child(SharedString::from(card.label.clone())),
                                    ),
                            )
                            .when(is_active, |row| {
                                row.child(render_icon(IconType::Check, theme.accent, 14.0))
                            }),
                    ),
            );
        }

        let theme_count = self.theme_cards.len();
        let carousel_offset = self.theme_carousel_scroll.offset().x.as_f32();
        let carousel_max = self.theme_carousel_scroll.max_offset().x.as_f32().abs();
        let can_prev = carousel_offset < -0.5;
        let can_next = carousel_offset > -carousel_max + 0.5;
        let current_theme_index = (((self
            .theme_carousel_scroll
            .offset()
            .x
            .as_f32()
            .abs()
            / 320.0)
            .round() as usize)
            + 1)
            .min(theme_count.max(1));

        let opacity_pills = [1.0, 0.95, 0.90, 0.85, 0.80, 0.75];

        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            // Color Theme
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(render_section_header("Color Theme", theme))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme.muted)
                            .child(SharedString::from(format!(
                                "Currently applied: {}",
                                self.current_theme_name
                            ))),
                    )
                    .child(theme_cards)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .pt(px(4.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(theme.muted)
                                    .child(SharedString::from(format!(
                                        "{current_theme_index} / {theme_count} themes"
                                    ))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .id("theme-prev")
                                            .w(px(34.))
                                            .h(px(34.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(8.))
                                            .bg(if can_prev { theme.surface_raised } else { theme.surface })
                                            .cursor(if can_prev { CursorStyle::PointingHand } else { CursorStyle::Arrow })
                                            .when(can_prev, |button| {
                                                button.hover(move |style| style.bg(theme.hover))
                                            })
                                            .when(can_prev, |button| {
                                                button.on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.scroll_theme_carousel(1.0);
                                                    cx.notify();
                                                }))
                                            })
                                            .child(render_icon(
                                                IconType::ChevronLeft,
                                                if can_prev { theme.muted_strong } else { theme.muted.opacity(0.4) },
                                                15.0,
                                            )),
                                    )
                                    .child(
                                        div()
                                            .id("theme-next")
                                            .w(px(34.))
                                            .h(px(34.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(8.))
                                            .bg(if can_next { theme.surface_raised } else { theme.surface })
                                            .cursor(if can_next { CursorStyle::PointingHand } else { CursorStyle::Arrow })
                                            .when(can_next, |button| {
                                                button.hover(move |style| style.bg(theme.hover))
                                            })
                                            .when(can_next, |button| {
                                                button.on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.scroll_theme_carousel(-1.0);
                                                    cx.notify();
                                                }))
                                            })
                                            .child(render_icon(
                                                IconType::ChevronRight,
                                                if can_next { theme.muted_strong } else { theme.muted.opacity(0.4) },
                                                15.0,
                                            )),
                                    ),
                            ),
                    ),
            )
            // Window
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Window", theme))
                    .child(render_plain_row(
                        "Window Opacity",
                        Some("Background transparency and macOS vibrancy blur"),
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(2.))
                            .p(px(3.))
                            .rounded(px(8.))
                            .bg(theme.surface_raised)
                            .children(opacity_pills.into_iter().map(|value| {
                                let is_active = (self.opacity - value).abs() < 0.025;
                                let label = format!("{:.0}%", value * 100.0);
                                div()
                                    .flex()
                                    .items_center()
                                    .px(px(10.))
                                    .py(px(5.))
                                    .rounded(px(6.))
                                    .when(is_active, |el| el.bg(theme.accent))
                                    .hover(move |style| {
                                        if !is_active {
                                            style.bg(theme.hover)
                                        } else {
                                            style
                                        }
                                    })
                                    .text_color(if is_active {
                                        theme.background
                                    } else {
                                        theme.muted
                                    })
                                    .text_size(px(12.))
                                    .font_weight(if is_active {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                        this.adjust_opacity(value, cx);
                                    }))
                                    .child(label)
                            })),
                        true,
                        theme,
                    )),
            )
            // Typography
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header("Typography", theme))
                    .child(render_plain_row(
                        "Font Size",
                        Some("Terminal text grid cell point size"),
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2.))
                            .p(px(3.))
                            .rounded(px(8.))
                            .bg(theme.surface_raised)
                            .child(
                                div()
                                    .w(px(28.))
                                    .h(px(28.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.))
                                    .hover(|style| style.bg(theme.hover))
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
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
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
                                    .hover(|style| style.bg(theme.hover))
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
                    .child(render_plain_row(
                        "Ligatures",
                        Some("Combine character pairs like -> into a single glyph"),
                        div()
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                this.toggle_font_ligatures(cx);
                            }))
                            .child(render_toggle_switch(self.font_ligatures, theme)),
                        false,
                        theme,
                    ))
                    .when(cfg!(target_os = "macos"), |this| {
                        this.child(render_plain_row(
                            "Monospace Fonts Only",
                            Some("Show only fonts whose characters share one cell width"),
                            div()
                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                    this.toggle_monospace_fonts_only(cx);
                                }))
                                .child(render_toggle_switch(self.monospace_fonts_only, theme)),
                            false,
                            theme,
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .py(px(16.))
                            .child(
                                div()
                                    .text_size(px(14.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .child("Font Family"),
                            )
                            .child(self.render_font_combobox(theme, window, cx)),
                    )
                    .child({
                        let preview_font = SharedString::from(self.font_family.clone());
                        let preview_size = self.font_size.clamp(10.0, 24.0);
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .px(px(14.))
                            .py(px(10.))
                            .rounded(px(8.))
                            .bg(theme.surface_raised)
                            .child(
                                div()
                                    .font_family(preview_font.clone())
                                    .text_size(px(preview_size))
                                    .text_color(theme.accent)
                                    .child("~ >"),
                            )
                            .child(
                                div()
                                    .font_family(preview_font)
                                    .text_size(px(preview_size))
                                    .text_color(theme.muted_strong)
                                    .child("echo \"fast ⇒ fastty\" ≠ slow"),
                            )
                    }),
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
                    .relative()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(40.))
                            .px(px(12.))
                            .rounded(px(8.))
                            .bg(if is_open { theme.selected } else { theme.surface_raised })
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
                                            .text_size(px(13.5))
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

                        combobox.child(gpui::deferred(
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
                                                                    .w_full()
                                                                    .h(px(32.))
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .justify_between()
                                                                    .px(px(10.))
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
                        ).with_priority(100))
                    }),
            )
    }

    fn render_keyboard_page(&mut self, theme: &Theme, _window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let theme = *theme;
        let rows = std::rc::Rc::new(self.keyboard_rows());
        if self.keyboard_row_count != rows.len() {
            let previous = self.keyboard_list_state.logical_scroll_top();
            self.keyboard_list_state.reset(rows.len());
            self.keyboard_list_state.scroll_to(previous);
            self.keyboard_row_count = rows.len();
        }
        let render = cx.processor(move |this: &mut SettingsView, ix: usize, _window, cx| {
            let Some(row) = rows.get(ix).copied() else {
                return div().into_any_element();
            };
            this.render_keyboard_row(row, &theme, cx)
        });
        let title = self.render_page_title(&theme);
        div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .px(px(56.))
            .pt(px(28.))
            .pb(px(24.))
            .gap(px(24.))
            .child(title)
            .child(
                list(self.keyboard_list_state.clone(), render)
                    .w_full()
                    .flex_1(),
            )
    }

    fn keyboard_rows(&self) -> Vec<KeyboardRow> {
        let mut rows = vec![KeyboardRow::PresetsHeader];
        rows.extend((0..KEYBOARD_PRESETS.len()).map(KeyboardRow::Preset));
        rows.push(KeyboardRow::CustomHeader);
        let all = crate::keybindings::Action::all();
        for category in crate::keybindings::Action::categories() {
            let actions: Vec<_> = all
                .iter()
                .copied()
                .filter(|action| action.category() == category)
                .collect();
            if actions.is_empty() {
                continue;
            }
            rows.push(KeyboardRow::Category(category));
            for action in actions {
                rows.push(KeyboardRow::Action(action));
                if self
                    .binding_conflict
                    .is_some_and(|conflict| conflict.target == action)
                {
                    rows.push(KeyboardRow::Conflict(action));
                }
            }
        }
        rows
    }

    fn render_keyboard_row(
        &mut self,
        row: KeyboardRow,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match row {
            KeyboardRow::PresetsHeader => render_section_header("Keybinding Presets", theme)
                .into_any_element(),
            KeyboardRow::Preset(idx) => self.render_keyboard_preset(idx, theme, cx),
            KeyboardRow::CustomHeader => {
                let has_custom = !self.config.keybindings.is_empty();
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .pt(px(28.))
                    .pb(px(6.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(render_section_header("Custom Shortcuts", theme))
                            .when(has_custom, |this| {
                                this.child(
                                    div()
                                        .id("kb-reset-all")
                                        .px(px(12.))
                                        .py(px(6.))
                                        .rounded(px(7.))
                                        .bg(theme.surface_raised)
                                        .border_1()
                                        .border_color(theme.border)
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme.muted_strong)
                                        .cursor(CursorStyle::PointingHand)
                                        .hover(|s| s.bg(theme.hover))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _ev, _window, cx| {
                                                this.reset_all_bindings(cx);
                                            }),
                                        )
                                        .child("Reset all"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme.muted)
                            .child("Click a shortcut, then press the keys. Esc cancels."),
                    )
                    .into_any_element()
            }
            KeyboardRow::Category(category) => div()
                .w_full()
                .pt(px(18.))
                .pb(px(4.))
                .text_size(px(13.))
                .text_color(theme.muted)
                .child(category)
                .into_any_element(),
            KeyboardRow::Action(action) => self.render_keyboard_action(action, theme, cx),
            KeyboardRow::Conflict(action) => self.render_keyboard_conflict(action, theme, cx),
        }
    }

    fn render_keyboard_preset(
        &self,
        idx: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some((preset, name, desc)) = KEYBOARD_PRESETS.get(idx).copied() else {
            return div().into_any_element();
        };
        let is_active = self.keybinding_preset == preset;
        div()
            .id(SharedString::from(format!("preset-row-{}", preset)))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(14.))
            .py(px(14.))
            .rounded(px(8.))
            .when(is_active, |row| row.bg(theme.surface_raised))
            .hover(move |style| if is_active { style } else { style.bg(theme.hover) })
            .cursor(CursorStyle::PointingHand)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                this.set_keybinding_preset(preset, cx);
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(15.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.foreground)
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme.muted)
                            .child(desc),
                    ),
            )
            .child(
                div()
                    .w(px(22.))
                    .h(px(22.))
                    .flex_shrink_0()
                    .rounded_full()
                    .border_1()
                    .border_color(if is_active { theme.accent } else { theme.muted })
                    .bg(if is_active { theme.accent } else { theme.surface })
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(is_active, |dot| {
                        dot.child(render_icon(IconType::Check, theme.background, 13.0))
                    }),
            )
            .into_any_element()
    }

    fn render_keyboard_action(
        &mut self,
        action: crate::keybindings::Action,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let binding_data = self.binding_render_data();
        let combos: &[Vec<String>] = binding_data
            .combos
            .get(&action)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let is_capturing = self.capturing_action == Some(action);
        let customized = binding_data.customized.contains(&action);

        let mut badges = div().flex().flex_row().flex_wrap().items_center().gap_2();
        if combos.is_empty() {
            badges = badges.child(
                div()
                    .id(SharedString::from(format!("kb-badge-{}", action.binding_id())))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .border_dashed()
                    .text_size(px(12.))
                    .text_color(theme.muted)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(theme.surface_raised))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _ev, _window, cx| {
                            this.start_binding_capture(action, cx);
                        }),
                    )
                    .child(if is_capturing { "Press keys…" } else { "Unbound" }),
            );
        } else {
            for (i, keycaps) in combos.iter().enumerate() {
                let is_recording = is_capturing && i == 0;
                badges = badges.child(
                    div()
                        .id(SharedString::from(format!(
                            "kb-badge-{}-{i}",
                            action.binding_id()
                        )))
                        .px(px(6.))
                        .py(px(5.))
                        .rounded(px(6.))
                        .bg(theme.surface_raised)
                        .border_1()
                        .border_color(if is_recording { theme.accent } else { theme.border })
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .cursor(CursorStyle::PointingHand)
                        .hover(|s| s.bg(theme.hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _ev, _window, cx| {
                                this.start_binding_capture(action, cx);
                            }),
                        )
                        .when(is_recording, |el| {
                            el.child(
                                div()
                                    .h(px(18.))
                                    .px(px(5.))
                                    .rounded(px(4.))
                                    .bg(theme.accent)
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.background)
                                            .child("Press keys…"),
                                    ),
                            )
                        })
                        .when(!is_recording, |el| {
                            el.children(keycaps.iter().map(|cap| {
                                div()
                                    .h(px(20.))
                                    .min_w(px(20.))
                                    .px(px(5.))
                                    .rounded(px(4.))
                                    .bg(theme.surface)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.foreground)
                                            .child(SharedString::from(cap.clone())),
                                    )
                            }))
                        }),
                );
            }
        }
        if customized {
            badges = badges.child(
                div()
                    .id(SharedString::from(format!("kb-reset-{}", action.binding_id())))
                    .px(px(6.))
                    .py(px(3.))
                    .rounded(px(5.))
                    .text_size(px(11.))
                    .text_color(theme.muted)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(theme.surface_raised))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _ev, _window, cx| {
                            this.reset_action_bindings_ui(action, cx);
                        }),
                    )
                    .child("↺"),
            );
        }
        if !combos.is_empty() {
            badges = badges.child(
                div()
                    .id(SharedString::from(format!("kb-unbind-{}", action.binding_id())))
                    .px(px(6.))
                    .py(px(3.))
                    .rounded(px(5.))
                    .text_size(px(11.))
                    .text_color(theme.muted)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(theme.surface_raised))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _ev, _window, cx| {
                            this.unbind_action(action, cx);
                        }),
                    )
                    .child("✕"),
            );
        }

        render_plain_row_inset(
            format!(
                "{}{}",
                action.display_name(),
                if customized { " •" } else { "" }
            ),
            None::<&str>,
            badges,
            false,
            theme,
        )
        .w_full()
        .into_any_element()
    }

    fn render_keyboard_conflict(
        &self,
        action: crate::keybindings::Action,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(conflict) = self.binding_conflict else {
            return div().into_any_element();
        };
        if conflict.target != action {
            return div().into_any_element();
        }
        let owner_name = conflict.owner.display_name();
        let combo_label = crate::keybindings::format_combo(&conflict.combo);
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(14.))
            .py(px(10.))
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(theme.foreground)
                    .child(format!("{combo_label} is already assigned to {owner_name}.")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        div()
                            .id("kb-conflict-reassign")
                            .px(px(12.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .bg(theme.accent)
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.background)
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _ev, _window, cx| {
                                    this.confirm_binding_conflict(cx);
                                }),
                            )
                            .child("Reassign"),
                    )
                    .child(
                        div()
                            .id("kb-conflict-cancel")
                            .px(px(12.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.border)
                            .text_size(px(12.))
                            .text_color(theme.muted_strong)
                            .cursor(CursorStyle::PointingHand)
                            .hover(|s| s.bg(theme.surface))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _ev, _window, cx| {
                                    this.cancel_binding_capture(cx);
                                }),
                            )
                            .child("Cancel"),
                    ),
            )
            .into_any_element()
    }

    fn render_page_title(&self, theme: &Theme) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(32.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.foreground)
                    .child(self.active_tab.label()),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme.muted)
                    .child(self.active_tab.subtitle()),
            )
    }

    fn render_tab_ai(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let is_acp = self.config.ai.providers.get(&self.ai_provider)
            .is_some_and(crate::ai::ProviderConfig::is_acp);
        let providers = [
            ("ollama", "Ollama", "Local / Free", "Offline private models on localhost:11434"),
            ("anthropic", "Anthropic", "Cloud API", "Claude 3.7 Sonnet & 3.5 Haiku"),
            ("openai", "OpenAI", "Cloud API", "GPT-4o, GPT-4o-mini & o3-mini"),
            ("openrouter", "OpenRouter", "Unified API", "Hundreds of open & proprietary models"),
            ("lm-studio", "LM Studio", "Local Server", "Local LLM server on localhost:1234"),
            ("opencode", "OpenCode", "ACP Agent", "Native agent with tools, MCP, and permissions"),
            ("claude-acp", "Claude Agent", "ACP Agent", "Claude Agent SDK over ACP"),
            ("antigravity-acp", "Antigravity", "ACP Agent", "Google Antigravity over ACP"),
        ];

        let is_ollama = self.ai_provider == "ollama";
        let default_presets = crate::ai::AiConfig::default_preset_models(&self.ai_provider);
        let chips_to_render: Vec<String> = if self.config.ai.providers.get(&self.ai_provider).is_some_and(crate::ai::ProviderConfig::is_acp) {
            self.config.ai.providers.get(&self.ai_provider)
                .map(|provider| provider.models().to_vec()).unwrap_or_default()
        } else if is_ollama {
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
                        div().flex().flex_col().children(providers.into_iter().enumerate().map(|(idx, (id, name, badge, desc))| {
                            let is_active = self.ai_provider == id;
                            let _is_last = idx == providers.len() - 1;
                            div()
                                .id(SharedString::from(format!("ai-prov-row-{}", id)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .px(px(14.))
                                .py(px(14.))
                                .rounded(px(8.))
                                .when(is_active, |row| row.bg(theme.surface_raised))
                                .hover(move |s| if is_active { s } else { s.bg(theme.hover) })
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, window, cx| {
                                    this.set_ai_provider(id, window, cx);
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
                                                        .text_size(px(15.5))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(theme.foreground)
                                                        .child(name),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(10.))
                                                        .px(px(6.))
                                                        .py(px(2.))
                                                        .rounded(px(4.))
                                                        .bg(theme.surface_raised)
                                                        .text_color(theme.muted)
                                                        .child(badge.to_uppercase()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .text_color(theme.muted)
                                                .child(desc),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(22.))
                                        .h(px(22.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(if is_active { theme.accent } else { theme.muted })
                                        .bg(if is_active { theme.accent } else { theme.surface })
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(is_active, |this| {
                                            this.child(render_icon(IconType::Check, theme.background, 13.0))
                                        }),
                                )
                        })),
                    ),
            )
            // Section 2: Model selector
            .child({
                let is_model_focused = self.active_input_field == Some(ActiveInputField::AiModel);
                let model_val = self.ai_model_input.clone();
                let is_open = self.ai_model_dropdown_open;
                let model_count = chips_to_render.len();
                let subtitle = if is_acp {
                    format!("{model_count} available from the agent, or type any model ID")
                } else if is_ollama {
                    format!("{model_count} installed models, or type any model ID")
                } else {
                    format!("{model_count} presets, or type any model ID")
                };

                div()
                    .flex()
                    .flex_col()
                    .child(render_section_header(if is_acp { "ACP Models" } else { "Model Configuration (BYOK)" }, theme))
                    .child(
                        div().flex().flex_col()
                            .child(render_plain_row_inset(
                                if is_acp { "Model" } else { "Model Identifier" },
                                Some(subtitle),
                                div()
                                    .w(px(300.))
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(40.))
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
                                            .px(px(10.))
                                            .rounded(px(8.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(if is_model_focused || is_open { theme.accent } else { theme.border })
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .cursor(CursorStyle::IBeam)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                                this.active_input_field = Some(ActiveInputField::AiModel);
                                                this.ai_model_state.last_bounds = this.ai_model_bounds.get();
                                                let col = this.ai_model_state.index_for_position(ev.position, 13.0, 10.0, window);
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
                                                                    .text_size(px(13.))
                                                                    .text_color(theme.muted)
                                                                    .child("Type any model ID"),
                                                            )
                                                    } else {
                                                        crate::ui::text_input::render_line_spans(
                                                            &model_val,
                                                            0,
                                                            if is_model_focused { Some(self.ai_model_state.cursor) } else { None },
                                                            self.ai_model_state.selection,
                                                            13.0,
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
                                                        .text_size(px(11.))
                                                        .text_color(theme.muted)
                                                        .child("✕")
                                                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                            cx.stop_propagation();
                                                            this.clear_ai_model(cx);
                                                        })),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .pl(px(4.))
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                        cx.stop_propagation();
                                                        this.ai_model_dropdown_open = !this.ai_model_dropdown_open;
                                                        cx.notify();
                                                    }))
                                                    .child(render_icon(
                                                        if is_open { IconType::ChevronUp } else { IconType::ChevronDown },
                                                        theme.muted,
                                                        14.0,
                                                    )),
                                            ),
                                    )
                                    .when(is_open, |container| {
                                        let open_upward = self.ai_model_bounds.get().is_some_and(|bounds| {
                                            let window_h = window.viewport_size().height;
                                            let space_below = window_h - bounds.bottom();
                                            space_below < px(260.) && bounds.top() > space_below
                                        });
                                        container.child(gpui::deferred(
                                            div()
                                                .id("ai-model-dropdown")
                                                .absolute()
                                                .when(open_upward, |el| el.bottom(px(44.)))
                                                .when(!open_upward, |el| el.top(px(44.)))
                                                .left_0()
                                                .right_0()
                                                .occlude()
                                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                                    cx.stop_propagation();
                                                })
                                                .on_scroll_wheel(|_ev, _window, cx| cx.stop_propagation())
                                                .rounded(px(8.))
                                                .bg(theme.surface_raised)
                                                .border_1()
                                                .border_color(theme.border)
                                                .shadow_lg()
                                                .p(px(6.))
                                                .flex()
                                                .flex_col()
                                                .gap(px(2.))
                                                .when(is_ollama, |list| {
                                                    list.child(
                                                        div()
                                                            .h(px(30.))
                                                            .px(px(8.))
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .justify_between()
                                                            .rounded(px(6.))
                                                            .hover(move |style| style.bg(theme.hover))
                                                            .cursor(CursorStyle::PointingHand)
                                                            .text_size(px(12.))
                                                            .text_color(theme.accent)
                                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                                cx.stop_propagation();
                                                                this.refresh_ollama_models(cx);
                                                            }))
                                                            .child("Refresh installed models")
                                                            .child("↺"),
                                                    )
                                                })
                                                .child(
                                                    div()
                                                        .id("ai-model-list")
                                                        .max_h(px(220.))
                                                        .overflow_y_scroll()
                                                        .track_scroll(&self.ai_model_scroll_handle)
                                                        .flex()
                                                        .flex_col()
                                                        .gap(px(2.))
                                                        .children(chips_to_render.into_iter().map(|model_name| {
                                                            let is_active = self.ai_model_input.trim() == model_name;
                                                            let selected = model_name.clone();
                                                            div()
                                                                .id(SharedString::from(format!("ai-model-pill-{}", model_name)))
                                                                .w_full()
                                                                .h(px(32.))
                                                                .flex()
                                                                .flex_row()
                                                                .items_center()
                                                                .justify_between()
                                                                .px(px(10.))
                                                                .rounded(px(6.))
                                                                .bg(if is_active { theme.accent } else { theme.surface_raised })
                                                                .hover(move |style| if is_active { style } else { style.bg(theme.hover) })
                                                                .cursor(CursorStyle::PointingHand)
                                                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                    cx.stop_propagation();
                                                                    this.set_ai_model(&selected, cx);
                                                                    this.ai_model_dropdown_open = false;
                                                                    cx.notify();
                                                                }))
                                                                .child(
                                                                    div()
                                                                        .text_size(px(12.5))
                                                                        .text_color(if is_active { theme.background } else { theme.foreground })
                                                                        .child(SharedString::from(model_name)),
                                                                )
                                                                .when(is_active, |row| {
                                                                    row.child(render_icon(IconType::Check, theme.background, 12.0))
                                                                })
                                                        })),
                                                ),
                                        ))
                                    }),
                                false,
                                theme,
                            )),
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
                    .child(render_section_header(if is_acp { "ACP Agent" } else { "Endpoint & Authentication" }, theme))
                    .child(
                        div().flex().flex_col()
                            .child(render_plain_row_inset(
                                if is_acp { "Executable Command" } else { "Endpoint URL" },
                                Some(if is_acp { "Command used to start the ACP agent" } else { "HTTP API base address for model requests" }),
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(300.))
                                            .h(px(40.))
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
                                            .px(px(10.))
                                            .rounded(px(8.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(if is_url_focused { theme.accent } else { theme.border })
                                            .flex()
                                            .items_center()
                                            .cursor(CursorStyle::IBeam)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                                this.active_input_field = Some(ActiveInputField::AiBaseUrl);
                                                this.ai_base_url_state.last_bounds = this.ai_base_url_bounds.get();
                                                let col = this.ai_base_url_state.index_for_position(ev.position, 13.0, 10.0, window);
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
                                                            .text_size(px(13.))
                                                            .text_color(theme.muted)
                                                    .child(if is_acp { "e.g. claude-agent-acp" } else { "e.g. http://localhost:11434/v1" }),
                                                    )
                                            } else {
                                                crate::ui::text_input::render_line_spans(
                                                    &url_val,
                                                    0,
                                                    if is_url_focused { Some(self.ai_base_url_state.cursor) } else { None },
                                                    self.ai_base_url_state.selection,
                                                    13.0,
                                                    theme,
                                                    Some(window),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .px(px(12.))
                                            .py(px(6.))
                                            .rounded(px(7.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(theme.border)
                                            .hover(move |s| s.bg(theme.hover))
                                            .cursor(CursorStyle::PointingHand)
                                            .text_size(px(12.5))
                                            .text_color(theme.muted_strong)
                                            .child(if is_acp { "Default" } else { "Reset" })
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.reset_ai_base_url(cx);
                                            })),
                                    ),
                                false,
                                theme,
                            ))
                            .when(!is_acp, |card| card.child(render_plain_row_inset(
                                "API Secret Key",
                                Some("Custom key stored in fastty.toml (overrides env)"),
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(300.))
                                            .h(px(40.))
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
                                            .px(px(10.))
                                            .rounded(px(8.))
                                            .bg(theme.surface_raised)
                                            .border_1()
                                            .border_color(if is_key_focused { theme.accent } else { theme.border })
                                            .flex()
                                            .items_center()
                                            .cursor(CursorStyle::IBeam)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                                this.active_input_field = Some(ActiveInputField::AiApiKey);
                                                this.ai_api_key_state.last_bounds = this.ai_api_key_bounds.get();
                                                let col = this.ai_api_key_state.index_for_position(ev.position, 13.0, 10.0, window);
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
                                                            .text_size(px(13.))
                                                            .text_color(theme.muted)
                                                            .child("Paste key (⌘V)..."),
                                                    )
                                            } else if is_key_focused {
                                                crate::ui::text_input::render_line_spans(
                                                    &key_val,
                                                    0,
                                                    Some(self.ai_api_key_state.cursor),
                                                    self.ai_api_key_state.selection,
                                                    13.0,
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
                            )))
                            .when(!is_acp, |card| card.child(render_plain_row_inset(
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
                            )))
                            .child(render_plain_row_inset(
                                "Test Connection",
                                Some(if is_acp { "Refresh the model catalog from the ACP agent" } else { "Test endpoint reachability and model response" }),
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
                                                    .px(px(10.))
                                                    .py(px(5.))
                                                    .rounded(px(6.))
                                                    .bg(theme.surface_raised)
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
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
                                                    .px(px(10.))
                                                    .py(px(5.))
                                                    .rounded(px(6.))
                                                    .bg(theme.surface_raised)
                                                    .border_1()
                                                    .border_color(theme.green)
                                                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(theme.green))
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(theme.green)
                                                            .child(SharedString::from(format!("OK ({}ms)", ms))),
                                                    )
                                            }
                                            AiTestStatus::Failed(err) => {
                                                let short_err = if err.chars().count() > 30 {
                                                    format!("{}...", err.chars().take(27).collect::<String>())
                                                } else {
                                                    err.clone()
                                                };
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .px(px(10.))
                                                    .py(px(5.))
                                                    .rounded(px(6.))
                                                    .bg(theme.surface_raised)
                                                    .border_1()
                                                    .border_color(theme.red)
                                                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(theme.red))
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
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
                                            .px(px(14.))
                                            .py(px(7.))
                                            .rounded(px(7.))
                                            .bg(if is_testing { theme.surface_raised } else { theme.accent })
                                            .border_1()
                                            .border_color(if is_testing { theme.border } else { theme.accent })
                                            .hover(move |s| {
                                                if is_testing { s } else { s.opacity(0.88) }
                                            })
                                            .cursor(if is_testing { CursorStyle::Arrow } else { CursorStyle::PointingHand })
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(if is_testing { theme.muted } else { theme.background })
                                            .child(if is_testing { "Testing..." } else if is_acp { "Refresh Models" } else { "Test Connection" })
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
                        div().flex().flex_col().children(permissions.into_iter().enumerate().map(|(idx, (mode, name, desc))| {
                            let is_active = self.ai_permission_mode == mode;
                            let _is_last = idx == permissions.len() - 1;
                            div()
                                .id(SharedString::from(format!("ai-mode-row-{:?}", mode)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .px(px(14.))
                                .py(px(14.))
                                .rounded(px(8.))
                                .when(is_active, |row| row.bg(theme.surface_raised))
                                .hover(move |s| if is_active { s } else { s.bg(theme.hover) })
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
                                                .text_size(px(15.5))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(theme.foreground)
                                                .child(name),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .text_color(theme.muted)
                                                .child(desc),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(22.))
                                        .h(px(22.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(if is_active { theme.accent } else { theme.muted })
                                        .bg(if is_active { theme.accent } else { theme.surface })
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(is_active, |this| {
                                            this.child(render_icon(IconType::Check, theme.background, 13.0))
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
                        div().flex().flex_col().child(
                            render_plain_row_inset(
                                "Token Budget",
                                Some("Max tokens the model context holds before compaction triggers."),
                                {
                                    let current = crate::config::ai_context_window();
                                    div()
                                        .flex_col()
                                        .gap_1()
                                        .child(
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
                                                                .on_click(cx.listener(move |this, _ev, _window, cx| {
                                                                    this.set_ai_context_window(size, cx);
                                                                }))
                                                                .child(label)
                                                        })
                                                ),
                                        )
                                        .when_some(self.ai_context_window_error.as_ref(), |control, error| {
                                            control.child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(theme.bright_red)
                                                    .child(error.clone()),
                                            )
                                        })
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
                        div().flex().flex_col()
                            .when(!self.detected_external_configs.is_empty(), |this| {
                                this.children(self.detected_external_configs.iter().enumerate().map(|(idx, detected)| {
                                    let app = detected.app;
                                    let path = detected.path.clone();
                                    let label = detected.label.clone();
                                    let details = detected.details.clone();
                                    let _is_last = idx == self.detected_external_configs.len() - 1 && self.import_status_message.is_none();

                                    render_plain_row_inset(
                                        label,
                                        Some(details),
                                        div()
                                            .px(px(14.))
                                            .py(px(8.))
                                            .rounded(px(8.))
                                            .bg(theme.surface_raised)
                                            .hover(|s| s.bg(theme.hover))
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.foreground)
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                this.import_external(app, path.clone(), cx);
                                            }))
                                            .child("Import & Adopt"),
                                        _is_last,
                                        theme,
                                    )
                                }))
                            })
                            .when(self.detected_external_configs.is_empty(), |this| {
                                this.child(
                                    div()
                                        .px(px(14.))
                                        .py(px(18.))
                                        .flex()
                                        .flex_col()
                                        .gap(px(4.))
                                        .child(
                                            div()
                                                .text_size(px(14.5))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(theme.foreground)
                                                .child("No external terminal configurations detected"),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .text_color(theme.muted)
                                                .child("Configurations for Ghostty, Warp, Alacritty, iTerm2, and WezTerm are checked automatically."),
                                        ),
                                )
                            })
                            .when_some(self.import_status_message.as_ref(), |this, msg| {
                                this.child(
                                    div()
                                        .px(px(14.))
                                        .py(px(10.))
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::MEDIUM)
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
                        div().flex().flex_col()
                            .child(render_plain_row_inset(
                                "Configuration Directory",
                                Some("~/.config/fastty"),
                                div()
                                    .px(px(14.))
                                    .py(px(8.))
                                    .rounded(px(8.))
                                    .bg(theme.surface_raised)
                                    .hover(|s| s.bg(theme.hover))
                                    .cursor(CursorStyle::PointingHand)
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                        Self::open_config_folder();
                                    })
                                    .child("Open Folder ↗"),
                                false,
                                theme,
                            ))
                            .child(render_plain_row_inset(
                                "fastty.toml",
                                Some("Directly edit raw configuration file"),
                                div()
                                    .px(px(14.))
                                    .py(px(8.))
                                    .rounded(px(8.))
                                    .bg(theme.surface_raised)
                                    .hover(|s| s.bg(theme.hover))
                                    .cursor(CursorStyle::PointingHand)
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
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
                        div().flex().flex_col()
                            .child(render_plain_row_inset(
                                "Version",
                                None::<&str>,
                                div()
                                    .px(px(9.))
                                    .py(px(4.))
                                    .rounded(px(6.))
                                    .bg(theme.surface_raised)
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.muted_strong)
                                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                                false,
                                theme,
                            ))
                            .child(render_plain_row_inset(
                                "Rendering Engine",
                                None::<&str>,
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(theme.muted_strong)
                                    .child("GPUI (Metal / DirectX)"),
                                false,
                                theme,
                            ))
                            .child(render_plain_row_inset(
                                "Platform Target",
                                None::<&str>,
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::NORMAL)
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

    fn scroll_theme_carousel(&mut self, direction: f32) {
        const STEP: f32 = 320.0;
        let max = self.theme_carousel_scroll.max_offset().x.as_f32().abs();
        let current = self.theme_carousel_scroll.offset().x.as_f32();
        self.theme_carousel_scroll
            .set_offset(gpui::point(px((current + direction * STEP).clamp(-max, 0.0)), px(0.)));
    }

    fn sync_settings_scroll_target(&mut self) {
        let offset = self.window_scroll_handle.offset().y.as_f32();
        self.settings_scroll_target = offset;
        self.settings_scroll_offset = offset;
    }

    fn handle_settings_scroll(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Precise deltas (trackpad and its momentum) already carry the
        // system's easing, so they follow the finger 1:1. A notched wheel
        // reports lines and gets the frame-locked glide below.
        let (delta, precise) = match event.delta {
            gpui::ScrollDelta::Pixels(delta) => (delta.y.as_f32(), true),
            gpui::ScrollDelta::Lines(delta) => (delta.y * 48.0, false),
        };
        if !delta.is_finite() || delta == 0.0 {
            self.sync_settings_scroll_target();
            return;
        }
        // `max_offset` is positive; offsets are negative (0 at the top).
        let limit = self.window_scroll_handle.max_offset().y.as_f32().abs();
        if !limit.is_finite() {
            return;
        }
        // Undo any delta the built-in scroll already applied, then move the
        // authoritative offset by exactly this event's delta.
        self.window_scroll_handle
            .set_offset(gpui::point(px(0.), px(self.settings_scroll_offset)));
        self.settings_scroll_target = (self.settings_scroll_target + delta).clamp(-limit, 0.0);
        if precise {
            self.settings_scroll_offset = self.settings_scroll_target;
            self.window_scroll_handle
                .set_offset(gpui::point(px(0.), px(self.settings_scroll_target)));
        } else if !self.settings_scroll_animating {
            self.settings_scroll_animating = true;
            self.settings_scroll_last_frame = None;
            let entity = cx.weak_entity();
            window.on_next_frame(move |window, app| {
                Self::step_settings_scroll(entity, window, app)
            });
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Advance one vsync frame of the notched-wheel glide and reschedule while
    /// the offset has not reached its target.
    fn step_settings_scroll(entity: WeakEntity<Self>, window: &mut Window, app: &mut App) {
        let continuing = entity
            .update(app, |this, cx| {
                let now = std::time::Instant::now();
                let dt = this
                    .settings_scroll_last_frame
                    .replace(now)
                    .map(|last| now.duration_since(last).as_secs_f32())
                    .unwrap_or(0.016)
                    .min(0.05);
                // Exponential approach: 50 ms covers ~63% of the distance.
                const TIME_CONSTANT: f32 = 0.05;
                let blend = 1.0 - (-dt / TIME_CONSTANT).exp();
                let target = this.settings_scroll_target;
                let next =
                    this.settings_scroll_offset + (target - this.settings_scroll_offset) * blend;
                let settled = (target - next).abs() <= 0.25;
                this.settings_scroll_offset = if settled { target } else { next };
                this.window_scroll_handle
                    .set_offset(gpui::point(px(0.), px(this.settings_scroll_offset)));
                this.settings_scroll_animating = !settled;
                cx.notify();
                !settled
            })
            .unwrap_or(false);
        if continuing {
            window.on_next_frame(move |window, app| {
                Self::step_settings_scroll(entity, window, app)
            });
        }
    }

    fn handle_menu_outside_click(
        &mut self,
        ev: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_model_dropdown_open {
            self.ai_model_dropdown_open = false;
            cx.notify();
        }
        if !self.font_combobox_open {
            return;
        }
        let is_font_trigger = self.font_trigger_bounds.get().is_some_and(|bounds| {
            let x = ev.position.x.to_f64();
            let y = ev.position.y.to_f64();
            x >= bounds.left().to_f64()
                && x <= bounds.right().to_f64()
                && y >= bounds.top().to_f64()
                && y <= bounds.bottom().to_f64()
        });
        if !is_font_trigger {
            self.font_combobox_open = false;
            if self.active_input_field == Some(ActiveInputField::FontSearch) {
                self.active_input_field = None;
            }
            cx.notify();
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
        // UX3: detect on-disk edits made outside this window (throttled).
        self.refresh_stale_state();
        let view = div().track_focus(&self.focus_handle);
        #[cfg(target_os = "macos")]
        let view = view
            .on_key_down(cx.listener(Self::handle_native_key_down))
            .on_action(cx.listener(|this, _: &crate::ui::native_menu::Find, window, cx| {
                this.active_input_field = Some(ActiveInputField::Search);
                this.font_combobox_open = false;
                window.focus(&this.focus_handle, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::ui::native_menu::OpenSettings, window, cx| {
                window.activate_window();
                window.focus(&this.focus_handle, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ui::native_menu::CheckForUpdates, window, cx| {
                this.active_tab = SettingsTab::General;
                    this.sync_settings_scroll_target();
                this.check_updates_now(window, cx);
                cx.notify();
            }))
            .when(self.active_input_field.is_some() && self.capturing_action.is_none(), |view| {
                view.on_action(cx.listener(|this, _: &crate::ui::native_menu::Copy, _, _| {
                    this.copy_active_input();
                }))
                .on_action(cx.listener(|this, _: &crate::ui::native_menu::Paste, _, cx| {
                    this.paste_active_input(cx);
                }))
                .on_action(cx.listener(|this, _: &crate::ui::native_menu::SelectAll, _, cx| {
                    this.select_all_active_input(cx);
                }))
            });
        #[cfg(not(target_os = "macos"))]
        let view = view.on_key_down(cx.listener(Self::handle_key_down));
        view.key_context("SettingsView")
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_menu_outside_click))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::handle_menu_outside_click))
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
            .when(self.celebration_generation > 0, |this| {
                this.child(self.render_celebration())
            })
    }
}

impl SettingsView {
}

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_selection_keeps_current_models_and_variants() {
        let models = vec!["first-model".to_string(), "selected-model".to_string()];
        let variants = std::collections::HashMap::from([
            ("selected-model".to_string(), vec!["high".to_string()]),
        ]);
        for current in ["selected-model", "selected-model/high"] {
            assert_eq!(super::SettingsView::catalog_model_selection(
                Some(current), &models, &variants,
            ).as_deref(), Some(current));
        }
    }

    #[test]
    fn catalog_selection_replaces_removed_models_and_variants() {
        let models = vec!["available-model".to_string()];
        let variants = std::collections::HashMap::new();
        for current in [Some("removed-model"), Some("available-model/removed-variant"), None] {
            assert_eq!(super::SettingsView::catalog_model_selection(
                current, &models, &variants,
            ).as_deref(), Some("available-model"));
        }
    }

    use super::SettingsView;

    #[test]
    fn test_stale_state_same_hash_is_fresh() {
        assert!(!SettingsView::is_stale_state(Some(1), Some(1), None));
        assert!(!SettingsView::is_stale_state(None, None, None));
    }

    #[test]
    fn test_stale_state_external_edit_is_stale() {
        assert!(SettingsView::is_stale_state(Some(2), Some(1), None));
        // File created or deleted counts as a change too.
        assert!(SettingsView::is_stale_state(Some(1), None, None));
        assert!(SettingsView::is_stale_state(None, Some(1), None));
    }

    #[test]
    fn test_stale_state_dismiss_hides_until_next_change() {
        // Dismissed exactly this content: hidden.
        assert!(!SettingsView::is_stale_state(Some(2), Some(1), Some(2)));
        // File changed again after dismiss: shown.
        assert!(SettingsView::is_stale_state(Some(3), Some(1), Some(2)));
    }
}
