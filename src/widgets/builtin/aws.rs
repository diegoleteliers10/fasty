//! AWS profile widget. Shells out to `aws sts get-caller-identity`.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::widgets::{Align, ClickAction, Segment, Widget, WidgetContext};
use crate::widgets::proc_util::{
    FailureBackoff, NETWORK_PROC_TIMEOUT, binary_on_path, run_with_timeout,
};

const DEFAULT_INTERVAL_MS: u64 = 300_000;

pub struct AwsWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    state: Arc<Mutex<AwsState>>,
    is_running: Arc<AtomicBool>,
    backoff: Arc<Mutex<FailureBackoff>>,
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
            state: Arc::new(Mutex::new(AwsState::Unknown)),
            is_running: Arc::new(AtomicBool::new(false)),
            backoff: Arc::new(Mutex::new(FailureBackoff::default())),
        }
    }
}

impl Widget for AwsWidget {
    fn id(&self) -> &'static str { "aws" }
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
        if self.is_running.swap(true, Ordering::Relaxed) {
            return;
        }
        let profile = std::env::var("AWS_PROFILE").ok().filter(|s| !s.is_empty());
        let state_clone = self.state.clone();
        let is_running_clone = self.is_running.clone();
        let backoff_clone = self.backoff.clone();
        std::thread::spawn(move || {
            let mut cmd = std::process::Command::new("aws");
            cmd.args(["sts", "get-caller-identity", "--query", "Arn", "--output", "text"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            let next = match run_with_timeout(&mut cmd, NETWORK_PROC_TIMEOUT) {
                Some(out) if out.status.success() => {
                    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if id.is_empty() {
                        AwsState::Unknown
                    } else {
                        AwsState::Ok { profile, identity: id }
                    }
                }
                Some(_) => AwsState::Unknown,
                // Missing binary stays silent (as before); a present-but-hung
                // binary reports and backs off.
                None if !binary_on_path("aws") => AwsState::NoAws,
                None => AwsState::Error("aws sts timed out".to_string()),
            };
            let ok = !matches!(next, AwsState::Error(_));
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
            AwsState::Unknown => Vec::new(),
            // UX2: subtle one-glance hint instead of silence; the tooltip
            // carries the explanation.
            AwsState::NoAws => vec![Segment {
                text: " aws: – ".to_string(),
                color: [0.55, 0.55, 0.62, 1.0],
                tooltip: Some(
                    "AWS CLI not found on PATH — install it to show caller identity here."
                        .to_string(),
                ),
            }],
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
