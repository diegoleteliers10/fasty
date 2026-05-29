mod config;
mod event_listener;
mod pty;
mod renderer;
mod terminal_state;

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use config::Config;
use renderer::Renderer;
use terminal_state::TerminalState;
use tracing_subscriber::util::SubscriberInitExt;
use winit::{
    event::{ElementState, WindowEvent},
    event_loop::EventLoop,
    keyboard::Key,
};

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

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let config = Config::load()?;
    tracing::info!(
        "fasty: starting, shell={}",
        config.shell.as_deref().unwrap_or("default")
    );

    let shell = if let Some(ref s) = config.shell {
        s.clone()
    } else {
        get_login_shell()
    };

    let event_loop = EventLoop::new()?;
    let window = event_loop.create_window(winit::window::WindowAttributes::default()
        .with_title("fasty")
        .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 800.0)))?;

    let window_arc = Arc::new(window);
    let window_for_renderer = window_arc.as_ref();
    let renderer = pollster::block_on(Renderer::new(window_for_renderer))?;
    let cell_width = renderer.cell_width();
    let cell_height = renderer.cell_height();


    let viewport_width = renderer.config.width as f32;
    const PADDING_LEFT: f32 = 10.0;
    const PADDING_TOP: f32 = 10.0;

    let viewport_height = renderer.config.height as f32;
    let shell_cols = ((viewport_width - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0) as usize;
    let shell_rows = ((viewport_height - PADDING_TOP * 2.0) / cell_height).floor().max(1.0) as usize;
    let terminal_state = TerminalState::new(&shell, config.scrollback, config.font.clone(), cell_width, cell_height, shell_cols.max(80) as f32 * cell_width, shell_rows.max(24) as f32 * cell_height)?;
    let terminal_state = Arc::new(parking_lot::Mutex::new(terminal_state));
    let renderer = Arc::new(parking_lot::Mutex::new(renderer));
    let mut modifiers = winit::keyboard::ModifiersState::default();
    let render_generation = Arc::new(AtomicU64::new(0));
    let rg = Arc::clone(&render_generation);

    let window_for_redraw = window_arc.clone();

    event_loop.run(move |event, target| {
        match event {
            winit::event::Event::WindowEvent { window_id: _, event } => {
                match event {
                    WindowEvent::CloseRequested => {
                        target.exit();
                    }
                    WindowEvent::Resized(_size) => {
                        let physical_size = window_for_redraw.inner_size();
                        let cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                        let rows = (((physical_size.height as f32 - PADDING_TOP * 2.0) / cell_height).floor().max(1.0)) as usize;

                        let mut r = renderer.lock();
                        r.resize(physical_size.width, physical_size.height);
                        drop(r);
                        terminal_state.lock().resize(cols, rows);
                        renderer.lock().set_dirty(true);
                    }
                    WindowEvent::ModifiersChanged(modified) => {
                        modifiers = modified.state();
                    }
                    WindowEvent::KeyboardInput { event, .. } => {
                        let pressed = event.state == ElementState::Pressed;
                        if !pressed {
                            return;
                        }

                        let key_str = match &event.logical_key {
                            Key::Character(s) => s.to_string(),
                            Key::Named(n) => format!("{:?}", n),
                            _ => String::new(),
                        };

                        if modifiers.control_key() {
                            match key_str.as_str() {
                                "c" => { terminal_state.lock().write_to_pty(&[0x03]); return; }
                                "d" => { terminal_state.lock().write_to_pty(&[0x04]); return; }
                                "z" => { terminal_state.lock().write_to_pty(&[0x1A]); return; }
                                "l" => { terminal_state.lock().write_to_pty(&[0x0C]); return; }
                                _ => {}
                            }
                        }

                        let bytes = key_to_bytes(&event.logical_key, &modifiers);
                        if !bytes.is_empty() {
                            terminal_state.lock().write_to_pty(&bytes);
                        }
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        let lines = match delta {
                            winit::event::MouseScrollDelta::LineDelta(_, y) => y as isize,
                            winit::event::MouseScrollDelta::PixelDelta(_) => 0,
                        };
                        if lines != 0 {
                            terminal_state.lock().scroll(lines);
                            renderer.lock().set_dirty(true);
                        }
                    }
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => {
                window_for_redraw.request_redraw();
                let term = terminal_state.lock();
                term.update_render_generation(&rg);
                term.mark_dirty();
                renderer.lock().set_dirty(true);
                let term_ref: &TerminalState = &*term;
                renderer.lock().render(term_ref);
            }
            _ => {}
        }
    })?;

    Ok(())
}

fn key_to_bytes(key: &Key, modifiers: &winit::keyboard::ModifiersState) -> Vec<u8> {
    match key {
        Key::Character(s) if !s.is_empty() => s.as_bytes().to_vec(),
        Key::Named(n) => {
            let name = format!("{:?}", n);
            match name.as_str() {
                "Enter" => vec![b'\r'],
                "Space" => vec![b' '],
                "Backspace" => vec![0x7F],
                "Tab" => vec![0x09],
                "Escape" => vec![0x1B],
                "ArrowUp" => {
                    if modifiers.alt_key() {
                        vec![0x1B, 0x5B, 0x41]
                    } else {
                        vec![0x1B, 0x5B, 0x41]
                    }
                }
                "ArrowDown" => vec![0x1B, 0x5B, 0x42],
                "ArrowRight" => vec![0x1B, 0x5B, 0x43],
                "ArrowLeft" => vec![0x1B, 0x5B, 0x44],
                "Home" => vec![0x1B, 0x5B, 0x48],
                "End" => vec![0x1B, 0x5B, 0x46],
                "PageUp" => vec![0x1B, 0x5B, 0x35, 0x7E],
                "PageDown" => vec![0x1B, 0x5B, 0x36, 0x7E],
                "Insert" => vec![0x1B, 0x5B, 0x32, 0x7E],
                "Delete" => vec![0x1B, 0x5B, 0x33, 0x7E],
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}