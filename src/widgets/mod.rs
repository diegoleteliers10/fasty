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

use std::path::Path;
use std::time::{Duration, Instant};

use crate::git::GitStatus;
use crate::config::WidgetSpec;

/// Side of the bottombar a widget anchors to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Right,
}

/// Per-frame context handed to widgets on render and click.
#[derive(Debug, Clone, Copy)]
pub struct WidgetContext<'a> {
    pub active_tab_cwd: Option<&'a Path>,
    pub active_tab_git: Option<&'a GitStatus>,
    pub opacity: f32,
}

/// A run of glyphs with one color, optionally carrying a hover tooltip.
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub color: [f32; 4],
    pub tooltip: Option<String>,
}

/// What a widget does when clicked.
#[derive(Debug, Clone)]
pub enum ClickAction {
    None,
    CopyToClipboard(String),
    RunCommand(String),
    OpenUrl(String),
    Custom,
}

/// A single bottombar widget.
pub trait Widget: Send {
    /// Stable id for hit-test, debug logs, and config `name` field.
    fn id(&self) -> &'static str;
    /// Anchor side (left or right).
    fn align(&self) -> Align { Align::Left }
    /// Min width in px; layout uses this to decide if the widget fits.
    fn min_width(&self) -> f32 { 16.0 }
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
    fn tooltip(&self) -> Option<String> { None }
    /// Click handler. Default: nothing.
    fn on_click(&mut self, _ctx: &WidgetContext) -> ClickAction { ClickAction::None }
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
        let widgets: Vec<Box<dyn Widget>> = specs
            .iter()
            .filter_map(|s| builtin::build(s))
            .collect();
        Self::new(widgets)
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        self.hit_rects
            .iter()
            .position(|r| r.contains(x, y))
    }

    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }
}

/// Build a `Box<dyn Widget>` from a config spec. Returns `None` on unknown
/// widget type so the rest of the bar still renders.
pub fn build(spec: &WidgetSpec) -> Option<Box<dyn Widget>> {
    builtin::build(spec)
}
