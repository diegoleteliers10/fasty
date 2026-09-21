//! Generic command widget. Runs a user-configured shell command, shows stdout
//! in the bar. Click action configurable: `copy`, `run`, `open`.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::widgets::{Align, ClickAction, Segment, Widget, WidgetContext};
use crate::widgets::proc_util::{FailureBackoff, USER_CMD_TIMEOUT, run_with_timeout};

const DEFAULT_INTERVAL_MS: u64 = 5_000;
const MAX_OUTPUT_BYTES: usize = 4096;

pub struct CommandWidget {
    name: String,
    command: String,
    on_click_action: Option<String>,
    align: Align,
    last_poll: Instant,
    interval: Duration,
    state: Arc<Mutex<CommandState>>,
    is_running: Arc<AtomicBool>,
    backoff: Arc<Mutex<FailureBackoff>>,
}

#[derive(Debug, Clone, Default)]
enum CommandState {
    #[default]
    Unknown,
    Ok(String),
    Error(String),
}

impl CommandWidget {
    pub fn new(
        name: String,
        command: String,
        on_click: Option<String>,
        align: Align,
        interval_ms: Option<u64>,
    ) -> Self {
        Self {
            name,
            command,
            on_click_action: on_click,
            align,
            last_poll: Instant::now() - Duration::from_secs(60),
            interval: Duration::from_millis(interval_ms.unwrap_or(DEFAULT_INTERVAL_MS)),
            state: Arc::new(Mutex::new(CommandState::Unknown)),
            is_running: Arc::new(AtomicBool::new(false)),
            backoff: Arc::new(Mutex::new(FailureBackoff::default())),
        }
    }
}

impl Widget for CommandWidget {
    fn id(&self) -> &'static str { "command" }
    fn align(&self) -> Align { self.align }
    fn poll_interval(&self) -> Duration {
        self.backoff
            .lock()
            .map(|b| b.effective_interval(self.interval))
            .unwrap_or(self.interval)
    }
    fn last_poll(&self) -> Instant { self.last_poll }
    fn set_last_poll(&mut self, t: Instant) { self.last_poll = t; }

    fn poll(&mut self, ctx: &WidgetContext) {
        if !ctx.window_focused {
            return;
        }
        // `poll()` runs on the render path: never block it, dispatch and go.
        // The gate also prevents piling up slow user commands.
        if self.is_running.swap(true, Ordering::Relaxed) {
            return;
        }
        let command = self.command.clone();
        let state_clone = self.state.clone();
        let is_running_clone = self.is_running.clone();
        let backoff_clone = self.backoff.clone();
        std::thread::spawn(move || {
            let mut cmd = shell_command(&command);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            let next = match run_with_timeout(&mut cmd, USER_CMD_TIMEOUT) {
                Some(out) if out.status.success() => {
                    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
                    if s.ends_with('\n') { s.pop(); }
                    if s.len() > MAX_OUTPUT_BYTES {
                        s.truncate(MAX_OUTPUT_BYTES - 1);
                        s.push('\u{2026}');
                    }
                    CommandState::Ok(s)
                }
                Some(_) => CommandState::Unknown,
                None => CommandState::Error(format!(
                    "command timed out after {}s",
                    USER_CMD_TIMEOUT.as_secs()
                )),
            };
            let ok = !matches!(next, CommandState::Error(_));
            if let Ok(mut b) = backoff_clone.lock() {
                b.record(ok);
            }
            if let Ok(mut guard) = state_clone.lock() {
                *guard = next;
            }
            is_running_clone.store(false, Ordering::Relaxed);
        });
    }

    fn render(&mut self, _ctx: &WidgetContext) -> Vec<Segment> {
        let guard = self.state.lock().unwrap();
        match &*guard {
            CommandState::Ok(s) => vec![Segment {
                text: format!(" {}: {} ", self.name, s),
                color: [0.80, 0.80, 0.85, 1.0],
                tooltip: Some(self.command.clone()),
            }],
            CommandState::Error(e) => vec![Segment {
                text: format!(" {}:err ", self.name),
                color: [0.90, 0.55, 0.45, 1.0],
                tooltip: Some(e.clone()),
            }],
            CommandState::Unknown => Vec::new(),
        }
    }

    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        let payload = match &*self.state.lock().unwrap() {
            CommandState::Ok(s) => s.clone(),
            _ => return ClickAction::None,
        };
        match self.on_click_action.as_deref() {
            Some("copy") => ClickAction::CopyToClipboard(payload),
            Some("run") => ClickAction::RunCommand(payload),
            Some("open") => ClickAction::OpenUrl(payload),
            _ => ClickAction::None,
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn shell_command(cmd: &str) -> std::process::Command {
    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(cmd);
    c
}

#[cfg(target_os = "windows")]
fn shell_command(cmd: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new("cmd");
    c.arg("/C").arg(cmd);
    c.creation_flags(0x08000000);
    c
}
