//! Git status widget.
//!
//! Reads the active tab's cached `GitStatus`. Renders branch + dirty dot +
//! ahead/behind/modified/staged/untracked counts.

use std::time::{Duration, Instant};

use icons::common::IconType;

use crate::widgets::{
    Align, ClickAction, ContextMenuItem, Segment, SegmentPart, Widget, WidgetContext,
};

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
    fn id(&self) -> &'static str {
        "git"
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
        self.cached = ctx.active_tab_git.cloned();
        if let Some(p) = ctx.active_tab_cwd {
            self.pending_cwd = Some(p.to_path_buf());
        }
    }

    fn render(&mut self, _ctx: &WidgetContext) -> Vec<Segment> {
        let Some(gs) = &self.cached else {
            return Vec::new();
        };
        let mut segs = Vec::with_capacity(8);

        // A branch marker, drawn as an icon rather than a glyph. `⎇` and `\u{27F3}`
        // are text-presentation characters, so whether they render at all
        // depends on the system font, which differs per platform. An SVG draws
        // the same shape everywhere.
        let (icon, icon_tooltip) = if gs.is_detached {
            // A detached HEAD sits on a commit, not on a branch.
            (
                IconType::GitCommitHorizontal,
                format!("Detached HEAD at {}", gs.last_commit_hash),
            )
        } else {
            (IconType::GitBranch, format!("Branch: {}", gs.branch))
        };
        segs.push(
            Segment::parts(
                vec![
                    SegmentPart::Icon(icon),
                    SegmentPart::Text(gs.branch.clone()),
                ],
                [0.85, 0.88, 0.95, 1.0],
            )
            .with_tooltip(icon_tooltip)
            .git_menu(),
        );

        // Staged: yellow dot + count (index changes).
        // `Dot` is a circle of r=1 stroked with width 2, so the stroke covers the
        // whole circle: it draws as a solid dot, the same as the `\u{25CF}` it
        // replaces.
        if gs.staged > 0 {
            segs.push(
                Segment::parts(
                    vec![
                        SegmentPart::Icon(IconType::Dot),
                        SegmentPart::Text(format!("+{}", gs.staged)),
                    ],
                    [0.95, 0.80, 0.45, 1.0],
                )
                .with_tooltip(format!("{} staged file(s)", gs.staged)),
            );
        }
        // Unstaged: green + count (working tree modifications)
        if gs.unstaged > 0 {
            segs.push(
                Segment::text(format!(" +{}", gs.unstaged), [0.45, 0.85, 0.55, 1.0])
                    .with_tooltip(format!("{} modified file(s) in working tree", gs.unstaged)),
            );
        }
        // Untracked: dim ? + count
        if gs.untracked > 0 {
            segs.push(
                Segment::text(format!(" ?{}", gs.untracked), [0.55, 0.55, 0.65, 1.0])
                    .with_tooltip(format!("{} untracked file(s)", gs.untracked)),
            );
        }
        // Ahead / behind
        if gs.ahead > 0 {
            segs.push(
                Segment::text(format!(" \u{2191}{}", gs.ahead), [0.45, 0.85, 0.55, 1.0])
                    .with_tooltip(format!("{} commit(s) ahead of upstream", gs.ahead)),
            );
        }
        if gs.behind > 0 {
            segs.push(
                Segment::text(format!(" \u{2193}{}", gs.behind), [0.90, 0.55, 0.40, 1.0])
                    .with_tooltip(format!("{} commit(s) behind upstream", gs.behind)),
            );
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

    fn get_context_menu_items(&self) -> Option<Vec<ContextMenuItem>> {
        let cwd_str = self.pending_cwd.as_ref()?.to_string_lossy().into_owned();
        let mut items = Vec::new();

        if let Some(gs) = &self.cached {
            // Sync status
            let (label, status) = match &gs.sync_status {
                crate::git::SyncStatus::UpToDate => (
                    "\u{2713} Up to date with remote".to_string(),
                    "success".to_string(),
                ),
                crate::git::SyncStatus::Behind(n) => (
                    format!("\u{2193} Behind remote by {} commit(s)", n),
                    "queued".to_string(),
                ),
                crate::git::SyncStatus::Ahead(n) => (
                    format!("\u{2191} Ahead of remote by {} commit(s)", n),
                    "queued".to_string(),
                ),
                crate::git::SyncStatus::Diverged => (
                    "\u{26A1} Diverged from remote".to_string(),
                    "failure".to_string(),
                ),
                crate::git::SyncStatus::Unknown => (
                    "\u{2014} No remote tracked".to_string(),
                    "skipped".to_string(),
                ),
            };
            items.push(ContextMenuItem::GithubActionInfo {
                label,
                status,
                url: None,
            });

            // Last commit
            if !gs.last_commit_summary.is_empty() {
                let truncated = if gs.last_commit_summary.len() > 30 {
                    format!("{}...", &gs.last_commit_summary[..27])
                } else {
                    gs.last_commit_summary.clone()
                };
                items.push(ContextMenuItem::GithubActionInfo {
                    label: format!("\u{1F50F} {}", truncated),
                    status: "skipped".to_string(),
                    url: None,
                });
            }

            // Remote URL (if GitHub)
            if let Some(remote_url) = &gs.remote_url {
                if remote_url.contains("github.com") {
                    // GitHub Actions link
                    let repo_path = remote_url
                        .trim_start_matches("https://")
                        .trim_start_matches("git@github.com:")
                        .trim_start_matches("http://")
                        .trim_start_matches("github.com/")
                        .trim_end_matches(".git");

                    items.push(ContextMenuItem::GithubActionInfo {
                        label: "\u{2692} GitHub Actions".to_string(),
                        status: "skipped".to_string(),
                        url: Some(format!("https://github.com/{}/actions", repo_path)),
                    });

                    // Repository link
                    items.push(ContextMenuItem::GithubActionInfo {
                        label: "\u{1F517} Open Repository".to_string(),
                        status: "skipped".to_string(),
                        url: Some(remote_url.clone()),
                    });
                }
            }

            items.push(ContextMenuItem::Separator);
        }

        // Sync actions
        items.push(ContextMenuItem::CommandItem {
            label: "\u{2B07} Pull  [git pull]".to_string(),
            command: "git pull".to_string(),
            cwd: cwd_str.clone(),
        });
        items.push(ContextMenuItem::CommandItem {
            label: "\u{2B06} Push  [git push]".to_string(),
            command: "git push".to_string(),
            cwd: cwd_str.clone(),
        });
        items.push(ContextMenuItem::CommandItem {
            label: "\u{21BA} Fetch [git fetch]".to_string(),
            command: "git fetch".to_string(),
            cwd: cwd_str.clone(),
        });

        // Additional git commands
        items.push(ContextMenuItem::Separator);

        items.push(ContextMenuItem::CommandItem {
            label: "\u{1F4CB} Status [git status]".to_string(),
            command: "git status".to_string(),
            cwd: cwd_str.clone(),
        });

        items.push(ContextMenuItem::CommandItem {
            label: "\u{1F4C5} Log (5) [git log]".to_string(),
            command: "git log --oneline -5".to_string(),
            cwd: cwd_str.clone(),
        });

        items.push(ContextMenuItem::CommandItem {
            label: "\u{1F4DD} Diff [git diff]".to_string(),
            command: "git diff".to_string(),
            cwd: cwd_str.clone(),
        });

        items.push(ContextMenuItem::CommandItem {
            label: "\u{2795} Stage All [git add]".to_string(),
            command: "git add .".to_string(),
            cwd: cwd_str.clone(),
        });

        items.push(ContextMenuItem::CommandItem {
            label: "\u{1F4BE} Commit [git commit]".to_string(),
            command: "git commit".to_string(),
            cwd: cwd_str,
        });

        Some(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitStatus;
    use crate::widgets::SegmentPart;

    fn status(branch: &str, is_detached: bool) -> GitStatus {
        GitStatus {
            branch: branch.to_string(),
            is_detached,
            last_commit_hash: "abc1234".to_string(),
            ..Default::default()
        }
    }

    fn first_part(widget: &mut GitWidget) -> SegmentPart {
        let segs = widget.render(&WidgetContext {
            active_tab_cwd: None,
            active_tab_git: None,
            opacity: 1.0,
            window_focused: true,
        });
        segs.first()
            .expect("a branch segment must exist")
            .parts
            .first()
            .cloned()
            .expect("the branch segment must lead with a part")
    }

    /// `⎇` is a text-presentation character, so whether it draws at all depends
    /// on the system font. The branch marker has to be an icon so the status
    /// bar looks the same on macOS, Windows and Linux.
    #[test]
    fn the_branch_marker_is_an_icon_not_a_glyph() {
        let mut widget = GitWidget::new(Align::Left, None);
        widget.cached = Some(status("main", false));
        assert!(
            matches!(
                first_part(&mut widget),
                SegmentPart::Icon(IconType::GitBranch)
            ),
            "a normal branch must lead with the branch icon"
        );
    }

    /// A detached HEAD is on a commit, not on a branch, so it gets a different
    /// icon. Reusing the branch icon would read as "on main".
    #[test]
    fn a_detached_head_gets_a_commit_icon() {
        let mut widget = GitWidget::new(Align::Left, None);
        widget.cached = Some(status("abc1234", true));
        assert!(
            matches!(
                first_part(&mut widget),
                SegmentPart::Icon(IconType::GitCommitHorizontal)
            ),
            "a detached HEAD must lead with the commit icon"
        );
    }

    /// No segment may smuggle the branch glyph back in as text, which is how it
    /// was drawn before. This catches a partial revert of the icon conversion.
    #[test]
    fn no_segment_carries_the_branch_glyph_as_text() {
        for detached in [false, true] {
            let mut widget = GitWidget::new(Align::Left, None);
            widget.cached = Some(status("main", detached));
            for seg in widget.render(&WidgetContext {
                active_tab_cwd: None,
                active_tab_git: None,
                opacity: 1.0,
                window_focused: true,
            }) {
                for part in &seg.parts {
                    if let SegmentPart::Text(text) = part {
                        assert!(
                            !text.contains('\u{2387}'),
                            "the branch glyph must not come back as text: {text:?}"
                        );
                    }
                }
            }
        }
    }

    /// The branch segment is the one that opens the git menu.
    #[test]
    fn the_branch_segment_opens_the_git_menu() {
        let mut widget = GitWidget::new(Align::Left, None);
        widget.cached = Some(status("main", false));
        let segs = widget.render(&WidgetContext {
            active_tab_cwd: None,
            active_tab_git: None,
            opacity: 1.0,
            window_focused: true,
        });
        assert!(segs.first().is_some_and(|s| s.opens_git_menu));
    }
}
