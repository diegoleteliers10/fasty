//! Shared top-bar geometry for window chrome.
//!
//! Button rects are computed here so the renderer (draw) and the input handler
//! (hit-test) read the exact same numbers and never drift apart.
//!
//! All values are logical design pixels, scaled at runtime by the window's
//! `scale_factor` (see [`set_scale`]). The pipeline draws in physical pixels
//! (`SurfaceConfiguration.width/height`), so multiplying the logical design
//! constants by the backing-store scale keeps the chrome a consistent physical
//! size across 1× and 2× (Retina) displays — otherwise the top bar and its
//! icons render at half height on Retina.
//!
//! Layout:
//! - Linux/Windows: control buttons (settings/min/max/close) right-aligned,
//!   app icon top-left.
//! - macOS: traffic-light buttons (close/min/max) on the LEFT, app icon + gear
//!   on the RIGHT; tabs start further right to clear the lights.
//!
//! `with_platform_chrome` below also applies the per-OS `WindowAttributes`
//! policy (decorations, transparent titlebar) for both the main and
//! secondary windows.

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;

use std::sync::atomic::{AtomicU32, Ordering};

/// Backing-store scale (1.0 on 1× displays, 2.0 on Retina). Stored as
/// `scale * 1000` so it can live in an `AtomicU32` without a mutex. The
/// renderer sets this once per (re)created surface so that draw and hit-test
/// agree on the same chrome size.
static CHROME_SCALE_MILLI: AtomicU32 = AtomicU32::new(1000);

/// Update the chrome scale factor. Called by the renderer whenever the window
/// surface is (re)created (initial build, font change, display change).
pub fn set_scale(scale: f32) {
    let milli = (scale * 1000.0).round().clamp(500.0, 4000.0) as u32;
    CHROME_SCALE_MILLI.store(milli, Ordering::Relaxed);
}

/// Current chrome scale factor (defaults to 1.0 until the renderer sets it).
pub fn scale() -> f32 {
    CHROME_SCALE_MILLI.load(Ordering::Relaxed) as f32 / 1000.0
}

/// Same as [`scale`] but as `f64`, for hit-testing code that works in physical
/// pixel coordinates from winit (which are `f64`).
pub fn scale_f64() -> f64 {
    CHROME_SCALE_MILLI.load(Ordering::Relaxed) as f64 / 1000.0
}

/// Bottom edge (physical px) of the tab-strip click region in the top bar.
/// Hit-testing in `main.rs` uses this to decide whether a click landed on a
/// tab vs the terminal area. Must stay in sync with the rendered top-bar
/// height (see `topbar_h` in `renderer/pipeline.rs`).
pub fn topbar_bottom_f64() -> f64 {
    40.0 * scale_f64()
}

/// Applies per-platform chrome: macOS gets a transparent, full-size titlebar
/// (native traffic lights); other platforms stay borderless.
pub fn with_platform_chrome(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    #[cfg(target_os = "macos")]
    {
        attrs
            .with_decorations(true)
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        attrs.with_decorations(false)
    }
}

/// Control-button cell: 28×28 logical design, at y = 6 from the top.
const CELL_DESIGN: f32 = 28.0;
const Y_DESIGN: f32 = 6.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn cell(x: f32) -> Self {
        let s = scale();
        Self { x, y: Y_DESIGN * s, w: CELL_DESIGN * s, h: CELL_DESIGN * s }
    }
    pub fn contains(&self, mx: f64, my: f64) -> bool {
        mx >= self.x as f64 && mx < (self.x + self.w) as f64
            && my >= self.y as f64 && my < (self.y + self.h) as f64
    }
}

const fn macos() -> bool {
    cfg!(target_os = "macos")
}

pub fn close_rect(vw: f32) -> Rect {
    if macos() {
        // System owns the traffic lights: zero-area so the hit-test never
        // matches and the top-bar drag fallback covers the gaps.
        Rect::default()
    } else {
        Rect::cell(vw - 36.0 * scale())
    }
}

pub fn min_rect(vw: f32) -> Rect {
    if macos() {
        Rect::default()
    } else {
        Rect::cell(vw - 100.0 * scale())
    }
}

pub fn max_rect(vw: f32) -> Rect {
    if macos() {
        Rect::default()
    } else {
        Rect::cell(vw - 68.0 * scale())
    }
}

pub fn settings_rect(vw: f32) -> Rect {
    if macos() {
        // Park the gear flush beside the icon (vw-137 was its slot while
        // min/max/close clustered on the right; that leftover was the gap).
        Rect::cell(vw - 64.0 * scale())
    } else {
        Rect::cell(vw - 137.0 * scale())
    }
}

/// Update button: 70px wide, 12px left of the gear. Derived from
/// `settings_rect` so draw and hit-test stay in sync.
pub fn update_rect(vw: f32) -> Rect {
    let s = scale();
    let settings = settings_rect(vw);
    // y = controls_y (1.0) + 4.0, matching the draw path's vertical offset.
    Rect { x: settings.x - 70.0 * s - 12.0 * s, y: 5.0 * s, w: 70.0 * s, h: 20.0 * s }
}

pub fn icon_rect(vw: f32) -> Rect {
    let s = scale();
    if macos() {
        Rect { x: vw - 26.0 * s, y: 7.0 * s, w: 16.0 * s, h: 16.0 * s }
    } else {
        Rect { x: 8.0 * s, y: 7.0 * s, w: 16.0 * s, h: 16.0 * s }
    }
}

pub fn tab_start_x() -> f32 {
    if macos() {
        112.0 * scale()
    } else {
        36.0 * scale()
    }
}

pub fn drag_max_x(vw: f32) -> f32 {
    if macos() {
        // 8px gap before the gear at vw-64.
        vw - 72.0 * scale()
    } else {
        vw - 141.0 * scale()
    }
}
