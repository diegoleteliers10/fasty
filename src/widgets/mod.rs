//! Bottombar widget system.
//!
//! Composable per-window status widgets. The bar layout lives in main.rs and
//! gets walked once per frame; the renderer just consumes pre-laid-out
//! segments. Per-widget polling is driven by the `AboutToWait` tick.
//!
//! ## Adding a widget
//!
//! 1. Add a variant to [`WidgetSpec`] in `config.rs` (tag = "kebab-case").
//! 2. Implement the [`Widget`] trait in `widgets/builtin/`.
//! 3. Wire the spec → widget conversion in [`build`].

pub mod builtin;
pub mod proc_util;

use std::path::Path;
use std::time::{Duration, Instant};

use crate::config::WidgetSpec;
use crate::git::GitStatus;

/// Side of the bottombar a widget anchors to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Right,
}

/// Per-frame context handed to widgets on render and click.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct WidgetContext<'a> {
    pub active_tab_cwd: Option<&'a Path>,
    pub active_tab_git: Option<&'a GitStatus>,
    pub opacity: f32,
    /// False when the window is not the OS-active one. Network widgets use
    /// this to pause polling and keep cached state instead of burning
    /// CPU/quota on a bar nobody is looking at.
    pub window_focused: bool,
}

/// One piece of a status bar segment.
///
/// Icons are modelled rather than typed as a glyph in a string, so a segment
/// cannot smuggle in an emoji that renders differently per platform. A glyph
/// in a string is drawn by the system font; an icon is drawn by the same SVG
/// path on macOS, Windows and Linux.
#[derive(Debug, Clone)]
pub enum SegmentPart {
    Text(String),
    Icon(icons::common::IconType),
}

/// A run of text and icons with one color, optionally carrying a hover tooltip.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Segment {
    pub parts: Vec<SegmentPart>,
    pub color: [f32; 4],
    pub tooltip: Option<String>,
    /// Set by the git widget on its branch segment, so the status bar knows
    /// which segment opens the git menu. This is an explicit flag rather than a
    /// substring match on the display text, because a segment can now be built
    /// from icons alone and would have no text to match.
    pub opens_git_menu: bool,
}

impl Segment {
    pub fn text(text: impl Into<String>, color: [f32; 4]) -> Self {
        Self {
            parts: vec![SegmentPart::Text(text.into())],
            color,
            tooltip: None,
            opens_git_menu: false,
        }
    }

    /// Text and icons in the order they read left to right.
    pub fn parts(parts: Vec<SegmentPart>, color: [f32; 4]) -> Self {
        assert!(
            !parts.is_empty(),
            "a segment needs at least one part to draw"
        );
        Self {
            parts,
            color,
            tooltip: None,
            opens_git_menu: false,
        }
    }

    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Marks this segment as the one that opens the git context menu.
    pub fn git_menu(mut self) -> Self {
        self.opens_git_menu = true;
        self
    }
}

/// What a widget does when clicked.
#[derive(Debug, Clone)]
pub enum ClickAction {
    None,
    CopyToClipboard(String),
    RunCommand(String),
    OpenUrl(String),
    Custom,
    ShowActionsMenu,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ContextMenuItem {
    Copy,
    Paste,
    Separator,
    NewTab,
    CloseTab,
    About,
    OpenLink,
    CopyWord,
    CopyLine,
    CdHere,
    OpenInEditor,
    OpenEmail,
    MoveToNewWindow,
    CopyHex,
    GithubActionInfo {
        label: String,
        status: String,
        url: Option<String>,
    },
    CommandItem {
        label: String,
        command: String,
        cwd: String,
    },
}

/// A single bottombar widget.
#[allow(dead_code)]
pub trait Widget: Send {
    /// Stable id for hit-test, debug logs, and config `name` field.
    fn id(&self) -> &'static str;
    /// Anchor side (left or right).
    fn align(&self) -> Align {
        Align::Left
    }
    /// Min width in px; layout uses this to decide if the widget fits.
    fn min_width(&self) -> f32 {
        16.0
    }
    /// Produce the segments that make up this widget's text.
    fn render(&mut self, ctx: &WidgetContext) -> Vec<Segment>;
    /// Refresh internal state from the world.
    fn poll(&mut self, _ctx: &WidgetContext) {}
    /// How often [`Widget::poll`] should fire.
    fn poll_interval(&self) -> Duration;
    /// Last poll timestamp (set by the layout).
    fn last_poll(&self) -> Instant;
    fn set_last_poll(&mut self, t: Instant);
    /// Optional hover tooltip for the whole widget.
    fn tooltip(&self) -> Option<String> {
        None
    }
    /// Click handler. Default: nothing.
    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction {
        ClickAction::None
    }
    /// Get custom context menu items for this widget (e.g. for ClickAction::ShowActionsMenu).
    fn get_context_menu_items(&self) -> Option<Vec<ContextMenuItem>> {
        None
    }
}

/// Axis-aligned pixel rectangle.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

/// Pre-laid-out widget data, fed into the renderer.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LaidOutWidget {
    pub widget_index: usize,
    pub rect: Rect,
    pub segments: Vec<Segment>,
    pub tooltip: Option<String>,
}

/// The full bottombar layout for one window.
pub struct BarLayout {
    pub widgets: Vec<Box<dyn Widget>>,
    /// Latest laid-out widgets (computed each frame from `widgets`).
    pub laid_out: Vec<LaidOutWidget>,
    /// Hit-rects mirroring `laid_out` for fast mouse lookup.
    pub hit_rects: Vec<Rect>,
}

impl BarLayout {
    pub fn new(widgets: Vec<Box<dyn Widget>>) -> Self {
        Self {
            widgets,
            laid_out: Vec::new(),
            hit_rects: Vec::new(),
        }
    }

    /// Build a layout from a list of widget specs. Unknown widget types are
    /// skipped with a warning; the bar degrades gracefully to a smaller set.
    pub fn from_specs(specs: &[WidgetSpec]) -> Self {
        let widgets: Vec<Box<dyn Widget>> =
            specs.iter().filter_map(|s| builtin::build(s)).collect();
        Self::new(widgets)
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        self.hit_rects.iter().position(|r| r.contains(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRANCH_COLOR: [f32; 4] = [0.85, 0.88, 0.95, 1.0];

    /// The status bar opens the git menu from an explicit flag, not from a
    /// substring of the text. A segment built only from icons has no text, and
    /// the old check would silently stop recognising it.
    #[test]
    fn git_menu_flag_does_not_depend_on_the_text() {
        let from_text = Segment::text(" ⎇ main", BRANCH_COLOR);
        assert!(!from_text.opens_git_menu);

        let from_icons = Segment::parts(
            vec![
                SegmentPart::Icon(icons::common::IconType::GitBranch),
                SegmentPart::Text("main".to_string()),
            ],
            BRANCH_COLOR,
        );
        assert!(
            !from_icons.opens_git_menu,
            "an unmarked segment must not open the git menu"
        );

        let marked = from_icons.git_menu();
        assert!(
            marked.opens_git_menu,
            "the flag survives an icon-only segment"
        );
    }

    /// Only the git widget's branch segment opens the menu, so the builder has
    /// to be opt-in rather than default.
    #[test]
    fn only_the_git_menu_builder_sets_the_flag() {
        let plain = Segment::text(" aws:default ", BRANCH_COLOR);
        assert!(!plain.opens_git_menu);
        assert!(plain.git_menu().opens_git_menu);
    }

    /// A segment must carry something to draw, or it still occupies a slot in
    /// the status bar row and shows as a gap.
    #[test]
    #[should_panic(expected = "at least one part")]
    fn a_segment_needs_at_least_one_part() {
        Segment::parts(Vec::new(), BRANCH_COLOR);
    }

    /// The git branch segment must lead with an icon, not a glyph. `⎇` is
    /// text-presentation, so a font without it renders a box or nothing at all,
    /// and the status bar would differ per platform.
    #[test]
    fn the_git_branch_segment_leads_with_an_icon() {
        let seg = Segment::parts(
            vec![
                SegmentPart::Icon(icons::common::IconType::GitBranch),
                SegmentPart::Text("main".to_string()),
            ],
            BRANCH_COLOR,
        )
        .git_menu();
        assert!(matches!(
            seg.parts[0],
            SegmentPart::Icon(icons::common::IconType::GitBranch)
        ));
        assert!(seg.opens_git_menu);
        // The branch name is still drawn, as text.
        assert!(seg
            .parts
            .iter()
            .any(|p| matches!(p, SegmentPart::Text(t) if t == "main")));
    }

    /// Text and icons alternate in the order they were given, so the renderer
    /// draws them left to right.
    #[test]
    fn parts_keep_their_order() {
        let seg = Segment::parts(
            vec![
                SegmentPart::Text("PRs:".to_string()),
                SegmentPart::Icon(icons::common::IconType::GitPullRequest),
                SegmentPart::Text("3".to_string()),
            ],
            BRANCH_COLOR,
        );
        let kinds: Vec<bool> = seg
            .parts
            .iter()
            .map(|p| matches!(p, SegmentPart::Icon(_)))
            .collect();
        assert_eq!(kinds, vec![false, true, false]);
    }
}
