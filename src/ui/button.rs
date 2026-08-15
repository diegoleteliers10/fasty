use gpui::{
    App, ClickEvent, FontWeight, IntoElement, RenderOnce, SharedString, Window, div, prelude::*,
    px, rgba,
};

use super::theme::Theme;

type ButtonClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    Blurry,
    #[default]
    Default,
    Dark,
    Light,
    Ghost,
    Danger,
    Warning,
    Info,
}

struct ButtonColors {
    bg: gpui::Hsla,
    hover_bg: gpui::Hsla,
    active_bg: gpui::Hsla,
    border: gpui::Hsla,
    hover_border: gpui::Hsla,
    fg: gpui::Hsla,
}

impl ButtonVariant {
    fn colors(self, theme: Theme) -> ButtonColors {
        match self {
            Self::Blurry => ButtonColors {
                bg: rgba(0xffffff18).into(),
                hover_bg: rgba(0xffffff28).into(),
                active_bg: rgba(0xffffff33).into(),
                border: rgba(0xffffff2e).into(),
                hover_border: rgba(0xffffff55).into(),
                fg: theme.foreground,
            },
            Self::Default => ButtonColors {
                bg: theme.surface,
                hover_bg: theme.hover,
                active_bg: theme.selected,
                border: theme.border,
                hover_border: theme.accent,
                fg: theme.foreground,
            },
            Self::Dark => ButtonColors {
                bg: rgba(0x0a0a0acc).into(),
                hover_bg: rgba(0x141414e6).into(),
                active_bg: rgba(0x000000f2).into(),
                border: rgba(0x2a2a2acc).into(),
                hover_border: theme.muted_strong,
                fg: theme.bright_white,
            },
            Self::Light => ButtonColors {
                bg: rgba(0xf3f3f3e6).into(),
                hover_bg: rgba(0xffffff).into(),
                active_bg: rgba(0xe8e8e8f2).into(),
                border: rgba(0xffffffaa).into(),
                hover_border: rgba(0xffffff).into(),
                fg: theme.black,
            },
            Self::Ghost => ButtonColors {
                bg: gpui::transparent_black(),
                hover_bg: rgba(0xffffff14).into(),
                active_bg: rgba(0xffffff22).into(),
                border: gpui::transparent_black(),
                hover_border: rgba(0xffffff33).into(),
                fg: theme.foreground,
            },
            Self::Danger => ButtonColors {
                bg: rgba(0xf7768e33).into(),
                hover_bg: rgba(0xf7768e55).into(),
                active_bg: rgba(0xf7768e77).into(),
                border: rgba(0xf7768e88).into(),
                hover_border: theme.bright_red,
                fg: theme.bright_red,
            },
            Self::Warning => ButtonColors {
                bg: rgba(0xe0af6833).into(),
                hover_bg: rgba(0xe0af6855).into(),
                active_bg: rgba(0xe0af6877).into(),
                border: rgba(0xe0af6888).into(),
                hover_border: theme.bright_yellow,
                fg: theme.bright_yellow,
            },
            Self::Info => ButtonColors {
                bg: rgba(0x7aa2f733).into(),
                hover_bg: rgba(0x7aa2f755).into(),
                active_bg: rgba(0x7aa2f777).into(),
                border: rgba(0x7aa2f788).into(),
                hover_border: theme.bright_blue,
                fg: theme.bright_blue,
            },
        }
    }
}

#[derive(IntoElement)]
pub struct Button {
    id: SharedString,
    label: SharedString,
    variant: ButtonVariant,
    theme: Theme,
    on_click: Option<ButtonClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant: ButtonVariant::Default,
            theme: Theme::default(),
            on_click: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn on_click(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(listener));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let colors = self.variant.colors(self.theme);
        let hover_bg = colors.hover_bg;
        let hover_border = colors.hover_border;
        let active_bg = colors.active_bg;

        div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .px(px(12.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(colors.border)
            .bg(colors.bg)
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(colors.fg)
            .hover(move |s| s.bg(hover_bg).border_color(hover_border))
            .active(move |s| s.bg(active_bg))
            .cursor_pointer()
            .when_some(self.on_click, |this, on_click| this.on_click(on_click))
            .child(self.label)
    }
}
