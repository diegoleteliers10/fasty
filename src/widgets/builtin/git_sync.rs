use icons::common::IconType;

use crate::widgets::proc_util::{run_with_timeout, FailureBackoff, LOCAL_PROC_TIMEOUT};
use crate::widgets::{
    Align, ClickAction, ContextMenuItem, Segment, SegmentPart, Widget, WidgetContext,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_INTERVAL_MS: u64 = 30_000; // 30 seconds

pub struct GitSyncWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    state: Arc<Mutex<Option<SyncSummary>>>,
    is_fetching: Arc<AtomicBool>,
    pending_cwd: Option<std::path::PathBuf>,
    /// Bumped on every cwd change; in-flight threads with a stale generation
    /// drop their result instead of overwriting fresh state.
    generation: Arc<AtomicU64>,
    backoff: Arc<Mutex<FailureBackoff>>,
}

#[derive(Debug, Clone)]
struct SyncSummary {
    ahead: usize,
    behind: usize,
    branch: String,
    has_upstream: bool,
    ahead_commits: Vec<String>,
    behind_commits: Vec<String>,
    cwd: std::path::PathBuf,
}

impl GitSyncWidget {
    pub fn new(align: Align, interval_ms: Option<u64>) -> Self {
        Self {
            align,
            last_poll: Instant::now() - Duration::from_secs(60),
            interval: Duration::from_millis(interval_ms.unwrap_or(DEFAULT_INTERVAL_MS)),
            state: Arc::new(Mutex::new(None)),
            is_fetching: Arc::new(AtomicBool::new(false)),
            pending_cwd: None,
            generation: Arc::new(AtomicU64::new(0)),
            backoff: Arc::new(Mutex::new(FailureBackoff::default())),
        }
    }
}

impl Widget for GitSyncWidget {
    fn id(&self) -> &'static str {
        "git-sync"
    }

    fn align(&self) -> Align {
        self.align
    }

    fn poll_interval(&self) -> Duration {
        self.backoff
            .lock()
            .map(|b| b.effective_interval(self.interval))
            .unwrap_or(self.interval)
    }

    fn last_poll(&self) -> Instant {
        self.last_poll
    }

    fn set_last_poll(&mut self, t: Instant) {
        self.last_poll = t;
    }

    fn poll(&mut self, ctx: &WidgetContext) {
        if !ctx.window_focused {
            return;
        }
        if ctx.active_tab_git.is_none() {
            let mut guard = self.state.lock().unwrap();
            *guard = None;
            self.pending_cwd = None;
            return;
        }

        if let Some(cwd) = ctx.active_tab_cwd {
            let cwd_path = cwd.to_path_buf();
            let is_new_cwd = Some(&cwd_path) != self.pending_cwd.as_ref();
            if is_new_cwd {
                self.pending_cwd = Some(cwd_path.clone());
                self.generation.fetch_add(1, Ordering::Relaxed);
                let mut guard = self.state.lock().unwrap();
                *guard = None;
            }

            if !self.is_fetching.load(Ordering::Relaxed) {
                self.is_fetching.store(true, Ordering::Relaxed);
                let state_clone = self.state.clone();
                let is_fetching_clone = self.is_fetching.clone();
                let generation_clone = self.generation.clone();
                let backoff_clone = self.backoff.clone();
                let gen = self.generation.load(Ordering::Relaxed);

                std::thread::spawn(move || {
                    let summary = (|| -> Option<SyncSummary> {
                        // 1. Get current branch name
                        let mut branch_cmd = std::process::Command::new("git");
                        branch_cmd.args(["rev-parse", "--abbrev-ref", "HEAD"]);
                        branch_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            branch_cmd.creation_flags(0x08000000);
                        }
                        let branch_out = run_with_timeout(&mut branch_cmd, LOCAL_PROC_TIMEOUT)?;
                        if !branch_out.status.success() {
                            return None;
                        }
                        let branch = String::from_utf8_lossy(&branch_out.stdout)
                            .trim()
                            .to_string();

                        // 2. Check if upstream branch is configured
                        let mut upstream_cmd = std::process::Command::new("git");
                        upstream_cmd.args(["rev-parse", "--abbrev-ref", "@{u}"]);
                        upstream_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            upstream_cmd.creation_flags(0x08000000);
                        }
                        let upstream_out = run_with_timeout(&mut upstream_cmd, LOCAL_PROC_TIMEOUT);
                        let has_upstream =
                            upstream_out.map(|o| o.status.success()).unwrap_or(false);

                        if !has_upstream {
                            return Some(SyncSummary {
                                ahead: 0,
                                behind: 0,
                                branch,
                                has_upstream: false,
                                ahead_commits: Vec::new(),
                                behind_commits: Vec::new(),
                                cwd: cwd_path,
                            });
                        }

                        // 3. Get ahead/behind counts
                        let mut count_cmd = std::process::Command::new("git");
                        count_cmd.args(["rev-list", "--left-right", "--count", "HEAD...@{u}"]);
                        count_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            count_cmd.creation_flags(0x08000000);
                        }
                        let count_out = run_with_timeout(&mut count_cmd, LOCAL_PROC_TIMEOUT)?;
                        let counts_str = String::from_utf8_lossy(&count_out.stdout);
                        let mut parts = counts_str.split_whitespace();
                        let ahead: usize = parts.next()?.parse().ok()?;
                        let behind: usize = parts.next()?.parse().ok()?;

                        // 4. Get ahead commits (to push)
                        let mut ahead_commits = Vec::new();
                        if ahead > 0 {
                            let mut log_cmd = std::process::Command::new("git");
                            log_cmd.args(["log", "@{u}..HEAD", "--oneline", "-n", "8"]);
                            log_cmd.current_dir(&cwd_path);
                            #[cfg(target_os = "windows")]
                            {
                                use std::os::windows::process::CommandExt;
                                log_cmd.creation_flags(0x08000000);
                            }
                            if let Some(out) = run_with_timeout(&mut log_cmd, LOCAL_PROC_TIMEOUT) {
                                let lines_str = String::from_utf8_lossy(&out.stdout);
                                for line in lines_str.lines() {
                                    ahead_commits.push(line.to_string());
                                }
                            }
                        }

                        // 5. Get behind commits (to pull)
                        let mut behind_commits = Vec::new();
                        if behind > 0 {
                            let mut log_cmd = std::process::Command::new("git");
                            log_cmd.args(["log", "HEAD..@{u}", "--oneline", "-n", "8"]);
                            log_cmd.current_dir(&cwd_path);
                            #[cfg(target_os = "windows")]
                            {
                                use std::os::windows::process::CommandExt;
                                log_cmd.creation_flags(0x08000000);
                            }
                            if let Some(out) = run_with_timeout(&mut log_cmd, LOCAL_PROC_TIMEOUT) {
                                let lines_str = String::from_utf8_lossy(&out.stdout);
                                for line in lines_str.lines() {
                                    behind_commits.push(line.to_string());
                                }
                            }
                        }

                        Some(SyncSummary {
                            ahead,
                            behind,
                            branch,
                            has_upstream: true,
                            ahead_commits,
                            behind_commits,
                            cwd: cwd_path,
                        })
                    })();

                    // Stale result (cwd changed mid-flight): drop it so it
                    // can't overwrite fresher state.
                    if gen != generation_clone.load(Ordering::Relaxed) {
                        is_fetching_clone.store(false, Ordering::Relaxed);
                        return;
                    }
                    if let Ok(mut b) = backoff_clone.lock() {
                        b.record(summary.is_some());
                    }
                    if let Some(sum) = summary {
                        let mut guard = state_clone.lock().unwrap();
                        *guard = Some(sum);
                    }
                    is_fetching_clone.store(false, Ordering::Relaxed);
                });
            }
        } else {
            let mut guard = self.state.lock().unwrap();
            *guard = None;
            self.pending_cwd = None;
        }
    }

    fn render(&mut self, _ctx: &WidgetContext) -> Vec<Segment> {
        let guard = self.state.lock().unwrap();
        let Some(summary) = guard.as_ref() else {
            return Vec::new();
        };

        if !summary.has_upstream {
            return vec![
                Segment::text(" Sync: no remote", [0.65, 0.65, 0.65, 1.0]).with_tooltip(format!(
                    "Branch '{}' has no upstream configured.",
                    summary.branch
                )),
            ];
        }

        // One color for the whole segment, as before: when a branch is both
        // ahead and behind, the behind color wins and the ahead count reads in
        // it too. Splitting this into one segment per direction would give each
        // its own color, at the cost of a wider gap between them.
        let mut parts = vec![SegmentPart::Text("Sync:".to_string())];
        let mut color = [0.85, 0.88, 0.95, 1.0]; // standard light gray/blue

        if summary.ahead == 0 && summary.behind == 0 {
            parts.push(SegmentPart::Icon(IconType::Check));
            color = [0.45, 0.85, 0.55, 1.0]; // green
        } else {
            if summary.ahead > 0 {
                parts.push(SegmentPart::Icon(IconType::ArrowUp));
                parts.push(SegmentPart::Text(summary.ahead.to_string()));
                color = [0.95, 0.80, 0.45, 1.0]; // yellow
            }
            if summary.behind > 0 {
                parts.push(SegmentPart::Icon(IconType::ArrowDown));
                parts.push(SegmentPart::Text(summary.behind.to_string()));
                color = [0.90, 0.40, 0.40, 1.0]; // red/orange
            }
        }

        vec![Segment::parts(parts, color).with_tooltip(format!(
            "Branch '{}' is ahead by {} and behind by {} relative to remote.",
            summary.branch, summary.ahead, summary.behind
        ))]
    }

    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        ClickAction::None
    }

    fn get_context_menu_items(&self) -> Option<Vec<ContextMenuItem>> {
        let guard = self.state.lock().unwrap();
        let summary = guard.as_ref()?;
        let cwd_str = summary.cwd.to_string_lossy().to_string();

        let mut items = Vec::new();

        items.push(ContextMenuItem::GithubActionInfo {
            label: format!("Sync: Branch '{}'", summary.branch),
            status: if summary.ahead == 0 && summary.behind == 0 {
                "success".to_string()
            } else if summary.behind > 0 {
                "failure".to_string()
            } else {
                "in_progress".to_string()
            },
            url: None,
        });

        items.push(ContextMenuItem::Separator);

        if !summary.has_upstream {
            items.push(ContextMenuItem::GithubActionInfo {
                label: "No upstream configured for branch".to_string(),
                status: "skipped".to_string(),
                url: None,
            });
            return Some(items);
        }

        // Actions
        items.push(ContextMenuItem::CommandItem {
            label: "Pull (git pull)".to_string(),
            command: "git pull".to_string(),
            cwd: cwd_str.clone(),
        });

        items.push(ContextMenuItem::CommandItem {
            label: "Push (git push)".to_string(),
            command: "git push".to_string(),
            cwd: cwd_str.clone(),
        });

        items.push(ContextMenuItem::CommandItem {
            label: "Fetch (git fetch)".to_string(),
            command: "git fetch".to_string(),
            cwd: cwd_str.clone(),
        });

        if summary.behind > 0 {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::GithubActionInfo {
                label: format!("Commits Behind ({})", summary.behind),
                status: "failure".to_string(),
                url: None,
            });
            for commit in &summary.behind_commits {
                items.push(ContextMenuItem::GithubActionInfo {
                    label: format!("  {}", commit),
                    status: "skipped".to_string(),
                    url: None,
                });
            }
        }

        if summary.ahead > 0 {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::GithubActionInfo {
                label: format!("Commits Ahead ({})", summary.ahead),
                status: "success".to_string(),
                url: None,
            });
            for commit in &summary.ahead_commits {
                items.push(ContextMenuItem::GithubActionInfo {
                    label: format!("  {}", commit),
                    status: "skipped".to_string(),
                    url: None,
                });
            }
        }

        if summary.ahead == 0 && summary.behind == 0 {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::GithubActionInfo {
                label: "Up to date with remote".to_string(),
                status: "success".to_string(),
                url: None,
            });
        }

        Some(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::SegmentPart;

    const CTX: WidgetContext<'static> = WidgetContext {
        active_tab_cwd: None,
        active_tab_git: None,
        opacity: 1.0,
        window_focused: true,
    };

    fn widget_with(summary: SyncSummary) -> GitSyncWidget {
        let widget = GitSyncWidget::new(Align::Left, None);
        *widget.state.lock().unwrap() = Some(summary);
        widget
    }

    fn summary(ahead: usize, behind: usize) -> SyncSummary {
        SyncSummary {
            ahead,
            behind,
            branch: "main".to_string(),
            has_upstream: true,
            ahead_commits: Vec::new(),
            behind_commits: Vec::new(),
            cwd: std::path::PathBuf::new(),
        }
    }

    /// `\u2713`, `\u{21e1}` and `\u{21e3}` are text-presentation and depend on the
    /// system font, so the sync state has to be icons.
    #[test]
    fn the_sync_state_is_icons() {
        for (ahead, behind, icons) in [
            (0, 0, vec![IconType::Check]),
            (3, 0, vec![IconType::ArrowUp]),
            (0, 2, vec![IconType::ArrowDown]),
            (3, 2, vec![IconType::ArrowUp, IconType::ArrowDown]),
        ] {
            let mut widget = widget_with(summary(ahead, behind));
            let segs = widget.render(&CTX);
            let seg = segs.first().expect("a sync segment must exist");
            let drawn: Vec<IconType> = seg
                .parts
                .iter()
                .filter_map(|p| match p {
                    SegmentPart::Icon(i) => Some(*i),
                    SegmentPart::Text(_) => None,
                })
                .collect();
            assert_eq!(drawn, icons, "ahead={ahead} behind={behind}");
        }
    }

    /// The label comes first, then each direction with its own count.
    #[test]
    fn the_sync_label_comes_before_the_counts() {
        let mut widget = widget_with(summary(3, 2));
        let segs = widget.render(&CTX);
        let seg = segs.first().unwrap();
        assert!(matches!(&seg.parts[0], SegmentPart::Text(t) if t == "Sync:"));
        assert_eq!(seg.parts.len(), 5, "label, up, count, down, count");
    }

    /// A branch with no upstream says so in words, with no glyph.
    #[test]
    fn no_upstream_is_plain_text() {
        let mut s = summary(0, 0);
        s.has_upstream = false;
        let mut widget = widget_with(s);
        let segs = widget.render(&CTX);
        let seg = segs.first().unwrap();
        assert!(seg.parts.iter().all(|p| matches!(p, SegmentPart::Text(_))));
    }
}
