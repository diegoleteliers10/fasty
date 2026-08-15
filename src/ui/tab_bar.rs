use gpui::{
    App, ClickEvent, CursorStyle, FontWeight, IntoElement, MouseButton, MouseDownEvent, RenderOnce,
    SharedString, Window, div, prelude::*, px,
};
use super::theme::Theme;

#[derive(Clone, Debug)]
pub struct TabItem {
    pub id: usize,
    pub title: String,
    pub active: bool,
    pub is_dirty: bool,
}

#[derive(IntoElement)]
pub struct TabBar {
    tabs: Vec<TabItem>,
    theme: Theme,
    on_select_tab: Option<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>,
    on_close_tab: Option<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>,
    on_rename_tab: Option<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>,
    on_tab_context_menu: Option<Box<dyn Fn(&(usize, f32, f32), &mut Window, &mut App) + 'static>>,
    on_new_tab: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_open_settings: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_logo_context_menu: Option<Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>>,
}

impl TabBar {
    pub fn new(tabs: Vec<TabItem>, theme: Theme) -> Self {
        Self {
            tabs,
            theme,
            on_select_tab: None,
            on_close_tab: None,
            on_rename_tab: None,
            on_tab_context_menu: None,
            on_new_tab: None,
            on_open_settings: None,
            on_logo_context_menu: None,
        }
    }

    pub fn on_select_tab(
        mut self,
        handler: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select_tab = Some(Box::new(handler));
        self
    }

    pub fn on_close_tab(
        mut self,
        handler: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close_tab = Some(Box::new(handler));
        self
    }

    pub fn on_rename_tab(
        mut self,
        handler: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_rename_tab = Some(Box::new(handler));
        self
    }

    pub fn on_tab_context_menu(
        mut self,
        handler: impl Fn(&(usize, f32, f32), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_tab_context_menu = Some(Box::new(handler));
        self
    }

    pub fn on_new_tab(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_new_tab = Some(Box::new(handler));
        self
    }

    pub fn on_open_settings(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_settings = Some(Box::new(handler));
        self
    }

    pub fn on_logo_context_menu(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_logo_context_menu = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for TabBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let on_select = self.on_select_tab.map(std::rc::Rc::new);
        let on_close = self.on_close_tab.map(std::rc::Rc::new);
        let on_rename = self.on_rename_tab.map(std::rc::Rc::new);
        let on_tab_context = self.on_tab_context_menu.map(std::rc::Rc::new);
        let on_logo_context = self.on_logo_context_menu.map(std::rc::Rc::new);
        let btn_hover_bg = theme.hover;
        let btn_default_bg = theme.surface;

        let left_padding = if cfg!(target_os = "macos") {
            px(76.)
        } else {
            px(8.)
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(32.))
            .w_full()
            .bg(theme.tab_bar_bg)
            .px(px(8.))
            .pl(left_padding)
            .child(
                // Left group: Tabs + New Tab (+) button
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .children(
                        self.tabs.into_iter().map(|tab| {
                            let tab_id = tab.id;
                            let is_active = tab.active;
                            let on_select_clone = on_select.clone();
                            let on_close_clone = on_close.clone();
                            let on_rename_clone = on_rename.clone();
                            let on_tab_context_clone = on_tab_context.clone();

                            let (bg, fg, border_b_color) = if is_active {
                                (theme.tab_active_bg, theme.foreground, theme.accent)
                            } else {
                                (theme.tab_inactive_bg, theme.muted_strong, gpui::transparent_black())
                            };

                            div()
                                .id(SharedString::from(format!("tab-{}", tab_id)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .h(px(26.))
                                .min_w(px(120.))
                                .max_w(px(220.))
                                .px(px(8.))
                                .rounded_t(px(4.))
                                .rounded_b(px(0.))
                                .bg(bg)
                                .border_b_2()
                                .border_color(border_b_color)
                                .text_size(px(11.5))
                                .font_weight(if is_active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(fg)
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    move |ev, window, cx| {
                                        if ev.click_count >= 2 {
                                            if let Some(ref rcb) = on_rename_clone {
                                                rcb(&tab_id, window, cx);
                                            }
                                        } else if let Some(ref cb) = on_select_clone {
                                            cb(&tab_id, window, cx);
                                        }
                                    },
                                )
                                .on_mouse_down(
                                    MouseButton::Right,
                                    move |ev, window, cx| {
                                        if let Some(ref cm_cb) = on_tab_context_clone {
                                            let x = ev.position.x.to_f64() as f32;
                                            let y = ev.position.y.to_f64() as f32;
                                            cm_cb(&(tab_id, x, y), window, cx);
                                        }
                                    },
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .child(SharedString::from(tab.title)),
                                        ),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!("tab-close-{}", tab_id)))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(14.))
                                        .h(px(14.))
                                        .rounded(px(3.))
                                        .hover(move |s| s.bg(btn_hover_bg))
                                        .text_size(px(11.))
                                        .text_color(theme.muted)
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            move |_ev, window, cx| {
                                                if let Some(ref cb) = on_close_clone {
                                                    cb(&tab_id, window, cx);
                                                }
                                            },
                                        )
                                        .child(super::icons::render_icon(icons::common::IconType::X, theme.accent, 10.0)),
                                )
                        }),
                    )
                    .child(
                        div()
                            .id("new-tab-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(22.))
                            .h(px(22.))
                            .rounded(px(4.))
                            .bg(btn_default_bg)
                            .hover(move |s| s.bg(btn_hover_bg))
                            .cursor(CursorStyle::PointingHand)
                            .when_some(self.on_new_tab, |this, on_click| this.on_click(on_click))
                            .child(super::icons::render_icon(icons::common::IconType::Plus, theme.accent, 12.0)),
                    ),
            )
            // Flexible spacer pushing right controls to the far right
            .child(div().flex_1())
            // Right group: Settings button + Logo icon (no background)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .pr(px(6.))
                    .child(
                        div()
                            .id("settings-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(22.))
                            .h(px(22.))
                            .rounded(px(5.))
                            .bg(btn_default_bg)
                            .hover(move |s| s.bg(btn_hover_bg))
                            .cursor(CursorStyle::PointingHand)
                            .when_some(self.on_open_settings, |this, on_click| this.on_click(on_click))
                            .child(super::icons::render_icon(icons::common::IconType::Settings, theme.accent, 13.0)),
                    )
                    .child(
                        // Fastty branding icon with context menu on right-click / left-click
                        div()
                            .id("fastty-logo")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(22.))
                            .h(px(22.))
                            .on_mouse_down(
                                MouseButton::Right,
                                move |ev, window, cx| {
                                    if let Some(ref cb) = on_logo_context {
                                        cb(ev, window, cx);
                                    }
                                },
                            )
                            .child(super::icons::render_app_logo(15.0)),
                    ),
            )
    }
}
