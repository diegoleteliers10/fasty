//! Tab system for fasty terminal.
//!
//! Provides TabBar, Tab, and tab management functionality.

use gpui::{
    div, hsla, px, AppContext, InteractiveElement,
    IntoElement, ParentElement, Render, Styled, Window,
};

pub struct TabBar {
    tabs: Vec<Tab>,
    active_index: usize,
    on_new_tab: Option<Box<dyn Fn(&mut Window, &mut AppContext) + 'static>>,
}

pub struct Tab {
    pub title: String,
    pub id: usize,
}

impl TabBar {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_index: 0,
            on_new_tab: None,
        }
    }

    pub fn with_tabs(tabs: Vec<Tab>, active_index: usize) -> Self {
        Self {
            tabs,
            active_index,
            on_new_tab: None,
        }
    }

    pub fn on_new_tab(mut self, cb: impl Fn(&mut Window, &mut AppContext) + 'static) -> Self {
        self.on_new_tab = Some(Box::new(cb));
        self
    }
}

impl Default for TabBar {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for TabBar {
    fn render(&mut self, _window: &mut Window, _cx: &mut AppContext) -> impl IntoElement {
        let new_tab_cb = self.on_new_tab.take();
        let tabs = &self.tabs;
        let active_index = self.active_index;

        div()
            .h(px(36.0))
            .bg(hsla(0.0, 0.0, 0.0, 0.2))
            .flex()
            .items_center()
            .children(tabs.iter().enumerate().map(|(i, tab)| {
                let is_active = i == active_index;
                div()
                    .id(format!("tab-{}", tab.id))
                    .px(px(12.0))
                    .py(px(6.0))
                    .bg(if is_active {
                        hsla(0.0, 0.0, 0.15, 1.0)
                    } else {
                        hsla(0.0, 0.0, 0.0, 0.0)
                    })
                    .rounded_top(px(4.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(if is_active {
                                hsla(0.0, 0.0, 0.95, 1.0)
                            } else {
                                hsla(0.0, 0.0, 0.6, 1.0)
                            })
                            .child(tab.title.clone()),
                    )
                    .child(
                        div()
                            .w(px(20.0))
                            .h(px(20.0))
                            .rounded_full()
                            .text_xs()
                            .text_color(hsla(0.0, 0.0, 0.5, 1.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("x")
                            .hoverable()
                            .on_click(move |_event, _window, _cx| {
                                // Close tab handled via parent
                            }),
                    )
            }))
            .child(
                div()
                    .w(px(32.0))
                    .h(px(24.0))
                    .rounded(px(4.0))
                    .bg(hsla(0.0, 0.0, 0.15, 1.0))
                    .text_md()
                    .text_color(hsla(0.0, 0.0, 0.7, 1.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("+")
                    .hoverable()
                    .on_click(move |_event, window, cx| {
                        if let Some(cb) = new_tab_cb.as_ref() {
                            cb(window, cx);
                        }
                    }),
            )
    }
}