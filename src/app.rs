//! AppView - Main application shell for fasty with tabs, titlebar, and settings.
//!
//! Manages multiple terminal tabs with a custom titlebar and settings panel.

use crate::config::Config;
use crate::settings::SettingsView;
use crate::terminal::TerminalView;
use gpui::{
    div, hsla, px, App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, Styled, Window, WindowBounds, WindowDecorations,
    WindowOptions,
};

#[derive(Clone)]
pub struct Tab {
    pub title: String,
    pub id: usize,
}

pub enum ActiveView {
    Terminal,
    Settings,
}

pub struct AppView {
    tabs: Vec<Tab>,
    active_tab: usize,
    terminal_views: Vec<Entity<TerminalView>>,
    show_settings: bool,
    focus_handle: FocusHandle,
    config: Config,
    new_tab_cb: Option<Box<dyn Fn(&mut Window, &mut Context<Self>) + 'static>>,
    settings_window: Option<gpui::WindowHandle<SettingsView>>,
}

impl AppView {
    pub fn new(
        cx: &mut Context<Self>,
        terminal_view: Entity<TerminalView>,
        config: Config,
        new_tab_cb: Option<Box<dyn Fn(&mut Window, &mut Context<Self>) + 'static>>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        Self {
            tabs: vec![Tab {
                title: "Terminal 1".to_string(),
                id: 0,
            }],
            active_tab: 0,
            terminal_views: vec![terminal_view],
            show_settings: false,
            focus_handle,
            config,
            new_tab_cb,
            settings_window: None,
        }
    }

    pub fn toggle_settings(&mut self) {
        self.show_settings = !self.show_settings;
    }

    pub fn save_settings(&mut self, new_config: Config) {
        self.config = new_config.clone();
        if let Err(e) = new_config.save(&Config::config_path()) {
            tracing::error!("failed to save config: {}", e);
        }
    }

    pub fn active_terminal_view(&self) -> Option<Entity<TerminalView>> {
        self.terminal_views.get(self.active_tab).cloned()
    }

    pub fn add_terminal_view(&mut self, terminal_view: Entity<TerminalView>) {
        let tab_id = self.tabs.len();
        self.tabs.push(Tab {
            title: format!("Terminal {}", tab_id + 1),
            id: tab_id,
        });
        self.terminal_views.push(terminal_view);
        self.active_tab = self.tabs.len() - 1;
    }

    pub fn set_new_tab_cb(
        &mut self,
        cb: Option<Box<dyn Fn(&mut Window, &mut Context<Self>) + 'static>>,
    ) {
        self.new_tab_cb = cb;
    }

    pub fn update_config_for_all_terminals(&self, config: Config, cx: &mut Context<Self>) {
        for terminal_view in &self.terminal_views {
            terminal_view.update(cx, |tv, cx| {
                tv.update_font_config(config.font.clone(), cx);
            });
        }
    }

    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

impl Focusable for AppView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AppView {
    fn on_settings_click(
        &mut self,
        _event: &gpui::MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();

        if let Some(settings_window) = self.settings_window.clone() {
            let _ = settings_window.update(cx, |_, window, _| {
                window.remove_window();
            });
            self.settings_window = None;
            return;
        }

        let config = self.config.clone();
        let app_view_entity = cx.entity();

        let settings_window_bounds = gpui::Bounds::new(
            gpui::point(px(100.0), px(100.0)),
            gpui::size(px(520.0), px(560.0)),
        );

        let settings_window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(settings_window_bounds)),
                    window_decorations: Some(WindowDecorations::Client),
                    titlebar: None,
                    is_resizable: false,
                    focus: true,
                    ..Default::default()
                },
                move |_window, cx| cx.new(|cx| SettingsView::new(cx, config, app_view_entity)),
            )
            .expect("Failed to open settings window");

        self.settings_window = Some(settings_window);
    }

    fn on_new_tab_click(
        &mut self,
        _event: &gpui::MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if let Some(cb) = self.new_tab_cb.as_ref() {
            cb(_window, cx);
        }
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _new_tab_cb = self.new_tab_cb.take();

        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this: &mut Self, event: &gpui::KeyDownEvent, _window: &mut Window, _cx: &mut Context<Self>| {
                if event.keystroke.modifiers.control {
                    match event.keystroke.key.as_str() {
                        "t" => {
                            if let Some(cb) = this.new_tab_cb.as_ref() {
                                cb(_window, _cx);
                            }
                        }
                        "w" => {
                            if this.tabs.len() > 1 {
                                this.tabs.remove(this.active_tab);
                                this.terminal_views.remove(this.active_tab);
                                if this.active_tab >= this.tabs.len() {
                                    this.active_tab = this.tabs.len().saturating_sub(1);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }))
            .child(
                div()
                    .h(px(28.0))
                    .bg(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .cursor_pointer()
                            .on_mouse_down(gpui::MouseButton::Left, |_event, window, _cx| {
                                window.start_window_move();
                            })
                            .child(
                                div()
                                    .pl(px(8.0))
                                    .text_xs()
                                    .text_color(hsla(0.0, 0.0, 0.6, 1.0))
                                    .font_weight(gpui::FontWeight(600.0))
                                    .child("fasty"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h_full()
                            .child(
                                div()
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .flex()
                                    .justify_center()
                                    .items_center()
                                    .cursor_pointer()
                                    .rounded_2xl()
                                    .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                    .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)).text_color(hsla(0.0, 0.0, 0.8, 1.0)))
                                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, event: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Self>| {
                                        this.on_settings_click(event, window, cx);
                                    }))
                                    .child(div().text_sm().child("⚙")),
                            )
                            .child(
                                div()
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .flex()
                                    .justify_center()
                                    .items_center()
                                    .cursor_pointer()
                                    .rounded_2xl()
                                    .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                    .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)).text_color(hsla(0.0, 0.0, 0.8, 1.0)))
                                    .on_mouse_down(gpui::MouseButton::Left, |_event, window, cx| {
                                        cx.stop_propagation();
                                        window.minimize_window();
                                    })
                                    .child(div().text_sm().child("─")),
                            )
                            .child(
                                div()
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .flex()
                                    .justify_center()
                                    .items_center()
                                    .cursor_pointer()
                                    .rounded_2xl()
                                    .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                    .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)).text_color(hsla(0.0, 0.0, 0.8, 1.0)))
                                    .on_mouse_down(gpui::MouseButton::Left, |_event, window, cx| {
                                        cx.stop_propagation();
                                        window.zoom_window();
                                    })
                                    .child(div().text_sm().child("▢")),
                            )
                            .child(
                                div()
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .flex()
                                    .justify_center()
                                    .items_center()
                                    .cursor_pointer()
                                    .rounded_2xl()
                                    .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                    .hover(|div| div.bg(hsla(0.0, 0.85, 0.45, 1.0)).text_color(hsla(0.0, 0.0, 1.0, 1.0)))
                                    .on_mouse_down(gpui::MouseButton::Left, |_event, _window, cx| {
                                        cx.stop_propagation();
                                        cx.quit();
                                    })
                                    .child(div().text_sm().child("✕")),
                            ),
                    ),
            )
            .child(
                div()
                    .h(px(24.0))
                    .bg(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .items_end()
                    .children(self.tabs.iter().enumerate().map(|(i, tab)| {
                        let is_active = i == self.active_tab;
                        let tab_id = tab.id;
                        div()
                            .id(tab.id)
                            .px(px(8.0))
                            .pb(px(2.0))
                            .text_xs()
                            .text_color(if is_active { hsla(0.0, 0.0, 0.9, 1.0) } else { hsla(0.0, 0.0, 0.55, 1.0) })
                            .border_b(px(2.0))
                            .border_color(if is_active { hsla(0.0, 0.0, 0.4, 1.0) } else { hsla(0.0, 0.0, 0.0, 0.0) })
                            .cursor_pointer()
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, _: &mut Context<Self>| {
                                this.active_tab = tab_id;
                            }))
                            .child(tab.title.clone())
                    }))
                    .child(
                        div()
                            .w(px(24.0))
                            .h(px(18.0))
                            .mb(px(2.0))
                            .rounded_2xl()
                            .text_xs()
                            .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)))
                            .cursor_pointer()
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, event: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Self>| {
                                this.on_new_tab_click(event, window, cx);
                            }))
                            .child("+"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .size_full()
                    .children(self.active_terminal_view().map(|tv| tv.into_element())),
            )
    }
}
