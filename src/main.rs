use gpui::{
    App, Bounds, KeyBinding, Menu, MenuItem, QuitMode, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, prelude::*, px, size,
};
use gpui_platform::application;
use fastty::cli::CliOptions;
use fastty::ui::RootView;

actions!(fastty, [Quit]);

fn main() {
    let _ = fastty::paths::init();
    let cli_opts = CliOptions::parse();

    application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);
            cx.set_menus([Menu::new("Fastty").items([MenuItem::action(
                "Quit Fastty",
                Quit,
            )])]);

            open_main_window(cx, cli_opts);
            cx.activate(true);
        });
}

fn open_main_window(cx: &mut App, cli_opts: CliOptions) {
    let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);

    cx.open_window(
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
            cx.new(|cx| RootView::with_options(window, cli_opts, cx))
        },
    )
    .expect("failed to open Fastty window");
}