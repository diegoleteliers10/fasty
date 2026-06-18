//! Current kubectl context widget. Shells out to `kubectl config current-context`.

use std::time::{Duration, Instant};

use crate::widgets::{Align, ClickAction, Segment, Widget, WidgetContext};

const DEFAULT_INTERVAL_MS: u64 = 30_000;

pub struct KubeWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    cached: KubeState,
}

#[derive(Debug, Clone, Default)]
enum KubeState {
    #[default]
    Unknown,
    Ok(String),
    NoKubectl,
    Error(String),
}

impl KubeWidget {
    pub fn new(align: Align, interval_ms: Option<u64>) -> Self {
        Self {
            align,
            last_poll: Instant::now() - Duration::from_secs(60),
            interval: Duration::from_millis(interval_ms.unwrap_or(DEFAULT_INTERVAL_MS)),
            cached: KubeState::Unknown,
        }
    }
}

impl Widget for KubeWidget {
    fn id(&self) -> &'static str { "kube" }
    fn align(&self) -> Align { self.align }
    fn poll_interval(&self) -> Duration { self.interval }
    fn last_poll(&self) -> Instant { self.last_poll }
    fn set_last_poll(&mut self, t: Instant) { self.last_poll = t; }

    fn poll(&mut self, _ctx: &WidgetContext) {
        let mut cmd = std::process::Command::new("kubectl");
        cmd.args(["config", "current-context"]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        self.cached = match cmd.output() {
            Ok(out) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if s.is_empty() { KubeState::Unknown } else { KubeState::Ok(s) }
            }
            Ok(_) => KubeState::Unknown,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => KubeState::NoKubectl,
            Err(e) => KubeState::Error(e.to_string()),
        };
    }

    fn render(&mut self, _ctx: &WidgetContext) -> Vec<Segment> {
        match &self.cached {
            KubeState::Ok(ctx) => vec![Segment {
                text: format!(" k8s:{} ", ctx),
                color: [0.40, 0.75, 0.95, 1.0],
                tooltip: Some(format!("kubectl context: {}", ctx)),
            }],
            KubeState::NoKubectl => Vec::new(),
            KubeState::Error(e) => vec![Segment {
                text: format!(" k8s:err "),
                color: [0.90, 0.55, 0.45, 1.0],
                tooltip: Some(format!("kubectl error: {}", e)),
            }],
            KubeState::Unknown => Vec::new(),
        }
    }

    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        ClickAction::Custom
    }
}
