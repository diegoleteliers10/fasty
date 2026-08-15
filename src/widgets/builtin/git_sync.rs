use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::widgets::{Align, ClickAction, ContextMenuItem, Segment, Widget, WidgetContext};

const DEFAULT_INTERVAL_MS: u64 = 30_000; // 30 seconds

pub struct GitSyncWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    state: Arc<Mutex<Option<SyncSummary>>>,
    is_fetching: Arc<AtomicBool>,
    pending_cwd: Option<std::path::PathBuf>,
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
        self.interval
    }

    fn last_poll(&self) -> Instant {
        self.last_poll
    }

    fn set_last_poll(&mut self, t: Instant) {
        self.last_poll = t;
    }

    fn poll(&mut self, ctx: &WidgetContext) {
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
                let mut guard = self.state.lock().unwrap();
                *guard = None;
            }

            if !self.is_fetching.load(Ordering::Relaxed) {
                self.is_fetching.store(true, Ordering::Relaxed);
                let state_clone = self.state.clone();
                let is_fetching_clone = self.is_fetching.clone();

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
                        let branch_out = branch_cmd.output().ok()?;
                        if !branch_out.status.success() {
                            return None;
                        }
                        let branch = String::from_utf8_lossy(&branch_out.stdout).trim().to_string();

                        // 2. Check if upstream branch is configured
                        let mut upstream_cmd = std::process::Command::new("git");
                        upstream_cmd.args(["rev-parse", "--abbrev-ref", "@{u}"]);
                        upstream_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            upstream_cmd.creation_flags(0x08000000);
                        }
                        let upstream_out = upstream_cmd.output().ok();
                        let has_upstream = upstream_out.map(|o| o.status.success()).unwrap_or(false);

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
                        let count_out = count_cmd.output().ok()?;
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
                            if let Ok(out) = log_cmd.output() {
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
                            if let Ok(out) = log_cmd.output() {
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
            return vec![Segment {
                text: " Sync: no remote".to_string(),
                color: [0.65, 0.65, 0.65, 1.0],
                tooltip: Some(format!("Branch '{}' has no upstream configured.", summary.branch)),
            }];
        }

        let mut text = String::new();
        let mut color = [0.85, 0.88, 0.95, 1.0]; // standard light gray/blue

        if summary.ahead == 0 && summary.behind == 0 {
            text = " Sync: ✓".to_string();
            color = [0.45, 0.85, 0.55, 1.0]; // green
        } else {
            text.push_str(" Sync:");
            if summary.ahead > 0 {
                text.push_str(&format!(" ⇡{}", summary.ahead));
                color = [0.95, 0.80, 0.45, 1.0]; // yellow
            }
            if summary.behind > 0 {
                text.push_str(&format!(" ⇣{}", summary.behind));
                color = [0.90, 0.40, 0.40, 1.0]; // red/orange
            }
        }

        vec![Segment {
            text,
            color,
            tooltip: Some(format!(
                "Branch '{}' is ahead by {} and behind by {} relative to remote.",
                summary.branch, summary.ahead, summary.behind
            )),
        }]
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
            label: "⇣ Pull (git pull)".to_string(),
            command: "git pull".to_string(),
            cwd: cwd_str.clone(),
        });

        items.push(ContextMenuItem::CommandItem {
            label: "⇡ Push (git push)".to_string(),
            command: "git push".to_string(),
            cwd: cwd_str.clone(),
        });

        items.push(ContextMenuItem::CommandItem {
            label: "↻ Fetch (git fetch)".to_string(),
            command: "git fetch".to_string(),
            cwd: cwd_str.clone(),
        });

        if summary.behind > 0 {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::GithubActionInfo {
                label: format!("⇣ Commits Behind ({})", summary.behind),
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
                label: format!("⇡ Commits Ahead ({})", summary.ahead),
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
                label: "✓ Up to date with remote".to_string(),
                status: "success".to_string(),
                url: None,
            });
        }

        Some(items)
    }
}
