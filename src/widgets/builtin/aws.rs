//! AWS profile widget. Shells out to `aws sts get-caller-identity`.

use std::time::{Duration, Instant};

use crate::widgets::{Align, ClickAction, Segment, Widget, WidgetContext};

const DEFAULT_INTERVAL_MS: u64 = 300_000;

pub struct AwsWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    cached: AwsState,
}

#[derive(Debug, Clone, Default)]
enum AwsState {
    #[default]
    Unknown,
    Ok { profile: Option<String>, identity: String },
    NoAws,
    Error(String),
}

impl AwsWidget {
    pub fn new(align: Align, interval_ms: Option<u64>) -> Self {
        Self {
            align,
            last_poll: Instant::now() - Duration::from_secs(60),
            interval: Duration::from_millis(interval_ms.unwrap_or(DEFAULT_INTERVAL_MS)),
            cached: AwsState::Unknown,
        }
    }
}

impl Widget for AwsWidget {
    fn id(&self) -> &'static str { "aws" }
    fn align(&self) -> Align { self.align }
    fn poll_interval(&self) -> Duration { self.interval }
    fn last_poll(&self) -> Instant { self.last_poll }
    fn set_last_poll(&mut self, t: Instant) { self.last_poll = t; }

    fn poll(&mut self, _ctx: &WidgetContext) {
        let mut cmd = std::process::Command::new("aws");
        cmd.args(["sts", "get-caller-identity", "--query", "Arn", "--output", "text"]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let profile = std::env::var("AWS_PROFILE").ok().filter(|s| !s.is_empty());
        self.cached = match cmd.output() {
            Ok(out) if out.status.success() => {
                let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if id.is_empty() {
                    AwsState::Unknown
                } else {
                    AwsState::Ok { profile, identity: id }
                }
            }
            Ok(_) => AwsState::Unknown,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => AwsState::NoAws,
            Err(e) => AwsState::Error(e.to_string()),
        };
    }

    fn render(&mut self, _ctx: &WidgetContext) -> Vec<Segment> {
        match &self.cached {
            AwsState::Ok { profile, identity } => {
                let label = match profile {
                    Some(p) => format!("aws:{} ", p),
                    None => "aws:default ".to_string(),
                };
                vec![Segment {
                    text: format!(" {} ", label),
                    color: [0.95, 0.65, 0.30, 1.0],
                    tooltip: Some(identity.clone()),
                }]
            }
            AwsState::NoAws | AwsState::Unknown => Vec::new(),
            AwsState::Error(e) => vec![Segment {
                text: " aws:err ".to_string(),
                color: [0.90, 0.55, 0.45, 1.0],
                tooltip: Some(format!("aws error: {}", e)),
            }],
        }
    }

    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        ClickAction::Custom
    }
}
