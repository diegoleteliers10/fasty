use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::widgets::{Align, ClickAction, Segment, Widget, WidgetContext};
use crate::renderer::ContextMenuItem;

const DEFAULT_INTERVAL_MS: u64 = 60_000; // 60 seconds (since GitHub API is queried)

pub struct GitPrsWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    state: Arc<Mutex<Option<PrsSummary>>>,
    is_fetching: Arc<AtomicBool>,
    pending_cwd: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PrsSummary {
    current_pr: Option<GhPrView>,
    open_prs: Vec<GhPrList>,
    review_requested_prs: Vec<GhPrList>,
    cwd: std::path::PathBuf,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct GhPrAuthor {
    login: String,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct GhPrList {
    number: usize,
    title: String,
    url: String,
    author: Option<GhPrAuthor>,
}

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GhPrView {
    state: String,
    number: usize,
    title: String,
    url: String,
    review_decision: Option<String>,
}

impl GitPrsWidget {
    pub fn new(align: Align, interval_ms: Option<u64>) -> Self {
        Self {
            align,
            last_poll: Instant::now() - Duration::from_secs(120),
            interval: Duration::from_millis(interval_ms.unwrap_or(DEFAULT_INTERVAL_MS)),
            state: Arc::new(Mutex::new(None)),
            is_fetching: Arc::new(AtomicBool::new(false)),
            pending_cwd: None,
        }
    }
}

impl Widget for GitPrsWidget {
    fn id(&self) -> &'static str {
        "git-prs"
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
                    let summary = (|| -> Option<PrsSummary> {
                        // 1. Check current branch PR status
                        let mut view_cmd = std::process::Command::new("gh");
                        view_cmd.args(["pr", "view", "--json", "state,number,title,url,reviewDecision"]);
                        view_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            view_cmd.creation_flags(0x08000000);
                        }
                        let view_out = view_cmd.output().ok()?;
                        let current_pr: Option<GhPrView> = if view_out.status.success() {
                            serde_json::from_slice(&view_out.stdout).ok()
                        } else {
                            None
                        };

                        // 2. Get active PRs list
                        let mut list_cmd = std::process::Command::new("gh");
                        list_cmd.args(["pr", "list", "--limit", "10", "--json", "number,title,url,author"]);
                        list_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            list_cmd.creation_flags(0x08000000);
                        }
                        let list_out = list_cmd.output().ok()?;
                        let open_prs: Vec<GhPrList> = if list_out.status.success() {
                            serde_json::from_slice(&list_out.stdout).unwrap_or_default()
                        } else {
                            Vec::new()
                        };

                        // 3. Get review requested PRs
                        let mut review_cmd = std::process::Command::new("gh");
                        review_cmd.args(["pr", "list", "--search", "review-requested:@me", "--limit", "5", "--json", "number,title,url,author"]);
                        review_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            review_cmd.creation_flags(0x08000000);
                        }
                        let review_out = review_cmd.output().ok()?;
                        let review_requested_prs: Vec<GhPrList> = if review_out.status.success() {
                            serde_json::from_slice(&review_out.stdout).unwrap_or_default()
                        } else {
                            Vec::new()
                        };

                        Some(PrsSummary {
                            current_pr,
                            open_prs,
                            review_requested_prs,
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

        let mut segs = Vec::new();

        if let Some(ref pr) = summary.current_pr {
            // State indicators
            let (status_text, color) = match pr.review_decision.as_deref() {
                Some("APPROVED") => (" ✓", [0.45, 0.85, 0.55, 1.0]), // green
                Some("CHANGES_REQUESTED") => (" ✗", [0.90, 0.40, 0.40, 1.0]), // red
                Some("REVIEW_REQUIRED") => (" ↻", [0.95, 0.80, 0.45, 1.0]), // yellow
                _ => (" ↻", [0.95, 0.80, 0.45, 1.0]), // default yellow review pending
            };
            segs.push(Segment {
                text: format!(" PR #{}", pr.number),
                color: [0.85, 0.88, 0.95, 1.0],
                tooltip: Some(format!("PR #{}: {}\nClick to open.", pr.number, pr.title)),
            });
            segs.push(Segment {
                text: status_text.to_string(),
                color,
                tooltip: Some(format!("Review status: {:?}", pr.review_decision)),
            });
        } else {
            // No current branch PR, display open PR count if positive
            if !summary.open_prs.is_empty() {
                let mut text = format!(" PRs: 📥{}", summary.open_prs.len());
                let mut tooltip = format!("{} open PRs in repository.", summary.open_prs.len());

                if !summary.review_requested_prs.is_empty() {
                    text.push_str(&format!(" 👤{}", summary.review_requested_prs.len()));
                    tooltip.push_str(&format!("\n{} PRs requesting your review.", summary.review_requested_prs.len()));
                }

                segs.push(Segment {
                    text,
                    color: [0.85, 0.88, 0.95, 1.0],
                    tooltip: Some(tooltip),
                });
            }
        }

        segs
    }

    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        ClickAction::ShowActionsMenu
    }

    fn get_context_menu_items(&self) -> Option<Vec<ContextMenuItem>> {
        let guard = self.state.lock().unwrap();
        let summary = guard.as_ref()?;

        let mut items = Vec::new();

        if let Some(ref pr) = summary.current_pr {
            items.push(ContextMenuItem::GithubActionInfo {
                label: "Current Branch PR".to_string(),
                status: "success".to_string(),
                url: None,
            });
            
            let status_str = match pr.review_decision.as_deref() {
                Some("APPROVED") => "approved",
                Some("CHANGES_REQUESTED") => "failure",
                _ => "in_progress",
            };
            
            items.push(ContextMenuItem::GithubActionInfo {
                label: format!("  #{} {}", pr.number, pr.title),
                status: status_str.to_string(),
                url: Some(pr.url.clone()),
            });
        }

        if !summary.review_requested_prs.is_empty() {
            if !items.is_empty() {
                items.push(ContextMenuItem::Separator);
            }
            items.push(ContextMenuItem::GithubActionInfo {
                label: "📥 Awaiting Your Review".to_string(),
                status: "failure".to_string(),
                url: None,
            });
            for pr in &summary.review_requested_prs {
                items.push(ContextMenuItem::GithubActionInfo {
                    label: format!("  👤 #{} {}", pr.number, pr.title),
                    status: "in_progress".to_string(),
                    url: Some(pr.url.clone()),
                });
            }
        }

        if !summary.open_prs.is_empty() {
            if !items.is_empty() {
                items.push(ContextMenuItem::Separator);
            }
            items.push(ContextMenuItem::GithubActionInfo {
                label: "📥 Open Pull Requests".to_string(),
                status: "success".to_string(),
                url: None,
            });
            for pr in &summary.open_prs {
                let author_str = pr.author.as_ref().map(|a| format!(" by @{}", a.login)).unwrap_or_default();
                items.push(ContextMenuItem::GithubActionInfo {
                    label: format!("  #{} {}{}", pr.number, pr.title, author_str),
                    status: "skipped".to_string(),
                    url: Some(pr.url.clone()),
                });
            }
        }

        if items.is_empty() {
            items.push(ContextMenuItem::GithubActionInfo {
                label: "No open pull requests found".to_string(),
                status: "skipped".to_string(),
                url: None,
            });
        }

        Some(items)
    }
}
