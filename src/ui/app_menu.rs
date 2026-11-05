//! The menu that opens from the Fastty logo in the top bar.

use gpui::prelude::*;
use gpui::{
    div, px, App, CursorStyle, FontWeight, IntoElement, MouseButton, MouseDownEvent, RenderOnce,
    SharedString, Window,
};
use icons::common::IconType;

use super::icons::{render_app_logo, render_icon};
use super::theme::Theme;

const MENU_WIDTH: f32 = 268.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMenuAction {
    About,
    CheckForUpdates,
    Settings,
    CommandPalette,
    FasttyAi,
    SshManager,
    FindInBuffer,
    NewTab,
    ClearScrollback,
    OpenConfigFolder,
    ToggleFullscreen,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppMenuStatus {
    Unknown,
    UpToDate,
    CheckFailed,
    Updating,
    Ready,
    Available(String),
}

impl AppMenuAction {
    pub const ALL: &'static [AppMenuAction] = &[
        AppMenuAction::About,
        AppMenuAction::CheckForUpdates,
        AppMenuAction::Settings,
        AppMenuAction::CommandPalette,
        AppMenuAction::FasttyAi,
        AppMenuAction::SshManager,
        AppMenuAction::FindInBuffer,
        AppMenuAction::NewTab,
        AppMenuAction::ClearScrollback,
        AppMenuAction::OpenConfigFolder,
        AppMenuAction::ToggleFullscreen,
        AppMenuAction::Quit,
    ];
}

impl AppMenuStatus {
    fn label(&self) -> String {
        match self {
            Self::Unknown => "checking...".to_string(),
            Self::UpToDate => "up to date".to_string(),
            Self::CheckFailed => "check failed".to_string(),
            Self::Updating => "updating...".to_string(),
            Self::Ready => "restart to update".to_string(),
            Self::Available(version) => format!("v{version} available"),
        }
    }

    fn color(&self, theme: Theme) -> gpui::Hsla {
        match self {
            Self::Unknown | Self::CheckFailed => theme.muted,
            Self::UpToDate => theme.green,
            Self::Updating => theme.yellow,
            Self::Ready | Self::Available(_) => theme.accent,
        }
    }
}

/// Split a shortcut into one chip per modifier, with the remaining keys in a
/// single chip.
pub fn shortcut_keys(shortcut: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = shortcut;
    while let Some(first) = rest.chars().next() {
        if "⌘⌃⌥⇧".contains(first) {
            keys.push(first.to_string());
            rest = &rest[first.len_utf8()..];
        } else {
            break;
        }
    }
    if !rest.is_empty() {
        keys.push(rest.to_string());
    }
    keys
}

pub type AppMenuCallback = Box<dyn Fn(&AppMenuAction, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct AppMenu {
    theme: Theme,
    version: String,
    status: AppMenuStatus,
    max_height: f32,
    on_action: Option<AppMenuCallback>,
}

impl AppMenu {
    pub fn new(
        theme: Theme,
        version: impl Into<String>,
        status: AppMenuStatus,
        max_height: f32,
    ) -> Self {
        Self {
            theme,
            version: version.into(),
            status,
            max_height,
            on_action: None,
        }
    }

    pub fn on_action(
        mut self,
        handler: impl Fn(&AppMenuAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for AppMenu {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let callback = self.on_action.map(std::rc::Rc::new);
        let mut menu = div()
            .id("fastty-app-menu")
            .w(px(MENU_WIDTH))
            .p(px(5.))
            .rounded(px(11.))
            .bg(theme.surface_raised)
            .border_1()
            .border_color(theme.border)
            .shadow_xl()
            .occlude()
            .max_h(px(self.max_height))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .on_mouse_down(MouseButton::Right, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .on_mouse_move(|_event, _window, cx| cx.stop_propagation())
            .on_scroll_wheel(|_event, _window, cx| cx.stop_propagation());

        // Header: logo, name, version, update state, and the About link.
        let version = self.version.clone();
        let status = self.status.clone();
        menu = menu.child(render_header(&version, &status, theme, callback.clone()));
        menu = menu.child(div().h(px(1.)).mx(px(5.)).my(px(4.)).bg(theme.border));

        for row in rows() {
            match row {
                MenuRow::Gap => {
                    menu = menu.child(div().h(px(6.)).flex_shrink_0());
                }
                MenuRow::Item(entry) => {
                    menu = menu.child(render_row(
                        entry.icon,
                        entry.label,
                        entry.shortcut,
                        entry.action,
                        entry.accent,
                        theme,
                        callback.clone(),
                    ));
                }
            }
        }
        menu
    }
}

struct MenuEntry {
    icon: IconType,
    label: &'static str,
    shortcut: &'static str,
    action: AppMenuAction,
    accent: bool,
}

enum MenuRow {
    Item(MenuEntry),
    Gap,
}

fn item(
    icon: IconType,
    label: &'static str,
    shortcut: &'static str,
    action: AppMenuAction,
) -> MenuRow {
    MenuRow::Item(MenuEntry {
        icon,
        label,
        shortcut,
        action,
        accent: false,
    })
}

fn rows() -> Vec<MenuRow> {
    let mac = cfg!(target_os = "macos");
    let palette = if mac { "⌘P" } else { "Ctrl+Shift+P" };
    let ai = if mac { "⌘L" } else { "Ctrl+Shift+L" };
    let ssh = if mac { "⌘O" } else { "Ctrl+Shift+O" };
    let find = if mac { "⌘F" } else { "Ctrl+Shift+F" };
    let settings = if mac { "⌘," } else { "Ctrl+," };
    let new_tab = if mac { "⌘T" } else { "Ctrl+Shift+T" };
    let clear = if mac { "⌘K" } else { "Ctrl+Shift+K" };
    let fullscreen = if mac { "⌃⌘F" } else { "F11" };
    let quit = if mac { "⌘Q" } else { "Alt+F4" };

    vec![
        item(
            IconType::RotateCcw,
            "Check for Updates",
            "",
            AppMenuAction::CheckForUpdates,
        ),
        item(
            IconType::Settings,
            "Settings...",
            settings,
            AppMenuAction::Settings,
        ),
        MenuRow::Gap,
        item(
            IconType::Command,
            "Command Palette",
            palette,
            AppMenuAction::CommandPalette,
        ),
        MenuRow::Item(MenuEntry {
            icon: IconType::Sparkles,
            label: "Fastty AI",
            shortcut: ai,
            action: AppMenuAction::FasttyAi,
            accent: true,
        }),
        item(
            IconType::Rows3,
            "SSH Manager",
            ssh,
            AppMenuAction::SshManager,
        ),
        item(
            IconType::Search,
            "Find in Buffer",
            find,
            AppMenuAction::FindInBuffer,
        ),
        MenuRow::Gap,
        item(IconType::Plus, "New Tab", new_tab, AppMenuAction::NewTab),
        item(
            IconType::Trash2,
            "Clear Scrollback",
            clear,
            AppMenuAction::ClearScrollback,
        ),
        item(
            IconType::Folder,
            "Open Config Folder",
            "",
            AppMenuAction::OpenConfigFolder,
        ),
        MenuRow::Gap,
        item(
            IconType::Maximize2,
            "Toggle Fullscreen",
            fullscreen,
            AppMenuAction::ToggleFullscreen,
        ),
        MenuRow::Gap,
        item(
            IconType::ArrowRightFromLine,
            "Quit Fastty",
            quit,
            AppMenuAction::Quit,
        ),
    ]
}

fn render_header(
    version: &str,
    status: &AppMenuStatus,
    theme: Theme,
    callback: Option<std::rc::Rc<AppMenuCallback>>,
) -> impl IntoElement {
    let about = callback.clone();
    div()
        .id("fastty-app-menu-header")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .p(px(8.))
        .flex_shrink_0()
        .rounded(px(9.))
        .bg(theme.selected)
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(30.))
                .h(px(30.))
                .rounded(px(8.))
                .flex_shrink_0()
                .bg(theme.accent.opacity(0.16))
                .child(render_app_logo(18.0)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child("Fastty"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(10.5))
                        .text_color(theme.muted)
                        .child(SharedString::from(format!("v{version}")))
                        .child(div().child("·"))
                        .child(
                            div()
                                .text_color(status.color(theme))
                                .child(SharedString::from(status.label())),
                        ),
                ),
        )
        .child(
            div()
                .id("fastty-app-menu-about")
                .flex_shrink_0()
                .px(px(5.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(10.5))
                .text_color(theme.muted)
                .cursor(CursorStyle::PointingHand)
                .hover(move |style| style.bg(theme.hover).text_color(theme.foreground))
                .child("About")
                .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                    cx.stop_propagation();
                    if let Some(callback) = about.as_ref() {
                        callback(&AppMenuAction::About, window, cx);
                    }
                }),
        )
}

fn render_row(
    icon: IconType,
    label: &'static str,
    shortcut: &'static str,
    action: AppMenuAction,
    accent: bool,
    theme: Theme,
    callback: Option<std::rc::Rc<AppMenuCallback>>,
) -> impl IntoElement {
    menu_row(
        icon,
        label,
        shortcut,
        accent,
        theme,
        move |_event, window, cx| {
            if let Some(callback) = callback.as_ref() {
                callback(&action, window, cx);
            }
        },
    )
}

/// One row of a menu: icon, label, and shortcut chips. Shared by the logo
/// menu, the terminal context menu, and the git menu.
pub(crate) fn menu_row(
    icon: IconType,
    label: &'static str,
    shortcut: &'static str,
    accent: bool,
    theme: Theme,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    menu_row_trailing(icon, label, shortcut, None, accent, theme, on_click)
}

/// Same row with a plain trailing label instead of shortcut chips, such as the
/// local branch count in the git menu.
pub(crate) fn menu_row_trailing(
    icon: IconType,
    label: &'static str,
    shortcut: &'static str,
    trailing: Option<String>,
    accent: bool,
    theme: Theme,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let hover = theme.hover;
    let active = theme.selected;
    let icon_color = if accent { theme.accent } else { theme.muted };
    div()
        .id(SharedString::from(format!("menu-row-{label}")))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .flex_shrink_0()
        .px(px(7.))
        .rounded(px(6.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(active))
        .child(render_icon(icon, icon_color, 12.5))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.foreground)
                .truncate()
                .child(label),
        )
        .when_some(trailing, |row, trailing| {
            row.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.5))
                    .text_color(theme.muted)
                    .child(SharedString::from(trailing)),
            )
        })
        .when(!shortcut.is_empty(), |row| {
            row.child(render_shortcut(shortcut, theme))
        })
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx);
        })
}

fn render_shortcut(shortcut: &'static str, theme: Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.))
        .flex_shrink_0()
        .children(shortcut_keys(shortcut).into_iter().map(|key| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(16.))
                .h(px(16.))
                .px(px(3.))
                .rounded(px(4.))
                .bg(theme.selected)
                .border_1()
                .border_color(theme.border)
                .text_size(px(9.5))
                .text_color(theme.muted_strong)
                .child(SharedString::from(key))
        }))
}

#[cfg(test)]
mod tests {
    use super::{rows, shortcut_keys, AppMenuAction, MenuRow};

    #[test]
    fn a_mac_shortcut_splits_into_modifier_and_key() {
        assert_eq!(shortcut_keys("⌘P"), ["⌘", "P"]);
        assert_eq!(shortcut_keys("⌘,"), ["⌘", ","]);
        assert_eq!(shortcut_keys("⌃⌘F"), ["⌃", "⌘", "F"]);
    }

    #[test]
    fn every_action_except_the_header_link_has_one_row() {
        let mut actions: Vec<AppMenuAction> = rows()
            .into_iter()
            .filter_map(|row| match row {
                MenuRow::Item(entry) => Some(entry.action),
                MenuRow::Gap => None,
            })
            .collect();
        let mut unique = actions.clone();
        unique.sort_by_key(|action| format!("{action:?}"));
        unique.dedup();
        assert_eq!(unique.len(), actions.len(), "duplicate rows: {actions:?}");
        actions.retain(|action| *action != AppMenuAction::About);
        for action in AppMenuAction::ALL {
            if *action == AppMenuAction::About {
                continue;
            }
            assert!(
                actions.contains(action),
                "{action:?} has no row in the logo menu"
            );
        }
    }

    #[test]
    fn a_long_key_stays_in_one_chip() {
        assert_eq!(shortcut_keys("F11"), ["F11"]);
        assert_eq!(shortcut_keys("Ctrl+Shift+P"), ["Ctrl+Shift+P"]);
        assert_eq!(shortcut_keys(""), Vec::<String>::new());
    }
}
