use gpui::{
    App, FontWeight, IntoElement, MouseDownEvent, RenderOnce, SharedString, Window,
    div, prelude::*, px,
};
use parking_lot::Mutex;
use std::sync::Arc;
use crate::config::Config;
use crate::git::GitStatus;
use crate::widgets::{Align, Segment, Widget, WidgetContext};
use super::theme::{Theme, rgb_to_hsla};

pub struct StatusBarModel {
    pub widgets: Arc<Mutex<Vec<Box<dyn Widget>>>>,
    pub theme: Theme,
}

impl StatusBarModel {
    pub fn new(config: &Config, theme: Theme) -> Self {
        let mut widgets = Vec::new();
        for spec in &config.bottombar.widgets {
            if let Some(w) = crate::widgets::builtin::build(spec) {
                widgets.push(w);
            }
        }
        Self {
            widgets: Arc::new(Mutex::new(widgets)),
            theme,
        }
    }

    pub fn poll_in_background(
        widgets_arc: Arc<Mutex<Vec<Box<dyn Widget>>>>,
        cwd: Option<std::path::PathBuf>,
        git: Option<GitStatus>,
    ) {
        std::thread::spawn(move || {
            let now = std::time::Instant::now();
            let ctx = WidgetContext {
                active_tab_cwd: cwd.as_deref(),
                active_tab_git: git.as_ref(),
                opacity: 1.0,
            };
            let mut widgets = widgets_arc.lock();
            for w in widgets.iter_mut() {
                if now.duration_since(w.last_poll()) >= w.poll_interval() {
                    w.poll(&ctx);
                    w.set_last_poll(now);
                }
            }
        });
    }

    pub fn render_segments(
        &self,
        cwd: Option<&std::path::Path>,
        git: Option<&GitStatus>,
    ) -> (Vec<Segment>, Vec<Segment>) {
        let ctx = WidgetContext {
            active_tab_cwd: cwd,
            active_tab_git: git,
            opacity: 1.0,
        };
        let mut left = Vec::new();
        let mut right = Vec::new();

        let mut widgets = self.widgets.lock();
        let now = std::time::Instant::now();
        for w in widgets.iter_mut() {
            if now.duration_since(w.last_poll()) >= w.poll_interval() {
                w.poll(&ctx);
                w.set_last_poll(now);
            }
            let segs = w.render(&ctx);
            match w.align() {
                Align::Left => left.extend(segs),
                Align::Right => right.extend(segs),
            }
        }
        (left, right)
    }
}

#[derive(Clone, Debug, Default)]
pub struct StatusInfo {
    pub git_branch: Option<String>,
    pub git_dirty: bool,
    pub cwd: Option<String>,
    pub last_duration_ms: Option<u128>,
    pub last_exit_code: Option<i32>,
}

pub type GitContextCallback = Arc<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct StatusBar {
    left_segments: Vec<Segment>,
    right_segments: Vec<Segment>,
    fallback_info: StatusInfo,
    theme: Theme,
    on_git_context_menu: Option<GitContextCallback>,
}

impl StatusBar {
    pub fn new(
        left_segments: Vec<Segment>,
        right_segments: Vec<Segment>,
        fallback_info: StatusInfo,
        theme: Theme,
    ) -> Self {
        Self {
            left_segments,
            right_segments,
            fallback_info,
            theme,
            on_git_context_menu: None,
        }
    }

    pub fn on_git_context_menu(mut self, handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_git_context_menu = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for StatusBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let git_menu_cb = self.on_git_context_menu.clone();

        let has_widgets = !self.left_segments.is_empty() || !self.right_segments.is_empty();

        let left_children: Vec<_> = if has_widgets {
            self.left_segments
                .into_iter()
                .enumerate()
                .map(|(idx, seg)| {
                    let c = seg.color;
                    let u_r = (c[0] * 255.0).clamp(0.0, 255.0) as u8;
                    let u_g = (c[1] * 255.0).clamp(0.0, 255.0) as u8;
                    let u_b = (c[2] * 255.0).clamp(0.0, 255.0) as u8;
                    let fg = rgb_to_hsla(u_r, u_g, u_b);
                    let is_git_seg = seg.text.contains('⎇') || seg.text.contains("git");
                    let git_cb = git_menu_cb.clone();

                    let mut el = div()
                        .id(SharedString::from(format!("w-left-{}", idx)))
                        .text_size(px(11.))
                        .text_color(fg);

                    if is_git_seg {
                        if let Some(cb) = git_cb {
                            let cb_left = cb.clone();
                            el = el
                                .cursor(gpui::CursorStyle::PointingHand)
                                .on_mouse_down(gpui::MouseButton::Left, move |ev, win, app| {
                                    cb_left(ev, win, app);
                                })
                                .on_mouse_down(gpui::MouseButton::Right, move |ev, win, app| {
                                    cb(ev, win, app);
                                });
                        }
                    }

                    el.child(SharedString::from(seg.text))
                })
                .collect()
        } else {
            let mut items = Vec::new();
            if let Some(branch) = self.fallback_info.git_branch {
                let color = if self.fallback_info.git_dirty {
                    theme.yellow
                } else {
                    theme.green
                };
                let git_cb = git_menu_cb.clone();
                let mut el = div()
                    .id("fallback-git")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .cursor(gpui::CursorStyle::PointingHand)
                    .child(super::icons::render_icon(icons::common::IconType::GitBranch, color, 12.0))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.foreground)
                            .child(SharedString::from(branch)),
                    );

                if let Some(cb) = git_cb {
                    let cb_left = cb.clone();
                    el = el
                        .on_mouse_down(gpui::MouseButton::Left, move |ev, win, app| {
                            cb_left(ev, win, app);
                        })
                        .on_mouse_down(gpui::MouseButton::Right, move |ev, win, app| {
                            cb(ev, win, app);
                        });
                }
                items.push(el);
            }
            let cwd_str = self.fallback_info.cwd.unwrap_or_else(|| "~".to_string());
            items.push(
                div()
                    .id("fallback-cwd")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(super::icons::render_icon(icons::common::IconType::Folder, theme.accent, 11.0))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted_strong)
                            .child(SharedString::from(cwd_str)),
                    ),
            );
            items
        };

        let right_children: Vec<_> = if has_widgets {
            self.right_segments
                .into_iter()
                .enumerate()
                .map(|(idx, seg)| {
                    let c = seg.color;
                    let u_r = (c[0] * 255.0).clamp(0.0, 255.0) as u8;
                    let u_g = (c[1] * 255.0).clamp(0.0, 255.0) as u8;
                    let u_b = (c[2] * 255.0).clamp(0.0, 255.0) as u8;
                    let fg = rgb_to_hsla(u_r, u_g, u_b);

                    div()
                        .id(SharedString::from(format!("w-right-{}", idx)))
                        .text_size(px(11.))
                        .text_color(fg)
                        .child(SharedString::from(seg.text))
                })
                .collect()
        } else {
            let mut items = Vec::new();
            if let Some(ms) = self.fallback_info.last_duration_ms {
                let text = if ms < 1000 {
                    format!("{}ms", ms)
                } else {
                    format!("{:.2}s", ms as f64 / 1000.0)
                };
                items.push(
                    div()
                        .id("fallback-dur")
                        .text_size(px(11.))
                        .text_color(theme.muted)
                        .child(SharedString::from(text)),
                );
            }
            if let Some(code) = self.fallback_info.last_exit_code {
                let color = if code == 0 {
                    theme.green
                } else {
                    theme.red
                };
                items.push(
                    div()
                        .id("fallback-exit")
                        .text_size(px(11.))
                        .text_color(color)
                        .child(SharedString::from(format!("↵ {}", code))),
                );
            }
            items
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(20.))
            .w_full()
            .bg(theme.status_bar_bg)
            .px(px(12.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .children(left_children),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .children(right_children),
            )
    }
}
