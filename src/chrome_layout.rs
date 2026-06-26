//! Shared top-bar geometry for window chrome.
//!
//! Button rects are computed here so the renderer (draw) and the input handler
//! (hit-test) read the exact same numbers and never drift apart.
//!
//! All values are logical pixels. The top bar is 40px tall; each control button
//! is a 28x28 cell sitting at y = 6..34.
//!
//! Layout:
//! - Linux/Windows: control buttons (settings/min/max/close) right-aligned,
//!   app icon top-left.
//! - macOS: traffic-light buttons (close/min/max) on the LEFT, app icon + gear
//!   on the RIGHT; tabs start further right to clear the lights.

const CELL: f32 = 28.0;
const Y: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    const fn cell(x: f32) -> Self {
        Self { x, y: Y, w: CELL, h: CELL }
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
        Rect::cell(8.0)
    } else {
        Rect::cell(vw - 36.0)
    }
}

pub fn min_rect(vw: f32) -> Rect {
    if macos() {
        Rect::cell(40.0)
    } else {
        Rect::cell(vw - 100.0)
    }
}

pub fn max_rect(vw: f32) -> Rect {
    if macos() {
        Rect::cell(72.0)
    } else {
        Rect::cell(vw - 68.0)
    }
}

pub fn settings_rect(vw: f32) -> Rect {
    Rect::cell(vw - 137.0)
}

pub fn icon_rect(vw: f32) -> Rect {
    if macos() {
        Rect { x: vw - 26.0, y: 7.0, w: 16.0, h: 16.0 }
    } else {
        Rect { x: 8.0, y: 7.0, w: 16.0, h: 16.0 }
    }
}

pub fn tab_start_x() -> f32 {
    if macos() {
        112.0
    } else {
        36.0
    }
}

pub fn drag_max_x(vw: f32) -> f32 {
    if macos() {
        vw - 160.0
    } else {
        vw - 141.0
    }
}
