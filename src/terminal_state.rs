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
    Exit { shell_pid: Option<u32> },
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
        window_id: Option<usize>,
        tab_idx: usize,
        status: Option<crate::git::GitStatus>,
    },
    GitRepoChanged {
        repo_path: std::path::PathBuf,
    },
}



/// Events derived from raw PTY byte-stream CSI scanning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsiEvent {
    /// Plain `CSI 2J` — callers clear main-screen scrollback.
    ClearHistory,
    /// `CSI ? ... 2026 ... h` — Synchronized Output frame begins.
    SyncOutputBegin,
    /// `CSI ? ... 2026 ... l` — Synchronized Output frame ends.
    SyncOutputEnd,
}

/// Byte-level watcher for the CSI sequences fastty handles outside the
/// alacritty model: scrollback wipe on plain `CSI 2J`, and Synchronized
/// Output (DEC private mode 2026), which alacritty_terminal 0.24 accepts as
/// a silent no-op. TUI frameworks (ratatui, bubbletea, Ink) wrap each frame
/// in `?2026h`/`?2026l`, so the frontend must suppress repaints between the
/// two to avoid painting half-built frames.
#[derive(Debug)]
pub struct CsiWatcher {
    state: CsiState,
    private: bool,
    params: [u32; 8],
    param_count: usize,
    digit_acc: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CsiState {
    Ground,
    Esc,
    Params,
}

impl CsiWatcher {
    pub fn new() -> Self {
        Self {
            state: CsiState::Ground,
            private: false,
            params: [0; 8],
            param_count: 0,
            digit_acc: 0,
        }
    }

    pub fn feed(&mut self, byte: u8) -> Option<CsiEvent> {
        match self.state {
            CsiState::Ground => {
                if byte == 0x1b {
                    self.state = CsiState::Esc;
                }
                None
            }
            CsiState::Esc => {
                if byte == b'[' {
                    self.state = CsiState::Params;
                    self.private = false;
                    self.params = [0; 8];
                    self.param_count = 0;
                    self.digit_acc = 0;
                    None
                } else {
                    self.state = CsiState::Ground;
                    None
                }
            }
            CsiState::Params => {
                if byte.is_ascii_digit() {
                    self.digit_acc = self
                        .digit_acc
                        .saturating_mul(10)
                        .saturating_add((byte - b'0') as u32);
                    None
                } else if byte == b'?' && self.param_count == 0 && self.digit_acc == 0 {
                    self.private = true;
                    None
                } else if byte == b';' || byte == b':' {
                    self.push_param();
                    None
                } else {
                    self.push_param();
                    let params = &self.params[..self.param_count];
                    let event = if byte == b'J' && !self.private && params.contains(&2) {
                        Some(CsiEvent::ClearHistory)
                    } else if self.private && byte == b'h' && params.contains(&2026) {
                        Some(CsiEvent::SyncOutputBegin)
                    } else if self.private && byte == b'l' && params.contains(&2026) {
                        Some(CsiEvent::SyncOutputEnd)
                    } else {
                        None
                    };
                    self.state = CsiState::Ground;
                    event
                }
            }
        }
    }

    fn push_param(&mut self) {
        if self.param_count < self.params.len() {
            self.params[self.param_count] = self.digit_acc;
            self.param_count += 1;
        }
        self.digit_acc = 0;
    }
}

impl Default for CsiWatcher {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TerminalState {
    term: Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>>,
    render_generation: Arc<AtomicU64>,
    writer: Arc<ParkingMutex<Box<dyn Write + Send>>>,
    master: Arc<ParkingMutex<Box<dyn MasterPty + Send>>>,
    shell_pid: Option<u32>,
    pub total_lines_pushed: Arc<AtomicU64>,
    pub prompt_lines: Arc<ParkingMutex<Vec<u64>>>,
    event_listener: EventListenerProxy,
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
        sender: crate::event_listener::EventSender,
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

        // Determine the shell binary name (e.g. "zsh", "bash", "fish") so we can
        // choose the right CommandBuilder constructor and integration strategy.
        let _shell_name = std::path::Path::new(&executable)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // On macOS, when launched as a .app bundle from Finder/Dock/Spotlight,
        // the process inherits a minimal launchd PATH instead of the full login-shell
        // PATH. Using new_default_prog() for zsh activates portable-pty's argv0 `-`
        // trick (e.g. `-zsh`), which makes the kernel treat the process as a login
        // shell so /etc/zprofile (and path_helper) run correctly.
        //
        // Restriction: new_default_prog() panics if .arg()/.args() is called on it,
        // so we only use it for the zsh-without-exec-args branch. bash and fish still
        // use .args() for their --rcfile / -C wrappers and must stay on new().
        let mut cmd = CommandBuilder::new(executable);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "fastty");
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        if let Ok(lang) = std::env::var("LANG") {
            cmd.env("LANG", lang);
        }
        if let Ok(path) = std::env::var("PATH") {
            #[cfg(target_os = "macos")]
            {
                let mut full_path = path;
                for p in ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/local/sbin"] {
                    if !full_path.split(':').any(|segment| segment == p) && std::path::Path::new(p).exists() {
                        full_path = format!("{}:{}", p, full_path);
                    }
                }
                cmd.env("PATH", full_path);
            }
            #[cfg(not(target_os = "macos"))]
            {
                cmd.env("PATH", path);
            }
        }

        // Shell integration: write OSC 133 command markers so fastty can
        // detect command start/finish for duration tracking.
        //
        // The integration scripts are identical for every tab and never change
        // between releases, so materialise them at most once per process. This
        // avoids redundant filesystem writes on every `create_new_tab` call —
        // which matters at startup when a saved session is restored and many
        // tabs are created back-to-back.
        static INTEGRATION_WRITTEN: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        let integration_dir = crate::paths::get().cache_dir.join("shell_integration");
        let integration_path = integration_dir.join("fastty_shell_integration.sh");
        let integration_path_zsh = integration_dir.join("fastty_shell_integration.zsh");
        let integration_path_fish = integration_dir.join("fastty_shell_integration.fish");
        let zdotdir = integration_dir.join("fastty_zsh");
        INTEGRATION_WRITTEN.get_or_init(|| {
            let _ = std::fs::create_dir_all(&integration_dir);
            let _ = std::fs::create_dir_all(&zdotdir);

            // POSIX shell integration (bash)
            let _ = std::fs::write(
                &integration_path,
                "# fastty shell integration — OSC 133 command markers\n\
                 __fastty_cmd_start() {\n\
                 \techo -ne \"\\e]133;B\\e\\\\\"\n\
                 }\n\
                 __fastty_prompt() {\n\
                 \techo -ne \"\\e]133;D;$?\\e\\\\\"\n\
                 \techo -ne \"\\e]133;A\\e\\\\\"\n\
                 }\n\
                 PROMPT_COMMAND=\"__fastty_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"\n\
                 trap '__fastty_cmd_start' DEBUG\n",
            );

            // Zsh shell integration: precmd/preexec hooks appended via add-zsh-hook
            // so user-defined hooks in ~/.zshrc are preserved, not overwritten.
            let _ = std::fs::write(
                &integration_path_zsh,
                "# fastty shell integration — OSC 133 command markers\n\
                 autoload -Uz add-zsh-hook\n\
                 __fastty_preexec() {\n\
                 \techo -ne \"\\e]133;B\\e\\\\\"\n\
                 }\n\
                 __fastty_precmd() {\n\
                 \techo -ne \"\\e]133;D;$?\\e\\\\\"\n\
                 \techo -ne \"\\e]133;A\\e\\\\\"\n\
                 }\n\
                 add-zsh-hook preexec __fastty_preexec\n\
                 add-zsh-hook precmd __fastty_precmd\n",
            );

            // Fish shell integration
            let _ = std::fs::write(
                &integration_path_fish,
                "# fastty shell integration — OSC 133 command markers\n\
                 function __fastty_cmd_start --on-event fish_preexec\n\
                 \techo -ne \"\\e]133;B\\e\\\\\"\n\
                 end\n\
                 function __fastty_cmd_end --on-event fish_postexec\n\
                 \techo -ne \"\\e]133;D;$status\\e\\\\\"\n\
                 end\n\
                 function __fastty_prompt --on-event fish_prompt\n\
                 \techo -ne \"\\e]133;A\\e\\\\\"\n\
                 end\n",
            );

            // Bash wrapper
            let wrapper_path_bash = integration_dir.join("fastty_bashrc");
            let _ = std::fs::write(
                &wrapper_path_bash,
                format!(
                    "# Auto-generated by fastty — shell integration wrapper\n\
                     [ -f \"$HOME/.bash_profile\" ] && . \"$HOME/.bash_profile\"\n\
                     [ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"\n\
                     [ -f \"$HOME/.profile\" ] && . \"$HOME/.profile\"\n\
                     . \"{}\"\n",
                    integration_path.display(),
                ),
            );

            // Zsh wrappers
            let _ = std::fs::write(
                zdotdir.join(".zshenv"),
                "# Auto-generated by fastty\n\
                 [ -f \"$HOME/.zshenv\" ] && . \"$HOME/.zshenv\"\n",
            );
            let _ = std::fs::write(
                zdotdir.join(".zprofile"),
                "# Auto-generated by fastty\n\
                 [ -f \"$HOME/.zprofile\" ] && . \"$HOME/.zprofile\"\n",
            );
            let _ = std::fs::write(
                zdotdir.join(".zshrc"),
                format!(
                    "# Auto-generated by fastty\n\
                     [ -f \"$HOME/.zshrc\" ] && . \"$HOME/.zshrc\"\n\
                     . \"{}\"\n\
                     if [ -n \"$FASTTY_ORIG_ZDOTDIR\" ]; then\n\
                     \texport ZDOTDIR=\"$FASTTY_ORIG_ZDOTDIR\"\n\
                     else\n\
                     \tunset ZDOTDIR\n\
                     fi\n",
                    integration_path_zsh.display(),
                ),
            );
            let _ = std::fs::write(
                zdotdir.join(".zlogin"),
                "# Auto-generated by fastty\n\
                 [ -f \"$HOME/.zlogin\" ] && . \"$HOME/.zlogin\"\n",
            );
        });

        // Wrap the shell command to source the integration before launching.
        if !exec_args.is_empty() {
            cmd.args(exec_args);
        } else {
            #[cfg(unix)]
            {
                let shell_name = std::path::Path::new(&executable)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if matches!(shell_name, "zsh" | "bash" | "fish" | "sh" | "dash" | "ksh") {
                    cmd.arg("-l");
                }
            }
        }

        let no_spawn = std::env::var("FASTTY_NO_SPAWN").is_ok();
        let child = if no_spawn {
            None
        } else {
            Some(pair.slave.spawn_command(cmd)?)
        };
        let shell_pid = child.as_ref().and_then(|c| c.process_id());
        let reader_shell_pid = shell_pid;

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

        let event_listener = EventListenerProxy::from_arc(writer_arc.clone());
        event_listener.set_event_sender(sender.clone());
        let term = Arc::new(ParkingMutex::new(alacritty_terminal::term::Term::new(
            config,
            &size,
            event_listener.clone(),
        )));

        let render_generation = Arc::new(AtomicU64::new(0));
        let render_gen_clone = Arc::clone(&render_generation);
        let term_clone = Arc::clone(&term);
        let event_listener_clone = event_listener.clone();
        let total_lines_pushed = Arc::new(AtomicU64::new(0));
        let total_lines_pushed_clone = Arc::clone(&total_lines_pushed);
        let prompt_lines = Arc::new(ParkingMutex::new(Vec::new()));
        let prompt_lines_clone = Arc::clone(&prompt_lines);
        let writer_clone = Arc::clone(&writer_arc);
        if !std::env::var("FASTTY_NO_READER").is_ok() {
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

            // CSI detection: see `CsiWatcher` (module level). The watcher
            // emits events for `CSI 2J` scrollback wipes and for Synchronized
            // Output mode 2026, which alacritty_terminal 0.24 ignores.
            let mut csi_watcher = CsiWatcher::new();
            let mut clear_history_flag = false;

            const SYNC_OUTPUT_TIMEOUT: std::time::Duration =
                std::time::Duration::from_millis(150);
            let mut sync_output_active = false;
            let mut sync_started: Option<std::time::Instant> = None;

            // Debug: capture PTY bytes to log if FASTTY_PTY_DEBUG=1
            let pty_debug = std::env::var("FASTTY_PTY_DEBUG").ok().as_deref() == Some("1");
            let pty_log_path = crate::paths::get().state_dir.join("fastty_pty_debug.log");
            if pty_debug {
                let _ = std::fs::write(&pty_log_path, "");  // truncate on start
            }

            struct PendingNotification {
                title: String,
                body: String,
            }
            let mut pending_notifications: std::collections::HashMap<String, PendingNotification> = std::collections::HashMap::new();

            let mut handle_cmd = |cmd: OscCommand, cursor_line: i32, screen_lines: i32, base: u64| {
                match cmd {
                    OscCommand::PromptStarted => {
                        let scrolled = (base as i64 - screen_lines as i64).max(0);
                        let absolute_line = (scrolled + cursor_line as i64).max(0) as u64;
                        {
                            let mut p = prompt_lines_clone.lock();
                            if p.last().copied() != Some(absolute_line) {
                                p.push(absolute_line);
                                if p.len() > 500 {
                                    let excess = p.len() - 500;
                                    p.drain(0..excess);
                                }
                            }
                        }
                        dispatch_osc_action(&OscCommand::PromptStarted, &event_listener_clone, cursor_line, base, screen_lines);
                    }
                    OscCommand::NotificationQuery { id } => {
                        let resp = format!("\x1b]99;i={}:p=OK;\x1b\\", id);
                        let mut w = writer_clone.lock();
                        let _ = w.write_all(resp.as_bytes());
                        let _ = w.flush();
                    }
                    OscCommand::ColorQuery { code } => {
                        let theme_name = crate::config::ACTIVE_THEME.read().clone();
                        let theme = crate::ui::theme::Theme::from_name(&theme_name);
                        let hsla = match code {
                            10 => theme.foreground,
                            11 => theme.background,
                            12 => theme.cursor,
                            _ => theme.foreground,
                        };
                        let rgba: gpui::Rgba = hsla.into();
                        let r = (rgba.r * 255.0).clamp(0.0, 255.0) as u8;
                        let g = (rgba.g * 255.0).clamp(0.0, 255.0) as u8;
                        let b = (rgba.b * 255.0).clamp(0.0, 255.0) as u8;
                        let resp = format!("\x1b]{};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\", code, r, r, g, g, b, b);
                        let mut w = writer_clone.lock();
                        let _ = w.write_all(resp.as_bytes());
                        let _ = w.flush();
                    }
                    OscCommand::PaletteQuery { index } => {
                        let theme_name = crate::config::ACTIVE_THEME.read().clone();
                        let theme = crate::ui::theme::Theme::from_name(&theme_name);
                        let hsla = match index {
                            0 => theme.black,
                            1 => theme.red,
                            2 => theme.green,
                            3 => theme.yellow,
                            4 => theme.blue,
                            5 => theme.magenta,
                            6 => theme.cyan,
                            7 => theme.white,
                            8 => theme.bright_black,
                            9 => theme.bright_red,
                            10 => theme.bright_green,
                            11 => theme.bright_yellow,
                            12 => theme.bright_blue,
                            13 => theme.bright_magenta,
                            14 => theme.bright_cyan,
                            15 => theme.bright_white,
                            _ => theme.foreground,
                        };
                        let rgba: gpui::Rgba = hsla.into();
                        let r = (rgba.r * 255.0).clamp(0.0, 255.0) as u8;
                        let g = (rgba.g * 255.0).clamp(0.0, 255.0) as u8;
                        let b = (rgba.b * 255.0).clamp(0.0, 255.0) as u8;
                        let resp = format!("\x1b]4;{};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\", index, r, r, g, g, b, b);
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
                                        "Fastty".to_string()
                                    } else {
                                        finished.title
                                    };
                                    dispatch_osc_action(
                                        &OscCommand::Notification { title, body: finished.body },
                                        &event_listener_clone,
                                        cursor_line,
                                        base,
                                        screen_lines,
                                    );
                                }
                            }
                        } else {
                            dispatch_osc_action(
                                &OscCommand::Notification { title: "Fastty".to_string(), body: payload },
                                &event_listener_clone,
                                cursor_line,
                                base,
                                screen_lines,
                            );
                        }
                    }
                    _ => {
                        dispatch_osc_action(&cmd, &event_listener_clone, cursor_line, base, screen_lines);
                    }
                }
            };

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        event_listener_clone.send_app_event(AppEvent::Exit { shell_pid: reader_shell_pid });
                        break;
                    }
                    Ok(n) => {
                        let mut term_locked = term_clone.lock();
                        let mut local_lines = 0;

                        // Debug: write raw PTY bytes to log file
                        if pty_debug {
                            use std::io::Write as _;
                            if let Ok(mut f) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&pty_log_path)
                            {
                                let ts = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis();
                                let escaped: String = buf[..n].iter().map(|&b| {
                                    match b {
                                        0x1b => "ESC".to_string(),
                                        0x07 => "BEL".to_string(),
                                        b if b.is_ascii_graphic() || b == b' ' => format!("{}", b as char),
                                        b => format!("\\x{:02x}", b),
                                    }
                                }).collect();
                                let _ = writeln!(f, "[{}ms] PTY({} bytes): {}", ts, n, escaped);
                            }
                        }

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
                                        if osc_buf.len() > 131072 {
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
                                        if osc_buf.len() > 131072 {
                                            osc_state = OscParseState::Normal;
                                        }
                                    }
                                }
                            }

                            // CSI detection: dispatch watcher events for
                            // scrollback wipes and Synchronized Output.
                            match csi_watcher.feed(byte) {
                                Some(CsiEvent::ClearHistory) => clear_history_flag = true,
                                Some(CsiEvent::SyncOutputBegin) => {
                                    sync_output_active = true;
                                    sync_started = Some(std::time::Instant::now());
                                }
                                Some(CsiEvent::SyncOutputEnd) => {
                                    sync_output_active = false;
                                    sync_started = None;
                                }
                                None => {}
                            }
                        }
                        // After the byte loop: clear scrollback history if we detected
                        // \x1b[2J (EraseDisplay::All) on the main screen.
                        if clear_history_flag {
                            use alacritty_terminal::term::TermMode;
                            let mode = *term_locked.mode();
                            if !mode.contains(TermMode::ALT_SCREEN) {
                                term_locked.grid_mut().clear_history();
                            }
                            clear_history_flag = false;
                        }
                        drop(term_locked);

                        total_lines_pushed_clone.fetch_add(local_lines, Ordering::Relaxed);

                        // Synchronized Output: if the app still holds mode 2026
                        // set and the safety deadline passed, release it so a
                        // crashed app can never freeze the screen.
                        if sync_output_active
                            && sync_started.is_some_and(|t| t.elapsed() >= SYNC_OUTPUT_TIMEOUT)
                        {
                            sync_output_active = false;
                            sync_started = None;
                        }
                        if !sync_output_active {
                            render_gen_clone.fetch_add(1, Ordering::Relaxed);
                            event_listener_clone.send_app_event(AppEvent::Wakeup);
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                        continue;
                    }
                    Err(_) => {
                        event_listener_clone.send_app_event(AppEvent::Exit { shell_pid: reader_shell_pid });
                        break;
                    }
                }
            }
        });
        }

        Ok(Self {
            term,
            render_generation,
            writer: writer_arc,
            master: master_arc,
            shell_pid,
            total_lines_pushed,
            prompt_lines,
            event_listener,
        })
    }

    pub fn shell_pid(&self) -> Option<u32> {
        self.shell_pid
    }

    pub fn set_event_sender(&self, sender: crate::event_listener::EventSender) {
        self.event_listener.set_event_sender(sender);
    }

    pub fn get_current_working_directory(&self) -> Option<std::path::PathBuf> {
        let pid = self.shell_pid?;
        get_process_cwd(pid)
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

    pub fn is_mouse_mode_enabled(&self) -> bool {
        let term = self.term.lock();
        let mode = *term.mode();
        mode.intersects(
            alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                | alacritty_terminal::term::TermMode::MOUSE_DRAG
                | alacritty_terminal::term::TermMode::MOUSE_MOTION,
        )
    }

    pub fn is_bracketed_paste_enabled(&self) -> bool {
        let term = self.term.lock();
        term.mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE)
    }

    pub fn send_mouse_event(&self, button: u8, col: usize, row: usize, pressed: bool) {
        self.send_mouse_button_with_mods(button, col, row, pressed, false, false, false);
    }

    pub fn send_mouse_button_with_mods(
        &self,
        button: u8,
        col: usize,
        row: usize,
        pressed: bool,
        shift: bool,
        alt: bool,
        control: bool,
    ) {
        let term = self.term.lock();
        let mode = *term.mode();
        let is_mouse_active = mode.intersects(
            alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                | alacritty_terminal::term::TermMode::MOUSE_DRAG
                | alacritty_terminal::term::TermMode::MOUSE_MOTION,
        );
        let sgr = mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE);
        drop(term);

        if !is_mouse_active {
            return;
        }

        let mut btn_code = button;
        if shift {
            btn_code |= 4;
        }
        if alt {
            btn_code |= 8;
        }
        if control {
            btn_code |= 16;
        }

        let col_1 = col.max(1);
        let row_1 = row.max(1);

        if sgr {
            // SGR 1006 format: \x1b[<{button};{col};{row}{M/m}
            let flag = if pressed { 'M' } else { 'm' };
            let seq = format!("\x1b[<{};{};{}{}", btn_code, col_1, row_1, flag);
            self.write_to_pty(seq.as_bytes());
        } else {
            // Normal X10/1000 format: \x1b[M{btn + 32}{col + 32}{row + 32}
            let b_byte = if pressed { btn_code } else { 3 };
            let cb = (b_byte.saturating_add(32)).min(255);
            let cx = ((col_1 as u8).saturating_add(32)).min(255);
            let cy = ((row_1 as u8).saturating_add(32)).min(255);
            let seq = [0x1b, b'[', b'M', cb, cx, cy];
            self.write_to_pty(&seq);
        }
    }

    pub fn send_mouse_motion(
        &self,
        col: usize,
        row: usize,
        left_down: bool,
        middle_down: bool,
        right_down: bool,
        shift: bool,
        alt: bool,
        control: bool,
    ) {
        let term = self.term.lock();
        let mode = *term.mode();
        let is_mouse_active = mode.intersects(
            alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                | alacritty_terminal::term::TermMode::MOUSE_DRAG
                | alacritty_terminal::term::TermMode::MOUSE_MOTION,
        );
        let sgr = mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE);
        let motion_any = mode.contains(alacritty_terminal::term::TermMode::MOUSE_MOTION);
        let motion_drag = mode.contains(alacritty_terminal::term::TermMode::MOUSE_DRAG);
        drop(term);

        if !is_mouse_active {
            return;
        }

        let is_dragging = left_down || middle_down || right_down;
        if is_dragging && !motion_drag && !motion_any {
            return;
        }
        if !is_dragging && !motion_any {
            return;
        }

        let mut btn_code: u8 = if left_down {
            0 + 32
        } else if middle_down {
            1 + 32
        } else if right_down {
            2 + 32
        } else {
            3 + 32 // 35 = motion without button
        };

        if shift {
            btn_code |= 4;
        }
        if alt {
            btn_code |= 8;
        }
        if control {
            btn_code |= 16;
        }

        let col_1 = col.max(1);
        let row_1 = row.max(1);

        if sgr {
            let seq = format!("\x1b[<{};{};{}M", btn_code, col_1, row_1);
            self.write_to_pty(seq.as_bytes());
        } else {
            let cb = (btn_code.saturating_add(32)).min(255);
            let cx = ((col_1 as u8).saturating_add(32)).min(255);
            let cy = ((row_1 as u8).saturating_add(32)).min(255);
            let seq = [0x1b, b'[', b'M', cb, cx, cy];
            self.write_to_pty(&seq);
        }
    }

    pub fn dimensions(&self) -> (usize, usize) {
        let term = self.term.lock();
        (term.grid().columns(), term.grid().screen_lines())
    }

    pub fn resize(&self, cols: usize, rows: usize) {
        self.resize_with_pixels(cols, rows, (cols as u16) * 8, (rows as u16) * 16);
    }

    pub fn resize_with_pixels(&self, cols: usize, rows: usize, pixel_width: u16, pixel_height: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        let mut term = self.term.lock();
        let cur_cols = term.grid().columns();
        let cur_rows = term.grid().screen_lines();
        if cur_cols == cols && cur_rows == rows {
            return;
        }
        let size = TermSize::new(cols, rows);
        term.resize(size);
        drop(term);

        self.master.lock().resize(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width,
            pixel_height,
        }).ok();

        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll(&self, delta: isize) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        term.scroll_display(Scroll::Delta(delta as i32));
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll_to_offset(&self, offset: usize) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        let cur_offset = term.grid().display_offset();
        let delta = (offset as isize) - (cur_offset as isize);
        if delta != 0 {
            term.scroll_display(Scroll::Delta(delta as i32));
            self.render_generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn scroll_to_bottom(&self) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        term.scroll_display(Scroll::Bottom);
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll_to_top(&self) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        term.scroll_display(Scroll::Top);
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll_page(&self, pages: i32) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        if pages > 0 {
            for _ in 0..pages {
                term.scroll_display(Scroll::PageDown);
            }
        } else {
            for _ in 0..(-pages) {
                term.scroll_display(Scroll::PageUp);
            }
        }
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn display_offset(&self) -> usize {
        let term = self.term.lock();
        term.grid().display_offset()
    }

    pub fn scroll_to_prev_prompt(&self) {
        let cur_offset = self.display_offset() as u64;
        let total_pushed = self.total_lines_pushed.load(Ordering::Relaxed);
        let prompts = self.prompt_lines.lock();
        let mut candidates: Vec<u64> = prompts
            .iter()
            .map(|&line| total_pushed.saturating_sub(line))
            .filter(|&offset| offset > cur_offset + 1)
            .collect();
        candidates.sort_unstable();
        if let Some(&target) = candidates.first() {
            self.scroll_to_offset(target as usize);
        } else if let Some(&min_line) = prompts.first() {
            let target = total_pushed.saturating_sub(min_line);
            if target > cur_offset {
                self.scroll_to_offset(target as usize);
            }
        }
    }

    pub fn scroll_to_next_prompt(&self) {
        let cur_offset = self.display_offset() as u64;
        if cur_offset == 0 {
            return;
        }
        let total_pushed = self.total_lines_pushed.load(Ordering::Relaxed);
        let prompts = self.prompt_lines.lock();
        let mut candidates: Vec<u64> = prompts
            .iter()
            .map(|&line| total_pushed.saturating_sub(line))
            .filter(|&offset| offset + 1 < cur_offset)
            .collect();
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        if let Some(&target) = candidates.first() {
            self.scroll_to_offset(target as usize);
        } else {
            self.scroll_to_bottom();
        }
    }

    pub fn history_size(&self) -> usize {
        use alacritty_terminal::grid::Dimensions;
        let term = self.term.lock();
        term.history_size()
    }



    pub fn search_matches(&self, query: &str) -> Vec<usize> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        use alacritty_terminal::index::Line;
        let query_lower = query.to_lowercase();
        let term = self.term.lock();
        let grid = term.grid();
        let history = grid.history_size();
        let screen_lines = grid.screen_lines();
        if screen_lines == 0 {
            return Vec::new();
        }
        let mut matches = Vec::new();

        for line_i in -(history as i32)..(screen_lines as i32) {
            let row = &grid[Line(line_i)];
            let mut line_str = String::new();
            for cell in row.into_iter() {
                let c = cell.c;
                if c != '\0' {
                    line_str.push(c);
                }
            }
            if line_str.to_lowercase().contains(&query_lower) {
                let offset = if line_i < 0 {
                    (-line_i) as usize
                } else {
                    0
                };
                matches.push(offset);
            }
        }
        matches.dedup();
        matches
    }

    pub fn render_generation(&self) -> u64 {
        self.render_generation.load(Ordering::Relaxed)
    }

    pub fn term(&self) -> &Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>> {
        &self.term
    }
}

/// Decode a `file://` URL or plain directory path to a local filesystem path.
/// Accepts `file:///abs/path`, `file://hostname/abs/path`, and raw `/path` or `C:\path` forms.
fn file_url_to_path(buf: &[u8]) -> Option<std::path::PathBuf> {
    let s = std::str::from_utf8(buf).ok()?;
    if let Some(path) = s.strip_prefix("file://") {
        let path = if let Some(slash_idx) = path.find('/') {
            &path[slash_idx..]
        } else {
            path
        };
        let decoded = percent_decode(path);
        Some(std::path::PathBuf::from(decoded))
    } else {
        let trimmed = s.trim();
        if !trimmed.is_empty() && (trimmed.starts_with('/') || trimmed.starts_with('~') || (trimmed.len() >= 2 && trimmed.chars().nth(1) == Some(':'))) {
            Some(std::path::PathBuf::from(trimmed))
        } else {
            None
        }
    }
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
#[allow(dead_code)]
enum OscCommand {
    Cwd(String),
    SetTitle(String),
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
    ColorQuery { code: u8 },
    PaletteQuery { index: u8 },
    ResetPalette,
    ResetColor { code: u8 },
}

fn parse_osc(
    buf: &[u8],
    cmd_start_time: &mut Option<std::time::Instant>,
) -> Option<OscCommand> {
    let semicolon = buf.iter().position(|&b| b == b';')?;
    let code = &buf[..semicolon];
    let payload = &buf[semicolon + 1..];

    match code {
        b"0" | b"1" | b"2" => {
            if let Ok(title) = std::str::from_utf8(payload) {
                Some(OscCommand::SetTitle(title.to_string()))
            } else {
                None
            }
        }
        b"4" => {
            let s = std::str::from_utf8(payload).ok()?;
            let parts: Vec<&str> = s.split(';').collect();
            if parts.len() >= 2 {
                if let Ok(index) = parts[0].parse::<u8>() {
                    if parts[1] == "?" {
                        return Some(OscCommand::PaletteQuery { index });
                    }
                } else if parts[0] == "?" {
                    if let Ok(index) = parts[1].parse::<u8>() {
                        return Some(OscCommand::PaletteQuery { index });
                    }
                }
            }
            None
        }
        b"6" | b"7" | b"176" => {
            if let Ok(s) = std::str::from_utf8(payload) {
                Some(OscCommand::Cwd(s.to_string()))
            } else {
                None
            }
        }
        b"9" => {
            if let Ok(msg) = std::str::from_utf8(payload) {
                Some(OscCommand::Notification {
                    title: "Fastty".to_string(),
                    body: msg.to_string(),
                })
            } else {
                None
            }
        }
        b"10" | b"11" | b"12" => {
            let code_num = match code {
                b"10" => 10,
                b"11" => 11,
                b"12" => 12,
                _ => 10,
            };
            if payload.starts_with(b"?") || payload == b"?" {
                Some(OscCommand::ColorQuery { code: code_num })
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
        b"104" => {
            Some(OscCommand::ResetPalette)
        }
        b"110" | b"111" | b"112" => {
            let code_num = match code {
                b"110" => 10,
                b"111" => 11,
                b"112" => 12,
                _ => 10,
            };
            Some(OscCommand::ResetColor { code: code_num })
        }
        b"777" => {
            if payload.starts_with(b"notify;") {
                let parts: Vec<&[u8]> = payload[7..].split(|&b| b == b';').collect();
                if parts.len() >= 2 {
                    let title = std::str::from_utf8(parts[0]).unwrap_or("Fastty").to_string();
                    let body = parts[1..]
                        .iter()
                        .map(|p| std::str::from_utf8(p).unwrap_or(""))
                        .collect::<Vec<&str>>()
                        .join(";");
                    Some(OscCommand::Notification { title, body })
                } else if parts.len() == 1 {
                    let body = std::str::from_utf8(parts[0]).unwrap_or("").to_string();
                    Some(OscCommand::Notification {
                        title: "Fastty".to_string(),
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
                b"C" => {
                    if cmd_start_time.is_none() {
                        *cmd_start_time = Some(std::time::Instant::now());
                    }
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
        b"633" => {
            match payload {
                b"A" => {
                    Some(OscCommand::PromptStarted)
                }
                b"B" => {
                    *cmd_start_time = Some(std::time::Instant::now());
                    Some(OscCommand::CommandStarted)
                }
                b"C" => {
                    if cmd_start_time.is_none() {
                        *cmd_start_time = Some(std::time::Instant::now());
                    }
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
                    } else if payload.starts_with(b"P;Cwd=") {
                        let cwd_str = std::str::from_utf8(&payload[6..]).ok()?;
                        Some(OscCommand::Cwd(cwd_str.to_string()))
                    } else {
                        None
                    }
                }
            }
        }
        b"1337" => {
            if let Ok(s) = std::str::from_utf8(payload) {
                if let Some(dir) = s.strip_prefix("CurrentDir=") {
                    Some(OscCommand::Cwd(dir.to_string()))
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn dispatch_osc_action(
    cmd: &OscCommand,
    sender: &crate::event_listener::EventListenerProxy,
    cursor_line: i32,
    absolute_base: u64,
    screen_lines: i32,
) {
    match cmd {
        OscCommand::Cwd(path) => {
            if let Some(p) = file_url_to_path(path.as_bytes()) {
                sender.send_app_event(AppEvent::CwdChanged(p));
            }
        }
        OscCommand::SetTitle(title) => {
            sender.send_app_event(AppEvent::TitleChanged(title.clone()));
        }
        OscCommand::CommandStarted => {
            sender.send_app_event(AppEvent::CommandStarted);
        }
        OscCommand::CommandFinished { duration_ms, exit_code } => {
            sender.send_app_event(AppEvent::CommandFinished {
                duration_ms: *duration_ms,
                exit_code: *exit_code,
            });
        }
        OscCommand::PromptStarted => {
            let scrolled = (absolute_base as i32 - screen_lines).max(0);
            let absolute_line = scrolled + cursor_line;
            sender.send_app_event(AppEvent::PromptStarted {
                absolute_line: absolute_line.max(0) as u64,
            });
        }
        OscCommand::Notification { title, body } => {
            sender.send_app_event(AppEvent::Notification {
                title: title.clone(),
                body: body.clone(),
            });
        }
        OscCommand::NotificationQuery { .. }
        | OscCommand::NotificationFragment { .. }
        | OscCommand::ColorQuery { .. }
        | OscCommand::PaletteQuery { .. }
        | OscCommand::ResetPalette
        | OscCommand::ResetColor { .. } => {}
    }
}

#[cfg(target_os = "macos")]
fn get_process_cwd(pid: u32) -> Option<std::path::PathBuf> {
    use std::ffi::CStr;
    use std::os::raw::{c_int, c_void};

    #[repr(C)]
    struct VnodeInfoPath {
        _vip_vi: [u8; 152],
        vip_path: [libc::c_char; 1024],
    }

    #[repr(C)]
    struct ProcVnodePathInfo {
        pvi_cdir: VnodeInfoPath,
        pvi_rdir: VnodeInfoPath,
    }

    extern "C" {
        fn proc_pidinfo(
            pid: c_int,
            flavor: c_int,
            arg: u64,
            buffer: *mut c_void,
            buffersize: c_int,
        ) -> c_int;
    }

    const PROC_PIDVNODEPATHINFO: c_int = 9;

    let mut path_info = std::mem::MaybeUninit::<ProcVnodePathInfo>::uninit();
    let size = std::mem::size_of::<ProcVnodePathInfo>() as c_int;
    let res = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDVNODEPATHINFO,
            0,
            path_info.as_mut_ptr() as *mut c_void,
            size,
        )
    };

    if res == size {
        let path_info = unsafe { path_info.assume_init() };
        let c_str = unsafe { CStr::from_ptr(path_info.pvi_cdir.vip_path.as_ptr()) };
        if let Ok(path_str) = c_str.to_str() {
            if !path_str.is_empty() {
                return Some(std::path::PathBuf::from(path_str));
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn get_process_cwd(pid: u32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{}/cwd", pid)).ok()
}

#[cfg(target_os = "windows")]
fn get_process_cwd(_pid: u32) -> Option<std::path::PathBuf> {
    None
}
#[cfg(test)]
mod tests {
    use super::CsiEvent::{self, *};
    use super::CsiWatcher;

    fn run(bytes: &[u8]) -> Vec<CsiEvent> {
        let mut w = CsiWatcher::new();
        bytes.iter().filter_map(|&b| w.feed(b)).collect()
    }

    #[test]
    fn detects_synchronized_output_set_and_reset() {
        assert_eq!(run(b"\x1b[?2026h"), vec![SyncOutputBegin]);
        assert_eq!(run(b"\x1b[?2026l"), vec![SyncOutputEnd]);
    }

    #[test]
    fn detects_2026_combined_with_other_private_modes() {
        assert_eq!(run(b"\x1b[?1049;2026h"), vec![SyncOutputBegin]);
        assert_eq!(run(b"\x1b[?2026;1049l"), vec![SyncOutputEnd]);
    }

    #[test]
    fn detects_2026_across_split_chunks() {
        let mut w = CsiWatcher::new();
        assert_eq!(w.feed(0x1b), None);
        assert_eq!(w.feed(b'['), None);
        assert_eq!(w.feed(b'?'), None);
        assert_eq!(w.feed(b'2'), None);
        assert_eq!(w.feed(b'0'), None);
        assert_eq!(w.feed(b'2'), None);
        assert_eq!(w.feed(b'6'), None);
        assert_eq!(w.feed(b'h'), Some(SyncOutputBegin));
    }

    #[test]
    fn plain_2j_clears_history_but_private_2j_does_not() {
        assert_eq!(run(b"\x1b[2J"), vec![ClearHistory]);
        assert_eq!(run(b"\x1b[?2J"), Vec::<CsiEvent>::new());
        assert_eq!(run(b"\x1b[3J"), Vec::<CsiEvent>::new());
    }

    #[test]
    fn other_private_modes_are_ignored() {
        assert_eq!(
            run(b"\x1b[?1049h\x1b[?25l\x1b[?1000;1006h"),
            Vec::<CsiEvent>::new()
        );
    }

    #[test]
    fn plain_text_emits_nothing() {
        assert_eq!(
            run(b"hello \x1b]0;title\x07 world"),
            Vec::<CsiEvent>::new()
        );
    }

    #[test]
    fn test_osc_parsing() {
        use super::parse_osc;
        let mut cmd_start = None;

        // OSC 0/1/2 - Title
        let cmd = parse_osc(b"0;My Terminal", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::SetTitle(t) if t == "My Terminal"));

        // OSC 6, 7, 176 - CWD
        let cmd = parse_osc(b"7;file:///Users/foo/project", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "file:///Users/foo/project"));

        let cmd = parse_osc(b"6;file:///tmp", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "file:///tmp"));

        let cmd = parse_osc(b"176;file:///var", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "file:///var"));

        // OSC 1337 CurrentDir
        let cmd = parse_osc(b"1337;CurrentDir=/home/user", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "/home/user"));

        // OSC 133 - Shell Integration
        let cmd = parse_osc(b"133;A", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::PromptStarted));

        let cmd = parse_osc(b"133;B", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::CommandStarted));
        assert!(cmd_start.is_some());

        let cmd = parse_osc(b"133;D;0", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::CommandFinished { exit_code: Some(0), .. }));

        // OSC 633 - VS Code Shell Integration
        let cmd = parse_osc(b"633;A", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::PromptStarted));

        let cmd = parse_osc(b"633;P;Cwd=/opt/app", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "/opt/app"));

        // OSC 10/11/12 - Color Query
        let cmd = parse_osc(b"10;?", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::ColorQuery { code: 10 }));

        let cmd = parse_osc(b"11;?", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::ColorQuery { code: 11 }));

        // OSC 4 - Palette Query
        let cmd = parse_osc(b"4;2;?", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::PaletteQuery { index: 2 }));
    }
}
