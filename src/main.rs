mod config;
mod event_listener;
mod pty;
mod renderer;
mod terminal_state;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use config::Config;
use renderer::Renderer;
use renderer::Selection;
use terminal_state::TerminalState;
use tracing_subscriber::util::SubscriberInitExt;
use alacritty_terminal::grid::Dimensions;
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
        .with_decorations(false)
        .with_transparent(true)
        .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 800.0)))?;

    let window_arc = Arc::new(window);
    let window_for_renderer = window_arc.as_ref();
    let renderer = pollster::block_on(Renderer::new(window_for_renderer))?;
    let cell_width = renderer.cell_width();
    let cell_height = renderer.cell_height();


    let viewport_width = renderer.config.width as f32;
    const PADDING_LEFT: f32 = 10.0;
    const PADDING_TOP: f32 = 46.0;
    const PADDING_BOTTOM: f32 = 10.0;

    let viewport_height = renderer.config.height as f32;
    let shell_cols = ((viewport_width - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0) as usize;
    let mut shell_rows = ((viewport_height - (PADDING_TOP + PADDING_BOTTOM)) / cell_height).floor().max(1.0) as usize;
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
    let mut ctrl_held = false;
    let mut shift_held = false;
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
    let mut last_click_time: Option<std::time::Instant> = None;
    let mut last_activity_time = std::time::Instant::now();
    let mut selection: Option<Selection> = None;
    let mut is_selecting_text = false;
    let mut selection_start_pos: Option<(f64, f64)> = None;
    let mut toast: Option<(String, std::time::Instant)> = None;
    let start_time = std::time::Instant::now();
    let mut clipboard = arboard::Clipboard::new().ok();
    let mut hovered_url: Option<renderer::HoveredUrl> = None;

    // Hover states for main window topbar buttons
    let mut hover_close = false;
    let mut hover_max = false;
    let mut hover_min = false;
    let mut hover_settings = false;

    // Secondary settings window state
    let mut settings_window: Option<Arc<winit::window::Window>> = None;
    let mut settings_renderer: Option<Renderer<'static>> = None;
    let mut settings_family = String::new();
    let mut settings_size = 14.0f32;
    let mut settings_scrollback = 10000usize;
    let mut settings_active_field = 0usize; // 0 = none, 1 = font family
    
    let mut s_hover_close = false;
    let mut s_hover_family = false;
    let mut s_hover_size_minus = false;
    let mut s_hover_size_plus = false;
    let mut s_hover_scroll_minus = false;
    let mut s_hover_scroll_plus = false;
    let mut s_hover_save = false;
    let mut s_hover_cancel = false;
    

    event_loop.run(move |event, target| {
        match event {
            winit::event::Event::UserEvent(()) => {
                renderer.lock().set_dirty(true);
                window_for_redraw.request_redraw();
            }
            winit::event::Event::WindowEvent { window_id, event } => {
                if window_id == window_for_redraw.id() {
                    // --- Main Window Event Handler ---
                    match event {
                        WindowEvent::CloseRequested => {
                            target.exit();
                        }
                        WindowEvent::Resized(_size) => {
                            let physical_size = window_for_redraw.inner_size();
                            let cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                            let rows = (((physical_size.height as f32 - (PADDING_TOP + PADDING_BOTTOM)) / cell_height).floor().max(1.0)) as usize;

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

                            let last_activity_time_secs = last_activity_time.saturating_duration_since(start_time).as_secs_f32();
                            let current_time = start_time.elapsed().as_secs_f32();

                            renderer.lock().render(
                                term_ref,
                                scrollbar_alpha,
                                scroll_current,
                                max_history,
                                shell_rows as f32,
                                hover_close,
                                hover_max,
                                hover_min,
                                hover_settings,
                                last_activity_time_secs,
                                current_time,
                                selection,
                                hovered_url,
                                toast.as_ref().map(|(msg, t)| (msg.as_str(), *t)),
                            );
                        }
                        WindowEvent::ModifiersChanged(modified) => {
                            modifiers = modified.state();
                            let new_hover = detect_hovered_url(
                                current_mouse_x,
                                current_mouse_y,
                                modifiers.control_key() || ctrl_held,
                                &terminal_state,
                                scroll_current,
                                cell_width,
                                cell_height,
                                shell_cols,
                                shell_rows,
                            );
                            if hovered_url != new_hover {
                                hovered_url = new_hover;
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. } => {
                            last_activity_time = std::time::Instant::now();
                            let pressed = event.state == ElementState::Pressed;
                            
                            // Track Ctrl and Shift modifiers manually in case ModifiersChanged is missed
                            match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ControlLeft) |
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ControlRight) => {
                                    ctrl_held = pressed;
                                }
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ShiftLeft) |
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ShiftRight) => {
                                    shift_held = pressed;
                                }
                                _ => {}
                            }

                            // Re-evaluate hovered URL if modifier keys changed
                            let ctrl_active = modifiers.control_key() || ctrl_held;
                            let shift_active = modifiers.shift_key() || shift_held;

                            let new_hover = detect_hovered_url(
                                current_mouse_x,
                                current_mouse_y,
                                ctrl_active,
                                &terminal_state,
                                scroll_current,
                                cell_width,
                                cell_height,
                                shell_cols,
                                shell_rows,
                            );
                            if hovered_url != new_hover {
                                hovered_url = new_hover;
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }

                            if !pressed {
                                return;
                            }

                            let key_str = match &event.logical_key {
                                Key::Character(s) => s.to_string(),
                                Key::Named(n) => format!("{:?}", n),
                                _ => String::new(),
                            };

                            if ctrl_active {
                                 let is_v_key = match event.physical_key {
                                     winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyV) => true,
                                     _ => key_str.eq_ignore_ascii_case("v") || key_str == "\u{16}"
                                 };

                                 if shift_active && is_v_key {
                                     tracing::info!("Ctrl+Shift+V paste shortcut detected!");
                                     let mut ctx_opt = if clipboard.is_none() {
                                         match arboard::Clipboard::new() {
                                             Ok(ctx) => {
                                                 clipboard = Some(ctx);
                                                 clipboard.as_mut()
                                             }
                                             Err(e) => {
                                                 eprintln!("fasty clipboard initialization failed: {:?}", e);
                                                 None
                                             }
                                         }
                                     } else {
                                         clipboard.as_mut()
                                     };

                                     if let Some(ref mut ctx) = ctx_opt {
                                         match ctx.get_text() {
                                             Ok(text) => {
                                                 tracing::info!("Pasting {} characters from clipboard", text.len());
                                                 if !text.is_empty() {
                                                     let term = terminal_state.lock();
                                                     let term_guard = term.term().lock();
                                                     let mode = term_guard.mode();
                                                     let bracketed = mode.contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
                                                     drop(term_guard);
                                                     drop(term);

                                                     let mut paste_bytes = Vec::new();
                                                     if bracketed {
                                                         paste_bytes.extend_from_slice(b"\x1b[200~");
                                                         paste_bytes.extend_from_slice(text.as_bytes());
                                                         paste_bytes.extend_from_slice(b"\x1b[201~");
                                                     } else {
                                                         paste_bytes.extend_from_slice(text.as_bytes());
                                                     }
                                                     terminal_state.lock().write_to_pty(&paste_bytes);
                                                 }
                                             }
                                             Err(e) => {
                                                 eprintln!("fasty clipboard get_text failed: {:?}", e);
                                             }
                                         }
                                     } else {
                                         eprintln!("fasty clipboard not available");
                                     }
                                     return;
                                 }

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
                            last_activity_time = std::time::Instant::now();
                            if button == MouseButton::Left {
                                let pressed = state == ElementState::Pressed;
                                if pressed {
                                    let r = renderer.lock();
                                    let v_width = r.config.width as f64;
                                    let v_height = r.config.height as f32;
                                    drop(r);

                                    // Check topbar buttons
                                    let is_hovering_close = current_mouse_y >= 4.0 && current_mouse_y <= 32.0 && current_mouse_x >= (v_width - 32.0) && current_mouse_x < (v_width - 4.0);
                                    let is_hovering_max = current_mouse_y >= 4.0 && current_mouse_y <= 32.0 && current_mouse_x >= (v_width - 64.0) && current_mouse_x < (v_width - 36.0);
                                    let is_hovering_min = current_mouse_y >= 4.0 && current_mouse_y <= 32.0 && current_mouse_x >= (v_width - 96.0) && current_mouse_x < (v_width - 68.0);
                                    let is_hovering_settings = current_mouse_y >= 4.0 && current_mouse_y <= 32.0 && current_mouse_x >= (v_width - 128.0) && current_mouse_x < (v_width - 100.0);

                                    if is_hovering_close {
                                        target.exit();
                                        return;
                                    } else if is_hovering_max {
                                        let is_max = window_for_redraw.is_maximized();
                                        window_for_redraw.set_maximized(!is_max);
                                        return;
                                    } else if is_hovering_min {
                                        window_for_redraw.set_minimized(true);
                                        return;
                                    } else if is_hovering_settings {
                                        if settings_window.is_none() {
                                            if let Ok(cfg) = Config::load() {
                                                settings_family = cfg.font.family.clone();
                                                settings_size = cfg.font.size;
                                                settings_scrollback = cfg.scrollback;
                                            } else {
                                                settings_family = "JetBrains Mono".to_string();
                                                settings_size = 14.0;
                                                settings_scrollback = 10000;
                                            }
                                            settings_active_field = 0;
                                            if let Ok(sw) = target.create_window(winit::window::WindowAttributes::default()
                                                .with_title("fasty Settings")
                                                .with_decorations(false)
                                                .with_transparent(true)
                                                .with_inner_size(winit::dpi::LogicalSize::new(400.0, 300.0))) {
                                                let sw_arc = Arc::new(sw);
                                                let sw_ref: &winit::window::Window = &*sw_arc;
                                                let sw_static: &'static winit::window::Window = unsafe { std::mem::transmute(sw_ref) };
                                                if let Ok(sr) = pollster::block_on(Renderer::new(sw_static)) {
                                                    settings_renderer = Some(sr);
                                                }
                                                settings_window = Some(sw_arc);
                                            }
                                        } else {
                                            settings_window = None;
                                            settings_renderer = None;
                                        }
                                        window_for_redraw.request_redraw();
                                        return;
                                    } else if current_mouse_y <= 36.0 {
                                        // Don't drag the window if clicking near the control buttons region
                                        if current_mouse_x < (v_width - 132.0) {
                                            let now = std::time::Instant::now();
                                            let is_double_click = if let Some(last_time) = last_click_time {
                                                now.duration_since(last_time) < std::time::Duration::from_millis(300)
                                            } else {
                                                false
                                            };
                                            last_click_time = Some(now);

                                            if is_double_click {
                                                let is_max = window_for_redraw.is_maximized();
                                                window_for_redraw.set_maximized(!is_max);
                                            } else {
                                                let _ = window_for_redraw.drag_window();
                                            }
                                        }
                                        return;
                                    }

                                    let is_hovering_scrollbar = current_mouse_y > 36.0 && current_mouse_x > (v_width - 20.0);
                                    if is_hovering_scrollbar {
                                        let term = terminal_state.lock();
                                        let history_size = term.history_size() as f32;
                                        let visible_rows = shell_rows as f32;
                                        drop(term);

                                        let total_lines = visible_rows + history_size;
                                        if total_lines > 0.0 {
                                            let ratio = visible_rows / total_lines;
                                            let track_h = v_height - 40.0;
                                            let thumb_h = (track_h * ratio).max(20.0).min(track_h);

                                            let scroll_ratio = if history_size > 0.0 {
                                                scroll_current / history_size
                                            } else {
                                                0.0
                                            };

                                            let thumb_y = 36.0 + (1.0 - scroll_ratio) * (track_h - thumb_h);

                                            if current_mouse_y >= thumb_y as f64 && current_mouse_y <= (thumb_y + thumb_h) as f64 {
                                                is_dragging_scrollbar = true;
                                                scrollbar_drag_offset_y = (current_mouse_y - thumb_y as f64) as f32;
                                            } else {
                                                let track_center = track_h - thumb_h;
                                                let click_y = (current_mouse_y - 36.0 - thumb_h as f64 / 2.0).clamp(0.0, track_center as f64);
                                                let new_ratio = 1.0 - (click_y / track_center as f64) as f32;
                                                scroll_target = new_ratio * history_size;

                                                is_dragging_scrollbar = true;
                                                scrollbar_drag_offset_y = thumb_h / 2.0;
                                                window_for_redraw.request_redraw();
                                            }
                                        }
                                        if modifiers.control_key() || ctrl_held {
                                            // Ctrl + Click: Try to detect and open URL
                                            let term = terminal_state.lock();
                                            let scroll_fraction = scroll_current - term.display_offset() as f32;
                                            let display_offset = term.display_offset();
                                            let history_size = term.history_size();
                                            
                                            let click_point = mouse_to_grid_point(
                                                current_mouse_x,
                                                current_mouse_y,
                                                cell_width,
                                                cell_height,
                                                scroll_fraction,
                                                display_offset,
                                                shell_cols,
                                                shell_rows,
                                            );
                                            
                                            if click_point.line.0 >= -(history_size as i32) && click_point.line.0 < shell_rows as i32 {
                                                let term_guard = term.term().lock();
                                                let grid = term_guard.grid();
                                                let row = &grid[alacritty_terminal::index::Line(click_point.line.0)];
                                                let mut chars = Vec::new();
                                                for col_idx in 0..shell_cols {
                                                    let cell = &row[alacritty_terminal::index::Column(col_idx)];
                                                    chars.push(cell.c);
                                                }
                                                
                                                let col = click_point.column.0;
                                                if col < chars.len() {
                                                    let mut start = col;
                                                    while start > 0 {
                                                        let c = chars[start - 1];
                                                        if c == ' ' || c == '\0' || c == '"' || c == '\'' || c == '`' || c == '<' || c == '>' || c == '[' || c == ']' || c == '(' || c == ')' || c == '{' || c == '}' {
                                                            break;
                                                        }
                                                        start -= 1;
                                                    }
                                                    
                                                    let mut end = col;
                                                    while end < chars.len() {
                                                        let c = chars[end];
                                                        if c == ' ' || c == '\0' || c == '"' || c == '\'' || c == '`' || c == '<' || c == '>' || c == '[' || c == ']' || c == '(' || c == ')' || c == '{' || c == '}' {
                                                            break;
                                                        }
                                                        end += 1;
                                                    }
                                                    
                                                    if start < end {
                                                        let word: String = chars[start..end].iter().collect();
                                                        let mut trimmed = word.trim();
                                                        while trimmed.ends_with('.') || trimmed.ends_with(',') || trimmed.ends_with(';') || trimmed.ends_with(':') || trimmed.ends_with('?') || trimmed.ends_with('!') || trimmed.ends_with(')') || trimmed.ends_with(']') || trimmed.ends_with('}') {
                                                            trimmed = &trimmed[..trimmed.len() - 1];
                                                        }
                                                        if is_url(trimmed) {
                                                            open_url(trimmed);
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            selection_start_pos = Some((current_mouse_x, current_mouse_y));
                                            is_selecting_text = false;
                                        }
                                    } else {
                                        is_dragging = true;
                                    }
                                } else {
                                    is_dragging = false;
                                    is_dragging_scrollbar = false;
                                     if is_selecting_text {
                                         is_selecting_text = false;
                                         if let Some(sel) = selection {
                                             copy_selection_to_clipboard(&terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
                                             toast = Some((
                                                 "✓  Text copied".to_string(),
                                                 std::time::Instant::now(),
                                             ));
                                            renderer.lock().set_dirty(true);
                                            window_for_redraw.request_redraw();
                                        }
                                    } else if selection_start_pos.is_some() {
                                        // Simple click release (no drag occurred): clear selection
                                        selection = None;
                                        renderer.lock().set_dirty(true);
                                        window_for_redraw.request_redraw();
                                    }
                                    selection_start_pos = None;
                                }
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            let delta_y = position.y - last_cursor_y;
                            last_cursor_y = position.y;
                            current_mouse_x = position.x;
                            current_mouse_y = position.y;

                            let r = renderer.lock();
                            let v_width = r.config.width as f64;
                            let v_height = r.config.height as f32;
                            drop(r);

                            // Update mouse cursor icon (I-beam in terminal content area, Arrow/Default elsewhere)
                            let is_in_terminal_area = current_mouse_y > 36.0 && current_mouse_x <= (v_width - 20.0);
                            if is_in_terminal_area {
                                window_for_redraw.set_cursor(winit::window::CursorIcon::Text);
                            } else {
                                window_for_redraw.set_cursor(winit::window::CursorIcon::Default);
                            }

                            let old_hover_close = hover_close;
                            let old_hover_max = hover_max;
                            let old_hover_min = hover_min;
                            let old_hover_settings = hover_settings;

                            hover_close = current_mouse_y >= 4.0 && current_mouse_y <= 32.0 && current_mouse_x >= (v_width - 32.0) && current_mouse_x < (v_width - 4.0);
                            hover_max = current_mouse_y >= 4.0 && current_mouse_y <= 32.0 && current_mouse_x >= (v_width - 64.0) && current_mouse_x < (v_width - 36.0);
                            hover_min = current_mouse_y >= 4.0 && current_mouse_y <= 32.0 && current_mouse_x >= (v_width - 96.0) && current_mouse_x < (v_width - 68.0);
                            hover_settings = current_mouse_y >= 4.0 && current_mouse_y <= 32.0 && current_mouse_x >= (v_width - 128.0) && current_mouse_x < (v_width - 100.0);

                            if hover_close != old_hover_close || hover_max != old_hover_max || hover_min != old_hover_min || hover_settings != old_hover_settings {
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }

                            if is_dragging_scrollbar {
                                let term = terminal_state.lock();
                                let history_size = term.history_size() as f32;
                                let visible_rows = shell_rows as f32;
                                drop(term);

                                let total_lines = visible_rows + history_size;
                                if total_lines > 0.0 {
                                    let ratio = visible_rows / total_lines;
                                    let track_h = v_height - 40.0;
                                    let thumb_h = (track_h * ratio).max(20.0).min(track_h);
                                    let track_center = track_h - thumb_h;

                                    if track_center > 0.0 {
                                        let new_thumb_y = (current_mouse_y as f32 - 36.0 - scrollbar_drag_offset_y).clamp(0.0, track_center);
                                        let scroll_ratio = 1.0 - (new_thumb_y / track_center);
                                        scroll_target = scroll_ratio * history_size;
                                    }
                                }
                                window_for_redraw.request_redraw();
                            } else if let Some((sx, sy)) = selection_start_pos {
                                if !is_selecting_text {
                                    let dist = ((current_mouse_x - sx).powi(2) + (current_mouse_y - sy).powi(2)).sqrt();
                                    if dist > 5.0 {
                                        is_selecting_text = true;
                                        let term = terminal_state.lock();
                                        let scroll_fraction = scroll_current - term.display_offset() as f32;
                                        let display_offset = term.display_offset();
                                        drop(term);

                                        let start_point = mouse_to_grid_point(
                                            sx,
                                            sy,
                                            cell_width,
                                            cell_height,
                                            scroll_fraction,
                                            display_offset,
                                            shell_cols,
                                            shell_rows,
                                        );
                                        let current_point = mouse_to_grid_point(
                                            current_mouse_x,
                                            current_mouse_y,
                                            cell_width,
                                            cell_height,
                                            scroll_fraction,
                                            display_offset,
                                            shell_cols,
                                            shell_rows,
                                        );
                                        selection = Some(Selection { start: start_point, end: current_point });
                                        renderer.lock().set_dirty(true);
                                        window_for_redraw.request_redraw();
                                    }
                                } else {
                                    let term = terminal_state.lock();
                                    let scroll_fraction = scroll_current - term.display_offset() as f32;
                                    let display_offset = term.display_offset();
                                    drop(term);

                                    let current_point = mouse_to_grid_point(
                                        current_mouse_x,
                                        current_mouse_y,
                                        cell_width,
                                        cell_height,
                                        scroll_fraction,
                                        display_offset,
                                        shell_cols,
                                        shell_rows,
                                    );
                                    if let Some(ref mut sel) = selection {
                                        if sel.end != current_point {
                                            sel.end = current_point;
                                            renderer.lock().set_dirty(true);
                                            window_for_redraw.request_redraw();
                                        }
                                    }
                                }
                            } else if is_dragging {
                                let drag_speed = 1.0f32;
                                let delta_lines = (delta_y as f32 / cell_height) * drag_speed;
                                let max_history = terminal_state.lock().history_size() as f32;
                                scroll_target = (scroll_target + delta_lines).clamp(0.0, max_history);
                                window_for_redraw.request_redraw();
                            }

                            let new_hover = detect_hovered_url(
                                current_mouse_x,
                                current_mouse_y,
                                modifiers.control_key() || ctrl_held,
                                &terminal_state,
                                scroll_current,
                                cell_width,
                                cell_height,
                                shell_cols,
                                shell_rows,
                            );
                            if hovered_url != new_hover {
                                hovered_url = new_hover;
                                renderer.lock().set_dirty(true);
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
                        WindowEvent::CursorLeft { .. } => {
                            window_for_redraw.set_cursor(winit::window::CursorIcon::Default);

                            let old_hover_close = hover_close;
                            let old_hover_max = hover_max;
                            let old_hover_min = hover_min;
                            let old_hover_settings = hover_settings;

                            hover_close = false;
                            hover_max = false;
                            hover_min = false;
                            hover_settings = false;

                            let url_changed = hovered_url.is_some();
                            hovered_url = None;

                            if old_hover_close || old_hover_max || old_hover_min || old_hover_settings || url_changed {
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }
                        }
                        WindowEvent::Focused(focused) => {
                            if !focused {
                                let old_hover_close = hover_close;
                                let old_hover_max = hover_max;
                                let old_hover_min = hover_min;
                                let old_hover_settings = hover_settings;

                                hover_close = false;
                                hover_max = false;
                                hover_min = false;
                                hover_settings = false;
                                is_dragging = false;
                                is_dragging_scrollbar = false;

                                if old_hover_close || old_hover_max || old_hover_min || old_hover_settings {
                                    renderer.lock().set_dirty(true);
                                    window_for_redraw.request_redraw();
                                }
                            }
                        }
                        _ => {}
                    }
                } else if let Some(ref sw) = settings_window {
                    if window_id == sw.id() {
                        // --- Settings Window Event Handler ---
                        match event {
                            WindowEvent::CloseRequested => {
                                settings_window = None;
                                settings_renderer = None;
                                window_for_redraw.request_redraw();
                            }
                            WindowEvent::Resized(size) => {
                                if let Some(ref mut r) = settings_renderer {
                                    r.resize(size.width, size.height);
                                }
                            }
                            WindowEvent::RedrawRequested => {
                                if let Some(ref mut r) = settings_renderer {
                                    r.render_settings(
                                        &settings_family,
                                        settings_size,
                                        settings_scrollback,
                                        settings_active_field,
                                        s_hover_close,
                                        s_hover_family,
                                        s_hover_size_minus,
                                        s_hover_size_plus,
                                        s_hover_scroll_minus,
                                        s_hover_scroll_plus,
                                        s_hover_save,
                                        s_hover_cancel,
                                    );
                                }
                            }
                            WindowEvent::CursorMoved { position, .. } => {
                                let s_mouse_x = position.x;
                                let s_mouse_y = position.y;

                                let old_hover_close = s_hover_close;
                                let old_hover_family = s_hover_family;
                                let old_hover_size_minus = s_hover_size_minus;
                                let old_hover_size_plus = s_hover_size_plus;
                                let old_hover_scroll_minus = s_hover_scroll_minus;
                                let old_hover_scroll_plus = s_hover_scroll_plus;
                                let old_hover_save = s_hover_save;
                                let old_hover_cancel = s_hover_cancel;

                                s_hover_close = s_mouse_y >= 4.0 && s_mouse_y <= 32.0 && s_mouse_x >= (400.0 - 32.0) && s_mouse_x < (400.0 - 4.0);
                                s_hover_family = s_mouse_y >= 52.0 && s_mouse_y <= 78.0 && s_mouse_x >= 140.0 && s_mouse_x < 380.0;

                                s_hover_size_minus = s_mouse_y >= 92.0 && s_mouse_y <= 118.0 && s_mouse_x >= 140.0 && s_mouse_x < 168.0;
                                s_hover_size_plus = s_mouse_y >= 92.0 && s_mouse_y <= 118.0 && s_mouse_x >= 220.0 && s_mouse_x < 248.0;

                                s_hover_scroll_minus = s_mouse_y >= 132.0 && s_mouse_y <= 158.0 && s_mouse_x >= 140.0 && s_mouse_x < 168.0;
                                s_hover_scroll_plus = s_mouse_y >= 132.0 && s_mouse_y <= 158.0 && s_mouse_x >= 240.0 && s_mouse_x < 268.0;

                                s_hover_save = s_mouse_y >= 220.0 && s_mouse_y <= 252.0 && s_mouse_x >= 90.0 && s_mouse_x < 190.0;
                                s_hover_cancel = s_mouse_y >= 220.0 && s_mouse_y <= 252.0 && s_mouse_x >= 210.0 && s_mouse_x < 310.0;

                                let any_changed = s_hover_close != old_hover_close
                                    || s_hover_family != old_hover_family
                                    || s_hover_size_minus != old_hover_size_minus
                                    || s_hover_size_plus != old_hover_size_plus
                                    || s_hover_scroll_minus != old_hover_scroll_minus
                                    || s_hover_scroll_plus != old_hover_scroll_plus
                                    || s_hover_save != old_hover_save
                                    || s_hover_cancel != old_hover_cancel;

                                if any_changed {
                                    if let Some(ref mut r) = settings_renderer {
                                        r.set_dirty(true);
                                    }
                                    sw.request_redraw();
                                }
                            }
                            WindowEvent::MouseInput { state, button, .. } => {
                                if button == MouseButton::Left && state == ElementState::Pressed {
                                    if s_hover_close || s_hover_cancel {
                                        settings_window = None;
                                        settings_renderer = None;
                                        window_for_redraw.request_redraw();
                                    } else if s_hover_family {
                                        settings_active_field = 1;
                                    } else if s_hover_size_minus {
                                        settings_size = (settings_size - 0.5).max(6.0);
                                    } else if s_hover_size_plus {
                                        settings_size = (settings_size + 0.5).min(72.0);
                                    } else if s_hover_scroll_minus {
                                        settings_scrollback = settings_scrollback.saturating_sub(1000).max(1000);
                                    } else if s_hover_scroll_plus {
                                        settings_scrollback = settings_scrollback.saturating_add(1000).min(1000000);
                                    } else if s_hover_save {
                                        let mut current_config = Config::load().unwrap_or_default();
                                        current_config.font.family = settings_family.clone();
                                        current_config.font.size = settings_size;
                                        current_config.scrollback = settings_scrollback;
                                        let _ = current_config.save(&Config::config_path());

                                        settings_window = None;
                                        settings_renderer = None;

                                        // Try to reload dynamic variables if possible
                                        tracing::info!("fasty: saved and applied settings to config.json");
                                        
                                        window_for_redraw.request_redraw();
                                    } else {
                                        settings_active_field = 0;
                                    }
                                    if let Some(ref mut r) = settings_renderer {
                                        r.set_dirty(true);
                                    }
                                    if let Some(ref w) = settings_window {
                                        w.request_redraw();
                                    }
                                }
                            }
                            WindowEvent::KeyboardInput { event, .. } => {
                                if event.state == ElementState::Pressed && settings_active_field == 1 {
                                    match &event.logical_key {
                                        Key::Character(s) => {
                                            settings_family.push_str(s);
                                        }
                                        Key::Named(winit::keyboard::NamedKey::Backspace) => {
                                            settings_family.pop();
                                        }
                                        Key::Named(winit::keyboard::NamedKey::Enter) | Key::Named(winit::keyboard::NamedKey::Escape) => {
                                            settings_active_field = 0;
                                        }
                                        _ => {}
                                    }
                                    if let Some(ref mut r) = settings_renderer {
                                        r.set_dirty(true);
                                    }
                                    sw.request_redraw();
                                }
                            }
                            WindowEvent::CursorLeft { .. } => {
                                s_hover_close = false;
                                s_hover_family = false;
                                s_hover_size_minus = false;
                                s_hover_size_plus = false;
                                s_hover_scroll_minus = false;
                                s_hover_scroll_plus = false;
                                s_hover_save = false;
                                s_hover_cancel = false;
                                if let Some(ref mut r) = settings_renderer {
                                    r.set_dirty(true);
                                }
                                sw.request_redraw();
                            }
                            WindowEvent::Focused(focused) => {
                                if !focused {
                                    s_hover_close = false;
                                    s_hover_family = false;
                                    s_hover_size_minus = false;
                                    s_hover_size_plus = false;
                                    s_hover_scroll_minus = false;
                                    s_hover_scroll_plus = false;
                                    s_hover_save = false;
                                    s_hover_cancel = false;
                                    if let Some(ref mut r) = settings_renderer {
                                        r.set_dirty(true);
                                    }
                                    sw.request_redraw();
                                }
                            }
                            _ => {}
                        }
                    }
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
                let is_hovering = (current_mouse_y > 36.0 && current_mouse_x > (v_width - 20.0)) || is_dragging_scrollbar;
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
                    last_activity_time = std::time::Instant::now();
                    renderer.lock().set_dirty(true);
                    needs_redraw = true;
                }

                // If cursor is idle (quieto), force redraw to animate its pulsing
                let cursor_is_active = std::time::Instant::now().duration_since(last_activity_time) < std::time::Duration::from_millis(500);
                if !cursor_is_active {
                    renderer.lock().set_dirty(true);
                    needs_redraw = true;
                }

                // Toast auto-clear and redraw management
                if let Some((_, start_time)) = toast {
                    if start_time.elapsed() < std::time::Duration::from_millis(1920) {
                        renderer.lock().set_dirty(true);
                        needs_redraw = true;
                    } else {
                        toast = None;
                        renderer.lock().set_dirty(true);
                        needs_redraw = true;
                    }
                }

                if needs_redraw || renderer.lock().dirty {
                    window_for_redraw.request_redraw();
                }

                // Control flow adjustment: sleep until the cursor should start pulsing, or poll if animating, or wait for next event.
                if needs_redraw {
                    target.set_control_flow(winit::event_loop::ControlFlow::Poll);
                } else if cursor_is_active {
                    let wake_time = last_activity_time + std::time::Duration::from_millis(500);
                    target.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(wake_time));
                } else {
                    target.set_control_flow(winit::event_loop::ControlFlow::Wait);
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

fn copy_selection_to_clipboard(
    terminal_state: &Arc<parking_lot::Mutex<TerminalState>>,
    selection: Selection,
    shell_cols: usize,
    shell_rows: usize,
    clipboard: &mut Option<arboard::Clipboard>,
) {
    let term = terminal_state.lock();
    let term_guard = term.term().lock();
    let grid = term_guard.grid();
    let history_size = term_guard.history_size();
    
    let (min_p, max_p) = if selection.start <= selection.end {
        (selection.start, selection.end)
    } else {
        (selection.end, selection.start)
    };

    let mut text = String::new();
    
    for line_idx in min_p.line.0..=max_p.line.0 {
        let mut line_str = String::new();
        let start_col = if line_idx == min_p.line.0 { min_p.column.0 } else { 0 };
        let end_col = if line_idx == max_p.line.0 { max_p.column.0 } else { shell_cols - 1 };
        
        if line_idx >= -(history_size as i32) && line_idx < shell_rows as i32 {
            let row = &grid[alacritty_terminal::index::Line(line_idx)];
            for col_idx in start_col..=end_col {
                let cell = &row[alacritty_terminal::index::Column(col_idx)];
                if cell.c != '\0' {
                    line_str.push(cell.c);
                }
            }
        }
        
        let trimmed = line_str.trim_end();
        text.push_str(trimmed);
        if line_idx < max_p.line.0 {
            text.push('\n');
        }
    }

    tracing::info!("Extracted selected text ({} chars): {:?}", text.len(), text);

    if !text.is_empty() {
        let ctx_opt = if clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(ctx) => {
                    *clipboard = Some(ctx);
                    clipboard.as_mut()
                }
                Err(e) => {
                    eprintln!("fasty clipboard copy initialization failed: {:?}", e);
                    None
                }
            }
        } else {
            clipboard.as_mut()
        };
        if let Some(ctx) = ctx_opt {
            if let Err(e) = ctx.set_text(text) {
                eprintln!("fasty clipboard copy set_text failed: {:?}", e);
            } else {
                tracing::info!("Successfully copied selection to clipboard!");
            }
        } else {
            eprintln!("fasty clipboard copy not available");
        }
    }
}

fn mouse_to_grid_point(
    mouse_x: f64,
    mouse_y: f64,
    cell_width: f32,
    cell_height: f32,
    scroll_fraction: f32,
    display_offset: usize,
    shell_cols: usize,
    shell_rows: usize,
) -> alacritty_terminal::index::Point {
    use alacritty_terminal::index::{Point, Line, Column};
    
    const PADDING_LEFT: f32 = 10.0;
    const PADDING_TOP: f32 = 46.0;

    let col = (((mouse_x as f32 - PADDING_LEFT) / cell_width).floor() as i32)
        .clamp(0, shell_cols as i32 - 1) as usize;
    let row = ((mouse_y as f32 - PADDING_TOP) / cell_height - scroll_fraction).floor() as i32;
    
    let clamped_row = row.clamp(0, shell_rows as i32 - 1);
    let line = clamped_row - display_offset as i32;
    
    Point::new(Line(line), Column(col))
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://") || s.starts_with("mailto:") || s.starts_with("www.")
}

fn open_url(url: &str) {
    let target_url = if url.starts_with("www.") {
        format!("https://{}", url)
    } else {
        url.to_string()
    };

    tracing::info!("Opening URL: {}", target_url);

    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "start";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";

    #[cfg(target_os = "windows")]
    let res = std::process::Command::new("cmd")
        .args(&["/C", "start", "", &target_url])
        .spawn();

    #[cfg(not(target_os = "windows"))]
    let res = std::process::Command::new(cmd)
        .arg(&target_url)
        .spawn();

    match res {
        Ok(_) => tracing::info!("Successfully opened URL: {}", target_url),
        Err(e) => tracing::error!("Failed to open URL: {:?}", e),
    }
}

fn detect_hovered_url(
    current_mouse_x: f64,
    current_mouse_y: f64,
    ctrl_pressed: bool,
    terminal_state: &Arc<parking_lot::Mutex<TerminalState>>,
    scroll_current: f32,
    cell_width: f32,
    cell_height: f32,
    shell_cols: usize,
    shell_rows: usize,
) -> Option<renderer::HoveredUrl> {
    if current_mouse_y <= 36.0 || !ctrl_pressed {
        return None;
    }

    let term = terminal_state.lock();
    let scroll_fraction = scroll_current - term.display_offset() as f32;
    let display_offset = term.display_offset();
    let history_size = term.history_size();
    
    let hover_point = mouse_to_grid_point(
        current_mouse_x,
        current_mouse_y,
        cell_width,
        cell_height,
        scroll_fraction,
        display_offset,
        shell_cols,
        shell_rows,
    );
    
    if hover_point.line.0 >= -(history_size as i32) && hover_point.line.0 < shell_rows as i32 {
        let term_guard = term.term().lock();
        let grid = term_guard.grid();
        let row = &grid[alacritty_terminal::index::Line(hover_point.line.0)];
        let mut chars = Vec::with_capacity(shell_cols);
        for col_idx in 0..shell_cols {
            let cell = &row[alacritty_terminal::index::Column(col_idx)];
            chars.push(cell.c);
        }
        
        let col = hover_point.column.0;
        if col < chars.len() {
            let mut start = col;
            while start > 0 {
                let c = chars[start - 1];
                if c == ' ' || c == '\0' || c == '"' || c == '\'' || c == '`' || c == '<' || c == '>' || c == '[' || c == ']' || c == '(' || c == ')' || c == '{' || c == '}' {
                    break;
                }
                start -= 1;
            }
            
            let mut end = col;
            while end < chars.len() {
                let c = chars[end];
                if c == ' ' || c == '\0' || c == '"' || c == '\'' || c == '`' || c == '<' || c == '>' || c == '[' || c == ']' || c == '(' || c == ')' || c == '{' || c == '}' {
                    break;
                }
                end += 1;
            }
            
            if start < end {
                let word: String = chars[start..end].iter().collect();
                let leading_spaces = word.len() - word.trim_start().len();
                let trailing_spaces = word.len() - word.trim_end().len();
                
                let trimmed_start = start + leading_spaces;
                let mut trimmed_end = end - trailing_spaces;
                
                if trimmed_start < trimmed_end {
                    let mut trimmed_str = chars[trimmed_start..trimmed_end].iter().collect::<String>();
                    while (trimmed_str.ends_with('.') || trimmed_str.ends_with(',') || trimmed_str.ends_with(';') || trimmed_str.ends_with(':') || trimmed_str.ends_with('?') || trimmed_str.ends_with('!') || trimmed_str.ends_with(')') || trimmed_str.ends_with(']') || trimmed_str.ends_with('}')) && trimmed_end > trimmed_start {
                        trimmed_end -= 1;
                        trimmed_str = chars[trimmed_start..trimmed_end].iter().collect::<String>();
                    }
                    
                    if is_url(&trimmed_str) {
                        return Some(renderer::HoveredUrl {
                            line: hover_point.line.0,
                            start_col: trimmed_start,
                            end_col: trimmed_end - 1,
                        });
                    }
                }
            }
        }
    }

    None
}