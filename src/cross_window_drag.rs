//! Cross-window drag protocol.
//!
//! Holds the in-flight [`Tab`] while it is being dragged from one window to
//! another. Both windows share the same winit event loop, so a `parking_lot`
//! static is sufficient — no IPC.

use parking_lot::Mutex;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::terminal_state::AppEvent;

pub static DRAG: Mutex<Option<CrossWindowDrag>> = Mutex::new(None);

pub struct CrossWindowDrag {
    pub source_window_id: WindowId,
    pub tab: crate::Tab,
    pub source_proxy: EventLoopProxy<AppEvent>,
}

pub fn take() -> Option<CrossWindowDrag> {
    DRAG.lock().take()
}

pub fn is_active() -> bool {
    DRAG.lock().is_some()
}

pub fn current_source() -> Option<WindowId> {
    DRAG.lock().as_ref().map(|d| d.source_window_id)
}
