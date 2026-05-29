mod config;
mod event_listener;
mod pty;
mod renderer;
mod terminal_state;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use config::Config;
use renderer::Renderer;
use terminal_state::TerminalState;
use tracing_subscriber::util::SubscriberInitExt;
use winit::{
    event::{ElementState, WindowEvent, MouseButton, MouseScrollDelta},
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
    let mut shell_rows = ((viewport_height - PADDING_TOP * 2.0) / cell_height).floor().max(1.0) as usize;
    let proxy = event_loop.create_proxy();
    let terminal_state = TerminalState::new(
        &shell,
        config.scrollback,
        config.font.clone(),
        cell_width,
        cell_height,
        shell_cols.max(80) as f32 * cell_width,
        shell_rows.max(24) as f32 * cell_height,
        proxy,
    )?;
    let terminal_state = Arc::new(parking_lot::Mutex::new(terminal_state));
    let renderer = Arc::new(parking_lot::Mutex::new(renderer));
    let mut modifiers = winit::keyboard::ModifiersState::default();
    let render_generation = Arc::new(AtomicU64::new(0));
    let rg = Arc::clone(&render_generation);

    let window_for_redraw = window_arc.clone();
    
    let mut scroll_target = 0.0f32;
    let mut scroll_current = 0.0f32;
    let mut is_dragging = false;
    let mut last_cursor_y = 0.0f64;
    let mut last_actual_offset = 0;
    let mut last_scroll_diff = 0;

    let mut scrollbar_alpha = 0.0f32;
    let mut is_dragging_scrollbar = false;
    let mut scrollbar_drag_offset_y = 0.0f32;
    let mut current_mouse_x = 0.0f64;
    let mut current_mouse_y = 0.0f64;

    event_loop.run(move |event, target| {
        match event {
            winit::event::Event::UserEvent(()) => {
                renderer.lock().set_dirty(true);
                window_for_redraw.request_redraw();
            }
            winit::event::Event::WindowEvent { window_id: _, event } => {
                match event {
                    WindowEvent::CloseRequested => {
                        target.exit();
                    }
                    WindowEvent::Resized(_size) => {
                        let physical_size = window_for_redraw.inner_size();
                        let cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                        let rows = (((physical_size.height as f32 - PADDING_TOP * 2.0) / cell_height).floor().max(1.0)) as usize;

                        shell_rows = rows;

                        let mut r = renderer.lock();
                        r.resize(physical_size.width, physical_size.height);
                        drop(r);
                        terminal_state.lock().resize(cols, rows);
                        renderer.lock().set_dirty(true);
                        window_for_redraw.request_redraw();
                    }
                    WindowEvent::RedrawRequested => {
                        let term = terminal_state.lock();
                        let max_history = term.history_size() as f32;
                        let term_ref: &TerminalState = &*term;
                        renderer.lock().render(
                            term_ref,
                            scrollbar_alpha,
                            scroll_current,
                            max_history,
                            shell_rows as f32,
                        );
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
                    WindowEvent::MouseInput { state, button, .. } => {
                        if button == MouseButton::Left {
                            let pressed = state == ElementState::Pressed;
                            if pressed {
                                let r = renderer.lock();
                                let v_width = r.config.width as f64;
                                let v_height = r.config.height as f32;
                                drop(r);

                                let is_hovering_scrollbar = current_mouse_x > (v_width - 20.0);
                                if is_hovering_scrollbar {
                                    let term = terminal_state.lock();
                                    let history_size = term.history_size() as f32;
                                    let visible_rows = shell_rows as f32;
                                    drop(term);

                                    let total_lines = visible_rows + history_size;
                                    if total_lines > 0.0 {
                                        let ratio = visible_rows / total_lines;
                                        let thumb_h = (v_height * ratio).max(30.0).min(v_height);

                                        let scroll_ratio = if history_size > 0.0 {
                                            scroll_current / history_size
                                        } else {
                                            0.0
                                        };

                                        let thumb_y = (1.0 - scroll_ratio) * (v_height - thumb_h);

                                        if current_mouse_y >= thumb_y as f64 && current_mouse_y <= (thumb_y + thumb_h) as f64 {
                                            is_dragging_scrollbar = true;
                                            scrollbar_drag_offset_y = (current_mouse_y - thumb_y as f64) as f32;
                                        } else {
                                            let track_center = v_height - thumb_h;
                                            let click_y = (current_mouse_y - thumb_h as f64 / 2.0).clamp(0.0, track_center as f64);
                                            let new_ratio = 1.0 - (click_y / track_center as f64) as f32;
                                            scroll_target = new_ratio * history_size;

                                            is_dragging_scrollbar = true;
                                            scrollbar_drag_offset_y = thumb_h / 2.0;
                                            window_for_redraw.request_redraw();
                                        }
                                    }
                                } else {
                                    is_dragging = true;
                                }
                            } else {
                                is_dragging = false;
                                is_dragging_scrollbar = false;
                            }
                        }
                    }
                    WindowEvent::CursorMoved { position, .. } => {
                        let delta_y = position.y - last_cursor_y;
                        last_cursor_y = position.y;
                        current_mouse_x = position.x;
                        current_mouse_y = position.y;

                        if is_dragging_scrollbar {
                            let r = renderer.lock();
                            let v_height = r.config.height as f32;
                            drop(r);

                            let term = terminal_state.lock();
                            let history_size = term.history_size() as f32;
                            let visible_rows = shell_rows as f32;
                            drop(term);

                            let total_lines = visible_rows + history_size;
                            if total_lines > 0.0 {
                                let ratio = visible_rows / total_lines;
                                let thumb_h = (v_height * ratio).max(30.0).min(v_height);
                                let track_center = v_height - thumb_h;

                                if track_center > 0.0 {
                                    let new_thumb_y = (current_mouse_y as f32 - scrollbar_drag_offset_y).clamp(0.0, track_center);
                                    let scroll_ratio = 1.0 - (new_thumb_y / track_center);
                                    scroll_target = scroll_ratio * history_size;
                                }
                            }
                            window_for_redraw.request_redraw();
                        } else if is_dragging {
                            // Drag directly on text to scroll
                            let drag_speed = 1.0f32;
                            let delta_lines = (delta_y as f32 / cell_height) * drag_speed;
                            let max_history = terminal_state.lock().history_size() as f32;
                            scroll_target = (scroll_target + delta_lines).clamp(0.0, max_history);
                            window_for_redraw.request_redraw();
                        }
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        let lines = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y,
                            MouseScrollDelta::PixelDelta(pos) => {
                                pos.y as f32 / cell_height
                            }
                        };

                        let max_history = terminal_state.lock().history_size() as f32;
                        let scroll_speed = 3.0f32;

                        let delta_scroll = match delta {
                            MouseScrollDelta::LineDelta(_, _) => lines * scroll_speed,
                            MouseScrollDelta::PixelDelta(_) => lines,
                        };

                        scroll_target = (scroll_target + delta_scroll).clamp(0.0, max_history);
                        window_for_redraw.request_redraw();
                    }
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => {
                let term = terminal_state.lock();

                // Sync scroll position if PTY printed output or terminal was resized
                let actual_offset = term.display_offset();
                let mut needs_redraw = false;
                if actual_offset != last_actual_offset {
                    let diff_offset = actual_offset as f32 - last_actual_offset as f32;
                    let pty_change = diff_offset - last_scroll_diff as f32;
                    if pty_change.abs() > 0.01 {
                        scroll_target += pty_change;
                        scroll_current += pty_change;
                        needs_redraw = true;
                    }
                }

                let max_history = term.history_size() as f32;
                scroll_target = scroll_target.clamp(0.0, max_history);

                let diff = scroll_target - scroll_current;
                let mut current_scroll_diff = 0;
                if diff.abs() > 0.01 {
                    scroll_current += diff * 0.15;

                    let target_offset = scroll_current.round() as isize;
                    let scroll_diff = target_offset - term.display_offset() as isize;
                    if scroll_diff != 0 {
                        term.scroll(scroll_diff);
                        current_scroll_diff = scroll_diff;
                    }
                    renderer.lock().set_dirty(true);
                    needs_redraw = true;
                } else {
                    scroll_current = scroll_target;
                }

                last_actual_offset = term.display_offset();
                last_scroll_diff = current_scroll_diff;

                // Opacity animation of the scrollbar
                let v_width = renderer.lock().config.width as f64;
                let is_hovering = current_mouse_x > (v_width - 20.0) || is_dragging_scrollbar;
                let target_alpha = if is_hovering { 1.0 } else { 0.0 };

                let alpha_diff = target_alpha - scrollbar_alpha;
                if alpha_diff.abs() > 0.01 {
                    scrollbar_alpha += alpha_diff * 0.15;
                    renderer.lock().set_dirty(true);
                    needs_redraw = true;
                } else {
                    scrollbar_alpha = target_alpha;
                }

                let last_rg = rg.load(Ordering::Relaxed);
                term.update_render_generation(&rg);
                let current_rg = rg.load(Ordering::Relaxed);
                if current_rg != last_rg {
                    renderer.lock().set_dirty(true);
                    needs_redraw = true;
                }

                if needs_redraw || renderer.lock().dirty {
                    window_for_redraw.request_redraw();
                }
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