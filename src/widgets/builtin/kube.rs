//! Current kubectl context widget. Shells out to `kubectl config current-context`.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::widgets::{Align, ClickAction, Segment, Widget, WidgetContext};
use crate::widgets::proc_util::{FailureBackoff, LOCAL_PROC_TIMEOUT, binary_on_path, run_with_timeout};

const DEFAULT_INTERVAL_MS: u64 = 60_000; // 60 seconds: context switches are rare

pub struct KubeWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    state: Arc<Mutex<KubeState>>,
    is_running: Arc<AtomicBool>,
    backoff: Arc<Mutex<FailureBackoff>>,
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
            state: Arc::new(Mutex::new(KubeState::Unknown)),
            is_running: Arc::new(AtomicBool::new(false)),
            backoff: Arc::new(Mutex::new(FailureBackoff::default())),
        }
    }
}

impl Widget for KubeWidget {
    fn id(&self) -> &'static str { "kube" }
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
        let state_clone = self.state.clone();
        let is_running_clone = self.is_running.clone();
        let backoff_clone = self.backoff.clone();
        std::thread::spawn(move || {
            let mut cmd = std::process::Command::new("kubectl");
            cmd.args(["config", "current-context"]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            let next = match run_with_timeout(&mut cmd, LOCAL_PROC_TIMEOUT) {
                Some(out) if out.status.success() => {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if s.is_empty() { KubeState::Unknown } else { KubeState::Ok(s) }
                }
                Some(_) => KubeState::Unknown,
                // Missing binary stays silent (as before); a present-but-hung
                // binary reports and backs off.
                None if !binary_on_path("kubectl") => KubeState::NoKubectl,
                None => KubeState::Error("kubectl timed out".to_string()),
            };
            let ok = !matches!(next, KubeState::Error(_));
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
            KubeState::Ok(ctx) => vec![Segment {
                text: format!(" k8s:{} ", ctx),
                color: [0.40, 0.75, 0.95, 1.0],
                tooltip: Some(format!("kubectl context: {}", ctx)),
            }],
            KubeState::NoKubectl => vec![Segment {
                // UX2: subtle one-glance hint instead of silence; the tooltip
                // carries the explanation.
                text: " k8s: – ".to_string(),
                color: [0.55, 0.55, 0.62, 1.0],
                tooltip: Some(
                    "kubectl not found on PATH — install it to show the current context here."
                        .to_string(),
                ),
            }],
            KubeState::Error(e) => vec![Segment {
                text: " k8s:err ".to_string(),
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
