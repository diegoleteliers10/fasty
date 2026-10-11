use crate::git::{GitCommit, GitFileChange, GitFileSide, GitFileStatus, GitPanelAvailability, GitPanelSnapshot, ProjectGitAction};
use gpui::{
    div, prelude::*, px, App, CursorStyle, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    RenderOnce, ScrollHandle, ScrollWheelEvent, SharedString, Window,
};
use std::ops::Range;

use super::icons;
use super::theme::Theme;
use ::icons::common::IconType;

type CloseCallback = std::sync::Arc<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;
type TabCallback = std::sync::Arc<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;
type ActionCallback = std::sync::Arc<dyn Fn(&ProjectGitAction, &MouseDownEvent, &mut Window, &mut App) + 'static>;
type SimpleCallback = std::sync::Arc<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;
type LoadMoreCallback = std::sync::Arc<dyn Fn(usize, bool, bool, &ScrollWheelEvent, &mut Window, &mut App) + 'static>;

const GRAPH_LANE_WIDTH: f32 = 14.0;
const GRAPH_LANE_LIMIT: usize = 8;
const GRAPH_ROW_HEIGHT: f32 = 52.0;
const GRAPH_OVERSCAN_ROWS: usize = 5;
const GRAPH_TOP_PADDING: f32 = 10.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProjectPanelTab {
    #[default]
    Changes,
    Graph,
}

#[derive(IntoElement)]
pub struct ProjectSidebar {
    theme: Theme,
    width: f32,
    snapshot: GitPanelSnapshot,
    active_tab: ProjectPanelTab,
    on_close: Option<CloseCallback>,
    on_select_changes: Option<TabCallback>,
    on_select_graph: Option<TabCallback>,
    menu_open: bool,
    pending_confirmation: Option<ProjectGitAction>,
    feedback: Option<String>,
    on_action: Option<ActionCallback>,
    on_confirm_action: Option<ActionCallback>,
    on_toggle_menu: Option<SimpleCallback>,
    on_cancel_confirmation: Option<SimpleCallback>,
    graph_scroll_handle: ScrollHandle,
    graph_rendered_range: std::rc::Rc<std::cell::Cell<(usize, usize)>>,
    graph_layout: Option<std::sync::Arc<GraphLayout>>,
    on_load_more: Option<LoadMoreCallback>,
}

impl ProjectSidebar {
    pub fn new(
        theme: Theme,
        width: f32,
        snapshot: GitPanelSnapshot,
        active_tab: ProjectPanelTab,
    ) -> Self {
        Self { theme, width, snapshot, active_tab, on_close: None, on_select_changes: None, on_select_graph: None, menu_open: false, pending_confirmation: None, feedback: None, on_action: None, on_confirm_action: None, on_toggle_menu: None, on_cancel_confirmation: None, graph_scroll_handle: ScrollHandle::new(), graph_rendered_range: std::rc::Rc::new(std::cell::Cell::new((0, 0))), graph_layout: None, on_load_more: None }
    }

    pub fn on_close(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(std::sync::Arc::new(handler));
        self
    }

    pub fn on_select_changes(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select_changes = Some(std::sync::Arc::new(handler));
        self
    }

    pub fn on_select_graph(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select_graph = Some(std::sync::Arc::new(handler));
        self
    }

    pub fn actions(
        mut self,
        menu_open: bool,
        pending_confirmation: Option<ProjectGitAction>,
        feedback: Option<String>,
        on_action: impl Fn(&ProjectGitAction, &MouseDownEvent, &mut Window, &mut App) + 'static,
        on_confirm_action: impl Fn(&ProjectGitAction, &MouseDownEvent, &mut Window, &mut App) + 'static,
        on_toggle_menu: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
        on_cancel_confirmation: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.menu_open = menu_open;
        self.pending_confirmation = pending_confirmation;
        self.feedback = feedback;
        self.on_action = Some(std::sync::Arc::new(on_action));
        self.on_confirm_action = Some(std::sync::Arc::new(on_confirm_action));
        self.on_toggle_menu = Some(std::sync::Arc::new(on_toggle_menu));
        self.on_cancel_confirmation = Some(std::sync::Arc::new(on_cancel_confirmation));
        self
    }

    pub fn graph_pagination(
        mut self,
        scroll_handle: ScrollHandle,
        rendered_range: std::rc::Rc<std::cell::Cell<(usize, usize)>>,
        on_load_more: impl Fn(usize, bool, bool, &ScrollWheelEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.graph_scroll_handle = scroll_handle;
        self.graph_rendered_range = rendered_range;
        self.on_load_more = Some(std::sync::Arc::new(on_load_more));
        self
    }

    pub fn graph_layout(mut self, layout: std::sync::Arc<GraphLayout>) -> Self {
        self.graph_layout = Some(layout);
        self
    }
}

impl RenderOnce for ProjectSidebar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        let close = self.on_close;
        let snapshot = self.snapshot;
        let changes_callback = self.on_select_changes;
        let graph_callback = self.on_select_graph;
        let menu_open = self.menu_open;
        let pending_confirmation = self.pending_confirmation;
        let feedback = self.feedback;
        let action_callback = self.on_action;
        let confirm_action_callback = self.on_confirm_action;
        let toggle_menu_callback = self.on_toggle_menu;
        let cancel_confirmation_callback = self.on_cancel_confirmation;
        let graph_scroll_handle = self.graph_scroll_handle;
        let graph_rendered_range = self.graph_rendered_range;
        let graph_layout = self.graph_layout;
        let load_more_callback = self.on_load_more;

        div()
            .id("project-context-sidebar")
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(self.width))
            .h_full()
            .bg(theme.sidebar_bg)
            .border_l_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(42.))
                    .px(px(14.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .child(tab_button(theme, "Changes", ProjectPanelTab::Changes, self.active_tab, changes_callback.as_ref()))
                            .child(tab_button(theme, "Graph", ProjectPanelTab::Graph, self.active_tab, graph_callback.as_ref())),
                    )
                    .child(
                        div()
                            .id("project-context-close")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(24.))
                            .h(px(24.))
                            .rounded(px(4.))
                            .hover(|style| style.bg(theme.hover))
                            .cursor(CursorStyle::PointingHand)
                            .when_some(close, |button, on_close| {
                                button.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                                    on_close(event, window, cx);
                                })
                            })
                            .child(icons::render_icon(IconType::X, theme.muted_strong, 12.0)),
                    ),
            )
            .when(self.active_tab == ProjectPanelTab::Changes, |panel| {
                panel.child(render_changes(theme, &snapshot, feedback, action_callback.as_ref(), toggle_menu_callback.as_ref()))
            })
            .when(self.active_tab == ProjectPanelTab::Graph, |panel| {
                panel.child(render_graph(theme, &snapshot, graph_scroll_handle.clone(), graph_rendered_range.clone(), graph_layout.as_deref(), load_more_callback.as_ref()))
            })
            .when(self.active_tab == ProjectPanelTab::Changes && menu_open, |panel| {
                panel.child(div().id("project-panel-menu-overlay").absolute().top(px(84.)).right(px(8.)).occlude()
                    .child(action_menu(theme, action_callback.as_ref())))
            })
            .when_some(pending_confirmation, |panel, action| {
                panel.child(render_confirmation(theme, action, confirm_action_callback.as_ref(), cancel_confirmation_callback.as_ref()))
            })
    }
}

fn tab_button(
    theme: Theme,
    label: &'static str,
    tab: ProjectPanelTab,
    active: ProjectPanelTab,
    callback: Option<&TabCallback>,
) -> impl IntoElement {
    let is_active = active == tab;
    div()
        .id(match tab { ProjectPanelTab::Changes => "project-panel-changes-tab", ProjectPanelTab::Graph => "project-panel-graph-tab" })
        .flex()
        .items_center()
        .h_full()
        .border_b_2()
        .border_color(if is_active { theme.accent } else { theme.sidebar_bg })
        .text_size(px(12.))
        .font_weight(if is_active { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
        .text_color(if is_active { theme.foreground } else { theme.muted_strong })
        .cursor(CursorStyle::PointingHand)
        .when_some(callback.map(std::sync::Arc::clone), |button, callback| {
            button.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                callback(event, window, cx);
            })
        })
        .child(label)
}

fn render_changes(
    theme: Theme,
    snapshot: &GitPanelSnapshot,
    feedback: Option<String>,
    action_callback: Option<&ActionCallback>,
    toggle_menu_callback: Option<&SimpleCallback>,
) -> impl IntoElement {
    let mut content = div().id("project-panel-changes-content").flex().flex_col().flex_1().overflow_y_scroll().w_full().px(px(12.)).pt(px(12.)).gap(px(12.));
    content = content.child(
        div().flex().items_center().justify_between()
            .child(action_button(theme, "Stage All", ProjectGitAction::StageAll, action_callback, true))
                .child(simple_button(theme, "···", toggle_menu_callback)),
    );
    if let Some(feedback) = feedback {
        content = content.child(div().text_size(px(10.)).text_color(theme.muted_strong).child(SharedString::from(feedback)));
    }
    if snapshot.changes.is_empty() {
        let message = empty_changes_message(snapshot);
        return content.child(
            div().py(px(28.)).text_size(px(12.)).text_color(theme.muted_strong).text_center().child(message),
        );
    }
    if snapshot.changes_truncated {
        content = content.child(
            div().text_size(px(10.)).text_color(theme.muted_strong).child(SharedString::from(format!("Change list incomplete · {} rows shown", snapshot.changes.len()))),
        );
    }
    let staged: Vec<_> = snapshot.changes.iter().filter_map(|item| item.index.as_ref().map(|side| (item, side))).collect();
    let changes: Vec<_> = snapshot.changes.iter().filter_map(|item| item.worktree.as_ref().filter(|side| side.status != GitFileStatus::Untracked).map(|side| (item, side))).collect();
    let untracked: Vec<_> = snapshot.changes.iter().filter_map(|item| item.worktree.as_ref().filter(|side| side.status == GitFileStatus::Untracked).map(|side| (item, side))).collect();
    for (label, items) in [("STAGED", staged), ("CHANGES", changes), ("UNTRACKED", untracked)] {
        if items.is_empty() { continue; }
        content = content.child(
            div().flex().flex_col().gap(px(3.))
                .child(div().flex().justify_between().px(px(2.)).pb(px(3.))
                    .child(div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.muted).child(label))
                    .child(div().text_size(px(10.)).text_color(theme.muted).child(items.len().to_string())))
                .children(items.into_iter().enumerate().map(|(index, (item, side))| {
                    let staged = side.status != GitFileStatus::Untracked && label == "STAGED";
                    change_row(theme, item, side, index, staged, action_callback)
                })),
        );
    }
    content
}

fn empty_changes_message(snapshot: &GitPanelSnapshot) -> &'static str {
    if snapshot.changes_truncated { return "Change list incomplete"; }
    match snapshot.availability {
        GitPanelAvailability::Ready => "Working tree clean",
        GitPanelAvailability::Loading => "Loading Git status",
        GitPanelAvailability::NoProject => "No project folder",
        GitPanelAvailability::NoRepository => "No Git repository",
        GitPanelAvailability::FetchFailed => "Git status unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, parents: &[&str]) -> GitCommit {
        GitCommit {
            id: id.to_string(),
            hash: id.chars().take(7).collect(),
            subject: id.to_string(),
            author: "Author".to_string(),
            date: "today".to_string(),
            refs: Vec::new(),
            parents: parents.iter().map(|parent| parent.to_string()).collect(),
            parents_truncated: false,
        }
    }

    #[test]
    fn project_panel_truncated_empty_snapshot_never_reports_clean_tree() {
        let snapshot = GitPanelSnapshot { availability: GitPanelAvailability::Ready, changes_truncated: true, ..GitPanelSnapshot::default() };
        assert_eq!(empty_changes_message(&snapshot), "Change list incomplete");
        assert_ne!(empty_changes_message(&snapshot), "Working tree clean");
    }

    #[test]
    fn project_panel_graph_maps_linear_history_to_one_lane() {
        let commits = vec![commit("a", &["b"]), commit("b", &["c"]), commit("c", &[])];
        let rows = map_commit_lanes(&commits).expect("complete linear graph");

        assert_eq!(rows.iter().map(|row| row.node_lane).collect::<Vec<_>>(), vec![0, 0, 0]);
        assert!(rows.iter().all(|row| row.connections.is_empty()));
        assert_eq!(rows[0].bottom_lanes, vec![0]);
        assert_eq!(rows[2].top_lanes, vec![0]);
    }

    #[test]
    fn project_panel_graph_maps_fork_and_merge_connections() {
        let commits = vec![
            commit("merge", &["left", "right"]),
            commit("left", &["base"]),
            commit("right", &["base"]),
            commit("base", &["root"]),
            commit("root", &[]),
        ];
        let rows = map_commit_lanes(&commits).expect("complete fork and merge graph");

        assert!(rows[0].connections.contains(&(0, 1)));
        assert!(rows[2].bottom_lanes.contains(&1));
        assert!(rows[3].connections.contains(&(1, 0)));
        assert_eq!(rows[4].node_lane, 0);
    }

    #[test]
    fn project_panel_graph_simplifies_incomplete_topology_without_edges() {
        let mut incomplete = vec![commit("head", &["parent"]), commit("parent", &[])];
        incomplete[0].parents_truncated = true;

        for commits in [incomplete, {
            let parents = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
            let mut commits = vec![commit("head", &parents)];
            commits.extend(parents.iter().map(|parent| commit(parent, &[])));
            commits
        }] {
            let (rows, simplified) = graph_layout_or_simplified(&commits);
            assert!(simplified);
            assert!(rows.iter().all(|row| {
                row.top_lanes.is_empty()
                    && row.bottom_lanes.is_empty()
                    && row.full_lanes.is_empty()
                    && row.connections.is_empty()
            }));
        }
    }

    #[test]
    fn project_panel_file_checkbox_toggles_staged_state() {
        assert_eq!(stage_toggle_action("src/main.rs", false), ProjectGitAction::StagePath("src/main.rs".to_string()));
        assert_eq!(stage_toggle_action("src/main.rs", true), ProjectGitAction::UnstagePath("src/main.rs".to_string()));
    }

    #[test]
    fn project_panel_graph_lane_colors_repeat_deterministically_after_palette() {
        let slots: Vec<_> = (0..8).map(|lane| lane_color_slot(lane, 4)).collect();
        assert_eq!(slots, vec![0, 1, 2, 3, 0, 1, 2, 3]);
        assert_eq!(lane_color_slot(2, 4), lane_color_slot(2, 4));
    }

    #[test]
    fn project_panel_graph_virtualizes_to_viewport_with_overscan() {
        let range = graph_visible_range(2_000, 10.0 + 500.0 * GRAPH_ROW_HEIGHT, 10.0 * GRAPH_ROW_HEIGHT, 0.0);

        assert_eq!(range, 495..515);
        assert!(range.len() < 2_000);
        assert_eq!(graph_visible_range(4, 0.0, 500.0, 0.0), 0..4);
    }

    #[test]
    fn project_panel_graph_layout_cache_reuses_topology_and_invalidates_on_parent_change() {
        let commits = vec![commit("head", &["base"]), commit("base", &[])];
        let mut cache = None;
        let first = cached_graph_layout(&commits, &mut cache);
        let same_topology = vec![commit("head", &["base"]), commit("base", &[])];
        let second = cached_graph_layout(&same_topology, &mut cache);
        assert!(std::sync::Arc::ptr_eq(&first, &second));

        let changed_topology = vec![commit("head", &["other"]), commit("base", &[])];
        let third = cached_graph_layout(&changed_topology, &mut cache);
        assert!(!std::sync::Arc::ptr_eq(&second, &third));
    }
}

fn change_row(theme: Theme, item: &GitFileChange, side: &GitFileSide, index: usize, staged: bool, callback: Option<&ActionCallback>) -> impl IntoElement {
    let (icon, color) = match side.status {
        GitFileStatus::Added => ("A", theme.green),
        GitFileStatus::Modified => ("M", theme.yellow),
        GitFileStatus::Untracked => ("?", theme.green),
        GitFileStatus::Deleted => ("D", theme.red),
        GitFileStatus::Renamed => ("R", theme.accent),
        GitFileStatus::Conflicted => ("!", theme.red),
    };
    let name = item.path.rsplit('/').next().unwrap_or(&item.path);
    let parent = item.path.rsplit_once('/').map(|(parent, _)| parent);
    div()
        .id(SharedString::from(format!("project-change-{:?}-{index}-{}", side.status, item.path)))
        .flex()
        .items_center()
        .gap(px(8.))
        .py(px(4.))
        .child(div().w(px(14.)).text_size(px(10.)).font_weight(FontWeight::BOLD).text_color(color).child(icon))
        .child(div().flex().flex_col().flex_grow(1.).min_w(px(0.)).child(div().text_size(px(12.)).text_color(theme.foreground).truncate().child(SharedString::from(name.to_string()))).when_some(parent, |row, parent| row.child(div().text_size(px(10.)).text_color(theme.muted).truncate().child(SharedString::from(parent.to_string())))))
        .child(div().flex().gap(px(4.)).text_size(px(10.))
            .when(side.added > 0, |counts| counts.child(div().text_color(theme.green).child(SharedString::from(format!("+{}", side.added)))))
            .when(side.removed > 0, |counts| counts.child(div().text_color(theme.red).child(SharedString::from(format!("-{}", side.removed))))),
        )
        .child(action_button(theme, if staged { "☑" } else { "□" }, stage_toggle_action(&item.path, staged), callback, false))
}

fn stage_toggle_action(path: &str, staged: bool) -> ProjectGitAction {
    if staged { ProjectGitAction::UnstagePath(path.to_string()) } else { ProjectGitAction::StagePath(path.to_string()) }
}

fn lane_color_slot(lane: usize, palette_len: usize) -> usize {
    lane % palette_len
}

fn simple_button(theme: Theme, label: &'static str, callback: Option<&SimpleCallback>) -> impl IntoElement {
    div().flex().items_center().justify_center().px(px(8.)).h(px(25.)).rounded(px(4.))
        .hover(|style| style.bg(theme.hover)).text_size(px(11.)).text_color(theme.muted_strong)
        .cursor(CursorStyle::PointingHand)
        .when_some(callback.map(std::sync::Arc::clone), |button, callback| button.on_mouse_down(MouseButton::Left, move |event, window, cx| callback(event, window, cx)))
        .child(label)
}

fn action_button(theme: Theme, label: &'static str, action: ProjectGitAction, callback: Option<&ActionCallback>, primary: bool) -> impl IntoElement {
    div().flex().items_center().justify_center().px(px(8.)).h(px(25.)).rounded(px(4.))
        .bg(if primary { theme.selected } else { theme.sidebar_bg })
        .hover(|style| style.bg(theme.hover)).text_size(px(10.)).text_color(if primary { theme.foreground } else { theme.muted_strong })
        .cursor(CursorStyle::PointingHand)
        .when_some(callback.map(std::sync::Arc::clone), |button, callback| {
            let action = action.clone();
            button.on_mouse_down(MouseButton::Left, move |event, window, cx| callback(&action, event, window, cx))
        })
        .child(label)
}

fn action_menu(theme: Theme, callback: Option<&ActionCallback>) -> impl IntoElement {
    let actions = [
        ProjectGitAction::UnstageAll,
        ProjectGitAction::RestoreAllChanges,
        ProjectGitAction::StashAll,
        ProjectGitAction::StashTracked,
        ProjectGitAction::StashPop,
        ProjectGitAction::ViewStash,
        ProjectGitAction::DiscardTrackedChanges,
        ProjectGitAction::TrashUntrackedFiles,
    ];
    div().w(px(190.)).p(px(5.)).rounded(px(6.))
        .bg(theme.surface).border_1().border_color(theme.border).shadow_xl().flex().flex_col().gap(px(2.))
        .children(actions.into_iter().map(|action| {
            let label = action.label();
            div().id(SharedString::from(format!("project-action-{}", label.to_lowercase().replace(' ', "-"))))
                .w_full().px(px(8.)).py(px(6.)).rounded(px(3.)).hover(|style| style.bg(theme.hover))
                .text_size(px(11.)).text_color(theme.foreground).cursor(CursorStyle::PointingHand)
                .when_some(callback.map(std::sync::Arc::clone), |button, callback| {
                    let action = action.clone();
                    button.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        callback(&action, event, window, cx);
                    })
                })
                .child(label)
        }))
}

fn render_confirmation(theme: Theme, action: ProjectGitAction, on_confirm: Option<&ActionCallback>, on_cancel: Option<&SimpleCallback>) -> impl IntoElement {
    let warning = action.confirmation_message().unwrap_or("Confirm this Git action.");
    div().id("project-git-confirmation").absolute().inset_0().flex().items_center().justify_center().bg(theme.black.opacity(0.35))
        .child(div().w(px(250.)).p(px(14.)).rounded(px(8.)).bg(theme.surface).border_1().border_color(theme.border).shadow_xl()
            .flex().flex_col().gap(px(12.))
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.foreground).child(action.label()))
            .child(div().text_size(px(11.)).text_color(theme.muted_strong).child(warning))
            .child(div().flex().justify_end().gap(px(8.))
                .child(simple_button(theme, "Cancel", on_cancel))
                .child(action_button(theme, "Continue", action, on_confirm, true))))
}

fn render_graph(theme: Theme, snapshot: &GitPanelSnapshot, scroll_handle: ScrollHandle, rendered_range: std::rc::Rc<std::cell::Cell<(usize, usize)>>, layout: Option<&GraphLayout>, on_load_more: Option<&LoadMoreCallback>) -> impl IntoElement {
    if snapshot.commits.is_empty() {
        let message = match snapshot.availability {
            GitPanelAvailability::Ready => "No commit history",
            GitPanelAvailability::Loading => "Loading commit history",
            GitPanelAvailability::NoProject => "No project folder",
            GitPanelAvailability::NoRepository => "No Git repository",
            GitPanelAvailability::FetchFailed => "Git history unavailable",
        };
        return div().id("project-panel-empty-graph").flex().flex_col().w_full().px(px(12.)).pt(px(28.)).text_size(px(12.)).text_color(theme.muted_strong).text_center().child(message);
    }
    let fallback_layout;
    let layout = if let Some(layout) = layout {
        layout
    } else {
        fallback_layout = GraphLayout::new(&snapshot.commits);
        &fallback_layout
    };
    let lane_colors = graph_lane_colors(theme);
    let visible_commit_count = snapshot.commits.len();
    let simplified = layout.simplified;
    let viewport_height = scroll_handle.bounds().size.height.as_f32();
    let scroll_offset = -scroll_handle.offset().y.as_f32();
    let range = graph_visible_range(visible_commit_count, scroll_offset, viewport_height, if simplified { 20.0 } else { 0.0 });
    rendered_range.set((range.start, range.end));
    div().id("project-panel-graph-content").flex().flex_col().flex_1().overflow_y_scroll().track_scroll(&scroll_handle).w_full().px(px(12.))
        .on_scroll_wheel({
            let callback = on_load_more.cloned();
            move |event, window, cx| {
                let delta_y = match event.delta { gpui::ScrollDelta::Pixels(delta) => delta.y.as_f32(), gpui::ScrollDelta::Lines(delta) => delta.y };
                let max = scroll_handle.max_offset().y.as_f32();
                let offset = -scroll_handle.offset().y.as_f32();
                let next_range = graph_visible_range(visible_commit_count, offset, scroll_handle.bounds().size.height.as_f32(), if simplified { 20.0 } else { 0.0 });
                let should_refresh_rows = rendered_range.get() != (next_range.start, next_range.end);
                if let Some(callback) = callback.as_ref() {
                    callback(visible_commit_count, delta_y < 0.0 && max - offset <= 100.0, should_refresh_rows, event, window, cx);
                    if should_refresh_rows { rendered_range.set((next_range.start, next_range.end)); }
                }
            }
        })
        .child(div().id("project-panel-graph-scroll-content").flex().flex_col().pt(px(GRAPH_TOP_PADDING))
            .when(simplified, |content| content.child(div().id("project-panel-graph-simplified").h(px(20.)).flex_shrink_0().text_size(px(10.)).text_color(theme.muted_strong).child("Simplified graph · topology incomplete")))
            .child(div().h(px(range.start as f32 * GRAPH_ROW_HEIGHT)).flex_shrink_0())
            .children(snapshot.commits[range.clone()].iter().zip(layout.rows[range.clone()].iter()).map(|(commit, row)| commit_row(theme, commit, row, layout.lane_count, &lane_colors)))
            .child(div().h(px((visible_commit_count - range.end) as f32 * GRAPH_ROW_HEIGHT)).flex_shrink_0())
        )
        .when(snapshot.history_has_more, |content| content.child(div().id("project-panel-graph-loading-more").flex_shrink_0().py(px(8.)).text_size(px(10.)).text_color(theme.muted).text_center().child("Scroll for older commits")))
}

fn graph_visible_range(total: usize, offset: f32, viewport_height: f32, header_height: f32) -> Range<usize> {
    if total == 0 { return 0..0; }
    let content_offset = (offset - GRAPH_TOP_PADDING - header_height).max(0.0);
    let first_visible = (content_offset / GRAPH_ROW_HEIGHT).floor() as usize;
    let visible_rows = (viewport_height.max(GRAPH_ROW_HEIGHT) / GRAPH_ROW_HEIGHT).ceil() as usize;
    let start = first_visible.saturating_sub(GRAPH_OVERSCAN_ROWS).min(total);
    let end = first_visible.saturating_add(visible_rows).saturating_add(GRAPH_OVERSCAN_ROWS).min(total);
    start..end.max(start)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphRowLayout {
    node_lane: usize,
    lane_count: usize,
    top_lanes: Vec<usize>,
    bottom_lanes: Vec<usize>,
    full_lanes: Vec<usize>,
    connections: Vec<(usize, usize)>,
}

#[derive(Debug)]
struct GraphEdge {
    row: usize,
    node_lane: usize,
    path_lane: usize,
    target_row: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphTopologyKey {
    id: String,
    parents: Vec<String>,
    parents_truncated: bool,
}

#[derive(Debug, Clone)]
pub struct GraphLayout {
    topology: Vec<GraphTopologyKey>,
    rows: Vec<GraphRowLayout>,
    lane_count: usize,
    simplified: bool,
}

impl GraphLayout {
    fn new(commits: &[GitCommit]) -> Self {
        let (rows, simplified) = graph_layout_or_simplified(commits);
        let lane_count = rows.first().map(|row| row.lane_count).unwrap_or(1);
        Self {
            topology: commits.iter().map(|commit| GraphTopologyKey {
                id: commit.id.clone(),
                parents: commit.parents.clone(),
                parents_truncated: commit.parents_truncated,
            }).collect(),
            rows,
            lane_count,
            simplified,
        }
    }

    fn matches(&self, commits: &[GitCommit]) -> bool {
        self.topology.len() == commits.len() && self.topology.iter().zip(commits).all(|(key, commit)| {
            key.id == commit.id && key.parents == commit.parents && key.parents_truncated == commit.parents_truncated
        })
    }
}

pub fn cached_graph_layout(commits: &[GitCommit], cache: &mut Option<std::sync::Arc<GraphLayout>>) -> std::sync::Arc<GraphLayout> {
    if cache.as_ref().is_none_or(|layout| !layout.matches(commits)) {
        *cache = Some(std::sync::Arc::new(GraphLayout::new(commits)));
    }
    cache.as_ref().expect("graph layout cache initialized").clone()
}

fn map_commit_lanes(commits: &[GitCommit]) -> Option<Vec<GraphRowLayout>> {
    if commits.iter().any(|commit| commit.id.is_empty() || commit.parents_truncated) {
        return None;
    }
    let row_by_id: std::collections::HashMap<_, _> = commits.iter().enumerate()
        .map(|(index, commit)| (commit.id.as_str(), index)).collect();
    if row_by_id.len() != commits.len() {
        return None;
    }

    let mut lanes = vec![None::<String>];
    let mut edges = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    for (row_index, commit) in commits.iter().enumerate() {
        let incoming: Vec<_> = lanes.iter().enumerate()
            .filter_map(|(lane, expected)| (expected.as_deref() == Some(commit.id.as_str())).then_some(lane))
            .collect();
        let node_lane = incoming.first().copied().or_else(|| lanes.iter().position(Option::is_none)).unwrap_or_else(|| {
            lanes.push(None);
            lanes.len() - 1
        });
        for lane in incoming {
            lanes[lane] = None;
        }

        let mut row_edges = Vec::new();
        for (parent_index, parent) in commit.parents.iter().enumerate() {
            let path_lane = if parent_index == 0 {
                node_lane
            } else if let Some(free_lane) = lanes.iter().position(Option::is_none) {
                free_lane
            } else {
                lanes.push(None);
                lanes.len() - 1
            };
            lanes[path_lane] = Some(parent.clone());
            let target_row = row_by_id.get(parent.as_str()).copied();
            if target_row.is_some_and(|target| target <= row_index) {
                return None;
            }
            row_edges.push(GraphEdge { row: row_index, node_lane, path_lane, target_row });
        }
        if commit.parents.is_empty() {
            lanes[node_lane] = None;
        }
        if lanes.len() > GRAPH_LANE_LIMIT {
            return None;
        }
        rows.push(GraphRowLayout {
            node_lane,
            lane_count: 1,
            top_lanes: Vec::new(),
            bottom_lanes: Vec::new(),
            full_lanes: Vec::new(),
            connections: row_edges.iter().filter_map(|edge| {
                (edge.node_lane != edge.path_lane).then_some((edge.node_lane, edge.path_lane))
            }).collect(),
        });
        edges.extend(row_edges);
    }

    for edge in edges {
        if let Some(target_row) = edge.target_row {
            if !rows[target_row].top_lanes.contains(&edge.path_lane) {
                rows[target_row].top_lanes.push(edge.path_lane);
            }
            let target_lane = rows[target_row].node_lane;
            if target_lane != edge.path_lane {
                rows[target_row].connections.push((edge.path_lane, target_lane));
            }
            for row in rows.iter_mut().take(target_row).skip(edge.row + 1) {
                if !row.full_lanes.contains(&edge.path_lane) {
                    row.full_lanes.push(edge.path_lane);
                }
            }
        } else {
            for row in rows.iter_mut().skip(edge.row + 1) {
                if !row.full_lanes.contains(&edge.path_lane) {
                    row.full_lanes.push(edge.path_lane);
                }
            }
        }
        if !rows[edge.row].bottom_lanes.contains(&edge.path_lane) {
            rows[edge.row].bottom_lanes.push(edge.path_lane);
        }
    }
    let lane_count = lanes.len().max(1);
    for row in &mut rows {
        row.lane_count = lane_count;
    }
    Some(rows)
}

fn graph_layout_or_simplified(commits: &[GitCommit]) -> (Vec<GraphRowLayout>, bool) {
    if let Some(rows) = map_commit_lanes(commits) {
        (rows, false)
    } else {
        (disconnected_graph(commits.len()), true)
    }
}

fn disconnected_graph(total: usize) -> Vec<GraphRowLayout> {
    (0..total).map(|_| GraphRowLayout {
        node_lane: 0,
        lane_count: 1,
        top_lanes: Vec::new(),
        bottom_lanes: Vec::new(),
        full_lanes: Vec::new(),
        connections: Vec::new(),
    }).collect()
}

fn graph_lane_colors(theme: Theme) -> Vec<gpui::Hsla> {
    vec![theme.accent, theme.blue, theme.green, theme.yellow, theme.red, theme.cyan, theme.bright_blue, theme.bright_cyan]
}

fn lane_color(colors: &[gpui::Hsla], lane: usize) -> gpui::Hsla {
    colors[lane_color_slot(lane, colors.len())]
}

fn commit_row(theme: Theme, commit: &GitCommit, row: &GraphRowLayout, lane_count: usize, colors: &[gpui::Hsla]) -> gpui::Div {
    let graph = div().relative().flex_shrink_0().w(px(lane_count as f32 * GRAPH_LANE_WIDTH)).h(px(52.))
        .children(row.top_lanes.iter().map(|lane| graph_vertical(*lane, true, false, lane_color(colors, *lane))))
        .children(row.bottom_lanes.iter().map(|lane| graph_vertical(*lane, false, true, lane_color(colors, *lane))))
        .children(row.full_lanes.iter().map(|lane| graph_vertical(*lane, true, true, lane_color(colors, *lane))))
        .children(row.connections.iter().filter_map(|(from, to)| {
            let left = from.min(to);
            let width = from.abs_diff(*to) as f32 * GRAPH_LANE_WIDTH;
            (width > 0.0).then(|| div().absolute().left(px(*left as f32 * GRAPH_LANE_WIDTH + GRAPH_LANE_WIDTH / 2.0)).top(px(25.5)).w(px(width)).h(px(1.)).bg(lane_color(colors, *from)))
        }))
        .child(div().absolute().left(px(row.node_lane as f32 * GRAPH_LANE_WIDTH + 3.5)).top(px(22.5)).w(px(7.)).h(px(7.)).rounded(px(4.)).bg(lane_color(colors, row.node_lane)));
    let refs = commit.refs.iter().map(|name| div().px(px(4.)).rounded(px(3.)).bg(theme.selected).text_size(px(9.)).text_color(theme.accent).child(SharedString::from(name.clone())));
    let details = format!("{} · {}", commit.author, commit.date);
    let text = div()
        .flex()
        .flex_col()
        .flex_grow(1.)
        .min_w(px(0.))
        .gap(px(2.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .children(refs)
                .child(div().text_size(px(10.)).text_color(theme.muted).child(SharedString::from(commit.hash.clone()))),
        )
        .child(div().text_size(px(12.)).text_color(theme.foreground).truncate().child(SharedString::from(commit.subject.clone())))
        .child(div().text_size(px(10.)).text_color(theme.muted_strong).truncate().child(SharedString::from(details)));
    div().flex().gap(px(6.)).h(px(GRAPH_ROW_HEIGHT)).flex_shrink_0().child(graph).child(text)
}

fn graph_vertical(lane: usize, top: bool, bottom: bool, color: gpui::Hsla) -> gpui::Div {
    let mut line = div().absolute().left(px(lane as f32 * GRAPH_LANE_WIDTH + GRAPH_LANE_WIDTH / 2.0)).w(px(1.)).bg(color);
    line = if top { line.top(px(0.)) } else { line.top(px(26.)) };
    line = if bottom { line.bottom(px(0.)) } else { line.bottom(px(26.)) };
    line
}
