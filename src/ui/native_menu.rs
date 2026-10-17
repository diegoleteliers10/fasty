use crate::keybindings::{Action as TerminalAction, KeyBindingResolver, KeyCombo, NamedKey};
use crate::ui::{RootView, SettingsView};
use gpui::{
    actions, div, prelude::*, px, size, App, Bounds, Context, Global, KeyBinding, KeyDownEvent,
    Menu, MenuItem, OsAction, SystemMenuType, TitlebarOptions, Window, WindowBackgroundAppearance,
    WindowBounds, WindowOptions,
};

actions!(
    fastty_menu,
    [
        About,
        OpenSettings,
        CheckForUpdates,
        Hide,
        HideOthers,
        ShowAll,
        Quit,
        NewWindow,
        NewTab,
        SplitRight,
        SplitDown,
        ClosePane,
        CloseTab,
        Copy,
        Paste,
        SelectAll,
        Find,
        CommandPalette,
        ToggleAiPanel,
        ToggleGitPanel,
        ToggleSidebar,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        ToggleFullscreen,
        Minimize,
        NextWindow,
        BringAllToFront,
        Documentation,
        ReportIssue,
        CopyDiagnostics,
    ]
);

struct MenuBindings(Vec<(String, String)>);
impl Global for MenuBindings {}

pub fn install(cx: &mut App) {
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &NewWindow, cx| {
        if !active_root(cx) {
            open_terminal_window(cx);
        }
    });
    cx.on_action(|_: &OpenSettings, cx| {
        if !active_root(cx) {
            open_settings(cx);
        }
    });
    cx.on_action(|_: &About, cx| {
        if active_root(cx) {
            return;
        }
        if let Some(root) = cx
            .windows()
            .into_iter()
            .find_map(|window| window.downcast::<RootView>())
        {
            let _ = root.update(cx, |_, window, cx| {
                window.activate_window();
                window.dispatch_action(Box::new(About), cx);
            });
        } else {
            open_about(cx);
        }
    });
    cx.on_action(|_: &CheckForUpdates, cx| {
        if !active_root(cx) {
            open_settings(cx);
            let settings = *crate::ui::settings_view::SETTINGS_WINDOW_HANDLE.lock();
            if let Some(handle) = settings {
                let _ = handle.update(cx, |view, window, cx| {
                    view.active_tab = crate::ui::settings_view::SettingsTab::General;
                    view.check_updates_now(window, cx);
                    cx.notify();
                });
            }
        }
    });
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.on_action(|_: &NextWindow, cx| {
        let windows = cx.windows();
        if windows.len() < 2 {
            return;
        }
        let active = cx.active_window();
        let index = windows
            .iter()
            .position(|window| Some(*window) == active)
            .unwrap_or(0);
        let next = windows[(index + 1) % windows.len()];
        let _ = next.update(cx, |_, window, _| window.activate_window());
    });
    cx.on_action(|_: &BringAllToFront, cx| {
        let active = cx.active_window();
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        if let Some(active) = active {
            let _ = active.update(cx, |_, window, _| window.activate_window());
        }
    });
    cx.on_action(|_: &Documentation, cx| {
        cx.open_url("https://github.com/diegoleteliers10/fasty#readme")
    });
    cx.on_action(|_: &ReportIssue, cx| {
        cx.open_url("https://github.com/diegoleteliers10/fasty/issues/new")
    });
    cx.on_action(|_: &CopyDiagnostics, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(format!(
            "Fastty {}\nOS: {}\nArchitecture: {}\nRenderer: GPUI\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        )));
    });
    refresh_bindings(cx);
}

fn active_root(cx: &App) -> bool {
    cx.active_window()
        .is_some_and(|window| window.downcast::<RootView>().is_some())
}

pub fn refresh_bindings(cx: &mut App) {
    let resolver = crate::keybindings::RESOLVER
        .get_or_init(|| parking_lot::RwLock::new(KeyBindingResolver::with_defaults()));
    let resolver = resolver.read();
    let mut bindings = Vec::new();
    let mut signature = Vec::new();
    macro_rules! bind {
        ($terminal:ident, $menu:ident) => {
            for combo in resolver.combos_for(TerminalAction::$terminal) {
                let key = gpui_keystroke(combo);
                signature.push((stringify!($menu).to_string(), key.clone()));
                bindings.push(KeyBinding::new(
                    &key,
                    $menu,
                    Some("FasttyMenuLabel && !FasttyMenuLabel"),
                ));
            }
        };
    }
    bind!(OpenSettings, OpenSettings);
    bind!(NewWindow, NewWindow);
    bind!(NewTab, NewTab);
    bind!(SplitRight, SplitRight);
    bind!(SplitDown, SplitDown);
    bind!(ClosePane, ClosePane);
    bind!(CloseTab, CloseTab);
    bind!(Copy, Copy);
    bind!(Paste, Paste);
    bind!(OpenSearch, Find);
    bind!(CommandPalette, CommandPalette);
    bind!(ToggleAiSidebar, ToggleAiPanel);
    bind!(ToggleProjectSidebar, ToggleGitPanel);
    bind!(ToggleTabSidebar, ToggleSidebar);
    bind!(IncreaseFontSize, IncreaseFontSize);
    bind!(DecreaseFontSize, DecreaseFontSize);
    bind!(ResetFontSize, ResetFontSize);
    bind!(ToggleFullscreen, ToggleFullscreen);
    bind!(Quit, Quit);
    macro_rules! fixed {
        ($key:literal, $menu:ident) => {
            if crate::keybindings::parse_combo(&$key.replace('-', "+").replace("cmd+", "super+"))
                .is_some_and(|combo| resolver.resolve(&combo).is_none())
            {
                signature.push((stringify!($menu).to_string(), $key.to_string()));
                bindings.push(KeyBinding::new(
                    $key,
                    $menu,
                    Some("FasttyMenuLabel && !FasttyMenuLabel"),
                ));
            }
        };
    }
    fixed!("cmd-a", SelectAll);
    fixed!("cmd-h", Hide);
    fixed!("cmd-alt-h", HideOthers);
    fixed!("cmd-m", Minimize);
    fixed!("cmd-`", NextWindow);
    drop(resolver);
    if cx
        .try_global::<MenuBindings>()
        .is_some_and(|state| state.0 == signature)
    {
        return;
    }
    cx.clear_key_bindings();
    cx.bind_keys(bindings);
    cx.set_global(MenuBindings(signature));
    set_menus(cx);
}

fn event_combo(event: &KeyDownEvent) -> Option<KeyCombo> {
    let modifiers = &event.keystroke.modifiers;
    let alt_gr = modifiers.control && modifiers.alt && !modifiers.platform;
    if alt_gr || event.prefer_character_input {
        return None;
    }
    crate::keybindings::combo_from_key(
        &event.keystroke.key,
        modifiers.control,
        modifiers.shift,
        modifiers.alt,
        modifiers.platform,
    )
}

fn configured_menu_action(action: TerminalAction) -> Option<Box<dyn gpui::Action>> {
    macro_rules! action {
        ($action:ident) => {
            Some(Box::new($action) as Box<dyn gpui::Action>)
        };
    }
    match action {
        TerminalAction::NewWindow => action!(NewWindow),
        TerminalAction::OpenSettings => action!(OpenSettings),
        TerminalAction::Copy => action!(Copy),
        TerminalAction::Paste => action!(Paste),
        TerminalAction::OpenSearch => action!(Find),
        TerminalAction::Quit => action!(Quit),
        _ => None,
    }
}

fn system_action(combo: KeyCombo) -> Option<Box<dyn gpui::Action>> {
    if !combo.logo || combo.ctrl || combo.shift {
        return None;
    }
    match (combo.key, combo.alt) {
        (NamedKey::Char('a'), false) => Some(Box::new(SelectAll)),
        (NamedKey::Char('h'), false) => Some(Box::new(Hide)),
        (NamedKey::Char('h'), true) => Some(Box::new(HideOthers)),
        (NamedKey::Char('m'), false) => Some(Box::new(Minimize)),
        (NamedKey::Char('`'), false) => Some(Box::new(NextWindow)),
        _ => None,
    }
}

fn dispatch_if_available(action: Box<dyn gpui::Action>, window: &mut Window, cx: &mut App) -> bool {
    if !window.is_action_available(action.as_ref(), cx) {
        return false;
    }
    window.dispatch_action(action, cx);
    true
}

pub fn dispatch_system_shortcut(event: &KeyDownEvent, window: &mut Window, cx: &mut App) -> bool {
    let Some(combo) = event_combo(event) else {
        return false;
    };
    if crate::keybindings::RESOLVER
        .get()
        .is_some_and(|resolver| resolver.read().resolve(&combo).is_some())
    {
        return false;
    }
    system_action(combo).is_some_and(|action| dispatch_if_available(action, window, cx))
}

pub fn dispatch_settings_shortcut(
    event: &KeyDownEvent,
    input_active: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(combo) = event_combo(event) else {
        return false;
    };
    if input_active
        && !combo.logo
        && !combo.ctrl
        && !combo.alt
        && !matches!(
            combo.key,
            NamedKey::F1
                | NamedKey::F2
                | NamedKey::F3
                | NamedKey::F4
                | NamedKey::F5
                | NamedKey::F6
                | NamedKey::F7
                | NamedKey::F8
                | NamedKey::F9
                | NamedKey::F10
                | NamedKey::F11
                | NamedKey::F12
        )
    {
        return false;
    }
    let configured = crate::keybindings::RESOLVER
        .get()
        .and_then(|resolver| resolver.read().resolve(&combo));
    let action = match configured {
        Some(action) => configured_menu_action(action),
        None => system_action(combo),
    };
    action.is_some_and(|action| dispatch_if_available(action, window, cx))
}

fn gpui_keystroke(combo: KeyCombo) -> String {
    let mut parts = Vec::new();
    if combo.logo {
        parts.push("cmd".to_string());
    }
    if combo.ctrl {
        parts.push("ctrl".to_string());
    }
    if combo.alt {
        parts.push("alt".to_string());
    }
    if combo.shift {
        parts.push("shift".to_string());
    }
    parts.push(match combo.key {
        NamedKey::Char(value) => value.to_string(),
        NamedKey::Plus => "+".to_string(),
        NamedKey::Minus => "-".to_string(),
        NamedKey::Equal => "=".to_string(),
        _ => KeyCombo {
            ctrl: false,
            shift: false,
            alt: false,
            logo: false,
            key: combo.key,
        }
        .to_string(),
    });
    parts.join("-")
}

fn set_menus(cx: &App) {
    cx.set_menus([
        Menu::new("Fastty").items([
            MenuItem::action("About Fastty", About),
            MenuItem::separator(),
            MenuItem::action("Settings...", OpenSettings),
            MenuItem::action("Check for Updates...", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Fastty", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Fastty", Quit),
        ]),
        Menu::new("Terminal").items([
            MenuItem::action("New Window", NewWindow),
            MenuItem::action("New Tab", NewTab),
            MenuItem::separator(),
            MenuItem::action("Split Right", SplitRight),
            MenuItem::action("Split Down", SplitDown),
            MenuItem::separator(),
            MenuItem::action("Close Pane", ClosePane),
            MenuItem::action("Close Tab", CloseTab),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Find...", Find),
        ]),
        Menu::new("View").items([
            MenuItem::action("Command Palette...", CommandPalette),
            MenuItem::separator(),
            MenuItem::action("AI Panel", ToggleAiPanel),
            MenuItem::action("Git Panel", ToggleGitPanel),
            MenuItem::action("Tab Sidebar", ToggleSidebar),
            MenuItem::separator(),
            MenuItem::action("Increase Font Size", IncreaseFontSize),
            MenuItem::action("Decrease Font Size", DecreaseFontSize),
            MenuItem::action("Reset Font Size", ResetFontSize),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", ToggleFullscreen),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Next Window", NextWindow),
            MenuItem::separator(),
            MenuItem::action("Bring All to Front", BringAllToFront),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Fastty Documentation", Documentation),
            MenuItem::action("Report an Issue...", ReportIssue),
            MenuItem::action("Copy Diagnostics", CopyDiagnostics),
        ]),
    ]);
}

fn open_terminal_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(640.), px(420.))),
            window_background: WindowBackgroundAppearance::Blurred,
            app_id: Some("com.fastty.app".into()),
            titlebar: Some(TitlebarOptions {
                title: Some("Fastty".into()),
                appears_transparent: true,
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            window.set_background_appearance(WindowBackgroundAppearance::Blurred);
            cx.new(|cx| RootView::with_options(window, crate::cli::CliOptions::default(), cx))
        },
    );
}

fn open_settings(cx: &mut App) {
    let existing = *crate::ui::settings_view::SETTINGS_WINDOW_HANDLE.lock();
    if let Some(handle) = existing {
        if handle
            .update(cx, |view, window, cx| {
                window.activate_window();
                window.focus(&view.focus_handle, cx);
            })
            .is_ok()
        {
            return;
        }
    }
    let bounds = Bounds::centered(None, size(px(880.), px(640.)), cx);
    if let Ok(handle) = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(740.), px(520.))),
            window_background: WindowBackgroundAppearance::Blurred,
            app_id: Some("com.fastty.app.settings".into()),
            titlebar: Some(TitlebarOptions {
                title: Some("Fastty Settings".into()),
                appears_transparent: true,
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            window.set_background_appearance(WindowBackgroundAppearance::Blurred);
            cx.new(|cx| SettingsView::new(window, cx))
        },
    ) {
        *crate::ui::settings_view::SETTINGS_WINDOW_HANDLE.lock() = Some(handle);
    }
}

struct AboutWindow;

impl Render for AboutWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .bg(gpui::rgb(0x202127))
            .text_color(gpui::rgb(0xe6e6e6))
            .child(div().text_xl().child("Fastty"))
            .child(format!("Version {}", env!("CARGO_PKG_VERSION")))
    }
}

fn open_about(cx: &mut App) {
    if let Some(handle) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<AboutWindow>())
    {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let bounds = Bounds::centered(None, size(px(340.), px(180.)), cx);
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("About Fastty".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |_, cx| cx.new(|_| AboutWindow),
    );
}
