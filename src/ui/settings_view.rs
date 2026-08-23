use gpui::{
    Context, CursorStyle, FontWeight, KeyDownEvent, MouseButton, Render, ScrollHandle,
    Window, WindowHandle, div, prelude::*, px,
};
use icons::common::IconType;
use parking_lot::Mutex;
use crate::config::{self, Config};
use crate::ui::icons::{render_app_logo, render_icon};
use crate::ui::root_view::{available_system_fonts, open_path_or_url};
use crate::ui::theme::Theme;

pub static SETTINGS_WINDOW_HANDLE: Mutex<Option<WindowHandle<SettingsView>>> = Mutex::new(None);

pub struct SettingsView {
    pub config: Config,
    pub theme: Theme,
    pub current_theme_name: String,
    pub font_size: f32,
    pub font_family: String,
    pub opacity: f32,
    pub cursor_blink: bool,
    pub copy_on_select: bool,
    pub focus_handle: gpui::FocusHandle,
    pub font_scroll_handle: ScrollHandle,
    pub window_scroll_handle: ScrollHandle,
    pub system_fonts: Vec<String>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        config::load_custom_themes();
        let loaded_config = Config::load().unwrap_or_default();
        let theme_name = loaded_config.theme.as_deref().unwrap_or("default").to_string();
        let theme = Theme::from_name(&theme_name).with_opacity(loaded_config.opacity);

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
        let system_fonts = available_system_fonts();

        Self {
            opacity: loaded_config.opacity,
            cursor_blink: loaded_config.cursor.blink,
            copy_on_select: loaded_config.copy_on_select,
            config: loaded_config,
            theme,
            current_theme_name: theme_name,
            font_size,
            font_family,
            focus_handle,
            font_scroll_handle: ScrollHandle::new(),
            window_scroll_handle: ScrollHandle::new(),
            system_fonts,
        }
    }

    fn handle_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            *SETTINGS_WINDOW_HANDLE.lock() = None;
            window.remove_window();
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

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;

        div()
            .track_focus(&self.focus_handle)
            .key_context("SettingsView")
            .on_key_down(cx.listener(Self::handle_key_down))
            .w_full()
            .h_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .flex()
            .flex_col()
            .overflow_hidden()
            // Topbar
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(38.))
                    .w_full()
                    .bg(theme.tab_bar_bg)
                    .border_b_1()
                    .border_color(theme.border)
                    .when(cfg!(target_os = "macos"), |d| d.pl(px(76.)).pr(px(14.)))
                    .when(!cfg!(target_os = "macos"), |d| d.px(px(12.)))
                    .on_mouse_down(MouseButton::Left, |ev, window, _cx| {
                        if ev.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(render_app_logo(16.0))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .child("Settings"),
                            ),
                    )
                    .when(!cfg!(target_os = "macos"), |this| {
                        this.child(
                            div()
                                .id("settings-win-close")
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(28.))
                                .h(px(22.))
                                .rounded(px(4.))
                                .hover(move |s| s.bg(gpui::Hsla { h: 0.0, s: 0.7, l: 0.45, a: 1.0 }))
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, |_ev, window, _cx| {
                                    *SETTINGS_WINDOW_HANDLE.lock() = None;
                                    window.remove_window();
                                })
                                .child(render_icon(IconType::X, theme.foreground, 10.0)),
                        )
                    }),
            )
            // Scrollable Content
            .child(
                div()
                    .id("settings-scroll-content")
                    .track_scroll(&self.window_scroll_handle)
                    .flex_1()
                    .overflow_y_scroll()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap_5()
                    // 1. Theme Section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.muted_strong)
                                    .child("THEME"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_wrap()
                                    .gap_1p5()
                                    .children(
                                        crate::config::all_theme_names().into_iter().map(|name| {
                                            let is_active = self.current_theme_name == name;
                                            let theme_name_str = name.clone();
                                            let label = match name.as_str() {
                                                "default" => "Default".to_string(),
                                                "catppuccin" => "Catppuccin".to_string(),
                                                "one-dark" => "One Dark".to_string(),
                                                "solarized-dark" => "Solarized".to_string(),
                                                "high-contrast" => "High Contrast".to_string(),
                                                other => other.to_string(),
                                            };
                                            div()
                                                .px(px(10.))
                                                .py(px(5.))
                                                .rounded(px(6.))
                                                .bg(if is_active { theme.accent } else { theme.surface_raised })
                                                .border_1()
                                                .border_color(if is_active { theme.accent } else { theme.border })
                                                .text_color(if is_active { theme.black } else { theme.foreground })
                                                .text_size(px(11.5))
                                                .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                    this.set_theme(&theme_name_str, cx);
                                                }))
                                                .child(label)
                                        }),
                                    ),
                            ),
                    )
                    // 2. Font Size Section
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .p(px(12.))
                            .rounded(px(8.))
                            .bg(theme.surface_raised)
                            .border_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_strong)
                                            .child("FONT SIZE"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme.muted)
                                            .child("Terminal buffer font size in pixels"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(26.))
                                            .h(px(26.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(5.))
                                            .bg(theme.surface)
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
                                            .min_w(px(40.))
                                            .flex()
                                            .justify_center()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.foreground)
                                            .child(format!("{:.0}px", self.font_size)),
                                    )
                                    .child(
                                        div()
                                            .w(px(26.))
                                            .h(px(26.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(5.))
                                            .bg(theme.surface)
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
                            ),
                    )
                    // 3. Font Family Section
                    .child(
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
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_strong)
                                            .child("FONT FAMILY"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.accent)
                                            .child(self.font_family.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("settings-font-family-list")
                                    .track_scroll(&self.font_scroll_handle)
                                    .flex()
                                    .flex_col()
                                    .max_h(px(160.))
                                    .overflow_y_scroll()
                                    .p(px(4.))
                                    .rounded(px(8.))
                                    .bg(theme.surface_raised)
                                    .border_1()
                                    .border_color(theme.border)
                                    .gap_1()
                                    .on_scroll_wheel(|_ev, _window, cx| {
                                        cx.stop_propagation();
                                    })
                                    .children(
                                        self.system_fonts
                                            .iter()
                                            .map(|font_name| {
                                                let is_active = self.font_family == *font_name;
                                                let name_clone = font_name.clone();
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .px(px(10.))
                                                    .py(px(5.))
                                                    .rounded(px(5.))
                                                    .bg(if is_active { theme.accent } else { theme.surface })
                                                    .hover(|s| if !is_active { s.bg(theme.hover) } else { s })
                                                    .text_color(if is_active { theme.black } else { theme.foreground })
                                                    .text_size(px(11.5))
                                                    .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                        this.set_font_family(&name_clone, cx);
                                                    }))
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .overflow_hidden()
                                                            .child(font_name.clone()),
                                                    )
                                                    .when(is_active, |el| el.child(render_icon(IconType::Check, theme.black, 12.0)))
                                            }),
                                    ),
                            ),
                    )
                    // 4. Window & Behavior Section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.muted_strong)
                                    .child("WINDOW & BEHAVIOR"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .p(px(12.))
                                    .rounded(px(8.))
                                    .bg(theme.surface_raised)
                                    .border_1()
                                    .border_color(theme.border)
                                    // Opacity
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_size(px(11.5))
                                                    .text_color(theme.foreground)
                                                    .child("Window Opacity"),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .gap_1()
                                                    .children([1.0, 0.95, 0.90, 0.85, 0.75].into_iter().map(|val| {
                                                        let is_active = (self.opacity - val).abs() < 0.02;
                                                        let label = format!("{:.0}%", val * 100.0);
                                                        div()
                                                            .px(px(6.))
                                                            .py(px(2.))
                                                            .rounded(px(4.))
                                                            .bg(if is_active { theme.accent } else { theme.surface })
                                                            .text_color(if is_active { theme.black } else { theme.foreground })
                                                            .text_size(px(10.5))
                                                            .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                this.adjust_opacity(val, cx);
                                                            }))
                                                            .child(label)
                                                    })),
                                            ),
                                    )
                                    // Cursor Blink
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_size(px(11.5))
                                                    .text_color(theme.foreground)
                                                    .child("Cursor Blinking"),
                                            )
                                            .child(
                                                div()
                                                    .px(px(8.))
                                                    .py(px(2.))
                                                    .rounded(px(4.))
                                                    .bg(if self.cursor_blink { theme.accent } else { theme.surface })
                                                    .text_color(if self.cursor_blink { theme.black } else { theme.muted })
                                                    .text_size(px(10.5))
                                                    .font_weight(FontWeight::BOLD)
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                        this.toggle_cursor_blink(cx);
                                                    }))
                                                    .child(if self.cursor_blink { "ON" } else { "OFF" }),
                                            ),
                                    )
                                    // Copy on select
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_size(px(11.5))
                                                    .text_color(theme.foreground)
                                                    .child("Copy on Select"),
                                            )
                                            .child(
                                                div()
                                                    .px(px(8.))
                                                    .py(px(2.))
                                                    .rounded(px(4.))
                                                    .bg(if self.copy_on_select { theme.accent } else { theme.surface })
                                                    .text_color(if self.copy_on_select { theme.black } else { theme.muted })
                                                    .text_size(px(10.5))
                                                    .font_weight(FontWeight::BOLD)
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                        this.toggle_copy_on_select(cx);
                                                    }))
                                                    .child(if self.copy_on_select { "ON" } else { "OFF" }),
                                            ),
                                    ),
                            ),
                    )
                    // 5. Config File Action Buttons
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .py(px(8.))
                                    .rounded(px(6.))
                                    .bg(theme.surface_raised)
                                    .hover(|s| s.bg(theme.hover))
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.foreground)
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                        Self::open_settings_file();
                                    })
                                    .child("Edit config.toml ↗"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .py(px(8.))
                                    .rounded(px(6.))
                                    .bg(theme.surface_raised)
                                    .hover(|s| s.bg(theme.hover))
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.foreground)
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                        Self::open_config_folder();
                                    })
                                    .child("Open Config Folder ↗"),
                            ),
                    ),
            )
    }
}
