// Suppress the console window on Windows; without this attribute a CMD/PowerShell
// window appears behind the app on every launch.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use fastty::cli::CliOptions;
use fastty::ui::RootView;
use gpui::{
    actions, prelude::*, px, size, App, Bounds, KeyBinding, Menu, MenuItem, QuitMode,
    TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use gpui_platform::application;

actions!(fastty, [Quit]);

/// `--wait` (10s default) or `--wait=N`. Returns `None` for anything else,
/// including unrelated flags -- callers use that to fall through to their
/// own flag matching rather than treating it as an error.
fn parse_wait_flag(arg: &str) -> Option<u64> {
    if arg == "--wait" {
        return Some(10);
    }
    arg.strip_prefix("--wait=")?.parse().ok()
}

fn main() {
    let _ = fastty::paths::init();

    // `sessions`/`attach` are CLI-only subcommands that talk to an already
    // running fastty's daemon (see `fastty::daemon_client`) and never touch
    // GPUI -- handled before any of the normal `-e`/`-d`/`-t` flag parsing,
    // and exit the process themselves.
    let mut subcommand_args = std::env::args().skip(1);
    match subcommand_args.next().as_deref() {
        Some("sessions") => {
            let mut watch = false;
            let mut json = false;
            let mut wait: Option<u64> = None;
            for arg in subcommand_args {
                match parse_wait_flag(&arg) {
                    Some(w) => wait = Some(w),
                    None if arg == "--watch" => watch = true,
                    None if arg == "--json" => json = true,
                    None => {
                        eprintln!("fastty sessions: unknown flag {arg}");
                        std::process::exit(1);
                    }
                }
            }
            fastty::daemon_client::run_sessions_command(watch, wait, json);
        }
        Some("gateway") => {
            let mut port: u16 = 8765;
            let mut host = "127.0.0.1".to_string();
            let mut read_only = false;
            let mut token: Option<String> = None;
            let mut iter = subcommand_args.peekable();
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "-p" | "--port" => {
                        if let Some(p) = iter.next().and_then(|s| s.parse::<u16>().ok()) {
                            port = p;
                        } else {
                            eprintln!("fastty gateway: missing or invalid port");
                            std::process::exit(1);
                        }
                    }
                    "-h" | "--host" | "--bind" => {
                        if let Some(h) = iter.next() {
                            host = h;
                        } else {
                            eprintln!("fastty gateway: missing host address");
                            std::process::exit(1);
                        }
                    }
                    "-t" | "--token" => {
                        if let Some(t) = iter.next() {
                            token = Some(t);
                        } else {
                            eprintln!("fastty gateway: missing token argument");
                            std::process::exit(1);
                        }
                    }
                    "--read-only" => {
                        read_only = true;
                    }
                    "--help" => {
                        println!(
                            "Usage: fastty gateway [--port <PORT>] [--host <ADDR>] [--token <TOKEN>] [--read-only]\n\n\
                             Options:\n  \
                             -p, --port <PORT>     Port to listen on (default: 8765)\n  \
                             -h, --host <ADDR>     Host address to bind to (default: 127.0.0.1)\n  \
                             -t, --token <TOKEN>   Access token (required on non-loopback, auto-generated if omitted)\n      \
                             --read-only           Enforce read-only access for all browser sessions\n      \
                             --help                Print this help message"
                        );
                        std::process::exit(0);
                    }
                    unknown => {
                        if let Some(p) = unknown.strip_prefix("--port=").and_then(|s| s.parse::<u16>().ok()) {
                            port = p;
                        } else if let Some(h) = unknown.strip_prefix("--host=").or_else(|| unknown.strip_prefix("--bind=")) {
                            host = h.to_string();
                        } else if let Some(t) = unknown.strip_prefix("--token=") {
                            token = Some(t.to_string());
                        } else {
                            eprintln!("fastty gateway: unknown flag {unknown}");
                            std::process::exit(1);
                        }
                    }
                }
            }
            fastty::gateway::run_gateway(&host, port, read_only, token);
            std::process::exit(0);
        }
        Some("attach") => {
            let Some(id) = subcommand_args.next().and_then(|s| s.parse::<usize>().ok()) else {
                eprintln!(
                    "Usage: fastty attach <session-id> [--read-only] [--wait[=SECONDS]]\n\
                     Run `fastty sessions` to see available session ids."
                );
                std::process::exit(1);
            };
            let mut read_only = false;
            let mut wait: Option<u64> = None;
            for arg in subcommand_args {
                match parse_wait_flag(&arg) {
                    Some(w) => wait = Some(w),
                    None if arg == "--read-only" => read_only = true,
                    None => {
                        eprintln!("fastty attach: unknown flag {arg}");
                        std::process::exit(1);
                    }
                }
            }
            fastty::daemon_client::run_attach_command(id, read_only, wait);
        }
        _ => {}
    }

    let cli_opts = CliOptions::parse();
    fastty::daemon::start();

    application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|_cx, window_id| {
                fastty::session::persist_window(window_id);
            })
            .detach();
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);
            cx.set_menus([Menu::new("Fastty").items([MenuItem::action("Quit Fastty", Quit)])]);

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
