//! GitHub Actions status widget.
//!
//! Spawns background tasks to query the GitHub CLI `gh` for the latest workflow run status
//! and step outcomes. Shows success, failure, and skipped step counts on the bottombar,
//! and provides a context menu with the jobs/steps details when clicked.

use icons::common::IconType;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::widgets::proc_util::{
    binary_on_path, gh_authenticated, run_with_timeout, FailureBackoff, NETWORK_PROC_TIMEOUT,
};
use crate::widgets::{
    Align, ClickAction, ContextMenuItem, Segment, SegmentPart, Widget, WidgetContext,
};

const DEFAULT_INTERVAL_MS: u64 = 60_000; // 60 seconds (GitHub API is queried)

pub struct GitActionsWidget {
    align: Align,
    last_poll: Instant,
    interval: Duration,
    state: Arc<Mutex<Option<ActionsSummary>>>,
    is_fetching: Arc<AtomicBool>,
    pending_cwd: Option<PathBuf>,
    /// Bumped on every cwd change; in-flight threads with a stale generation
    /// drop their result instead of overwriting fresher state.
    generation: Arc<AtomicU64>,
    backoff: Arc<Mutex<FailureBackoff>>,
    /// True when the `gh` CLI is missing (not merely offline): render a
    /// subtle install hint instead of silence.
    gh_missing: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
pub struct ActionsSummary {
    pub success_count: usize,
    pub failure_count: usize,
    pub skipped_count: usize,
    pub total_count: usize,
    pub in_progress: bool,
    pub jobs: Vec<JobInfo>,
}

#[derive(Clone, Debug)]
pub struct JobInfo {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
    pub steps: Vec<StepInfo>,
}

#[derive(Clone, Debug)]
pub struct StepInfo {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct GhRun {
    #[serde(rename = "databaseId")]
    database_id: Option<u64>,
}

#[derive(serde::Deserialize, Debug)]
struct GhStepRaw {
    name: String,
    status: String,
    conclusion: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct GhJobRaw {
    name: String,
    status: String,
    conclusion: Option<String>,
    url: Option<String>,
    steps: Option<Vec<GhStepRaw>>,
}

#[derive(serde::Deserialize, Debug)]
struct GhRunViewRaw {
    jobs: Option<Vec<GhJobRaw>>,
}

impl GitActionsWidget {
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
            gh_missing: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Widget for GitActionsWidget {
    fn id(&self) -> &'static str {
        "git-actions"
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
                let gh_missing_clone = self.gh_missing.clone();
                let gen = self.generation.load(Ordering::Relaxed);

                std::thread::spawn(move || {
                    // Offline / unauthenticated fast path: keep cached state
                    // instead of launching doomed requests.
                    if !gh_authenticated() {
                        if let Ok(mut b) = backoff_clone.lock() {
                            b.record(false);
                        }
                        gh_missing_clone.store(!binary_on_path("gh"), Ordering::Relaxed);
                        is_fetching_clone.store(false, Ordering::Relaxed);
                        return;
                    }
                    let summary = (|| -> Option<ActionsSummary> {
                        // 1. Run "gh run list" to check databaseId
                        let mut list_cmd = std::process::Command::new("gh");
                        list_cmd.args(["run", "list", "--limit", "1", "--json", "databaseId"]);
                        list_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            list_cmd.creation_flags(0x08000000);
                        }
                        let list_out = run_with_timeout(&mut list_cmd, NETWORK_PROC_TIMEOUT)?;
                        if !list_out.status.success() {
                            return None;
                        }
                        let runs: Vec<GhRun> = serde_json::from_slice(&list_out.stdout).ok()?;
                        let run = runs.first()?;
                        let run_id = run.database_id?;

                        // 2. Run "gh run view <id> --json jobs"
                        let mut view_cmd = std::process::Command::new("gh");
                        view_cmd.args(["run", "view", &run_id.to_string(), "--json", "jobs"]);
                        view_cmd.current_dir(&cwd_path);
                        #[cfg(target_os = "windows")]
                        {
                            use std::os::windows::process::CommandExt;
                            view_cmd.creation_flags(0x08000000);
                        }
                        let view_out = run_with_timeout(&mut view_cmd, NETWORK_PROC_TIMEOUT)?;
                        if !view_out.status.success() {
                            return None;
                        }
                        let view_data: GhRunViewRaw =
                            serde_json::from_slice(&view_out.stdout).ok()?;
                        let jobs_raw = view_data.jobs?;

                        let mut success_count = 0;
                        let mut failure_count = 0;
                        let mut skipped_count = 0;
                        let mut total_count = 0;
                        let mut in_progress = false;
                        let mut jobs = Vec::new();

                        for j in jobs_raw {
                            let job_url = j.url.unwrap_or_default();
                            let mut steps = Vec::new();

                            if j.status == "in_progress" || j.status == "queued" {
                                in_progress = true;
                            }

                            if let Some(steps_raw) = j.steps {
                                for s in steps_raw {
                                    total_count += 1;
                                    if s.status == "in_progress" || s.status == "queued" {
                                        in_progress = true;
                                    }
                                    match s.conclusion.as_deref() {
                                        Some("success") => success_count += 1,
                                        Some("failure")
                                        | Some("timed_out")
                                        | Some("action_required") => failure_count += 1,
                                        Some("skipped") => skipped_count += 1,
                                        _ => {}
                                    }
                                    steps.push(StepInfo {
                                        name: s.name,
                                        status: s.status,
                                        conclusion: s.conclusion,
                                    });
                                }
                            }

                            jobs.push(JobInfo {
                                name: j.name,
                                status: j.status,
                                conclusion: j.conclusion,
                                url: job_url,
                                steps,
                            });
                        }

                        Some(ActionsSummary {
                            success_count,
                            failure_count,
                            skipped_count,
                            total_count,
                            in_progress,
                            jobs,
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
                        gh_missing_clone.store(false, Ordering::Relaxed);
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
            if self.is_fetching.load(Ordering::Relaxed) {
                return vec![Segment::parts(
                    vec![
                        SegmentPart::Text("Actions:".to_string()),
                        SegmentPart::Icon(IconType::Loader),
                    ],
                    [0.65, 0.65, 0.65, 1.0],
                )
                .with_tooltip("Fetching GitHub Actions status...")];
            }
            // UX2: subtle one-glance hint instead of silence when `gh` is
            // missing; the tooltip carries the explanation.
            if self.gh_missing.load(Ordering::Relaxed) {
                return vec![
                    Segment::text(" Actions: – ", [0.55, 0.55, 0.62, 1.0]).with_tooltip(
                        "gh CLI not found on PATH — install it to show Actions status here."
                            .to_string(),
                    ),
                ];
            }
            return Vec::new();
        };

        if summary.total_count == 0 {
            return Vec::new();
        }

        let mut segs = Vec::new();
        segs.push(Segment::text(" Actions:", [0.85, 0.88, 0.95, 1.0]));

        if summary.success_count > 0 {
            segs.push(Segment::parts(
                vec![
                    SegmentPart::Icon(IconType::Check),
                    SegmentPart::Text(summary.success_count.to_string()),
                ],
                [0.45, 0.85, 0.55, 1.0],
            ));
        }

        if summary.failure_count > 0 {
            segs.push(Segment::parts(
                vec![
                    SegmentPart::Icon(IconType::X),
                    SegmentPart::Text(summary.failure_count.to_string()),
                ],
                [0.90, 0.40, 0.40, 1.0],
            ));
        }

        if summary.skipped_count > 0 {
            segs.push(Segment::parts(
                vec![
                    SegmentPart::Icon(IconType::SkipForward),
                    SegmentPart::Text(summary.skipped_count.to_string()),
                ],
                [0.65, 0.65, 0.65, 1.0],
            ));
        }

        if summary.in_progress {
            segs.push(
                Segment::parts(
                    vec![SegmentPart::Icon(IconType::Loader)],
                    [0.95, 0.80, 0.45, 1.0],
                )
                .with_tooltip("GitHub Actions run is in progress"),
            );
        }

        segs
    }

    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        ClickAction::ShowActionsMenu
    }

    fn tooltip(&self) -> Option<String> {
        let guard = self.state.lock().unwrap();
        if let Some(summary) = guard.as_ref() {
            let mut lines = Vec::new();
            lines.push("GitHub Actions Summary:".to_string());
            lines.push(format!("{} successful", summary.success_count));
            lines.push(format!("{} failed", summary.failure_count));
            lines.push(format!("{} skipped", summary.skipped_count));
            if summary.in_progress {
                lines.push("Run is in progress".to_string());
            }
            lines.push("\nClick to see job and step details.".to_string());
            Some(lines.join("\n"))
        } else {
            None
        }
    }

    fn get_context_menu_items(&self) -> Option<Vec<ContextMenuItem>> {
        let guard = self.state.lock().unwrap();
        let summary = guard.as_ref()?;

        let mut items = Vec::new();
        let run_url = summary.jobs.first().map(|j| j.url.clone());

        items.push(ContextMenuItem::GithubActionInfo {
            label: "GitHub Actions Run".to_string(),
            status: if summary.in_progress {
                "in_progress".to_string()
            } else if summary.failure_count > 0 {
                "failure".to_string()
            } else {
                "success".to_string()
            },
            url: run_url,
        });

        items.push(ContextMenuItem::Separator);

        let total_steps: usize = summary.jobs.iter().map(|j| j.steps.len()).sum();
        let show_all = total_steps <= 15;

        for (job_idx, job) in summary.jobs.iter().enumerate() {
            if job_idx > 0 {
                items.push(ContextMenuItem::Separator);
            }

            items.push(ContextMenuItem::GithubActionInfo {
                label: format!("Job: {}", job.name),
                status: job.conclusion.clone().unwrap_or_else(|| job.status.clone()),
                url: Some(job.url.clone()),
            });

            let mut succeeded_count = 0;
            for step in &job.steps {
                let is_success = step.conclusion.as_deref() == Some("success");
                if is_success {
                    succeeded_count += 1;
                }

                if show_all || !is_success {
                    items.push(ContextMenuItem::GithubActionInfo {
                        label: step.name.clone(),
                        status: step
                            .conclusion
                            .clone()
                            .unwrap_or_else(|| step.status.clone()),
                        url: Some(job.url.clone()),
                    });
                }
            }

            if !show_all && succeeded_count > 0 {
                items.push(ContextMenuItem::GithubActionInfo {
                    label: format!("{} more step(s) succeeded", succeeded_count),
                    status: "success".to_string(),
                    url: Some(job.url.clone()),
                });
            }
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

    fn widget_with(summary: ActionsSummary) -> GitActionsWidget {
        let widget = GitActionsWidget::new(Align::Left, None);
        *widget.state.lock().unwrap() = Some(summary);
        widget
    }

    fn summary(
        success: usize,
        failure: usize,
        skipped: usize,
        in_progress: bool,
    ) -> ActionsSummary {
        ActionsSummary {
            success_count: success,
            failure_count: failure,
            skipped_count: skipped,
            total_count: success + failure + skipped,
            in_progress,
            jobs: Vec::new(),
        }
    }

    fn drawn(mut widget: GitActionsWidget) -> Vec<IconType> {
        widget
            .render(&CTX)
            .into_iter()
            .filter_map(|seg| {
                seg.parts.into_iter().find_map(|p| match p {
                    SegmentPart::Icon(i) => Some(i),
                    SegmentPart::Text(_) => None,
                })
            })
            .collect()
    }

    /// `✓`, `✗`, `↷` and `↻` are text-presentation, so the counts and the
    /// spinner have to be icons.
    #[test]
    fn the_counts_are_icons() {
        assert_eq!(
            drawn(widget_with(summary(3, 0, 0, false))),
            vec![IconType::Check]
        );
        assert_eq!(
            drawn(widget_with(summary(0, 2, 0, false))),
            vec![IconType::X]
        );
        assert_eq!(
            drawn(widget_with(summary(0, 0, 5, false))),
            vec![IconType::SkipForward]
        );
        assert_eq!(
            drawn(widget_with(summary(3, 2, 5, true))),
            vec![
                IconType::Check,
                IconType::X,
                IconType::SkipForward,
                IconType::Loader
            ]
        );
    }

    /// A count is an icon followed by its number, not a glyph inside a string.
    #[test]
    fn each_count_carries_its_number() {
        let mut widget = widget_with(summary(3, 2, 0, false));
        let segs = widget.render(&CTX);
        let numbers: Vec<String> = segs
            .iter()
            .filter_map(|seg| {
                seg.parts.iter().find_map(|p| match p {
                    SegmentPart::Text(t)
                        if t.chars().all(|c| c.is_ascii_digit()) && !t.is_empty() =>
                    {
                        Some(t.clone())
                    }
                    _ => None,
                })
            })
            .collect();
        assert_eq!(numbers, vec!["3", "2"]);
    }

    /// While `gh` is still being queried the bar shows a spinner instead of
    /// silence.
    #[test]
    fn the_fetching_placeholder_is_a_spinner() {
        let widget = GitActionsWidget::new(Align::Left, None);
        widget.is_fetching.store(true, Ordering::Relaxed);
        assert_eq!(drawn(widget), vec![IconType::Loader]);
    }

    /// No segment may carry a text-presentation glyph, which is how these were
    /// all drawn before.
    #[test]
    fn no_segment_carries_a_status_glyph_as_text() {
        const GLYPHS: [char; 4] = ['\u{2713}', '\u{2717}', '\u{21bb}', '\u{21b7}'];
        let mut widget = widget_with(summary(3, 2, 5, true));
        for seg in widget.render(&CTX) {
            for part in &seg.parts {
                if let SegmentPart::Text(text) = part {
                    assert!(
                        !GLYPHS.iter().any(|g| text.contains(*g)),
                        "a status glyph must not come back as text: {text:?}"
                    );
                }
            }
        }
    }
}
