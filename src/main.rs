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

struct Tab {
    terminal_state: Arc<parking_lot::Mutex<TerminalState>>,
    scroll_current: f32,
    scroll_target: f32,
    selection: Option<Selection>,
    is_selecting_text: bool,
    selection_start_pos: Option<(f64, f64)>,
    hovered_url: Option<renderer::HoveredUrl>,
    is_dragging: bool,
    last_activity_time: std::time::Instant,
    last_actual_offset: usize,
    last_scroll_diff: isize,
}

fn create_new_tab(
    shell: &str,
    scrollback: usize,
    font: config::FontConfig,
    cell_width: f32,
    cell_height: f32,
    cols: usize,
    rows: usize,
    proxy: winit::event_loop::EventLoopProxy<()>,
) -> anyhow::Result<Tab> {
    let terminal_state = TerminalState::new(
        shell,
        scrollback,
        font,
        cell_width,
        cell_height,
        cols.max(80) as f32 * cell_width,
        rows.max(24) as f32 * cell_height,
        proxy,
    )?;
    Ok(Tab {
        terminal_state: Arc::new(parking_lot::Mutex::new(terminal_state)),
        scroll_current: 0.0,
        scroll_target: 0.0,
        selection: None,
        is_selecting_text: false,
        selection_start_pos: None,
        hovered_url: None,
        is_dragging: false,
        last_activity_time: std::time::Instant::now(),
        last_actual_offset: 0,
        last_scroll_diff: 0,
    })
}

fn get_padding_top(_tab_count: usize) -> f32 {
    48.0
}

fn resize_all_tabs(
    tabs: &[Tab],
    width: u32,
    height: u32,
    cell_width: f32,
    cell_height: f32,
) -> (usize, usize) {
    const PADDING_LEFT: f32 = 10.0;
    const PADDING_BOTTOM: f32 = 10.0;
    let padding_top = get_padding_top(tabs.len());
    let cell_w = cell_width.max(1.0);
    let cell_h = cell_height.max(1.0);
    let cols = (((width as f32 - PADDING_LEFT * 2.0) / cell_w).floor().max(1.0)) as usize;
    let rows = (((height as f32 - (padding_top + PADDING_BOTTOM)) / cell_h).floor().max(1.0)) as usize;

    for tab in tabs {
        tab.terminal_state.lock().resize(cols, rows);
    }
    (cols, rows)
}

fn get_current_dir_shortened(pid: u32) -> Option<String> {
    let path = std::fs::read_link(format!("/proc/{}/cwd", pid)).ok()?;
    let path_str = path.to_string_lossy().into_owned();
    if let Ok(home) = std::env::var("HOME") {
        if path_str == home {
            return Some("~".to_string());
        }
        if let Some(stripped) = path_str.strip_prefix(&home) {
            if stripped.starts_with('/') {
                return Some(format!("~{}", stripped));
            }
        }
    }
    Some(path_str)
}

fn get_last_path_component(path_str: &str) -> String {
    if path_str == "~" {
        return "~".to_string();
    }
    if let Some(last) = std::path::Path::new(path_str).file_name() {
        last.to_string_lossy().into_owned()
    } else {
        path_str.to_string()
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let mut config = Config::load()?;
    tracing::info!(
        "fasty: starting, shell={}",
        config.shell.as_deref().unwrap_or("default")
    );

    let shell = if let Some(ref s) = config.shell {
        s.clone()
    } else {
        get_login_shell()
    };

    tracing::info!("Initializing EventLoop and Window...");
    let event_loop = EventLoop::new()?;
    let window = event_loop.create_window(winit::window::WindowAttributes::default()
        .with_title("fasty")
        .with_decorations(false)
        .with_transparent(true)
        .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 800.0)))?;
    tracing::info!("Window created successfully!");

    // Load and set the window icon at runtime for the taskbar/desktop bar
    if let Ok(icon_image) = image::open("assets/fastyIcon.png") {
        let icon_image = icon_image.into_rgba8();
        let (width, height) = icon_image.dimensions();
        let rgba = icon_image.into_raw();
        if let Ok(icon) = winit::window::Icon::from_rgba(rgba, width, height) {
            window.set_window_icon(Some(icon));
        }
    }

    let window_arc = Arc::new(window);
    let window_for_renderer = window_arc.as_ref();
    tracing::info!("Creating Renderer...");
    let renderer = pollster::block_on(Renderer::new(window_for_renderer, &config.font.family, config.font.size))?;
    tracing::info!("Renderer created successfully!");
    let mut cell_width = renderer.cell_width();
    let mut cell_height = renderer.cell_height();


    let viewport_width = renderer.config.width as f32;
    const PADDING_LEFT: f32 = 10.0;
    const PADDING_BOTTOM: f32 = 10.0;

    let viewport_height = renderer.config.height as f32;
    let mut shell_cols = ((viewport_width - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0) as usize;
    let mut shell_rows = ((viewport_height - (get_padding_top(1) + PADDING_BOTTOM)) / cell_height).floor().max(1.0) as usize;
    let proxy = event_loop.create_proxy();
    let initial_tab = create_new_tab(
        &shell,
        config.scrollback,
        config.font.clone(),
        cell_width,
        cell_height,
        shell_cols,
        shell_rows,
        proxy.clone(),
    )?;

    let mut tabs = vec![initial_tab];
    let mut active_tab_index = 0usize;
    let renderer = Arc::new(parking_lot::Mutex::new(renderer));
    let mut modifiers = winit::keyboard::ModifiersState::default();
    let mut ctrl_held = false;
    let mut shift_held = false;
    let mut alt_held = false;
    let render_generation = Arc::new(AtomicU64::new(0));
    let rg = Arc::clone(&render_generation);

    let window_for_redraw = window_arc.clone();
    
    let mut last_cursor_y = 0.0f64;

    let mut scrollbar_alpha = 0.0f32;
    let mut is_dragging_scrollbar = false;
    let mut scrollbar_drag_offset_y = 0.0f32;
    let mut current_mouse_x = 0.0f64;
    let mut current_mouse_y = 0.0f64;
    let mut last_click_time: Option<std::time::Instant> = None;
    let mut toast: Option<(String, std::time::Instant)> = None;
    let start_time = std::time::Instant::now();
    let mut clipboard: Option<arboard::Clipboard> = None;
    let mut context_menu_visible = false;
    let mut context_menu_x = 0.0f64;
    let mut context_menu_y = 0.0f64;
    let mut context_menu_hovered_idx: Option<usize> = None;
    let mut context_menu_open_time: Option<std::time::Instant> = None;
    let mut context_menu_open_time_secs: Option<f32> = None;
    let mut last_scroll_event_time: Option<std::time::Instant> = None;

    // Hover states for main window topbar buttons
    let mut hover_close = false;
    let mut hover_max = false;
    let mut hover_min = false;
    let mut hover_settings = false;
    let mut hovered_tab_index: Option<usize> = None;
    let mut hovered_close_tab_index: Option<usize> = None;
    let mut hover_new_tab = false;

    // Secondary settings window state
    let mut settings_window: Option<Arc<winit::window::Window>> = None;
    let mut settings_renderer: Option<Renderer<'static>> = None;
    let mut settings_family = String::new();
    let mut settings_size = 14.0f32;
    let mut settings_scrollback = 10000usize;
    let mut settings_active_field = 0usize; // 0 = none, 1 = font family select dropdown
    
    let mut s_hover_close = false;
    let mut s_hover_family = false;
    let mut s_hover_size_minus = false;
    let mut s_hover_size_plus = false;
    let mut s_hover_scroll_minus = false;
    let mut s_hover_scroll_plus = false;
    let mut s_hover_save = false;
    let mut s_hover_cancel = false;
    
    let mut settings_font_scroll_y = 0.0f32;
    let mut settings_hovered_font_idx: Option<usize> = None;
    let mut system_fonts = Vec::<String>::new();
    let mut s_mouse_x = 0.0f64;
    let mut s_mouse_y = 0.0f64;
    let mut first_frame_rendered = false;
    tracing::info!("Entering event loop run...");
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
                            let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);

                            shell_cols = cols;
                            shell_rows = rows;

                            let mut r = renderer.lock();
                            r.resize(physical_size.width, physical_size.height);
                            drop(r);
                            renderer.lock().set_dirty(true);
                            window_for_redraw.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            first_frame_rendered = true;

                            let mut tab_titles = Vec::new();
                            let mut active_tab_path = "fasty".to_string();
                            for (idx, tab) in tabs.iter().enumerate() {
                                let path_str = if let Some(pid) = tab.terminal_state.lock().shell_pid() {
                                    get_current_dir_shortened(pid)
                                } else {
                                    None
                                };
                                
                                let title = if let Some(ref path) = path_str {
                                    get_last_path_component(path)
                                } else {
                                    "bash".to_string()
                                };
                                
                                if idx == active_tab_index {
                                    if let Some(ref path) = path_str {
                                        active_tab_path = path.clone();
                                    } else {
                                        active_tab_path = "bash".to_string();
                                    }
                                }
                                tab_titles.push(title);
                            }

                            let active_tab = &tabs[active_tab_index];
                            let term = active_tab.terminal_state.lock();
                            let max_history = term.history_size() as f32;
                            let term_ref: &TerminalState = &*term;

                            let last_activity_time_secs = active_tab.last_activity_time.saturating_duration_since(start_time).as_secs_f32();
                            let current_time = start_time.elapsed().as_secs_f32();

                            renderer.lock().render(
                                term_ref,
                                scrollbar_alpha,
                                active_tab.scroll_current,
                                max_history,
                                shell_rows as f32,
                                hover_close,
                                hover_max,
                                hover_min,
                                hover_settings,
                                last_activity_time_secs,
                                current_time,
                                active_tab.selection,
                                active_tab.hovered_url,
                                toast.as_ref().map(|(msg, t)| (msg.as_str(), *t)),
                                active_tab_index,
                                &tab_titles,
                                &active_tab_path,
                                context_menu_visible,
                                context_menu_x as f32,
                                context_menu_y as f32,
                                context_menu_hovered_idx,
                                context_menu_open_time_secs,
                                hovered_tab_index,
                                hovered_close_tab_index,
                                hover_new_tab,
                            );
                        }
                        WindowEvent::ModifiersChanged(modified) => {
                            modifiers = modified.state();
                            let padding_top = get_padding_top(tabs.len());
                            let new_hover = detect_hovered_url(
                                current_mouse_x,
                                current_mouse_y,
                                modifiers.control_key() || ctrl_held,
                                &tabs[active_tab_index].terminal_state,
                                tabs[active_tab_index].scroll_current,
                                cell_width,
                                cell_height,
                                shell_cols,
                                shell_rows,
                                padding_top,
                            );
                            if tabs[active_tab_index].hovered_url != new_hover {
                                tabs[active_tab_index].hovered_url = new_hover;
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. } => {
                            let pressed = event.state == ElementState::Pressed;
                            
                            // Track Ctrl, Shift and Alt modifiers manually in case ModifiersChanged is missed
                            match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ControlLeft) |
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ControlRight) => {
                                    ctrl_held = pressed;
                                }
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ShiftLeft) |
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ShiftRight) => {
                                    shift_held = pressed;
                                }
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::AltLeft) |
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::AltRight) => {
                                    alt_held = pressed;
                                }
                                _ => {}
                            }

                            let ctrl_active = modifiers.control_key() || ctrl_held;
                            let shift_active = modifiers.shift_key() || shift_held;
                            let alt_active = modifiers.alt_key() || alt_held;

                            tabs[active_tab_index].last_activity_time = std::time::Instant::now();

                            let padding_top = get_padding_top(tabs.len());

                            let new_hover = detect_hovered_url(
                                current_mouse_x,
                                current_mouse_y,
                                ctrl_active,
                                &tabs[active_tab_index].terminal_state,
                                tabs[active_tab_index].scroll_current,
                                cell_width,
                                cell_height,
                                shell_cols,
                                shell_rows,
                                padding_top,
                            );
                            if tabs[active_tab_index].hovered_url != new_hover {
                                tabs[active_tab_index].hovered_url = new_hover;
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

                            let is_t_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyT) => true,
                                _ => key_str.eq_ignore_ascii_case("t")
                            };

                            if ctrl_active && shift_active && is_t_key {
                                let new_tab_count = tabs.len() + 1;
                                let padding_top = get_padding_top(new_tab_count);
                                let physical_size = window_for_redraw.inner_size();
                                let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                let new_rows = (((physical_size.height as f32 - (padding_top + PADDING_BOTTOM)) / cell_height).floor().max(1.0)) as usize;
                                
                                match create_new_tab(
                                    &shell,
                                    config.scrollback,
                                    config.font.clone(),
                                    cell_width,
                                    cell_height,
                                    new_cols,
                                    new_rows,
                                    proxy.clone(),
                                ) {
                                    Ok(new_tab) => {
                                        tabs.push(new_tab);
                                        active_tab_index = tabs.len() - 1;
                                        
                                        let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                        shell_cols = cols;
                                        shell_rows = rows;
                                        
                                        renderer.lock().set_dirty(true);
                                        window_for_redraw.request_redraw();
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to create new tab: {:?}", e);
                                    }
                                }
                                return;
                            }

                            let is_w_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyW) => true,
                                _ => key_str.eq_ignore_ascii_case("w")
                            };
                            if ctrl_active && shift_active && is_w_key {
                                if tabs.len() <= 1 {
                                    target.exit();
                                    return;
                                } else {
                                    tabs.remove(active_tab_index);
                                    if active_tab_index >= tabs.len() {
                                        active_tab_index = tabs.len() - 1;
                                    }
                                    let physical_size = window_for_redraw.inner_size();
                                    let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                    shell_cols = cols;
                                    shell_rows = rows;
                                    
                                    renderer.lock().set_dirty(true);
                                    window_for_redraw.request_redraw();
                                }
                                return;
                            }

                            // Ctrl+Tab / Ctrl+PageDown to switch to next tab
                            let is_tab_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Tab) => true,
                                _ => key_str == "Tab"
                            };
                            let is_page_down_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::PageDown) => true,
                                _ => key_str == "PageDown"
                            };
                            if ctrl_active && !shift_active && (is_tab_key || is_page_down_key) {
                                if tabs.len() > 1 {
                                    active_tab_index = (active_tab_index + 1) % tabs.len();
                                    renderer.lock().set_dirty(true);
                                    window_for_redraw.request_redraw();
                                }
                                return;
                            }

                            // Ctrl+Shift+Tab / Ctrl+PageUp to switch to previous tab
                            let is_page_up_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::PageUp) => true,
                                _ => key_str == "PageUp"
                            };
                            if ctrl_active && shift_active && (is_tab_key || is_page_up_key) {
                                if tabs.len() > 1 {
                                    if active_tab_index == 0 {
                                        active_tab_index = tabs.len() - 1;
                                    } else {
                                        active_tab_index -= 1;
                                    }
                                    renderer.lock().set_dirty(true);
                                    window_for_redraw.request_redraw();
                                }
                                return;
                            }

                            // Alt+1 to Alt+9 to switch tabs
                            if alt_active && !ctrl_active && !shift_active {
                                if let Some(digit_char) = key_str.chars().next() {
                                    if digit_char.is_ascii_digit() && digit_char != '0' {
                                        let target_idx = (digit_char as usize - '1' as usize);
                                        if target_idx < tabs.len() {
                                            active_tab_index = target_idx;
                                            renderer.lock().set_dirty(true);
                                            window_for_redraw.request_redraw();
                                            return;
                                        }
                                    }
                                }
                            }

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
                                                     let term = tabs[active_tab_index].terminal_state.lock();
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
                                                     tabs[active_tab_index].terminal_state.lock().write_to_pty(&paste_bytes);
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
                                    "c" => { tabs[active_tab_index].terminal_state.lock().write_to_pty(&[0x03]); return; }
                                    "d" => { tabs[active_tab_index].terminal_state.lock().write_to_pty(&[0x04]); return; }
                                    "z" => { tabs[active_tab_index].terminal_state.lock().write_to_pty(&[0x1A]); return; }
                                    "l" => { tabs[active_tab_index].terminal_state.lock().write_to_pty(&[0x0C]); return; }
                                    _ => {}
                                }
                            }

                            let bytes = key_to_bytes(&event.logical_key, &modifiers);
                            if !bytes.is_empty() {
                                tabs[active_tab_index].terminal_state.lock().write_to_pty(&bytes);
                            }
                        }
                        WindowEvent::MouseInput { state, button, .. } => {
                            let padding_top = get_padding_top(tabs.len());
                            tabs[active_tab_index].last_activity_time = std::time::Instant::now();

                            if context_menu_visible {
                                let pressed = state == ElementState::Pressed;
                                if pressed {
                                    if button == MouseButton::Left {
                                        if let Some(hovered_idx) = context_menu_hovered_idx {
                                            let menu_items = get_context_menu_items(&tabs, active_tab_index);
                                            if hovered_idx < menu_items.len() {
                                                let item = menu_items[hovered_idx];
                                                match item {
                                                    crate::renderer::ContextMenuItem::Copy => {
                                                        if let Some(sel) = tabs[active_tab_index].selection {
                                                            copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
                                                            toast = Some((
                                                                "✓  Text copied".to_string(),
                                                                std::time::Instant::now(),
                                                            ));
                                                        }
                                                    }
                                                    crate::renderer::ContextMenuItem::Paste => {
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
                                                                    if !text.is_empty() {
                                                                        let term = tabs[active_tab_index].terminal_state.lock();
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
                                                                        tabs[active_tab_index].terminal_state.lock().write_to_pty(&paste_bytes);
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    eprintln!("fasty clipboard get_text failed: {:?}", e);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    crate::renderer::ContextMenuItem::NewTab => {
                                                        let new_tab_count = tabs.len() + 1;
                                                        let padding_top = get_padding_top(new_tab_count);
                                                        let physical_size = window_for_redraw.inner_size();
                                                        let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                                        let new_rows = (((physical_size.height as f32 - (padding_top + PADDING_BOTTOM)) / cell_height).floor().max(1.0)) as usize;
                                                        
                                                        match create_new_tab(
                                                            &shell,
                                                            config.scrollback,
                                                            config.font.clone(),
                                                            cell_width,
                                                            cell_height,
                                                            new_cols,
                                                            new_rows,
                                                            proxy.clone(),
                                                        ) {
                                                            Ok(new_tab) => {
                                                                tabs.push(new_tab);
                                                                active_tab_index = tabs.len() - 1;
                                                                
                                                                let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                                shell_cols = cols;
                                                                shell_rows = rows;
                                                            }
                                                            Err(e) => {
                                                                tracing::error!("Failed to create new tab: {:?}", e);
                                                            }
                                                        }
                                                    }
                                                    crate::renderer::ContextMenuItem::CloseTab => {
                                                        tabs.remove(active_tab_index);
                                                        if active_tab_index >= tabs.len() {
                                                            active_tab_index = tabs.len() - 1;
                                                        }
                                                        let physical_size = window_for_redraw.inner_size();
                                                        let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                        shell_cols = cols;
                                                        shell_rows = rows;
                                                    }
                                                    crate::renderer::ContextMenuItem::Separator => {}
                                                }
                                            }
                                        }
                                    } else if button == MouseButton::Right {
                                        // Reposition menu
                                        let menu_items = get_context_menu_items(&tabs, active_tab_index);
                                        let (menu_w, menu_h) = get_context_menu_size(&menu_items);
                                        let r = renderer.lock();
                                        let v_width = r.config.width as f64;
                                        drop(r);
                                        
                                        if current_mouse_y >= padding_top as f64 && current_mouse_x <= (v_width - 20.0) {
                                            context_menu_x = current_mouse_x;
                                            context_menu_y = current_mouse_y;
                                            context_menu_open_time = Some(std::time::Instant::now());
                                            context_menu_open_time_secs = Some(start_time.elapsed().as_secs_f32());
                                            
                                            // Adjust bounds
                                            let v_height = window_for_redraw.inner_size().height as f64;
                                            if context_menu_x + menu_w > v_width {
                                                context_menu_x = v_width - menu_w - 4.0;
                                            }
                                            if context_menu_y + menu_h > v_height {
                                                context_menu_y = v_height - menu_h - 4.0;
                                            }
                                            
                                            context_menu_hovered_idx = None;
                                            renderer.lock().set_dirty(true);
                                            window_for_redraw.request_redraw();
                                            return;
                                        }
                                    }
                                    context_menu_visible = false;
                                    context_menu_open_time = None;
                                    context_menu_open_time_secs = None;
                                    context_menu_hovered_idx = None;
                                    renderer.lock().set_dirty(true);
                                    window_for_redraw.request_redraw();
                                }
                                return;
                            }

                            if button == MouseButton::Left {
                                let pressed = state == ElementState::Pressed;
                                if pressed {
                                    let r = renderer.lock();
                                    let v_width = r.config.width as f64;
                                    let v_height = r.config.height as f32;
                                    drop(r);

                                    const RESIZE_BORDER_WIDTH: f64 = 8.0;
                                    if let Some(dir) = get_resize_direction(current_mouse_x, current_mouse_y, v_width, v_height as f64, RESIZE_BORDER_WIDTH) {
                                        let _ = window_for_redraw.drag_resize_window(dir);
                                        return;
                                    }

                                    if current_mouse_y <= 40.0 {
                                         // 1. Check topbar control buttons first
                                         let is_hovering_close = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 36.0) && current_mouse_x < (v_width - 8.0);
                                         let is_hovering_max = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 68.0) && current_mouse_x < (v_width - 40.0);
                                         let is_hovering_min = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 100.0) && current_mouse_x < (v_width - 72.0);
                                         let is_hovering_settings = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 137.0) && current_mouse_x < (v_width - 109.0);

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
                                                  settings_family = config.font.family.clone();
                                                  settings_size = config.font.size;
                                                  settings_scrollback = config.scrollback;
                                                  settings_active_field = 0;
                                                 if let Ok(sw) = target.create_window(winit::window::WindowAttributes::default()
                                                     .with_title("fasty Settings")
                                                     .with_decorations(false)
                                                     .with_transparent(true)
                                                     .with_inner_size(winit::dpi::LogicalSize::new(400.0, 300.0))) {
                                                     let sw_arc = Arc::new(sw);
                                                     let sw_ref: &winit::window::Window = &*sw_arc;
                                                     let sw_static: &'static winit::window::Window = unsafe { std::mem::transmute(sw_ref) };
                                                     if let Ok(sr) = pollster::block_on(Renderer::new(sw_static, &settings_family, settings_size)) {
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
                                         }

                                         // 2. Check tab clicks & close tab clicks & new tab click
                                         let tab_start_x = 36.0;
                                         let path_center_x = v_width / 2.0;
                                         let tab_area_max_x = path_center_x - 40.0;
                                         let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                         let tabs_len = tabs.len();
                                         let tab_width = if tabs_len > 0 {
                                             (tab_area_width / tabs_len as f64).clamp(80.0, 160.0)
                                         } else {
                                             160.0
                                         };

                                         let tabs_total_width = tabs_len as f64 * tab_width;
                                         if current_mouse_x >= tab_start_x && current_mouse_x < tab_start_x + tabs_total_width {
                                             let clicked_tab_idx = ((current_mouse_x - tab_start_x) / tab_width) as usize;
                                             if clicked_tab_idx < tabs_len {
                                                 let tab_x = tab_start_x + clicked_tab_idx as f64 * tab_width;
                                                 let close_x = tab_x + tab_width - 30.0;
                                                 let close_min_x = close_x - 4.0;
                                                 let close_max_x = close_x + 20.0;
                                                 let close_min_y = 8.0;
                                                 let close_max_y = 32.0;
                                                 let is_close_click = current_mouse_x >= close_min_x && current_mouse_x <= close_max_x
                                                     && current_mouse_y >= close_min_y && current_mouse_y <= close_max_y;

                                                 if is_close_click {
                                                     tabs.remove(clicked_tab_idx);
                                                     if tabs.is_empty() {
                                                         target.exit();
                                                         return;
                                                     }
                                                     if active_tab_index >= tabs.len() {
                                                         active_tab_index = tabs.len() - 1;
                                                     }
                                                     let physical_size = window_for_redraw.inner_size();
                                                     let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                     shell_cols = cols;
                                                     shell_rows = rows;
                                                 } else {
                                                     active_tab_index = clicked_tab_idx;
                                                 }
                                                 renderer.lock().set_dirty(true);
                                                 window_for_redraw.request_redraw();
                                             }
                                             return;
                                         }

                                         // Check new tab button click
                                         let new_tab_x = tab_start_x + tabs_total_width;
                                         if current_mouse_x >= new_tab_x && current_mouse_x < new_tab_x + 32.0 {
                                             let new_tab_count = tabs.len() + 1;
                                             let padding_top = get_padding_top(new_tab_count);
                                             let physical_size = window_for_redraw.inner_size();
                                             let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                             let new_rows = (((physical_size.height as f32 - (padding_top + PADDING_BOTTOM)) / cell_height).floor().max(1.0)) as usize;
                                             
                                             match create_new_tab(
                                                 &shell,
                                                 config.scrollback,
                                                 config.font.clone(),
                                                 cell_width,
                                                 cell_height,
                                                 new_cols,
                                                 new_rows,
                                                 proxy.clone(),
                                             ) {
                                                 Ok(new_tab) => {
                                                     tabs.push(new_tab);
                                                     active_tab_index = tabs.len() - 1;
                                                     
                                                     let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                     shell_cols = cols;
                                                     shell_rows = rows;
                                                 }
                                                 Err(e) => {
                                                     tracing::error!("Failed to create new tab: {:?}", e);
                                                 }
                                             }
                                             renderer.lock().set_dirty(true);
                                             window_for_redraw.request_redraw();
                                             return;
                                         }

                                         // 3. Otherwise (blank space click), drag the window
                                         // Don't drag the window if clicking near the control buttons region
                                         if current_mouse_x < (v_width - 141.0) {
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

                                    let scrollbar_top_margin = padding_top - 10.0;
                                    let is_hovering_scrollbar = current_mouse_y > scrollbar_top_margin as f64 && current_mouse_x > (v_width - 20.0);
                                    if is_hovering_scrollbar {
                                        let term = tabs[active_tab_index].terminal_state.lock();
                                        let history_size = term.history_size() as f32;
                                        let visible_rows = shell_rows as f32;
                                        drop(term);

                                        let total_lines = visible_rows + history_size;
                                        if total_lines > 0.0 {
                                            let ratio = visible_rows / total_lines;
                                            let track_h = v_height - scrollbar_top_margin - 4.0;
                                            let thumb_h = (track_h * ratio).max(20.0).min(track_h);

                                            let scroll_ratio = if history_size > 0.0 {
                                                tabs[active_tab_index].scroll_current / history_size
                                            } else {
                                                0.0
                                            };

                                            let thumb_y = scrollbar_top_margin + (1.0 - scroll_ratio) * (track_h - thumb_h);

                                            if current_mouse_y >= thumb_y as f64 && current_mouse_y <= (thumb_y + thumb_h) as f64 {
                                                is_dragging_scrollbar = true;
                                                scrollbar_drag_offset_y = (current_mouse_y - thumb_y as f64) as f32;
                                            } else {
                                                let track_center = track_h - thumb_h;
                                                let click_y = (current_mouse_y - scrollbar_top_margin as f64 - thumb_h as f64 / 2.0).clamp(0.0, track_center as f64);
                                                let new_ratio = 1.0 - (click_y / track_center as f64) as f32;
                                                tabs[active_tab_index].scroll_target = new_ratio * history_size;

                                                is_dragging_scrollbar = true;
                                                scrollbar_drag_offset_y = thumb_h / 2.0;
                                                window_for_redraw.request_redraw();
                                            }
                                        }
                                    } else if modifiers.control_key() || ctrl_held {
                                        // Ctrl + Click: Try to detect and open URL
                                        let term = tabs[active_tab_index].terminal_state.lock();
                                        let scroll_fraction = tabs[active_tab_index].scroll_current - term.display_offset() as f32;
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
                                            padding_top,
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
                                        tabs[active_tab_index].selection_start_pos = Some((current_mouse_x, current_mouse_y));
                                        tabs[active_tab_index].is_selecting_text = false;
                                    }
                                } else {
                                    tabs[active_tab_index].is_dragging = false;
                                    is_dragging_scrollbar = false;
                                    if tabs[active_tab_index].is_selecting_text {
                                        tabs[active_tab_index].is_selecting_text = false;
                                        if let Some(sel) = tabs[active_tab_index].selection {
                                            copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
                                            toast = Some((
                                                "✓  Text copied".to_string(),
                                                std::time::Instant::now(),
                                            ));
                                            renderer.lock().set_dirty(true);
                                            window_for_redraw.request_redraw();
                                        }
                                    } else if tabs[active_tab_index].selection_start_pos.is_some() {
                                        // Simple click release (no drag occurred): clear selection
                                        tabs[active_tab_index].selection = None;
                                        renderer.lock().set_dirty(true);
                                        window_for_redraw.request_redraw();
                                    }
                                    tabs[active_tab_index].selection_start_pos = None;
                                }
                            } else if button == MouseButton::Right {
                                let pressed = state == ElementState::Pressed;
                                if pressed {
                                    let r = renderer.lock();
                                    let v_width = r.config.width as f64;
                                    drop(r);
                                    
                                    if current_mouse_y >= padding_top as f64 && current_mouse_x <= (v_width - 20.0) {
                                        let menu_items = get_context_menu_items(&tabs, active_tab_index);
                                        let (menu_w, menu_h) = get_context_menu_size(&menu_items);
                                        
                                        context_menu_x = current_mouse_x;
                                        context_menu_y = current_mouse_y;
                                        context_menu_open_time = Some(std::time::Instant::now());
                                        context_menu_open_time_secs = Some(start_time.elapsed().as_secs_f32());
                                        
                                        // Adjust X to stay within viewport
                                        let v_height = window_for_redraw.inner_size().height as f64;
                                        if context_menu_x + menu_w > v_width {
                                            context_menu_x = v_width - menu_w - 4.0;
                                        }
                                        // Adjust Y to stay within viewport
                                        if context_menu_y + menu_h > v_height {
                                            context_menu_y = v_height - menu_h - 4.0;
                                        }
                                        
                                        context_menu_visible = true;
                                        context_menu_hovered_idx = None;
                                        renderer.lock().set_dirty(true);
                                        window_for_redraw.request_redraw();
                                    }
                                }
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            let delta_y = position.y - last_cursor_y;
                            last_cursor_y = position.y;
                            current_mouse_x = position.x;
                            current_mouse_y = position.y;

                            if context_menu_visible {
                                let menu_items = get_context_menu_items(&tabs, active_tab_index);
                                let (menu_w, menu_h) = get_context_menu_size(&menu_items);
                                if current_mouse_x >= context_menu_x && current_mouse_x <= context_menu_x + menu_w
                                   && current_mouse_y >= context_menu_y && current_mouse_y <= context_menu_y + menu_h {
                                    let relative_y = (current_mouse_y - context_menu_y) as f32;
                                    context_menu_hovered_idx = get_menu_item_at_y(&menu_items, relative_y);
                                } else {
                                    context_menu_hovered_idx = None;
                                }
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }

                            let r = renderer.lock();
                            let v_width = r.config.width as f64;
                            let v_height = r.config.height as f32;
                            drop(r);

                            let padding_top = get_padding_top(tabs.len());
                            let is_in_terminal_area = current_mouse_y > padding_top as f64 && current_mouse_x <= (v_width - 20.0);
                            let is_dragging_anything = tabs[active_tab_index].is_dragging || is_dragging_scrollbar || tabs[active_tab_index].selection_start_pos.is_some();
                            const RESIZE_BORDER_WIDTH: f64 = 8.0;
                             
                            if context_menu_visible {
                                window_for_redraw.set_cursor(winit::window::CursorIcon::Default);
                            } else if !is_dragging_anything {
                                if let Some(dir) = get_resize_direction(current_mouse_x, current_mouse_y, v_width, v_height as f64, RESIZE_BORDER_WIDTH) {
                                    window_for_redraw.set_cursor(resize_direction_to_cursor(dir));
                                } else if is_in_terminal_area {
                                    window_for_redraw.set_cursor(winit::window::CursorIcon::Text);
                                } else {
                                    window_for_redraw.set_cursor(winit::window::CursorIcon::Default);
                                }
                            } else if tabs[active_tab_index].selection_start_pos.is_some() || is_in_terminal_area {
                                window_for_redraw.set_cursor(winit::window::CursorIcon::Text);
                            } else {
                                window_for_redraw.set_cursor(winit::window::CursorIcon::Default);
                            }

                            let old_hover_close = hover_close;
                            let old_hover_max = hover_max;
                            let old_hover_min = hover_min;
                            let old_hover_settings = hover_settings;
                            let old_hovered_tab = hovered_tab_index;
                            let old_hovered_close = hovered_close_tab_index;
                            let old_hover_new = hover_new_tab;

                            hover_close = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 36.0) && current_mouse_x < (v_width - 8.0);
                            hover_max = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 68.0) && current_mouse_x < (v_width - 40.0);
                            hover_min = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 100.0) && current_mouse_x < (v_width - 72.0);
                            hover_settings = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 137.0) && current_mouse_x < (v_width - 109.0);

                            hovered_tab_index = None;
                            hovered_close_tab_index = None;
                            hover_new_tab = false;

                            if current_mouse_y >= 0.0 && current_mouse_y <= 40.0 {
                                let tab_start_x = 36.0;
                                let path_center_x = v_width / 2.0;
                                let tab_area_max_x = path_center_x - 40.0;
                                let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                let tabs_len = tabs.len();
                                let tab_width = if tabs_len > 0 {
                                    (tab_area_width / tabs_len as f64).clamp(80.0, 160.0)
                                } else {
                                    160.0
                                };

                                let tabs_total_width = tabs_len as f64 * tab_width;
                                if current_mouse_x >= tab_start_x && current_mouse_x < tab_start_x + tabs_total_width {
                                    let idx = ((current_mouse_x - tab_start_x) / tab_width) as usize;
                                    if idx < tabs_len {
                                        hovered_tab_index = Some(idx);
                                        
                                        let tab_x = tab_start_x + idx as f64 * tab_width;
                                        let close_x = tab_x + tab_width - 30.0;
                                        let close_min_x = close_x - 4.0;
                                        let close_max_x = close_x + 20.0;
                                        let close_min_y = 8.0;
                                        let close_max_y = 32.0;
                                        if current_mouse_x >= close_min_x && current_mouse_x <= close_max_x
                                            && current_mouse_y >= close_min_y && current_mouse_y <= close_max_y
                                        {
                                            hovered_close_tab_index = Some(idx);
                                        }
                                    }
                                } else {
                                    let new_tab_x = tab_start_x + tabs_total_width;
                                    if current_mouse_x >= new_tab_x && current_mouse_x < new_tab_x + 32.0 {
                                        hover_new_tab = true;
                                    }
                                }
                            }

                            if hover_close != old_hover_close
                                || hover_max != old_hover_max
                                || hover_min != old_hover_min
                                || hover_settings != old_hover_settings
                                || hovered_tab_index != old_hovered_tab
                                || hovered_close_tab_index != old_hovered_close
                                || hover_new_tab != old_hover_new
                            {
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }

                            if is_dragging_scrollbar {
                                let term = tabs[active_tab_index].terminal_state.lock();
                                let history_size = term.history_size() as f32;
                                let visible_rows = shell_rows as f32;
                                drop(term);

                                let total_lines = visible_rows + history_size;
                                if total_lines > 0.0 {
                                    let ratio = visible_rows / total_lines;
                                    let scrollbar_top_margin = padding_top - 10.0;
                                    let track_h = v_height - scrollbar_top_margin - 4.0;
                                    let thumb_h = (track_h * ratio).max(20.0).min(track_h);
                                    let track_center = track_h - thumb_h;

                                    if track_center > 0.0 {
                                        let new_thumb_y = (current_mouse_y as f32 - scrollbar_top_margin - scrollbar_drag_offset_y).clamp(0.0, track_center);
                                        let scroll_ratio = 1.0 - (new_thumb_y / track_center);
                                        tabs[active_tab_index].scroll_target = scroll_ratio * history_size;
                                    }
                                }
                                window_for_redraw.request_redraw();
                            } else if let Some((sx, sy)) = tabs[active_tab_index].selection_start_pos {
                                if !tabs[active_tab_index].is_selecting_text {
                                    let dist = ((current_mouse_x - sx).powi(2) + (current_mouse_y - sy).powi(2)).sqrt();
                                    if dist > 5.0 {
                                        tabs[active_tab_index].is_selecting_text = true;
                                        let term = tabs[active_tab_index].terminal_state.lock();
                                        let scroll_fraction = tabs[active_tab_index].scroll_current - term.display_offset() as f32;
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
                                            padding_top,
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
                                            padding_top,
                                        );
                                        tabs[active_tab_index].selection = Some(Selection { start: start_point, end: current_point });
                                        renderer.lock().set_dirty(true);
                                        window_for_redraw.request_redraw();
                                    }
                                } else {
                                    let term = tabs[active_tab_index].terminal_state.lock();
                                    let scroll_fraction = tabs[active_tab_index].scroll_current - term.display_offset() as f32;
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
                                        padding_top,
                                    );
                                    if let Some(ref mut sel) = tabs[active_tab_index].selection {
                                        if sel.end != current_point {
                                            sel.end = current_point;
                                            renderer.lock().set_dirty(true);
                                            window_for_redraw.request_redraw();
                                        }
                                    }
                                }
                            } else if tabs[active_tab_index].is_dragging {
                                let drag_speed = 1.0f32;
                                let delta_lines = (delta_y as f32 / cell_height) * drag_speed;
                                let max_history = tabs[active_tab_index].terminal_state.lock().history_size() as f32;
                                tabs[active_tab_index].scroll_target = (tabs[active_tab_index].scroll_target + delta_lines).clamp(0.0, max_history);
                                window_for_redraw.request_redraw();
                            }

                            let new_hover = detect_hovered_url(
                                current_mouse_x,
                                current_mouse_y,
                                modifiers.control_key() || ctrl_held,
                                &tabs[active_tab_index].terminal_state,
                                tabs[active_tab_index].scroll_current,
                                cell_width,
                                cell_height,
                                shell_cols,
                                shell_rows,
                                padding_top,
                            );
                            if tabs[active_tab_index].hovered_url != new_hover {
                                tabs[active_tab_index].hovered_url = new_hover;
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }
                        }
                        WindowEvent::MouseWheel { delta, .. } => {
                            last_scroll_event_time = Some(std::time::Instant::now());
                            renderer.lock().set_dirty(true);

                            let lines = match delta {
                                MouseScrollDelta::LineDelta(_, y) => y,
                                MouseScrollDelta::PixelDelta(pos) => {
                                    pos.y as f32 / cell_height
                                }
                            };

                            let max_history = tabs[active_tab_index].terminal_state.lock().history_size() as f32;
                            let scroll_speed = 3.0f32;

                            let delta_scroll = match delta {
                                MouseScrollDelta::LineDelta(_, _) => lines * scroll_speed,
                                MouseScrollDelta::PixelDelta(_) => lines,
                            };

                            tabs[active_tab_index].scroll_target = (tabs[active_tab_index].scroll_target + delta_scroll).clamp(0.0, max_history);
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

                            let url_changed = tabs[active_tab_index].hovered_url.is_some();
                            tabs[active_tab_index].hovered_url = None;

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
                                tabs[active_tab_index].is_dragging = false;
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
                        macro_rules! apply_settings {
                            () => {
                                let mut current_config = Config::load().unwrap_or_default();
                                current_config.font.family = settings_family.clone();
                                current_config.font.size = settings_size;
                                current_config.scrollback = settings_scrollback;
                                let _ = current_config.save(&Config::config_path());

                                config = current_config;

                                // Apply the new font family and size dynamically to the renderer!
                                if let Err(e) = renderer.lock().update_font(&config.font.family, config.font.size) {
                                    tracing::error!("Failed to update renderer font: {:?}", e);
                                }

                                // Apply new scrollback limit to all existing terminal state instances!
                                for tab in &tabs {
                                    tab.terminal_state.lock().update_scrollback(config.scrollback);
                                }

                                // Recalculate columns and rows based on the new cell sizes!
                                let cell_w = renderer.lock().cell_width();
                                let cell_h = renderer.lock().cell_height();
                                let physical_size = window_for_redraw.inner_size();
                                let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_w, cell_h);
                                shell_cols = cols;
                                shell_rows = rows;
                                cell_width = cell_w;
                                cell_height = cell_h;

                                window_for_redraw.request_redraw();
                            }
                        }

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
                                        &system_fonts,
                                        settings_font_scroll_y,
                                        settings_hovered_font_idx,
                                    );
                                }
                            }
                            WindowEvent::CursorMoved { position, .. } => {
                                let scale_factor = sw.scale_factor();
                                s_mouse_x = position.x / scale_factor;
                                s_mouse_y = position.y / scale_factor;

                                let old_hover_close = s_hover_close;
                                let old_hover_family = s_hover_family;
                                let old_hover_size_minus = s_hover_size_minus;
                                let old_hover_size_plus = s_hover_size_plus;
                                let old_hover_scroll_minus = s_hover_scroll_minus;
                                let old_hover_scroll_plus = s_hover_scroll_plus;
                                let old_hover_save = s_hover_save;
                                let old_hover_cancel = s_hover_cancel;
                                let old_hovered_font_idx = settings_hovered_font_idx;

                                s_hover_close = s_mouse_y >= 4.0 && s_mouse_y <= 32.0 && s_mouse_x >= (400.0 - 32.0) && s_mouse_x < (400.0 - 4.0);
                                s_hover_family = s_mouse_y >= 52.0 && s_mouse_y <= 78.0 && s_mouse_x >= 140.0 && s_mouse_x < 380.0;

                                s_hover_size_minus = s_mouse_y >= 92.0 && s_mouse_y <= 118.0 && s_mouse_x >= 140.0 && s_mouse_x < 168.0;
                                s_hover_size_plus = s_mouse_y >= 92.0 && s_mouse_y <= 118.0 && s_mouse_x >= 220.0 && s_mouse_x < 248.0;

                                s_hover_scroll_minus = s_mouse_y >= 132.0 && s_mouse_y <= 158.0 && s_mouse_x >= 140.0 && s_mouse_x < 168.0;
                                s_hover_scroll_plus = s_mouse_y >= 132.0 && s_mouse_y <= 158.0 && s_mouse_x >= 240.0 && s_mouse_x < 268.0;

                                s_hover_save = s_mouse_y >= 220.0 && s_mouse_y <= 252.0 && s_mouse_x >= 90.0 && s_mouse_x < 190.0;
                                s_hover_cancel = s_mouse_y >= 220.0 && s_mouse_y <= 252.0 && s_mouse_x >= 210.0 && s_mouse_x < 310.0;

                                let mut hovered_font_idx = None;
                                if settings_active_field == 1 && s_mouse_x >= 140.0 && s_mouse_x < 380.0 && s_mouse_y >= 78.0 && s_mouse_y < 258.0 {
                                    let idx = (((s_mouse_y - 78.0) + settings_font_scroll_y as f64) / 22.0) as usize;
                                    if idx < system_fonts.len() {
                                        hovered_font_idx = Some(idx);
                                    }
                                }
                                settings_hovered_font_idx = hovered_font_idx;

                                let any_changed = s_hover_close != old_hover_close
                                    || s_hover_family != old_hover_family
                                    || s_hover_size_minus != old_hover_size_minus
                                    || s_hover_size_plus != old_hover_size_plus
                                    || s_hover_scroll_minus != old_hover_scroll_minus
                                    || s_hover_scroll_plus != old_hover_scroll_plus
                                    || s_hover_save != old_hover_save
                                    || s_hover_cancel != old_hover_cancel
                                    || settings_hovered_font_idx != old_hovered_font_idx;

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
                                    } else if s_mouse_y <= 36.0 {
                                        let _ = sw.drag_window();
                                    } else if s_hover_family {
                                        if system_fonts.is_empty() {
                                            system_fonts = get_system_fonts();
                                        }
                                        if let Some(pos) = system_fonts.iter().position(|f| f == &settings_family) {
                                            let current = system_fonts.remove(pos);
                                            system_fonts.insert(0, current);
                                        } else {
                                            system_fonts.insert(0, settings_family.clone());
                                        }
                                        settings_active_field = 1;
                                        settings_font_scroll_y = 0.0;
                                    } else if settings_active_field == 1 && s_mouse_x >= 140.0 && s_mouse_x < 380.0 && s_mouse_y >= 78.0 && s_mouse_y < 258.0 {
                                        let idx = (((s_mouse_y - 78.0) + settings_font_scroll_y as f64) / 22.0) as usize;
                                        if idx < system_fonts.len() {
                                            settings_family = system_fonts[idx].clone();
                                            settings_active_field = 0;
                                            apply_settings!();
                                        }
                                    } else if s_hover_size_minus {
                                        settings_size = (settings_size - 0.5).max(6.0);
                                        apply_settings!();
                                    } else if s_hover_size_plus {
                                        settings_size = (settings_size + 0.5).min(72.0);
                                        apply_settings!();
                                    } else if s_hover_scroll_minus {
                                        settings_scrollback = settings_scrollback.saturating_sub(1000).max(1000);
                                        apply_settings!();
                                    } else if s_hover_scroll_plus {
                                        settings_scrollback = settings_scrollback.saturating_add(1000).min(1000000);
                                        apply_settings!();
                                    } else if s_hover_save {
                                        settings_window = None;
                                        settings_renderer = None;
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
                            WindowEvent::MouseWheel { delta, .. } => {
                                if settings_active_field == 1 {
                                    let lines = match delta {
                                        MouseScrollDelta::LineDelta(_, y) => y,
                                        MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 22.0,
                                    };
                                    let item_h = 22.0f32;
                                    let visible_h = 180.0f32;
                                    let total_h = system_fonts.len() as f32 * item_h;
                                    let max_scroll = (total_h - visible_h).max(0.0);
                                    settings_font_scroll_y = (settings_font_scroll_y - lines * item_h).clamp(0.0, max_scroll);
                                    
                                    if let Some(ref mut r) = settings_renderer {
                                        r.set_dirty(true);
                                    }
                                    sw.request_redraw();
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
                                        Key::Named(winit::keyboard::NamedKey::Enter) => {
                                            settings_active_field = 0;
                                            apply_settings!();
                                        }
                                        Key::Named(winit::keyboard::NamedKey::Escape) => {
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
                if !first_frame_rendered {
                    first_frame_rendered = true;
                    // Force render the first frame directly to commit the Wayland buffer and map the window!
                    let mut tab_titles = Vec::new();
                    let mut active_tab_path = "fasty".to_string();
                    for (idx, tab) in tabs.iter().enumerate() {
                        let path_str = if let Some(pid) = tab.terminal_state.lock().shell_pid() {
                            get_current_dir_shortened(pid)
                        } else {
                            None
                        };
                        
                        let title = if let Some(ref path) = path_str {
                            get_last_path_component(path)
                        } else {
                            "bash".to_string()
                        };
                        
                        if idx == active_tab_index {
                            if let Some(ref path) = path_str {
                                active_tab_path = path.clone();
                            } else {
                                active_tab_path = "bash".to_string();
                            }
                        }
                        tab_titles.push(title);
                    }

                    let active_tab = &tabs[active_tab_index];
                    let term = active_tab.terminal_state.lock();
                    let max_history = term.history_size() as f32;
                    let term_ref: &TerminalState = &*term;

                    let last_activity_time_secs = active_tab.last_activity_time.saturating_duration_since(start_time).as_secs_f32();
                    let current_time = start_time.elapsed().as_secs_f32();

                    renderer.lock().set_dirty(true);
                    renderer.lock().render(
                        term_ref,
                        scrollbar_alpha,
                        active_tab.scroll_current,
                        max_history,
                        shell_rows as f32,
                        hover_close,
                        hover_max,
                        hover_min,
                        hover_settings,
                        last_activity_time_secs,
                        current_time,
                        active_tab.selection,
                        active_tab.hovered_url,
                        toast.as_ref().map(|(msg, t)| (msg.as_str(), *t)),
                        active_tab_index,
                        &tab_titles,
                        &active_tab_path,
                        context_menu_visible,
                        context_menu_x as f32,
                        context_menu_y as f32,
                        context_menu_hovered_idx,
                        context_menu_open_time_secs,
                        hovered_tab_index,
                        hovered_close_tab_index,
                        hover_new_tab,
                    );
                }

                let mut needs_redraw = false;

                for (idx, tab) in tabs.iter_mut().enumerate() {
                    let term = tab.terminal_state.lock();

                    // Sync scroll position if PTY printed output or terminal was resized
                    let actual_offset = term.display_offset();
                    if actual_offset != tab.last_actual_offset {
                        let diff_offset = actual_offset as f32 - tab.last_actual_offset as f32;
                        let pty_change = diff_offset - tab.last_scroll_diff as f32;
                        if pty_change.abs() > 0.01 {
                            tab.scroll_target += pty_change;
                            tab.scroll_current += pty_change;
                            if idx == active_tab_index {
                                needs_redraw = true;
                            }
                        }
                    }

                    let max_history = term.history_size() as f32;
                    tab.scroll_target = tab.scroll_target.clamp(0.0, max_history);

                    let diff = tab.scroll_target - tab.scroll_current;
                    let mut current_scroll_diff = 0;
                    if diff.abs() > 0.01 {
                        tab.scroll_current += diff * 0.15;

                        let target_offset = tab.scroll_current.round() as isize;
                        let scroll_diff = target_offset - term.display_offset() as isize;
                        if scroll_diff != 0 {
                            term.scroll(scroll_diff);
                            current_scroll_diff = scroll_diff;
                        }
                        if idx == active_tab_index {
                            renderer.lock().set_dirty(true);
                            needs_redraw = true;
                        }
                    } else {
                        tab.scroll_current = tab.scroll_target;
                    }

                    tab.last_actual_offset = term.display_offset();
                    tab.last_scroll_diff = current_scroll_diff;

                    // Sync render generation
                    let last_rg = rg.load(Ordering::Relaxed);
                    term.update_render_generation(&rg);
                    let current_rg = rg.load(Ordering::Relaxed);
                    if current_rg != last_rg {
                        tab.last_activity_time = std::time::Instant::now();
                        if idx == active_tab_index {
                            renderer.lock().set_dirty(true);
                            needs_redraw = true;
                        }
                    }
                }

                // Opacity animation of the scrollbar (uses active tab details)
                let v_width = renderer.lock().config.width as f64;
                let padding_top = get_padding_top(tabs.len());
                let scrollbar_top_margin = padding_top - 10.0;
                
                let now = std::time::Instant::now();
                let is_scrolling_recently = if let Some(scroll_time) = last_scroll_event_time {
                    if now.duration_since(scroll_time) < std::time::Duration::from_millis(1500) {
                        needs_redraw = true; // force redraw to check timer until it expires
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                let is_hovering = (current_mouse_y > scrollbar_top_margin as f64 && current_mouse_x > (v_width - 20.0)) || is_dragging_scrollbar;
                let target_alpha = if is_hovering || is_scrolling_recently { 1.0 } else { 0.0 };

                let alpha_diff = target_alpha - scrollbar_alpha;
                if alpha_diff.abs() > 0.01 {
                    scrollbar_alpha += alpha_diff * 0.15;
                    renderer.lock().set_dirty(true);
                    needs_redraw = true;
                } else {
                    scrollbar_alpha = target_alpha;
                }

                // Fade-in animation of the context menu
                if context_menu_visible {
                    if let Some(open_time) = context_menu_open_time {
                        if open_time.elapsed() < std::time::Duration::from_millis(80) {
                            renderer.lock().set_dirty(true);
                            needs_redraw = true;
                        }
                    }
                }

                // If active tab cursor is idle, force redraw to animate its pulsing
                let cursor_is_active = std::time::Instant::now().duration_since(tabs[active_tab_index].last_activity_time) < std::time::Duration::from_millis(500);
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

                // Control flow adjustment
                if needs_redraw {
                    target.set_control_flow(winit::event_loop::ControlFlow::Poll);
                } else if cursor_is_active {
                    let wake_time = tabs[active_tab_index].last_activity_time + std::time::Duration::from_millis(500);
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
    padding_top: f32,
) -> alacritty_terminal::index::Point {
    use alacritty_terminal::index::{Point, Line, Column};
    
    const PADDING_LEFT: f32 = 10.0;

    let col = (((mouse_x as f32 - PADDING_LEFT) / cell_width).floor() as i32)
        .clamp(0, shell_cols as i32 - 1) as usize;
    let row = ((mouse_y as f32 - padding_top) / cell_height - scroll_fraction).floor() as i32;
    
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
    padding_top: f32,
) -> Option<renderer::HoveredUrl> {
    if current_mouse_y <= padding_top as f64 || !ctrl_pressed {
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
        padding_top,
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

fn get_resize_direction(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    border: f64,
) -> Option<winit::window::ResizeDirection> {
    let top = y <= border;
    let bottom = y >= height - border;
    let left = x <= border;
    let right = x >= width - 3.0; // tighter hit area (3.0px) on the right edge to avoid scrollbar conflict

    if top && left {
        Some(winit::window::ResizeDirection::NorthWest)
    } else if top && right {
        Some(winit::window::ResizeDirection::NorthEast)
    } else if bottom && left {
        Some(winit::window::ResizeDirection::SouthWest)
    } else if bottom && right {
        Some(winit::window::ResizeDirection::SouthEast)
    } else if top {
        Some(winit::window::ResizeDirection::North)
    } else if bottom {
        Some(winit::window::ResizeDirection::South)
    } else if left {
        Some(winit::window::ResizeDirection::West)
    } else if right {
        Some(winit::window::ResizeDirection::East)
    } else {
        None
    }
}

fn resize_direction_to_cursor(dir: winit::window::ResizeDirection) -> winit::window::CursorIcon {
    match dir {
        winit::window::ResizeDirection::North => winit::window::CursorIcon::NResize,
        winit::window::ResizeDirection::South => winit::window::CursorIcon::SResize,
        winit::window::ResizeDirection::East => winit::window::CursorIcon::EResize,
        winit::window::ResizeDirection::West => winit::window::CursorIcon::WResize,
        winit::window::ResizeDirection::NorthEast => winit::window::CursorIcon::NeResize,
        winit::window::ResizeDirection::NorthWest => winit::window::CursorIcon::NwResize,
        winit::window::ResizeDirection::SouthEast => winit::window::CursorIcon::SeResize,
        winit::window::ResizeDirection::SouthWest => winit::window::CursorIcon::SwResize,
    }
}

fn get_context_menu_items(tabs: &[Tab], active_idx: usize) -> Vec<crate::renderer::ContextMenuItem> {
    let mut items = Vec::new();
    if tabs[active_idx].selection.is_some() {
        items.push(crate::renderer::ContextMenuItem::Copy);
    }
    items.push(crate::renderer::ContextMenuItem::Paste);
    items.push(crate::renderer::ContextMenuItem::Separator);
    items.push(crate::renderer::ContextMenuItem::NewTab);
    if tabs.len() > 1 {
        items.push(crate::renderer::ContextMenuItem::CloseTab);
    }
    items
}

fn get_context_menu_size(menu_items: &[crate::renderer::ContextMenuItem]) -> (f64, f64) {
    let mut h = 12.0f64; // 6px top + 6px bottom padding
    for item in menu_items {
        h += match item {
            crate::renderer::ContextMenuItem::Separator => 9.0,
            _ => 32.0,
        };
    }
    (180.0, h)
}

fn get_menu_item_at_y(menu_items: &[crate::renderer::ContextMenuItem], relative_y: f32) -> Option<usize> {
    if relative_y < 6.0 {
        return None;
    }
    let mut current_y = 6.0f32;
    for (idx, item) in menu_items.iter().enumerate() {
        let item_h = match item {
            crate::renderer::ContextMenuItem::Separator => 9.0f32,
            _ => 32.0f32,
        };
        if relative_y >= current_y && relative_y < current_y + item_h {
            match item {
                crate::renderer::ContextMenuItem::Separator => return None,
                _ => return Some(idx),
            }
        }
        current_y += item_h;
    }
    None
}

fn get_system_fonts() -> Vec<String> {
    let mut fonts = std::collections::BTreeSet::new();
    if let Ok(output) = std::process::Command::new("fc-list")
        .arg(":")
        .arg("family")
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                for family in line.split(',') {
                    let family = family.trim();
                    if !family.is_empty() && !family.starts_with('.') {
                        fonts.insert(family.to_string());
                    }
                }
            }
        }
    }
    if fonts.is_empty() {
        fonts.insert("monospace".to_string());
        fonts.insert("JetBrains Mono".to_string());
        fonts.insert("Fira Code".to_string());
    }
    fonts.into_iter().collect()
}