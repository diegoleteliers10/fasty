//! Settings view for fasty terminal.
//!
//! A modal settings panel for configuring font, appearance, and terminal options.

use crate::config::Config;
use gpui::{
    div, hsla, px, App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Window,
};

pub struct SettingsView {
    config: Config,
    focus_handle: FocusHandle,
    original_config: Config,
    app_view: gpui::Entity<crate::app::AppView>,
    font_list: Vec<String>,
    show_font_picker: bool,
    font_picker_search: String,
}

impl SettingsView {
    pub fn new(
        cx: &mut Context<Self>,
        config: Config,
        app_view: gpui::Entity<crate::app::AppView>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        Self {
            config: config.clone(),
            focus_handle,
            original_config: config,
            app_view,
            font_list: Vec::new(),
            show_font_picker: false,
            font_picker_search: String::new(),
        }
    }

    pub fn set_app_view(&mut self, app_view: gpui::Entity<crate::app::AppView>) {
        self.app_view = app_view;
    }

    pub fn update_font_family(&mut self, family: String) {
        self.config.font.family = family;
    }

    pub fn update_font_size(&mut self, size: f32) {
        self.config.font.size = size;
    }

    pub fn update_font_weight(&mut self, weight: f32) {
        self.config.font.weight = weight;
    }

    pub fn update_ligatures(&mut self, enabled: bool) {
        self.config.font.ligatures = enabled;
    }

    pub fn update_scrollback(&mut self, lines: usize) {
        self.config.scrollback = lines;
    }

    fn increment_font_size(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.config.font.size = (self.config.font.size + 1.0).min(72.0);
        cx.notify();
    }

    fn decrement_font_size(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.config.font.size = (self.config.font.size - 1.0).max(6.0);
        cx.notify();
    }

    fn increment_font_weight(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.config.font.weight = (self.config.font.weight + 100.0).min(900.0);
        cx.notify();
    }

    fn decrement_font_weight(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.config.font.weight = (self.config.font.weight - 100.0).max(100.0);
        cx.notify();
    }

    fn toggle_ligatures(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.config.font.ligatures = !self.config.font.ligatures;
        cx.notify();
    }

    fn open_font_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.font_list = window.text_system().all_font_names();
        self.show_font_picker = true;
        cx.notify();
    }

    fn toggle_font_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.show_font_picker {
            self.font_list = window.text_system().all_font_names();
        }
        self.show_font_picker = !self.show_font_picker;
        cx.notify();
    }

    fn close_font_picker(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.show_font_picker = false;
        self.font_picker_search.clear();
        cx.notify();
    }

    fn select_font(&mut self, family: String, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.config.font.family = family;
        self.show_font_picker = false;
        self.font_picker_search.clear();
        cx.notify();
    }

    fn filtered_font_list(&self) -> Vec<&String> {
        if self.font_picker_search.is_empty() {
            self.font_list.iter().collect()
        } else {
            let search = self.font_picker_search.to_lowercase();
            self.font_list
                .iter()
                .filter(|f| f.to_lowercase().contains(&search))
                .collect()
        }
    }

    fn handle_save(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();

        let new_config = self.config.clone();

        self.app_view.update(cx, |app, cx| {
            app.save_settings(new_config.clone());
            app.update_config_for_all_terminals(new_config, cx);
        });

        for window in cx.windows().iter() {
            if window.downcast::<SettingsView>().is_some() {
                let _ = window.update(cx, |_, w, _| {
                    w.remove_window();
                });
                break;
            }
        }
    }

    fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();

        for window in cx.windows().iter() {
            if window.downcast::<SettingsView>().is_some() {
                let _ = window.update(cx, |_, w, _| {
                    w.remove_window();
                });
                break;
            }
        }
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(0.0, 0.0, 0.0, 1.0))
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .h(px(28.0))
                    .bg(hsla(0.0, 0.0, 0.02, 1.0))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .pl(px(8.0))
                            .text_xs()
                            .text_color(hsla(0.0, 0.0, 0.6, 1.0))
                            .font_weight(gpui::FontWeight(600.0))
                            .child("Settings"),
                    )
                    .flex_1()
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
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                this.handle_cancel(cx);
                            }))
                            .child(div().text_sm().child("✕")),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(16.0))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_sm()
                            .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                            .font_weight(gpui::FontWeight(600.0))
                            .child("Font")
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                    .child("Family")
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .relative()
                                    .child(
                                        div()
                                            .h(px(28.0))
                                            .px(px(8.0))
                                            .bg(hsla(0.0, 0.0, 0.06, 1.0))
                                            .border(px(1.0))
                                                    .border_color(if self.show_font_picker {
                                                        hsla(0.0, 0.6, 0.5, 1.0)
                                                    } else {
                                                        hsla(0.0, 0.0, 0.1, 1.0)
                                                    })
                                                    .rounded_md()
                                                    .text_sm()
                                                    .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .cursor_pointer()
                                                    .hover(|div| div.border_color(hsla(0.0, 0.0, 0.2, 1.0)))
                                                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Self>| {
                                                        this.toggle_font_picker(window, cx);
                                                    }))
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                                            .child(if self.show_font_picker { "▲" } else { "▼" })
                                                    ),
                                            )
                                            .children(if self.show_font_picker {
                                                Some(
                                                    div()
                                                        .absolute()
                                                        .mt(px(2.0))
                                                        .w_full()
                                                        .bg(hsla(0.0, 0.0, 0.05, 1.0))
                                                        .border(px(1.0))
                                                        .border_color(hsla(0.0, 0.0, 0.15, 1.0))
                                                        .rounded_md()
                                                        .overflow_hidden()
                                                        .flex()
                                                        .flex_col()
                                                        .max_h(px(200.0))
                                                        .child(
                                                            div()
                                                                .p(px(4.0))
                                                                .bg(hsla(0.0, 0.0, 0.08, 1.0))
                                                                .child(
                                                                    div()
                                                                        .flex_1()
                                                                        .h(px(28.0))
                                                                        .px(px(8.0))
                                                                        .bg(hsla(0.0, 0.0, 0.06, 1.0))
                                                                        .border(px(1.0))
                                                                        .border_color(hsla(0.0, 0.0, 0.1, 1.0))
                                                                        .rounded_sm()
                                                                        .text_sm()
                                                                        .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                                                                        .child("Search fonts...")
                                                                )
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .overflow_y_hidden()
                                                                .children(self.filtered_font_list().iter().map(|font_name| {
                                                                    let is_selected = **font_name == self.config.font.family;
                                                                    let font_name_for_click = (*font_name).clone();
                                                                    let font_name_for_display = (*font_name).clone();
                                                                    div()
                                                                        .h(px(28.0))
                                                                        .px(px(8.0))
                                                                        .flex()
                                                                        .items_center()
                                                                        .text_sm()
                                                                        .text_color(if is_selected { hsla(0.0, 0.6, 0.5, 1.0) } else { hsla(0.0, 0.0, 0.9, 1.0) })
                                                                        .bg(if is_selected { hsla(0.0, 0.0, 0.08, 1.0) } else { hsla(0.0, 0.0, 0.0, 0.0) })
                                                                        .cursor_pointer()
                                                                        .hover(|div| div.bg(hsla(0.0, 0.0, 0.08, 1.0)))
                                                                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                                                            this.select_font(font_name_for_click.clone(), cx);
                                                                        }))
                                                                        .child(font_name_for_display)
                                                                }))
                                                        ),
                                                )
                                            } else {
                                                None
                                            }),
                                    )
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                            .child("Size")
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .h(px(28.0))
                                                    .px(px(8.0))
                                                    .w(px(80.0))
                                                    .bg(hsla(0.0, 0.0, 0.06, 1.0))
                                                    .border(px(1.0))
                                                    .border_color(hsla(0.0, 0.0, 0.1, 1.0))
                                                    .rounded_md()
                                                    .text_sm()
                                                    .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                                                    .child(format!("{}px", self.config.font.size))
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .w(px(24.0))
                                                            .h(px(24.0))
                                                            .rounded_md()
                                                            .bg(hsla(0.0, 0.0, 0.08, 1.0))
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .cursor_pointer()
                                                            .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)))
                                                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                                                this.increment_font_size(cx);
                                                            }))
                                                            .child(div().text_xs().text_color(hsla(0.0, 0.0, 0.7, 1.0)).child("+"))
                                                    )
                                                    .child(
                                                        div()
                                                            .w(px(24.0))
                                                            .h(px(24.0))
                                                            .rounded_md()
                                                            .bg(hsla(0.0, 0.0, 0.08, 1.0))
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .cursor_pointer()
                                                            .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)))
                                                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                                                this.decrement_font_size(cx);
                                                            }))
                                                            .child(div().text_xs().text_color(hsla(0.0, 0.0, 0.7, 1.0)).child("-"))
                                                    )
                                            )
                                    )
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                            .child("Weight")
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .h(px(28.0))
                                                    .px(px(8.0))
                                                    .w(px(80.0))
                                                    .bg(hsla(0.0, 0.0, 0.06, 1.0))
                                                    .border(px(1.0))
                                                    .border_color(hsla(0.0, 0.0, 0.1, 1.0))
                                                    .rounded_md()
                                                    .text_sm()
                                                    .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                                                    .child(format!("{}", self.config.font.weight as i32))
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .w(px(24.0))
                                                            .h(px(24.0))
                                                            .rounded_md()
                                                            .bg(hsla(0.0, 0.0, 0.08, 1.0))
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .cursor_pointer()
                                                            .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)))
                                                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                                                this.increment_font_weight(cx);
                                                            }))
                                                            .child(div().text_xs().text_color(hsla(0.0, 0.0, 0.7, 1.0)).child("+"))
                                                    )
                                                    .child(
                                                        div()
                                                            .w(px(24.0))
                                                            .h(px(24.0))
                                                            .rounded_md()
                                                            .bg(hsla(0.0, 0.0, 0.08, 1.0))
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .cursor_pointer()
                                                            .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)))
                                                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                                                this.decrement_font_weight(cx);
                                                            }))
                                                            .child(div().text_xs().text_color(hsla(0.0, 0.0, 0.7, 1.0)).child("-"))
                                                    )
                                            )
                                    )
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                                            .child("Ligatures")
                                    )
                                    .child(
                                        div()
                                            .w(px(40.0))
                                            .h(px(22.0))
                                            .rounded_full()
                                            .bg(if self.config.font.ligatures {
                                                hsla(0.0, 0.6, 0.5, 1.0)
                                            } else {
                                                hsla(0.0, 0.0, 0.2, 1.0)
                                            })
                                            .cursor_pointer()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                                this.toggle_ligatures(cx);
                                            }))
                                            .child(
                                                div()
                                                    .w(px(16.0))
                                                    .h(px(16.0))
                                                    .rounded_full()
                                                    .bg(hsla(0.0, 0.0, 1.0, 1.0))
                                            ),
                                    )
                            )
                    )
                    .child(
                        div()
                            .mt(px(8.0))
                            .text_sm()
                            .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                            .font_weight(gpui::FontWeight(600.0))
                            .child("Terminal")
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                                            .child("Scrollback Lines")
                                    )
                                    .child(
                                        div()
                                            .h(px(28.0))
                                            .px(px(8.0))
                                            .w(px(100.0))
                                            .bg(hsla(0.0, 0.0, 0.06, 1.0))
                                            .border(px(1.0))
                                            .border_color(hsla(0.0, 0.0, 0.1, 1.0))
                                            .rounded_md()
                                            .text_sm()
                                            .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                                            .child(format!("{}", self.config.scrollback))
                                    )
                            )
                    )
                    .child(
                        div()
                            .h(px(48.0))
                            .px(px(16.0))
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .h(px(28.0))
                                    .px(px(16.0))
                                    .rounded_md()
                                    .bg(hsla(0.0, 0.0, 0.08, 1.0))
                                    .text_sm()
                                    .text_color(hsla(0.0, 0.0, 0.7, 1.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|div| div.bg(hsla(0.0, 0.0, 0.12, 1.0)))
                                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                        this.handle_cancel(cx);
                                    }))
                                    .child("Cancel")
                            )
                            .child(
                                div()
                                    .h(px(28.0))
                                    .px(px(16.0))
                                    .rounded_md()
                                    .bg(hsla(0.0, 0.6, 0.5, 1.0))
                                    .text_sm()
                                    .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|div| div.bg(hsla(0.0, 0.6, 0.45, 1.0)))
                                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this: &mut Self, _: &gpui::MouseDownEvent, _: &mut Window, cx: &mut Context<Self>| {
                                        this.handle_save(cx);
                                    }))
                                    .child("Save")
                            )
                    )
            )
    }
}

impl SettingsView {
    fn render_font_picker(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        let current_family = self.config.font.family.clone();
        let font_list = self.font_list.clone();

        div()
            .mt(px(4.0))
            .h(px(200.0))
            .bg(hsla(0.0, 0.0, 0.05, 1.0))
            .border(px(1.0))
            .border_color(hsla(0.0, 0.0, 0.15, 1.0))
            .rounded_md()
            .flex()
            .flex_col()
            .children(font_list.iter().take(200).map(|font_name| {
                let is_selected = font_name.as_str() == current_family.as_str();
                div()
                    .h(px(28.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .text_sm()
                    .text_color(if is_selected {
                        hsla(0.0, 0.6, 0.5, 1.0)
                    } else {
                        hsla(0.0, 0.0, 0.9, 1.0)
                    })
                    .child(font_name.clone())
            }))
    }
}
