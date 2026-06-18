//! Per-window state lifted out of the `event_loop.run` closure.
//!
//! One [`WindowContext`] per terminal-hosting window. A new window is created
//! when a tab is dragged out of an existing window. The event loop dispatches
//! `WindowEvent`s to the right context by `WindowId`.

use std::sync::Arc;

use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use crate::terminal_state::AppEvent;
use crate::widgets::BarLayout;

pub struct WindowContext {
    pub window: Arc<Window>,
    pub renderer: Arc<parking_lot::Mutex<crate::renderer::Renderer<'static>>>,
    pub tabs: Vec<crate::Tab>,
    pub active_tab_index: usize,
    pub shell_cols: usize,
    pub shell_rows: usize,
    pub cell_width: f32,
    pub cell_height: f32,
    pub dragging_tab: Option<usize>,
    pub drag_start_x: f64,
    pub drag_current_x: f64,
    pub drag_tab_offset: f64,
    pub drag_threshold_passed: bool,
    pub bar_layout: BarLayout,
    pub proxy: EventLoopProxy<AppEvent>,
}

impl WindowContext {
    pub fn new(
        window: Arc<Window>,
        renderer: Arc<parking_lot::Mutex<crate::renderer::Renderer<'static>>>,
        tabs: Vec<crate::Tab>,
        cell_width: f32,
        cell_height: f32,
        shell_cols: usize,
        shell_rows: usize,
        bar_layout: BarLayout,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Self {
        Self {
            window,
            renderer,
            tabs,
            active_tab_index: 0,
            shell_cols,
            shell_rows,
            cell_width,
            cell_height,
            dragging_tab: None,
            drag_start_x: 0.0,
            drag_current_x: 0.0,
            drag_tab_offset: 0.0,
            drag_threshold_passed: false,
            bar_layout,
            proxy,
        }
    }

    /// Drag-out is only enabled when the window has 2+ tabs. A single-tab
    /// window cannot be split — the only tab stays put.
    pub fn can_pop_out(&self) -> bool {
        self.tabs.len() >= 2
    }
}
