//! Git status widget.
//!
//! Reads the active tab's cached `GitStatus`. Renders branch + dirty dot +
//! ahead/behind/modified/staged/untracked counts.

use std::time::{Duration, Instant};

use crate::widgets::{Align, ClickAction, Segment, Widget, WidgetContext};

const DEFAULT_INTERVAL_MS: u64 = 1500;

pub struct GitWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    cached: Option<crate::git::GitStatus>,
    pending_cwd: Option<std::path::PathBuf>,
}

impl GitWidget {
    pub fn new(align: Align, interval_ms: Option<u64>) -> Self {
        Self {
            align,
            last_poll: Instant::now() - Duration::from_secs(60),
            interval: Duration::from_millis(interval_ms.unwrap_or(DEFAULT_INTERVAL_MS)),
            cached: None,
            pending_cwd: None,
        }
    }
}

impl Widget for GitWidget {
    fn id(&self) -> &'static str { "git" }
    fn align(&self) -> Align { self.align }
    fn poll_interval(&self) -> Duration { self.interval }
    fn last_poll(&self) -> Instant { self.last_poll }
    fn set_last_poll(&mut self, t: Instant) { self.last_poll = t; }

    fn poll(&mut self, ctx: &WidgetContext) {
        self.cached = ctx.active_tab_git.cloned();
        if let Some(p) = ctx.active_tab_cwd {
            self.pending_cwd = Some(p.to_path_buf());
        }
    }

    fn render(&mut self, _ctx: &WidgetContext) -> Vec<Segment> {
        let Some(gs) = &self.cached else { return Vec::new() };
        let mut segs = Vec::with_capacity(8);

        segs.push(Segment {
            text: gs.branch.clone(),
            color: [0.85, 0.88, 0.95, 1.0],
            tooltip: None,
        });

        if !gs.is_clean() {
            segs.push(Segment {
                text: " ●".to_string(),
                color: [0.95, 0.80, 0.45, 1.0],
                tooltip: Some("working tree has uncommitted changes".to_string()),
            });
        }

        if gs.ahead > 0 {
            segs.push(Segment {
                text: format!(" \u{2191}{}", gs.ahead),
                color: [0.45, 0.85, 0.55, 1.0],
                tooltip: Some(format!("{} commit(s) ahead of upstream", gs.ahead)),
            });
        }
        if gs.behind > 0 {
            segs.push(Segment {
                text: format!(" \u{2193}{}", gs.behind),
                color: [0.90, 0.55, 0.40, 1.0],
                tooltip: Some(format!("{} commit(s) behind upstream", gs.behind)),
            });
        }
        if gs.modified > 0 {
            segs.push(Segment {
                text: format!(" ~{}", gs.modified),
                color: [0.95, 0.80, 0.45, 1.0],
                tooltip: None,
            });
        }
        if gs.staged > 0 {
            segs.push(Segment {
                text: format!(" +{}", gs.staged),
                color: [0.50, 0.80, 0.95, 1.0],
                tooltip: None,
            });
        }
        if gs.untracked > 0 {
            segs.push(Segment {
                text: format!(" ?{}", gs.untracked),
                color: [0.65, 0.65, 0.75, 1.0],
                tooltip: None,
            });
        }
        segs
    }

    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        ClickAction::Custom
    }

    fn tooltip(&self) -> Option<String> {
        self.cached.as_ref().and_then(|gs| {
            if gs.last_commit_summary.is_empty() {
                None
            } else {
                Some(gs.last_commit_summary.clone())
            }
        })
    }
}
