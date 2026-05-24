//! Custom titlebar component for fasty.
//!
//! Renders traffic lights, title, and settings button in a custom titlebar.

use gpui::{
    div, hsla, px, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window,
};
use crate::app::AppView;

pub struct Titlebar {
    on_settings_click: Option<Box<dyn Fn(&mut Window, &mut gpui::Context<AppView>) + 'static>>,
}

impl Titlebar {
    pub fn new(on_settings_click: impl Fn(&mut Window, &mut gpui::Context<AppView>) + 'static) -> Self {
        Self {
            on_settings_click: Some(Box::new(on_settings_click)),
        }
    }
}

impl Render for Titlebar {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<AppView>) -> impl IntoElement {
        let settings_click = self.on_settings_click.take();
        div()
            .h(px(36.0))
            .bg(hsla(0.0, 0.0, 0.0, 0.3))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(12.0))
                            .rounded_full()
                            .bg(hsla(0.0, 0.7, 0.5, 0.85)),
                    )
                    .child(
                        div()
                            .size(px(12.0))
                            .rounded_full()
                            .bg(hsla(0.33, 0.7, 0.5, 0.85)),
                    )
                    .child(
                        div()
                            .size(px(12.0))
                            .rounded_full()
                            .bg(hsla(0.55, 0.7, 0.5, 0.85)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .text_center()
                    .child("fasty"),
            )
            .child(
                div()
                    .w(px(60.0))
                    .h(px(24.0))
                    .rounded(px(4.0))
                    .bg(hsla(0.0, 0.0, 0.2, 1.0))
                    .text_sm()
                    .text_color(hsla(0.0, 0.0, 0.8, 1.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("Settings")
                    .hoverable()
                    .on_click(move |_event, window, cx| {
                        if let Some(cb) = settings_click.as_ref() {
                            cb(window, cx);
                        }
                    }),
            )
    }
}