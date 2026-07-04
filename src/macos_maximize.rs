//! Per-window maximize that bypasses macOS's animated NSWindow zoom.
//!
//! winit's `Window::set_maximized` maps to the animated zoom on macOS, which
//! visibly jumps the window to the bottom-left before settling. On macOS we
//! instead read the current screen's `visibleFrame` and call
//! `setFrame:display:animate:NO` directly, tracking the pre-maximize frame so
//! the toggle restores it. Other platforms keep winit's `set_maximized`.

#[cfg(target_os = "macos")]
mod macos_impl {
    use objc2::encode::{Encode, Encoding, RefEncode};
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    use raw_window_handle::HasWindowHandle;
    use winit::window::Window;

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct NSPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct NSSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct NSRect {
        origin: NSPoint,
        size: NSSize,
    }

    // NSPoint/NSSize/NSRect are aliases for CGPoint/CGSize/CGRect, so the
    // runtime reports their type codes under the CG* names. objc2 0.5 verifies
    // the declared encoding against the signature and panics on mismatch.
    unsafe impl Encode for NSPoint {
        const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl RefEncode for NSPoint {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }
    unsafe impl Encode for NSSize {
        const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl RefEncode for NSSize {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }
    unsafe impl Encode for NSRect {
        const ENCODING: Encoding =
            Encoding::Struct("CGRect", &[NSPoint::ENCODING, NSSize::ENCODING]);
    }
    unsafe impl RefEncode for NSRect {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }

    #[derive(Default)]
    pub struct MaximizeState(Option<NSRect>);

    fn ns_window(window: &Window) -> Option<*mut AnyObject> {
        let raw = window.window_handle().ok()?.as_raw();
        let raw_window_handle::RawWindowHandle::AppKit(appkit) = raw else {
            return None;
        };
        let ns_view = appkit.ns_view.as_ptr() as *mut AnyObject;
        if ns_view.is_null() {
            return None;
        }
        let ns_window: *mut AnyObject = unsafe { msg_send![ns_view, window] };
        if ns_window.is_null() {
            None
        } else {
            Some(ns_window)
        }
    }

    fn current_screen(ns_window: *mut AnyObject) -> *mut AnyObject {
        let screen: *mut AnyObject = unsafe { msg_send![ns_window, screen] };
        if !screen.is_null() {
            return screen;
        }
        unsafe { msg_send![class!(NSScreen), mainScreen] }
    }

    pub fn toggle_maximize(window: &Window, state: &mut MaximizeState) {
        let is_main: Bool = unsafe { msg_send![class!(NSThread), isMainThread] };
        assert!(is_main.as_bool(), "AppKit calls must be on the main thread");
        let Some(ns_window) = ns_window(window) else {
            return;
        };

        if let Some(saved) = state.0 {
            let _: () = unsafe {
                msg_send![
                    ns_window,
                    setFrame: saved,
                    display: Bool::YES,
                    animate: Bool::NO,
                ]
            };
            state.0 = None;
            return;
        }

        let saved: NSRect = unsafe { msg_send![ns_window, frame] };
        let screen = current_screen(ns_window);
        if screen.is_null() {
            return;
        }
        let visible: NSRect = unsafe { msg_send![screen, visibleFrame] };
        let _: () = unsafe {
            msg_send![
                ns_window,
                setFrame: visible,
                display: Bool::YES,
                animate: Bool::NO,
            ]
        };
        state.0 = Some(saved);
    }
}

#[cfg(not(target_os = "macos"))]
mod other_impl {
    use winit::window::Window;

    #[derive(Default)]
    pub struct MaximizeState;

    pub fn toggle_maximize(window: &Window, _state: &mut MaximizeState) {
        let is_max = window.is_maximized();
        window.set_maximized(!is_max);
    }
}

#[cfg(target_os = "macos")]
pub use macos_impl::*;
#[cfg(not(target_os = "macos"))]
pub use other_impl::*;
