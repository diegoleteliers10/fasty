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
    on_logo_context_menu: Option<Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>>,
    update_available: Option<String>,
    is_updating: bool,
    on_update: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
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
            on_logo_context_menu: None,
            update_available: None,
            is_updating: false,
            on_update: None,
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

    pub fn on_logo_context_menu(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_logo_context_menu = Some(Box::new(handler));
        self
    }

    pub fn update_available(mut self, version: Option<String>, is_updating: bool) -> Self {
        self.update_available = version;
        self.is_updating = is_updating;
        self
    }

    pub fn on_update(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_update = Some(Box::new(handler));
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

        let show_tabs = self.tabs.len() > 1;

        // macOS: traffic lights are OS-owned on the left.
        // Logo + gear live on the right. Tabs start at ~76 px.
        #[cfg(target_os = "macos")]
        {
            let left_padding = px(76.);

            return div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(32.))
                .w_full()
                .bg(theme.tab_bar_bg)
                .px(px(8.))
                .pl(left_padding)
                .when(show_tabs, |this| {
                    this.child(render_tab_strip(
                        self.tabs,
                        theme,
                        on_select,
                        on_close,
                        on_rename,
                        on_tab_context,
                        self.on_new_tab,
                        btn_hover_bg,
                        btn_default_bg,
                    ))
                })
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .pr(px(6.))
                        .when_some(self.update_available, |this, version| {
                            this.child(render_update_btn(
                                version,
                                self.is_updating,
                                theme,
                                self.on_update,
                            ))
                        })
                        .child(
                            div()
                                .id("fastty-logo")
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(22.))
                                .h(px(22.))
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                                    if let Some(ref cb) = on_logo_context {
                                        cb(ev, window, cx);
                                    }
                                })
                                .child(super::icons::render_app_logo(15.0)),
                        ),
                );
        }

        // Windows / Linux: [logo][tabs...+][drag spacer][update?][min][max][X]
        #[cfg(not(target_os = "macos"))]
        {
            div()
                .id("tab-bar-top-container")
                .flex()
                .flex_row()
                .items_center()
                .h(px(32.))
                .w_full()
                .bg(theme.tab_bar_bg)
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
                        .pl(px(4.))
                        .gap_1()
                        .child(
                            div()
                                .id("fastty-logo")
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(22.))
                                .h(px(22.))
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                                    if let Some(ref cb) = on_logo_context {
                                        cb(ev, window, cx);
                                    }
                                })
                                .child(super::icons::render_app_logo(15.0)),
                        )
                        .when(show_tabs, |this| {
                            this.child(render_tab_strip(
                                self.tabs,
                                theme,
                                on_select,
                                on_close,
                                on_rename,
                                on_tab_context,
                                self.on_new_tab,
                                btn_hover_bg,
                                btn_default_bg,
                            ))
                        }),
                )
                .child(
                    div()
                        .id("tab-bar-drag-spacer")
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
                        .when_some(self.update_available, |this, version| {
                            this.child(render_update_btn(
                                version,
                                self.is_updating,
                                theme,
                                self.on_update,
                            ))
                        })
                        .child(render_win_btn(
                            "win-minimize",
                            btn_hover_bg,
                            |window| window.minimize_window(),
                            render_minimize_icon(theme.foreground),
                        ))
                        .child(render_win_btn(
                            "win-maximize",
                            btn_hover_bg,
                            |window| window.zoom_window(),
                            render_maximize_icon(theme.foreground),
                        ))
                        .child(render_close_btn(theme)),
                )
        }
    }
}

// ── Shared sub-components ─────────────────────────────────────────────────────

fn render_tab_strip(
    tabs: Vec<TabItem>,
    theme: Theme,
    on_select: Option<std::rc::Rc<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>>,
    on_close: Option<std::rc::Rc<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>>,
    on_rename: Option<std::rc::Rc<Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>>>,
    on_tab_context: Option<std::rc::Rc<Box<dyn Fn(&(usize, f32, f32), &mut Window, &mut App) + 'static>>>,
    on_new_tab: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    btn_hover_bg: gpui::Hsla,
    btn_default_bg: gpui::Hsla,
) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .children(tabs.into_iter().map(|tab| {
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
                .font_weight(if is_active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                .text_color(fg)
                .cursor(CursorStyle::PointingHand)
                .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                    if ev.click_count >= 2 {
                        if let Some(ref rcb) = on_rename_clone {
                            rcb(&tab_id, window, cx);
                        }
                    } else if let Some(ref cb) = on_select_clone {
                        cb(&tab_id, window, cx);
                    }
                })
                .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
                    if let Some(ref cm_cb) = on_tab_context_clone {
                        let x = ev.position.x.to_f64() as f32;
                        let y = ev.position.y.to_f64() as f32;
                        cm_cb(&(tab_id, x, y), window, cx);
                    }
                })
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
                        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                            if let Some(ref cb) = on_close_clone {
                                cb(&tab_id, window, cx);
                            }
                        })
                        .child(super::icons::render_icon(
                            icons::common::IconType::X,
                            theme.accent,
                            10.0,
                        )),
                )
        }))
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
                .when_some(on_new_tab, |this, on_click| this.on_click(on_click))
                .child(super::icons::render_icon(
                    icons::common::IconType::Plus,
                    theme.accent,
                    12.0,
                )),
        )
}


fn render_update_btn(
    version: String,
    is_updating: bool,
    theme: Theme,
    on_update: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id("update-btn")
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1()
        .px(px(6.))
        .h(px(22.))
        .rounded(px(5.))
        .bg(theme.green.opacity(0.15))
        .border_1()
        .border_color(theme.green.opacity(0.4))
        .hover(move |s| s.bg(theme.green.opacity(0.25)).border_color(theme.green))
        .cursor(CursorStyle::PointingHand)
        .when_some(on_update, |el, on_click| el.on_click(on_click))
        .child(super::icons::render_icon(
            icons::common::IconType::Download,
            theme.green,
            12.0,
        ))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.green)
                .child(if is_updating {
                    "Updating...".to_string()
                } else {
                    format!("v{}", version)
                }),
        )
}

// ── Windows / Linux window-control buttons ────────────────────────────────────

/// Generic minimize or maximize button.
#[cfg(not(target_os = "macos"))]
fn render_win_btn(
    id: &'static str,
    btn_hover_bg: gpui::Hsla,
    action: impl Fn(&mut Window) + 'static,
    icon: impl gpui::IntoElement,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(46.))
        .h(px(32.))
        .hover(move |s| s.bg(btn_hover_bg))
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, move |_ev, window, _cx| {
            action(window);
        })
        .child(icon)
}

/// Close button with a red hover background (Windows convention).
#[cfg(not(target_os = "macos"))]
fn render_close_btn(theme: Theme) -> gpui::Stateful<gpui::Div> {
    let red_hover = gpui::Hsla { h: 0.0, s: 0.7, l: 0.45, a: 1.0 };
    div()
        .id("win-close")
        .flex()
        .items_center()
        .justify_center()
        .w(px(46.))
        .h(px(32.))
        .hover(move |s| s.bg(red_hover))
        .cursor(CursorStyle::PointingHand)
        .on_mouse_down(MouseButton::Left, |_ev, window, _cx| {
            window.remove_window();
        })
        .child(render_close_icon(theme.foreground))
}

/// A horizontal bar — the minimize symbol.
#[cfg(not(target_os = "macos"))]
fn render_minimize_icon(color: gpui::Hsla) -> gpui::Div {
    div().w(px(10.)).h(px(1.)).rounded(px(1.)).bg(color)
}

/// A hollow square — the maximize symbol.
#[cfg(not(target_os = "macos"))]
fn render_maximize_icon(color: gpui::Hsla) -> gpui::Div {
    div()
        .w(px(10.))
        .h(px(10.))
        .border_1()
        .border_color(color)
        .rounded(px(1.))
}

/// The X icon — the close symbol.
#[cfg(not(target_os = "macos"))]
fn render_close_icon(color: gpui::Hsla) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .child(super::icons::render_icon(
            icons::common::IconType::X,
            color,
            10.0,
        ))
}
