//! Command-palette preview state.
//!
//! The palette previews theme, font size and tab layout without writing the
//! config. This type owns both the open flag and the snapshot, so a closed
//! palette cannot keep a preview in flight. `close` is the only way to clear
//! the open flag and it always hands the snapshot back for restoration.

use crate::config::TabLayout;

/// Snapshot of the visual settings a palette preview may touch.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewState {
    pub theme_name: String,
    pub font_size: f32,
    pub tab_layout: TabLayout,
    pub sidebar_open: bool,
}

#[derive(Debug, Default)]
pub struct PaletteSession {
    open: bool,
    original: Option<PreviewState>,
}

impl PaletteSession {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the palette. A preview from the previous session must be gone:
    /// `close` is the only path that leaves the palette, and it restores it.
    pub fn open(&mut self) {
        debug_assert!(
            self.original.is_none(),
            "a palette preview survived the previous close"
        );
        self.open = true;
    }

    /// Starts a preview. Keeps the first snapshot so repeated steps always
    /// apply to the pre-preview state.
    pub fn begin(&mut self, current: PreviewState) {
        if self.original.is_none() {
            self.original = Some(current);
        }
    }

    pub fn is_previewing(&self) -> bool {
        self.original.is_some()
    }

    /// Baseline for relative previews. It is never the live value, which may
    /// already carry an uncommitted preview.
    pub fn base_font_size(&self, configured: f32) -> f32 {
        self.original
            .as_ref()
            .map_or(configured, |state| state.font_size)
    }

    /// Ends a preview and returns the state to restore. Keeps the palette open.
    pub fn revert(&mut self) -> Option<PreviewState> {
        self.original.take()
    }

    /// Accepts the previewed state. The caller writes the new value to config.
    pub fn commit(&mut self) {
        self.original = None;
    }

    /// Closes the palette and returns the state to restore, if any.
    pub fn close(&mut self) -> Option<PreviewState> {
        self.open = false;
        self.original.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(font_size: f32) -> PreviewState {
        PreviewState {
            theme_name: "default".to_string(),
            font_size,
            tab_layout: TabLayout::Horizontal,
            sidebar_open: false,
        }
    }

    #[test]
    fn close_hands_back_the_snapshot_so_the_caller_restores_it() {
        let mut session = PaletteSession::default();
        session.open();
        session.begin(state(16.0));

        let restored = session.close().expect("close must restore the snapshot");

        assert_eq!(restored.font_size, 16.0);
        assert!(!session.is_open());
        assert!(!session.is_previewing());
    }

    #[test]
    fn preview_after_a_leak_free_close_starts_from_the_configured_size() {
        let mut session = PaletteSession::default();
        session.open();
        session.begin(state(16.0));
        let first_preview = session.base_font_size(16.0) + 1.0;
        assert_eq!(first_preview, 17.0);

        let restored = session.close().expect("close restores the snapshot");

        session.open();
        session.begin(state(restored.font_size));
        let second_preview = session.base_font_size(16.0) + 1.0;
        assert_eq!(second_preview, 17.0, "the leaked preview must not stack");
    }

    #[test]
    fn repeated_preview_steps_keep_the_first_baseline() {
        let mut session = PaletteSession::default();
        session.open();
        session.begin(state(16.0));
        session.begin(state(17.0));

        assert_eq!(session.base_font_size(16.0), 16.0);
    }

    #[test]
    fn commit_keeps_the_palette_open_and_drops_the_snapshot() {
        let mut session = PaletteSession::default();
        session.open();
        session.begin(state(16.0));
        session.commit();

        assert!(session.is_open());
        assert!(!session.is_previewing());
        assert_eq!(session.base_font_size(18.0), 18.0);
    }

    #[test]
    fn revert_keeps_the_palette_open_for_the_next_row() {
        let mut session = PaletteSession::default();
        session.open();
        session.begin(state(16.0));
        let restored = session.revert().expect("revert restores the snapshot");

        assert_eq!(restored.font_size, 16.0);
        assert!(session.is_open());
        assert!(!session.is_previewing());
    }

    #[test]
    fn close_without_preview_has_nothing_to_restore() {
        let mut session = PaletteSession::default();
        session.open();

        assert!(session.close().is_none());
        assert!(!session.is_open());
    }
}
