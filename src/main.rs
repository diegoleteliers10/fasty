#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod config;
mod event_listener;
mod pty;
mod renderer;
mod terminal_state;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use config::Config;
use renderer::{Renderer, Selection, RenderReason};
use terminal_state::{TerminalState, AppEvent};
use alacritty_terminal::grid::Dimensions;
use winit::{
    event::{ElementState, WindowEvent, MouseButton, MouseScrollDelta},
    event_loop::EventLoop,
    keyboard::Key,
};

fn get_login_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(target_os = "windows"))]
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
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
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
    cursor_visible: bool,
}

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
        cursor_visible: true,
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

fn get_current_version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}


#[derive(Debug)]
struct FastyArgs {
    command: Option<Vec<String>>,   // -e cmd arg1 arg2...
    working_dir: Option<String>,    // -d /path/to/dir
    title: Option<String>,          // --title "My Window"
}

impl FastyArgs {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self::parse_from(args)
    }

    fn parse_from(args: Vec<String>) -> Self {
        let mut result = Self {
            command: None,
            working_dir: None,
            title: None,
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
                    }
                }
                "--title" => {
                    if i + 1 < args.len() {
                        result.title = Some(args[i+1].clone());
                        i += 2;
                    }
                }
                _ => { i += 1; }
            }
        }
        result
    }
}

fn main() -> anyhow::Result<()> {
    std::env::set_var("TERM", "xterm-256color");
    std::env::set_var("COLORTERM", "truecolor");
    std::env::set_var("TERM_PROGRAM", "fasty");
    let app_version = get_current_version();
    std::env::set_var("TERM_PROGRAM_VERSION", app_version.trim_start_matches('v'));

    #[cfg(debug_assertions)]
    {
        tracing_subscriber::fmt()
            .with_env_filter("warn,fasty=info")
            .init();
    }

    let mut config = Config::load()?;

    let fasty_args = FastyArgs::parse();

    // Resolve what to spawn
    let (executable, exec_args) = match &fasty_args.command {
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
    let cwd = fasty_args.working_dir.clone();

    // Window title
    let window_title = fasty_args.title
        .clone()
        .unwrap_or_else(|| {
            match &fasty_args.command {
                Some(cmd) => cmd[0].clone(),  // "arch-update", "htop", etc
                None => "fasty".to_string(),
            }
        });

    let auto_close = fasty_args.command.is_some();

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

    #[cfg(not(target_os = "windows"))]
    let window = event_loop.create_window(winit::window::WindowAttributes::default()
        .with_title(&window_title)
        .with_decorations(false)
        .with_transparent(true)
        .with_inner_size(winit::dpi::LogicalSize::new(800.0, 520.0)))?;

    // Load and set the window icon at runtime for the taskbar/desktop bar
    if let Ok(icon_image) = image::load_from_memory(include_bytes!("../assets/fastyIcon.png")) {
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
    const PADDING_BOTTOM: f32 = 10.0;

    let viewport_height = renderer.config.height as f32;
    let mut shell_cols = ((viewport_width - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0) as usize;
    let mut shell_rows = ((viewport_height - (get_padding_top(1) + PADDING_BOTTOM)) / cell_height).floor().max(1.0) as usize;
    let proxy = event_loop.create_proxy();
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
    let mut toast: Option<(String, std::time::Instant, u64)> = None;
    let start_time = std::time::Instant::now();
    let mut clipboard: Option<arboard::Clipboard> = None;
    let mut context_menu_visible = false;
    let mut context_menu_is_about = false;
    let mut context_menu_x = 0.0f64;
    let mut context_menu_y = 0.0f64;
    let mut context_menu_hovered_idx: Option<usize> = None;
    let mut context_menu_open_time: Option<std::time::Instant> = None;
    let mut context_menu_open_time_secs: Option<f32> = None;
    let mut last_scroll_event_time: Option<std::time::Instant> = None;
    let mut mouse_down_button: Option<winit::event::MouseButton> = None;

    // Hover states for main window topbar buttons
    let mut hover_close = false;
    let mut hover_max = false;
    let mut hover_min = false;
    let mut hover_settings = false;
    let mut hover_update = false;
    let mut hovered_tab_index: Option<usize> = None;
    let mut hovered_close_tab_index: Option<usize> = None;
    let mut hover_new_tab = false;

    // Secondary settings window state
    let mut settings_window: Option<Arc<winit::window::Window>> = None;
    let mut settings_renderer: Option<Renderer<'static>> = None;
    let mut settings_family = String::new();
    let mut settings_size = 14.0f32;
    let mut settings_scrollback = 3000usize;
    let mut settings_active_field = 0usize; // 0 = none, 1 = font family select dropdown
    
    let mut s_hover_close = false;
    let mut s_hover_family = false;
    let mut s_hover_size_minus = false;
    let mut s_hover_size_plus = false;
    let mut s_hover_scroll_minus = false;
    let mut s_hover_scroll_plus = false;
    let mut s_hover_open_config = false;
    let mut s_hover_save = false;
    let mut s_hover_cancel = false;
    
    let mut settings_font_scroll_y = 0.0f32;
    let mut settings_hovered_font_idx: Option<usize> = None;
    let mut system_fonts = Vec::<String>::new();
    let mut s_mouse_x = 0.0f64;
    let mut s_mouse_y = 0.0f64;

    // Secondary about window state
    let mut about_window: Option<Arc<winit::window::Window>> = None;
    let mut about_renderer: Option<Renderer<'static>> = None;
    let mut about_hover_close = false;
    let mut about_mouse_y = 0.0f64;

    let mut first_frame_rendered = false;
    let mut app_dirty = true;
    let mut last_render_time = std::time::Instant::now();
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
            
            let cmd = std::process::Command::new("curl")
                .arg("-s")
                .arg("-H")
                .arg("User-Agent: fasty")
                .arg("https://api.github.com/repos/diegoleteliers10/fasty/releases/latest")
                .output();

            if let Ok(output) = cmd {
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

    event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    event_loop.run(move |event, target| {
        match event {
            winit::event::Event::UserEvent(app_event) => {
                match app_event {
                    AppEvent::Wakeup => {
                        app_dirty = true;
                        renderer.lock().grid_dirty = true;
                    }
                    AppEvent::Exit => {
                        if auto_close {
                            target.exit();
                        }
                    }
                    AppEvent::ForceExit => {
                        target.exit();
                    }
                }
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
                            app_dirty = true;
                        }
                        WindowEvent::RedrawRequested => {
                            #[cfg(target_os = "windows")]
                            let was_rendered = first_frame_rendered;
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

                            let mut r = renderer.lock();
                            r.update_available = is_available;
                            r.update_in_progress = is_in_progress;
                            r.update_completed = completed;
                            r.hover_update = hover_update;
                            r.set_dirty(true);
                            r.render(
                                next_render_reason,
                                term_ref,
                                active_tab.cursor_visible,
                                config.font.ligatures,
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
                                toast.as_ref().map(|(msg, t, d)| (msg.as_str(), *t, *d)),
                                active_tab_index,
                                &tab_titles,
                                &active_tab_path,
                                context_menu_visible,
                                context_menu_is_about,
                                context_menu_x as f32,
                                context_menu_y as f32,
                                context_menu_hovered_idx,
                                context_menu_open_time_secs,
                                hovered_tab_index,
                                hovered_close_tab_index,
                                hover_new_tab,
                            );
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
                                let mut r = renderer.lock();
                                r.set_dirty(true);
                                r.grid_dirty = true;
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
                                let mut r = renderer.lock();
                                r.set_dirty(true);
                                r.grid_dirty = true;
                                app_dirty = true;
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
                            let is_w_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyW) => true,
                                _ => key_str.eq_ignore_ascii_case("w")
                            };
                            let is_n_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyN) => true,
                                _ => key_str.eq_ignore_ascii_case("n")
                            };
                            let is_c_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyC) => true,
                                _ => key_str.eq_ignore_ascii_case("c")
                            };
                            let is_v_key = match event.physical_key {
                                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyV) => true,
                                _ => key_str.eq_ignore_ascii_case("v") || key_str == "\u{16}"
                            };

                            // Ctrl+Shift+T -> new tab
                            if ctrl_active && shift_active && is_t_key {
                                let new_tab_count = tabs.len() + 1;
                                let padding_top = get_padding_top(new_tab_count);
                                let physical_size = window_for_redraw.inner_size();
                                let new_cols = (((physical_size.width as f32 - PADDING_LEFT * 2.0) / cell_width).floor().max(1.0)) as usize;
                                let new_rows = (((physical_size.height as f32 - (padding_top + PADDING_BOTTOM)) / cell_height).floor().max(1.0)) as usize;
                                
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
                                        let mut r = renderer.lock();
                                        r.set_dirty(true);
                                        r.grid_dirty = true;
                                        app_dirty = true;
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to create new tab: {:?}", e);
                                    }
                                }
                                return;
                            }

                            // Ctrl+Shift+W -> close tab
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
                                    
                                    let mut r = renderer.lock();
                                    r.set_dirty(true);
                                    r.grid_dirty = true;
                                    app_dirty = true;
                                }
                                return;
                            }

                            // Ctrl+Shift+N -> new window
                            if ctrl_active && shift_active && is_n_key {
                                if let Ok(exe) = std::env::current_exe() {
                                    let _ = std::process::Command::new(exe).spawn();
                                }
                                return;
                            }

                            // Ctrl+Shift+C -> copy selection to clipboard
                            if ctrl_active && shift_active && is_c_key {
                                if let Some(sel) = tabs[active_tab_index].selection {
                                    copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
                                    toast = Some((
                                        "✓  Text copied".to_string(),
                                        std::time::Instant::now(),
                                        1920,
                                    ));
                                    let mut r = renderer.lock();
                                    r.set_dirty(true);
                                    r.grid_dirty = true;
                                    app_dirty = true;
                                }
                                return;
                            }

                            // Ctrl+Shift+V -> paste from clipboard
                            if ctrl_active && shift_active && is_v_key {
                                let mut ctx_opt = if clipboard.is_none() {
                                    match arboard::Clipboard::new() {
                                        Ok(ctx) => {
                                            clipboard = Some(ctx);
                                            clipboard.as_mut()
                                        }
                                        Err(_e) => {
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
                                            eprintln!("fasty clipboard get_text failed: {:?}", e);
                                        }
                                    }
                                } else {
                                    eprintln!("fasty clipboard not available");
                                }
                                return;
                            }

                            // Ctrl+Shift+Equal / Ctrl+Plus -> increase font size
                            // Ctrl+Minus -> decrease font size
                            // Ctrl+Shift+0 -> reset font size
                            let is_increase = (ctrl_active && shift_active && (key_str == "=" || key_str == "+"))
                                || (ctrl_active && !shift_active && key_str == "+");
                            let is_decrease = ctrl_active && !shift_active && key_str == "-";
                            let is_reset = ctrl_active && shift_active && key_str == "0";

                            if is_increase || is_decrease || is_reset {
                                let mut current_config = Config::load().unwrap_or_default();
                                let mut new_size = current_config.font.size;
                                if is_increase {
                                    new_size = (new_size + 0.5).min(72.0);
                                } else if is_decrease {
                                    new_size = (new_size - 0.5).max(6.0);
                                } else if is_reset {
                                    new_size = 13.0;
                                }

                                if new_size != current_config.font.size {
                                    current_config.font.size = new_size;
                                    let _ = current_config.save(&Config::config_path());
                                    config = current_config;

                                    if let Err(e) = renderer.lock().update_font(&config.font.family, config.font.size) {
                                        tracing::error!("Failed to update renderer font: {:?}", e);
                                    }

                                    let cell_w = renderer.lock().cell_width();
                                    let cell_h = renderer.lock().cell_height();
                                    let physical_size = window_for_redraw.inner_size();
                                    let (cols, rows) = resize_all_tabs(&tabs, physical_size.width, physical_size.height, cell_w, cell_h);
                                    shell_cols = cols;
                                    shell_rows = rows;
                                    cell_width = cell_w;
                                    cell_height = cell_h;

                                    let mut r = renderer.lock();
                                    r.set_dirty(true);
                                    r.grid_dirty = true;
                                    app_dirty = true;
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
                                    let mut r = renderer.lock();
                                    r.set_dirty(true);
                                    r.grid_dirty = true;
                                    app_dirty = true;
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
                                    let mut r = renderer.lock();
                                    r.set_dirty(true);
                                    r.grid_dirty = true;
                                    app_dirty = true;
                                }
                                return;
                            }

                            // Alt+1 to Alt+9 to switch tabs
                            if alt_active && !ctrl_active && !shift_active {
                                if let Some(digit_char) = key_str.chars().next() {
                                    if digit_char.is_ascii_digit() && digit_char != '0' {
                                        let target_idx = digit_char as usize - '1' as usize;
                                        if target_idx < tabs.len() {
                                            active_tab_index = target_idx;
                                            let mut r = renderer.lock();
                                            r.set_dirty(true);
                                            r.grid_dirty = true;
                                            app_dirty = true;
                                            return;
                                        }
                                    }
                                }
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

                            if context_menu_visible {
                                let pressed = state == ElementState::Pressed;
                                if pressed {
                                    if button == MouseButton::Left {
                                        if let Some(hovered_idx) = context_menu_hovered_idx {
                                            let menu_items = get_context_menu_items(&tabs, active_tab_index, context_menu_is_about);
                                            if hovered_idx < menu_items.len() {
                                                let item = menu_items[hovered_idx];
                                                match item {
                                                    crate::renderer::ContextMenuItem::About => {
                                                          if about_window.is_none() {
                                                              match target.create_window(winit::window::WindowAttributes::default()
                                                                  .with_title("About Fasty")
                                                                  .with_decorations(false)
                                                                  .with_transparent(true)
                                                                  .with_visible(true)
                                                                  .with_inner_size(winit::dpi::LogicalSize::new(300.0, 200.0)))
                                                              {
                                                                  Ok(window) => {
                                                                      let about_window_arc = Arc::new(window);
                                                                      let aw_ref: &winit::window::Window = &*about_window_arc;
                                                                      let aw_static: &'static winit::window::Window = unsafe { std::mem::transmute(aw_ref) };
                                                                      let (shared_instance, shared_device, shared_queue, format, alpha_mode) = {
                                                                          let r = renderer.lock();
                                                                          (r.instance.clone(), r.device.clone(), r.queue.clone(), r.config.format, r.config.alpha_mode)
                                                                      };
                                                                      match Renderer::new_shared(aw_static, "Inter", 13.0, shared_instance, shared_device, shared_queue, format, alpha_mode) {
                                                                          Ok(renderer_obj) => {
                                                                              about_window = Some(about_window_arc);
                                                                              about_renderer = Some(renderer_obj);
                                                                          }
                                                                          Err(e) => {
                                                                              tracing::error!("Failed to create about renderer: {:?}", e);
                                                                          }
                                                                      }
                                                                  }
                                                                  Err(e) => {
                                                                      tracing::error!("Failed to create about window: {:?}", e);
                                                                  }
                                                              }
                                                          } else {
                                                              about_window = None;
                                                              about_renderer = None;
                                                          }
                                                         context_menu_visible = false;
                                                         context_menu_open_time = None;
                                                         context_menu_open_time_secs = None;
                                                         context_menu_hovered_idx = None;
                                                         let mut r = renderer.lock();
                                                         r.set_dirty(true);
                                                         r.grid_dirty = true;
                                                         app_dirty = true;
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
                                                                    eprintln!("fasty clipboard initialization failed: {:?}", _e);
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
                                                    crate::renderer::ContextMenuItem::Separator => {}
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
                                    let mut r = renderer.lock();
                                    r.set_dirty(true);
                                    r.grid_dirty = true;
                                    app_dirty = true;
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
                                         let is_hovering_close = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 36.0) && current_mouse_x < (v_width - 8.0);
                                         let is_hovering_max = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 68.0) && current_mouse_x < (v_width - 40.0);
                                         let is_hovering_min = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 100.0) && current_mouse_x < (v_width - 72.0);
                                         let is_hovering_settings = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 137.0) && current_mouse_x < (v_width - 109.0);

                                         let is_update_available = update_available.lock().is_some();
                                         if is_update_available {
                                             let is_hovering_update = current_mouse_y >= 10.0 && current_mouse_y <= 30.0
                                                 && current_mouse_x >= (v_width - 219.0) && current_mouse_x < (v_width - 149.0);
                                             if is_hovering_update {
                                                 trigger_update(
                                                     &update_available,
                                                     &update_in_progress,
                                                     &update_completed,
                                                     &window_for_redraw,
                                                     proxy.clone(),
                                                 );
                                                 return;
                                             }
                                         }

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
                                                  settings_scrollback = config.scrollback.min(3000);
                                                  settings_active_field = 0;
                                                  match target.create_window(winit::window::WindowAttributes::default()
                                                      .with_title("fasty Settings")
                                                      .with_decorations(false)
                                                      .with_transparent(true)
                                                      .with_visible(true)
                                                      .with_inner_size(winit::dpi::LogicalSize::new(400.0, 300.0)))
                                                  {
                                                      Ok(window) => {
                                                          let settings_window_arc = Arc::new(window);
                                                          let sw_ref: &winit::window::Window = &*settings_window_arc;
                                                          let sw_static: &'static winit::window::Window = unsafe { std::mem::transmute(sw_ref) };
                                                          let (shared_instance, shared_device, shared_queue, format, alpha_mode) = {
                                                              let r = renderer.lock();
                                                              (r.instance.clone(), r.device.clone(), r.queue.clone(), r.config.format, r.config.alpha_mode)
                                                          };
                                                          match Renderer::new_shared(sw_static, &config.font.family, 13.0, shared_instance, shared_device, shared_queue, format, alpha_mode) {
                                                              Ok(renderer_obj) => {
                                                                  settings_window = Some(settings_window_arc);
                                                                  settings_renderer = Some(renderer_obj);
                                                              }
                                                              Err(e) => {
                                                                  tracing::error!("Failed to create settings renderer: {:?}", e);
                                                              }
                                                          }
                                                      }
                                                      Err(e) => {
                                                          tracing::error!("Failed to create settings window: {:?}", e);
                                                      }
                                                  }
                                              } else {
                                                  settings_window = None;
                                                  settings_renderer = None;
                                              }
                                              app_dirty = true;
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
                                                 let mut r = renderer.lock();
                                                 r.set_dirty(true);
                                                 r.grid_dirty = true;
                                                 app_dirty = true;
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
                                             let mut r = renderer.lock();
                                             r.set_dirty(true);
                                             r.grid_dirty = true;
                                             app_dirty = true;
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

                                    const TOPBAR_HEIGHT: f32 = 40.0;
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
                                                app_dirty = true;
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
                                                1920,
                                            ));
                                            renderer.lock().set_dirty(true);
                                            app_dirty = true;
                                        }
                                    } else if tabs[active_tab_index].selection_start_pos.is_some() {
                                        // Simple click release (no drag occurred): clear selection
                                        tabs[active_tab_index].selection = None;
                                        let mut r = renderer.lock();
                                        r.set_dirty(true);
                                        r.grid_dirty = true;
                                        app_dirty = true;

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
                                        // Right-click on Fasty icon in topbar
                                        context_menu_x = 8.0;
                                        context_menu_y = 40.0;
                                        context_menu_is_about = true;
                                        context_menu_open_time = Some(std::time::Instant::now());
                                        context_menu_open_time_secs = Some(start_time.elapsed().as_secs_f32());
                                        context_menu_visible = true;
                                        context_menu_hovered_idx = None;
                                        renderer.lock().set_dirty(true);
                                        app_dirty = true;
                                    } else if current_mouse_y >= padding_top as f64 && current_mouse_x <= (v_width - 20.0) {
                                        context_menu_is_about = false;
                                        let menu_items = get_context_menu_items(&tabs, active_tab_index, false);
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

                            if context_menu_visible {
                                let menu_items = get_context_menu_items(&tabs, active_tab_index, context_menu_is_about);
                                let (menu_w, menu_h) = get_context_menu_size(&menu_items);
                                if current_mouse_x >= context_menu_x && current_mouse_x <= context_menu_x + menu_w
                                   && current_mouse_y >= context_menu_y && current_mouse_y <= context_menu_y + menu_h {
                                    let relative_y = (current_mouse_y - context_menu_y) as f32;
                                    context_menu_hovered_idx = get_menu_item_at_y(&menu_items, relative_y);
                                } else {
                                    context_menu_hovered_idx = None;
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
                            let old_hover_update = hover_update;
                            let old_hovered_tab = hovered_tab_index;
                            let old_hovered_close = hovered_close_tab_index;
                            let old_hover_new = hover_new_tab;

                            hover_close = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 36.0) && current_mouse_x < (v_width - 8.0);
                            hover_max = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 68.0) && current_mouse_x < (v_width - 40.0);
                            hover_min = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 100.0) && current_mouse_x < (v_width - 72.0);
                            hover_settings = current_mouse_y >= 6.0 && current_mouse_y <= 34.0 && current_mouse_x >= (v_width - 137.0) && current_mouse_x < (v_width - 109.0);

                            let is_update_available = update_available.lock().is_some();
                            if is_update_available {
                                hover_update = current_mouse_y >= 10.0 && current_mouse_y <= 30.0
                                    && current_mouse_x >= (v_width - 219.0) && current_mouse_x < (v_width - 149.0);
                            } else {
                                hover_update = false;
                            }

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
                                || hover_update != old_hover_update
                                || hovered_tab_index != old_hovered_tab
                                || hovered_close_tab_index != old_hovered_close
                                || hover_new_tab != old_hover_new
                            {
                                renderer.lock().set_dirty(true);
                                app_dirty = true;
                            }

                            if is_dragging_scrollbar {
                                let term = tabs[active_tab_index].terminal_state.lock();
                                let history_size = term.history_size() as f32;
                                let visible_rows = shell_rows as f32;
                                drop(term);

                                let total_lines = visible_rows + history_size;
                                if total_lines > 0.0 {
                                    let ratio = visible_rows / total_lines;
                                    const TOPBAR_HEIGHT: f32 = 40.0;
                                    let scrollbar_top_margin = TOPBAR_HEIGHT;
                                    let track_h = v_height - scrollbar_top_margin - 4.0;
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
                                        let mut r = renderer.lock();
                                        r.set_dirty(true);
                                        r.grid_dirty = true;
                                        app_dirty = true;
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
                                            let mut r = renderer.lock();
                                            r.set_dirty(true);
                                            r.grid_dirty = true;
                                            app_dirty = true;
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
                                let mut r = renderer.lock();
                                r.set_dirty(true);
                                r.grid_dirty = true;
                                app_dirty = true;
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
                                    tabs[active_tab_index].scroll_target = (tabs[active_tab_index].scroll_target + delta_scroll).clamp(0.0, max_history);
                                }
                            } else {
                                tabs[active_tab_index].scroll_target = (tabs[active_tab_index].scroll_target + delta_scroll).clamp(0.0, max_history);
                            }
                            app_dirty = true;
                        }
                        WindowEvent::CursorLeft { .. } => {
                            window_for_redraw.set_cursor(winit::window::CursorIcon::Default);

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

                            if old_hover_close || old_hover_max || old_hover_min || old_hover_settings || old_hover_update || url_changed {
                                let mut r = renderer.lock();
                                r.hover_update = false;
                                r.set_dirty(true);
                                app_dirty = true;
                            }
                        }
                        WindowEvent::Focused(focused) => {
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
                        _ => {}
                    }
                } else if settings_window.as_ref().map_or(false, |sw| window_id == sw.id()) {
                    if let Some(ref sw) = settings_window {
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
                                
                                // Apply font family update to the settings renderer as well, but at fixed font size 13.0
                                if let Some(ref mut sr) = settings_renderer {
                                    let _ = sr.update_font(&settings_family, 13.0);
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

                                app_dirty = true;
                            }
                        }

                        match event {
                            WindowEvent::CloseRequested => {
                                settings_window = None;
                                settings_renderer = None;
                                app_dirty = true;
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
                                        s_hover_open_config,
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
                                let old_hover_open_config = s_hover_open_config;
                                let old_hover_save = s_hover_save;
                                let old_hover_cancel = s_hover_cancel;
                                let old_hovered_font_idx = settings_hovered_font_idx;

                                let sw_width = sw.inner_size().width as f64 / scale_factor;
                                s_hover_close = s_mouse_y >= 4.0 && s_mouse_y <= 32.0 && s_mouse_x >= (sw_width - 32.0) && s_mouse_x < (sw_width - 4.0);
                                s_hover_family = s_mouse_y >= 52.0 && s_mouse_y <= 78.0 && s_mouse_x >= 140.0 && s_mouse_x < 380.0;

                                s_hover_size_minus = s_mouse_y >= 92.0 && s_mouse_y <= 118.0 && s_mouse_x >= 140.0 && s_mouse_x < 168.0;
                                s_hover_size_plus = s_mouse_y >= 92.0 && s_mouse_y <= 118.0 && s_mouse_x >= 220.0 && s_mouse_x < 248.0;

                                s_hover_scroll_minus = s_mouse_y >= 132.0 && s_mouse_y <= 158.0 && s_mouse_x >= 140.0 && s_mouse_x < 168.0;
                                s_hover_scroll_plus = s_mouse_y >= 132.0 && s_mouse_y <= 158.0 && s_mouse_x >= 240.0 && s_mouse_x < 268.0;

                                s_hover_open_config = s_mouse_y >= 172.0 && s_mouse_y <= 198.0 && s_mouse_x >= 140.0 && s_mouse_x < 380.0;

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
                                    || s_hover_open_config != old_hover_open_config
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
                                        app_dirty = true;
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
                                        settings_scrollback = settings_scrollback.saturating_add(1000).min(3000);
                                        apply_settings!();
                                    } else if s_hover_open_config {
                                        let mut current_config = Config::load().unwrap_or_default();
                                        current_config.font.family = settings_family.clone();
                                        current_config.font.size = settings_size;
                                        current_config.scrollback = settings_scrollback;
                                        let path = Config::get_active_config_path();
                                        let _ = current_config.save(&path);
                                        let _ = open_file_in_editor(&path);
                                    } else if s_hover_save {
                                        settings_window = None;
                                        settings_renderer = None;
                                        app_dirty = true;
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
                                s_hover_open_config = false;
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
                                    s_hover_open_config = false;
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
                } else if about_window.as_ref().map_or(false, |aw| window_id == aw.id()) {
                    if let Some(ref aw) = about_window {
                        // --- About Window Event Handler ---
                        match event {
                            WindowEvent::CloseRequested => {
                                about_window = None;
                                about_renderer = None;
                                app_dirty = true;
                            }
                            WindowEvent::Resized(size) => {
                                if let Some(ref mut r) = about_renderer {
                                    r.resize(size.width, size.height);
                                }
                            }
                            WindowEvent::RedrawRequested => {
                                if let Some(ref mut r) = about_renderer {
                                    r.set_dirty(true);
                                    r.render_about(&get_current_version(), about_hover_close);
                                }
                            }
                            WindowEvent::CursorMoved { position, .. } => {
                                let scale_factor = aw.scale_factor();
                                let m_x = position.x / scale_factor;
                                let m_y = position.y / scale_factor;
                                about_mouse_y = m_y;

                                let old_hover_close = about_hover_close;
                                let aw_width = aw.inner_size().width as f64 / scale_factor;
                                about_hover_close = m_y >= 4.0 && m_y <= 32.0 && m_x >= (aw_width - 32.0) && m_x < (aw_width - 4.0);

                                if about_hover_close != old_hover_close {
                                    if let Some(ref mut r) = about_renderer {
                                        r.set_dirty(true);
                                    }
                                    aw.request_redraw();
                                }
                            }
                            WindowEvent::MouseInput { state, button, .. } => {
                                if button == MouseButton::Left && state == ElementState::Pressed {
                                    if about_hover_close {
                                        about_window = None;
                                        about_renderer = None;
                                        app_dirty = true;
                                    } else if about_mouse_y <= 36.0 {
                                        let _ = aw.drag_window();
                                    }
                                }
                            }
                            WindowEvent::CursorLeft { .. } => {
                                about_hover_close = false;
                                if let Some(ref mut r) = about_renderer {
                                    r.set_dirty(true);
                                }
                                aw.request_redraw();
                            }
                            WindowEvent::Focused(focused) => {
                                if !focused {
                                    about_hover_close = false;
                                    if let Some(ref mut r) = about_renderer {
                                        r.set_dirty(true);
                                    }
                                    aw.request_redraw();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            winit::event::Event::AboutToWait => {
                let now = std::time::Instant::now();
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

                    let mut r = renderer.lock();
                    r.set_dirty(true);
                    r.render(
                        next_render_reason,
                        term_ref,
                        active_tab.cursor_visible,
                        config.font.ligatures,
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
                        toast.as_ref().map(|(msg, t, d)| (msg.as_str(), *t, *d)),
                        active_tab_index,
                        &tab_titles,
                        &active_tab_path,
                        context_menu_visible,
                        context_menu_is_about,
                        context_menu_x as f32,
                        context_menu_y as f32,
                        context_menu_hovered_idx,
                        context_menu_open_time_secs,
                        hovered_tab_index,
                        hovered_close_tab_index,
                        hover_new_tab,
                    );
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
                    let last_rg = rg.load(Ordering::Relaxed);
                    term.update_render_generation(&rg);
                    let current_rg = rg.load(Ordering::Relaxed);
                    if current_rg != last_rg {
                        tab.last_activity_time = std::time::Instant::now();
                        tab.cursor_visible = true;
                        if idx == active_tab_index {
                            app_dirty = true;
                            renderer.lock().grid_dirty = true;
                        }
                    }
                }

                // Opacity animation of the scrollbar (uses active tab details)
                let v_width = renderer.lock().config.width as f64;
                const TOPBAR_HEIGHT: f32 = 40.0;
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
                    scrollbar_alpha += alpha_diff * 0.15;
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

fn key_to_bytes(
    key: &Key,
    shift_active: bool,
    ctrl_active: bool,
    alt_active: bool,
    mode: alacritty_terminal::term::TermMode,
) -> Vec<u8> {
    match key {
        Key::Character(s) if !s.is_empty() => {
            if alt_active && !ctrl_active {
                // Alt + key (Meta key — prefix with Escape)
                let mut bytes = vec![0x1B];
                bytes.extend_from_slice(s.as_bytes());
                bytes
            } else {
                s.as_bytes().to_vec()
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
                    } else {
                        vec![b'\r']
                    }
                }
                NamedKey::Space => vec![b' '],
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
                NamedKey::ArrowUp => {
                    if ctrl_active {
                        vec![0x1B, 0x5B, 0x31, 0x3B, 0x35, 0x41] // \x1b[1;5A
                    } else {
                        vec![0x1B, 0x5B, 0x41] // \x1b[A
                    }
                }
                NamedKey::ArrowDown => {
                    if ctrl_active {
                        vec![0x1B, 0x5B, 0x31, 0x3B, 0x35, 0x42] // \x1b[1;5B
                    } else {
                        vec![0x1B, 0x5B, 0x42] // \x1b[B
                    }
                }
                NamedKey::ArrowRight => {
                    if ctrl_active {
                        vec![0x1B, 0x5B, 0x31, 0x3B, 0x35, 0x43] // \x1b[1;5C
                    } else {
                        vec![0x1B, 0x5B, 0x43] // \x1b[C
                    }
                }
                NamedKey::ArrowLeft => {
                    if ctrl_active {
                        vec![0x1B, 0x5B, 0x31, 0x3B, 0x35, 0x44] // \x1b[1;5D
                    } else {
                        vec![0x1B, 0x5B, 0x44] // \x1b[D
                    }
                }

                // Navigation
                NamedKey::Home => {
                    if shift_active {
                        vec![0x1B, 0x5B, 0x31, 0x3B, 0x32, 0x48] // \x1b[1;2H
                    } else {
                        vec![0x1B, 0x5B, 0x48] // \x1b[H
                    }
                }
                NamedKey::End => {
                    if shift_active {
                        vec![0x1B, 0x5B, 0x31, 0x3B, 0x32, 0x46] // \x1b[1;2F
                    } else {
                        vec![0x1B, 0x5B, 0x46] // \x1b[F
                    }
                }
                NamedKey::PageUp => vec![0x1B, 0x5B, 0x35, 0x7E],   // \x1b[5~
                NamedKey::PageDown => vec![0x1B, 0x5B, 0x36, 0x7E], // \x1b[6~
                NamedKey::Insert => vec![0x1B, 0x5B, 0x32, 0x7E],   // \x1b[2~
                NamedKey::Delete => vec![0x1B, 0x5B, 0x33, 0x7E],   // \x1b[3~

                // Function keys
                NamedKey::F1 => vec![0x1B, 0x4F, 0x50],  // \x1bOP
                NamedKey::F2 => vec![0x1B, 0x4F, 0x51],  // \x1bOQ
                NamedKey::F3 => vec![0x1B, 0x4F, 0x52],  // \x1bOR
                NamedKey::F4 => vec![0x1B, 0x4F, 0x53],  // \x1bOS
                NamedKey::F5 => vec![0x1B, 0x5B, 0x31, 0x35, 0x7E], // \x1b[15~
                NamedKey::F6 => vec![0x1B, 0x5B, 0x31, 0x37, 0x7E], // \x1b[17~
                NamedKey::F7 => vec![0x1B, 0x5B, 0x31, 0x38, 0x7E], // \x1b[18~
                NamedKey::F8 => vec![0x1B, 0x5B, 0x31, 0x39, 0x7E], // \x1b[19~
                NamedKey::F9 => vec![0x1B, 0x5B, 0x32, 0x30, 0x7E], // \x1b[20~
                NamedKey::F10 => vec![0x1B, 0x5B, 0x32, 0x31, 0x7E], // \x1b[21~
                NamedKey::F11 => vec![0x1B, 0x5B, 0x32, 0x33, 0x7E], // \x1b[23~
                NamedKey::F12 => vec![0x1B, 0x5B, 0x32, 0x34, 0x7E], // \x1b[24~

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
        Ok(_) => {}
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
        if let Ok(output) = std::process::Command::new("reg")
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
) {
    let completed = *update_completed.lock();
    if completed {
        // Spawn the newly updated fasty binary in the background!
        #[cfg(target_os = "windows")]
        {
            let home = std::env::var("USERPROFILE").unwrap_or_default();
            let fasty_path = std::path::Path::new(&home)
                .join(".local")
                .join("bin")
                .join("fasty.exe");
            let _ = std::process::Command::new(fasty_path).spawn();
        }

        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .arg("-a")
                .arg("Fasty")
                .spawn();
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            // Linux
            let binary_path = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/fasty"));
            let home = std::env::var("HOME").unwrap_or_default();
            let local_path = std::path::Path::new(&home)
                .join(".local")
                .join("bin")
                .join("fasty");
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

    let update_in_progress_clone = Arc::clone(update_in_progress);
    let update_completed_clone = Arc::clone(update_completed);
    let window_clone = Arc::clone(window);

    std::thread::spawn(move || {
        let mut success = false;

        #[cfg(target_os = "windows")]
        {
            if let Ok(mut child) = std::process::Command::new("powershell")
                .arg("-Command")
                .arg("irm https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.ps1 | iex")
                .spawn() {
                if let Ok(status) = child.wait() {
                    success = status.success();
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            if let Ok(mut child) = std::process::Command::new("sh")
                .arg("-c")
                .arg("curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash")
                .env("FASTY_USER_INSTALL", "1")
                .spawn() {
                if let Ok(status) = child.wait() {
                    success = status.success();
                }
            }
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            // Linux and others
            if let Ok(mut child) = std::process::Command::new("sh")
                .arg("-c")
                .arg("curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash")
                .env("FASTY_USER_INSTALL", "1")
                .spawn() {
                if let Ok(status) = child.wait() {
                    success = status.success();
                }
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

fn open_file_in_editor(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
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
    fn test_fasty_args_parsing() {
        // Test no args
        let args = FastyArgs::parse_from(vec![]);
        assert!(args.command.is_none());
        assert!(args.working_dir.is_none());
        assert!(args.title.is_none());

        // Test title and directory
        let args = FastyArgs::parse_from(vec![
            "--title".to_string(),
            "My Terminal".to_string(),
            "-d".to_string(),
            "/home/user/project".to_string(),
        ]);
        assert!(args.command.is_none());
        assert_eq!(args.working_dir.as_deref(), Some("/home/user/project"));
        assert_eq!(args.title.as_deref(), Some("My Terminal"));

        // Test command with multiple args
        let args = FastyArgs::parse_from(vec![
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
        let args = FastyArgs::parse_from(vec![
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