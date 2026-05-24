//! Biovity Terminal -- GPU-native terminal emulator powered by GPUI

mod app;
mod config;
mod input;
mod parser;
mod pty;
mod settings;
mod terminal;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::Config as AlacrittyConfig;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use app::AppView;
use flume;
use gpui::{
    px, App, AppContext, Entity, Styled, Window, WindowBounds, WindowDecorations, WindowOptions,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use terminal::{Terminal, TerminalView};

fn get_login_shell() -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/passwd") {
        let current_user = std::env::var("USER").unwrap_or_else(|_| "diegoleteliers".to_string());
        for line in content.lines() {
            if let Some(username) = line.split(':').next() {
                if username == current_user {
                    if let Some(shell) = line.split(':').last() {
                        if !shell.is_empty() {
                            return shell.trim().to_string();
                        }
                    }
                }
            }
        }
    }
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

struct PreSpawnedPty {
    term: Arc<Mutex<alacritty_terminal::term::Term<VoidListener>>>,
    render_generation: Arc<AtomicU64>,
    pty_writer: pty::PtyWriter,
    grid_wake_rx: flume::Receiver<()>,
}

fn pre_spawn_pty(shell: &str, scrollback: usize) -> PreSpawnedPty {
    let cols = 80;
    let rows = 24;

    let mut config = AlacrittyConfig::default();
    config.scrolling_history = scrollback;
    let size = TermSize::new(cols, rows);
    let term = Arc::new(Mutex::new(alacritty_terminal::term::Term::new(
        config,
        &size,
        VoidListener,
    )));
    let render_generation = Arc::new(AtomicU64::new(0));
    let render_generation_worker = Arc::clone(&render_generation);
    let (grid_wake_tx, grid_wake_rx) = flume::unbounded::<()>();

    let term_clone = Arc::clone(&term);
    let pty_worker = pty::PtyWorker::spawn(&[shell.to_string()], cols, rows, move |chunk| {
        if chunk.is_empty() {
            return;
        }
        let mut term_locked = match term_clone.lock() {
            Ok(t) => t,
            Err(_) => return,
        };
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut *term_locked, &chunk);
        render_generation_worker.fetch_add(1, Ordering::Relaxed);
        drop(term_locked);
        let _ = grid_wake_tx.send(());
    });

    let pty_writer = pty_worker.writer.clone();

    PreSpawnedPty {
        term,
        render_generation,
        pty_writer,
        grid_wake_rx,
    }
}

fn spawn_pty(shell: &str, scrollback: usize) -> PreSpawnedPty {
    let cols = 80;
    let rows = 24;

    let mut config = AlacrittyConfig::default();
    config.scrolling_history = scrollback;
    let size = TermSize::new(cols, rows);
    let term = Arc::new(Mutex::new(alacritty_terminal::term::Term::new(
        config,
        &size,
        VoidListener,
    )));
    let render_generation = Arc::new(AtomicU64::new(0));
    let render_generation_worker = Arc::clone(&render_generation);
    let (grid_wake_tx, grid_wake_rx) = flume::unbounded::<()>();

    let term_clone = Arc::clone(&term);
    let pty_worker = pty::PtyWorker::spawn(&[shell.to_string()], cols, rows, move |chunk| {
        if chunk.is_empty() {
            return;
        }
        let mut term_locked = match term_clone.lock() {
            Ok(t) => t,
            Err(_) => return,
        };
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut *term_locked, &chunk);
        render_generation_worker.fetch_add(1, Ordering::Relaxed);
        drop(term_locked);
        let _ = grid_wake_tx.send(());
    });

    let pty_writer = pty_worker.writer.clone();

    PreSpawnedPty {
        term,
        render_generation,
        pty_writer,
        grid_wake_rx,
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = config::Config::load()?;
    tracing::info!(
        "fasty: starting, shell={}",
        config.shell.as_deref().unwrap_or("default")
    );

    let shell = if let Some(ref s) = config.shell {
        s.clone()
    } else {
        get_login_shell()
    };

    let pre_spawned = pre_spawn_pty(&shell, config.scrollback);
    tracing::info!("fasty: PTY pre-spawned successfully");

    let app_config = config.clone();
    let app = gpui::Application::new();

    app.run(move |cx: &mut App| {
        let PreSpawnedPty {
            term,
            render_generation,
            pty_writer,
            grid_wake_rx,
        } = pre_spawned;

        let font_config = config.font.clone();

        let window_bounds = gpui::Bounds::new(
            gpui::point(px(0.0), px(0.0)),
            gpui::size(px(1200.0), px(800.0)),
        );

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(window_bounds)),
                window_decorations: Some(WindowDecorations::Client),
                titlebar: None,
                is_resizable: true,
                ..Default::default()
            },
            move |window, cx| {
                let terminal = cx.new(|cx| {
                    Terminal::from_pre_spawned(
                        cx,
                        pty_writer,
                        term,
                        render_generation,
                        grid_wake_rx,
                        font_config,
                    )
                });
                let terminal_view = cx.new(|cx| TerminalView::new(window, cx, terminal));
                let config = app_config.clone();

                let app_view = cx.new(|cx| AppView::new(cx, terminal_view, config, None));

                let shell_clone = shell.clone();
                let config_for_spawn = app_config.clone();
                let app_view_clone = app_view.clone();
                let new_tab_cb: Option<
                    Box<dyn Fn(&mut Window, &mut gpui::Context<AppView>) + 'static>,
                > = Some(Box::new(move |window, cx| {
                    let spawned = spawn_pty(&shell_clone, config_for_spawn.scrollback);
                    let term = spawned.term;
                    let render_gen = spawned.render_generation;
                    let pty_writer = spawned.pty_writer;
                    let grid_rx = spawned.grid_wake_rx;
                    let font_cfg = config_for_spawn.font.clone();

                    let terminal: Entity<Terminal> = cx.new(|cx| {
                        Terminal::from_pre_spawned(
                            cx, pty_writer, term, render_gen, grid_rx, font_cfg,
                        )
                    });
                    let terminal_view: Entity<TerminalView> =
                        cx.new(|cx| TerminalView::new(window, cx, terminal));

                    app_view_clone.update(cx, |app, _| {
                        app.add_terminal_view(terminal_view);
                    });
                }));

                app_view.update(cx, |app, _| {
                    app.set_new_tab_cb(new_tab_cb);
                });

                app_view
            },
        )
        .expect("Failed to open window");
    });

    Ok(())
}
