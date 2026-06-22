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

        // Branch icon: ⎇ for normal branch, ⟳ for detached HEAD
        let (icon, icon_tooltip) = if gs.is_detached {
            ("\u{27F3}", format!("Detached HEAD at {}", gs.last_commit_hash))
        } else {
            ("\u{2387}", format!("Branch: {}", gs.branch))
        };
        segs.push(Segment {
            text: format!(" {} {}", icon, gs.branch),
            color: [0.85, 0.88, 0.95, 1.0],
            tooltip: Some(icon_tooltip),
        });

        // Staged: yellow dot + count (index changes)
        if gs.staged > 0 {
            segs.push(Segment {
                text: format!(" \u{25CF} +{}", gs.staged),
                color: [0.95, 0.80, 0.45, 1.0],
                tooltip: Some(format!("{} staged file(s)", gs.staged)),
            });
        }
        // Unstaged: green + count (working tree modifications)
        if gs.unstaged > 0 {
            segs.push(Segment {
                text: format!(" +{}", gs.unstaged),
                color: [0.45, 0.85, 0.55, 1.0],
                tooltip: Some(format!("{} modified file(s) in working tree", gs.unstaged)),
            });
        }
        // Untracked: dim ? + count
        if gs.untracked > 0 {
            segs.push(Segment {
                text: format!(" ?{}", gs.untracked),
                color: [0.55, 0.55, 0.65, 1.0],
                tooltip: Some(format!("{} untracked file(s)", gs.untracked)),
            });
        }
        // Ahead / behind
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
        segs
    }

    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        if self.cached.is_some() && self.pending_cwd.is_some() {
            ClickAction::ShowActionsMenu
        } else {
            ClickAction::None
        }
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

    fn get_context_menu_items(&self) -> Option<Vec<crate::renderer::ContextMenuItem>> {
        let cwd_str = self.pending_cwd.as_ref()?.to_string_lossy().into_owned();
        let mut items = Vec::new();

        if let Some(gs) = &self.cached {
            let (label, status) = match &gs.sync_status {
                crate::git::SyncStatus::UpToDate => ("\u{2713} Up to date with remote".to_string(), "success".to_string()),
                crate::git::SyncStatus::Behind(n) => (format!("\u{2193} Behind remote by {} commit(s)", n), "queued".to_string()),
                crate::git::SyncStatus::Ahead(n) => (format!("\u{2191} Ahead of remote by {} commit(s)", n), "queued".to_string()),
                crate::git::SyncStatus::Diverged => ("\u{26A1} Diverged from remote".to_string(), "failure".to_string()),
                crate::git::SyncStatus::Unknown => ("\u{2014} No remote tracked".to_string(), "skipped".to_string()),
            };
            items.push(crate::renderer::ContextMenuItem::GithubActionInfo {
                label,
                status,
                url: None,
            });
            items.push(crate::renderer::ContextMenuItem::Separator);
        }

        items.push(crate::renderer::ContextMenuItem::CommandItem {
            label: "\u{2B07} Pull  [git pull]".to_string(),
            command: "git pull".to_string(),
            cwd: cwd_str.clone(),
        });
        items.push(crate::renderer::ContextMenuItem::CommandItem {
            label: "\u{2B06} Push  [git push]".to_string(),
            command: "git push".to_string(),
            cwd: cwd_str.clone(),
        });
        items.push(crate::renderer::ContextMenuItem::CommandItem {
            label: "\u{21BA} Fetch [git fetch]".to_string(),
            command: "git fetch".to_string(),
            cwd: cwd_str,
        });

        Some(items)
    }
}
