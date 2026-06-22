use parking_lot::Mutex;
use winit::window::WindowId;

pub static DRAG: Mutex<Option<CrossWindowDrag>> = Mutex::new(None);

#[allow(dead_code)]
pub struct CrossWindowDrag {
    pub source_window_id: WindowId,
    pub tab: crate::Tab,
}

pub fn take() -> Option<CrossWindowDrag> {
    DRAG.lock().take()
}

pub fn is_active() -> bool {
    DRAG.lock().is_some()
}
