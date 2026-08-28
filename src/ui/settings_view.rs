use gpui::{
    Context, CursorStyle, FontWeight, KeyDownEvent, MouseButton, Render, ScrollHandle,
    SharedString, Window, WindowHandle, div, prelude::*, px,
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

fn filter_programming_fonts(all_fonts: Vec<String>, current_font: &str) -> Vec<String> {
    let coding_keywords = [
        "mono", "code", "typewriter", "console", "courier", "hack", "menlo", "monaco",
        "consolas", "inconsolata", "fira", "jetbrains", "cascadia", "meslo", "source", "dejavu",
    ];

    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if !current_font.is_empty() {
        result.push(current_font.to_string());
        seen.insert(current_font.to_lowercase());
    }

    for font in all_fonts {
        let lower = font.to_lowercase();
        let matches = coding_keywords.iter().any(|k| lower.contains(k));
        if matches && !seen.contains(&lower) {
            seen.insert(lower);
            result.push(font);
        }
    }

    if result.is_empty() {
        result = vec![
            "Menlo".to_string(),
            "Monaco".to_string(),
            "Courier New".to_string(),
            "Fira Code".to_string(),
            "JetBrains Mono".to_string(),
        ];
    }

    result
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
    pub focus_handle: gpui::FocusHandle,
    pub window_scroll_handle: ScrollHandle,
    pub system_fonts: Vec<String>,
    pub theme_cards: Vec<ThemeCardInfo>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        config::load_custom_themes();
        let loaded_config = Config::load().unwrap_or_default();
        let theme_name = loaded_config.theme.as_deref().unwrap_or("default").to_string();
        let theme = Theme::from_name(&theme_name).with_opacity(loaded_config.opacity);
        let tab_layout = loaded_config.tab_layout;

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
        let raw_fonts = available_system_fonts();
        let system_fonts = filter_programming_fonts(raw_fonts, &font_family);

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

        Self {
            opacity: loaded_config.opacity,
            cursor_blink: loaded_config.cursor.blink,
            copy_on_select: loaded_config.copy_on_select,
            tab_layout,
            config: loaded_config,
            theme,
            current_theme_name: theme_name,
            font_size,
            font_family,
            focus_handle,
            window_scroll_handle: ScrollHandle::new(),
            system_fonts,
            theme_cards,
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

    pub fn set_tab_layout(&mut self, layout: crate::config::TabLayout, cx: &mut Context<Self>) {
        self.tab_layout = layout;
        self.config.tab_layout = layout;
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

fn render_toggle_switch(is_on: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(if is_on { "toggle-on" } else { "toggle-off" })
        .flex()
        .items_center()
        .w(px(38.))
        .h(px(22.))
        .rounded_full()
        .bg(if is_on { theme.accent } else { theme.surface })
        .border_1()
        .border_color(if is_on { theme.accent } else { theme.border })
        .p(px(2.))
        .cursor(CursorStyle::PointingHand)
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .rounded_full()
                .bg(if is_on { theme.black } else { theme.muted })
                .when(is_on, |this| this.ml(px(16.))),
        )
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
                    .id("settings-topbar")
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(32.))
                    .w_full()
                    .bg(theme.tab_bar_bg)
                    .border_b_1()
                    .border_color(theme.border)
                    .when(cfg!(target_os = "macos"), |d| d.pl(px(76.)).pr(px(12.)))
                    .when(!cfg!(target_os = "macos"), |d| d.pl(px(10.)).pr(px(8.)))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .when(!cfg!(target_os = "macos"), |this| {
                                this.child(render_app_logo(15.0))
                            })
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .child("Settings"),
                            )
                            .child(
                                div()
                                    .px(px(5.))
                                    .py(px(1.))
                                    .rounded(px(4.))
                                    .bg(theme.surface_raised)
                                    .text_size(px(9.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.muted)
                                    .child("v0.7.1"),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-drag-spacer")
                            .flex_1()
                            .h_full()
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
                            .gap_2p5()
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(theme.muted)
                                    .child("Press Esc to close"),
                            )
                            .when(cfg!(target_os = "macos"), |this| {
                                this.child(render_app_logo(15.0))
                            })
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
                    .id("settings-scroll-content")
                    .track_scroll(&self.window_scroll_handle)
                    .flex_1()
                    .overflow_y_scroll()
                    .p(px(24.))
                    .flex()
                    .flex_col()
                    .gap_6()
                    // 1. Tab Bar Layout Card Section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2p5()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(render_icon(IconType::Folder, theme.accent, 12.0))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_strong)
                                            .child("TAB BAR LAYOUT"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_3()
                                    .child({
                                        let is_active = self.tab_layout == crate::config::TabLayout::Horizontal;
                                        div()
                                            .id("layout-horizontal-btn")
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .p(px(12.))
                                            .rounded(px(8.))
                                            .bg(if is_active { theme.surface_raised } else { theme.surface })
                                            .border_1()
                                            .border_color(if is_active { theme.accent } else { theme.border })
                                            .hover(move |s| s.bg(theme.surface_raised))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.set_tab_layout(crate::config::TabLayout::Horizontal, cx);
                                            }))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                                            .child("Horizontal (Top Bar)"),
                                                    )
                                                    .when(is_active, |this| {
                                                        this.child(render_icon(IconType::Check, theme.accent, 13.0))
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(theme.muted)
                                                    .child("Tabs rendered horizontally at window top"),
                                            )
                                    })
                                    .child({
                                        let is_active = self.tab_layout == crate::config::TabLayout::Vertical;
                                        div()
                                            .id("layout-vertical-btn")
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .p(px(12.))
                                            .rounded(px(8.))
                                            .bg(if is_active { theme.surface_raised } else { theme.surface })
                                            .border_1()
                                            .border_color(if is_active { theme.accent } else { theme.border })
                                            .hover(move |s| s.bg(theme.surface_raised))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                this.set_tab_layout(crate::config::TabLayout::Vertical, cx);
                                            }))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(super::icons::render_sidebar_icon(if is_active { theme.accent } else { theme.muted_strong }, 13.0))
                                                            .child(
                                                                div()
                                                                    .text_size(px(12.))
                                                                    .font_weight(FontWeight::BOLD)
                                                                    .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                                                    .child("Vertical (Sidebar)"),
                                                            ),
                                                    )
                                                    .when(is_active, |this| {
                                                        this.child(render_icon(IconType::Check, theme.accent, 13.0))
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(theme.muted)
                                                    .child("Zed-style animated collapsible tab sidebar"),
                                            )
                                    }),
                            ),
                    )
                    // 2. Theme Section with Swatches
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2p5()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(render_icon(IconType::Palette, theme.accent, 12.0))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_strong)
                                            .child("COLOR THEME"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(
                                        self.theme_cards.iter().map(|card| {
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
                                        }),
                                    ),
                            ),
                    )
                    // 3. Typography Card (Font Size & Font Family)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2p5()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(render_icon(IconType::FileCode, theme.accent, 12.0))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_strong)
                                            .child("TYPOGRAPHY"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .p(px(14.))
                                    .rounded(px(8.))
                                    .bg(theme.surface_raised)
                                    .border_1()
                                    .border_color(theme.border)
                                    // Font Size Stepper
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(theme.foreground)
                                                            .child("Font Size"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted)
                                                            .child("Terminal buffer grid cell size"),
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
                                                            .w(px(28.))
                                                            .h(px(28.))
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .rounded(px(6.))
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
                                    // Font Family List
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
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(theme.foreground)
                                                            .child("Font Family"),
                                                    )
                                                    .child(
                                                        div()
                                                            .px(px(6.))
                                                            .py(px(1.5))
                                                            .rounded(px(4.))
                                                            .bg(theme.surface)
                                                            .text_size(px(10.5))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(theme.accent)
                                                            .child(self.font_family.clone()),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .flex_wrap()
                                                    .gap_1p5()
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
                                                                    .gap_1p5()
                                                                    .px(px(8.))
                                                                    .py(px(5.))
                                                                    .rounded(px(5.))
                                                                    .bg(if is_active { theme.surface_raised } else { theme.surface })
                                                                    .border_1()
                                                                    .border_color(if is_active { theme.accent } else { theme.border })
                                                                    .hover(move |s| s.bg(theme.surface_raised))
                                                                    .text_color(if is_active { theme.foreground } else { theme.muted_strong })
                                                                    .text_size(px(11.))
                                                                    .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                                    .cursor(CursorStyle::PointingHand)
                                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                        this.set_font_family(&name_clone, cx);
                                                                    }))
                                                                    .child(font_name.clone())
                                                                    .when(is_active, |el| el.child(render_icon(IconType::Check, theme.accent, 11.0)))
                                                            }),
                                                    ),
                                            ),
                                    ),
                            ),
                    )
                    // 4. Window & Behavior Section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2p5()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(render_icon(IconType::Settings, theme.accent, 12.0))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_strong)
                                            .child("WINDOW & BEHAVIOR"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .p(px(14.))
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
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(theme.foreground)
                                                            .child("Window Opacity"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted)
                                                            .child("Background transparency level"),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .gap_1p5()
                                                    .children([1.0, 0.95, 0.90, 0.85, 0.75].into_iter().map(|val| {
                                                        let is_active = (self.opacity - val).abs() < 0.02;
                                                        let label = format!("{:.0}%", val * 100.0);
                                                        div()
                                                            .px(px(8.))
                                                            .py(px(4.))
                                                            .rounded(px(5.))
                                                            .bg(if is_active { theme.accent } else { theme.surface })
                                                            .text_color(if is_active { theme.black } else { theme.foreground })
                                                            .text_size(px(11.))
                                                            .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::MEDIUM })
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
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(theme.foreground)
                                                            .child("Cursor Blinking"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted)
                                                            .child("Smooth animated terminal cursor blinking"),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                        this.toggle_cursor_blink(cx);
                                                    }))
                                                    .child(render_toggle_switch(self.cursor_blink, &theme)),
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
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(theme.foreground)
                                                            .child("Copy on Select"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(theme.muted)
                                                            .child("Automatically copy text when selecting with mouse"),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                        this.toggle_copy_on_select(cx);
                                                    }))
                                                    .child(render_toggle_switch(self.copy_on_select, &theme)),
                                            ),
                                    ),
                            ),
                    )
                    // 5. Config File Action Buttons
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_3()
                            .child(
                                div()
                                    .id("edit-config-toml-btn")
                                    .flex_1()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .py(px(10.))
                                    .rounded(px(8.))
                                    .bg(theme.surface_raised)
                                    .hover(|s| s.bg(theme.hover))
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                        Self::open_settings_file();
                                    })
                                    .child(render_icon(IconType::FileCode, theme.accent, 13.0))
                                    .child("Edit fastty.toml ↗"),
                            )
                            .child(
                                div()
                                    .id("open-config-folder-btn")
                                    .flex_1()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .py(px(10.))
                                    .rounded(px(8.))
                                    .bg(theme.surface_raised)
                                    .hover(|s| s.bg(theme.hover))
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                        Self::open_config_folder();
                                    })
                                    .child(render_icon(IconType::FolderOpen, theme.accent, 13.0))
                                    .child("Open Config Folder ↗"),
                            ),
                    ),
            )
    }
}
