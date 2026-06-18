//! Generic command widget. Runs a user-configured shell command, shows stdout
//! in the bar. Click action configurable: `copy`, `run`, `open`.

use std::time::{Duration, Instant};

use crate::widgets::{Align, ClickAction, Segment, Widget, WidgetContext};

const DEFAULT_INTERVAL_MS: u64 = 5_000;
const MAX_OUTPUT_BYTES: usize = 4096;

pub struct CommandWidget {
    name: String,
    command: String,
    on_click_action: Option<String>,
    align: Align,
    last_poll: Instant,
    interval: Duration,
    cached: CommandState,
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
            cached: CommandState::Unknown,
        }
    }
}

impl Widget for CommandWidget {
    fn id(&self) -> &'static str { "command" }
    fn align(&self) -> Align { self.align }
    fn poll_interval(&self) -> Duration { self.interval }
    fn last_poll(&self) -> Instant { self.last_poll }
    fn set_last_poll(&mut self, t: Instant) { self.last_poll = t; }

    fn poll(&mut self, _ctx: &WidgetContext) {
        let mut cmd = shell_command(&self.command);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        self.cached = match cmd.output() {
            Ok(out) if out.status.success() => {
                let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
                if s.ends_with('\n') { s.pop(); }
                if s.len() > MAX_OUTPUT_BYTES {
                    s.truncate(MAX_OUTPUT_BYTES - 1);
                    s.push('\u{2026}');
                }
                CommandState::Ok(s)
            }
            Ok(_) => CommandState::Unknown,
            Err(e) => CommandState::Error(e.to_string()),
        };
    }

    fn render(&mut self, _ctx: &WidgetContext) -> Vec<Segment> {
        match &self.cached {
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
        let payload = match &self.cached {
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
    let mut c = std::process::Command::new("cmd");
    c.arg("/C").arg(cmd);
    c
}
