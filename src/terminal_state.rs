//! Terminal state wrapper using alacritty_terminal.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::Config as AlacrittyConfig;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use alacritty_terminal::grid::Dimensions;
use parking_lot::Mutex as ParkingMutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::config::FontConfig;
use crate::event_listener::EventListenerProxy;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AppEvent {
    Wakeup,
    Exit,
    ForceExit,
    ConfigChanged,
    Bell,
    ClipboardStore(String),
    ClipboardLoad(u8),
    CwdChanged(std::path::PathBuf),
    TitleChanged(String),
    CommandStarted,
    CommandFinished { duration_ms: u128, exit_code: Option<i32> },
    Notification { title: String, body: String },
    PromptStarted { absolute_line: u64 },
    ShowToast { message: String, duration_ms: u64 },
    ForcePollWidgets,
    GitStatusUpdated {
        window_id: Option<winit::window::WindowId>,
        tab_idx: usize,
        status: Option<crate::git::GitStatus>,
    },
    GitRepoChanged {
        repo_path: std::path::PathBuf,
    },
}



pub struct TerminalState {
    term: Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>>,
    render_generation: Arc<AtomicU64>,
    writer: Arc<ParkingMutex<Box<dyn Write + Send>>>,
    master: Arc<ParkingMutex<Box<dyn MasterPty + Send>>>,
    shell_pid: Option<u32>,
    pub total_lines_pushed: Arc<AtomicU64>,
}

impl TerminalState {
    pub fn new(
        executable: &str,
        exec_args: &[String],
        cwd: Option<&str>,
        scrollback: usize,
        _font_config: FontConfig,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    ) -> anyhow::Result<Self> {
        let cell_w = (cell_width as usize).max(1);
        let cell_h = (cell_height as usize).max(1);
        let cols = ((viewport_width as usize) / cell_w).max(80);
        let rows = ((viewport_height as usize) / cell_h).max(24);

        let mut config = AlacrittyConfig::default();
        config.scrolling_history = scrollback.min(1000);
        let size = TermSize::new(cols, rows);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: (cols as u16) * (cell_width as u16),
                pixel_height: (rows as u16) * (cell_height as u16),
            })
            .expect("Failed to open PTY");

        let mut cmd = CommandBuilder::new(executable);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "ghostty");
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        if let Ok(lang) = std::env::var("LANG") {
            cmd.env("LANG", lang);
        }
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }

        // Shell integration: write OSC 133 command markers so fasty can
        // detect command start/finish for duration tracking.
        let integration_path = std::env::temp_dir().join("fasty_shell_integration.sh");
        let integration_path_fish = std::env::temp_dir().join("fasty_shell_integration.fish");

        // POSIX shell integration (bash/zsh)
        let _ = std::fs::write(
            &integration_path,
            "# fasty shell integration — OSC 133 command markers\n\
             __fasty_cmd_start() {\n\
             \techo -ne \"\\e]133;B\\e\\\\\"\n\
             }\n\
             __fasty_prompt() {\n\
             \techo -ne \"\\e]133;D;$?\\e\\\\\"\n\
             \techo -ne \"\\e]133;A\\e\\\\\"\n\
             }\n\
             PROMPT_COMMAND=\"__fasty_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"\n\
             trap '__fasty_cmd_start' DEBUG\n",
        );

        // Fish shell integration
        let _ = std::fs::write(
            &integration_path_fish,
            "# fasty shell integration — OSC 133 command markers\n\
             function __fasty_cmd_start --on-event fish_preexec\n\
             \techo -ne \"\\e]133;B\\e\\\\\"\n\
             end\n\
             function __fasty_cmd_end --on-event fish_postexec\n\
             \techo -ne \"\\e]133;D;$status\\e\\\\\"\n\
             end\n\
             function __fasty_prompt --on-event fish_prompt\n\
             \techo -ne \"\\e]133;A\\e\\\\\"\n\
             end\n",
        );

        // Wrap the shell command to source the integration before launching.
        let shell_name = std::path::Path::new(&executable)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if shell_name == "bash" && exec_args.is_empty() {
            // Bash: create wrapper bashrc that sources integration + user's .bashrc
            let user_bashrc = std::env::var("HOME")
                .map(|h| format!("{}/.bashrc", h))
                .unwrap_or_default();
            let wrapper_path = std::env::temp_dir().join("fasty_bashrc");
            let _ = std::fs::write(
                &wrapper_path,
                format!(
                    "# Auto-generated by fasty — shell integration wrapper\n\
                     . {}\n\
                     [ -f {} ] && . {}\n",
                    integration_path.display(),
                    user_bashrc,
                    user_bashrc,
                ),
            );
            cmd.args(&["--rcfile", wrapper_path.to_str().unwrap_or("")]);
        } else if shell_name == "zsh" && exec_args.is_empty() {
            // Zsh: use ZDOTDIR to inject .zshenv
            let zdotdir = std::env::temp_dir().join("fasty_zsh");
            let _ = std::fs::create_dir_all(&zdotdir);
            let _ = std::fs::write(
                zdotdir.join(".zshenv"),
                format!(". {}\n", integration_path.display()),
            );
            cmd.env("ZDOTDIR", zdotdir);
        } else if shell_name == "fish" && exec_args.is_empty() {
            // Fish: use -C to source integration on startup
            let source_cmd = format!("source {}", integration_path_fish.display());
            cmd.args(&["-C", &source_cmd]);
        } else {
            cmd.args(exec_args);
        }

        let child = pair.slave.spawn_command(cmd)?;
        let shell_pid = child.process_id();

        drop(pair.slave);

        let master: Box<dyn MasterPty + Send> = pair.master;
        let master_arc: Arc<ParkingMutex<Box<dyn MasterPty + Send>>> =
            Arc::new(ParkingMutex::new(master));
        let mut reader = master_arc
            .lock()
            .try_clone_reader()
            .expect("Failed to clone reader");
        let writer = master_arc.lock().take_writer().expect("Failed to take writer");

        let writer_boxed: Box<dyn Write + Send> = Box::new(writer);
        let writer_arc: Arc<ParkingMutex<Box<dyn Write + Send>>> =
            Arc::new(ParkingMutex::new(writer_boxed));

        let mut event_listener = EventListenerProxy::from_arc(writer_arc.clone());
        event_listener.set_app_proxy(proxy.clone());
        let term = Arc::new(ParkingMutex::new(alacritty_terminal::term::Term::new(
            config,
            &size,
            event_listener,
        )));

        // Write echo command directly in main thread (already done above)
        // Remove duplicate write in thread

        let render_generation = Arc::new(AtomicU64::new(0));
        let render_gen_clone = Arc::clone(&render_generation);
        let term_clone = Arc::clone(&term);
        let proxy_clone = proxy.clone();
        let total_lines_pushed = Arc::new(AtomicU64::new(0));
        let total_lines_pushed_clone = Arc::clone(&total_lines_pushed);
        let writer_clone = Arc::clone(&writer_arc);
        thread::spawn(move || {
            use std::io::Read;

            let mut buf = [0u8; 65536];
            let mut parser: Processor<StdSyncHandler> = Processor::new();

            #[derive(Clone, Copy, Debug, PartialEq)]
            enum OscParseState {
                Normal,
                Esc,
                Osc,
                OscEsc,
            }

            let mut osc_state = OscParseState::Normal;
            let mut osc_buf: Vec<u8> = Vec::new();
            let mut cmd_start_time: Option<std::time::Instant> = None;

            struct PendingNotification {
                title: String,
                body: String,
            }
            let mut pending_notifications: std::collections::HashMap<String, PendingNotification> = std::collections::HashMap::new();

            let mut handle_cmd = |cmd: OscCommand, cursor_line: i32, screen_lines: i32, base: u64| {
                match cmd {
                    OscCommand::NotificationQuery { id } => {
                        let resp = format!("\x1b]99;i={}:p=OK;\x1b\\", id);
                        let mut w = writer_clone.lock();
                        let _ = w.write_all(resp.as_bytes());
                        let _ = w.flush();
                    }
                    OscCommand::NotificationFragment { id, p_type, done, payload } => {
                        if let Some(query_id) = id {
                            let entry = pending_notifications.entry(query_id.clone()).or_insert_with(|| PendingNotification {
                                title: String::new(),
                                body: String::new(),
                            });
                            match p_type.as_deref() {
                                Some("title") => entry.title.push_str(&payload),
                                Some("body") | None => entry.body.push_str(&payload),
                                _ => {}
                            }
                            if done {
                                if let Some(finished) = pending_notifications.remove(&query_id) {
                                    let title = if finished.title.is_empty() {
                                        "Fasty".to_string()
                                    } else {
                                        finished.title
                                    };
                                    dispatch_osc_action(
                                        &OscCommand::Notification { title, body: finished.body },
                                        &proxy_clone,
                                        cursor_line,
                                        base,
                                        screen_lines,
                                    );
                                }
                            }
                        } else {
                            dispatch_osc_action(
                                &OscCommand::Notification { title: "Fasty".to_string(), body: payload },
                                &proxy_clone,
                                cursor_line,
                                base,
                                screen_lines,
                            );
                        }
                    }
                    _ => {
                        dispatch_osc_action(&cmd, &proxy_clone, cursor_line, base, screen_lines);
                    }
                }
            };

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = proxy_clone.send_event(AppEvent::Exit);
                        break;
                    }
                    Ok(n) => {
                        let mut term_locked = term_clone.lock();
                        let mut local_lines = 0;

                        for &byte in buf[..n].iter() {
                            if byte == 0x0A {
                                local_lines += 1;
                            }

                            parser.advance(&mut *term_locked, byte);

                            match osc_state {
                                OscParseState::Normal => {
                                    if byte == 0x1b {
                                        osc_state = OscParseState::Esc;
                                    }
                                }
                                OscParseState::Esc => {
                                    if byte == b']' {
                                        osc_state = OscParseState::Osc;
                                        osc_buf.clear();
                                    } else {
                                        osc_state = OscParseState::Normal;
                                    }
                                }
                                OscParseState::Osc => {
                                    if byte == 0x07 {
                                        if let Some(cmd) = parse_osc(&osc_buf, &mut cmd_start_time) {
                                            let cursor_line = term_locked.grid().cursor.point.line.0 as i32;
                                            let screen_lines = term_locked.grid().screen_lines() as i32;
                                            let base = total_lines_pushed_clone.load(Ordering::Relaxed) + local_lines;
                                            handle_cmd(cmd, cursor_line, screen_lines, base);
                                        }
                                        osc_state = OscParseState::Normal;
                                    } else if byte == 0x1b {
                                        osc_state = OscParseState::OscEsc;
                                    } else {
                                        osc_buf.push(byte);
                                        if osc_buf.len() > 4096 {
                                            osc_state = OscParseState::Normal;
                                        }
                                    }
                                }
                                OscParseState::OscEsc => {
                                    if byte == b'\\' {
                                        if let Some(cmd) = parse_osc(&osc_buf, &mut cmd_start_time) {
                                            let cursor_line = term_locked.grid().cursor.point.line.0 as i32;
                                            let screen_lines = term_locked.grid().screen_lines() as i32;
                                            let base = total_lines_pushed_clone.load(Ordering::Relaxed) + local_lines;
                                            handle_cmd(cmd, cursor_line, screen_lines, base);
                                        }
                                        osc_state = OscParseState::Normal;
                                    } else {
                                        osc_buf.push(0x1b);
                                        osc_buf.push(byte);
                                        osc_state = OscParseState::Osc;
                                        if osc_buf.len() > 4096 {
                                            osc_state = OscParseState::Normal;
                                        }
                                    }
                                }
                            }
                        }
                        drop(term_locked);

                        total_lines_pushed_clone.fetch_add(local_lines, Ordering::Relaxed);

                        render_gen_clone.fetch_add(1, Ordering::Relaxed);
                        let _ = proxy_clone.send_event(AppEvent::Wakeup);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                        continue;
                    }
                    Err(_) => {
                        let _ = proxy_clone.send_event(AppEvent::Exit);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            term,
            render_generation,
            writer: writer_arc,
            master: master_arc,
            shell_pid,
            total_lines_pushed,
        })
    }

    pub fn shell_pid(&self) -> Option<u32> {
        self.shell_pid
    }

    pub fn write_to_pty(&self, bytes: &[u8]) {
        let mut w = self.writer.lock();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    pub fn update_scrollback(&self, scrollback: usize) {
        let mut term = self.term.lock();
        term.grid_mut().update_history(scrollback.min(3000));
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 {
            return;
        }
        let mut term = self.term.lock();
        let size = TermSize::new(cols, rows);
        term.resize(size);
        drop(term);

        self.master.lock().resize(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        }).ok();

        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll(&self, delta: isize) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        term.scroll_display(Scroll::Delta(delta as i32));
    }

    pub fn display_offset(&self) -> usize {
        let term = self.term.lock();
        term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        use alacritty_terminal::grid::Dimensions;
        let term = self.term.lock();
        term.history_size()
    }



    pub fn render_generation(&self) -> u64 {
        self.render_generation.load(Ordering::Relaxed)
    }

    pub fn term(&self) -> &Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>> {
        &self.term
    }
}

/// Decode a `file://` URL to a local filesystem path.
/// Accepts both `file:///abs/path` and `file://hostname/abs/path` forms.
fn file_url_to_path(buf: &[u8]) -> Option<std::path::PathBuf> {
    let s = std::str::from_utf8(buf).ok()?;
    let path = s.strip_prefix("file://")?;
    // Reject host-qualified URLs like `file://localhost/abs`
    let path = if let Some(slash_idx) = path.find('/') {
        let prefix = &path[..slash_idx];
        // If the "host" part contains a dot, colon, or looks like a real hostname, reject it
        if prefix.contains('.') || prefix.contains(':') || (!prefix.is_empty() && prefix != "localhost") {
            return None;
        }
        &path[slash_idx..]
    } else {
        path
    };
    // URL-decode minimal: %20 -> space, etc.
    let decoded = percent_decode(path);
    Some(std::path::PathBuf::from(decoded))
}

/// Minimal percent-decode for file:// paths.
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                if let Some(c) = char::from_u32((h as u32) * 16 + (l as u32)) {
                    out.push(c);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut table = [0u8; 256];
    for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
        table[c as usize] = i as u8;
    }
    
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    
    for &b in s.as_bytes() {
        if b == b'=' {
            break;
        }
        let val = table[b as usize];
        if val == 0 && b != b'A' {
            continue; // Skip whitespace/invalid chars
        }
        buffer = (buffer << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[derive(Debug, Clone)]
enum OscCommand {
    Cwd(String),
    CommandStarted,
    CommandFinished { duration_ms: u128, exit_code: Option<i32> },
    PromptStarted,
    Notification { title: String, body: String },
    NotificationQuery { id: String },
    NotificationFragment {
        id: Option<String>,
        p_type: Option<String>,
        done: bool,
        payload: String,
    },
}

fn parse_osc(
    buf: &[u8],
    cmd_start_time: &mut Option<std::time::Instant>,
) -> Option<OscCommand> {
    let semicolon = buf.iter().position(|&b| b == b';')?;
    let code = &buf[..semicolon];
    let payload = &buf[semicolon + 1..];

    match code {
        b"7" => {
            if let Ok(s) = std::str::from_utf8(payload) {
                Some(OscCommand::Cwd(s.to_string()))
            } else {
                None
            }
        }
        b"9" => {
            if let Ok(msg) = std::str::from_utf8(payload) {
                Some(OscCommand::Notification {
                    title: "Fasty".to_string(),
                    body: msg.to_string(),
                })
            } else {
                None
            }
        }
        b"99" => {
            let (metadata_bytes, payload_bytes) = if let Some(idx) = payload.iter().position(|&b| b == b';') {
                (&payload[..idx], &payload[idx + 1..])
            } else {
                (payload, &[][..])
            };

            let metadata = std::str::from_utf8(metadata_bytes).ok()?;
            let actual_payload = std::str::from_utf8(payload_bytes).ok()?;

            let mut id = None;
            let mut p_type = None;
            let mut done = true;
            let mut is_base64 = false;

            for part in metadata.split(':') {
                if let Some(eq) = part.find('=') {
                    let key = &part[..eq];
                    let val = &part[eq + 1..];
                    match key {
                        "i" => id = Some(val.to_string()),
                        "p" => p_type = Some(val.to_string()),
                        "d" => done = val == "1" || val != "0",
                        "e" => is_base64 = val == "1",
                        _ => {}
                    }
                }
            }

            let decoded_payload = if is_base64 {
                if let Some(bytes) = base64_decode(actual_payload) {
                    String::from_utf8(bytes).unwrap_or_else(|_| actual_payload.to_string())
                } else {
                    actual_payload.to_string()
                }
            } else {
                actual_payload.to_string()
            };

            if p_type.as_deref() == Some("?") {
                if let Some(query_id) = id {
                    return Some(OscCommand::NotificationQuery { id: query_id });
                }
            }

            Some(OscCommand::NotificationFragment {
                id,
                p_type,
                done,
                payload: decoded_payload,
            })
        }
        b"777" => {
            if payload.starts_with(b"notify;") {
                let parts: Vec<&[u8]> = payload[7..].split(|&b| b == b';').collect();
                if parts.len() >= 2 {
                    let title = std::str::from_utf8(parts[0]).unwrap_or("Fasty").to_string();
                    let body = parts[1..]
                        .iter()
                        .map(|p| std::str::from_utf8(p).unwrap_or(""))
                        .collect::<Vec<&str>>()
                        .join(";");
                    Some(OscCommand::Notification { title, body })
                } else if parts.len() == 1 {
                    let body = std::str::from_utf8(parts[0]).unwrap_or("").to_string();
                    Some(OscCommand::Notification {
                        title: "Fasty".to_string(),
                        body,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }
        b"133" => {
            match payload {
                b"A" => {
                    Some(OscCommand::PromptStarted)
                }
                b"B" => {
                    *cmd_start_time = Some(std::time::Instant::now());
                    Some(OscCommand::CommandStarted)
                }
                b"D" => {
                    if let Some(start) = cmd_start_time.take() {
                        let duration_ms = start.elapsed().as_millis();
                        Some(OscCommand::CommandFinished {
                            duration_ms,
                            exit_code: None,
                        })
                    } else {
                        None
                    }
                }
                _ => {
                    if payload.starts_with(b"D;") {
                        let exit_str = std::str::from_utf8(&payload[2..]).unwrap_or("0");
                        let exit_code: Option<i32> = exit_str.parse().ok();
                        if let Some(start) = cmd_start_time.take() {
                            let duration_ms = start.elapsed().as_millis();
                            Some(OscCommand::CommandFinished {
                                duration_ms,
                                exit_code,
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            }
        }
        _ => None,
    }
}

fn dispatch_osc_action(
    cmd: &OscCommand,
    proxy: &winit::event_loop::EventLoopProxy<AppEvent>,
    cursor_line: i32,
    absolute_base: u64,
    screen_lines: i32,
) {
    match cmd {
        OscCommand::Cwd(path) => {
            if let Some(p) = file_url_to_path(path.as_bytes()) {
                let _ = proxy.send_event(AppEvent::CwdChanged(p));
            }
        }
        OscCommand::CommandStarted => {
            let _ = proxy.send_event(AppEvent::CommandStarted);
        }
        OscCommand::CommandFinished { duration_ms, exit_code } => {
            let _ = proxy.send_event(AppEvent::CommandFinished {
                duration_ms: *duration_ms,
                exit_code: *exit_code,
            });
        }
        OscCommand::PromptStarted => {
            let scrolled = (absolute_base as i32 - screen_lines).max(0);
            let absolute_line = scrolled + cursor_line;
            let _ = proxy.send_event(AppEvent::PromptStarted {
                absolute_line: absolute_line.max(0) as u64,
            });
        }
        OscCommand::Notification { title, body } => {
            let _ = proxy.send_event(AppEvent::Notification {
                title: title.clone(),
                body: body.clone(),
            });
        }
        OscCommand::NotificationQuery { .. } | OscCommand::NotificationFragment { .. } => {}
    }
}