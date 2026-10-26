// Suppress the console window on Windows; without this attribute a CMD/PowerShell
// window appears behind the app on every launch.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use fastty::cli::CliOptions;
use fastty::ui::RootView;
use gpui::{
    actions, point, prelude::*, px, size, App, Bounds, QuitMode,
    TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use gpui_platform::application;
#[cfg(not(target_os = "macos"))]
use gpui::{KeyBinding, Menu, MenuItem};

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

fn run_actions_command(args: impl Iterator<Item = String>) {
    let mut json = false;
    for arg in args {
        if arg == "--json" {
            json = true;
        } else {
            eprintln!("fastty actions: unknown flag {arg}");
            std::process::exit(2);
        }
    }
    if !json {
        eprintln!("Usage: fastty actions --json");
        std::process::exit(2);
    }

    let mut seen = std::collections::HashSet::new();
    let mut actions: Vec<_> = fastty::keybindings::Action::all()
        .into_iter()
        .map(|action| {
            seen.insert(action.binding_id());
            serde_json::json!({
                "id": action.binding_id(),
                "label": action.display_name(),
                "category": action.category(),
            })
        })
        .collect();
    for action in fastty::config::WorkflowAction::all() {
        let id = action.palette_command_id();
        if seen.insert(id.to_string()) {
            actions.push(serde_json::json!({
                "id": id,
                "label": id,
                "category": "Workflow",
            }));
        }
    }
    println!(
        "{}",
        serde_json::json!({ "schema_version": 1, "actions": actions })
    );
}

fn run_keybindings_command(args: impl Iterator<Item = String>) {
    let mut json = false;
    for arg in args {
        if arg == "--json" {
            json = true;
        } else {
            eprintln!("fastty keybindings: unknown flag {arg}");
            std::process::exit(2);
        }
    }
    if !json {
        eprintln!("Usage: fastty keybindings --json");
        std::process::exit(2);
    }

    let config = fastty::config::load_lenient();
    let preset = config.keybinding_preset.unwrap_or_default();
    let resolver = fastty::keybindings::effective_resolver(preset, &config.keybindings);
    let modes = fastty::keybindings::KeyModeResolver::new(&config.key_modes).descriptions();
    let bindings: Vec<_> = fastty::keybindings::Action::all()
        .into_iter()
        .map(|action| {
            serde_json::json!({
                "action": action.binding_id(),
                "label": action.display_name(),
                "category": action.category(),
                "keys": resolver.combos_for(action).iter().map(ToString::to_string).collect::<Vec<_>>(),
            })
        })
        .collect();
    let sequences: Vec<_> = resolver
        .sequences()
        .into_iter()
        .map(|(sequence, action)| {
            serde_json::json!({
                "keys": sequence.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "action": action.binding_id(),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({ "schema_version": 1, "preset": preset.to_string(), "bindings": bindings, "sequences": sequences, "modes": modes })
    );
}

fn main() {
    if fastty::session_host::is_host_argument(std::env::args().nth(1).as_deref()) {
        let _ = fastty::paths::init();
        fastty::paths::install_panic_log();
        fastty::session_host::run_host();
        return;
    }

    fastty::updater::cleanup_old_installations();
    let _ = fastty::paths::init();
    fastty::paths::install_panic_log();

    // `sessions`/`attach` are CLI-only subcommands that talk to an already
    // running fastty's daemon (see `fastty::daemon_client`) and never touch
    // GPUI -- handled before any of the normal `-e`/`-d`/`-t` flag parsing,
    // and exit the process themselves.
    let mut subcommand_args = std::env::args().skip(1);
    match subcommand_args.next().as_deref() {
        Some("api") => fastty::daemon_client::run_api_command(subcommand_args),
        Some("sessions") | Some("list") => {
            let mut watch = false;
            let mut json = false;
            let mut wait: Option<u64> = None;
            let mut remote: Option<String> = None;
            let mut remote_socket: Option<String> = None;
            let mut args: Vec<String> = subcommand_args.collect();
            let mut i = 0;
            while i < args.len() {
                let arg = args[i].clone();
                if arg == "--remote" || arg == "--remote-socket" {
                    let value = args.get(i + 1).cloned().filter(|v| !v.starts_with('-'));
                    match value {
                        Some(v) => {
                            if arg == "--remote" {
                                remote = Some(v)
                            } else {
                                remote_socket = Some(v)
                            }
                        }
                        None => {
                            eprintln!("fastty sessions: {} needs a value", arg);
                            std::process::exit(1);
                        }
                    }
                    args.drain(i..=i + 1);
                } else if let Some(value) = arg.strip_prefix("--remote=") {
                    remote = Some(value.to_string());
                    args.remove(i);
                } else if let Some(value) = arg.strip_prefix("--remote-socket=") {
                    remote_socket = Some(value.to_string());
                    args.remove(i);
                } else {
                    i += 1;
                }
            }
            for arg in args {
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
            if let Some(host) = remote {
                match fastty::daemon_client::setup_ssh_tunnel(&host, remote_socket) {
                    Ok(local_socket) => fastty::daemon_client::use_tunnel_socket(local_socket),
                    Err(error) => {
                        eprintln!("fastty sessions: {error}");
                        std::process::exit(1);
                    }
                }
            }
            fastty::daemon_client::run_sessions_command(watch, wait, json);
        }
        Some("capture") => {
            let Some(id) = subcommand_args
                .next()
                .and_then(|value| value.parse::<usize>().ok())
            else {
                eprintln!("Usage: fastty capture <session-id> [--wait[=SECONDS]]");
                std::process::exit(1);
            };
            let mut wait = None;
            for arg in subcommand_args {
                if let Some(seconds) = parse_wait_flag(&arg) {
                    wait = Some(seconds);
                } else {
                    eprintln!("fastty capture: unknown flag {arg}");
                    std::process::exit(1);
                }
            }
            fastty::daemon_client::run_capture_command(id, wait);
        }
        Some("focus") => {
            let Some(id) = subcommand_args
                .next()
                .and_then(|value| value.parse::<usize>().ok())
            else {
                eprintln!("Usage: fastty focus <session-id> [--wait[=SECONDS]]");
                std::process::exit(1);
            };
            let mut wait = None;
            for arg in subcommand_args {
                if let Some(seconds) = parse_wait_flag(&arg) {
                    wait = Some(seconds);
                } else {
                    eprintln!("fastty focus: unknown flag {arg}");
                    std::process::exit(1);
                }
            }
            fastty::daemon_client::run_focus_command(id, wait);
        }
        Some("send") => {
            let mut enter = false;
            let mut wait = None;
            let mut rest = Vec::new();
            let mut after_separator = false;
            for arg in subcommand_args {
                if !after_separator && arg == "--" {
                    after_separator = true;
                } else if !after_separator && arg == "--enter" {
                    enter = true;
                } else if !after_separator && parse_wait_flag(&arg).is_some() {
                    wait = parse_wait_flag(&arg);
                } else {
                    rest.push(arg);
                }
            }
            let Some(id) = rest.first().and_then(|value| value.parse::<usize>().ok()) else {
                eprintln!("Usage: fastty send <session-id> [--enter] [--wait[=SECONDS]] -- <TEXT>");
                std::process::exit(1);
            };
            if rest.len() < 2 {
                eprintln!("Usage: fastty send <session-id> [--enter] [--wait[=SECONDS]] -- <TEXT>");
                std::process::exit(1);
            }
            fastty::daemon_client::run_send_command(id, rest[1..].join(" "), enter, wait);
        }
        Some("events") => {
            let mut wait = None;
            for arg in subcommand_args {
                if let Some(seconds) = parse_wait_flag(&arg) {
                    wait = Some(seconds);
                } else if arg != "--json" {
                    eprintln!("fastty events: unknown flag {arg}");
                    std::process::exit(1);
                }
            }
            fastty::daemon_client::run_events_command(wait);
        }
        Some("host") => fastty::host_service::run_host_service_command(subcommand_args),
        Some("actions") => run_actions_command(subcommand_args),        Some("keybindings") => run_keybindings_command(subcommand_args),
        Some("agent-status") => {
            let Some(state) = subcommand_args.next() else {
                eprintln!("Usage: fastty agent-status <idle|working|done|blocked|error|clear> <claude|opencode|agy|gemini|pi|codex|hermes> [permission|question|auth]");
                std::process::exit(1);
            };
            let Some(app) = subcommand_args.next() else {
                eprintln!("Usage: fastty agent-status <idle|working|done|blocked|error|clear> <claude|opencode|agy|gemini|pi|codex|hermes> [permission|question|auth]");
                std::process::exit(1);
            };
            let kind = subcommand_args.next();
            if subcommand_args.next().is_some() {
                eprintln!("fastty agent-status: too many arguments");
                std::process::exit(1);
            }
            fastty::daemon_client::run_agent_status_command(state, app, kind);
        }
        Some("agent-hook") => {
            let Some(provider) = subcommand_args.next() else {
                eprintln!("Usage: fastty agent-hook <claude|gemini|codex|agy> | install|uninstall <provider> --project|--user");
                std::process::exit(1);
            };
            if matches!(provider.as_str(), "install" | "uninstall") {
                let Some(agent) = subcommand_args.next() else {
                    eprintln!("fastty agent-hook: provider is required");
                    std::process::exit(1);
                };
                let scope = match subcommand_args.next().as_deref() {
                    Some("--project") => fastty::agent_hooks::InstallScope::Project,
                    Some("--user") => fastty::agent_hooks::InstallScope::User,
                    _ => {
                        eprintln!("fastty agent-hook: choose --project or --user");
                        std::process::exit(1);
                    }
                };
                if subcommand_args.next().is_some() {
                    eprintln!("fastty agent-hook: too many arguments");
                    std::process::exit(1);
                }
                let result = if provider == "install" {
                    fastty::agent_hooks::install(&agent, scope)
                } else {
                    fastty::agent_hooks::uninstall(&agent, scope)
                };
                match result {
                    Ok(path) => {
                        println!("{}: {}", provider, path.display());
                        if provider == "install" && agent == "hermes" {
                            println!("Enable the plugin with: hermes plugins enable fastty-status");
                        }
                        std::process::exit(0);
                    }
                    Err(error) => {
                        eprintln!("fastty agent-hook: {error}");
                        std::process::exit(1);
                    }
                }
            } else {
                fastty::daemon_client::run_agent_hook_command(provider, subcommand_args.next());
            }
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
                        if let Some(p) = unknown
                            .strip_prefix("--port=")
                            .and_then(|s| s.parse::<u16>().ok())
                        {
                            port = p;
                        } else if let Some(h) = unknown
                            .strip_prefix("--host=")
                            .or_else(|| unknown.strip_prefix("--bind="))
                        {
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
            let mut flags: Vec<String> = subcommand_args.collect();
            // Remote flags can sit before or after the session id.
            let mut remote: Option<String> = None;
            let mut remote_socket: Option<String> = None;
            let mut i = 0;
            while i < flags.len() {
                let arg = flags[i].clone();
                if arg == "--remote" || arg == "--remote-socket" {
                    let value = flags.get(i + 1).cloned().filter(|v| !v.starts_with('-'));
                    match value {
                        Some(v) => {
                            if arg == "--remote" {
                                remote = Some(v)
                            } else {
                                remote_socket = Some(v)
                            }
                        }
                        None => {
                            eprintln!("fastty attach: {} needs a value", arg);
                            std::process::exit(1);
                        }
                    }
                    flags.drain(i..=i + 1);
                } else if let Some(value) = arg.strip_prefix("--remote=") {
                    remote = Some(value.to_string());
                    flags.remove(i);
                } else if let Some(value) = arg.strip_prefix("--remote-socket=") {
                    remote_socket = Some(value.to_string());
                    flags.remove(i);
                } else {
                    i += 1;
                }
            }
            let mut parts = flags.into_iter();
            let Some(id) = parts.next().and_then(|s| s.parse::<usize>().ok()) else {
                eprintln!(
                    "Usage: fastty attach [--remote HOST] [--remote-socket PATH] <session-id> [--read-only] [--wait[=SECONDS]]\n\
                     Run `fastty sessions` to see available session ids."
                );
                std::process::exit(1);
            };
            let mut read_only = false;
            let mut wait: Option<u64> = None;
            for arg in parts {
                match parse_wait_flag(&arg) {
                    Some(w) => wait = Some(w),
                    None if arg == "--read-only" => read_only = true,
                    None => {
                        eprintln!("fastty attach: unknown flag {arg}");
                        std::process::exit(1);
                    }
                }
            }
            if let Some(host) = remote {
                match fastty::daemon_client::setup_ssh_tunnel(&host, remote_socket) {
                    Ok(local_socket) => fastty::daemon_client::use_tunnel_socket(local_socket),
                    Err(error) => {
                        eprintln!("fastty attach: {error}");
                        std::process::exit(1);
                    }
                }
            }
            fastty::daemon_client::run_attach_command(id, read_only, wait);
        }
        Some("spawn") => {
            // Flags before the command; `--` (or the first non-flag) hands
            // the rest over verbatim, so `fastty spawn -- bash -c 'x'` works.
            let mut args: Vec<String> = Vec::new();
            let mut cwd: Option<String> = None;
            let mut cols: Option<usize> = None;
            let mut rows: Option<usize> = None;
            let mut json = false;
            let mut open = false;
            let mut wait: Option<u64> = None;
            let mut rest: Vec<String> = Vec::new();
            while let Some(arg) = subcommand_args.next() {
                if rest.is_empty() {
                    if arg == "--" {
                        continue;
                    } else if let Some(w) = parse_wait_flag(&arg) {
                        wait = Some(w);
                    } else if arg == "--json" {
                        json = true;
                    } else if arg == "--open" {
                        open = true;
                    } else if arg == "--cwd" {
                        cwd = subcommand_args.next();
                    } else if let Some(v) = arg.strip_prefix("--cwd=") {
                        cwd = Some(v.to_string());
                    } else if arg == "--cols" {
                        cols = subcommand_args.next().and_then(|v| v.parse().ok());
                    } else if let Some(v) = arg.strip_prefix("--cols=") {
                        cols = v.parse().ok();
                    } else if arg == "--rows" {
                        rows = subcommand_args.next().and_then(|v| v.parse().ok());
                    } else if let Some(v) = arg.strip_prefix("--rows=") {
                        rows = v.parse().ok();
                    } else if arg.starts_with('-') && arg.len() > 1 {
                        eprintln!("fastty spawn: unknown flag {arg}");
                        std::process::exit(1);
                    } else {
                        rest.push(arg);
                    }
                } else {
                    rest.push(arg);
                }
            }
            let command = rest.first().cloned();
            if rest.len() > 1 {
                args = rest[1..].to_vec();
            }
            fastty::daemon_client::run_spawn_command(
                command, args, cwd, cols, rows, open, wait, json,
            );
        }
        Some("split") => {
            // `fastty split <pane-id> --direction right [--cwd DIR] [-- COMMAND]`
            let Some(pane_id) = subcommand_args.next().and_then(|s| s.parse::<usize>().ok()) else {
                eprintln!("Usage: fastty split <pane-id> --direction <left|right|top|down> [--cwd DIR] [--json] [--wait[=SECONDS]] [--] [COMMAND [ARGS...]]");
                std::process::exit(1);
            };
            let mut direction = String::new();
            let mut args: Vec<String> = Vec::new();
            let mut cwd: Option<String> = None;
            let mut json = false;
            let mut wait: Option<u64> = None;
            let mut rest: Vec<String> = Vec::new();
            while let Some(arg) = subcommand_args.next() {
                if rest.is_empty() {
                    if arg == "--" {
                        continue;
                    } else if let Some(w) = parse_wait_flag(&arg) {
                        wait = Some(w);
                    } else if arg == "--json" {
                        json = true;
                    } else if let Some(v) = arg.strip_prefix("--direction=") {
                        direction = v.to_string();
                    } else if arg == "--direction" {
                        direction = subcommand_args.next().unwrap_or_default();
                    } else if arg == "--cwd" {
                        cwd = subcommand_args.next();
                    } else if let Some(v) = arg.strip_prefix("--cwd=") {
                        cwd = Some(v.to_string());
                    } else if arg.starts_with('-') && arg.len() > 1 {
                        eprintln!("fastty split: unknown flag {arg}");
                        std::process::exit(1);
                    } else {
                        rest.push(arg);
                    }
                } else {
                    rest.push(arg);
                }
            }
            if !matches!(direction.as_str(), "left" | "right" | "top" | "down") {
                eprintln!("fastty split: --direction must be left, right, top, or down");
                std::process::exit(1);
            }
            let command = rest.first().cloned();
            if rest.len() > 1 {
                args = rest[1..].to_vec();
            }
            fastty::daemon_client::run_split_pane_command(
                pane_id, direction, command, args, cwd, wait, json,
            );
        }
        Some("close") => {
            let Some(id) = subcommand_args.next().and_then(|s| s.parse::<usize>().ok()) else {
                eprintln!("Usage: fastty close <session-id> [--force] [--wait[=SECONDS]]");
                std::process::exit(1);
            };
            let mut wait: Option<u64> = None;
            let mut force = false;
            for arg in subcommand_args {
                match parse_wait_flag(&arg) {
                    Some(w) => wait = Some(w),
                    None if arg == "--force" => force = true,
                    None => {
                        eprintln!("fastty close: unknown flag {arg}");
                        std::process::exit(1);
                    }
                }
            }
            fastty::daemon_client::run_close_command(id, force, wait);
        }
        Some("resize") => {
            let mut rest: Vec<String> = Vec::new();
            let mut wait: Option<u64> = None;
            for arg in subcommand_args {
                if let Some(w) = parse_wait_flag(&arg) {
                    wait = Some(w);
                } else {
                    rest.push(arg);
                }
            }
            let (Some(id), Some(cols), Some(rows)) = (
                rest.first().and_then(|s| s.parse::<usize>().ok()),
                rest.get(1).and_then(|s| s.parse::<usize>().ok()),
                rest.get(2).and_then(|s| s.parse::<usize>().ok()),
            ) else {
                eprintln!("Usage: fastty resize <session-id> <cols> <rows> [--wait[=SECONDS]]");
                std::process::exit(1);
            };
            fastty::daemon_client::run_resize_command(id, cols, rows, wait);
        }
        Some("write") => {
            let mut rest: Vec<String> = Vec::new();
            let mut enter = false;
            let mut wait: Option<u64> = None;
            for arg in subcommand_args {
                if let Some(w) = parse_wait_flag(&arg) {
                    wait = Some(w);
                } else if arg == "--enter" {
                    enter = true;
                } else {
                    rest.push(arg);
                }
            }
            let Some(id) = rest.first().and_then(|s| s.parse::<usize>().ok()) else {
                eprintln!("Usage: fastty write <session-id> [--enter] <TEXT...>");
                std::process::exit(1);
            };
            if rest.len() < 2 {
                eprintln!("Usage: fastty write <session-id> [--enter] <TEXT...>");
                std::process::exit(1);
            }
            let text = rest[1..].join(" ");
            fastty::daemon_client::run_write_command(id, text, enter, wait);
        }
        Some("ask") => {
            run_ask_command(subcommand_args);
            std::process::exit(0);
        }
        Some("mcp") => {
            let mcp_args: Vec<String> = subcommand_args.collect();
            if mcp_args.first().map(String::as_str) == Some("setup") {
                let code = fastty::mcp::run_mcp_setup(mcp_args.get(1).map(String::as_str));
                std::process::exit(code);
            }
            if let Some(arg) = mcp_args.first() {
                eprintln!("fastty mcp: unknown argument {arg}");
                eprintln!("Usage: fastty mcp [setup [AGENT]]");
                std::process::exit(1);
            }
            // MCP server over stdio: stdout carries JSON-RPC only.
            fastty::mcp::run_mcp_server();
        }
        _ => {}
    }

    let cli_opts = CliOptions::parse();
    let restore_session = fastty::config::load_lenient().session_restore
        && cli_opts.command.is_none()
        && cli_opts.working_dir.is_none()
        && cli_opts.title.is_none();
    if let Err(error) = fastty::config::start_config_watcher(
        fastty::config::Config::get_active_config_path(),
        fastty::config::increment_config_version,
    ) {
        eprintln!("Fastty could not watch its settings file: {error}");
    }
    let saved_session = if restore_session {
        match fastty::session::load_result() {
            Ok(session) => session,
            Err(error) => {
                eprintln!("Fastty could not load the saved session: {error}");
                None
            }
        }
    } else {
        None
    };
    if let Err(error) = fastty::session_host::ensure_running() {
        eprintln!("fastty: {error}");
        std::process::exit(1);
    }

    application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            #[cfg(target_os = "macos")]
            fastty::notifications::request_permission();
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|_cx, window_id| {
                if let Err(error) = fastty::session::persist_window(window_id) {
                    eprintln!("Fastty could not save the session after closing a window: {error}");
                }
            })
            .detach();
            cx.on_app_quit(|cx| {
                let active_window = cx
                    .window_stack()
                    .and_then(|windows| windows.first().map(|window| window.window_id()));
                for window in cx.windows() {
                    if let Some(root) = window.downcast::<RootView>() {
                        if let Err(error) = root.update(cx, |view, window, _cx| {
                            view.persist_session_for_quit(window);
                        }) {
                            eprintln!("Fastty could not read a window before quit: {error}");
                        }
                    }
                }
                if let Err(error) = fastty::session::persist_before_quit(active_window) {
                    eprintln!("Fastty could not save the session before quit: {error}");
                }
                std::future::ready(())
            })
            .detach();
            #[cfg(not(target_os = "macos"))]
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);
            #[cfg(not(target_os = "macos"))]
            cx.set_menus([Menu::new("Fastty").items([MenuItem::action("Quit Fastty", Quit)])]);
            #[cfg(target_os = "macos")]
            fastty::ui::native_menu::install(cx);

            if let Some(session) = saved_session.filter(|session| !session.windows.is_empty()) {
                let active_window = session.active_window.min(session.windows.len() - 1);
                fastty::session::begin_restore(&session);
                for (index, saved_window) in session.windows.iter().cloned().enumerate() {
                    if index != active_window {
                        open_main_window(cx, CliOptions::default(), Some((index, saved_window)));
                    }
                }
                open_main_window(
                    cx,
                    CliOptions::default(),
                    Some((active_window, session.windows[active_window].clone())),
                );
            } else {
                open_main_window(cx, cli_opts, None);
            }
            cx.activate(true);
        });
}

fn open_main_window(
    cx: &mut App,
    cli_opts: CliOptions,
    restored_window: Option<(usize, fastty::session::WindowSession)>,
) {
    let bounds = restored_window
        .as_ref()
        .and_then(|(_, saved)| match (saved.position, saved.size) {
            (Some((x, y)), Some((width, height)))
                if (640..=10_000).contains(&width) && (420..=10_000).contains(&height) =>
            {
                Some(Bounds {
                    origin: point(px(x as f32), px(y as f32)),
                    size: size(px(width as f32), px(height as f32)),
                })
            }
            _ => None,
        })
        .unwrap_or_else(|| Bounds::centered(None, size(px(960.), px(640.)), cx));

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
            cx.new(|cx| match restored_window {
                Some((index, saved)) => {
                    RootView::with_restored_window(window, cli_opts, index, saved, cx)
                }
                None => RootView::with_options(window, cli_opts, cx),
            })
        },
    )
    .expect("failed to open Fastty window");
}

fn run_ask_command(mut args: impl Iterator<Item = String>) {
    use std::io::Write;
    use std::sync::Arc;

    let mut prompt_parts = Vec::new();
    let mut provider_override: Option<String> = None;
    let mut model_override: Option<String> = None;
    let mut no_tools = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-p" | "--provider" => {
                provider_override = args.next();
            }
            "-m" | "--model" => {
                model_override = args.next();
            }
            "--no-tools" => {
                no_tools = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: fastty ask [OPTIONS] <PROMPT>\n\n\
                     Ask the Fastty AI agent directly from the terminal.\n\n\
                     Options:\n  \
                     -p, --provider <NAME>  Specify AI provider (e.g. ollama, anthropic)\n  \
                     -m, --model <MODEL>    Specify model name\n  \
                     --no-tools             Disable tool execution\n  \
                     -h, --help             Print this help message"
                );
                std::process::exit(0);
            }
            other => {
                prompt_parts.push(other.to_string());
            }
        }
    }

    let prompt = prompt_parts.join(" ");
    if prompt.trim().is_empty() {
        eprintln!("fastty ask: prompt cannot be empty. Run `fastty ask --help` for usage.");
        std::process::exit(1);
    }

    let cfg = fastty::config::load_lenient();
    let (model, model_name) = match fastty::ai::create_model_from_config(
        &cfg.ai,
        provider_override.as_deref(),
        model_override.as_deref(),
    ) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("fastty ask error: {}", e);
            std::process::exit(1);
        }
    };

    let tools = if no_tools {
        Vec::new()
    } else {
        fastty::ai::default_tools()
    };

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let checker = Arc::new(fastty::ai::PermissionChecker::new(cfg.ai.permission_mode));
    let perm_handler = Arc::new(fastty::ai::CliPermissionHandler::new(checker.clone()));
    let ctx = fastty::ai::ToolCtx { cwd: cwd.clone() };

    let mut agent = fastty::ai::Agent::new(model, model_name, tools, checker, perm_handler, ctx);

    agent.add_message(fastty::ai::Message::system(fastty::ai::system_prompt(&cwd)));
    agent.add_message(fastty::ai::Message::user(prompt));

    if let Err(e) = agent.run_turn(|ev| match ev {
        fastty::ai::AgentEvent::TextDelta(d) => {
            print!("{}", d);
            let _ = std::io::stdout().flush();
        }
        fastty::ai::AgentEvent::ThinkingDelta(t) => {
            print!("\x1b[2m{}\x1b[0m", t);
            let _ = std::io::stdout().flush();
        }
        fastty::ai::AgentEvent::ToolStart { name, args, .. } => {
            println!("\n\x1b[36m⚡ Running tool '{}':\x1b[0m {}", name, args);
        }
        fastty::ai::AgentEvent::ToolEnd {
            output, is_error, ..
        } => {
            if is_error {
                println!("\x1b[31m✗ Tool error:\x1b[0m\n{}", output);
            } else {
                println!("\x1b[32m✓ Tool result:\x1b[0m\n{}", output);
            }
        }
        fastty::ai::AgentEvent::TurnEnd => {
            println!();
        }
        fastty::ai::AgentEvent::Usage { .. } => {}
        fastty::ai::AgentEvent::ToolArgs { .. } => {}
        fastty::ai::AgentEvent::ContextWindow { .. }
        | fastty::ai::AgentEvent::ConversationSession(_)
        | fastty::ai::AgentEvent::AvailableCommands(_) => {}
        fastty::ai::AgentEvent::Error(err) => {
            eprintln!("\n\x1b[31m[fastty-ai error]: {}\x1b[0m", err);
        }
    }) {
        eprintln!("Turn failed: {}", e);
        std::process::exit(1);
    }
}
