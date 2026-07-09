//! macOS-specific CAMetalLayer resize helpers.
//!
//! Setting `presentsWithTransaction = YES` on the `CAMetalLayer` that wgpu
//! creates and wrapping each resize-triggered render in an explicit
//! `CATransaction` eliminates the visual stutter ("truncated" frame) seen
//! during live window resize on macOS.
//!
//! ## Why this works
//!
//! By default, `CAMetalLayer` commits new frames to the compositor
//! asynchronously through its own internal transaction.  During live resize
//! the window geometry and the GPU frame can arrive at the compositor in
//! different transactions, so the old frame gets stretched over the new
//! frame boundary for one or more compositing ticks.
//!
//! With `presentsWithTransaction = YES` the layer waits for an explicit
//! `CATransaction commit` before handing the frame to the compositor, giving
//! the application a chance to atomically update both geometry and pixels in
//! one transaction.  The `begin_transaction` / `commit_transaction` helpers
//! here are called around the render+present for every frame that follows a
//! resize event.
//!
//! ## Thread safety
//!
//! All functions must be called from the main thread (AppKit constraint).

#[cfg(target_os = "macos")]
pub use macos_impl::*;

#[cfg(target_os = "macos")]
mod macos_impl {
    use objc2::runtime::{AnyObject, Bool};
    use objc2::{class, msg_send};
    use raw_window_handle::HasWindowHandle;

    /// Set `presentsWithTransaction = YES` on the `CAMetalLayer` backing the
    /// given window's `NSView`.
    ///
    /// Call this once for every [`winit::window::Window`] immediately after
    /// `surface.configure()` finishes in the renderer constructor.
    pub fn enable_presents_with_transaction(window: &winit::window::Window) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let raw_window_handle::RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };
        let ns_view = appkit.ns_view.as_ptr() as *mut AnyObject;
        if ns_view.is_null() {
            return;
        }

        // wgpu replaces the NSView's default layer with a CAMetalLayer; fetch it.
        let layer: *mut AnyObject = unsafe { msg_send![ns_view, layer] };
        if layer.is_null() {
            return;
        }

        // Opt in to synchronised presentation.
        let _: () = unsafe { msg_send![layer, setPresentsWithTransaction: Bool::YES] };
    }

    /// Open a `CATransaction` and disable implicit animations for the duration.
    ///
    /// Must be balanced by a call to [`commit_transaction`].
    /// Only call when there is a pending surface resize.
    pub fn begin_transaction() {
        let _: () = unsafe { msg_send![class!(CATransaction), begin] };
        // Prevent the layer from interpolating its size between the old and
        // new geometry (the "rubbery" stretch artefact).
        let _: () = unsafe { msg_send![class!(CATransaction), setDisableActions: Bool::YES] };
    }

    /// Commit the currently open `CATransaction`.
    ///
    /// This hands the rendered Metal frame to the compositor atomically with
    /// the window geometry update, eliminating the one-frame gap that causes
    /// the truncated-content artefact during live resize.
    pub fn commit_transaction() {
        let _: () = unsafe { msg_send![class!(CATransaction), commit] };
    }
}
