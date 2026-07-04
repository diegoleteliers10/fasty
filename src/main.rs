#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod config;
mod crash;
mod cross_window_drag;
mod event_listener;
mod git;
mod keybindings;
mod macos_maximize;
mod chrome_layout;
mod paths;
mod renderer;
mod secondary_window;
mod selection_classifier;
mod session;
mod snippets;
mod ssh;
mod terminal_state;
mod widgets;
mod window_context;

use std::sync::atomic::Ordering;
use std::sync::Arc;

const SCROLL_DECELERATION: f32 = 0.88;
const SCROLL_SNAP_THRESHOLD: f32 = 0.3;

use config::Config;
use renderer::{Renderer, Selection, RenderReason};
use selection_classifier::extract_token;
use terminal_state::{TerminalState, AppEvent};
use alacritty_terminal::grid::Dimensions;
use winit::{
    event::{ElementState, WindowEvent, MouseButton, MouseScrollDelta},
    event_loop::EventLoop,
    keyboard::Key,
    window::CursorGrabMode,
};
// macOS only: titlebar flags so the system draws native traffic lights.
#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;

/// Applies per-platform chrome: macOS gets a transparent, full-size titlebar
/// (native traffic lights); other platforms stay borderless.
fn with_platform_chrome(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    #[cfg(target_os = "macos")]
    {
        attrs
            .with_decorations(true)
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        attrs.with_decorations(false)
    }
}



fn get_login_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            if !shell.is_empty() {
                return shell;
            }
        }

        #[cfg(target_os = "linux")]
        {
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
        }

        if cfg!(target_os = "macos") {
            "/bin/zsh".to_string()
        } else {
            "/bin/bash".to_string()
        }
    }
}

#[cfg(target_os = "windows")]
fn no_window_cmd(program: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new(program);
    c.creation_flags(0x08000000);
    c
}

struct Tab {
    terminal_state: Arc<parking_lot::Mutex<TerminalState>>,
    cwd: Option<std::path::PathBuf>,
    scroll_current: f32,
    scroll_target: f32,
    selection: Option<Selection>,
    is_selecting_text: bool,
    selection_start_pos: Option<(f64, f64)>,
    hovered_url: Option<renderer::HoveredUrl>,
    hovered_url_text: Option<String>,
    hovered_hyperlink: Option<String>,
    pending_hyperlink_open: Option<String>,
    hyperlink_press_pos: Option<(f64, f64)>,
    is_dragging: bool,
    last_activity_time: std::time::Instant,
    last_actual_offset: usize,
    last_scroll_diff: isize,
    cursor_visible: bool,
    search_matches: Vec<renderer::SearchMatch>,
    search_query: String,
    search_visible: bool,
    search_current_idx: usize,
    custom_name: Option<String>,
    title_override: Option<String>,
    git_status: Option<GitStatus>,
    git_status_check_at: std::time::Instant,
    last_git_cwd: Option<std::path::PathBuf>,
    is_running: bool,
    last_exit_code: Option<i32>,
    /// Cursor blink phase index, reset on activity. Per-tab so that
    /// popped-out windows can blink independently of the main window.
    last_blink_index: u64,
    prompts: Vec<u64>,
    last_render_generation: u64,
}

/// Cached git status for a single tab. Populated by a background thread.
use git::GitStatus;

fn create_new_tab(
    executable: &str,
    exec_args: &[String],
    cwd: Option<&str>,
    scrollback: usize,
    font: config::FontConfig,
    cell_width: f32,
    cell_height: f32,
    cols: usize,
    rows: usize,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
) -> anyhow::Result<Tab> {
    let terminal_state = TerminalState::new(
        executable,
        exec_args,
        cwd,
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
        cwd: cwd.map(std::path::PathBuf::from),
        scroll_current: 0.0,
        scroll_target: 0.0,
        selection: None,
        is_selecting_text: false,
        selection_start_pos: None,
        hovered_url: None,
        hovered_url_text: None,
        hovered_hyperlink: None,
        pending_hyperlink_open: None,
        hyperlink_press_pos: None,
        is_dragging: false,
        last_activity_time: std::time::Instant::now(),
        last_actual_offset: 0,
        last_scroll_diff: 0,
        cursor_visible: true,
        search_matches: Vec::new(),
        search_query: String::new(),
        search_visible: false,
        search_current_idx: 0,
        custom_name: None,
        title_override: None,
        git_status: None,
        git_status_check_at: std::time::Instant::now(),
        last_git_cwd: None,
        is_running: false,
        last_exit_code: None,
        last_blink_index: 0,
        prompts: Vec::new(),
        last_render_generation: 0,
    })
}

fn get_padding_top(_tab_count: usize) -> f32 {
    58.0
}

fn mark_grid_dirty(renderer: &Arc<parking_lot::Mutex<Renderer>>, app_dirty: &mut bool) {
    let mut r = renderer.lock();
    r.set_dirty(true);
    r.grid_dirty = true;
    *app_dirty = true;
}

fn apply_font_size(
    config: &mut Config,
    new_size: f32,
    tabs: &mut Vec<Tab>,
    shell_cols: &mut usize,
    shell_rows: &mut usize,
    cell_width: &mut f32,
    cell_height: &mut f32,
    renderer: &Arc<parking_lot::Mutex<Renderer>>,
    window: &winit::window::Window,
    app_dirty: &mut bool,
) {
    if config.font.size == new_size {
        return;
    }
    config.font.size = new_size;
    if let Err(e) = config.save(&Config::get_active_config_path()) {
        tracing::warn!("config: save failed: {e}");
    }
    if let Err(e) = renderer.lock().update_font(&config.font.family, config.font.size) {
        tracing::error!("Failed to update renderer font: {:?}", e);
    }
    let cell_w = renderer.lock().cell_width();
    let cell_h = renderer.lock().cell_height();
    let physical_size = window.inner_size();
    let (cols, rows) = resize_all_tabs(tabs, physical_size.width, physical_size.height, cell_w, cell_h);
    *shell_cols = cols;
    *shell_rows = rows;
    *cell_width = cell_w;
    *cell_height = cell_h;
    mark_grid_dirty(renderer, app_dirty);
}

fn cursor_outside_tab_area(mx: f64, my: f64, vw: f64, vh: f64) -> bool {
    mx < 0.0 || mx >= vw || my < 0.0 || my >= vh || my >= 80.0
}

fn handle_popped_out_event(
    window_id: winit::window::WindowId,
    event: WindowEvent,
    popped: &mut std::collections::HashMap<winit::window::WindowId, window_context::WindowContext>,
    config: &Config,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    main_window_id: winit::window::WindowId,
    main_tabs: &mut Vec<crate::Tab>,
    main_active_tab_index: &mut usize,
    main_shell_cols: &mut usize,
    main_shell_rows: &mut usize,
    main_cell_width: f32,
    main_cell_height: f32,
    main_window: &winit::window::Window,
    hovered_window: &mut Option<winit::window::WindowId>,
    pending_new_window_from_drag: &mut bool,
    main_mouse_x: f64,
    shell: &str,
) {
    match event {
        WindowEvent::CloseRequested => {
            popped.remove(&window_id);
        }
        WindowEvent::CursorEntered { .. } => {
            *hovered_window = Some(window_id);
        }
        WindowEvent::CursorLeft { .. } => {
            if *hovered_window == Some(window_id) {
                *hovered_window = None;
            }
            // If the cursor left the window while dragging a tab into
            // cross_window_drag, create a new window immediately.
            if let Some(wc) = popped.get_mut(&window_id) {
                if wc.dragging_tab.is_some() && cross_window_drag::is_active() {
                    wc.dragging_tab = None;
                    wc.drag_threshold_passed = false;
                    wc.pending_pop_out = None;
                    *pending_new_window_from_drag = true;
                    {
                        let mut r = wc.renderer.lock();
                        r.set_dirty(true);
                        r.grid_dirty = true;
                    }
                    wc.window.request_redraw();
                }
            }
        }
        WindowEvent::Focused(focused) => {
            if let Some(wc) = popped.get_mut(&window_id) {
                if let Some(tab) = wc.tabs.get(wc.active_tab_index) {
                    let term_state = tab.terminal_state.lock();
                    let term = term_state.term().lock();
                    if term.mode().contains(alacritty_terminal::term::TermMode::FOCUS_IN_OUT) {
                        let seq = if focused { "\x1b[I" } else { "\x1b[O" };
                        drop(term);
                        term_state.write_to_pty(seq.as_bytes());
                    }
                }
            }
        }
        WindowEvent::CursorMoved { position, .. } => {
            let Some(wc) = popped.get_mut(&window_id) else { return; };
            wc.drag_current_x = position.x;
            wc.drag_current_y = position.y;

            let old_hover_close = wc.hover_close;
            let old_hover_max = wc.hover_max;
            let old_hover_min = wc.hover_min;
            let old_hover_settings = wc.hover_settings;
            let old_hovered_tab = wc.hovered_tab_index;
            let old_hovered_close = wc.hovered_close_tab_index;
            let old_hover_new = wc.hover_new_tab;

            let (vw, vh) = {
                let r = wc.renderer.lock();
                (r.config.width as f64, r.config.height as f64)
            };

            let vw_f = vw as f32;
            wc.hover_close = chrome_layout::close_rect(vw_f).contains(position.x, position.y);
            wc.hover_max = chrome_layout::max_rect(vw_f).contains(position.x, position.y);
            wc.hover_min = chrome_layout::min_rect(vw_f).contains(position.x, position.y);
            wc.hover_settings = chrome_layout::settings_rect(vw_f).contains(position.x, position.y);

            wc.hovered_tab_index = None;
            wc.hovered_close_tab_index = None;
            wc.hover_new_tab = false;

            if position.y >= 0.0 && position.y <= 40.0 {
                let tab_start_x = chrome_layout::tab_start_x() as f64;
                let path_center_x = vw / 2.0;
                let tab_area_max_x = path_center_x - 40.0;
                let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                let tabs_len = wc.tabs.len();
                let tab_width = if tabs_len > 0 {
                    (tab_area_width / tabs_len as f64).clamp(80.0, 160.0)
                } else {
                    160.0
                };

                let tabs_total_width = tabs_len as f64 * tab_width;
                if tabs_len > 1 && position.x >= tab_start_x && position.x < tab_start_x + tabs_total_width {
                    let idx = ((position.x - tab_start_x) / tab_width) as usize;
                    if idx < tabs_len {
                        wc.hovered_tab_index = Some(idx);
                        
                        let tab_x = tab_start_x + idx as f64 * tab_width;
                        let close_x = tab_x + tab_width - 30.0;
                        let close_min_x = close_x - 4.0;
                        let close_max_x = close_x + 20.0;
                        let close_min_y = 8.0;
                        let close_max_y = 32.0;
                        if position.x >= close_min_x && position.x <= close_max_x
                            && position.y >= close_min_y && position.y <= close_max_y
                        {
                            wc.hovered_close_tab_index = Some(idx);
                        }
                    }
                } else {
                    let new_tab_x = tab_start_x + tabs_total_width;
                    if position.x >= new_tab_x && position.x < new_tab_x + 32.0 {
                        wc.hover_new_tab = true;
                    }
                }
            }

            if wc.hover_close != old_hover_close
                || wc.hover_max != old_hover_max
                || wc.hover_min != old_hover_min
                || wc.hover_settings != old_hover_settings
                || wc.hovered_tab_index != old_hovered_tab
                || wc.hovered_close_tab_index != old_hovered_close
                || wc.hover_new_tab != old_hover_new
            {
                wc.renderer.lock().set_dirty(true);
                wc.window.request_redraw();
            }

            // Window resizing cursor icon setting
            let is_dragging_anything = wc.dragging_tab.is_some() || wc.tabs.get(wc.active_tab_index).map(|t| t.is_dragging || t.selection_start_pos.is_some()).unwrap_or(false);
            const RESIZE_BORDER_WIDTH: f64 = 8.0;
            if !is_dragging_anything {
                if let Some(dir) = get_resize_direction(position.x, position.y, vw, vh, RESIZE_BORDER_WIDTH) {
                    wc.window.set_cursor(resize_direction_to_cursor(dir));
                } else {
                    let padding_top = get_padding_top(wc.tabs.len()) as f64;
                    if position.y >= padding_top {
                        wc.window.set_cursor(winit::window::CursorIcon::Text);
                    } else {
                        wc.window.set_cursor(winit::window::CursorIcon::Default);
                    }
                }
            }

            if let Some(drag_idx) = wc.dragging_tab {
                if !wc.drag_threshold_passed && ((position.x - wc.drag_start_x).abs() > 5.0 || (position.y - wc.drag_start_y).abs() > 5.0) {
                    wc.drag_threshold_passed = true;
                }
                if wc.drag_threshold_passed {
                    // On Linux (X11/Wayland), an implicit pointer grab keeps
                    // CursorMoved events flowing with out-of-bounds coordinates
                    // while the mouse button is held. Track when the cursor has
                    // physically left so the MouseInput release handler treats
                    // it as a drop outside.
                    let cursor_outside_window =
                        position.x < 0.0 || position.x >= vw
                        || position.y < 0.0 || position.y >= vh;
                    if cursor_outside_window && *hovered_window == Some(window_id) {
                        *hovered_window = None;
                    } else if !cursor_outside_window && hovered_window.is_none() {
                        *hovered_window = Some(window_id);
                    }

                    if cursor_outside_window {
                        if wc.pending_pop_out.is_none() && wc.tabs.len() >= 2 {
                            wc.pending_pop_out = Some(drag_idx);
                            let tab = wc.tabs.remove(drag_idx);
                            *cross_window_drag::DRAG.lock() = Some(cross_window_drag::CrossWindowDrag {
                                source_window_id: window_id,
                                tab,
                            });
                            if wc.active_tab_index >= wc.tabs.len() && !wc.tabs.is_empty() {
                                wc.active_tab_index = wc.tabs.len() - 1;
                            }
                            let (cols, rows) = resize_all_tabs(&wc.tabs, wc.window.inner_size().width, wc.window.inner_size().height, wc.cell_width, wc.cell_height);
                            wc.shell_cols = cols;
                            wc.shell_rows = rows;
                        }
                        if cross_window_drag::is_active() {
                            wc.dragging_tab = None;
                            wc.drag_threshold_passed = false;
                            wc.pending_pop_out = None;
                            let _ = wc.window.set_cursor_grab(winit::window::CursorGrabMode::None);
                            *pending_new_window_from_drag = true;
                        }
                    } else if cursor_outside_tab_area(position.x, position.y, vw, vh) {
                        if wc.pending_pop_out.is_none() && wc.tabs.len() >= 2 {
                            wc.pending_pop_out = Some(drag_idx);
                            let tab = wc.tabs.remove(drag_idx);
                            *cross_window_drag::DRAG.lock() = Some(cross_window_drag::CrossWindowDrag {
                                source_window_id: window_id,
                                tab,
                            });
                            if wc.active_tab_index >= wc.tabs.len() && !wc.tabs.is_empty() {
                                wc.active_tab_index = wc.tabs.len() - 1;
                            }
                            let (cols, rows) = resize_all_tabs(&wc.tabs, wc.window.inner_size().width, wc.window.inner_size().height, wc.cell_width, wc.cell_height);
                            wc.shell_cols = cols;
                            wc.shell_rows = rows;
                        }
                    } else {
                        // Dragged back inside
                        if let Some(original_idx) = wc.pending_pop_out.take() {
                            if let Some(drag) = cross_window_drag::take() {
                                let insert_idx = original_idx.min(wc.tabs.len());
                                wc.tabs.insert(insert_idx, drag.tab);
                                wc.active_tab_index = insert_idx;
                                let (cols, rows) = resize_all_tabs(&wc.tabs, wc.window.inner_size().width, wc.window.inner_size().height, wc.cell_width, wc.cell_height);
                                wc.shell_cols = cols;
                                wc.shell_rows = rows;
                            }
                        }
                    }
                    wc.renderer.lock().set_dirty(true);
                    wc.window.request_redraw();
                }
            } else {
                if cross_window_drag::is_active() && *hovered_window == Some(window_id) {
                    wc.window.request_redraw();
                }
            }
        }
        WindowEvent::MouseInput { state, button, .. } => {
            use winit::event::ElementState;
            use winit::event::MouseButton;

            if state == ElementState::Pressed && button == MouseButton::Left {
                let Some(wc) = popped.get_mut(&window_id) else { return; };
                let vw = wc.window.inner_size().width as f64;
                let vh = wc.window.inner_size().height as f64;
                const RESIZE_BORDER_WIDTH: f64 = 8.0;
                if let Some(dir) = get_resize_direction(wc.drag_current_x, wc.drag_current_y, vw, vh, RESIZE_BORDER_WIDTH) {
                    let _ = wc.window.drag_resize_window(dir);
                    return;
                }

                if wc.drag_current_y <= 40.0 {
                    if wc.hover_close {
                        popped.remove(&window_id);
                        return;
                    } else if wc.hover_max {
                        macos_maximize::toggle_maximize(&wc.window, &mut wc.maximize_state);
                        return;
                    } else if wc.hover_min {
                        wc.window.set_minimized(true);
                        return;
                    }

                    let tab_start_x = chrome_layout::tab_start_x() as f64;
                    let path_center_x = vw / 2.0;
                    let tab_area_max_x = path_center_x - 40.0;
                    let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                    let tabs_len = wc.tabs.len();
                    let tab_width = if tabs_len > 0 {
                        (tab_area_width / tabs_len as f64).clamp(80.0, 160.0)
                    } else {
                        160.0
                    };

                    let tabs_total_width = tabs_len as f64 * tab_width;
                    if tabs_len > 1 && wc.drag_current_x >= tab_start_x && wc.drag_current_x < tab_start_x + tabs_total_width {
                        let clicked_tab_idx = ((wc.drag_current_x - tab_start_x) / tab_width) as usize;
                        if clicked_tab_idx < tabs_len {
                            let tab_x = tab_start_x + clicked_tab_idx as f64 * tab_width;
                            let close_x = tab_x + tab_width - 30.0;
                            let close_min_x = close_x - 4.0;
                            let close_max_x = close_x + 20.0;
                            let close_min_y = 8.0;
                            let close_max_y = 32.0;
                            let is_close_click = wc.drag_current_x >= close_min_x && wc.drag_current_x <= close_max_x
                                && wc.drag_current_y >= close_min_y && wc.drag_current_y <= close_max_y;

                            if is_close_click {
                                if wc.close_tab(clicked_tab_idx) {
                                    popped.remove(&window_id);
                                    return;
                                }
                            } else {
                                wc.dragging_tab = Some(clicked_tab_idx);
                                wc.drag_start_x = wc.drag_current_x;
                                wc.drag_start_y = wc.drag_current_y;
                                wc.drag_tab_offset = wc.drag_current_x - tab_x;
                                wc.drag_threshold_passed = false;
                                let _ = wc.window.set_cursor_grab(winit::window::CursorGrabMode::None);
                            }
                            wc.renderer.lock().set_dirty(true);
                            wc.window.request_redraw();
                            return;
                        }
                    }

                    // Check new tab button click
                    let new_tab_x = tab_start_x + tabs_total_width;
                    if tabs_len > 1 && wc.drag_current_x >= new_tab_x && wc.drag_current_x < new_tab_x + 32.0 {
                        let new_tab_count = wc.tabs.len() + 1;
                        let padding_top = get_padding_top(new_tab_count);
                        let physical_size = wc.window.inner_size();
                        const PADDING_LEFT: f32 = 10.0;
                        let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / wc.cell_width).floor().max(1.0)) as usize;
                        let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / wc.cell_height).floor().max(1.0)) as usize;
                        
                        match create_new_tab(
                            shell,
                            &[],
                            None,
                            config.scrollback,
                            config.font.clone(),
                            wc.cell_width,
                            wc.cell_height,
                            new_cols,
                            new_rows,
                            proxy.clone(),
                        ) {
                            Ok(new_tab) => {
                                wc.tabs.push(new_tab);
                                wc.active_tab_index = wc.tabs.len() - 1;
                                wc.shell_cols = new_cols;
                                wc.shell_rows = new_rows;
                            }
                            Err(e) => {
                                tracing::error!("Failed to create new tab in popped window: {:?}", e);
                            }
                        }
                        wc.renderer.lock().set_dirty(true);
                        wc.renderer.lock().grid_dirty = true;
                        wc.window.request_redraw();
                        return;
                    }

                    // Otherwise (blank space click), drag the window
                    if wc.drag_current_x < (chrome_layout::drag_max_x(vw as f32) as f64) {
                        let now = std::time::Instant::now();
                        let is_double_click = if let Some(last_time) = wc.last_click_time {
                            now.duration_since(last_time) < std::time::Duration::from_millis(300)
                        } else {
                            false
                        };
                        wc.last_click_time = Some(now);

                        if is_double_click {
                            macos_maximize::toggle_maximize(&wc.window, &mut wc.maximize_state);
                        } else {
                            let _ = wc.window.drag_window();
                        }
                    }
                }
            } else if state == ElementState::Released && button == MouseButton::Left {
                // To avoid E0499 double-mutable-borrow of `popped`, we scope the access to `wc`
                let (drag_idx, vw, _vh, drag_threshold_passed) = {
                    let Some(wc) = popped.get_mut(&window_id) else { return; };
                    let drag_idx = wc.dragging_tab;
                    let vw = wc.window.inner_size().width as f64;
                    let vh = wc.window.inner_size().height as f64;
                    let drag_threshold_passed = wc.drag_threshold_passed;
                    
                    wc.pending_pop_out = None;
                    wc.dragging_tab = None;
                    wc.drag_threshold_passed = false;

                    (drag_idx, vw, vh, drag_threshold_passed)
                };

                if let Some(drag_idx_val) = drag_idx {
                    if let Some(target_win) = *hovered_window {
                        // Take the tab from cross_window_drag, or remove it
                        // from the source window's tabs.
                        let tab_opt = cross_window_drag::take().map(|d| d.tab).or_else(|| {
                            let wc = popped.get_mut(&window_id)?;
                            if wc.tabs.len() >= 2 {
                                Some(wc.tabs.remove(drag_idx_val))
                            } else {
                                None
                            }
                        });
                        if target_win != window_id {
                            if let Some(tab) = tab_opt {
                                if target_win == main_window_id {
                                    let tab_start_x = chrome_layout::tab_start_x() as f64;
                                    let target_vw = main_window.inner_size().width as f64;
                                    let path_center_x = target_vw / 2.0;
                                    let tab_area_max_x = path_center_x - 40.0;
                                    let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                    let tabs_len = main_tabs.len() + 1;
                                    let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                                    let insert_idx = compute_drop_target(main_mouse_x, tab_start_x, tab_width, tabs_len);
                                    main_tabs.insert(insert_idx, tab);
                                    *main_active_tab_index = insert_idx;
                                    let (cols, rows) = resize_all_tabs(main_tabs, main_window.inner_size().width, main_window.inner_size().height, main_cell_width, main_cell_height);
                                    *main_shell_cols = cols;
                                    *main_shell_rows = rows;
                                    main_window.request_redraw();
                                } else if let Some(target_wc) = popped.get_mut(&target_win) {
                                    let tab_start_x = chrome_layout::tab_start_x() as f64;
                                    let target_vw = target_wc.window.inner_size().width as f64;
                                    let path_center_x = target_vw / 2.0;
                                    let tab_area_max_x = path_center_x - 40.0;
                                    let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                    let tabs_len = target_wc.tabs.len() + 1;
                                    let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                                    let insert_idx = compute_drop_target(target_wc.drag_current_x, tab_start_x, tab_width, tabs_len);
                                    
                                    target_wc.tabs.insert(insert_idx, tab);
                                    target_wc.active_tab_index = insert_idx;
                                    let (cols, rows) = resize_all_tabs(&target_wc.tabs, target_wc.window.inner_size().width, target_wc.window.inner_size().height, target_wc.cell_width, target_wc.cell_height);
                                    target_wc.shell_cols = cols;
                                    target_wc.shell_rows = rows;
                                    target_wc.renderer.lock().set_dirty(true);
                                    target_wc.window.request_redraw();
                                }
                            }
                        } else {
                            // Dropped on itself
                            let Some(wc) = popped.get_mut(&window_id) else { return; };
                            if let Some(tab) = tab_opt {
                                let tab_start_x = chrome_layout::tab_start_x() as f64;
                                let path_center_x = vw / 2.0;
                                let tab_area_max_x = path_center_x - 40.0;
                                let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                let tabs_len = wc.tabs.len() + 1;
                                let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                                let target = compute_drop_target(wc.drag_current_x, tab_start_x, tab_width, tabs_len);
                                let target_idx = target.min(wc.tabs.len());
                                wc.tabs.insert(target_idx, tab);
                                wc.active_tab_index = target_idx;
                                let (cols, rows) = resize_all_tabs(&wc.tabs, wc.window.inner_size().width, wc.window.inner_size().height, wc.cell_width, wc.cell_height);
                                wc.shell_cols = cols;
                                wc.shell_rows = rows;
                            } else if drag_threshold_passed {
                                let tab_start_x = chrome_layout::tab_start_x() as f64;
                                let path_center_x = vw / 2.0;
                                let tab_area_max_x = path_center_x - 40.0;
                                let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                let tabs_len = wc.tabs.len();
                                let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                                let target = compute_drop_target(wc.drag_current_x, tab_start_x, tab_width, tabs_len);
                                if target != drag_idx_val {
                                    let tab = wc.tabs.remove(drag_idx_val);
                                    wc.tabs.insert(target, tab);
                                    wc.active_tab_index = target;
                                    let (cols, rows) = resize_all_tabs(&wc.tabs, wc.window.inner_size().width, wc.window.inner_size().height, wc.cell_width, wc.cell_height);
                                    wc.shell_cols = cols;
                                    wc.shell_rows = rows;
                                }
                            } else {
                                wc.active_tab_index = drag_idx_val;
                            }
                        }
                    } else if cross_window_drag::is_active() {
                        // Cursor left the window — create a new window
                        *pending_new_window_from_drag = true;
                    } else if !drag_threshold_passed {
                        let Some(wc) = popped.get_mut(&window_id) else { return; };
                        wc.active_tab_index = drag_idx_val;
                    }
                }

                // If source window is empty, clean it up. Otherwise redraw it.
                if let Some(wc) = popped.get_mut(&window_id) {
                    if wc.tabs.is_empty() {
                        popped.remove(&window_id);
                        return;
                    }
                    let (cols, rows) = resize_all_tabs(&wc.tabs, wc.window.inner_size().width, wc.window.inner_size().height, wc.cell_width, wc.cell_height);
                    wc.shell_cols = cols;
                    wc.shell_rows = rows;
                    wc.renderer.lock().set_dirty(true);
                    wc.window.request_redraw();
                }
            }
        }
        WindowEvent::Resized(size) => {
            let Some(wc) = popped.get_mut(&window_id) else { return; };
            if size.width < 100 || size.height < 100 {
                return;
            }
            const PADDING_LEFT: f32 = 10.0;
            let padding_top = get_padding_top(wc.tabs.len());
            let padding_bottom = get_padding_bottom();
            let cell_w = wc.cell_width.max(1.0);
            let cell_h = wc.cell_height.max(1.0);
            let cols = (((size.width as f32 - PADDING_LEFT * 2.0) / cell_w).floor().max(1.0)) as usize;
            let rows = (((size.height as f32 - (padding_top + padding_bottom)) / cell_h).floor().max(1.0)) as usize;
            
            wc.shell_cols = cols;
            wc.shell_rows = rows;
            wc.pending_term_resize = Some((cols, rows));
            wc.pending_surface_resize = Some((size.width, size.height));
            wc.window.request_redraw();
        }
        WindowEvent::ScaleFactorChanged { .. } => {
            let Some(wc) = popped.get_mut(&window_id) else { return; };
            wc.window.request_redraw();
        }
        WindowEvent::KeyboardInput { event, .. } => {
            use winit::event::ElementState;
            let Some(wc) = popped.get_mut(&window_id) else { return; };
            let pressed = event.state == ElementState::Pressed;
            
            // Track Ctrl, Shift and Alt modifiers manually in case ModifiersChanged is missed
            match event.physical_key {
                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ControlLeft) |
                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ControlRight) => {
                    wc.ctrl_held = pressed;
                }
                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ShiftLeft) |
                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::ShiftRight) => {
                    wc.shift_held = pressed;
                }
                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::AltLeft) |
                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::AltRight) => {
                    wc.alt_held = pressed;
                }
                _ => {}
            }

            let active_idx = wc.active_tab_index.min(wc.tabs.len().saturating_sub(1));
            if let Some(tab) = wc.tabs.get_mut(active_idx) {
                if event.state == ElementState::Pressed {
                    if let Some(text) = &event.text {
                        let bytes = text.as_bytes();
                        if !bytes.is_empty() {
                            let _ = tab.terminal_state.lock().write_to_pty(bytes);
                        }
                    }
                    use winit::keyboard::{Key, NamedKey};
                    let key = &event.logical_key;
                    let bytes: Option<Vec<u8>> = match key {
                        Key::Named(NamedKey::Enter) => Some(b"\r".to_vec()),
                        Key::Named(NamedKey::Backspace) => Some(vec![0x7f]),
                        Key::Named(NamedKey::Tab) => Some(b"\t".to_vec()),
                        Key::Named(NamedKey::Escape) => Some(b"\x1b".to_vec()),
                        Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A".to_vec()),
                        Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".to_vec()),
                        Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C".to_vec()),
                        Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".to_vec()),
                        Key::Named(NamedKey::Home) => Some(b"\x1b[H".to_vec()),
                        Key::Named(NamedKey::End) => Some(b"\x1b[F".to_vec()),
                        Key::Named(NamedKey::Delete) => Some(b"\x1b[3~".to_vec()),
                        Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~".to_vec()),
                        Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~".to_vec()),
                        _ => None,
                    };
                    if let Some(b) = bytes {
                        let _ = tab.terminal_state.lock().write_to_pty(&b);
                    }
                }
            }
            wc.window.request_redraw();
        }
        WindowEvent::ModifiersChanged(modified) => {
            let Some(wc) = popped.get_mut(&window_id) else { return; };
            wc.modifiers = modified.state();
        }
        WindowEvent::MouseWheel { delta, .. } => {
            use winit::event::MouseScrollDelta;
            let Some(wc) = popped.get_mut(&window_id) else { return; };
            let tabs_len = wc.tabs.len();
            let active_idx = wc.active_tab_index.min(tabs_len.saturating_sub(1));
            if let Some(tab) = wc.tabs.get_mut(active_idx) {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => {
                        pos.y as f32 / wc.cell_height
                    }
                };

                let scroll_speed = 1.0f32;

                let delta_scroll = match delta {
                    MouseScrollDelta::LineDelta(_, _) => lines * scroll_speed,
                    MouseScrollDelta::PixelDelta(_) => lines,
                };

                let r = wc.renderer.lock();
                let v_width = r.config.width as f64;
                drop(r);
                let padding_top = get_padding_top(tabs_len);
                let is_in_term = wc.drag_current_y > padding_top as f64 && wc.drag_current_x <= (v_width - 20.0);

                let shift_active = wc.modifiers.shift_key() || wc.shift_held;

                if lines != 0.0 && !shift_active {
                    let term_state = tab.terminal_state.lock();
                    let (has_sgr, has_click) = {
                        let term_locked = term_state.term().lock();
                        let mode = term_locked.mode();
                        (
                            mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE),
                            mode.contains(alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK),
                        )
                    };

                    if is_in_term && (has_sgr || has_click) {
                        let padding_top = get_padding_top(tabs_len);
                        let col = (((wc.drag_current_x as f32 - 10.0) / wc.cell_width).floor() as i32)
                            .clamp(0, wc.shell_cols as i32 - 1) + 1;
                        let row = (((wc.drag_current_y as f32 - padding_top) / wc.cell_height).floor() as i32)
                            .clamp(0, wc.shell_rows as i32 - 1) + 1;

                        if has_sgr {
                            let seq = if lines > 0.0 {
                                format!("\x1b[<64;{};{}M", col, row)
                            } else {
                                format!("\x1b[<65;{};{}M", col, row)
                            };
                            term_state.write_to_pty(seq.as_bytes());
                        } else {
                            let btn = if lines > 0.0 { 96 } else { 97 };
                            let col_byte = (col.clamp(1, 223) as u8) + 32;
                            let row_byte = (row.clamp(1, 223) as u8) + 32;
                            term_state.write_to_pty(&[0x1b, b'[', b'M', btn, col_byte, row_byte]);
                        }
                    } else {
                        drop(term_state);
                        wc.scroll_velocity += delta_scroll;
                    }
                } else {
                    wc.scroll_velocity += delta_scroll;
                }
            }
            wc.window.request_redraw();
        }
        WindowEvent::RedrawRequested => {
            let Some(wc) = popped.get_mut(&window_id) else { return; };
            let active_idx = wc.active_tab_index.min(wc.tabs.len().saturating_sub(1));
            if let Some((cols, rows)) = wc.pending_term_resize.take() {
                for t in &wc.tabs {
                    t.terminal_state.lock().resize(cols, rows);
                }
            }
            if let Some((w, h)) = wc.pending_surface_resize.take() {
                wc.renderer.lock().resize(w, h);
            }
            if let Some(tab) = wc.tabs.get(active_idx) {
                let mut tab_titles = Vec::new();
                let mut active_tab_path = "fastty".to_string();
                for (idx, t) in wc.tabs.iter().enumerate() {
                    let title = if let Some(ref name) = t.custom_name {
                        name.clone()
                    } else {
                        let shell_pid = t.terminal_state.lock().shell_pid();
                        let agent = detect_tui_agent(shell_pid);
                        if let Some(ref agent_name) = agent {
                            let path_str = if let Some(pid) = t.terminal_state.lock().shell_pid() {
                                get_current_dir_shortened(pid)
                            } else {
                                None
                            };
                            let path_component = path_str.as_ref()
                                .map(|p| get_last_path_component(p))
                                .unwrap_or_else(|| "fastty".to_string());
                            format!("{} - {}", agent_name, path_component)
                        } else {
                            let path_str = if let Some(pid) = t.terminal_state.lock().shell_pid() {
                                get_current_dir_shortened(pid)
                            } else {
                                None
                            };
                            if let Some(ref path) = path_str {
                                get_last_path_component(path)
                            } else {
                                "bash".to_string()
                            }
                        }
                    };
                    
                    if idx == active_idx {
                        let path_str = if let Some(pid) = t.terminal_state.lock().shell_pid() {
                            get_current_dir_shortened(pid)
                        } else {
                            None
                        };
                        if let Some(ref path) = path_str {
                            active_tab_path = path.clone();
                        } else {
                            active_tab_path = "bash".to_string();
                        }
                    }
                    tab_titles.push(title);
                }

                let tab_running_states: Vec<bool> = wc.tabs.iter().map(|t| t.is_running).collect();
                let tab_exit_codes: Vec<Option<i32>> = wc.tabs.iter().map(|t| t.last_exit_code).collect();

                // Extract cwd info BEFORE acquiring the long-held terminal
                // lock for rendering — avoids same-thread deadlock on
                // parking_lot::Mutex (which is non-reentrant).
                let active_tab_cwd = tab.terminal_state.lock()
                    .shell_pid()
                    .and_then(|pid| std::fs::read_link(format!("/proc/{}/cwd", pid)).ok())
                    .or_else(|| tab.cwd.clone());
                let active_tab_git = tab.git_status.clone();

                let (v_width, v_height) = {
                    let r_lock = wc.renderer.lock();
                    (r_lock.config.width as f64, r_lock.config.height as f32)
                };

                let term = tab.terminal_state.lock();
                let max_history = term.history_size() as f32;
                let visible_rows = wc.shell_rows as f32;
                let term_ref: &crate::terminal_state::TerminalState = &*term;

                let (computed_bar_y, computed_bar_h) = poll_and_layout_bar(
                    &mut wc.bar_layout,
                    active_tab_cwd.as_deref(),
                    active_tab_git.as_ref(),
                    config.opacity,
                    v_width as f32,
                    v_height,
                );

                // Compute drop target for target window
                let adopting_drag = cross_window_drag::is_active() && *hovered_window == Some(window_id);
                let (dragging_tab_val, drop_target_val) = if adopting_drag {
                    let tab_start_x = chrome_layout::tab_start_x() as f64;
                    let path_center_x = v_width / 2.0;
                    let tab_area_max_x = path_center_x - 40.0;
                    let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                    let tabs_len = wc.tabs.len() + 1;
                    let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                    let target = compute_drop_target(wc.drag_current_x, tab_start_x, tab_width, tabs_len);
                    (Some(wc.tabs.len()), Some(target))
                } else if wc.drag_threshold_passed {
                    wc.dragging_tab.map(|_| {
                        let tab_start_x = chrome_layout::tab_start_x() as f64;
                        let path_center_x = v_width / 2.0;
                        let tab_area_max_x = path_center_x - 40.0;
                        let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                        let tabs_len = wc.tabs.len();
                        let tab_width = if tabs_len > 0 {
                            (tab_area_width / tabs_len as f64).clamp(80.0, 160.0)
                        } else {
                            160.0
                        };
                        compute_drop_target(wc.drag_current_x, tab_start_x, tab_width, tabs_len)
                    }).map(|t| (wc.dragging_tab, Some(t))).unwrap_or((wc.dragging_tab, None))
                } else {
                    (wc.dragging_tab, None)
                };

                let inputs = renderer::RenderInputs {
                    ligatures: config.font.ligatures,
                    scroll_current: tab.scroll_current,
                    history_size: max_history,
                    visible_rows,
                    selection: tab.selection,
                    active_tab_index: active_idx,
                    tab_titles: &tab_titles,
                    tab_running_states: &tab_running_states,
                    tab_exit_codes: &tab_exit_codes,
                    active_tab_path: &active_tab_path,
                    dragging_tab: dragging_tab_val,
                    drag_current_x: wc.drag_current_x as f32,
                    drag_tab_offset: wc.drag_tab_offset as f32,
                    drop_target_idx: drop_target_val,
                    git_status: tab.git_status.as_ref(),
                    opacity: config.opacity,
                    hover_close: wc.hover_close,
                    hover_max: wc.hover_max,
                    hover_min: wc.hover_min,
                    hover_settings: wc.hover_settings,
                    hovered_tab_index: wc.hovered_tab_index,
                    hovered_close_tab_index: wc.hovered_close_tab_index,
                    hover_new_tab: wc.hover_new_tab,
                    bar_segments: &wc.bar_layout.laid_out,
                    bar_y: computed_bar_y,
                    bar_h: computed_bar_h,
                    ..renderer::RenderInputs::default()
                };

                let mut r = wc.renderer.lock();
                r.set_dirty(true);
                r.grid_dirty = true;
                r.render(renderer::RenderReason::GridChanged, term_ref, tab.cursor_visible, inputs);
            }
        }
        _ => {}
    }
}


fn build_bar_layout(config: &Config) -> widgets::BarLayout {
    if config.bottombar.widgets.is_empty() {
        widgets::BarLayout::new(vec![Box::new(
            widgets::builtin::git::GitWidget::new(widgets::Align::Left, None),
        )])
    } else {
        widgets::BarLayout::from_specs(&config.bottombar.widgets)
    }
}

fn poll_and_layout_bar(
    layout: &mut widgets::BarLayout,
    active_tab_cwd: Option<&std::path::Path>,
    active_tab_git: Option<&GitStatus>,
    opacity: f32,
    viewport_width: f32,
    viewport_height: f32,
) -> (f32, f32) {
    let now = std::time::Instant::now();
    let ctx = widgets::WidgetContext {
        active_tab_cwd,
        active_tab_git,
        opacity,
    };

    for w in layout.widgets.iter_mut() {
        if now.duration_since(w.last_poll()) >= w.poll_interval() {
            w.set_last_poll(now);
            w.poll(&ctx);
        }
    }

    const BB_H: f32 = 20.0;
    const SCROLLBAR_COL_W: f32 = 10.0;
    let bar_y = viewport_height - BB_H;
    let right_edge = viewport_width - SCROLLBAR_COL_W;
    let mid_x = viewport_width * 0.5;

    let mut left_x: f32 = 8.0;
    let mut right_x: f32 = right_edge - 8.0;
    let mut left_out: Vec<widgets::LaidOutWidget> = Vec::new();
    let mut right_out: Vec<widgets::LaidOutWidget> = Vec::new();

    for (idx, w) in layout.widgets.iter_mut().enumerate() {
        let segments = w.render(&widgets::WidgetContext {
            active_tab_cwd,
            active_tab_git,
            opacity,
        });
        if segments.is_empty() {
            continue;
        }
        let tooltip = w.tooltip();

        let measure = |text: &str, scale: f32| -> f32 {
            text.chars().fold(0.0f32, |acc, c| {
                if c == ' ' { return acc + 7.0 * scale; }
                acc + 7.0 * scale + 1.0
            })
        };
        let scale_approx: f32 = 12.0 / 14.0;
        let width = segments.iter().map(|s| measure(&s.text, scale_approx)).sum::<f32>();

        match w.align() {
            widgets::Align::Left => {
                if left_x + width >= mid_x { continue; }
                let rect = widgets::Rect { x: left_x, y: bar_y, w: width, h: BB_H };
                left_out.push(widgets::LaidOutWidget {
                    widget_index: idx,
                    rect,
                    segments,
                    tooltip: tooltip.clone(),
                });
                left_x = rect.x + width + 12.0;
            }
            widgets::Align::Right => {
                if right_x - width <= mid_x { continue; }
                let rect = widgets::Rect {
                    x: right_x - width,
                    y: bar_y,
                    w: width,
                    h: BB_H,
                };
                right_out.push(widgets::LaidOutWidget {
                    widget_index: idx,
                    rect,
                    segments,
                    tooltip,
                });
                right_x = rect.x - 12.0;
            }
        }
    }

    left_out.extend(right_out);
    let hit_rects = left_out.iter().map(|lo| lo.rect).collect();
    layout.laid_out = left_out;
    layout.hit_rects = hit_rects;
    (bar_y, BB_H)
}

fn get_padding_bottom() -> f32 {
    10.0 + 20.0 // PADDING_BOTTOM + BOTTOMBAR_HEIGHT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandAction {
    NewTab,
    CloseTab,
    NewWindow,
    NextTab,
    PrevTab,
    OpenSettings,
    OpenSearch,
    ReloadConfig,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    SnapToBottom,
    RenameTab,
}

fn build_palette_commands() -> Vec<(String, CommandAction)> {
    let mut cmds = Vec::new();
    cmds.extend(vec![
        ("new tab".to_string(), CommandAction::NewTab),
        ("close tab".to_string(), CommandAction::CloseTab),
        ("new window".to_string(), CommandAction::NewWindow),
        ("next tab".to_string(), CommandAction::NextTab),
        ("previous tab".to_string(), CommandAction::PrevTab),
        ("open settings".to_string(), CommandAction::OpenSettings),
        ("open search".to_string(), CommandAction::OpenSearch),
        ("reload config".to_string(), CommandAction::ReloadConfig),
        ("increase font size".to_string(), CommandAction::IncreaseFontSize),
        ("decrease font size".to_string(), CommandAction::DecreaseFontSize),
        ("reset font size".to_string(), CommandAction::ResetFontSize),
        ("snap to bottom".to_string(), CommandAction::SnapToBottom),
        ("rename tab".to_string(), CommandAction::RenameTab),
    ]);
    cmds
}

fn filter_palette(commands: &[(String, CommandAction)], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return (0..commands.len()).collect();
    }
    commands.iter().enumerate()
        .filter(|(_, (label, _))| label.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

fn compute_palette_filtered(commands: &[(String, CommandAction)], query: &str) -> Vec<String> {
    filter_palette(commands, query).into_iter().map(|i| commands[i].0.clone()).collect()
}

fn filter_ssh_hosts(hosts: &[ssh::SshHost], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return (0..hosts.len()).collect();
    }
    hosts.iter().enumerate()
        .filter(|(_, h)| {
            h.name.to_lowercase().contains(&q)
                || h.hostname.to_lowercase().contains(&q)
                || h.user.to_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

fn compute_ssh_filtered(hosts: &[ssh::SshHost], query: &str) -> Vec<String> {
    filter_ssh_hosts(hosts, query).into_iter().map(|i| hosts[i].display()).collect()
}

fn compute_drop_target(
    mouse_x: f64,
    tab_start_x: f64,
    tab_width: f64,
    tabs_len: usize,
) -> usize {
    if tabs_len <= 1 {
        return 0;
    }
    let remaining = tabs_len - 1;
    let pos = mouse_x - tab_start_x;
    for i in 0..remaining {
        let midpoint = (i as f64 + 0.5) * tab_width;
        if pos < midpoint {
            return i;
        }
    }
    remaining
}

fn resize_all_tabs(
    tabs: &[Tab],
    width: u32,
    height: u32,
    cell_width: f32,
    cell_height: f32,
) -> (usize, usize) {
    const PADDING_LEFT: f32 = 10.0;
    let padding_top = get_padding_top(tabs.len());
    let padding_bottom = get_padding_bottom();
    let cell_w = cell_width.max(1.0);
    let cell_h = cell_height.max(1.0);
    let cols = (((width as f32 - PADDING_LEFT * 2.0) / cell_w).floor().max(1.0)) as usize;
    let rows = (((height as f32 - (padding_top + padding_bottom)) / cell_h).floor().max(1.0)) as usize;

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

fn shorten_path(path: &std::path::Path) -> String {
    let path_str = path.to_string_lossy().into_owned();
    if let Ok(home) = std::env::var("HOME") {
        if path_str == home {
            return "~".to_string();
        }
        if let Some(stripped) = path_str.strip_prefix(&home) {
            if stripped.starts_with('/') {
                return format!("~{}", stripped);
            }
        }
    }
    path_str
}

fn tail_n(path: &std::path::Path, n: usize) -> String {
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.len() <= n {
        parts.join("/")
    } else {
        parts[parts.len() - n..].join("/")
    }
}

/// Build a short display label for a path that disambiguates against other
/// paths in the same list. Starts with the basename; if any other path has
/// the same tail-N, adds another component. Caps at 3 components.
fn display_short(path: &std::path::Path, siblings: &[std::path::PathBuf]) -> String {
    for depth in 1..=3usize {
        let candidate = tail_n(path, depth);
        let ambig = siblings.iter().any(|s| s != path && tail_n(s, depth) == candidate);
        if !ambig {
            return candidate;
        }
    }
    tail_n(path, 3)
}

fn collect_unique_project_dirs(tabs: &[Tab]) -> Vec<std::path::PathBuf> {
    use std::collections::HashSet;
    use std::path::PathBuf;
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    for t in tabs.iter().rev() {
        if let Some(p) = tab_live_cwd(t) {
            let canon = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if seen.insert(canon.clone()) {
                out.push(canon);
                continue;
            }
        }
        if let Some(p) = t.cwd.as_ref() {
            let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            if seen.insert(canon.clone()) {
                out.push(canon);
            }
        }
    }
    out
}

fn filter_project_dirs(items: &[std::path::PathBuf], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    items.iter().enumerate()
        .filter(|(_, p)| shorten_path(p).to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

fn compute_project_filtered(items: &[std::path::PathBuf], query: &str) -> Vec<String> {
    filter_project_dirs(items, query)
        .into_iter()
        .map(|i| display_short(&items[i], items))
        .collect()
}

fn filter_worktrees(items: &[git::Worktree], query: &str) -> Vec<usize> {
    let q = query.strip_prefix('+').unwrap_or(query).to_lowercase();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    items.iter().enumerate()
        .filter(|(_, w)| {
            shorten_path(&w.path).to_lowercase().contains(&q)
                || w.short_branch().to_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

fn compute_worktree_filtered(
    items: &[git::Worktree],
    query: &str,
    toplevel: Option<&std::path::Path>,
) -> Vec<String> {
    let paths: Vec<std::path::PathBuf> = items.iter().map(|w| w.path.clone()).collect();
    let mut out: Vec<String> = filter_worktrees(items, query)
        .into_iter()
        .map(|i| display_worktree(&items[i], &paths))
        .collect();
    let create_branch = query.strip_prefix('+').map(str::trim).unwrap_or("");
    if query.starts_with('+') && !create_branch.is_empty() {
        if let Some(t) = toplevel {
            out.push(proposed_worktree_label(t, create_branch, &paths));
        }
    }
    out
}

fn display_worktree(w: &git::Worktree, all_paths: &[std::path::PathBuf]) -> String {
    let label = display_short(&w.path, all_paths);
    let branch = w.short_branch();
    format!("{:<32}  {:<24}  {}", label, branch, w.short_commit())
}

fn proposed_worktree_label(toplevel: &std::path::Path, branch: &str, all_paths: &[std::path::PathBuf]) -> String {
    let parent = match toplevel.parent() {
        Some(p) => p,
        None => return format!("+{}", branch),
    };
    let dir_name = toplevel
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    let new_path = parent.join(format!("{}-{}", dir_name, git::sanitize_branch_for_path(branch)));
    let label = display_short(&new_path, all_paths);
    format!("+ create  {:<32}  {:<24}  new", label, branch)
}

fn tab_live_cwd(tab: &Tab) -> Option<std::path::PathBuf> {
    let pid = tab.terminal_state.lock().shell_pid()?;
    std::fs::read_link(format!("/proc/{}/cwd", pid)).ok()
}

fn read_prompt_prefix(terminal_state: &Arc<parking_lot::Mutex<TerminalState>>, cols: usize) -> String {
    let state = terminal_state.lock();
    let term_guard = state.term().lock();
    let grid = term_guard.grid();
    let point = grid.cursor.point;
    let line_idx = point.line.0;
    if !(0..grid.screen_lines() as i32).contains(&line_idx) {
        return String::new();
    }
    let row = &grid[alacritty_terminal::index::Line(line_idx)];
    let end_col = (point.column.0 as usize).min(cols);
    let mut s = String::with_capacity(end_col);
    for c in 0..end_col {
        let ch = row[alacritty_terminal::index::Column(c)].c;
        if ch == '\0' { continue; }
        s.push(ch);
    }
    s
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

fn get_current_version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

const TUI_AGENTS: &[&str] = &["claude", "opencode", "hermes", "forge", "agy"];

#[cfg(not(target_os = "windows"))]
fn detect_tui_agent(shell_pid: Option<u32>) -> Option<String> {
    let pid = shell_pid?;
    let children_path = format!("/proc/{}/task/{}/children", pid, pid);
    let children = std::fs::read_to_string(&children_path).ok()?;
    for child_pid_str in children.split_whitespace() {
        let child_pid: u32 = match child_pid_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let cmdline_path = format!("/proc/{}/cmdline", child_pid);
        let cmdline = match std::fs::read(&cmdline_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let cmd_str = String::from_utf8_lossy(&cmdline);
        let first_arg = cmd_str.split('\0').next().unwrap_or("");
        let cmd_name = std::path::Path::new(first_arg)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        for agent in TUI_AGENTS {
            if cmd_name == *agent {
                return Some(agent.to_string());
            }
        }
        // Check grandchildren (one level deeper)
        let gc_path = format!("/proc/{}/task/{}/children", child_pid, child_pid);
        if let Ok(gc) = std::fs::read_to_string(&gc_path) {
            for gc_pid_str in gc.split_whitespace() {
                let gc_pid: u32 = match gc_pid_str.parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let gc_cmdline_path = format!("/proc/{}/cmdline", gc_pid);
                if let Ok(gc_cmdline) = std::fs::read(&gc_cmdline_path) {
                    let gc_str = String::from_utf8_lossy(&gc_cmdline);
                    let gc_first = gc_str.split('\0').next().unwrap_or("");
                    let gc_name = std::path::Path::new(gc_first)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    for agent in TUI_AGENTS {
                        if gc_name == *agent {
                            return Some(agent.to_string());
                        }
                    }
                    // Check great-grandchildren (needed for Node/Bun agents like claude
                    // which may spawn: shell → node/bun → claude-worker)
                    let ggc_path = format!("/proc/{}/task/{}/children", gc_pid, gc_pid);
                    if let Ok(ggc) = std::fs::read_to_string(&ggc_path) {
                        for ggc_pid_str in ggc.split_whitespace() {
                            let ggc_pid: u32 = match ggc_pid_str.parse() {
                                Ok(p) => p,
                                Err(_) => continue,
                            };
                            let ggc_cmdline_path = format!("/proc/{}/cmdline", ggc_pid);
                            if let Ok(ggc_cmdline) = std::fs::read(&ggc_cmdline_path) {
                                let ggc_str = String::from_utf8_lossy(&ggc_cmdline);
                                let ggc_first = ggc_str.split('\0').next().unwrap_or("");
                                let ggc_name = std::path::Path::new(ggc_first)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("");
                                for agent in TUI_AGENTS {
                                    if ggc_name == *agent {
                                        return Some(agent.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_tui_agent(_shell_pid: Option<u32>) -> Option<String> {
    None
}

/// Detect git status for the given working directory.
/// Runs `git status --porcelain=v2 --branch` and parses the output.
/// Returns None if the directory is not inside a git repo or git is missing.
fn detect_git_status(cwd: &std::path::Path) -> Option<GitStatus> {
    git::fetch_git_info(cwd)
}


#[derive(Debug)]
struct FasttyArgs {
    command: Option<Vec<String>>,   // -e cmd arg1 arg2...
    working_dir: Option<String>,    // -d /path/to/dir
    title: Option<String>,          // --title "My Window"
    paths: bool,                    // --paths
}

impl FasttyArgs {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::parse_from(args)
    }

    fn parse_from(args: Vec<String>) -> Self {
        let mut result = Self {
            command: None,
            working_dir: None,
            title: None,
            paths: false,
        };
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-e" | "--command" => {
                    // Everything after -e is the command + its args
                    if i + 1 < args.len() {
                        result.command = Some(args[i+1..].to_vec());
                    }
                    break; // -e consumes the rest
                }
                "-d" | "--working-dir" => {
                    if i + 1 < args.len() {
                        result.working_dir = Some(args[i+1].clone());
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--title" => {
                    if i + 1 < args.len() {
                        result.title = Some(args[i+1].clone());
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--paths" => {
                    result.paths = true;
                    i += 1;
                }
                _ => { i += 1; }
            }
        }
        result
    }
}

#[allow(deprecated, unused_mut)]
fn main() -> anyhow::Result<()> {
    std::env::set_var("TERM", "xterm-256color");
    std::env::set_var("COLORTERM", "truecolor");
    std::env::set_var("TERM_PROGRAM", "fastty");
    let app_version = get_current_version();
    std::env::set_var("TERM_PROGRAM_VERSION", app_version.trim_start_matches('v'));

    let _ = std::fs::remove_file("/tmp/fastty-update-done");

    #[cfg(target_os = "windows")]
    {
        if let Ok(current_exe) = std::env::current_exe() {
            let old_exe = current_exe.with_extension("exe.old");
            if old_exe.exists() {
                let _ = std::fs::remove_file(old_exe);
            }
        }
    }

    paths::init()?;

    crash::install_hook();

    tracing_subscriber::fmt()
        .with_env_filter("warn,fastty=info")
        .init();

    let mut config = Config::load().unwrap_or_else(|e| {
        tracing::warn!("config: startup load failed, using defaults: {e}");
        Config::default()
    });
    config::load_custom_themes();
    keybindings::init_resolver(config.keybindings.clone());

    let fastty_args = FasttyArgs::parse();

    if fastty_args.paths {
        let dirs = paths::get();
        println!("config_dir: {}", dirs.config_dir.display());
        println!("data_dir: {}", dirs.data_dir.display());
        println!("state_dir: {}", dirs.state_dir.display());
        println!("cache_dir: {}", dirs.cache_dir.display());
        std::process::exit(0);
    }

    // Resolve what to spawn
    let (executable, exec_args) = match &fastty_args.command {
        Some(cmd) => {
            // -e was passed — spawn command directly
            let exe = cmd[0].clone();
            let args = cmd[1..].to_vec();
            (exe, args)
        }
        None => {
            // No -e — spawn default shell
            let shell = if let Some(ref s) = config.shell {
                s.clone()
            } else {
                get_login_shell()
            };
            (shell, vec![])
        }
    };

    // Resolve working directory
    let cwd = fastty_args.working_dir.clone();

    // Window title
    let window_title = fastty_args.title
        .clone()
        .unwrap_or_else(|| {
            match &fastty_args.command {
                Some(cmd) => cmd[0].clone(),  // "arch-update", "htop", etc
                None => "fastty".to_string(),
            }
        });

    let auto_close = fastty_args.command.is_some();

    // Resolve default shell for subsequent tabs
    let shell = if let Some(ref s) = config.shell {
        s.clone()
    } else {
        get_login_shell()
    };

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    #[cfg(target_os = "windows")]
    let window = event_loop.create_window(winit::window::WindowAttributes::default()
        .with_title(&window_title)
        .with_decorations(false)
        .with_transparent(true)
        .with_visible(false)
        .with_inner_size(winit::dpi::LogicalSize::new(800.0, 520.0)))?;

    // macOS + Linux: platform chrome via `with_platform_chrome`.
    // Windows has its own block above (it uniquely hides until first show).
    #[cfg(not(target_os = "windows"))]
    let window = event_loop.create_window(with_platform_chrome(
        winit::window::WindowAttributes::default()
            .with_title(&window_title)
            .with_transparent(true)
            .with_inner_size(winit::dpi::LogicalSize::new(800.0, 520.0)),
    ))?;

    // Load and set the window icon at runtime for the taskbar/desktop bar
    if let Ok(icon_image) = image::load_from_memory(include_bytes!("../assets/fasttyIcon.png")) {
        let icon_image = icon_image.into_rgba8();
        let (width, height) = icon_image.dimensions();
        let rgba = icon_image.into_raw();
        if let Ok(icon) = winit::window::Icon::from_rgba(rgba, width, height) {
            window.set_window_icon(Some(icon));
        }
    }

    let window_arc = Arc::new(window);
    let window_for_renderer = window_arc.as_ref();
    let renderer = pollster::block_on(Renderer::new(window_for_renderer, &config.font.family, config.font.size))?;
    let mut cell_width = renderer.cell_width();
    let mut cell_height = renderer.cell_height();




    let viewport_width = renderer.config.width as f32;
    const PADDING_LEFT: f32 = 10.0;

    let viewport_height = renderer.config.height as f32;
    let mut shell_cols = ((viewport_width - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0) as usize;
    let mut shell_rows = ((viewport_height - (get_padding_top(1) + get_padding_bottom())) / cell_height).floor().max(1.0) as usize;
    let proxy = event_loop.create_proxy();
    {
        let watcher_proxy = proxy.clone();
        let watch_path = Config::get_active_config_path();
        if let Err(e) = config::start_config_watcher(watch_path, move || {
            let _ = watcher_proxy.send_event(AppEvent::ConfigChanged);
        }) {
            tracing::warn!("config: file watcher unavailable, live-reload disabled: {e}");
        }
    }
    snippets::load();
    {
        let snippets_proxy = proxy.clone();
        if let Err(e) = snippets::start_watcher(move || {
            snippets::load();
            let _ = snippets_proxy.send_event(AppEvent::ConfigChanged);
        }) {
            tracing::warn!("snippets: file watcher unavailable, live-reload disabled: {e}");
        }
    }
    let initial_tab = create_new_tab(
        &executable,
        &exec_args,
        cwd.as_deref(),
        config.scrollback,
        config.font.clone(),
        cell_width,
        cell_height,
        shell_cols,
        shell_rows,
        proxy.clone(),
    )?;

    let mut active_tab_index = 0usize;
    let mut last_active_tab_index = active_tab_index;
    let mut hovered_window: Option<winit::window::WindowId> = None;
    let mut pending_new_window_from_drag = false;

    let mut pending_pop_out: Option<usize> = None;
    let mut popped_out_windows: std::collections::HashMap<
        winit::window::WindowId,
        window_context::WindowContext,
    > = std::collections::HashMap::new();
    // Windows saved in the session that should be re-spawned as popped-out
    // windows. We can't create them here (no `ActiveEventLoop` yet) so we
    // process the queue in `AboutToWait`.
    let mut pending_session_windows: Vec<session::WindowSession> = Vec::new();

    const BB_H: f32 = 20.0;
    let mut bar_layout = build_bar_layout(&config);
    let mut bar_y: f32 = 0.0;
    let mut bar_h: f32 = BB_H;
    let mut tabs = if config.session_restore && fastty_args.command.is_none() && fastty_args.working_dir.is_none() {
        match session::load() {
            Some(s) if !s.windows.is_empty() => {
                let active_idx = s.active_window.min(s.windows.len() - 1);
                let first_window = &s.windows[active_idx];
                let mut restored = Vec::new();
                for tab_info in &first_window.tabs {
                    let tab_cwd = tab_info.cwd.as_ref().and_then(|p| p.to_str());
                    match create_new_tab(
                        &shell, &[], tab_cwd, config.scrollback, config.font.clone(),
                        cell_width, cell_height, shell_cols, shell_rows, proxy.clone(),
                    ) {
                        Ok(t) => restored.push(t),
                        Err(e) => tracing::warn!("session: failed to restore tab: {e:?}"),
                    }
                }
                // Queue the other windows regardless of whether the first window
                // had recoverable tabs, so a partial failure on window 0 doesn't
                // drop the rest of the saved session.
                for (i, w) in s.windows.iter().enumerate() {
                    if i != active_idx {
                        pending_session_windows.push(w.clone());
                    }
                }
                if !restored.is_empty() {
                    active_tab_index = first_window.active_tab.min(restored.len() - 1);
                    restored
                } else {
                    vec![initial_tab]
                }
            }
            _ => vec![initial_tab],
        }
    } else {
        vec![initial_tab]
    };
    let renderer = Arc::new(parking_lot::Mutex::new(renderer));
    let mut modifiers = winit::keyboard::ModifiersState::default();
    let mut ctrl_held = false;
    let mut shift_held = false;
    let mut alt_held = false;

    let window_for_redraw = window_arc.clone();
    
    let mut last_cursor_y = 0.0f64;

    let mut scrollbar_alpha = 0.0f32;
    let mut is_dragging_scrollbar = false;
    let mut scrollbar_drag_offset_y = 0.0f32;
    let mut current_mouse_x = 0.0f64;
    let mut current_mouse_y = 0.0f64;
    let mut last_click_time: Option<std::time::Instant> = None;
    let mut maximize_state = macos_maximize::MaximizeState::default();
    let mut last_term_click_time: Option<std::time::Instant> = None;
    let mut last_term_click_cell: Option<(i32, usize)> = None;
    let mut toast: Option<(String, std::time::Instant, u64)> = None;

    let mut command_palette_visible = false;
    let mut command_palette_query: String = String::new();
    let mut command_palette_selected: usize = 0;
    let mut command_palette_scroll: usize = 0;
    let mut palette_commands: Vec<(String, CommandAction)> = build_palette_commands();

    let mut ssh_hosts: Vec<ssh::SshHost> = ssh::parse_ssh_config();
    let mut ssh_picker_visible = false;
    let mut ssh_picker_query: String = String::new();
    let mut ssh_picker_selected: usize = 0;

    let mut project_jumper_items: Vec<std::path::PathBuf> = Vec::new();
    let mut project_jumper_visible = false;
    let mut project_jumper_query: String = String::new();
    let mut project_jumper_selected: usize = 0;

    let mut worktree_items: Vec<git::Worktree> = Vec::new();
    let mut worktree_toplevel: Option<std::path::PathBuf> = None;
    let mut worktree_picker_visible = false;
    let mut worktree_picker_query: String = String::new();
    let mut worktree_picker_selected: usize = 0;

    let start_time = std::time::Instant::now();
    let mut clipboard: Option<arboard::Clipboard> = None;
    let mut context_menu_visible = false;
    let mut context_menu_is_about = false;
    let mut context_menu_x = 0.0f64;
    let mut context_menu_y = 0.0f64;
    let mut context_menu_hovered_idx: Option<usize> = None;
    let mut context_menu_classification: Option<selection_classifier::Classification> = None;
    let mut context_menu_items: Vec<renderer::ContextMenuItem> = Vec::new();
    let mut context_menu_scroll_y = 0.0f64;
    let mut context_menu_open_time: Option<std::time::Instant> = None;
    let mut context_menu_open_time_secs: Option<f32> = None;
    let mut last_scroll_event_time: Option<std::time::Instant> = None;
    let mut mouse_down_button: Option<winit::event::MouseButton> = None;

    // Bell, command timing, and tooltip state
    let mut bell_flash_time: Option<std::time::Instant> = None;
    let mut last_command_duration: Option<(u128, Option<i32>)> = None;
    let mut last_command_duration_display_time: Option<std::time::Instant> = None;

    // Scroll momentum
    let mut scroll_velocity: f32 = 0.0;

    // Hover states for main window topbar buttons
    let mut hover_close = false;
    let mut hover_max = false;
    let mut hover_min = false;
    let mut hover_settings = false;
    let mut hover_update = false;
    let mut hovered_tab_index: Option<usize> = None;
    let mut hovered_close_tab_index: Option<usize> = None;
    let mut hover_new_tab = false;
    let mut dragging_tab: Option<usize> = None;
    let mut drag_start_x: f64 = 0.0;
    let mut drag_start_y: f64 = 0.0;
    let mut drag_tab_offset: f64 = 0.0;
    let mut drag_current_x: f64 = 0.0;
    let mut drag_threshold_passed = false;
    let mut window_focused = true;
    let mut window_occluded = false;

    // Tab rename state
    let mut renaming_tab: Option<usize> = None;
    let mut rename_buffer = String::new();
    let mut rename_cursor: usize = 0;

    // Tab right-click context menu state
    let mut tab_ctx_visible = false;
    let mut tab_ctx_x = 0.0f64;
    let mut tab_ctx_y = 0.0f64;
    let mut tab_ctx_tab_idx: usize = 0;
    let mut tab_ctx_hovered: Option<usize> = None;

    // Secondary settings window state
    let mut settings_sw: Option<secondary_window::SecondaryWindow> = None;
    let mut settings_family = String::new();
    let mut settings_size = 14.0f32;
    let mut settings_scrollback = 3000usize;
    let mut settings_active_field = 0usize; // 0 = none, 1 = font family select dropdown, 2 = theme select dropdown
    let mut settings_theme = String::new();
    let mut s_hover_theme = false;
    let mut settings_hovered_theme_idx: Option<usize> = None;
    let themes_list = config::all_theme_names();
    
    let mut s_hover_close = false;
    let mut s_hover_family = false;
    let mut s_hover_size_minus = false;
    let mut s_hover_size_plus = false;
    let mut s_hover_scroll_minus = false;
    let mut s_hover_scroll_plus = false;
    let mut s_hover_open_config = false;
    
    let mut settings_font_scroll_y = 0.0f32;
    let mut settings_theme_scroll_y = 0.0f32;
    let mut settings_hovered_font_idx: Option<usize> = None;
    let mut system_fonts = Vec::<String>::new();
    let mut s_mouse_x = 0.0f64;
    let mut s_mouse_y = 0.0f64;

    // Secondary about window state
    let mut about: Option<secondary_window::SecondaryWindow> = None;

    let mut first_frame_rendered = false;
    let mut app_dirty = true;
    // Set when the window is resized; at the start of the next frame we
    // flush pending resizes to all background tabs so they catch up.
    let mut pending_bg_resize = false;
    // Deferred surface resize: we store the latest physical size and
    // apply it only right before the next render.  During drag this
    // avoids N × surface.configure() stalls when N resize events fire
    // between two frames.
    let mut pending_surface_resize: Option<(u32, u32)> = None;
    // Deferred terminal resize: same idea — term.resize() does expensive
    // text reflow when cols change, so we collapse N resize events into 1.
    let mut pending_term_resize: Option<(usize, usize)> = None;
    let mut last_render_time = std::time::Instant::now();
    let mut last_tui_title: Option<String> = None;
    let mut last_blink_index = 0;
    let mut next_render_reason = RenderReason::GridChanged;

    let update_available = Arc::new(parking_lot::Mutex::new(None::<String>));
    let update_in_progress = Arc::new(parking_lot::Mutex::new(false));
    let update_completed = Arc::new(parking_lot::Mutex::new(false));
    let mut has_shown_update_toast = false;
    let mut has_shown_success_toast = false;

    // Spawn a background thread to check for updates at startup
    {
        let update_available = Arc::clone(&update_available);
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            // Wait 2 seconds before checking to let the terminal boot up smoothly
            std::thread::sleep(std::time::Duration::from_secs(2));
            
            let mut cmd = std::process::Command::new("curl");
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            let cmd_res = cmd
                .arg("-s")
                .arg("-H")
                .arg("User-Agent: fastty")
                .arg("https://api.github.com/repos/diegoleteliers10/fastty/releases/latest")
                .output();

            if let Ok(output) = cmd_res {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                        if let Some(tag_name) = json.get("tag_name").and_then(|v| v.as_str()) {
                            let clean_tag = tag_name.trim_start_matches('v').trim();
                            let current_version = get_current_version();
                            let clean_current = current_version.trim_start_matches('v').trim();
                            if clean_tag != clean_current {
                                tracing::info!("Update available: {} (current: {})", tag_name, current_version);
                                *update_available.lock() = Some(tag_name.to_string());
                                let _ = proxy.send_event(AppEvent::Wakeup);
                            }
                        }
                    }
                }
            }
        });
    }

    let mut git_watcher_manager = git::GitWatcherManager::new(proxy.clone());

    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    event_loop.run(move |event, target| {
        match event {
            winit::event::Event::LoopExiting => {
                if config.session_restore {
                    let saved: Vec<session::TabInfo> = tabs.iter().map(|t| {
                        session::TabInfo {
                            cwd: tab_live_cwd(t).or_else(|| t.cwd.clone()),
                            custom_name: t.custom_name.clone(),
                            title_override: t.title_override.clone(),
                        }
                    }).collect();
                    let main_pos = window_for_redraw.outer_position().ok().map(|p| (p.x, p.y));
                    let main_size = {
                        let s = window_for_redraw.inner_size();
                        (s.width, s.height)
                    };
                    let mut all_windows = vec![session::WindowSession {
                        tabs: saved,
                        active_tab: active_tab_index,
                        position: main_pos,
                        size: Some(main_size),
                    }];
                    for wc in popped_out_windows.values() {
                        let saved_tabs: Vec<session::TabInfo> = wc.tabs.iter().map(|t| {
                            session::TabInfo {
                                cwd: tab_live_cwd(t).or_else(|| t.cwd.clone()),
                                custom_name: t.custom_name.clone(),
                                title_override: t.title_override.clone(),
                            }
                        }).collect();
                        let pos = wc.window.outer_position().ok().map(|p| (p.x, p.y));
                        let size = {
                            let s = wc.window.inner_size();
                            (s.width, s.height)
                        };
                        all_windows.push(session::WindowSession {
                            tabs: saved_tabs,
                            active_tab: wc.active_tab_index,
                            position: pos,
                            size: Some(size),
                        });
                    }
                    let s = session::Session {
                        windows: all_windows,
                        active_window: 0,
                        legacy_tabs: Vec::new(),
                        legacy_active_tab: 0,
                    };
                    if let Err(e) = session::save(&s) {
                        tracing::warn!("session: save failed: {e}");
                    }
                }
            }
            winit::event::Event::UserEvent(app_event) => {
                match app_event {
                    AppEvent::Wakeup => {
                        app_dirty = true;
                        renderer.lock().grid_dirty = true;
                    }
                    AppEvent::ShowToast { message, duration_ms } => {
                        toast = Some((message, std::time::Instant::now(), duration_ms));
                        app_dirty = true;
                        renderer.lock().grid_dirty = true;
                        window_for_redraw.request_redraw();
                    }
                    AppEvent::ForcePollWidgets => {
                        let now = std::time::Instant::now();
                        for w in bar_layout.widgets.iter_mut() {
                            w.set_last_poll(now - w.poll_interval());
                        }
                        app_dirty = true;
                        renderer.lock().grid_dirty = true;
                        window_for_redraw.request_redraw();
                    }
                    AppEvent::GitStatusUpdated { window_id, tab_idx, status } => {
                        if let Some(win_id) = window_id {
                            if let Some(wc) = popped_out_windows.get_mut(&win_id) {
                                if let Some(tab) = wc.tabs.get_mut(tab_idx) {
                                    tab.git_status = status;
                                    wc.renderer.lock().set_dirty(true);
                                    wc.window.request_redraw();
                                }
                            }
                        } else {
                            if let Some(tab) = tabs.get_mut(tab_idx) {
                                tab.git_status = status;
                                app_dirty = true;
                                renderer.lock().set_dirty(true);
                                window_for_redraw.request_redraw();
                            }
                        }
                    }
                    AppEvent::GitRepoChanged { repo_path } => {
                        for (idx, tab) in tabs.iter_mut().enumerate() {
                            let cwd = tab.terminal_state.lock().shell_pid()
                                .and_then(|pid| std::fs::read_link(format!("/proc/{}/cwd", pid)).ok())
                                .or_else(|| tab.cwd.clone());
                            if let Some(ref dir) = cwd {
                                if let Some(top) = git::git_toplevel(dir) {
                                    if top == repo_path {
                                        let dir_clone = dir.clone();
                                        let proxy_clone = proxy.clone();
                                        let tab_idx = idx;
                                        std::thread::spawn(move || {
                                            let status = detect_git_status(&dir_clone);
                                            let _ = proxy_clone.send_event(AppEvent::GitStatusUpdated {
                                                window_id: None,
                                                tab_idx,
                                                status,
                                            });
                                        });
                                    }
                                }
                            }
                        }
                        for (win_id, wc) in popped_out_windows.iter_mut() {
                            let win_id_val = *win_id;
                            for (tidx, tab) in wc.tabs.iter_mut().enumerate() {
                                let cwd = tab.terminal_state.lock().shell_pid()
                                    .and_then(|pid| std::fs::read_link(format!("/proc/{}/cwd", pid)).ok())
                                    .or_else(|| tab.cwd.clone());
                                if let Some(ref dir) = cwd {
                                    if let Some(top) = git::git_toplevel(dir) {
                                        if top == repo_path {
                                            let dir_clone = dir.clone();
                                            let proxy_clone = proxy.clone();
                                            let tab_idx = tidx;
                                            std::thread::spawn(move || {
                                                let status = detect_git_status(&dir_clone);
                                                let _ = proxy_clone.send_event(AppEvent::GitStatusUpdated {
                                                    window_id: Some(win_id_val),
                                                    tab_idx,
                                                    status,
                                                });
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    AppEvent::Exit { shell_pid } => {
                        if auto_close {
                            target.exit();
                        } else if let Some(pid) = shell_pid {
                            // The shell in one tab died (e.g. user typed `exit`).
                            // Close that tab; if it was the last tab in its
                            // window, close the window (or the app for main).
                            let found_in_main = tabs
                                .iter()
                                .position(|t| t.terminal_state.lock().shell_pid() == Some(pid));
                            if let Some(idx) = found_in_main {
                                tabs.remove(idx);
                                if tabs.is_empty() {
                                    target.exit();
                                } else {
                                    if active_tab_index >= tabs.len() {
                                        active_tab_index = tabs.len() - 1;
                                    }
                                    let physical_size = window_for_redraw.inner_size();
                                    let (cols, rows) = resize_all_tabs(
                                        &tabs,
                                        physical_size.width,
                                        physical_size.height,
                                        cell_width,
                                        cell_height,
                                    );
                                    shell_cols = cols;
                                    shell_rows = rows;
                                    app_dirty = true;
                                    renderer.lock().grid_dirty = true;
                                    window_for_redraw.request_redraw();
                                }
                            } else {
                                // Search popped-out windows.
                                let mut close_wid = None;
                                for (wid, wc) in popped_out_windows.iter_mut() {
                                    if let Some(idx) = wc
                                        .tabs
                                        .iter()
                                        .position(|t| {
                                            t.terminal_state.lock().shell_pid() == Some(pid)
                                        })
                                    {
                                        if wc.close_tab(idx) {
                                            close_wid = Some(*wid);
                                        }
                                        break;
                                    }
                                }
                                if let Some(wid) = close_wid {
                                    popped_out_windows.remove(&wid);
                                }
                            }
                        }
                    }
                    AppEvent::ForceExit => {
                        target.exit();
                    }
                    AppEvent::ConfigChanged => {
                        match Config::load() {
                            Ok(new_config) => {
                                keybindings::init_resolver(new_config.keybindings.clone());
                                let theme_changed = new_config.theme != config.theme;
                                let font_changed = new_config.font.family != config.font.family
                                    || (new_config.font.size - config.font.size).abs() > f32::EPSILON;
                                let scrollback_changed = new_config.scrollback != config.scrollback;
                                let ligatures_changed = new_config.font.ligatures != config.font.ligatures;
                                let weight_changed = (new_config.font.weight - config.font.weight).abs() > f32::EPSILON;
                                let shell_changed = new_config.shell != config.shell;
                                let opacity_changed = (new_config.opacity - config.opacity).abs() > f32::EPSILON;

                                if theme_changed || font_changed || scrollback_changed || ligatures_changed || weight_changed || shell_changed || opacity_changed {
                                    tracing::info!(
                                        "config: live-reload theme={} font={} scrollback={} ligatures={} weight={} shell={} opacity={}",
                                        theme_changed, font_changed, scrollback_changed, ligatures_changed, weight_changed, shell_changed, opacity_changed
                                    );
                                    config = new_config;
                                    bar_layout = build_bar_layout(&config);

                                    if theme_changed {
                                        if let Some(t) = &config.theme {
                                            config::set_active_theme(t);
                                        }
                                    }

                                    if font_changed {
                                        if let Err(e) = renderer.lock().update_font(&config.font.family, config.font.size) {
                                            tracing::error!("config live-reload: font update failed: {e:?}");
                                        }
                                        if let Some(ref mut sr) = settings_sw {
                                            let _ = sr.renderer.update_font(&config.font.family, 13.0);
                                        }
                                        let cell_w = renderer.lock().cell_width();
                                        let cell_h = renderer.lock().cell_height();
                                        let physical_size = window_for_redraw.inner_size();
                                        let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_w, cell_h);
                                        shell_cols = cols;
                                        shell_rows = rows;
                                        cell_width = cell_w;
                                        cell_height = cell_h;
                                    }

                                    if scrollback_changed {
                                        for tab in &tabs {
                                            tab.terminal_state.lock().update_scrollback(config.scrollback);
                                        }
                                    }

                                    if ligatures_changed {
                                        tracing::info!("config: font.ligatures changed; restart fastty to apply");
                                    }

                                    if weight_changed {
                                        tracing::info!("config: font.weight changed; restart fastty to apply");
                                    }

                                    if shell_changed {
                                        tracing::info!("config: shell changed; applies to newly spawned tabs only");
                                    }

                                    if settings_sw.is_some() {
                                        settings_family = config.font.family.clone();
                                        settings_size = config.font.size;
                                        settings_scrollback = config.scrollback.min(1000);
                                        settings_theme = config.theme.clone().unwrap_or_else(|| "default".to_string());
                                        if let Some(ref mut sr) = settings_sw {
                                            sr.renderer.set_dirty(true);
                                        }
                                    }

                                    renderer.lock().grid_dirty = true;
                                    app_dirty = true;
                                    window_for_redraw.request_redraw();
                                    if let Some(ref sw) = settings_sw {
                                        sw.window.request_redraw();
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("config: reload failed, keeping in-memory: {e}");
                                toast = Some((
                                    "⚠ Not a valid config".to_string(),
                                    std::time::Instant::now(),
                                    4000,
                                ));
                                app_dirty = true;
                                window_for_redraw.request_redraw();
                            }
                        }
                    }
                    AppEvent::Bell => {
                        bell_flash_time = Some(std::time::Instant::now());
                        renderer.lock().set_dirty(true);
                        window_for_redraw.request_redraw();

                        if !window_focused || window_occluded {
                            let tab_title = tabs.get(active_tab_index)
                                .and_then(|t| t.title_override.clone())
                                .unwrap_or_else(|| format!("Tab {}", active_tab_index + 1));
                            let body = format!("Terminal bell in {}", tab_title);
                            if let Err(e) = notify_rust::Notification::new()
                                .summary("Fastty Bell Alert")
                                .body(&body)
                                .appname("Fastty")
                                .show()
                            {
                                tracing::warn!("Failed to send bell desktop notification: {:?}", e);
                            }
                        }
                    }
                    AppEvent::ClipboardStore(text) => {
                        let mut cb = if clipboard.is_none() {
                            match arboard::Clipboard::new() {
                                Ok(ctx) => {
                                    clipboard = Some(ctx);
                                    clipboard.as_mut()
                                }
                                Err(_) => None,
                            }
                        } else {
                            clipboard.as_mut()
                        };
                        if let Some(ctx) = cb.as_mut() {
                            let _ = ctx.set_text(text);
                        }
                    }
                    AppEvent::ClipboardLoad(_ty) => {
                        // Note: OSC 52 read responses are handled inline in
                        // EventListenerProxy (it has the callback). This arm is
                        // a no-op for forward compatibility.
                    }
                    AppEvent::CwdChanged(path) => {
                        if let Some(tab) = tabs.get_mut(active_tab_index) {
                            tab.cwd = Some(path.clone());
                        }
                        tracing::info!("cwd changed: {}", path.display());
                        app_dirty = true;
                        window_for_redraw.request_redraw();
                    }
                    AppEvent::TitleChanged(title) => {
                        if let Some(tab) = tabs.get_mut(active_tab_index) {
                            tab.title_override = Some(title);
                        }
                        app_dirty = true;
                        window_for_redraw.request_redraw();
                    }
                    AppEvent::CommandStarted => {
                        tabs[active_tab_index].is_running = true;
                        renderer.lock().set_dirty(true);
                        window_for_redraw.request_redraw();
                    }
                    AppEvent::CommandFinished { duration_ms, exit_code } => {
                        tabs[active_tab_index].is_running = false;
                        tabs[active_tab_index].last_exit_code = exit_code;
                        last_command_duration = Some((duration_ms, exit_code));
                        last_command_duration_display_time = Some(std::time::Instant::now());
                        renderer.lock().set_dirty(true);
                        window_for_redraw.request_redraw();

                        // Send desktop notification when window is unfocused or occluded
                        if (!window_focused || window_occluded) && config.notify_on_command_finish {
                            let duration_str = if duration_ms < 1000 {
                                format!("{}ms", duration_ms)
                            } else {
                                format!("{:.1}s", duration_ms as f64 / 1000.0)
                            };
                            let exit_str = match exit_code {
                                Some(code) => format!(" (exit: {})", code),
                                None => String::new(),
                            };
                            let body = format!("Finished in {}{}", duration_str, exit_str);
                            if let Err(e) = notify_rust::Notification::new()
                                .summary("Fastty")
                                .body(&body)
                                .appname("Fastty")
                                .show()
                            {
                                tracing::warn!("Failed to send command finished notification: {:?}", e);
                            }
                        }
                    }
                    AppEvent::Notification { title, body } => {
                        if let Err(e) = notify_rust::Notification::new()
                            .summary(&title)
                            .body(&body)
                            .appname("Fastty")
                            .show()
                        {
                            tracing::warn!("Failed to send desktop notification: {:?}", e);
                        }
                    }
                    AppEvent::PromptStarted { absolute_line } => {
                        if let Some(tab) = tabs.get_mut(active_tab_index) {
                            if tab.prompts.last() != Some(&absolute_line) {
                                tab.prompts.push(absolute_line);
                                if tab.prompts.len() > 1000 {
                                    tab.prompts.remove(0);
                                }
                            }
                            // If a TUI agent (like claude) left cursor hidden (ESC[?25l) without
                            // restoring it (ESC[?25h), force cursor visible now that the shell
                            // prompt is back. This fixes the permanent cursor disappearance
                            // that happens with claude Code and similar TUI agents.
                            {
                                let term_guard = tab.terminal_state.lock();
                                let mode = *term_guard.term().lock().mode();
                                let show_cursor = mode.contains(alacritty_terminal::term::TermMode::SHOW_CURSOR);
                                if !show_cursor {
                                    tracing::info!("prompt started: SHOW_CURSOR was off, restoring cursor via ESC[?25h");
                                    term_guard.write_to_pty(b"\x1b[?25h");
                                }
                            }
                        }
                    }
                }
            }
            winit::event::Event::WindowEvent { window_id, event } => {
                if window_id != window_for_redraw.id() {
                    if popped_out_windows.contains_key(&window_id) {
                        handle_popped_out_event(
                            window_id,
                            event,
                            &mut popped_out_windows,
                            &config,
                            proxy.clone(),
                            window_for_redraw.id(),
                            &mut tabs,
                            &mut active_tab_index,
                            &mut shell_cols,
                            &mut shell_rows,
                            cell_width,
                            cell_height,
                            &window_for_redraw,
                            &mut hovered_window,
                            &mut pending_new_window_from_drag,
                            current_mouse_x,
                            &shell,
                        );
                        return;
                    }
                }
                if window_id == window_for_redraw.id() {
                    // --- Main Window Event Handler ---
                    match event {
                        WindowEvent::CloseRequested => {
                            target.exit();
                        }
                        WindowEvent::Resized(_size) => {
                            let physical_size = window_for_redraw.inner_size();
                            // Calculate cols/rows from the new size
                            const PADDING_LEFT: f32 = 10.0;
                            let padding_top = get_padding_top(tabs.len());
                            let padding_bottom = get_padding_bottom();
                            let cell_w = cell_width.max(1.0);
                            let cell_h = cell_height.max(1.0);
                            let cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_w).floor().max(1.0)) as usize;
                            let rows = (((physical_size.height as f32 - (padding_top + padding_bottom)) / cell_h).floor().max(1.0)) as usize;

                            shell_cols = cols;
                            shell_rows = rows;

                            // Defer terminal reflow + PTY resize to render time.
                            // Width changes trigger expensive text reflow in
                            // alacritty — collapsing N events into 1 per frame.
                            pending_term_resize = Some((cols, rows));

                            // Defer surface.configure() to render time.
                            // During rapid resize events (hundreds per second on
                            // Wayland), only the last size matters because we
                            // render once per frame.
                            pending_surface_resize = Some((physical_size.width, physical_size.height));
                            app_dirty = true;
                            // Background tabs catch up on the next frame
                            pending_bg_resize = true;
                            // Bypass the 16ms frame_time gate in AboutToWait
                            // so the resize renders on the very next event loop
                            // tick instead of waiting up to one full frame.
                            window_for_redraw.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            // Flush pending background tab resizes from
                            // the most recent window resize event.
                            if pending_bg_resize {
                                pending_bg_resize = false;
                                for (idx, tab) in tabs.iter().enumerate() {
                                    if idx != active_tab_index {
                                        tab.terminal_state.lock().resize(shell_cols, shell_rows);
                                    }
                                }
                            }
                            #[cfg(target_os = "windows")]
                            let was_rendered = first_frame_rendered;
                            first_frame_rendered = true;

                            // Apply deferred resizes BEFORE acquiring the
                            // terminal state lock below — otherwise the
                            // non-reentrant parking_lot::Mutex deadlocks.
                            if let Some((cols, rows)) = pending_term_resize.take() {
                                tabs[active_tab_index].terminal_state.lock().resize(cols, rows);
                            }
                            if let Some((w, h)) = pending_surface_resize.take() {
                                renderer.lock().resize(w, h);
                            }

                            let r_cfg = renderer.lock();
                            let vw = r_cfg.config.width as f32;
                            let vh = r_cfg.config.height as f32;
                            drop(r_cfg);

                            let active_tab_cwd = tabs[active_tab_index].terminal_state.lock()
                                .shell_pid()
                                .and_then(|pid| std::fs::read_link(format!("/proc/{}/cwd", pid)).ok())
                                .or_else(|| tabs[active_tab_index].cwd.clone());
                            let active_tab_git = tabs[active_tab_index].git_status.clone();

                            let (computed_bar_y, computed_bar_h) = poll_and_layout_bar(
                                &mut bar_layout,
                                active_tab_cwd.as_deref(),
                                active_tab_git.as_ref(),
                                config.opacity,
                                vw,
                                vh,
                            );
                            bar_y = computed_bar_y;
                            bar_h = computed_bar_h;

                            let mut tab_titles = Vec::new();
                            let mut active_tab_path = "fastty".to_string();
                            for (idx, tab) in tabs.iter().enumerate() {
                                let title = if let Some(ref name) = tab.custom_name {
                                    name.clone()
                                } else {
                                    let shell_pid = tab.terminal_state.lock().shell_pid();
                                    let agent = detect_tui_agent(shell_pid);
                                    if let Some(ref agent_name) = agent {
                                        let path_str = if let Some(pid) = tab.terminal_state.lock().shell_pid() {
                                            get_current_dir_shortened(pid)
                                        } else {
                                            None
                                        };
                                        let path_component = path_str.as_ref()
                                            .map(|p| get_last_path_component(p))
                                            .unwrap_or_else(|| "fastty".to_string());
                                        format!("{} - {}", agent_name, path_component)
                                    } else {
                                        let path_str = if let Some(pid) = tab.terminal_state.lock().shell_pid() {
                                            get_current_dir_shortened(pid)
                                        } else {
                                            None
                                        };
                                        if let Some(ref path) = path_str {
                                            get_last_path_component(path)
                                        } else {
                                            "bash".to_string()
                                        }
                                    }
                                };

                                if idx == active_tab_index {
                                    if tab.custom_name.is_some() {
                                        active_tab_path = title.clone();
                                    } else {
                                        active_tab_path = title.clone();
                                    }
                                }
                                tab_titles.push(title);
                            }

                            let tab_running_states: Vec<bool> = tabs.iter().map(|t| t.is_running).collect();
                            let tab_exit_codes: Vec<Option<i32>> = tabs.iter().map(|t| t.last_exit_code).collect();

                            // Detect TUI agent running in the active tab
                            {
                                let active_tab = &tabs[active_tab_index];
                                let shell_pid = active_tab.terminal_state.lock().shell_pid();
                                let new_agent = detect_tui_agent(shell_pid);
                                let new_title = match &new_agent {
                                    Some(agent) => Some(format!("{} - fastty", agent)),
                                    None => Some("fastty".to_string()),
                                };
                                if new_title != last_tui_title {
                                    // TUI agent state changed
                                    let agent_just_exited = last_tui_title.as_deref() != Some("fastty")
                                        && new_agent.is_none();
                                    last_tui_title = new_title.clone();
                                    if let Some(ref t) = new_title {
                                        window_for_redraw.set_title(t);
                                    }
                                    // If a TUI agent just exited, check if it left cursor hidden.
                                    // Agents like Claude may emit ESC[?25l but fail to restore it.
                                    if agent_just_exited {
                                        let term_guard = active_tab.terminal_state.lock();
                                        let mode = *term_guard.term().lock().mode();
                                        let show_cursor = mode.contains(alacritty_terminal::term::TermMode::SHOW_CURSOR);
                                        if !show_cursor {
                                            tracing::info!("TUI agent exited with cursor hidden — restoring via ESC[?25h");
                                            term_guard.write_to_pty(b"\x1b[?25h");
                                        }
                                    }
                                }
                            }

                            let active_tab = &tabs[active_tab_index];
                            let term = active_tab.terminal_state.lock();
                            let max_history = term.history_size() as f32;
                            let term_ref: &TerminalState = &*term;

                            let last_activity_time_secs = active_tab.last_activity_time.saturating_duration_since(start_time).as_secs_f32();
                            let current_time = start_time.elapsed().as_secs_f32();

                            let bell_flash_elapsed_ms = bell_flash_time.map(|t| t.elapsed().as_secs_f32() * 1000.0);
                            let (last_command_duration_ms, command_duration_display_secs, command_exit_code) =
                                match (last_command_duration, last_command_duration_display_time) {
                                    (Some((ms, code)), Some(display_time)) => {
                                        let elapsed = display_time.elapsed().as_secs_f32();
                                        (Some(ms), Some(elapsed), code)
                                    }
                                    _ => (None, None, None),
                                };

                            // Update checker and notifier state integration
                            let latest_ver = {
                                let guard = update_available.lock();
                                guard.clone()
                            };
                            if let Some(ver) = latest_ver {
                                if !has_shown_update_toast {
                                    has_shown_update_toast = true;
                                    let ver_str = if ver.starts_with('v') { ver.clone() } else { format!("v{}", ver) };
                                    toast = Some((
                                        format!("Update Available ({}) [Update Now]", ver_str),
                                        std::time::Instant::now(),
                                        2000,
                                    ));
                                    app_dirty = true;
                                }
                            }

                            let completed = {
                                let guard = update_completed.lock();
                                *guard
                            };
                            if completed && !has_shown_success_toast {
                                has_shown_success_toast = true;
                                toast = Some((
                                    "✓  Update success! Click 'Reiniciar' to restart.".to_string(),
                                    std::time::Instant::now(),
                                    3000,
                                ));
                                app_dirty = true;
                            }

                            let is_available = update_available.lock().is_some();
                            let is_in_progress = *update_in_progress.lock();

                            let drop_target_idx = if drag_threshold_passed {
                                dragging_tab.map(|_| {
                                    let vw = { let rr = renderer.lock(); let w = rr.config.width as f64; drop(rr); w };
                                    let tab_start_x = chrome_layout::tab_start_x() as f64;
                                    let path_center_x = vw / 2.0;
                                    let tab_area_max_x = path_center_x - 40.0;
                                    let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                    let tabs_len = tabs.len();
                                    let tab_width = if tabs_len > 0 {
                                        (tab_area_width / tabs_len as f64).clamp(80.0, 160.0)
                                    } else {
                                        160.0
                                    };
                                    compute_drop_target(current_mouse_x, tab_start_x, tab_width, tabs_len)
                                }).and_then(|t| if t < tabs.len() { Some(t) } else { None })
                            } else {
                                None
                            };

                            let mut r = renderer.lock();
                            r.update_available = is_available;
                            r.update_in_progress = is_in_progress;
                            r.update_completed = completed;
                            r.hover_update = hover_update;
                            r.set_dirty(true);
                            let palette_filtered: Vec<String> = if command_palette_visible {
                                compute_palette_filtered(&palette_commands, &command_palette_query)
                            } else {
                                Vec::new()
                            };

                            let ssh_filtered: Vec<String> = if ssh_picker_visible {
                                compute_ssh_filtered(&ssh_hosts, &ssh_picker_query)
                            } else {
                                Vec::new()
                            };

                            let project_filtered: Vec<String> = if project_jumper_visible {
                                compute_project_filtered(&project_jumper_items, &project_jumper_query)
                            } else {
                                Vec::new()
                            };

                            let worktree_filtered: Vec<String> = if worktree_picker_visible {
                                compute_worktree_filtered(&worktree_items, &worktree_picker_query, worktree_toplevel.as_deref())
                            } else {
                                Vec::new()
                            };

                            let inputs = renderer::RenderInputs {
                                ligatures: config.font.ligatures,
                                scrollbar_alpha,
                                scroll_current: active_tab.scroll_current,
                                history_size: max_history,
                                visible_rows: shell_rows as f32,
                                hover_close,
                                hover_max,
                                hover_min,
                                hover_settings,
                                last_activity_time_secs,
                                current_time,
                                selection: active_tab.selection,
                                hovered_url: active_tab.hovered_url,
                                hovered_hyperlink: active_tab.hovered_hyperlink.as_deref(),
                                search_matches: &active_tab.search_matches,
                                search_current_idx: active_tab.search_current_idx,
                                search_visible: active_tab.search_visible,
                                search_query_render: &active_tab.search_query,
                                terminal_font_size: config.font.size,
                                toast: toast.as_ref().map(|(msg, t, d)| (msg.as_str(), *t, *d)),
                                active_tab_index,
                                tab_titles: &tab_titles,
                                tab_running_states: &tab_running_states,
                                tab_exit_codes: &tab_exit_codes,
                                active_tab_path: &active_tab_path,
                                context_menu_visible,
                                context_menu_is_about,
                                context_menu_x: context_menu_x as f32,
                                context_menu_y: context_menu_y as f32,
                                context_menu_hovered_idx,
                                context_menu_open_time_secs,
                                context_menu_scroll_y: context_menu_scroll_y as f32,
                                context_menu_items: &context_menu_items,
                                hovered_tab_index,
                                hovered_close_tab_index,
                                hover_new_tab,
                                command_palette_visible,
                                command_palette_query: &command_palette_query,
                                command_palette_selected,
                                command_palette_filtered: &palette_filtered,
                                command_palette_scroll,
                                dragging_tab: if cross_window_drag::is_active() && hovered_window == Some(window_for_redraw.id()) { Some(tabs.len()) } else { dragging_tab },
                                drag_current_x: drag_current_x as f32,
                                drag_tab_offset: drag_tab_offset as f32,
                                drop_target_idx: if cross_window_drag::is_active() && hovered_window == Some(window_for_redraw.id()) {
                                    let tab_start_x = chrome_layout::tab_start_x() as f64;
                                    let path_center_x = (window_for_redraw.inner_size().width as f64) / 2.0;
                                    let tab_area_max_x = path_center_x - 40.0;
                                    let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                    let tabs_len = tabs.len() + 1;
                                    let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                                    Some(compute_drop_target(current_mouse_x, tab_start_x, tab_width, tabs_len))
                                } else {
                                    drop_target_idx
                                },
                                tab_ctx_visible,
                                tab_ctx_x: tab_ctx_x as f32,
                                tab_ctx_y: tab_ctx_y as f32,
                                tab_ctx_hovered,
                                renaming_tab,
                                rename_buffer: &rename_buffer,
                                rename_cursor,
                                git_status: active_tab.git_status.as_ref(),
                                bar_segments: &bar_layout.laid_out,
                                bar_y,
                                bar_h,
                                ssh_picker_visible,
                                ssh_picker_query: &ssh_picker_query,
                                ssh_picker_selected,
                                ssh_filtered: &ssh_filtered,
                                project_jumper_visible,
                                project_jumper_query: &project_jumper_query,
                                project_jumper_selected,
                                project_filtered: &project_filtered,
                                worktree_picker_visible,
                                worktree_picker_query: &worktree_picker_query,
                                worktree_picker_selected,
                                worktree_filtered: &worktree_filtered,
                                bell_flash_elapsed_ms,
                                last_command_duration_ms,
                                command_duration_display_secs,
                                exit_code: command_exit_code,
                                current_mouse_x: current_mouse_x as f32,
                                current_mouse_y: current_mouse_y as f32,
                                hovered_url_text: active_tab.hovered_url_text.as_deref(),
                                opacity: config.opacity,
                            };
                            r.render(next_render_reason, term_ref, active_tab.cursor_visible, inputs);
                            drop(r);

                            #[cfg(target_os = "windows")]
                            {
                                if !was_rendered {
                                    window_for_redraw.set_visible(true);
                                }
                            }

                            last_render_time = std::time::Instant::now();
                        }
                        WindowEvent::ModifiersChanged(modified) => {
                            modifiers = modified.state();
                            let padding_top = get_padding_top(tabs.len());
                            let (new_hover, new_hover_text) = detect_hovered_url(
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
                                tabs[active_tab_index].hovered_url_text = new_hover_text;
                                mark_grid_dirty(&renderer, &mut app_dirty);
                            }
                            let new_hyperlink = detect_hovered_hyperlink(
                                current_mouse_x, current_mouse_y,
                                &tabs[active_tab_index].terminal_state,
                                tabs[active_tab_index].scroll_current,
                                cell_width, cell_height, shell_cols, shell_rows, padding_top,
                            );
                            if tabs[active_tab_index].hovered_hyperlink != new_hyperlink {
                                tabs[active_tab_index].hovered_hyperlink = new_hyperlink;
                                let mut r = renderer.lock();
                                r.set_dirty(true);
                                app_dirty = true;
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
                            tabs[active_tab_index].cursor_visible = true;

                            let padding_top = get_padding_top(tabs.len());

                            let (new_hover, new_hover_text) = detect_hovered_url(
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
                                tabs[active_tab_index].hovered_url_text = new_hover_text;
                                mark_grid_dirty(&renderer, &mut app_dirty);
                            }
                            let new_hyperlink = detect_hovered_hyperlink(
                                current_mouse_x, current_mouse_y,
                                &tabs[active_tab_index].terminal_state,
                                tabs[active_tab_index].scroll_current,
                                cell_width, cell_height, shell_cols, shell_rows, padding_top,
                            );
                            if tabs[active_tab_index].hovered_hyperlink != new_hyperlink {
                                tabs[active_tab_index].hovered_hyperlink = new_hyperlink;
                                let mut r = renderer.lock();
                                r.set_dirty(true);
                                app_dirty = true;
                            }

                            if !pressed {
                                return;
                            }

                            if renaming_tab.is_some() {
                                use winit::keyboard::NamedKey;
                                match &event.logical_key {
                                    Key::Named(NamedKey::Enter) => {
                                        let idx = renaming_tab.take().unwrap();
                                        if rename_buffer.is_empty() {
                                            tabs[idx].custom_name = None;
                                        } else {
                                            tabs[idx].custom_name = Some(rename_buffer.clone());
                                        }
                                        rename_buffer.clear();
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                    }
                                    Key::Named(NamedKey::Escape) => {
                                        renaming_tab = None;
                                        rename_buffer.clear();
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                    }
                                    Key::Named(NamedKey::Backspace) => {
                                        if rename_cursor > 0 {
                                            rename_cursor -= 1;
                                            rename_buffer.remove(rename_cursor);
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                        }
                                    }
                                    Key::Named(NamedKey::Delete) => {
                                        if rename_cursor < rename_buffer.len() {
                                            rename_buffer.remove(rename_cursor);
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                        }
                                    }
                                    Key::Named(NamedKey::ArrowLeft) => {
                                        if rename_cursor > 0 {
                                            rename_cursor -= 1;
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                        }
                                    }
                                    Key::Named(NamedKey::ArrowRight) => {
                                        if rename_cursor < rename_buffer.len() {
                                            rename_cursor += 1;
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                        }
                                    }
                                    Key::Named(NamedKey::Home) => {
                                        rename_cursor = 0;
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                    }
                                    Key::Named(NamedKey::End) => {
                                        rename_cursor = rename_buffer.len();
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                    }
                                    Key::Character(s) => {
                                        if !ctrl_active && !alt_active {
                                            for ch in s.chars() {
                                                rename_buffer.insert(rename_cursor, ch);
                                                rename_cursor += 1;
                                            }
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                        }
                                    }
                                    _ => {}
                                }
                                window_for_redraw.request_redraw();
                                return;
                            }

                            if tab_ctx_visible {
                                if let Key::Named(winit::keyboard::NamedKey::Escape) = &event.logical_key {
                                    tab_ctx_visible = false;
                                    renderer.lock().set_dirty(true);
                                    app_dirty = true;
                                    window_for_redraw.request_redraw();
                                    return;
                                }
                            }

                            if context_menu_visible {
                                if let Key::Named(winit::keyboard::NamedKey::Escape) = &event.logical_key {
                                    context_menu_visible = false;
                                    context_menu_open_time = None;
                                    context_menu_open_time_secs = None;
                                    context_menu_hovered_idx = None;
                                    renderer.lock().set_dirty(true);
                                    app_dirty = true;
                                    window_for_redraw.request_redraw();
                                    return;
                                }
                            }

                            if command_palette_visible {
                                use winit::keyboard::NamedKey;
                                match &event.logical_key {
                                    Key::Named(NamedKey::Escape) => {
                                        command_palette_visible = false;
                                        command_palette_query.clear();
                                        command_palette_selected = 0;
                                        command_palette_scroll = 0;
                                    }
                                    Key::Named(NamedKey::Enter) => {
                                        let filtered = filter_palette(&palette_commands, &command_palette_query);
                                        if let Some(&idx) = filtered.get(command_palette_selected) {
                                            let action = palette_commands[idx].1;
                                            command_palette_visible = false;
                                            command_palette_query.clear();
                                            command_palette_selected = 0;
                                            command_palette_scroll = 0;
                                            match action {
                                                CommandAction::NewTab => {
                                                    let new_tab_count = tabs.len() + 1;
                                                    let padding_top = get_padding_top(new_tab_count);
                                                    let physical_size = window_for_redraw.inner_size();
                                                    let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                                    let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / cell_height).floor().max(1.0)) as usize;                                                    if let Ok(new_tab) = create_new_tab(
                                                        &shell, &[], None, config.scrollback, config.font.clone(),
                                                        cell_width, cell_height, new_cols, new_rows, proxy.clone(),
                                                    ) {
                                                        tabs.push(new_tab);
                                                        active_tab_index = tabs.len() - 1;
                                                        let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                        shell_cols = cols;
                                                        shell_rows = rows;
                                                        mark_grid_dirty(&renderer, &mut app_dirty);
                                                    }
                                                }
                                                CommandAction::CloseTab => {
                                                    if tabs.len() <= 1 {
                                                        target.exit();
                                                    } else {
                                                        tabs.remove(active_tab_index);
                                                        if active_tab_index >= tabs.len() {
                                                            active_tab_index = tabs.len() - 1;
                                                        }
                                                        let physical_size = window_for_redraw.inner_size();
                                                        let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                        shell_cols = cols;
                                                        shell_rows = rows;
                                                        mark_grid_dirty(&renderer, &mut app_dirty);
                                                    }
                                                }
                                                CommandAction::NextTab => {
                                                    if tabs.len() > 1 {
                                                        active_tab_index = (active_tab_index + 1) % tabs.len();
                                                        mark_grid_dirty(&renderer, &mut app_dirty);
                                                    }
                                                }
                                                CommandAction::PrevTab => {
                                                    if tabs.len() > 1 {
                                                        if active_tab_index == 0 {
                                                            active_tab_index = tabs.len() - 1;
                                                        } else {
                                                            active_tab_index -= 1;
                                                        }
                                                        mark_grid_dirty(&renderer, &mut app_dirty);
                                                    }
                                                }
                                                CommandAction::OpenSettings => {
                                                    // Inlined open-settings logic; see Settings dialog topbar.
                                                    if settings_sw.is_none() {
                                                        match Config::load() {
                                                            Ok(fresh) => { config = fresh; }
                                                            Err(e) => {
                                                                tracing::warn!("config: settings-open reload failed: {e}");
                                                                toast = Some(("⚠ Not a valid config".to_string(), std::time::Instant::now(), 4000));
                                                            }
                                                        }
                                                        settings_family = config.font.family.clone();
                                                        settings_size = config.font.size;
                                                        settings_scrollback = config.scrollback.min(1000);
                                                        settings_theme = config.theme.clone().unwrap_or_else(|| "default".to_string());
                                                        settings_active_field = 0;
                                                        match secondary_window::SecondaryWindow::create(
                                                             target,
                                                             "fastty Settings",
                                                             400.0,
                                                             260.0,
                                                             &renderer,
                                                             &config.font.family,
                                                         ) {
                                                             Ok(mut sw) => {
                                                                 #[cfg(target_os = "windows")]
                                                                 {
                                                                     sw.show_and_focus();
                                                                 }
                                                                 settings_sw = Some(sw);
                                                             }
                                                             Err(e) => {
                                                                 tracing::error!("Failed to create settings window: {:?}", e);
                                                             }
                                                         }
                                                    }
                                                    app_dirty = true;
                                                }
                                                CommandAction::OpenSearch => {
                                                    let tab = &mut tabs[active_tab_index];
                                                    tab.search_visible = true;
                                                    tab.search_query.clear();
                                                    tab.search_matches.clear();
                                                    tab.search_current_idx = 0;
                                                    renderer.lock().set_dirty(true);
                                                    app_dirty = true;
                                                }
                                                CommandAction::ReloadConfig => {
                                                    let _ = proxy.send_event(AppEvent::ConfigChanged);
                                                }
                                                CommandAction::IncreaseFontSize => {
                                                    config = Config::load().unwrap_or_default();
                                                    let new_size = (config.font.size + 0.5).min(72.0);
                                                    apply_font_size(
                                                        &mut config, new_size,
                                                        &mut tabs, &mut shell_cols, &mut shell_rows,
                                                        &mut cell_width, &mut cell_height,
                                                        &renderer, &window_for_redraw, &mut app_dirty,
                                                    );
                                                }
                                                CommandAction::DecreaseFontSize => {
                                                    config = Config::load().unwrap_or_default();
                                                    let new_size = (config.font.size - 0.5).max(6.0);
                                                    apply_font_size(
                                                        &mut config, new_size,
                                                        &mut tabs, &mut shell_cols, &mut shell_rows,
                                                        &mut cell_width, &mut cell_height,
                                                        &renderer, &window_for_redraw, &mut app_dirty,
                                                    );
                                                }
                                                CommandAction::ResetFontSize => {
                                                    config = Config::load().unwrap_or_default();
                                                    apply_font_size(
                                                        &mut config, 13.0,
                                                        &mut tabs, &mut shell_cols, &mut shell_rows,
                                                        &mut cell_width, &mut cell_height,
                                                        &renderer, &window_for_redraw, &mut app_dirty,
                                                    );
                                                }
                                                CommandAction::NewWindow => {
                                                    if let Ok(exe) = std::env::current_exe() {
                                                        #[cfg(target_os = "windows")]
                                                        let _ = no_window_cmd(&exe.to_string_lossy()).spawn();
                                                        #[cfg(not(target_os = "windows"))]
                                                        let _ = std::process::Command::new(exe).spawn();
                                                    }
                                                }
                                                CommandAction::SnapToBottom => {
                                                    tabs[active_tab_index].scroll_target = 0.0;
                                                    app_dirty = true;
                                                }
                                                CommandAction::RenameTab => {
                                                    let idx = active_tab_index;
                                                    renaming_tab = Some(idx);
                                                    rename_buffer = tabs[idx].custom_name.clone()
                                                        .unwrap_or_else(|| {
                                                            let path_str = if let Some(pid) = tabs[idx].terminal_state.lock().shell_pid() {
                                                                get_current_dir_shortened(pid)
                                                            } else { None };
                                                            if let Some(ref p) = path_str { get_last_path_component(p) } else { "bash".to_string() }
                                                        });
                                                    rename_cursor = rename_buffer.len();
                                                }
                                            }
                                        }
                                    }
                                    Key::Named(NamedKey::ArrowDown) => {
                                        let n = filter_palette(&palette_commands, &command_palette_query).len();
                                        if n > 0 {
                                            command_palette_selected = (command_palette_selected + 1).min(n - 1);
                                            const MAX_VISIBLE: usize = 8;
                                            if command_palette_selected >= command_palette_scroll + MAX_VISIBLE {
                                                command_palette_scroll = command_palette_selected + 1 - MAX_VISIBLE;
                                            }
                                        }
                                    }
                                    Key::Named(NamedKey::ArrowUp) => {
                                        command_palette_selected = command_palette_selected.saturating_sub(1);
                                        if command_palette_selected < command_palette_scroll {
                                            command_palette_scroll = command_palette_selected;
                                        }
                                    }
                                    Key::Named(NamedKey::Backspace) => {
                                        command_palette_query.pop();
                                        command_palette_selected = 0;
                                        command_palette_scroll = 0;
                                    }
                                    Key::Named(NamedKey::Space) => {
                                        if !ctrl_active && !alt_active {
                                            command_palette_query.push(' ');
                                            command_palette_selected = 0;
                                            command_palette_scroll = 0;
                                        }
                                    }
                                    Key::Character(s) => {
                                        if !ctrl_active && !alt_active {
                                            command_palette_query.push_str(s);
                                            command_palette_selected = 0;
                                            command_palette_scroll = 0;
                                        }
                                    }
                                    _ => {}
                                }
                                mark_grid_dirty(&renderer, &mut app_dirty);
                                window_for_redraw.request_redraw();
                                return;
                            }

                            if ssh_picker_visible {
                                use winit::keyboard::NamedKey;
                                match &event.logical_key {
                                    Key::Named(NamedKey::Escape) => {
                                        ssh_picker_visible = false;
                                        ssh_picker_query.clear();
                                        ssh_picker_selected = 0;
                                    }
                                    Key::Named(NamedKey::Enter) => {
                                        let filtered = filter_ssh_hosts(&ssh_hosts, &ssh_picker_query);
                                        if let Some(&idx) = filtered.get(ssh_picker_selected) {
                                            let host = &ssh_hosts[idx];
                                            let args = host.ssh_args();
                                            let new_tab_count = tabs.len() + 1;
                                            let padding_top = get_padding_top(new_tab_count);
                                            let physical_size = window_for_redraw.inner_size();
                                            let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                            let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / cell_height).floor().max(1.0)) as usize;
                                            if let Ok(new_tab) = create_new_tab(
                                                "ssh", &args, None, config.scrollback, config.font.clone(),
                                                cell_width, cell_height, new_cols, new_rows, proxy.clone(),
                                            ) {
                                                tabs.push(new_tab);
                                                active_tab_index = tabs.len() - 1;
                                                let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                shell_cols = cols;
                                                shell_rows = rows;
                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                            }
                                        }
                                        ssh_picker_visible = false;
                                        ssh_picker_query.clear();
                                        ssh_picker_selected = 0;
                                    }
                                    Key::Named(NamedKey::Backspace) => {
                                        ssh_picker_query.pop();
                                        ssh_picker_selected = 0;
                                    }
                                    Key::Named(NamedKey::ArrowUp) => {
                                        let f = filter_ssh_hosts(&ssh_hosts, &ssh_picker_query);
                                        if !f.is_empty() {
                                            ssh_picker_selected = ssh_picker_selected.saturating_sub(1);
                                            app_dirty = true;
                                        }
                                    }
                                    Key::Named(NamedKey::ArrowDown) => {
                                        let filtered = filter_ssh_hosts(&ssh_hosts, &ssh_picker_query);
                                        if ssh_picker_selected + 1 < filtered.len() {
                                            ssh_picker_selected += 1;
                                        }
                                    }
                                    Key::Named(NamedKey::Space) => {
                                        if !ctrl_active && !alt_active {
                                            ssh_picker_query.push(' ');
                                            ssh_picker_selected = 0;
                                        }
                                    }
                                    Key::Character(s) => {
                                        if !ctrl_active && !alt_active {
                                            ssh_picker_query.push_str(s);
                                            ssh_picker_selected = 0;
                                        }
                                    }
                                    _ => {}
                                }
                                mark_grid_dirty(&renderer, &mut app_dirty);
                                window_for_redraw.request_redraw();
                                return;
                            }

                            if project_jumper_visible {
                                use winit::keyboard::NamedKey;
                                match &event.logical_key {
                                    Key::Named(NamedKey::Escape) => {
                                        project_jumper_visible = false;
                                        project_jumper_query.clear();
                                        project_jumper_selected = 0;
                                    }
                                    Key::Named(NamedKey::Enter) => {
                                        let filtered = filter_project_dirs(&project_jumper_items, &project_jumper_query);
                                        if let Some(&idx) = filtered.get(project_jumper_selected) {
                                            let path = project_jumper_items[idx].clone();
                                            let path_str = path.to_string_lossy().into_owned();
                                            let new_tab_count = tabs.len() + 1;
                                            let padding_top = get_padding_top(new_tab_count);
                                            let physical_size = window_for_redraw.inner_size();
                                            let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                            let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / cell_height).floor().max(1.0)) as usize;
                                            if let Ok(new_tab) = create_new_tab(
                                                &shell, &[], Some(&path_str), config.scrollback, config.font.clone(),
                                                cell_width, cell_height, new_cols, new_rows, proxy.clone(),
                                            ) {
                                                tabs.push(new_tab);
                                                active_tab_index = tabs.len() - 1;
                                                let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                shell_cols = cols;
                                                shell_rows = rows;
                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                            }
                                        }
                                        project_jumper_visible = false;
                                        project_jumper_query.clear();
                                        project_jumper_selected = 0;
                                    }
                                    Key::Named(NamedKey::Backspace) => {
                                        project_jumper_query.pop();
                                        project_jumper_selected = 0;
                                    }
                                    Key::Named(NamedKey::ArrowUp) => {
                                        let f = filter_project_dirs(&project_jumper_items, &project_jumper_query);
                                        if !f.is_empty() {
                                            project_jumper_selected = project_jumper_selected.saturating_sub(1);
                                            app_dirty = true;
                                        }
                                    }
                                    Key::Named(NamedKey::ArrowDown) => {
                                        let filtered = filter_project_dirs(&project_jumper_items, &project_jumper_query);
                                        if project_jumper_selected + 1 < filtered.len() {
                                            project_jumper_selected += 1;
                                        }
                                    }
                                    Key::Named(NamedKey::Space) => {
                                        if !ctrl_active && !alt_active {
                                            project_jumper_query.push(' ');
                                            project_jumper_selected = 0;
                                        }
                                    }
                                    Key::Character(s) => {
                                        if !ctrl_active && !alt_active {
                                            project_jumper_query.push_str(s);
                                            project_jumper_selected = 0;
                                        }
                                    }
                                    _ => {}
                                }
                                mark_grid_dirty(&renderer, &mut app_dirty);
                                window_for_redraw.request_redraw();
                                return;
                            }

                            if worktree_picker_visible {
                                use winit::keyboard::NamedKey;
                                match &event.logical_key {
                                    Key::Named(NamedKey::Escape) => {
                                        worktree_picker_visible = false;
                                        worktree_picker_query.clear();
                                        worktree_picker_selected = 0;
                                    }
                                    Key::Named(NamedKey::Enter) => {
                                        let create_branch = worktree_picker_query.strip_prefix('+').map(str::trim).unwrap_or("");
                                        let filtered = filter_worktrees(&worktree_items, &worktree_picker_query);
                                        let total_rows = filtered.len() + if worktree_picker_query.starts_with('+') && !create_branch.is_empty() { 1 } else { 0 };
                                        let selected = worktree_picker_selected.min(total_rows.saturating_sub(1));
                                        if selected >= filtered.len() {
                                            if let Some(ref toplevel) = worktree_toplevel {
                                                if !create_branch.is_empty() {
                                                    match git::create_worktree(toplevel, create_branch) {
                                                        Some(new_path) => {
                                                            let path_str = new_path.to_string_lossy().into_owned();
                                                            let new_tab_count = tabs.len() + 1;
                                                            let padding_top = get_padding_top(new_tab_count);
                                                            let physical_size = window_for_redraw.inner_size();
                                                            let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                                            let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / cell_height).floor().max(1.0)) as usize;
                                                            if let Ok(new_tab) = create_new_tab(
                                                                &shell, &[], Some(&path_str), config.scrollback, config.font.clone(),
                                                                cell_width, cell_height, new_cols, new_rows, proxy.clone(),
                                                            ) {
                                                                tabs.push(new_tab);
                                                                active_tab_index = tabs.len() - 1;
                                                                let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                                shell_cols = cols;
                                                                shell_rows = rows;
                                                                worktree_items = git::list_worktrees(&new_path);
                                                                worktree_toplevel = Some(new_path);
                                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                                             }
                                                        }
                                                        None => {
                                                            toast = Some((
                                                                "git worktree add failed".to_string(),
                                                                std::time::Instant::now(),
                                                                3000,
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        } else if let Some(&wt_idx) = filtered.get(selected) {
                                            if let Some(wt) = worktree_items.get(wt_idx) {
                                                let path_str = wt.path.to_string_lossy().into_owned();
                                                let new_tab_count = tabs.len() + 1;
                                                let padding_top = get_padding_top(new_tab_count);
                                                let physical_size = window_for_redraw.inner_size();
                                                let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                                let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / cell_height).floor().max(1.0)) as usize;
                                                if let Ok(new_tab) = create_new_tab(
                                                    &shell, &[], Some(&path_str), config.scrollback, config.font.clone(),
                                                    cell_width, cell_height, new_cols, new_rows, proxy.clone(),
                                                ) {
                                                    tabs.push(new_tab);
                                                    active_tab_index = tabs.len() - 1;
                                                    let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                    shell_cols = cols;
                                                    shell_rows = rows;
                                                    mark_grid_dirty(&renderer, &mut app_dirty);
                                                }
                                            }
                                        }
                                        worktree_picker_visible = false;
                                        worktree_picker_query.clear();
                                        worktree_picker_selected = 0;
                                    }
                                    Key::Named(NamedKey::Backspace) => {
                                        worktree_picker_query.pop();
                                        worktree_picker_selected = 0;
                                    }
                                    Key::Named(NamedKey::ArrowUp) => {
                                        if worktree_picker_selected > 0 {
                                            worktree_picker_selected -= 1;
                                            app_dirty = true;
                                        }
                                    }
                                    Key::Named(NamedKey::ArrowDown) => {
                                        let filtered = filter_worktrees(&worktree_items, &worktree_picker_query);
                                        let create_extra = if worktree_picker_query.starts_with('+') && !worktree_picker_query.strip_prefix('+').map(str::trim).unwrap_or("").is_empty() { 1 } else { 0 };
                                        if worktree_picker_selected + 1 < filtered.len() + create_extra {
                                            worktree_picker_selected += 1;
                                        }
                                    }
                                    Key::Character(s) => {
                                        if !ctrl_active && !alt_active {
                                            worktree_picker_query.push_str(s);
                                            worktree_picker_selected = 0;
                                        }
                                    }
                                    _ => {}
                                }
                                mark_grid_dirty(&renderer, &mut app_dirty);
                                window_for_redraw.request_redraw();
                                return;
                            }

                            if let Some(combo) = keybindings::combo_from_event(&event, ctrl_active, shift_active, alt_active) {
                                if let Some(action) = keybindings::RESOLVER.get_or_init(|| parking_lot::RwLock::new(keybindings::KeyBindingResolver::with_defaults())).read().resolve(&combo) {
                                    match action {
                                        keybindings::Action::NewTab => {
                                            let new_tab_count = tabs.len() + 1;
                                            let padding_top = get_padding_top(new_tab_count);
                                            let physical_size = window_for_redraw.inner_size();
                                            let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                             let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / cell_height).floor().max(1.0)) as usize;
                                            match create_new_tab(
                                                &shell, &[], None, config.scrollback, config.font.clone(),
                                                cell_width, cell_height, new_cols, new_rows, proxy.clone(),
                                            ) {
                                                Ok(new_tab) => {
                                                    tabs.push(new_tab);
                                                    active_tab_index = tabs.len() - 1;
                                                    let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                    shell_cols = cols;
                                                    shell_rows = rows;
                                                    mark_grid_dirty(&renderer, &mut app_dirty);
                                                }
                                                Err(e) => tracing::error!("Failed to create new tab: {:?}", e),
                                            }
                                            return;
                                        }
                                        keybindings::Action::CloseTab => {
                                            if tabs.len() <= 1 {
                                                target.exit();
                                            } else {
                                                tabs.remove(active_tab_index);
                                                if active_tab_index >= tabs.len() {
                                                    active_tab_index = tabs.len() - 1;
                                                }
                                                let physical_size = window_for_redraw.inner_size();
                                                let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                shell_cols = cols;
                                                shell_rows = rows;
                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                            }
                                            return;
                                        }
                                        keybindings::Action::NewWindow => {
                                            if let Ok(exe) = std::env::current_exe() {
                                                #[cfg(target_os = "windows")]
                                                let _ = no_window_cmd(&exe.to_string_lossy()).spawn();
                                                #[cfg(not(target_os = "windows"))]
                                                let _ = std::process::Command::new(exe).spawn();
                                            }
                                            return;
                                        }
                                        keybindings::Action::Copy => {
                                            if let Some(sel) = tabs[active_tab_index].selection {
                                                copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
                                                toast = Some((
                                                    "✓  Text copied".to_string(),
                                                    std::time::Instant::now(),
                                                    1920,
                                                ));
                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                            }
                                            return;
                                        }
                                        keybindings::Action::Paste => {
                                            let mut ctx_opt = if clipboard.is_none() {
                                                match arboard::Clipboard::new() {
                                                    Ok(ctx) => {
                                                        clipboard = Some(ctx);
                                                        clipboard.as_mut()
                                                    }
                                                    Err(_) => None,
                                                }
                                            } else {
                                                clipboard.as_mut()
                                            };
                                            if let Some(ref mut ctx) = ctx_opt {
                                                if let Ok(text) = ctx.get_text() {
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
                                                        tabs[active_tab_index].scroll_target = 0.0;
                                                        tabs[active_tab_index].terminal_state.lock().write_to_pty(&paste_bytes);
                                                    }
                                                }
                                            }
                                            return;
                                        }
                                        keybindings::Action::OpenSearch => {
                                            let tab = &mut tabs[active_tab_index];
                                            tab.search_visible = true;
                                            tab.search_query.clear();
                                            tab.search_matches.clear();
                                            tab.search_current_idx = 0;
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                            return;
                                        }
                                        keybindings::Action::OpenSettings => {
                                            if settings_sw.is_none() {
                                                match Config::load() {
                                                    Ok(fresh) => { config = fresh; }
                                                    Err(e) => {
                                                        tracing::warn!("config: settings-open reload failed, keeping in-memory: {e}");
                                                        toast = Some((
                                                            "⚠ Not a valid config".to_string(),
                                                            std::time::Instant::now(),
                                                            4000,
                                                        ));
                                                    }
                                                }
                                                settings_family = config.font.family.clone();
                                                settings_size = config.font.size;
                                                settings_scrollback = config.scrollback.min(1000);
                                                settings_theme = config.theme.clone().unwrap_or_else(|| "default".to_string());
                                                settings_active_field = 0;
                                                match secondary_window::SecondaryWindow::create(
                                                     target,
                                                     "fastty Settings",
                                                     400.0,
                                                     260.0,
                                                     &renderer,
                                                     &config.font.family,
                                                 ) {
                                                     Ok(mut sw) => {
                                                         #[cfg(target_os = "windows")]
                                                         {
                                                             sw.renderer.set_dirty(true);
                                                             sw.renderer.render_settings(
                                                                 &config.font.family,
                                                                 config.font.size,
                                                                 config.scrollback.min(1000),
                                                                 0,
                                                                 false, false, false, false, false, false, false, false,
                                                                 &system_fonts,
                                                                 0.0,
                                                                 None,
                                                                 &settings_theme,
                                                                 &themes_list,
                                                                 None,
                                                                 0.0,
                                                                 config.opacity,
                                                             );
                                                             sw.window.set_visible(true);
                                                             sw.window.focus_window();
                                                         }
                                                         settings_sw = Some(sw);
                                                     }
                                                     Err(e) => {
                                                         tracing::error!("Failed to create settings window: {:?}", e);
                                                     }
                                                 }
                                            }
                                            app_dirty = true;
                                            return;
                                        }
                                        keybindings::Action::ReloadConfig => {
                                            let _ = proxy.send_event(AppEvent::ConfigChanged);
                                            return;
                                        }
                                        keybindings::Action::IncreaseFontSize => {
                                            config = Config::load().unwrap_or_default();
                                            let new_size = (config.font.size + 0.5).min(72.0);
                                            apply_font_size(
                                                &mut config, new_size,
                                                &mut tabs, &mut shell_cols, &mut shell_rows,
                                                &mut cell_width, &mut cell_height,
                                                &renderer, &window_for_redraw, &mut app_dirty,
                                            );
                                            return;
                                        }
                                        keybindings::Action::DecreaseFontSize => {
                                            config = Config::load().unwrap_or_default();
                                            let new_size = (config.font.size - 0.5).max(6.0);
                                            apply_font_size(
                                                &mut config, new_size,
                                                &mut tabs, &mut shell_cols, &mut shell_rows,
                                                &mut cell_width, &mut cell_height,
                                                &renderer, &window_for_redraw, &mut app_dirty,
                                            );
                                            return;
                                        }
                                        keybindings::Action::ResetFontSize => {
                                            config = Config::load().unwrap_or_default();
                                            apply_font_size(
                                                &mut config, 13.0,
                                                &mut tabs, &mut shell_cols, &mut shell_rows,
                                                &mut cell_width, &mut cell_height,
                                                &renderer, &window_for_redraw, &mut app_dirty,
                                            );
                                            return;
                                        }
                                        keybindings::Action::NextTab => {
                                            if tabs.len() > 1 {
                                                active_tab_index = (active_tab_index + 1) % tabs.len();
                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                            }
                                            return;
                                        }
                                        keybindings::Action::PrevTab => {
                                            if tabs.len() > 1 {
                                                if active_tab_index == 0 {
                                                    active_tab_index = tabs.len() - 1;
                                                } else {
                                                    active_tab_index -= 1;
                                                }
                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                            }
                                            return;
                                        }
                                        keybindings::Action::PrevPrompt => {
                                            let tab = &mut tabs[active_tab_index];
                                            let term_state = tab.terminal_state.lock();
                                            let term = term_state.term().lock();
                                            let current_total = term_state.total_lines_pushed.load(Ordering::Relaxed);
                                            let screen_lines = term.grid().screen_lines() as i32;
                                            let display_offset = term.grid().display_offset() as i32;

                                            let viewport_top_absolute = (current_total as i32 - screen_lines).max(0) - display_offset;

                                            let mut target_prompt = None;
                                            for &p in &tab.prompts {
                                                if (p as i32) < viewport_top_absolute {
                                                    target_prompt = Some(p);
                                                }
                                            }

                                            if let Some(p) = target_prompt {
                                                let new_offset = current_total as i32 - screen_lines - p as i32;
                                                let clamped_offset = new_offset.clamp(0, term.grid().history_size() as i32);
                                                drop(term);
                                                term_state.scroll(clamped_offset as isize - display_offset as isize);
                                                tab.scroll_target = clamped_offset as f32;
                                                let mut r = renderer.lock();
                                                r.set_dirty(true);
                                                app_dirty = true;
                                            }
                                            return;
                                        }
                                        keybindings::Action::NextPrompt => {
                                            let tab = &mut tabs[active_tab_index];
                                            let term_state = tab.terminal_state.lock();
                                            let term = term_state.term().lock();
                                            let current_total = term_state.total_lines_pushed.load(Ordering::Relaxed);
                                            let screen_lines = term.grid().screen_lines() as i32;
                                            let display_offset = term.grid().display_offset() as i32;

                                            let viewport_top_absolute = (current_total as i32 - screen_lines).max(0) - display_offset;

                                            let mut target_prompt = None;
                                            for &p in &tab.prompts {
                                                if (p as i32) > viewport_top_absolute {
                                                    target_prompt = Some(p);
                                                    break;
                                                }
                                            }

                                            if let Some(p) = target_prompt {
                                                let new_offset = current_total as i32 - screen_lines - p as i32;
                                                let clamped_offset = new_offset.clamp(0, term.grid().history_size() as i32);
                                                drop(term);
                                                term_state.scroll(clamped_offset as isize - display_offset as isize);
                                                tab.scroll_target = clamped_offset as f32;
                                                let mut r = renderer.lock();
                                                r.set_dirty(true);
                                                app_dirty = true;
                                            } else {
                                                drop(term);
                                                term_state.scroll(-display_offset as isize);
                                                tab.scroll_target = 0.0;
                                                let mut r = renderer.lock();
                                                r.set_dirty(true);
                                                app_dirty = true;
                                            }
                                            return;
                                        }
                                        keybindings::Action::SelectTab(n) => {
                                            let target_idx = (n - 1) as usize;
                                            if target_idx < tabs.len() {
                                                active_tab_index = target_idx;
                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                            }
                                            return;
                                        }
                                        keybindings::Action::CommandPalette => {
                                            command_palette_visible = !command_palette_visible;
                                            command_palette_query.clear();
                                            command_palette_selected = 0;
                                            command_palette_scroll = 0;
                                            if command_palette_visible {
                                                palette_commands = build_palette_commands();
                                            }
                                            let mut r = renderer.lock();
                                            r.set_dirty(true);
                                            app_dirty = true;
                                            return;
                                        }
                                        keybindings::Action::SshManager => {
                                            ssh_picker_visible = !ssh_picker_visible;
                                            ssh_picker_query.clear();
                                            ssh_picker_selected = 0;
                                            if ssh_picker_visible {
                                                ssh_hosts = ssh::parse_ssh_config();
                                            }
                                            let mut r = renderer.lock();
                                            r.set_dirty(true);
                                            app_dirty = true;
                                            return;
                                        }
                                        keybindings::Action::ProjectJumper => {
                                            project_jumper_visible = !project_jumper_visible;
                                            project_jumper_query.clear();
                                            project_jumper_selected = 0;
                                            if project_jumper_visible {
                                                project_jumper_items = collect_unique_project_dirs(&tabs);
                                            }
                                            let mut r = renderer.lock();
                                            r.set_dirty(true);
                                            app_dirty = true;
                                            return;
                                        }
                                        keybindings::Action::WorktreePicker => {
                                            let cwd = tab_live_cwd(&tabs[active_tab_index])
                                                .or_else(|| tabs[active_tab_index].cwd.clone());
                                            match cwd {
                                                Some(dir) if git::is_git_repo(&dir) => {
                                                    worktree_picker_visible = true;
                                                    worktree_picker_query.clear();
                                                    worktree_picker_selected = 0;
                                                    worktree_toplevel = git::git_toplevel(&dir);
                                                    worktree_items = git::list_worktrees(&dir);
                                                }
                                                Some(_) => {
                                                    worktree_picker_visible = false;
                                                    toast = Some((
                                                        "Not a git repository".to_string(),
                                                        std::time::Instant::now(),
                                                        2400,
                                                    ));
                                                }
                                                None => {
                                                    worktree_picker_visible = false;
                                                    toast = Some((
                                                        "No active cwd".to_string(),
                                                        std::time::Instant::now(),
                                                        2400,
                                                    ));
                                                }
                                            }
                                            let mut r = renderer.lock();
                                            r.set_dirty(true);
                                            app_dirty = true;
                                            return;
                                        }
                                    }
                                }
                            }

                            // When search bar is visible, all keyboard input goes to it.
                            if tabs[active_tab_index].search_visible {
                                use winit::keyboard::NamedKey;
                                let tab = &mut tabs[active_tab_index];
                                match &event.logical_key {
                                    Key::Named(NamedKey::Escape) => {
                                        tab.search_visible = false;
                                        tab.search_query.clear();
                                        tab.search_matches.clear();
                                        tab.search_current_idx = 0;
                                    }
                                    Key::Named(NamedKey::Backspace) => {
                                        tab.search_query.pop();
                                        tab.search_matches = compute_search_matches(&tab.terminal_state, &tab.search_query, shell_cols, shell_rows);
                                        tab.search_current_idx = 0;
                                    }
                                    Key::Named(NamedKey::Enter) => {
                                        if !tab.search_matches.is_empty() {
                                            tab.search_current_idx = (tab.search_current_idx + 1) % tab.search_matches.len();
                                            let m = tab.search_matches[tab.search_current_idx];
                                            if m.line < 0 {
                                                tab.scroll_target = (-m.line) as f32;
                                            } else {
                                                tab.scroll_target = 0.0;
                                            }
                                        }
                                    }
                                    Key::Character(s) => {
                                        if !ctrl_active && !alt_active {
                                            tab.search_query.push_str(s);
                                            tab.search_matches = compute_search_matches(&tab.terminal_state, &tab.search_query, shell_cols, shell_rows);
                                            tab.search_current_idx = 0;
                                        } else {
                                            return;
                                        }
                                    }
                                    _ => {}
                                }
                                mark_grid_dirty(&renderer, &mut app_dirty);
                                return;
                            }

                            // Plain Ctrl+letter always goes to the PTY without interception
                            if ctrl_active && !shift_active && !alt_active {
                                if let winit::keyboard::Key::Character(ref s) = event.logical_key {
                                    if let Some(ch) = s.chars().next() {
                                        if ch.is_ascii_alphabetic() || ['[', '\\', ']', '^', '_'].contains(&ch) {
                                            let b = (ch.to_ascii_uppercase() as u8) & 0x1F;
                                            tabs[active_tab_index].scroll_target = 0.0;
                                            tabs[active_tab_index].terminal_state.lock().write_to_pty(&[b]);
                                            return;
                                        }
                                    }
                                }
                            }

                            // Snippet expansion: bare Tab (no modifiers, no pickers, no TUI) tries
                            // to match the longest trigger at the end of the current prompt line.
                            if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) = &event.logical_key {
                                if !ctrl_active && !shift_active && !alt_active
                                    && !ssh_picker_visible && !project_jumper_visible && !command_palette_visible
                                {
                                    let snippet_mode = {
                                        let term = tabs[active_tab_index].terminal_state.lock();
                                        let term_guard = term.term().lock();
                                        *term_guard.mode()
                                    };
                                    let tui_active = snippet_mode.contains(
                                        alacritty_terminal::term::TermMode::ALT_SCREEN
                                    ) || snippet_mode.contains(
                                        alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                                    );
                                    if !tui_active {
                                        let prefix = read_prompt_prefix(&tabs[active_tab_index].terminal_state, shell_cols);
                                        if let Some(trigger_len) = snippets::match_trigger(&prefix) {
                                            let trigger_start = prefix.len() - trigger_len;
                                            let trigger = &prefix[trigger_start..];
                                            if let Some(body) = snippets::get_expansion(trigger) {
                                                let (expanded, _cursor_pos) = snippets::expand(&body);
                                                let mut bytes = Vec::with_capacity(trigger_len + expanded.len());
                                                for _ in 0..trigger_len {
                                                    bytes.push(0x7F);
                                                }
                                                bytes.extend_from_slice(expanded.as_bytes());
                                                tabs[active_tab_index].scroll_target = 0.0;
                                                tabs[active_tab_index].terminal_state.lock().write_to_pty(&bytes);
                                                mark_grid_dirty(&renderer, &mut app_dirty);
                                                return;
                                            }
                                        }
                                    }
                                }
                            }

                            let mode = {
                                let term = tabs[active_tab_index].terminal_state.lock();
                                let term_guard = term.term().lock();
                                *term_guard.mode()
                            };

                            let bytes = key_to_bytes(&event.logical_key, shift_active, ctrl_active, alt_active, mode);
                            if !bytes.is_empty() {
                                tabs[active_tab_index].scroll_target = 0.0;
                                tabs[active_tab_index].terminal_state.lock().write_to_pty(&bytes);
                            }
                        }
                        WindowEvent::MouseInput { state, button, .. } => {
                            let padding_top = get_padding_top(tabs.len());
                            tabs[active_tab_index].last_activity_time = std::time::Instant::now();
                            tabs[active_tab_index].cursor_visible = true;

                            if tab_ctx_visible {
                                if state == ElementState::Pressed {
                                    if button == MouseButton::Left {
                                        let menu_h = 76.0f64;
                                        let menu_w = 200.0f64;
                                        if current_mouse_x >= tab_ctx_x && current_mouse_x < tab_ctx_x + menu_w
                                            && current_mouse_y >= tab_ctx_y && current_mouse_y < tab_ctx_y + menu_h
                                        {
                                            let rel_y = current_mouse_y - tab_ctx_y;
                                            if rel_y >= 6.0 && rel_y < 38.0 {
                                                renaming_tab = Some(tab_ctx_tab_idx);
                                                rename_buffer = tabs[tab_ctx_tab_idx].custom_name.clone()
                                                    .unwrap_or_else(|| {
                                                        let path_str = if let Some(pid) = tabs[tab_ctx_tab_idx].terminal_state.lock().shell_pid() {
                                                            get_current_dir_shortened(pid)
                                                        } else { None };
                                                        if let Some(ref p) = path_str { get_last_path_component(p) } else { "bash".to_string() }
                                                    });
                                                rename_cursor = rename_buffer.len();
                                            } else if rel_y >= 38.0 && rel_y < 70.0 && tabs.len() >= 2 {
                                                pending_pop_out = Some(tab_ctx_tab_idx);
                                            }
                                        }
                                    }
                                    tab_ctx_visible = false;
                                    renderer.lock().set_dirty(true);
                                    app_dirty = true;
                                }
                                return;
                            }

                            if context_menu_visible {
                                let pressed = state == ElementState::Pressed;
                                if pressed {
                                    if button == MouseButton::Left {
                                        if let Some(hovered_idx) = context_menu_hovered_idx {
                                            let menu_items = if context_menu_is_about {
                                                vec![crate::renderer::ContextMenuItem::About]
                                            } else {
                                                context_menu_items.clone()
                                            };
                                            if hovered_idx < menu_items.len() {
                                                let item = &menu_items[hovered_idx];
                                                match item {
                                                    crate::renderer::ContextMenuItem::GithubActionInfo { url, .. } => {
                                                        if let Some(ref u) = url {
                                                            open_url(u);
                                                        }
                                                        context_menu_visible = false;
                                                    }
                                                    crate::renderer::ContextMenuItem::CommandItem { label, command, cwd } => {
                                                         let cmd_clone = command.clone();
                                                         let cwd_clone = cwd.clone();
                                                         let label_clone = label.clone();
                                                         let proxy_clone = proxy.clone();

                                                         let _ = proxy_clone.send_event(AppEvent::ShowToast {
                                                             message: format!("Running: {}...", label_clone),
                                                             duration_ms: 3000,
                                                         });

                                                         std::thread::spawn(move || {
                                                             let mut c = std::process::Command::new("sh");
                                                             c.arg("-c").arg(&cmd_clone);
                                                             c.current_dir(&cwd_clone);
                                                             #[cfg(target_os = "windows")]
                                                             {
                                                                 use std::os::windows::process::CommandExt;
                                                                 c.creation_flags(0x08000000);
                                                             }
                                                             match c.output() {
                                                                 Ok(output) => {
                                                                     if output.status.success() {
                                                                         let _ = proxy_clone.send_event(AppEvent::ShowToast {
                                                                             message: format!("✓ Success: {}", label_clone),
                                                                             duration_ms: 4000,
                                                                         });
                                                                     } else {
                                                                         let stderr = String::from_utf8_lossy(&output.stderr);
                                                                         let err_msg = if stderr.is_empty() {
                                                                             String::from_utf8_lossy(&output.stdout)
                                                                         } else {
                                                                             stderr
                                                                         };
                                                                         let clean_err = err_msg.trim().chars().take(40).collect::<String>();
                                                                         let _ = proxy_clone.send_event(AppEvent::ShowToast {
                                                                             message: format!("✗ Failed: {}", clean_err),
                                                                             duration_ms: 6000,
                                                                         });
                                                                     }
                                                                     let _ = proxy_clone.send_event(AppEvent::ForcePollWidgets);
                                                                 }
                                                                 Err(e) => {
                                                                     let _ = proxy_clone.send_event(AppEvent::ShowToast {
                                                                         message: format!("✗ Error: {}", e),
                                                                         duration_ms: 6000,
                                                                     });
                                                                     let _ = proxy_clone.send_event(AppEvent::ForcePollWidgets);
                                                                 }
                                                             }
                                                         });
                                                         context_menu_visible = false;
                                                     }
                                                    crate::renderer::ContextMenuItem::About => {
                                                           if about.is_none() {
                                                               match secondary_window::SecondaryWindow::create(
                                                                   target,
                                                                   "About Fasty",
                                                                   300.0,
                                                                   200.0,
                                                                   &renderer,
                                                                   &config.font.family,
                                                               ) {
                                                                   Ok(mut sw) => {
                                                                       #[cfg(target_os = "windows")]
                                                                       {
                                                                           sw.renderer.set_dirty(true);
                                                                           sw.renderer.render_about(&get_current_version(), false, config.opacity);
                                                                           sw.window.set_visible(true);
                                                                           sw.window.focus_window();
                                                                       }
                                                                       about = Some(sw);
                                                                   }
                                                                   Err(e) => {
                                                                       tracing::error!("Failed to create about window: {:?}", e);
                                                                   }
                                                               }
                                                           } else {
                                                               about = None;
                                                           }
                                                         context_menu_visible = false;
                                                         context_menu_open_time = None;
                                                         context_menu_open_time_secs = None;
                                                         context_menu_hovered_idx = None;
                                                         mark_grid_dirty(&renderer, &mut app_dirty);
                                                     }
                                                    crate::renderer::ContextMenuItem::Copy => {
                                                        if let Some(sel) = tabs[active_tab_index].selection {
                                                            copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
                                                            toast = Some((
                                                                "✓  Text copied".to_string(),
                                                                std::time::Instant::now(),
                                                                1920,
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
                                                                Err(_e) => {
                                                                    eprintln!("fastty clipboard initialization failed: {:?}", _e);
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
                                                                        tabs[active_tab_index].scroll_target = 0.0;
                                                                        tabs[active_tab_index].terminal_state.lock().write_to_pty(&paste_bytes);
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    eprintln!("fastty clipboard get_text failed: {:?}", e);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    crate::renderer::ContextMenuItem::NewTab => {
                                                        let new_tab_count = tabs.len() + 1;
                                                        let padding_top = get_padding_top(new_tab_count);
                                                        let physical_size = window_for_redraw.inner_size();
                                                        let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                            let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / cell_height).floor().max(1.0)) as usize;
                                                        
                                                        match create_new_tab(
                                                            &shell,
                                                            &[],
                                                            None,
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
                                                    crate::renderer::ContextMenuItem::MoveToNewWindow => {}
                                                    crate::renderer::ContextMenuItem::Separator => {}
                                                    crate::renderer::ContextMenuItem::OpenLink => {
                                                        if let Some(selection_classifier::Classification::Url(u)) =
                                                            context_menu_classification.as_ref()
                                                        {
                                                            open_url(u);
                                                        }
                                                        context_menu_visible = false;
                                                    }
                                                    crate::renderer::ContextMenuItem::CopyWord | crate::renderer::ContextMenuItem::CopyHex => {
                                                        let text = match context_menu_classification.as_ref() {
                                                            Some(selection_classifier::Classification::Url(s))
                                                            | Some(selection_classifier::Classification::Email(s))
                                                            | Some(selection_classifier::Classification::Path(s))
                                                            | Some(selection_classifier::Classification::Hex(s))
                                                            | Some(selection_classifier::Classification::Word(s)) => Some(s.clone()),
                                                            None => None,
                                                        };
                                                        if let Some(text) = text {
                                                            if let Some(ref mut ctx) = clipboard.as_mut() {
                                                                let _ = ctx.set_text(text.clone());
                                                            } else {
                                                                match arboard::Clipboard::new() {
                                                                    Ok(mut ctx) => {
                                                                        let _ = ctx.set_text(text.clone());
                                                                        clipboard = Some(ctx);
                                                                    }
                                                                    Err(e) => {
                                                                        eprintln!("fastty clipboard init failed: {:?}", e);
                                                                    }
                                                                }
                                                            }
                                                            toast = Some((
                                                                "\u{2713}  Text copied".to_string(),
                                                                std::time::Instant::now(),
                                                                1920,
                                                            ));
                                                        }
                                                        context_menu_visible = false;
                                                    }
                                                    crate::renderer::ContextMenuItem::CopyLine => {
                                                        context_menu_visible = false;
                                                    }
                                                    crate::renderer::ContextMenuItem::CdHere => {
                                                        if let Some(selection_classifier::Classification::Path(p)) =
                                                            context_menu_classification.as_ref()
                                                        {
                                                            let resolved = if p.starts_with('/') || p.starts_with('~') {
                                                                Some(p.clone())
                                                            } else if let Some(cwd) = tab_live_cwd(&tabs[active_tab_index]) {
                                                                Some(cwd.join(p).to_string_lossy().into_owned())
                                                            } else {
                                                                None
                                                            };
                                                            if let Some(cwd) = resolved {
                                                                if let Ok(new_tab) = create_new_tab(
                                                                    &shell,
                                                                    &[],
                                                                    Some(&cwd),
                                                                    config.scrollback,
                                                                    config.font.clone(),
                                                                    cell_width,
                                                                    cell_height,
                                                                    shell_cols,
                                                                    shell_rows,
                                                                    proxy.clone(),
                                                                ) {
                                                                    tabs.push(new_tab);
                                                                    active_tab_index = tabs.len() - 1;
                                                                }
                                                            }
                                                        }
                                                        context_menu_visible = false;
                                                    }
                                                    crate::renderer::ContextMenuItem::OpenInEditor => {
                                                        if let Some(selection_classifier::Classification::Path(p)) =
                                                            context_menu_classification.as_ref()
                                                        {
                                                            let resolved = if p.starts_with('/') || p.starts_with('~') {
                                                                p.clone()
                                                            } else if let Some(cwd) = tab_live_cwd(&tabs[active_tab_index]) {
                                                                cwd.join(p).to_string_lossy().into_owned()
                                                            } else {
                                                                p.clone()
                                                            };
                                                            if let Err(e) = open_file_in_editor(std::path::Path::new(&resolved)) {
                                                                eprintln!("fastty open_file_in_editor failed for '{}': {:?}", resolved, e);
                                                            }
                                                        }
                                                        context_menu_visible = false;
                                                    }
                                                    crate::renderer::ContextMenuItem::OpenEmail => {
                                                        if let Some(selection_classifier::Classification::Email(e)) =
                                                            context_menu_classification.as_ref()
                                                        {
                                                            let target = if e.starts_with("mailto:") {
                                                                e.clone()
                                                            } else {
                                                                format!("mailto:{}", e)
                                                            };
                                                            open_url(&target);
                                                        }
                                                        context_menu_visible = false;
                                                    }
                                                }
                                            }
                                        }
                                    } else if button == MouseButton::Right {
                                        // Reposition menu
                                        let menu_items = get_context_menu_items(&tabs, active_tab_index, context_menu_is_about);
                                        let (menu_w, menu_h) = get_context_menu_size(&menu_items);
                                        let r = renderer.lock();
                                        let v_width = r.config.width as f64;
                                        drop(r);
                                        
                                        if context_menu_is_about {
                                            context_menu_x = 8.0;
                                            context_menu_y = 40.0;
                                            context_menu_open_time = Some(std::time::Instant::now());
                                            context_menu_open_time_secs = Some(start_time.elapsed().as_secs_f32());
                                            context_menu_hovered_idx = None;
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                            return;
                                        } else if current_mouse_y >= padding_top as f64 && current_mouse_x <= (v_width - 20.0) {
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
                                            app_dirty = true;
                                            return;
                                        }
                                    }
                                    context_menu_visible = false;
                                    context_menu_open_time = None;
                                    context_menu_open_time_secs = None;
                                    context_menu_hovered_idx = None;
                                    mark_grid_dirty(&renderer, &mut app_dirty);
                                }
                                return;
                            }

                            if state == ElementState::Pressed && button == MouseButton::Left {
                                let r_cfg = renderer.lock();
                                let vh = r_cfg.config.height as f32;
                                drop(r_cfg);
                                if current_mouse_y as f32 >= vh - bar_h {
                                    if let Some(idx) = bar_layout.hit_test(current_mouse_x as f32, current_mouse_y as f32) {
                                        let cwd = tabs[active_tab_index].terminal_state.lock()
                                            .shell_pid()
                                            .and_then(|pid| std::fs::read_link(format!("/proc/{}/cwd", pid)).ok())
                                            .or_else(|| tabs[active_tab_index].cwd.clone());
                                        let ctx = widgets::WidgetContext {
                                            active_tab_cwd: cwd.as_deref(),
                                            active_tab_git: tabs[active_tab_index].git_status.as_ref(),
                                            opacity: config.opacity,
                                        };
                                        if let Some(w) = bar_layout.widgets.get_mut(idx) {
                                            let action = w.on_click(&ctx);
                                            match action {
                                                widgets::ClickAction::None => {}
                                                widgets::ClickAction::CopyToClipboard(s) => {
                                                    if let Ok(mut cb) = arboard::Clipboard::new() {
                                                        let _ = cb.set_text(s);
                                                    }
                                                    toast = Some(("copied to clipboard".to_string(), std::time::Instant::now(), 1500));
                                                }
                                                widgets::ClickAction::RunCommand(s) => {
                                                    let _ = tabs[active_tab_index].terminal_state.lock().write_to_pty(s.as_bytes());
                                                }
                                                widgets::ClickAction::OpenUrl(s) => {
                                                    open_url(&s);
                                                }
                                                widgets::ClickAction::Custom => {}
                                                widgets::ClickAction::ShowActionsMenu => {
                                                    if let Some(menu_items) = w.get_context_menu_items() {
                                                        context_menu_classification = None;
                                                        context_menu_items = menu_items;
                                                        context_menu_is_about = false;
                                                        context_menu_scroll_y = 0.0;
                                                        let (menu_w, menu_h) = get_context_menu_size(&context_menu_items);
                                                        context_menu_x = current_mouse_x;
                                                        context_menu_y = current_mouse_y;
                                                        context_menu_open_time = Some(std::time::Instant::now());
                                                        context_menu_open_time_secs = Some(start_time.elapsed().as_secs_f32());
                                                        
                                                        let v_width = renderer.lock().config.width as f64;
                                                        let v_height = window_for_redraw.inner_size().height as f64;
                                                        if context_menu_x + menu_w > v_width {
                                                            context_menu_x = v_width - menu_w - 4.0;
                                                        }
                                                        if context_menu_y + menu_h > v_height {
                                                            context_menu_y = context_menu_y - menu_h;
                                                            if context_menu_y < 0.0 {
                                                                context_menu_y = 4.0;
                                                            }
                                                        }
                                                        context_menu_visible = true;
                                                        context_menu_hovered_idx = None;
                                                    }
                                                }
                                            }
                                        }
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                        return;
                                    }
                                }
                            }

                            if renaming_tab.is_some() {
                                if state == ElementState::Pressed && button == MouseButton::Left {
                                    if current_mouse_y >= padding_top as f64 {
                                        let idx = renaming_tab.take().unwrap();
                                        if rename_buffer.is_empty() {
                                            tabs[idx].custom_name = None;
                                        } else {
                                            tabs[idx].custom_name = Some(rename_buffer.clone());
                                        }
                                        rename_buffer.clear();
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                    }
                                }
                                return;
                            }

                            if state == ElementState::Pressed {
                                mouse_down_button = Some(button);
                            } else {
                                mouse_down_button = None;
                            }

                            let r = renderer.lock();
                            let v_width = r.config.width as f64;
                            drop(r);
                            let padding_top = get_padding_top(tabs.len());
                            let is_in_terminal_area = current_mouse_y > padding_top as f64 && current_mouse_x <= (v_width - 20.0);

                            if is_in_terminal_area {
                                let term_guard = tabs[active_tab_index].terminal_state.lock();
                                let mode = *term_guard.term().lock().mode();
                                let tui_owns_mouse = mode.intersects(
                                    alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                                    | alacritty_terminal::term::TermMode::MOUSE_MOTION  
                                    | alacritty_terminal::term::TermMode::SGR_MOUSE
                                    | alacritty_terminal::term::TermMode::MOUSE_DRAG
                                );
                                drop(term_guard);

                                let shift_active = modifiers.shift_key() || shift_held;

                                if tui_owns_mouse && !shift_active {
                                    let col = (((current_mouse_x as f32 - 10.0) / cell_width).floor() as i32)
                                        .clamp(0, shell_cols as i32 - 1) as usize;
                                    let row = (((current_mouse_y as f32 - padding_top) / cell_height).floor() as i32)
                                        .clamp(0, shell_rows as i32 - 1) as usize;
                                    let term_state = tabs[active_tab_index].terminal_state.lock();
                                    let mode = *term_state.term().lock().mode();
                                    send_mouse_event_to_pty(&term_state, button, state, col, row, mode);
                                    drop(term_state);
                                    return;
                                }
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
                                         let vw_f = v_width as f32;
                                         let is_hovering_close = chrome_layout::close_rect(vw_f).contains(current_mouse_x, current_mouse_y);
                                         let is_hovering_max = chrome_layout::max_rect(vw_f).contains(current_mouse_x, current_mouse_y);
                                         let is_hovering_min = chrome_layout::min_rect(vw_f).contains(current_mouse_x, current_mouse_y);
                                         let is_hovering_settings = chrome_layout::settings_rect(vw_f).contains(current_mouse_x, current_mouse_y);

                                         let is_update_available = update_available.lock().is_some();
                                         if is_update_available {
                                             let is_hovering_update = chrome_layout::update_rect(vw_f).contains(current_mouse_x, current_mouse_y);
                                             if is_hovering_update {
                                                 trigger_update(
                                                     &update_available,
                                                     &update_in_progress,
                                                     &update_completed,
                                                     &window_for_redraw,
                                                     proxy.clone(),
                                                     &tabs,
                                                     active_tab_index,
                                                 );
                                                 return;
                                             }
                                         }

                                         if is_hovering_close {
                                             target.exit();
                                             return;
                                         } else if is_hovering_max {
                                             macos_maximize::toggle_maximize(&window_for_redraw, &mut maximize_state);
                                             return;
                                         } else if is_hovering_min {
                                             window_for_redraw.set_minimized(true);
                                             return;
                                          } else if is_hovering_settings {
                                               if settings_sw.is_none() {
                                                    match Config::load() {
                                                        Ok(fresh) => { config = fresh; }
                                                        Err(e) => {
                                                            tracing::warn!("config: settings-open reload failed, keeping in-memory: {e}");
                                                            toast = Some((
                                                                "⚠ Not a valid config".to_string(),
                                                                std::time::Instant::now(),
                                                                4000,
                                                            ));
                                                        }
                                                    }
                                                    settings_family = config.font.family.clone();
                                                    settings_size = config.font.size;
                                                    settings_scrollback = config.scrollback.min(1000);
                                                    settings_theme = config.theme.clone().unwrap_or_else(|| "default".to_string());
                                                   settings_active_field = 0;
                                                    match secondary_window::SecondaryWindow::create(
                                                         target,
                                                         "fastty Settings",
                                                         400.0,
                                                         260.0,
                                                         &renderer,
                                                         &config.font.family,
                                                     ) {
                                                         Ok(mut sw) => {
                                                             #[cfg(target_os = "windows")]
                                                             {
                                                                 sw.renderer.set_dirty(true);
                                                                 sw.renderer.render_settings(
                                                                     &config.font.family,
                                                                     config.font.size,
                                                                     config.scrollback.min(1000),
                                                                     0,
                                                                     false, false, false, false, false, false, false, false,
                                                                     &system_fonts,
                                                                     0.0,
                                                                     None,
                                                                     &settings_theme,
                                                                     &themes_list,
                                                                     None,
                                                                     0.0,
                                                                     config.opacity,
                                                                 );
                                                                 sw.window.set_visible(true);
                                                                 sw.window.focus_window();
                                                             }
                                                             settings_sw = Some(sw);
                                                         }
                                                         Err(e) => {
                                                             tracing::error!("Failed to create settings window: {:?}", e);
                                                         }
                                                     }
                                                } else {
                                                    settings_sw = None;
                                                }
                                                app_dirty = true;
                                                return;
                                           }

                                         // 2. Check tab clicks & close tab clicks & new tab click
                                         let tab_start_x = chrome_layout::tab_start_x() as f64;
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
                                         if tabs_len > 1 && current_mouse_x >= tab_start_x && current_mouse_x < tab_start_x + tabs_total_width {
                                             let clicked_tab_idx = ((current_mouse_x - tab_start_x) / tab_width) as usize;
                                             if clicked_tab_idx < tabs_len {
                                                 tab_ctx_visible = false;
                                                 context_menu_visible = false;
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
                                                      dragging_tab = Some(clicked_tab_idx);
                                                      drag_start_x = current_mouse_x;
                                                      drag_start_y = current_mouse_y;
                                                      drag_current_x = current_mouse_x;
                                                      drag_tab_offset = current_mouse_x - tab_x;
                                                      drag_threshold_passed = false;
                                                      // Do not confine the cursor so dragging it out can trigger window pop-out.
                                                      let _ = window_for_redraw.set_cursor_grab(CursorGrabMode::None);
                                                  }
                                                 mark_grid_dirty(&renderer, &mut app_dirty);
                                             }
                                             return;
                                         }

                                         // Check new tab button click
                                         let new_tab_x = tab_start_x + tabs_total_width;
                                         if tabs_len > 1 && current_mouse_x >= new_tab_x && current_mouse_x < new_tab_x + 32.0 {
                                             let new_tab_count = tabs.len() + 1;
                                             let padding_top = get_padding_top(new_tab_count);
                                             let physical_size = window_for_redraw.inner_size();
                                             let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                                        let new_rows = (((physical_size.height as f32 - (padding_top + get_padding_bottom())) / cell_height).floor().max(1.0)) as usize;
                                             
                                             match create_new_tab(
                                                 &shell,
                                                 &[],
                                                 None,
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
                                             mark_grid_dirty(&renderer, &mut app_dirty);
                                             return;
                                         }

                                         // 3. Otherwise (blank space click), drag the window
                                         // Don't drag the window if clicking near the control buttons region
                                         if current_mouse_x < (chrome_layout::drag_max_x(v_width as f32) as f64) {
                                             let now = std::time::Instant::now();
                                             let is_double_click = if let Some(last_time) = last_click_time {
                                                 now.duration_since(last_time) < std::time::Duration::from_millis(300)
                                             } else {
                                                 false
                                             };
                                             last_click_time = Some(now);

                                             if is_double_click {
                                                 macos_maximize::toggle_maximize(&window_for_redraw, &mut maximize_state);
                                             } else {
                                                 let _ = window_for_redraw.drag_window();
                                             }
                                         }
                                         return;
                                     }

                                    const TOPBAR_HEIGHT: f32 = 30.0;
                                    let scrollbar_top_margin = TOPBAR_HEIGHT;
                                    let show_scrollbar = {
                                        let term_guard = tabs[active_tab_index].terminal_state.lock();
                                        let mode = *term_guard.term().lock().mode();
                                        drop(term_guard);
                                        let tui_owns_mouse = mode.intersects(
                                            alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                                            | alacritty_terminal::term::TermMode::MOUSE_MOTION
                                            | alacritty_terminal::term::TermMode::SGR_MOUSE
                                            | alacritty_terminal::term::TermMode::MOUSE_DRAG
                                        );
                                        let alt_screen_active = mode.contains(alacritty_terminal::term::TermMode::ALT_SCREEN);
                                        !tui_owns_mouse && !alt_screen_active
                                    };
                                    let is_hovering_scrollbar = show_scrollbar && current_mouse_y > scrollbar_top_margin as f64 && current_mouse_x > (v_width - 20.0);
                                    if is_hovering_scrollbar {
                                        let term = tabs[active_tab_index].terminal_state.lock();
                                        let history_size = term.history_size() as f32;
                                        let visible_rows = shell_rows as f32;
                                        drop(term);

                                        let total_lines = visible_rows + history_size;
                                        if total_lines > 0.0 {
                                            let ratio = visible_rows / total_lines;
                                            let track_h = v_height - scrollbar_top_margin - 2.0;
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
                                                app_dirty = true;
                                            }
                                        }
                                    } else if let Some(uri) = detect_hovered_hyperlink(
                                        current_mouse_x, current_mouse_y,
                                        &tabs[active_tab_index].terminal_state,
                                        tabs[active_tab_index].scroll_current,
                                        cell_width,
                                        cell_height,
                                        shell_cols,
                                        shell_rows,
                                        padding_top,
                                    ) {
                                        // OSC 8 hyperlink: open URL on plain click
                                        tabs[active_tab_index].pending_hyperlink_open = Some(uri.clone());
                                        tabs[active_tab_index].hyperlink_press_pos = Some((current_mouse_x, current_mouse_y));
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
                                        let term_state = tabs[active_tab_index].terminal_state.lock();
                                        let scroll_fraction = tabs[active_tab_index].scroll_current - term_state.display_offset() as f32;
                                        let display_offset = term_state.display_offset();
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
                                        let history_size = term_state.history_size();
                                        let in_history = click_point.line.0 < -(history_size as i32);
                                        drop(term_state);
                                        let now = std::time::Instant::now();
                                        let same_cell = last_term_click_cell == Some((click_point.line.0, click_point.column.0));
                                        let is_double = same_cell
                                            && last_term_click_time
                                                .map(|t| now.duration_since(t) < std::time::Duration::from_millis(300))
                                                .unwrap_or(false);
                                        last_term_click_time = Some(now);
                                        last_term_click_cell = Some((click_point.line.0, click_point.column.0));

                                        if is_double && !in_history {
                                            let token_info = {
                                                let term = tabs[active_tab_index].terminal_state.lock();
                                                let term_guard = term.term().lock();
                                                extract_token(term_guard.grid(), click_point, shell_cols)
                                            };
                                            if let Some((_token, start_col, end_col)) = token_info {
                                                let start = alacritty_terminal::index::Point::new(
                                                    click_point.line,
                                                    alacritty_terminal::index::Column(start_col),
                                                );
                                                let end = alacritty_terminal::index::Point::new(
                                                    click_point.line,
                                                    alacritty_terminal::index::Column(end_col),
                                                );
                                                tabs[active_tab_index].selection = Some(renderer::Selection { start, end });
                                                let selection = tabs[active_tab_index].selection.unwrap();
                                                copy_selection_to_clipboard(
                                                    &tabs[active_tab_index].terminal_state,
                                                    selection,
                                                    shell_cols,
                                                    shell_rows,
                                                    &mut clipboard,
                                                );
                                                toast = Some((
                                                    "\u{2713}  Text copied".to_string(),
                                                    std::time::Instant::now(),
                                                    1920,
                                                ));
                                            }
                                        } else {
                                            tabs[active_tab_index].selection_start_pos = Some((current_mouse_x, current_mouse_y));
                                            tabs[active_tab_index].is_selecting_text = false;
                                        }
                                    }
                                } else {
                                    tabs[active_tab_index].is_dragging = false;
                                    is_dragging_scrollbar = false;

                                    if let Some(drag_idx) = dragging_tab {
                                        let _ = window_for_redraw.set_cursor_grab(CursorGrabMode::None);

                                        let vw = renderer.lock().config.width as f64;

                                        pending_pop_out = None;

                                        if let Some(target_win) = hovered_window {
                                            if target_win != window_for_redraw.id() {
                                                // Dropped on another window!
                                                let tab_opt = cross_window_drag::take().map(|d| d.tab).or_else(|| {
                                                    if tabs.len() >= 2 {
                                                        Some(tabs.remove(drag_idx))
                                                    } else {
                                                        None
                                                    }
                                                });

                                                if let Some(tab) = tab_opt {
                                                    if let Some(target_wc) = popped_out_windows.get_mut(&target_win) {
                                                        let tab_start_x = chrome_layout::tab_start_x() as f64;
                                                        let target_vw = target_wc.window.inner_size().width as f64;
                                                        let path_center_x = target_vw / 2.0;
                                                        let tab_area_max_x = path_center_x - 40.0;
                                                        let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                                        let tabs_len = target_wc.tabs.len() + 1;
                                                        let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                                                        let insert_idx = compute_drop_target(target_wc.drag_current_x, tab_start_x, tab_width, tabs_len);
                                                        
                                                        target_wc.tabs.insert(insert_idx, tab);
                                                        target_wc.active_tab_index = insert_idx;
                                                        let (cols, rows) = resize_all_tabs(&target_wc.tabs, target_wc.window.inner_size().width, target_wc.window.inner_size().height, target_wc.cell_width, target_wc.cell_height);
                                                        target_wc.shell_cols = cols;
                                                        target_wc.shell_rows = rows;
                                                        target_wc.renderer.lock().set_dirty(true);
                                                        target_wc.window.request_redraw();
                                                    }
                                                    
                                                    if tabs.is_empty() {
                                                        target.exit();
                                                        return;
                                                    }
                                                    let physical_size = window_for_redraw.inner_size();
                                                    let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                    shell_cols = cols;
                                                    shell_rows = rows;
                                                }
                                            } else {
                                                // Dropped on itself!
                                                if let Some(drag) = cross_window_drag::take() {
                                                    let tab_start_x = chrome_layout::tab_start_x() as f64;
                                                    let path_center_x = vw / 2.0;
                                                    let tab_area_max_x = path_center_x - 40.0;
                                                    let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                                    let tabs_len = tabs.len() + 1;
                                                    let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                                                    let target = compute_drop_target(current_mouse_x, tab_start_x, tab_width, tabs_len);
                                                    let target_idx = target.min(tabs.len());
                                                    tabs.insert(target_idx, drag.tab);
                                                    active_tab_index = target_idx;
                                                    let physical_size = window_for_redraw.inner_size();
                                                    let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                    shell_cols = cols;
                                                    shell_rows = rows;
                                                } else if drag_threshold_passed {
                                                    let tab_start_x = chrome_layout::tab_start_x() as f64;
                                                    let path_center_x = vw / 2.0;
                                                    let tab_area_max_x = path_center_x - 40.0;
                                                    let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                                                    let tabs_len = tabs.len();
                                                    let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                                                    let target = compute_drop_target(current_mouse_x, tab_start_x, tab_width, tabs_len);
                                                    if target != drag_idx {
                                                        let tab = tabs.remove(drag_idx);
                                                        tabs.insert(target, tab);
                                                        active_tab_index = target;
                                                        let physical_size = window_for_redraw.inner_size();
                                                        let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                        shell_cols = cols;
                                                        shell_rows = rows;
                                                    }
                                                } else {
                                                    active_tab_index = drag_idx;
                                                }
                                            }
                                        } else if cross_window_drag::is_active() {
                                            pending_new_window_from_drag = true;
                                        } else if !drag_threshold_passed {
                                            active_tab_index = drag_idx;
                                        }

                                        dragging_tab = None;
                                        drag_threshold_passed = false;
                                        mark_grid_dirty(&renderer, &mut app_dirty);
                                        return;
                                    }

                                    if let Some(uri) = tabs[active_tab_index].pending_hyperlink_open.take() {
                                        tabs[active_tab_index].hyperlink_press_pos = None;
                                        open_url(&uri);
                                    }

                                    if tabs[active_tab_index].is_selecting_text {
                                        tabs[active_tab_index].is_selecting_text = false;
                                        if let Some(sel) = tabs[active_tab_index].selection {
                                            copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
                                            toast = Some((
                                                "✓  Text copied".to_string(),
                                                std::time::Instant::now(),
                                                1920,
                                            ));
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                        }
                                    } else if tabs[active_tab_index].selection_start_pos.is_some() {
                                        // Simple click release (no drag occurred): clear selection
                                        tabs[active_tab_index].selection = None;
                                        mark_grid_dirty(&renderer, &mut app_dirty);

                                        // Click-to-cursor-position in normal shell mode (no TUI app owns mouse):
                                        let padding_top = get_padding_top(tabs.len());
                                        let is_in_terminal_area = current_mouse_y > padding_top as f64 && current_mouse_x <= (v_width - 20.0);
                                        if is_in_terminal_area {
                                            let term_state = tabs[active_tab_index].terminal_state.lock();
                                            let mode = *term_state.term().lock().mode();
                                            let tui_owns_mouse = mode.intersects(
                                                alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                                                | alacritty_terminal::term::TermMode::MOUSE_MOTION  
                                                | alacritty_terminal::term::TermMode::SGR_MOUSE
                                                | alacritty_terminal::term::TermMode::MOUSE_DRAG
                                            );
                                            if !tui_owns_mouse {
                                                let scroll_fraction = tabs[active_tab_index].scroll_current - term_state.display_offset() as f32;
                                                let display_offset = term_state.display_offset();
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

                                                let click_col = click_point.column.0 as i32;
                                                let click_row = click_point.line.0;
                                                
                                                let term_guard = term_state.term().lock();
                                                let cursor_col = term_guard.grid().cursor.point.column.0 as i32;
                                                let cursor_row = term_guard.grid().cursor.point.line.0;

                                                if click_row == cursor_row {
                                                    let delta = click_col - cursor_col;
                                                    if delta > 0 {
                                                        // Click is to the RIGHT of cursor — send → arrows
                                                        for _ in 0..delta {
                                                            term_state.write_to_pty(b"\x1b[C");  // cursor forward
                                                        }
                                                    } else if delta < 0 {
                                                        // Click is to the LEFT of cursor — send ← arrows
                                                        for _ in 0..delta.abs() {
                                                            term_state.write_to_pty(b"\x1b[D");  // cursor backward
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    tabs[active_tab_index].selection_start_pos = None;
                                }
                            } else if button == MouseButton::Right {
                                let pressed = state == ElementState::Pressed;
                                if pressed {
                                    let r = renderer.lock();
                                    let v_width = r.config.width as f64;
                                    drop(r);

                                    if current_mouse_y < padding_top as f64 && current_mouse_x <= 36.0 {
                                        context_menu_x = 8.0;
                                        context_menu_y = 40.0;
                                        context_menu_is_about = true;
                                        context_menu_open_time = Some(std::time::Instant::now());
                                        context_menu_open_time_secs = Some(start_time.elapsed().as_secs_f32());
                                        context_menu_visible = true;
                                        context_menu_hovered_idx = None;
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                    } else if current_mouse_y < padding_top as f64 {
                                        let path_center_x = v_width / 2.0;
                                        let tab_area_max_x = path_center_x - 40.0;
                                        let tab_area_width = tab_area_max_x - 36.0 - 32.0;
                                        let tabs_len = tabs.len();
                                        let tab_width = if tabs_len > 0 {
                                            (tab_area_width / tabs_len as f64).clamp(80.0, 160.0)
                                        } else { 160.0 };
                                        let tabs_total_width = tabs_len as f64 * tab_width;
                                        if current_mouse_x >= 36.0 && current_mouse_x < 36.0 + tabs_total_width {
                                            let clicked_idx = ((current_mouse_x - 36.0) / tab_width) as usize;
                                            if clicked_idx < tabs_len {
                                                context_menu_visible = false;
                                                tab_ctx_tab_idx = clicked_idx;
                                                tab_ctx_x = current_mouse_x;
                                                tab_ctx_y = current_mouse_y;
                                                tab_ctx_visible = true;
                                                tab_ctx_hovered = None;
                                                renderer.lock().set_dirty(true);
                                                app_dirty = true;
                                            }
                                        }
                                    } else if current_mouse_y >= padding_top as f64 && current_mouse_x <= (v_width - 20.0) {
                                        context_menu_is_about = false;
                                        tab_ctx_visible = false;
                                        let cwd_resolvable = tab_live_cwd(&tabs[active_tab_index]).is_some();
                                        let (classification, menu_items) = {
                                            let term_state = tabs[active_tab_index].terminal_state.lock();
                                            let display_offset = term_state.display_offset();
                                            let click_point = mouse_to_grid_point(
                                                current_mouse_x,
                                                current_mouse_y,
                                                cell_width,
                                                cell_height,
                                                tabs[active_tab_index].scroll_current,
                                                display_offset,
                                                shell_cols,
                                                shell_rows,
                                                padding_top,
                                            );
                                            let in_history = click_point.line.0 < -(term_state.history_size() as i32);
                                            let classification = if in_history {
                                                None
                                            } else {
                                                let term_guard = term_state.term().lock();
                                                selection_classifier::classify_at_point(term_guard.grid(), click_point, shell_cols)
                                            };
                                            let items = build_smart_menu(
                                                classification.as_ref(),
                                                tabs[active_tab_index].selection.is_some(),
                                                tabs.len(),
                                                cwd_resolvable,
                                            );
                                            (classification, items)
                                        };
                                        context_menu_classification = classification;
                                        context_menu_items = menu_items;
                                        context_menu_scroll_y = 0.0;
                                        let (menu_w, menu_h) = get_context_menu_size(&context_menu_items);

                                        context_menu_x = current_mouse_x;
                                        context_menu_y = current_mouse_y;
                                        context_menu_open_time = Some(std::time::Instant::now());
                                        context_menu_open_time_secs = Some(start_time.elapsed().as_secs_f32());

                                        let v_height = window_for_redraw.inner_size().height as f64;
                                        if context_menu_x + menu_w > v_width {
                                            context_menu_x = v_width - menu_w - 4.0;
                                        }
                                        if context_menu_y + menu_h > v_height {
                                            context_menu_y = v_height - menu_h - 4.0;
                                        }

                                        context_menu_visible = true;
                                        context_menu_hovered_idx = None;
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                    }
                                }
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            let delta_y = position.y - last_cursor_y;
                            last_cursor_y = position.y;
                            current_mouse_x = position.x;
                            current_mouse_y = position.y;

                            if let Some((px, py)) = tabs[active_tab_index].hyperlink_press_pos {
                                let dx = current_mouse_x - px;
                                let dy = current_mouse_y - py;
                                if dx * dx + dy * dy > 25.0 {
                                    tabs[active_tab_index].pending_hyperlink_open = None;
                                    tabs[active_tab_index].hyperlink_press_pos = None;
                                }
                            }

                            if context_menu_visible {
                                let menu_items = if context_menu_is_about {
                                    vec![crate::renderer::ContextMenuItem::About]
                                } else {
                                    context_menu_items.clone()
                                };
                                let (menu_w, menu_h) = get_context_menu_size(&menu_items);
                                if current_mouse_x >= context_menu_x && current_mouse_x <= context_menu_x + menu_w
                                   && current_mouse_y >= context_menu_y && current_mouse_y <= context_menu_y + menu_h {
                                    let relative_y = (current_mouse_y - context_menu_y) as f32;
                                    context_menu_hovered_idx = get_menu_item_at_y(&menu_items, relative_y, context_menu_scroll_y as f32, menu_h as f32);
                                } else {
                                    context_menu_hovered_idx = None;
                                }
                                renderer.lock().set_dirty(true);
                                app_dirty = true;
                            }

                            if tab_ctx_visible {
                                let menu_w = 200.0f64;
                                let menu_h = 76.0f64;
                                if current_mouse_x >= tab_ctx_x && current_mouse_x < tab_ctx_x + menu_w
                                    && current_mouse_y >= tab_ctx_y && current_mouse_y < tab_ctx_y + menu_h
                                {
                                    let rel_y = current_mouse_y - tab_ctx_y;
                                    tab_ctx_hovered = if rel_y >= 6.0 && rel_y < 38.0 {
                                        Some(0)
                                    } else if rel_y >= 38.0 && rel_y < 70.0 {
                                        Some(1)
                                    } else {
                                        None
                                    };
                                } else {
                                    tab_ctx_hovered = None;
                                }
                                renderer.lock().set_dirty(true);
                                app_dirty = true;
                            }

                            let r = renderer.lock();
                            let v_width = r.config.width as f64;
                            let v_height = r.config.height as f32;
                            drop(r);

                            let padding_top = get_padding_top(tabs.len());
                            let is_in_terminal_area = current_mouse_y > padding_top as f64 && current_mouse_x <= (v_width - 20.0);

                            let (tui_owns_mouse, shift_active, term_mode) = if is_in_terminal_area {
                                let term_guard = tabs[active_tab_index].terminal_state.lock();
                                let mode = *term_guard.term().lock().mode();
                                let tui_owns = mode.intersects(
                                    alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                                    | alacritty_terminal::term::TermMode::MOUSE_MOTION  
                                    | alacritty_terminal::term::TermMode::SGR_MOUSE
                                    | alacritty_terminal::term::TermMode::MOUSE_DRAG
                                );
                                drop(term_guard);
                                let shift = modifiers.shift_key() || shift_held;
                                (tui_owns, shift, Some(mode))
                            } else {
                                (false, false, None)
                            };

                            if is_in_terminal_area && tui_owns_mouse && !shift_active {
                                if let Some(mode) = term_mode {
                                    let col = (((current_mouse_x as f32 - 10.0) / cell_width).floor() as i32)
                                        .clamp(0, shell_cols as i32 - 1) as usize;
                                    let row = (((current_mouse_y as f32 - padding_top) / cell_height).floor() as i32)
                                        .clamp(0, shell_rows as i32 - 1) as usize;
                                    
                                    let term_state = tabs[active_tab_index].terminal_state.lock();
                                    send_drag_event_to_pty(&term_state, mouse_down_button, col, row, mode);
                                    drop(term_state);
                                }
                                window_for_redraw.set_cursor(winit::window::CursorIcon::Default);
                                return;
                            }

                            let is_dragging_anything = tabs[active_tab_index].is_dragging || is_dragging_scrollbar || tabs[active_tab_index].selection_start_pos.is_some();
                            const RESIZE_BORDER_WIDTH: f64 = 8.0;
                             
                            if context_menu_visible || tab_ctx_visible {
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
                            let old_hover_update = hover_update;
                            let old_hovered_tab = hovered_tab_index;
                            let old_hovered_close = hovered_close_tab_index;
                            let old_hover_new = hover_new_tab;

                            if context_menu_visible || tab_ctx_visible {
                                hover_close = false;
                                hover_max = false;
                                hover_min = false;
                                hover_settings = false;
                                hover_update = false;
                                hovered_tab_index = None;
                                hovered_close_tab_index = None;
                                hover_new_tab = false;
                            } else {
                                let vw_f = v_width as f32;
                                hover_close = chrome_layout::close_rect(vw_f).contains(current_mouse_x, current_mouse_y);
                                hover_max = chrome_layout::max_rect(vw_f).contains(current_mouse_x, current_mouse_y);
                                hover_min = chrome_layout::min_rect(vw_f).contains(current_mouse_x, current_mouse_y);
                                hover_settings = chrome_layout::settings_rect(vw_f).contains(current_mouse_x, current_mouse_y);

                                let is_update_available = update_available.lock().is_some();
                                if is_update_available {
                                    hover_update = chrome_layout::update_rect(vw_f).contains(current_mouse_x, current_mouse_y);
                                } else {
                                    hover_update = false;
                                }

                                hovered_tab_index = None;
                                hovered_close_tab_index = None;
                                hover_new_tab = false;

                                if current_mouse_y >= 0.0 && current_mouse_y <= 40.0 {
                                    let tab_start_x = chrome_layout::tab_start_x() as f64;
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
                                    if tabs_len > 1 && current_mouse_x >= tab_start_x && current_mouse_x < tab_start_x + tabs_total_width {
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
                            }

                            if hover_close != old_hover_close
                                || hover_max != old_hover_max
                                || hover_min != old_hover_min
                                || hover_settings != old_hover_settings
                                || hover_update != old_hover_update
                                || hovered_tab_index != old_hovered_tab
                                || hovered_close_tab_index != old_hovered_close
                                || hover_new_tab != old_hover_new
                            {
                                renderer.lock().set_dirty(true);
                                app_dirty = true;
                            }

                            if let Some(drag_idx) = dragging_tab {
                                if !drag_threshold_passed && ((current_mouse_x - drag_start_x).abs() > 5.0 || (current_mouse_y - drag_start_y).abs() > 5.0) {
                                    drag_threshold_passed = true;
                                }
                                if drag_threshold_passed {
                                    drag_current_x = current_mouse_x;
                                    let (vw, vh) = {
                                        let r = renderer.lock();
                                        (r.config.width as f64, r.config.height as f64)
                                    };

                                    // On Linux (X11/Wayland), an implicit pointer grab
                                    // keeps CursorMoved events flowing with out-of-bounds
                                    // coordinates while the mouse button is held. CursorLeft
                                    // does NOT fire in this scenario. Track when the cursor
                                    // has physically left the window so the MouseInput
                                    // release handler treats it as a drop outside.
                                    let cursor_outside_window =
                                        current_mouse_x < 0.0 || current_mouse_x >= vw
                                        || current_mouse_y < 0.0 || current_mouse_y >= vh;
                                    if cursor_outside_window && hovered_window == Some(window_for_redraw.id()) {
                                        hovered_window = None;
                                    } else if !cursor_outside_window && hovered_window.is_none() {
                                        // Cursor came back inside the window
                                        hovered_window = Some(window_for_redraw.id());
                                    }

                                    if cursor_outside_window {
                                        if pending_pop_out.is_none() && tabs.len() >= 2 {
                                            pending_pop_out = Some(drag_idx);
                                            let tab = tabs.remove(drag_idx);
                                            *cross_window_drag::DRAG.lock() = Some(cross_window_drag::CrossWindowDrag {
                                                source_window_id: window_for_redraw.id(),
                                                tab,
                                            });
                                            if active_tab_index >= tabs.len() && !tabs.is_empty() {
                                                active_tab_index = tabs.len() - 1;
                                            }
                                            let physical_size = window_for_redraw.inner_size();
                                            let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                            shell_cols = cols;
                                            shell_rows = rows;
                                        }
                                        if cross_window_drag::is_active() {
                                            dragging_tab = None;
                                            drag_threshold_passed = false;
                                            pending_pop_out = None;
                                            let _ = window_for_redraw.set_cursor_grab(winit::window::CursorGrabMode::None);
                                            pending_new_window_from_drag = true;
                                        }
                                    } else if cursor_outside_tab_area(current_mouse_x, current_mouse_y, vw, vh) {
                                        if pending_pop_out.is_none() && tabs.len() >= 2 {
                                            pending_pop_out = Some(drag_idx);
                                            let tab = tabs.remove(drag_idx);
                                            *cross_window_drag::DRAG.lock() = Some(cross_window_drag::CrossWindowDrag {
                                                source_window_id: window_for_redraw.id(),
                                                tab,
                                            });
                                            if active_tab_index >= tabs.len() && !tabs.is_empty() {
                                                active_tab_index = tabs.len() - 1;
                                            }
                                            let physical_size = window_for_redraw.inner_size();
                                            let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                            shell_cols = cols;
                                            shell_rows = rows;
                                        }
                                    } else {
                                        if let Some(original_idx) = pending_pop_out.take() {
                                            if let Some(drag) = cross_window_drag::take() {
                                                let insert_idx = original_idx.min(tabs.len());
                                                tabs.insert(insert_idx, drag.tab);
                                                active_tab_index = insert_idx;
                                                let physical_size = window_for_redraw.inner_size();
                                                let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                                                shell_cols = cols;
                                                shell_rows = rows;
                                            }
                                        }
                                    }
                                    renderer.lock().set_dirty(true);
                                    app_dirty = true;
                                }
                            }

                            if is_dragging_scrollbar {
                                let term = tabs[active_tab_index].terminal_state.lock();
                                let history_size = term.history_size() as f32;
                                let visible_rows = shell_rows as f32;
                                drop(term);

                                let total_lines = visible_rows + history_size;
                                if total_lines > 0.0 {
                                    let ratio = visible_rows / total_lines;
                                    const TOPBAR_HEIGHT: f32 = 30.0;
                                    let scrollbar_top_margin = TOPBAR_HEIGHT;
                                    let track_h = v_height - scrollbar_top_margin - 2.0;
                                    let thumb_h = (track_h * ratio).max(20.0).min(track_h);
                                    let track_center = track_h - thumb_h;

                                    if track_center > 0.0 {
                                        let new_thumb_y = (current_mouse_y as f32 - scrollbar_top_margin - scrollbar_drag_offset_y).clamp(0.0, track_center);
                                        let scroll_ratio = 1.0 - (new_thumb_y / track_center);
                                        tabs[active_tab_index].scroll_target = scroll_ratio * history_size;
                                    }
                                }
                                app_dirty = true;
                            } else if let Some((sx, sy)) = tabs[active_tab_index].selection_start_pos {
                                if !tabs[active_tab_index].is_selecting_text {
                                    if (current_mouse_x - sx).abs() > 2.0 || (current_mouse_y - sy).abs() > 2.0 {
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
                                        mark_grid_dirty(&renderer, &mut app_dirty);
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
                                            mark_grid_dirty(&renderer, &mut app_dirty);
                                        }
                                    }
                                }
                            } else if tabs[active_tab_index].is_dragging {
                                let drag_speed = 1.0f32;
                                let delta_lines = (delta_y as f32 / cell_height) * drag_speed;
                                let max_history = tabs[active_tab_index].terminal_state.lock().history_size() as f32;
                                tabs[active_tab_index].scroll_target = (tabs[active_tab_index].scroll_target + delta_lines).clamp(0.0, max_history);
                                app_dirty = true;
                            }

                            let (new_hover, new_hover_text) = detect_hovered_url(
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
                                tabs[active_tab_index].hovered_url_text = new_hover_text;
                                mark_grid_dirty(&renderer, &mut app_dirty);
                            }
                            let new_hyperlink = detect_hovered_hyperlink(
                                current_mouse_x, current_mouse_y,
                                &tabs[active_tab_index].terminal_state,
                                tabs[active_tab_index].scroll_current,
                                cell_width, cell_height, shell_cols, shell_rows, padding_top,
                            );
                            if tabs[active_tab_index].hovered_hyperlink != new_hyperlink {
                                tabs[active_tab_index].hovered_hyperlink = new_hyperlink;
                                let mut r = renderer.lock();
                                r.set_dirty(true);
                                app_dirty = true;
                            }
                        }
                        WindowEvent::MouseWheel { delta, .. } => {
                            if context_menu_visible {
                                let is_git_actions_menu = context_menu_items.iter().any(|item| matches!(item, crate::renderer::ContextMenuItem::GithubActionInfo {..} | crate::renderer::ContextMenuItem::CommandItem {..}));
                                let mut total_menu_h = 12.0f64;
                                for item in &context_menu_items {
                                    total_menu_h += match item {
                                        crate::renderer::ContextMenuItem::Separator => 9.0,
                                        _ => 32.0,
                                    };
                                }
                                let menu_h = if is_git_actions_menu {
                                    320.0f64
                                } else {
                                    total_menu_h
                                };
                                let lines = match delta {
                                    MouseScrollDelta::LineDelta(_, y) => y,
                                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / cell_height,
                                };
                                let scroll_speed = 24.0f64;
                                context_menu_scroll_y = (context_menu_scroll_y - lines as f64 * scroll_speed)
                                    .clamp(0.0, (total_menu_h - menu_h).max(0.0));
                                renderer.lock().set_dirty(true);
                                app_dirty = true;
                                return;
                            }
                            if tab_ctx_visible {
                                return;
                            }
                            last_scroll_event_time = Some(std::time::Instant::now());
                            renderer.lock().set_dirty(true);

                            let lines = match delta {
                                MouseScrollDelta::LineDelta(_, y) => y,
                                MouseScrollDelta::PixelDelta(pos) => {
                                    pos.y as f32 / cell_height
                                }
                            };

                            let scroll_speed = 1.0f32;

                            let delta_scroll = match delta {
                                MouseScrollDelta::LineDelta(_, _) => lines * scroll_speed,
                                MouseScrollDelta::PixelDelta(_) => lines,
                            };

                            let r = renderer.lock();
                            let v_width = r.config.width as f64;
                            drop(r);
                            let padding_top = get_padding_top(tabs.len());
                            let is_in_term = current_mouse_y > padding_top as f64 && current_mouse_x <= (v_width - 20.0);

                            let shift_active = modifiers.shift_key() || shift_held;

                            if lines != 0.0 && !shift_active {
                                let term_state = tabs[active_tab_index].terminal_state.lock();
                                let (has_sgr, has_click) = {
                                    let term_locked = term_state.term().lock();
                                    let mode = term_locked.mode();
                                    (
                                        mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE),
                                        mode.contains(alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK),
                                    )
                                };

                                if is_in_term && (has_sgr || has_click) {
                                    let padding_top = get_padding_top(tabs.len());
                                    let col = (((current_mouse_x as f32 - 10.0) / cell_width).floor() as i32)
                                        .clamp(0, shell_cols as i32 - 1) + 1;
                                    let row = (((current_mouse_y as f32 - padding_top) / cell_height).floor() as i32)
                                        .clamp(0, shell_rows as i32 - 1) + 1;

                                    if has_sgr {
                                        let seq = if lines > 0.0 {
                                            format!("\x1b[<64;{};{}M", col, row)
                                        } else {
                                            format!("\x1b[<65;{};{}M", col, row)
                                        };
                                        term_state.write_to_pty(seq.as_bytes());
                                    } else {
                                        let btn = if lines > 0.0 { 96 } else { 97 };
                                        let col_byte = (col.clamp(1, 223) as u8) + 32;
                                        let row_byte = (row.clamp(1, 223) as u8) + 32;
                                        term_state.write_to_pty(&[0x1b, b'[', b'M', btn, col_byte, row_byte]);
                                    }
                                } else {
                                    drop(term_state);
                                    scroll_velocity += delta_scroll;
                                }
                            } else {
                                scroll_velocity += delta_scroll;
                            }
                            app_dirty = true;
                        }
                        WindowEvent::CursorLeft { .. } => {
                            window_for_redraw.set_cursor(winit::window::CursorIcon::Default);

                            // Update hovered_window tracking
                            if hovered_window == Some(window_for_redraw.id()) {
                                hovered_window = None;
                            }

                            // If we're mid-drag and the tab has been popped into
                            // cross_window_drag, the user has dragged outside the
                            // window.  Immediately create a new window — we will
                            // never get a MouseInput::Released because the cursor
                            // is no longer over this window.
                            if dragging_tab.is_some() && cross_window_drag::is_active() {
                                dragging_tab = None;
                                drag_threshold_passed = false;
                                pending_pop_out = None;
                                let _ = window_for_redraw.set_cursor_grab(CursorGrabMode::None);
                                pending_new_window_from_drag = true;
                                {
                                    let mut r = renderer.lock();
                                    r.set_dirty(true);
                                    r.grid_dirty = true;
                                }
                                app_dirty = true;
                            }

                            let old_hover_close = hover_close;
                            let old_hover_max = hover_max;
                            let old_hover_min = hover_min;
                            let old_hover_settings = hover_settings;
                            let old_hover_update = hover_update;

                            hover_close = false;
                            hover_max = false;
                            hover_min = false;
                            hover_settings = false;
                            hover_update = false;

                            let url_changed = tabs[active_tab_index].hovered_url.is_some();
                            tabs[active_tab_index].hovered_url = None;
                            tabs[active_tab_index].hovered_url_text = None;
                            let hyperlink_changed = tabs[active_tab_index].hovered_hyperlink.is_some();
                            tabs[active_tab_index].hovered_hyperlink = None;

                            if old_hover_close || old_hover_max || old_hover_min || old_hover_settings || old_hover_update || url_changed || hyperlink_changed {
                                let mut r = renderer.lock();
                                r.hover_update = false;
                                r.set_dirty(true);
                                app_dirty = true;
                            }
                        }
                        WindowEvent::CursorEntered { .. } => {
                            hovered_window = Some(window_for_redraw.id());
                        }

                        WindowEvent::Focused(focused) => {
                            window_focused = focused;
                            tracing::info!("Window focus changed: focused={}", focused);

                            // Send focus reporting sequences to PTY if requested by application
                            if let Some(tab) = tabs.get(active_tab_index) {
                                let term_state = tab.terminal_state.lock();
                                let term = term_state.term().lock();
                                if term.mode().contains(alacritty_terminal::term::TermMode::FOCUS_IN_OUT) {
                                    let seq = if focused { "\x1b[I" } else { "\x1b[O" };
                                    drop(term);
                                    term_state.write_to_pty(seq.as_bytes());
                                }
                            }

                            if !focused {
                                let old_hover_close = hover_close;
                                let old_hover_max = hover_max;
                                let old_hover_min = hover_min;
                                let old_hover_settings = hover_settings;
                                let old_hover_update = hover_update;

                                hover_close = false;
                                hover_max = false;
                                hover_min = false;
                                hover_settings = false;
                                hover_update = false;
                                tabs[active_tab_index].is_dragging = false;
                                is_dragging_scrollbar = false;

                                // If focus is lost mid-drag, the release event may
                                // never come. If the tab was already popped into
                                // cross_window_drag (user dragged it out), create
                                // a new window. Otherwise, cancel the gesture.
                                if dragging_tab.is_some() {
                                    if cross_window_drag::is_active() {
                                        pending_new_window_from_drag = true;
                                        {
                                            let mut r = renderer.lock();
                                            r.set_dirty(true);
                                            r.grid_dirty = true;
                                        }
                                        app_dirty = true;
                                    }
                                    dragging_tab = None;
                                    drag_threshold_passed = false;
                                    pending_pop_out = None;
                                    let _ = window_for_redraw.set_cursor_grab(CursorGrabMode::None);
                                }

                                ctrl_held = false;
                                shift_held = false;
                                alt_held = false;
                                modifiers = winit::keyboard::ModifiersState::default();

                                if old_hover_close || old_hover_max || old_hover_min || old_hover_settings || old_hover_update {
                                    let mut r = renderer.lock();
                                    r.hover_update = false;
                                    r.set_dirty(true);
                                    app_dirty = true;
                                }
                            }
                        }
                        WindowEvent::Occluded(occluded) => {
                            window_occluded = occluded;
                            tracing::info!("Window occlusion changed: occluded={}", occluded);
                        }
                        _ => {}
                    }
                } else if settings_sw.as_ref().map_or(false, |sw| window_id == sw.window.id()) {
                    if let Some(ref mut sw) = settings_sw {
                        let mut close_settings = false;
                        macro_rules! apply_settings {
                            () => {
                                let mut current_config = Config::load().unwrap_or_default();
                                current_config.font.family = settings_family.clone();
                                current_config.font.size = settings_size;
                                current_config.scrollback = settings_scrollback;
                                current_config.theme = Some(settings_theme.clone());
                                if let Err(e) = current_config.save(&Config::get_active_config_path()) {
                                    tracing::warn!("config: save failed: {e}");
                                }

                                tracing::info!("apply_settings: theme={:?} family={:?} size={} scrollback={}",
                                    settings_theme, settings_family, settings_size, settings_scrollback);

                                config = current_config;
                                config::set_active_theme(&settings_theme);
                                config.theme = Some(settings_theme.clone());

                                // Apply the new font family and size dynamically to the renderer!
                                if let Err(e) = renderer.lock().update_font(&config.font.family, config.font.size) {
                                    tracing::error!("Failed to update renderer font: {:?}", e);
                                }

                                // Apply font family update to the settings renderer as well, but at fixed font size 13.0
                                let _ = sw.renderer.update_font(&settings_family, 13.0);

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

                                // Theme change requires rebuilding cell instances so colors reflect live.
                                renderer.lock().grid_dirty = true;
                                app_dirty = true;
                            }
                        }

                        match event {
                            WindowEvent::CloseRequested => {
                                close_settings = true;
                                app_dirty = true;
                            }
                            WindowEvent::Resized(size) => {
                                sw.renderer.resize(size.width, size.height);
                            }
                            WindowEvent::RedrawRequested => {
                                 sw.renderer.render_settings(
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
                                    s_hover_theme,
                                    s_hover_open_config,
                                    &system_fonts,
                                    settings_font_scroll_y,
                                    settings_hovered_font_idx,
                                    &settings_theme,
                                    &themes_list,
                                    settings_hovered_theme_idx,
                                    settings_theme_scroll_y,
                                    config.opacity,
                                );
                                #[cfg(target_os = "windows")]
                                {
                                    sw.window.set_visible(true);
                                }
                            }
                            WindowEvent::CursorMoved { position, .. } => {
                                let scale_factor = sw.window.scale_factor();
                                s_mouse_x = position.x / scale_factor;
                                s_mouse_y = position.y / scale_factor;

                                let old_hover_close = s_hover_close;
                                let old_hover_family = s_hover_family;
                                let old_hover_size_minus = s_hover_size_minus;
                                let old_hover_size_plus = s_hover_size_plus;
                                let old_hover_scroll_minus = s_hover_scroll_minus;
                                let old_hover_scroll_plus = s_hover_scroll_plus;
                                let old_hover_theme = s_hover_theme;
                                let old_hover_open_config = s_hover_open_config;
                                let old_hovered_font_idx = settings_hovered_font_idx;
                                let old_hovered_theme_idx = settings_hovered_theme_idx;

                                let sw_width = sw.window.inner_size().width as f64 / scale_factor;
                                s_hover_close = if cfg!(target_os = "macos") {
                                    false // native close light owns this region
                                } else {
                                    s_mouse_y >= 4.0 && s_mouse_y <= 32.0 && s_mouse_x >= (sw_width - 32.0) && s_mouse_x < (sw_width - 4.0)
                                };
                                s_hover_family = s_mouse_y >= 52.0 && s_mouse_y <= 78.0 && s_mouse_x >= 140.0 && s_mouse_x < 380.0;

                                s_hover_size_minus = s_mouse_y >= 92.0 && s_mouse_y <= 118.0 && s_mouse_x >= 140.0 && s_mouse_x < 168.0;
                                s_hover_size_plus = s_mouse_y >= 92.0 && s_mouse_y <= 118.0 && s_mouse_x >= 220.0 && s_mouse_x < 248.0;

                                s_hover_scroll_minus = s_mouse_y >= 132.0 && s_mouse_y <= 158.0 && s_mouse_x >= 140.0 && s_mouse_x < 168.0;
                                s_hover_scroll_plus = s_mouse_y >= 132.0 && s_mouse_y <= 158.0 && s_mouse_x >= 240.0 && s_mouse_x < 268.0;

                                s_hover_theme = s_mouse_y >= 172.0 && s_mouse_y <= 198.0 && s_mouse_x >= 140.0 && s_mouse_x < 380.0;

                                s_hover_open_config = s_mouse_y >= 212.0 && s_mouse_y <= 238.0 && s_mouse_x >= 140.0 && s_mouse_x < 380.0;

                                if settings_active_field != 0 {
                                    s_hover_close = false;
                                    s_hover_family = false;
                                    s_hover_size_minus = false;
                                    s_hover_size_plus = false;
                                    s_hover_scroll_minus = false;
                                    s_hover_scroll_plus = false;
                                    s_hover_theme = false;
                                    s_hover_open_config = false;
                                }

                                let mut hovered_font_idx = None;
                                if settings_active_field == 1 && s_mouse_x >= 140.0 && s_mouse_x < 380.0 && s_mouse_y >= 78.0 && s_mouse_y < 258.0 {
                                    let idx = (((s_mouse_y - 78.0) + settings_font_scroll_y as f64) / 22.0) as usize;
                                    if idx < system_fonts.len() {
                                        hovered_font_idx = Some(idx);
                                    }
                                }
                                settings_hovered_font_idx = hovered_font_idx;

                                let mut hovered_theme_idx = None;
                                if settings_active_field == 2 && s_mouse_x >= 140.0 && s_mouse_x < 380.0 && s_mouse_y >= 198.0 && s_mouse_y < 198.0 + themes_list.len() as f64 * 22.0 {
                                    let idx = (((s_mouse_y - 198.0) + settings_theme_scroll_y as f64) / 22.0) as usize;
                                    if idx < themes_list.len() {
                                        hovered_theme_idx = Some(idx);
                                    }
                                }
                                settings_hovered_theme_idx = hovered_theme_idx;

                                let any_changed = s_hover_close != old_hover_close
                                    || s_hover_family != old_hover_family
                                    || s_hover_size_minus != old_hover_size_minus
                                    || s_hover_size_plus != old_hover_size_plus
                                    || s_hover_scroll_minus != old_hover_scroll_minus
                                    || s_hover_scroll_plus != old_hover_scroll_plus
                                    || s_hover_theme != old_hover_theme
                                    || s_hover_open_config != old_hover_open_config
                                    || settings_hovered_font_idx != old_hovered_font_idx
                                    || settings_hovered_theme_idx != old_hovered_theme_idx;

                                if any_changed {
                                    sw.renderer.set_dirty(true);
                                    sw.window.request_redraw();
                                }
                            }
                            WindowEvent::MouseInput { state, button, .. } => {
                                if button == MouseButton::Left && state == ElementState::Pressed {
                                    if settings_active_field == 1 {
                                        // Font dropdown is open. Bounding box: x: 140..380, y: 78..258
                                        if s_mouse_x >= 140.0 && s_mouse_x < 380.0 && s_mouse_y >= 78.0 && s_mouse_y < 258.0 {
                                            let idx = (((s_mouse_y - 78.0) + settings_font_scroll_y as f64) / 22.0) as usize;
                                            if idx < system_fonts.len() {
                                                settings_family = system_fonts[idx].clone();
                                                settings_active_field = 0;
                                                apply_settings!();
                                            }
                                        } else {
                                            // Clicked outside — close dropdown and consume event
                                            settings_active_field = 0;
                                        }
                                        sw.renderer.set_dirty(true);
                                        sw.window.request_redraw();
                                        return;
                                    } else if settings_active_field == 2 {
                                        // Theme dropdown is open. Bounding box: x: 140..380, y: 198..198 + themes_list.len() * 22.0
                                        let theme_dropdown_h = themes_list.len() as f64 * 22.0;
                                        if s_mouse_x >= 140.0 && s_mouse_x < 380.0 && s_mouse_y >= 198.0 && s_mouse_y < 198.0 + theme_dropdown_h {
                                            let idx = (((s_mouse_y - 198.0) + settings_theme_scroll_y as f64) / 22.0) as usize;
                                            if idx < themes_list.len() {
                                                settings_theme = themes_list[idx].clone();
                                                settings_active_field = 0;
                                                apply_settings!();
                                            }
                                        } else {
                                            // Clicked outside — close dropdown and consume event
                                            settings_active_field = 0;
                                        }
                                        sw.renderer.set_dirty(true);
                                        sw.window.request_redraw();
                                        return;
                                    }

                                    if s_hover_close {
                                        close_settings = true;
                                        app_dirty = true;
                                    } else if s_mouse_y <= 36.0 {
                                        let _ = sw.window.drag_window();
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
                                    } else if s_hover_theme {
                                        settings_active_field = 2;
                                        settings_theme_scroll_y = 0.0;
                                    } else if s_hover_size_minus {
                                        settings_size = (settings_size - 1.0).max(6.0);
                                        apply_settings!();
                                    } else if s_hover_size_plus {
                                        settings_size = (settings_size + 1.0).min(72.0);
                                        apply_settings!();
                                    } else if s_hover_scroll_minus {
                                        settings_scrollback = settings_scrollback.saturating_sub(1000).max(1000);
                                        apply_settings!();
                                    } else if s_hover_scroll_plus {
                                        settings_scrollback = settings_scrollback.saturating_add(1000).min(1000);
                                        apply_settings!();
                                    } else if s_hover_open_config {
                                        let mut current_config = Config::load().unwrap_or_default();
                                        current_config.font.family = settings_family.clone();
                                        current_config.font.size = settings_size;
                                        current_config.scrollback = settings_scrollback;
                                        current_config.theme = Some(settings_theme.clone());
                                        let path = Config::get_active_config_path();
                                        if let Err(e) = current_config.save(&path) {
                                            tracing::warn!("config: save failed before opening editor: {e}");
                                        }
                                        let _ = open_file_in_editor(&path);
                                    } else {
                                        settings_active_field = 0;
                                    }
                                    sw.renderer.set_dirty(true);
                                    sw.window.request_redraw();
                                }
                            }
                            WindowEvent::MouseWheel { delta, .. } => {
                                let lines = match delta {
                                    MouseScrollDelta::LineDelta(_, y) => y,
                                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 22.0,
                                };
                                let item_h = 22.0f32;
                                let mut handled = false;
                                if settings_active_field == 1 {
                                    let visible_h = 180.0f32;
                                    let total_h = system_fonts.len() as f32 * item_h;
                                    let max_scroll = (total_h - visible_h).max(0.0);
                                    settings_font_scroll_y = (settings_font_scroll_y - lines * item_h).clamp(0.0, max_scroll);
                                    handled = true;
                                } else if settings_active_field == 2 {
                                    let visible_h = ((260.0_f32 - 198.0 - 8.0) as f32).max(item_h);
                                    let total_h = themes_list.len() as f32 * item_h;
                                    let max_scroll = (total_h - visible_h).max(0.0);
                                    settings_theme_scroll_y = (settings_theme_scroll_y - lines * item_h).clamp(0.0, max_scroll);
                                    handled = true;
                                }
                                if handled {
                                    sw.renderer.set_dirty(true);
                                    sw.window.request_redraw();
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
                                    sw.renderer.set_dirty(true);
                                    sw.window.request_redraw();
                                }
                            }
                            WindowEvent::CursorLeft { .. } => {
                                s_hover_close = false;
                                s_hover_family = false;
                                s_hover_size_minus = false;
                                s_hover_size_plus = false;
                                s_hover_scroll_minus = false;
                                s_hover_scroll_plus = false;
                                s_hover_theme = false;
                                s_hover_open_config = false;
                                sw.renderer.set_dirty(true);
                                sw.window.request_redraw();
                            }
                            WindowEvent::Focused(focused) => {
                                if !focused {
                                    s_hover_close = false;
                                    s_hover_family = false;
                                    s_hover_size_minus = false;
                                    s_hover_size_plus = false;
                                    s_hover_scroll_minus = false;
                                    s_hover_scroll_plus = false;
                                    s_hover_theme = false;
                                    s_hover_open_config = false;
                                    sw.renderer.set_dirty(true);
                                    sw.window.request_redraw();
                                }
                            }
                            _ => {}
                        }
                        if close_settings {
                            settings_sw = None;
                        }
                    }
                } else if about.as_ref().map_or(false, |aw| window_id == aw.window.id()) {
                    if let Some(ref mut aw) = about {
                        match aw.handle_event(&event, &mut app_dirty) {
                            secondary_window::EventResult::Closed => {
                                about = None;
                            }
                            secondary_window::EventResult::Consumed => {
                                if let WindowEvent::RedrawRequested = event {
                                    aw.renderer.render_about(&get_current_version(), aw.hover_close, config.opacity);
                                }
                            }
                            secondary_window::EventResult::Ignored => {}
                        }
                    }
                }
            }
            winit::event::Event::AboutToWait => {
                let now = std::time::Instant::now();

                if let Some(idx) = pending_pop_out.take() {
                    if idx < tabs.len() && tabs.len() >= 2 {
                        let tab = tabs.remove(idx);
                        if active_tab_index >= tabs.len() && !tabs.is_empty() {
                            active_tab_index = tabs.len() - 1;
                        }
                        if !tabs.is_empty() {
                            let physical_size = window_for_redraw.inner_size();
                            let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_width, cell_height);
                            shell_cols = cols;
                            shell_rows = rows;
                        }
                        // Main window's tab strip just lost one tab; force a redraw
                        // so the user sees the tab disappear immediately.
                        {
                            let mut r = renderer.lock();
                            r.set_dirty(true);
                            r.grid_dirty = true;
                        }
                        window_for_redraw.request_redraw();

                        *cross_window_drag::DRAG.lock() = Some(cross_window_drag::CrossWindowDrag {
                            source_window_id: window_for_redraw.id(),
                            tab,
                        });
                        pending_new_window_from_drag = true;
                    }
                }

                if pending_new_window_from_drag {
                    pending_new_window_from_drag = false;
                    if let Some(drag) = cross_window_drag::take() {
                        let tab = drag.tab;
                        let source_id = drag.source_window_id;
                        // Compute screen position: use the source window's outer
                        // position + cursor offset. For the main window we use
                        // `current_mouse_x/y`; for a popped-out window we use
                        // its stored cursor position.
                        let (sx, sy) = if source_id == window_for_redraw.id() {
                            match window_for_redraw.outer_position() {
                                Ok(p) => (
                                    p.x + current_mouse_x.max(0.0) as i32,
                                    p.y + current_mouse_y.max(0.0) as i32,
                                ),
                                Err(_) => (0, 0),
                            }
                        } else if let Some(src_wc) = popped_out_windows.get(&source_id) {
                            match src_wc.window.outer_position() {
                                Ok(p) => (
                                    p.x + src_wc.drag_current_x.max(0.0) as i32,
                                    p.y + src_wc.drag_current_y.max(0.0) as i32,
                                ),
                                Err(_) => (0, 0),
                            }
                        } else {
                            (0, 0)
                        };
                        let attrs = with_platform_chrome(
                            winit::window::WindowAttributes::default()
                                .with_title(tab.custom_name.as_deref().unwrap_or("fastty"))
                                .with_transparent(true)
                                .with_visible(true)
                                .with_position(winit::dpi::PhysicalPosition::new(sx, sy))
                                .with_inner_size(winit::dpi::LogicalSize::new(800.0, 520.0)),
                        );
                        if let Ok(window) = target.create_window(attrs) {
                            let window_arc = Arc::new(window);
                            let w_ref: &winit::window::Window = &*window_arc;
                            let w_static: &'static winit::window::Window = unsafe { std::mem::transmute(w_ref) };
                            let (shared_instance, shared_device, shared_queue, format, alpha_mode, cloned_atlas, cloned_ui_atlas) = {
                                let r = renderer.lock();
                                (
                                    r.instance.clone(),
                                    r.device.clone(),
                                    r.queue.clone(),
                                    r.config.format,
                                    r.config.alpha_mode,
                                    r.atlas.try_clone().ok(),
                                    r.ui_atlas.try_clone().ok(),
                                )
                            };
                            let r_result = if let (Some(a), Some(ui_a)) = (cloned_atlas, cloned_ui_atlas) {
                                Renderer::new_shared_fast(w_static, shared_instance, shared_device, shared_queue, format, alpha_mode, a, ui_a)
                            } else {
                                Renderer::new_shared(w_static, &config.font.family, config.font.size, shared_instance, shared_device, shared_queue, format, alpha_mode)
                            };
                            if let Ok(r) = r_result {
                                 let actual_cell_w = r.cell_width();
                                 let actual_cell_h = r.cell_height();
                                 let w_size = window_arc.inner_size();
                                 let (w_width, w_height) = if w_size.width < 100 || w_size.height < 100 {
                                     let sf = window_arc.scale_factor() as f32;
                                     ((800.0 * sf) as u32, (520.0 * sf) as u32)
                                 } else {
                                     (w_size.width, w_size.height)
                                 };
                                 let (cols, rows) = resize_all_tabs(
                                     std::slice::from_ref(&tab),
                                     w_width,
                                     w_height,
                                     actual_cell_w, actual_cell_h,
                                 );
                                 let bar = build_bar_layout(&config);
                                 let wc = window_context::WindowContext::new(
                                     window_arc,
                                     Arc::new(parking_lot::Mutex::new(r)),
                                     vec![tab],
                                     actual_cell_w,
                                     actual_cell_h,
                                     cols,
                                     rows,
                                     bar,
                                 );
                                 let id = wc.window.id();
                                 popped_out_windows.insert(id, wc);
                                 if let Some(wc) = popped_out_windows.get(&id) {
                                     #[cfg(target_os = "windows")]
                                     wc.window.set_visible(true);
                                     wc.window.focus_window();
                                     let _ = wc.window.drag_window();
                                     // Mark dirty and request redraw immediately so
                                     // the first RedrawRequested renders full content.
                                     // Skip present_blank() — it forces a GPU roundtrip
                                     // for a blank frame the user never sees, adding
                                     // visible latency.
                                     {
                                         let mut r = wc.renderer.lock();
                                         r.set_dirty(true);
                                         r.grid_dirty = true;
                                     }
                                     wc.window.request_redraw();
                                 }
                             }
                        }
                    }
                }

                // Restore additional windows from the session file (Test 7).
                // These are popped-out windows saved on the previous run. We
                // create them in `AboutToWait` because we need an
                // `ActiveEventLoop` to call `create_window`.
                if !pending_session_windows.is_empty() {
                    let queue = std::mem::take(&mut pending_session_windows);
                    for ws in queue {
                        if ws.tabs.is_empty() {
                            continue;
                        }
                        let mut restored_tabs = Vec::with_capacity(ws.tabs.len());
                        for tab_info in &ws.tabs {
                            let tab_cwd = tab_info.cwd.as_ref().and_then(|p| p.to_str());
                            match create_new_tab(
                                &shell, &[], tab_cwd, config.scrollback, config.font.clone(),
                                cell_width, cell_height, shell_cols, shell_rows, proxy.clone(),
                            ) {
                                Ok(t) => restored_tabs.push(t),
                                Err(e) => tracing::warn!("session: failed to restore tab: {e:?}"),
                            }
                        }
                        if restored_tabs.is_empty() {
                            continue;
                        }
                        let mut attrs = with_platform_chrome(
                            winit::window::WindowAttributes::default()
                                .with_title("fastty")
                                .with_transparent(true)
                                .with_visible(true),
                        );
                        if let Some((x, y)) = ws.position {
                            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
                        }
                        if let Some((w, h)) = ws.size {
                            if w > 0 && h > 0 {
                                attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h));
                            } else {
                                attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(800.0, 520.0));
                            }
                        } else {
                            attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(800.0, 520.0));
                        }
                        if let Ok(window) = target.create_window(attrs) {
                            let window_arc = Arc::new(window);
                            let w_ref: &winit::window::Window = &*window_arc;
                            let w_static: &'static winit::window::Window = unsafe { std::mem::transmute(w_ref) };
                            let (shared_instance, shared_device, shared_queue, format, alpha_mode, cloned_atlas, cloned_ui_atlas) = {
                                let r = renderer.lock();
                                (
                                    r.instance.clone(),
                                    r.device.clone(),
                                    r.queue.clone(),
                                    r.config.format,
                                    r.config.alpha_mode,
                                    r.atlas.try_clone().ok(),
                                    r.ui_atlas.try_clone().ok(),
                                )
                            };
                            let r_result = if let (Some(a), Some(ui_a)) = (cloned_atlas, cloned_ui_atlas) {
                                Renderer::new_shared_fast(w_static, shared_instance, shared_device, shared_queue, format, alpha_mode, a, ui_a)
                            } else {
                                Renderer::new_shared(w_static, &config.font.family, config.font.size, shared_instance, shared_device, shared_queue, format, alpha_mode)
                            };
                            if let Ok(r) = r_result {
                                let actual_cell_w = r.cell_width();
                                let actual_cell_h = r.cell_height();
                                let w_size = window_arc.inner_size();
                                let (w_width, w_height) = if w_size.width < 100 || w_size.height < 100 {
                                    let sf = window_arc.scale_factor() as f32;
                                    ((800.0 * sf) as u32, (520.0 * sf) as u32)
                                } else {
                                    (w_size.width, w_size.height)
                                };
                                let (cols, rows) = resize_all_tabs(
                                    &restored_tabs,
                                    w_width,
                                    w_height,
                                    actual_cell_w, actual_cell_h,
                                );
                                let bar = build_bar_layout(&config);
                                let wc = window_context::WindowContext::new(
                                    window_arc,
                                    Arc::new(parking_lot::Mutex::new(r)),
                                    restored_tabs,
                                    actual_cell_w,
                                    actual_cell_h,
                                    cols,
                                    rows,
                                    bar,
                                );
                                let id = wc.window.id();
                                popped_out_windows.insert(id, wc);
                                if let Some(wc) = popped_out_windows.get(&id) {
                                    #[cfg(target_os = "windows")]
                                    wc.window.set_visible(true);
                                    wc.renderer.lock().present_blank();
                                }
                            }
                        }
                    }
                }

                if active_tab_index != last_active_tab_index {
                    if let Some(tab) = tabs.get_mut(active_tab_index) {
                        tab.git_status_check_at = now.checked_sub(std::time::Duration::from_secs(60)).unwrap_or(now);
                    }
                    last_active_tab_index = active_tab_index;
                }

                let mut active_repos = std::collections::HashSet::new();

                // Poll git status for each tab (throttled to once every 1.5s per tab).
                // Only the active tab is polled in real-time; other tabs are checked
                // at a slower cadence to keep the UI responsive.
                // However, if the current working directory of the tab changes, we bypass
                // the throttle to immediately reflect git widget visibility / state.
                for (idx, tab) in tabs.iter_mut().enumerate() {
                    let cwd = tab.terminal_state.lock().shell_pid()
                        .and_then(|pid| std::fs::read_link(format!("/proc/{}/cwd", pid)).ok())
                        .or_else(|| tab.cwd.clone());

                    if let Some(ref dir) = cwd {
                        if let Some(top) = git::git_toplevel(dir) {
                            git_watcher_manager.watch_repo(&top);
                            active_repos.insert(top);
                        }
                    }

                    let cwd_changed = cwd != tab.last_git_cwd;
                    let throttle_ms: u128 = if idx == active_tab_index { 1500 } else { 10000 };

                    if !cwd_changed && now.duration_since(tab.git_status_check_at).as_millis() < throttle_ms {
                        continue;
                    }

                    tab.git_status_check_at = now;
                    tab.last_git_cwd = cwd.clone();

                    if let Some(ref dir) = cwd {
                        let dir_clone = dir.clone();
                        let proxy_clone = proxy.clone();
                        let tab_idx = idx;
                        std::thread::spawn(move || {
                            let status = detect_git_status(&dir_clone);
                            let _ = proxy_clone.send_event(AppEvent::GitStatusUpdated {
                                window_id: None,
                                tab_idx,
                                status,
                            });
                        });
                    } else {
                        tab.git_status = None;
                    }
                }

                if !first_frame_rendered {
                    first_frame_rendered = true;
                    // Force render the first frame directly to commit the Wayland buffer and map the window!
                    let mut tab_titles = Vec::new();
                    let mut active_tab_path = "fastty".to_string();
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

                    let tab_running_states: Vec<bool> = tabs.iter().map(|t| t.is_running).collect();
                    let tab_exit_codes: Vec<Option<i32>> = tabs.iter().map(|t| t.last_exit_code).collect();

                    let active_tab = &tabs[active_tab_index];
                    let term = active_tab.terminal_state.lock();
                    let max_history = term.history_size() as f32;
                    let term_ref: &TerminalState = &*term;

                    let last_activity_time_secs = active_tab.last_activity_time.saturating_duration_since(start_time).as_secs_f32();
                    let current_time = start_time.elapsed().as_secs_f32();

                    let bell_flash_elapsed_ms = bell_flash_time.map(|t| t.elapsed().as_secs_f32() * 1000.0);
                    let (last_command_duration_ms, command_duration_display_secs, command_exit_code) =
                        match (last_command_duration, last_command_duration_display_time) {
                            (Some((ms, code)), Some(display_time)) => {
                                let elapsed = display_time.elapsed().as_secs_f32();
                                (Some(ms), Some(elapsed), code)
                            }
                            _ => (None, None, None),
                        };

                    let palette_filtered: Vec<String> = if command_palette_visible {
                        compute_palette_filtered(&palette_commands, &command_palette_query)
                    } else {
                        Vec::new()
                    };

                    let ssh_filtered: Vec<String> = if ssh_picker_visible {
                        compute_ssh_filtered(&ssh_hosts, &ssh_picker_query)
                    } else {
                        Vec::new()
                    };

                    let project_filtered: Vec<String> = if project_jumper_visible {
                        compute_project_filtered(&project_jumper_items, &project_jumper_query)
                    } else {
                        Vec::new()
                    };

                    let worktree_filtered: Vec<String> = if worktree_picker_visible {
                        compute_worktree_filtered(&worktree_items, &worktree_picker_query, worktree_toplevel.as_deref())
                    } else {
                        Vec::new()
                    };

                    let drop_target_idx = if drag_threshold_passed {
                        dragging_tab.map(|_| {
                            let r = renderer.lock();
                            let vw = r.config.width as f64;
                            drop(r);
                            let tab_start_x = chrome_layout::tab_start_x() as f64;
                            let path_center_x = vw / 2.0;
                            let tab_area_max_x = path_center_x - 40.0;
                            let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                            let tabs_len = tabs.len();
                            let tab_width = if tabs_len > 0 {
                                (tab_area_width / tabs_len as f64).clamp(80.0, 160.0)
                            } else {
                                160.0
                            };
                            compute_drop_target(current_mouse_x, tab_start_x, tab_width, tabs_len)
                        }).and_then(|t| if t < tabs.len() { Some(t) } else { None })
                    } else {
                        None
                    };

                    // Apply deferred terminal resize — exactly once per frame,
                    // collapsing all rapid resize events into one reflow.
                    if let Some((cols, rows)) = pending_term_resize.take() {
                        tabs[active_tab_index].terminal_state.lock().resize(cols, rows);
                    }

                    // Apply deferred surface resize — exactly once per frame,
                    // with the final size from all accumulated resize events.
                    if let Some((w, h)) = pending_surface_resize.take() {
                        renderer.lock().resize(w, h);
                    }

                    let mut r = renderer.lock();
                    r.set_dirty(true);
                    let inputs = renderer::RenderInputs {
                        ligatures: config.font.ligatures,
                        scrollbar_alpha,
                        scroll_current: active_tab.scroll_current,
                        history_size: max_history,
                        visible_rows: shell_rows as f32,
                        hover_close,
                        hover_max,
                        hover_min,
                        hover_settings,
                        last_activity_time_secs,
                        current_time,
                        selection: active_tab.selection,
                        hovered_url: active_tab.hovered_url,
                        hovered_hyperlink: active_tab.hovered_hyperlink.as_deref(),
                        search_matches: &active_tab.search_matches,
                        search_current_idx: active_tab.search_current_idx,
                        search_visible: active_tab.search_visible,
                        search_query_render: &active_tab.search_query,
                        terminal_font_size: config.font.size,
                        toast: toast.as_ref().map(|(msg, t, d)| (msg.as_str(), *t, *d)),
                        active_tab_index,
                        tab_titles: &tab_titles,
                        tab_running_states: &tab_running_states,
                        tab_exit_codes: &tab_exit_codes,
                        active_tab_path: &active_tab_path,
                        context_menu_visible,
                        context_menu_is_about,
                        context_menu_x: context_menu_x as f32,
                        context_menu_y: context_menu_y as f32,
                        context_menu_hovered_idx,
                        context_menu_open_time_secs,
                        context_menu_scroll_y: context_menu_scroll_y as f32,
                        context_menu_items: &context_menu_items,
                        hovered_tab_index,
                        hovered_close_tab_index,
                        hover_new_tab,
                        command_palette_visible,
                        command_palette_query: &command_palette_query,
                        command_palette_selected,
                        command_palette_filtered: &palette_filtered,
                        command_palette_scroll,
                        dragging_tab: if cross_window_drag::is_active() && hovered_window == Some(window_for_redraw.id()) { Some(tabs.len()) } else { dragging_tab },
                        drag_current_x: drag_current_x as f32,
                        drag_tab_offset: drag_tab_offset as f32,
                        drop_target_idx: if cross_window_drag::is_active() && hovered_window == Some(window_for_redraw.id()) {
                            let tab_start_x = chrome_layout::tab_start_x() as f64;
                            let path_center_x = (window_for_redraw.inner_size().width as f64) / 2.0;
                            let tab_area_max_x = path_center_x - 40.0;
                            let tab_area_width = tab_area_max_x - tab_start_x - 32.0;
                            let tabs_len = tabs.len() + 1;
                            let tab_width = (tab_area_width / tabs_len as f64).clamp(80.0, 160.0);
                            Some(compute_drop_target(current_mouse_x, tab_start_x, tab_width, tabs_len))
                        } else {
                            drop_target_idx
                        },
                        tab_ctx_visible,
                        tab_ctx_x: tab_ctx_x as f32,
                        tab_ctx_y: tab_ctx_y as f32,
                        tab_ctx_hovered,
                        renaming_tab,
                        rename_buffer: &rename_buffer,
                        rename_cursor,
                        git_status: active_tab.git_status.as_ref(),
                        bar_segments: &bar_layout.laid_out,
                        bar_y,
                        bar_h,
                        ssh_picker_visible,
                        ssh_picker_query: &ssh_picker_query,
                        ssh_picker_selected,
                        ssh_filtered: &ssh_filtered,
                        project_jumper_visible,
                        project_jumper_query: &project_jumper_query,
                        project_jumper_selected,
                        project_filtered: &project_filtered,
                        worktree_picker_visible,
                        worktree_picker_query: &worktree_picker_query,
                        worktree_picker_selected,
                        worktree_filtered: &worktree_filtered,
                        bell_flash_elapsed_ms,
                        last_command_duration_ms,
                        command_duration_display_secs,
                        exit_code: command_exit_code,
                        current_mouse_x: current_mouse_x as f32,
                        current_mouse_y: current_mouse_y as f32,
                        hovered_url_text: active_tab.hovered_url_text.as_deref(),
                        opacity: config.opacity,
                    };
                    r.render(next_render_reason, term_ref, active_tab.cursor_visible, inputs);
                    drop(r);
                    #[cfg(target_os = "windows")]
                    {
                        window_for_redraw.set_visible(true);
                    }
                    last_render_time = now;
                    app_dirty = false;
                }

                let mut animating = false;

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
                                app_dirty = true;
                                renderer.lock().grid_dirty = true;
                            }
                        }
                    }

                    let max_history = term.history_size() as f32;
                    tab.scroll_target = tab.scroll_target.clamp(0.0, max_history);

                    // Apply scroll momentum
                    if scroll_velocity.abs() > 0.01 {
                        tab.scroll_target = (tab.scroll_target + scroll_velocity).clamp(0.0, max_history);
                        scroll_velocity *= SCROLL_DECELERATION;
                        if scroll_velocity.abs() < SCROLL_SNAP_THRESHOLD {
                            scroll_velocity = 0.0;
                        }
                        animating = true;
                    }

                    let diff = tab.scroll_target - tab.scroll_current;
                    let mut current_scroll_diff = 0;
                    if diff.abs() > 0.01 {
                        // Smooth lerp toward target -- gentle, predictable
                        tab.scroll_current += diff * 0.18;
                        if diff.abs() <= 0.5 {
                            tab.scroll_current = tab.scroll_target;
                        }

                        let target_offset = tab.scroll_current.round() as isize;
                        let scroll_diff = target_offset - term.display_offset() as isize;
                        if scroll_diff != 0 {
                            term.scroll(scroll_diff);
                            current_scroll_diff = scroll_diff;
                        }
                        if idx == active_tab_index {
                            animating = true;
                            renderer.lock().grid_dirty = true;
                        }
                    } else {
                        if tab.scroll_current != tab.scroll_target {
                            tab.scroll_current = tab.scroll_target;
                            if idx == active_tab_index {
                                renderer.lock().grid_dirty = true;
                                app_dirty = true;
                            }
                        }
                    }

                    tab.last_actual_offset = term.display_offset();
                    tab.last_scroll_diff = current_scroll_diff;

                    // Sync render generation
                    let current_rg = term.render_generation();
                    if current_rg != tab.last_render_generation {
                        tab.last_render_generation = current_rg;
                        tab.last_activity_time = std::time::Instant::now();
                        tab.cursor_visible = true;
                        if idx == active_tab_index {
                            app_dirty = true;
                            renderer.lock().grid_dirty = true;
                        }
                    }
                }

                // Popped-out windows run their own PTY readers, so we
                // must poll their render generations here (the main tab
                // loop above only sees tabs in `tabs`). When a popped-out
                // tab's terminal state changes, mark its renderer dirty
                // and request a redraw so the new output is presented.
                let mut popped_out_redraw_wakeup: Option<std::time::Instant> = None;
                for wc in popped_out_windows.values_mut() {
                    let active_idx = wc.active_tab_index.min(wc.tabs.len().saturating_sub(1));

                    // Update scroll momentum and lerp offsets for all tabs in this popped-out window
                    let mut wc_animating = false;
                    for (idx, tab) in wc.tabs.iter_mut().enumerate() {
                        let term = tab.terminal_state.lock();

                        // Sync scroll position if PTY printed output or terminal was resized
                        let actual_offset = term.display_offset();
                        if actual_offset != tab.last_actual_offset {
                            let diff_offset = actual_offset as f32 - tab.last_actual_offset as f32;
                            let pty_change = diff_offset - tab.last_scroll_diff as f32;
                            if pty_change.abs() > 0.01 {
                                tab.scroll_target += pty_change;
                                tab.scroll_current += pty_change;
                                if idx == active_idx {
                                    wc_animating = true;
                                    wc.renderer.lock().grid_dirty = true;
                                }
                            }
                        }

                        let max_history = term.history_size() as f32;
                        tab.scroll_target = tab.scroll_target.clamp(0.0, max_history);

                        // Apply scroll momentum
                        if wc.scroll_velocity.abs() > 0.01 {
                            tab.scroll_target = (tab.scroll_target + wc.scroll_velocity).clamp(0.0, max_history);
                            wc.scroll_velocity *= SCROLL_DECELERATION;
                            if wc.scroll_velocity.abs() < SCROLL_SNAP_THRESHOLD {
                                wc.scroll_velocity = 0.0;
                            }
                            wc_animating = true;
                        }

                        let diff = tab.scroll_target - tab.scroll_current;
                        let mut current_scroll_diff = 0;
                        if diff.abs() > 0.01 {
                            // Smooth lerp toward target -- gentle, predictable
                            tab.scroll_current += diff * 0.18;
                            if diff.abs() <= 0.5 {
                                tab.scroll_current = tab.scroll_target;
                            }

                            let target_offset = tab.scroll_current.round() as isize;
                            let scroll_diff = target_offset - term.display_offset() as isize;
                            if scroll_diff != 0 {
                                term.scroll(scroll_diff);
                                current_scroll_diff = scroll_diff;
                            }
                            if idx == active_idx {
                                wc_animating = true;
                                wc.renderer.lock().grid_dirty = true;
                            }
                        } else {
                            if tab.scroll_current != tab.scroll_target {
                                tab.scroll_current = tab.scroll_target;
                                if idx == active_idx {
                                    wc.renderer.lock().grid_dirty = true;
                                    wc_animating = true;
                                }
                            }
                        }

                        tab.last_actual_offset = term.display_offset();
                        tab.last_scroll_diff = current_scroll_diff;
                    }
                    if wc_animating {
                        animating = true;
                        wc.window.request_redraw();
                    }

                    // Poll git status for popped-out tabs (throttled)
                    if active_idx != wc.last_active_tab_index {
                        if let Some(tab) = wc.tabs.get_mut(active_idx) {
                            tab.git_status_check_at = now.checked_sub(std::time::Duration::from_secs(60)).unwrap_or(now);
                        }
                        wc.last_active_tab_index = active_idx;
                    }

                    // Poll git status for popped-out tabs (throttled)
                    for (tidx, tab) in wc.tabs.iter_mut().enumerate() {
                        let cwd = tab.terminal_state.lock().shell_pid()
                            .and_then(|pid| std::fs::read_link(format!("/proc/{}/cwd", pid)).ok())
                            .or_else(|| tab.cwd.clone());

                        if let Some(ref dir) = cwd {
                            if let Some(top) = git::git_toplevel(dir) {
                                git_watcher_manager.watch_repo(&top);
                                active_repos.insert(top);
                            }
                        }

                        let cwd_changed = cwd != tab.last_git_cwd;
                        let throttle_ms: u128 = if tidx == active_idx { 1500 } else { 10000 };

                        if !cwd_changed && now.duration_since(tab.git_status_check_at).as_millis() < throttle_ms {
                            continue;
                        }

                        tab.git_status_check_at = now;
                        tab.last_git_cwd = cwd.clone();

                        let win_id = wc.window.id();
                        if let Some(ref dir) = cwd {
                            let dir_clone = dir.clone();
                            let proxy_clone = proxy.clone();
                            let tab_idx = tidx;
                            std::thread::spawn(move || {
                                let status = detect_git_status(&dir_clone);
                                let _ = proxy_clone.send_event(AppEvent::GitStatusUpdated {
                                    window_id: Some(win_id),
                                    tab_idx,
                                    status,
                                });
                            });
                        } else {
                            tab.git_status = None;
                        }
                    }

                    // Sync render generation for the active tab
                    if let Some(tab) = wc.tabs.get_mut(active_idx) {
                        let current_rg = tab.terminal_state.lock().render_generation();
                        if current_rg != tab.last_render_generation {
                            tab.last_render_generation = current_rg;
                            tab.last_activity_time = std::time::Instant::now();
                            tab.cursor_visible = true;
                            let mut r = wc.renderer.lock();
                            r.set_dirty(true);
                            r.grid_dirty = true;
                            wc.window.request_redraw();
                        }
                        // Cursor blink: flip visibility and request redraw
                        // every 500ms while the cursor is idle.
                        let now_b = std::time::Instant::now();
                        let activity_end = tab.last_activity_time + std::time::Duration::from_millis(500);
                        if now_b >= activity_end {
                            let idle_ms = now_b.duration_since(activity_end).as_millis();
                            let blink_index: u64 = (idle_ms / 500) as u64;
                            if blink_index > tab.last_blink_index {
                                tab.last_blink_index = blink_index;
                                tab.cursor_visible = !tab.cursor_visible;
                                let mut r = wc.renderer.lock();
                                r.set_dirty(true);
                                wc.window.request_redraw();
                            }
                            let next_blink = activity_end
                                + std::time::Duration::from_millis((blink_index + 1) * 500);
                            match popped_out_redraw_wakeup {
                                None => popped_out_redraw_wakeup = Some(next_blink),
                                Some(t) if next_blink < t => popped_out_redraw_wakeup = Some(next_blink),
                                _ => {}
                            }
                        }
                    }
                }

                git_watcher_manager.prune_unreferenced(&active_repos);

                // Opacity animation of the scrollbar (uses active tab details)
                let v_width = renderer.lock().config.width as f64;
                const TOPBAR_HEIGHT: f32 = 30.0;
                let scrollbar_top_margin = TOPBAR_HEIGHT;
                
                let show_scrollbar = {
                    let term_guard = tabs[active_tab_index].terminal_state.lock();
                    let mode = *term_guard.term().lock().mode();
                    drop(term_guard);
                    let tui_owns_mouse = mode.intersects(
                        alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                        | alacritty_terminal::term::TermMode::MOUSE_MOTION
                        | alacritty_terminal::term::TermMode::SGR_MOUSE
                        | alacritty_terminal::term::TermMode::MOUSE_DRAG
                    );
                    let alt_screen_active = mode.contains(alacritty_terminal::term::TermMode::ALT_SCREEN);
                    !tui_owns_mouse && !alt_screen_active
                };

                let is_scrolling_recently = if let Some(scroll_time) = last_scroll_event_time {
                    now.duration_since(scroll_time) < std::time::Duration::from_millis(1500)
                } else {
                    false
                };

                let is_hovering = show_scrollbar && ((current_mouse_y > scrollbar_top_margin as f64 && current_mouse_x > (v_width - 20.0)) || is_dragging_scrollbar);
                let target_alpha = if show_scrollbar && (is_hovering || is_scrolling_recently) { 1.0 } else { 0.0 };

                let alpha_diff = target_alpha - scrollbar_alpha;
                if alpha_diff.abs() > 0.01 {
                    // Faster fade-in, slower fade-out to avoid flicker
                    let fade_rate = if alpha_diff > 0.0 { 0.25 } else { 0.12 };
                    scrollbar_alpha += alpha_diff * fade_rate;
                    animating = true;
                } else {
                    scrollbar_alpha = target_alpha;
                }

                // Fade-in animation of the context menu
                if context_menu_visible {
                    if let Some(open_time) = context_menu_open_time {
                        if open_time.elapsed() < std::time::Duration::from_millis(80) {
                            animating = true;
                        }
                    }
                }

                // Toast auto-clear and redraw management
                if let Some((_, start_time, duration_ms)) = toast {
                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                    if elapsed_ms < duration_ms {
                        // Toast is only animating (fading in/out) during the first 120ms and last 300ms.
                        // During the static phase (in-between), we do not set animating = true to save CPU/GPU.
                        if elapsed_ms < 120 || elapsed_ms >= duration_ms.saturating_sub(300) {
                            animating = true;
                        }
                    } else {
                        toast = None;
                        app_dirty = true;
                    }
                }

                if animating {
                    app_dirty = true;
                }

                let mut next_wakeup = None;
                let frame_time = std::time::Duration::from_millis(16);

                if app_dirty {
                    let elapsed = last_render_time.elapsed();
                    if elapsed >= frame_time {
                        next_render_reason = RenderReason::GridChanged;
                        window_for_redraw.request_redraw();
                        app_dirty = false;
                        if animating {
                            next_wakeup = Some(now + frame_time);
                        }
                    } else {
                        next_wakeup = Some(last_render_time + frame_time);
                    }
                }

                if next_wakeup.is_none() {
                    // Check cursor blink wakeup
                    let activity_end_time = tabs[active_tab_index].last_activity_time + std::time::Duration::from_millis(500);
                    if now >= activity_end_time {
                        // Cursor is idle and blinking.
                        let idle_time = now.duration_since(activity_end_time);
                        let idle_ms = idle_time.as_millis();
                        
                        let current_blink_index = idle_ms / 500;
                        if current_blink_index != last_blink_index {
                            last_blink_index = current_blink_index;
                            tabs[active_tab_index].cursor_visible = !tabs[active_tab_index].cursor_visible;
                            next_render_reason = RenderReason::CursorBlink;
                            window_for_redraw.request_redraw();
                        }
                        
                        // Wake up at the next blink boundary
                        let next_multiple_ms = (current_blink_index + 1) * 500;
                        let next_blink_time = activity_end_time + std::time::Duration::from_millis(next_multiple_ms as u64);
                        next_wakeup = Some(next_blink_time);
                    } else {
                        // Cursor is active (static). Wake up when it transitions to blinking.
                        next_wakeup = Some(activity_end_time);
                    }
                    
                    // Check scrollbar recent scroll fade-out wakeup
                    if let Some(scroll_time) = last_scroll_event_time {
                        let fade_start_time = scroll_time + std::time::Duration::from_millis(1500);
                        if now < fade_start_time {
                            if next_wakeup.is_none() || fade_start_time < next_wakeup.unwrap() {
                                next_wakeup = Some(fade_start_time);
                            }
                        }
                    }
                    
                    // Check toast auto-clear and fade-out transition wakeup
                    if let Some((_, toast_start, duration_ms)) = toast {
                        let elapsed_ms = now.duration_since(toast_start).as_millis() as u64;
                        if elapsed_ms < duration_ms {
                            // Wake up when the fade-out starts so we resume drawing at 60 FPS
                            let fade_out_start_ms = duration_ms.saturating_sub(300);
                            if elapsed_ms < fade_out_start_ms {
                                let fade_out_time = toast_start + std::time::Duration::from_millis(fade_out_start_ms);
                                if next_wakeup.is_none() || fade_out_time < next_wakeup.unwrap() {
                                    next_wakeup = Some(fade_out_time);
                                }
                            }

                            // Also wake up when it completely expires to clear the toast
                            let toast_expire_time = toast_start + std::time::Duration::from_millis(duration_ms);
                            if next_wakeup.is_none() || toast_expire_time < next_wakeup.unwrap() {
                                next_wakeup = Some(toast_expire_time);
                            }
                        }
                    }

                    // Popped-out windows: schedule a wakeup at the next
                    // cursor-blink boundary so the loop ticks even when
                    // the main window is idle.
                    if let Some(t) = popped_out_redraw_wakeup {
                        if next_wakeup.is_none() || t < next_wakeup.unwrap() {
                            next_wakeup = Some(t);
                        }
                    }
                }

                if let Some(wakeup) = next_wakeup {
                    target.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(wakeup));
                } else {
                    target.set_control_flow(winit::event_loop::ControlFlow::Wait);
                }
            }
            _ => {}
        }
    })?;

    Ok(())
}

fn encode_cursor_key(
    key_char: u8,
    shift: bool,
    alt: bool,
    ctrl: bool,
    mode: alacritty_terminal::term::TermMode,
) -> Vec<u8> {
    let mut mod_val = 1;
    if shift { mod_val += 1; }
    if alt { mod_val += 2; }
    if ctrl { mod_val += 4; }

    if mod_val > 1 {
        vec![0x1B, 0x5B, b'1', b';', b'0' + mod_val, key_char]
    } else {
        let prefix = if mode.contains(alacritty_terminal::term::TermMode::APP_CURSOR) { 0x4F } else { 0x5B };
        vec![0x1B, prefix, key_char]
    }
}

fn encode_keypad_key(
    key_code: u8,
    shift: bool,
    alt: bool,
    ctrl: bool,
) -> Vec<u8> {
    let mut mod_val = 1;
    if shift { mod_val += 1; }
    if alt { mod_val += 2; }
    if ctrl { mod_val += 4; }

    if mod_val > 1 {
        vec![0x1B, 0x5B, key_code, b';', b'0' + mod_val, b'~']
    } else {
        vec![0x1B, 0x5B, key_code, b'~']
    }
}

fn key_to_bytes(
    key: &Key,
    shift_active: bool,
    ctrl_active: bool,
    alt_active: bool,
    mode: alacritty_terminal::term::TermMode,
) -> Vec<u8> {
    match key {
        Key::Character(s) if !s.is_empty() => {
            let bytes = if ctrl_active {
                if s.len() == 1 {
                    let c = s.chars().next().unwrap();
                    if (c as u32) < 32 {
                        vec![c as u8]
                    } else if c.is_ascii_alphabetic() {
                        let base = c.to_ascii_lowercase() as u8;
                        vec![base - b'a' + 1]
                    } else {
                        match c {
                            '@' => vec![0x00],
                            '[' => vec![0x1B],
                            '\\' => vec![0x1C],
                            ']' => vec![0x1D],
                            '^' => vec![0x1E],
                            '_' => vec![0x1F],
                            '?' => vec![0x7F],
                            _ => s.as_bytes().to_vec(),
                        }
                    }
                } else {
                    s.as_bytes().to_vec()
                }
            } else {
                s.as_bytes().to_vec()
            };

            if alt_active {
                let mut alt_bytes = vec![0x1B];
                alt_bytes.extend(bytes);
                alt_bytes
            } else {
                bytes
            }
        }
        Key::Named(n) => {
            use winit::keyboard::NamedKey;
            match n {
                NamedKey::Enter => {
                    if shift_active {
                        let has_kbd_proto = mode.intersects(
                            alacritty_terminal::term::TermMode::DISAMBIGUATE_ESC_CODES
                            | alacritty_terminal::term::TermMode::REPORT_EVENT_TYPES
                            | alacritty_terminal::term::TermMode::REPORT_ALTERNATE_KEYS
                            | alacritty_terminal::term::TermMode::REPORT_ALL_KEYS_AS_ESC
                            | alacritty_terminal::term::TermMode::REPORT_ASSOCIATED_TEXT
                        );
                        if has_kbd_proto {
                            vec![0x1B, 0x5B, 0x31, 0x33, 0x3B, 0x32, 0x75] // \x1b[13;2u
                        } else {
                            vec![0x1B, 0x0D] // \x1b\r (Esc + Enter / Alt+Enter)
                        }
                    } else if ctrl_active {
                        vec![0x0A] // Ctrl + Enter -> LF
                    } else {
                        vec![b'\r']
                    }
                }
                NamedKey::Space => {
                    if ctrl_active {
                        vec![0x00]
                    } else {
                        vec![b' ']
                    }
                }
                NamedKey::Backspace => vec![0x7F],
                NamedKey::Tab => {
                    if shift_active {
                        vec![0x1B, 0x5B, 0x5A] // \x1b[Z
                    } else {
                        vec![0x09] // \t
                    }
                }
                NamedKey::Escape => vec![0x1B],

                // Arrow keys
                NamedKey::ArrowUp => encode_cursor_key(b'A', shift_active, alt_active, ctrl_active, mode),
                NamedKey::ArrowDown => encode_cursor_key(b'B', shift_active, alt_active, ctrl_active, mode),
                NamedKey::ArrowRight => encode_cursor_key(b'C', shift_active, alt_active, ctrl_active, mode),
                NamedKey::ArrowLeft => encode_cursor_key(b'D', shift_active, alt_active, ctrl_active, mode),

                // Home/End
                NamedKey::Home => encode_cursor_key(b'H', shift_active, alt_active, ctrl_active, mode),
                NamedKey::End => encode_cursor_key(b'F', shift_active, alt_active, ctrl_active, mode),

                // Keypad navigation
                NamedKey::PageUp => encode_keypad_key(b'5', shift_active, alt_active, ctrl_active),
                NamedKey::PageDown => encode_keypad_key(b'6', shift_active, alt_active, ctrl_active),
                NamedKey::Insert => encode_keypad_key(b'2', shift_active, alt_active, ctrl_active),
                NamedKey::Delete => encode_keypad_key(b'3', shift_active, alt_active, ctrl_active),

                // Function keys
                NamedKey::F1 => vec![0x1B, 0x4F, 0x50],
                NamedKey::F2 => vec![0x1B, 0x4F, 0x51],
                NamedKey::F3 => vec![0x1B, 0x4F, 0x52],
                NamedKey::F4 => vec![0x1B, 0x4F, 0x53],
                NamedKey::F5 => vec![0x1B, 0x5B, 0x31, 0x35, 0x7E],
                NamedKey::F6 => vec![0x1B, 0x5B, 0x31, 0x37, 0x7E],
                NamedKey::F7 => vec![0x1B, 0x5B, 0x31, 0x38, 0x7E],
                NamedKey::F8 => vec![0x1B, 0x5B, 0x31, 0x39, 0x7E],
                NamedKey::F9 => vec![0x1B, 0x5B, 0x32, 0x30, 0x7E],
                NamedKey::F10 => vec![0x1B, 0x5B, 0x32, 0x31, 0x7E],
                NamedKey::F11 => vec![0x1B, 0x5B, 0x32, 0x33, 0x7E],
                NamedKey::F12 => vec![0x1B, 0x5B, 0x32, 0x34, 0x7E],

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

    if !text.is_empty() {
        let ctx_opt = if clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(ctx) => {
                    *clipboard = Some(ctx);
                    clipboard.as_mut()
                }
                Err(e) => {
                    eprintln!("fastty clipboard copy initialization failed: {:?}", e);
                    None
                }
            }
        } else {
            clipboard.as_mut()
        };
        if let Some(ctx) = ctx_opt {
            if let Err(e) = ctx.set_text(text) {
                eprintln!("fastty clipboard copy set_text failed: {:?}", e);
            }
        } else {
            eprintln!("fastty clipboard copy not available");
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

fn send_mouse_event_to_pty(
    terminal: &crate::terminal_state::TerminalState,
    button: winit::event::MouseButton,
    state: winit::event::ElementState,
    col: usize,
    row: usize,
    mode: alacritty_terminal::term::TermMode,
) {
    let is_press = state == winit::event::ElementState::Pressed;

    if mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE) {
        let btn = match button {
            winit::event::MouseButton::Left => 0,
            winit::event::MouseButton::Middle => 1,
            winit::event::MouseButton::Right => 2,
            _ => return,
        };
        let action = if is_press { 'M' } else { 'm' };
        let seq = format!("\x1b[<{};{};{}{}", btn, col + 1, row + 1, action);
        terminal.write_to_pty(seq.as_bytes());
    } else if mode.contains(alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK) {
        let btn_code = if is_press {
            match button {
                winit::event::MouseButton::Left => 0,
                winit::event::MouseButton::Middle => 1,
                winit::event::MouseButton::Right => 2,
                _ => return,
            }
        } else {
            3
        };
        let seq = [
            0x1b, b'[', b'M',
            btn_code as u8 + 32,
            (col + 1 + 32) as u8,
            (row + 1 + 32) as u8,
        ];
        terminal.write_to_pty(&seq);
    }
}

fn send_drag_event_to_pty(
    terminal: &crate::terminal_state::TerminalState,
    button: Option<winit::event::MouseButton>,
    col: usize,
    row: usize,
    mode: alacritty_terminal::term::TermMode,
) {
    let btn_code = match button {
        Some(winit::event::MouseButton::Left) => 32,
        Some(winit::event::MouseButton::Middle) => 33,
        Some(winit::event::MouseButton::Right) => 34,
        None => {
            if mode.contains(alacritty_terminal::term::TermMode::MOUSE_MOTION) {
                35
            } else {
                return;
            }
        }
        _ => return,
    };

    if mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE) {
        let seq = format!("\x1b[<{};{};{}M", btn_code, col + 1, row + 1);
        terminal.write_to_pty(seq.as_bytes());
    } else if mode.contains(alacritty_terminal::term::TermMode::MOUSE_DRAG)
        || mode.contains(alacritty_terminal::term::TermMode::MOUSE_MOTION)
    {
        let seq = [
            0x1b, b'[', b'M',
            btn_code as u8 + 32,
            (col + 1 + 32) as u8,
            (row + 1 + 32) as u8,
        ];
        terminal.write_to_pty(&seq);
    }
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

    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let _cmd = "start";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";

    #[cfg(target_os = "windows")]
    let res = no_window_cmd("cmd")
        .args(&["/C", "start", "", &target_url])
        .spawn();

    #[cfg(not(target_os = "windows"))]
    let res = std::process::Command::new(cmd)
        .arg(&target_url)
        .spawn();

    match res {
        Ok(_) => {}
        Err(e) => tracing::error!("Failed to open URL: {:?}", e),
    }
}

fn compute_search_matches(
    terminal_state: &Arc<parking_lot::Mutex<TerminalState>>,
    query: &str,
    shell_cols: usize,
    shell_rows: usize,
) -> Vec<renderer::SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let state = terminal_state.lock();
    let history_size = state.history_size();
    let term_guard = state.term().lock();
    let grid = term_guard.grid();

    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let q_chars: Vec<char> = query.chars().collect();
    let mut matches: Vec<renderer::SearchMatch> = Vec::new();
    let min_line: i32 = -(history_size as i32);
    let max_line: i32 = shell_rows as i32;

    for line_idx in min_line..max_line {
        let row = &grid[alacritty_terminal::index::Line(line_idx)];

        for start in 0..shell_cols {
            if start + q_chars.len() > shell_cols {
                break;
            }
            let mut ok = true;
            for i in 0..q_chars.len() {
                let cell = &row[alacritty_terminal::index::Column(start + i)];
                let cell_char = cell.c;
                if cell_char.to_lowercase().next().unwrap_or('\0') != query_lower[i] {
                    ok = false;
                    break;
                }
            }
            if ok {
                matches.push(renderer::SearchMatch { line: line_idx, col: start, len: q_chars.len() });
                if matches.len() >= 1000 {
                    break;
                }
            }
        }
        if matches.len() >= 1000 {
            break;
        }
    }
    matches
}

fn detect_hovered_hyperlink(
    current_mouse_x: f64,
    current_mouse_y: f64,
    terminal_state: &Arc<parking_lot::Mutex<TerminalState>>,
    scroll_current: f32,
    cell_width: f32,
    cell_height: f32,
    shell_cols: usize,
    shell_rows: usize,
    padding_top: f32,
) -> Option<String> {
    if current_mouse_y <= padding_top as f64 {
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

    if hover_point.line.0 < -(history_size as i32) || hover_point.line.0 >= shell_rows as i32 {
        return None;
    }

    let term_guard = term.term().lock();
    let grid = term_guard.grid();
    let row = &grid[alacritty_terminal::index::Line(hover_point.line.0)];
    let col_idx = hover_point.column.0;
    if col_idx >= shell_cols {
        return None;
    }
    let cell = &row[alacritty_terminal::index::Column(col_idx)];
    cell.hyperlink().map(|h| h.uri().to_string())
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
) -> (Option<renderer::HoveredUrl>, Option<String>) {
    if current_mouse_y <= padding_top as f64 || !ctrl_pressed {
        return (None, None);
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
                        return (
                            Some(renderer::HoveredUrl {
                                line: hover_point.line.0,
                                start_col: trimmed_start,
                                end_col: trimmed_end - 1,
                            }),
                            Some(trimmed_str),
                        );
                    }
                }
            }
        }
    }

    (None, None)
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

fn get_context_menu_items(tabs: &[Tab], active_idx: usize, is_about: bool) -> Vec<crate::renderer::ContextMenuItem> {
    if is_about {
        vec![crate::renderer::ContextMenuItem::About]
    } else {
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
}

fn build_smart_menu(
    classification: Option<&selection_classifier::Classification>,
    has_selection: bool,
    tabs_len: usize,
    cwd_resolvable: bool,
) -> Vec<renderer::ContextMenuItem> {
    use renderer::ContextMenuItem::*;
    use selection_classifier::Classification;
    let mut items = Vec::new();
    if has_selection {
        items.push(Copy);
    }
    match classification {
        Some(Classification::Url(_))   => items.push(OpenLink),
        Some(Classification::Email(_)) => items.push(OpenEmail),
        Some(Classification::Path(p))  => {
            items.push(CopyWord);
            if cwd_resolvable || p.starts_with('/') || p.starts_with('~') || p.starts_with('.') {
                items.push(CdHere);
            }
            items.push(OpenInEditor);
        }
        Some(Classification::Hex(_))   => items.push(CopyHex),
        Some(Classification::Word(_))  => items.push(CopyWord),
        None => {}
    }
    items.push(Paste);
    items.push(Separator);
    items.push(NewTab);
    if tabs_len > 1 {
        items.push(CloseTab);
    }
    items
}

fn get_context_menu_size(menu_items: &[crate::renderer::ContextMenuItem]) -> (f64, f64) {
    let is_git_actions_menu = menu_items.iter().any(|item| matches!(item, crate::renderer::ContextMenuItem::GithubActionInfo {..} | crate::renderer::ContextMenuItem::CommandItem {..}));
    if is_git_actions_menu {
        return (320.0, 320.0);
    }

    let mut h = 12.0f64; // 6px top + 6px bottom padding
    let mut w = 180.0f64;
    for item in menu_items {
        h += match item {
            crate::renderer::ContextMenuItem::Separator => 9.0,
            _ => 32.0,
        };
        match item {
            crate::renderer::ContextMenuItem::GithubActionInfo { label, .. } | crate::renderer::ContextMenuItem::CommandItem { label, .. } => {
                let estimated_w = 40.0 + label.chars().count() as f64 * 7.5;
                if estimated_w > w {
                    w = estimated_w;
                }
            }
            _ => {}
        }
    }
    (w.clamp(180.0, 450.0), h)
}

fn get_menu_item_at_y(
    menu_items: &[crate::renderer::ContextMenuItem],
    relative_y: f32,
    scroll_y: f32,
    menu_h: f32,
) -> Option<usize> {
    if relative_y < 6.0 || relative_y >= menu_h - 6.0 {
        return None;
    }
    let relative_y_content = relative_y + scroll_y;
    let mut current_y = 6.0f32;
    for (idx, item) in menu_items.iter().enumerate() {
        let item_h = match item {
            crate::renderer::ContextMenuItem::Separator => 9.0f32,
            _ => 32.0f32,
        };
        if relative_y_content >= current_y && relative_y_content < current_y + item_h {
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

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
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
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = no_window_cmd("reg")
            .args(&["query", "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if let Some(pos) = line.find("REG_SZ") {
                        let name_part = line[..pos].trim();
                        let clean_name = if let Some(p) = name_part.find(" (") {
                            &name_part[..p]
                        } else {
                            name_part
                        };
                        let lower = clean_name.to_lowercase();
                        if lower.contains("consolas") 
                            || lower.contains("courier") 
                            || lower.contains("lucida console") 
                            || lower.contains("cascadia")
                            || lower.contains("mono") 
                            || lower.contains("code") {
                            fonts.insert(clean_name.to_string());
                        }
                    }
                }
            }
        }
        
        fonts.insert("Consolas".to_string());
        fonts.insert("Courier New".to_string());
        fonts.insert("Lucida Console".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        fonts.insert("Menlo".to_string());
        fonts.insert("Monaco".to_string());
        fonts.insert("Courier".to_string());
        fonts.insert("Courier New".to_string());
    }

    if fonts.is_empty() {
        fonts.insert("monospace".to_string());
        fonts.insert("JetBrains Mono".to_string());
        fonts.insert("Fira Code".to_string());
    }
    fonts.into_iter().collect()
}

fn trigger_update(
    _update_available: &Arc<parking_lot::Mutex<Option<String>>>,
    update_in_progress: &Arc<parking_lot::Mutex<bool>>,
    update_completed: &Arc<parking_lot::Mutex<bool>>,
    window: &Arc<winit::window::Window>,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    _tabs: &Vec<Tab>,
    _active_tab_index: usize,
) {
    let completed = *update_completed.lock();
    if completed {
        // Spawn the newly updated fastty binary in the background!
        #[cfg(target_os = "windows")]
        {
            let home = std::env::var("USERPROFILE").unwrap_or_default();
            let fastty_path = std::path::Path::new(&home)
                .join(".local")
                .join("bin")
                .join("fastty.exe");
            let _ = no_window_cmd(&fastty_path.to_string_lossy()).spawn();
        }

        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .arg("-a")
                .arg("Fastty")
                .spawn();
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            // Linux
            let binary_path = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/fastty"));
            let home = std::env::var("HOME").unwrap_or_default();
            let local_path = std::path::Path::new(&home)
                .join(".local")
                .join("bin")
                .join("fastty");
            let spawn_path = if local_path.exists() {
                local_path
            } else {
                binary_path
            };
            let _ = std::process::Command::new(spawn_path).spawn();
        }

        // Close the current window/application
        let _ = proxy.send_event(AppEvent::ForceExit);
        return;
    }

    let mut in_progress = update_in_progress.lock();
    if *in_progress {
        return;
    }
    *in_progress = true;

    // Redraw window so the text switches to "Updating..." immediately
    window.request_redraw();

    #[cfg(target_os = "windows")]
    {
        let update_in_progress_clone = Arc::clone(update_in_progress);
        let update_completed_clone = Arc::clone(update_completed);
        let window_clone = Arc::clone(window);

        std::thread::spawn(move || {
            let mut success = false;
            if let Ok(mut child) = no_window_cmd("powershell")
                .arg("-Command")
                .arg("irm https://raw.githubusercontent.com/diegoleteliers10/fastty/main/instalar.ps1 | iex")
                .spawn() {
                if let Ok(status) = child.wait() {
                    success = status.success();
                }
            }
            let mut in_progress = update_in_progress_clone.lock();
            *in_progress = false;
            if success {
                let mut completed = update_completed_clone.lock();
                *completed = true;
            }
            window_clone.request_redraw();
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(tab) = _tabs.get(_active_tab_index) {
            let cmd = b"curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fastty/main/instalar.sh | bash -s -- --user\n";
            tab.terminal_state.lock().write_to_pty(cmd);
        }

        let update_in_progress_clone = Arc::clone(update_in_progress);
        let update_completed_clone = Arc::clone(update_completed);
        let window_clone = Arc::clone(window);
        std::thread::spawn(move || {
            let marker = std::path::Path::new("/tmp/fastty-update-done");
            for _ in 0..300u32 {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if marker.exists() {
                    let _ = std::fs::remove_file(marker);
                    *update_in_progress_clone.lock() = false;
                    *update_completed_clone.lock() = true;
                    window_clone.request_redraw();
                    return;
                }
            }
            *update_in_progress_clone.lock() = false;
            window_clone.request_redraw();
        });
    }
}

fn open_file_in_editor(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        no_window_cmd("cmd")
            .args(&["/C", "start", "", path.to_str().unwrap_or_default()])
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fastty_args_parsing() {
        // Test no args
        let args = FasttyArgs::parse_from(vec![]);
        assert!(args.command.is_none());
        assert!(args.working_dir.is_none());
        assert!(args.title.is_none());

        // Test title and directory
        let args = FasttyArgs::parse_from(vec![
            "--title".to_string(),
            "My Terminal".to_string(),
            "-d".to_string(),
            "/home/user/project".to_string(),
        ]);
        assert!(args.command.is_none());
        assert_eq!(args.working_dir.as_deref(), Some("/home/user/project"));
        assert_eq!(args.title.as_deref(), Some("My Terminal"));

        // Test command with multiple args
        let args = FasttyArgs::parse_from(vec![
            "-e".to_string(),
            "nvim".to_string(),
            "src/main.rs".to_string(),
        ]);
        assert_eq!(
            args.command,
            Some(vec!["nvim".to_string(), "src/main.rs".to_string()])
        );
        assert!(args.working_dir.is_none());
        assert!(args.title.is_none());

        // Test command mixed with other options (command consumes the rest)
        let args = FasttyArgs::parse_from(vec![
            "--title".to_string(),
            "My Dev Window".to_string(),
            "-d".to_string(),
            "/path/to/dir".to_string(),
            "-e".to_string(),
            "bun".to_string(),
            "run".to_string(),
            "dev".to_string(),
        ]);
        assert_eq!(args.title.as_deref(), Some("My Dev Window"));
        assert_eq!(args.working_dir.as_deref(), Some("/path/to/dir"));
        assert_eq!(
            args.command,
            Some(vec!["bun".to_string(), "run".to_string(), "dev".to_string()])
        );
    }
}