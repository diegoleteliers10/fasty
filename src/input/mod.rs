//! Keyboard and mouse input handling
//!
//! Input handling utilities for terminal key events

#![allow(dead_code)]

use gpui::Modifiers;

/// Convert a GPUI key event to PTY input bytes.
///
/// When `app_cursor_keys` is true (DECCKM mode set), arrow keys and
/// home/end use Application mode sequences (ESC OA/OB/OC/OD).
pub fn key_to_bytes(key: &str, _modifiers: &Modifiers, app_cursor_keys: bool) -> Vec<u8> {
    let arrow_prefix = if app_cursor_keys { b"\x1bO" } else { b"\x1b[" };

    match key.to_ascii_lowercase().as_str() {
        // ── Control keys ──────────────────────────────────────────────────────
        "enter" | "return" | "numpadenter" => vec![0x0D],
        "tab" => vec![0x09],
        "space" => vec![b' '],
        "escape" => vec![0x1B],
        "backspace" | "deletebackward" => vec![0x7F],
        "delete" => vec![0x1B, 0x5B, 0x33, 0x7E],

        // ── Arrow keys (Normal: CSI A/B/C/D | App: ESC OA/OB/OC/OD) ──
        "arrowup" | "up" => {
            let mut v = arrow_prefix.to_vec();
            v.push(b'A');
            v
        }
        "arrowdown" | "down" => {
            let mut v = arrow_prefix.to_vec();
            v.push(b'B');
            v
        }
        "arrowright" | "right" => {
            let mut v = arrow_prefix.to_vec();
            v.push(b'C');
            v
        }
        "arrowleft" | "left" => {
            let mut v = arrow_prefix.to_vec();
            v.push(b'D');
            v
        }

        // ── Navigation (Normal: CSI H/F | App: SS3 H/F = ESC O H/F) ──
        "home" => {
            if app_cursor_keys {
                vec![0x1B, 0x4F, 0x48] // SS3 H
            } else {
                vec![0x1B, 0x5B, 0x48] // CSI H
            }
        }
        "end" => {
            if app_cursor_keys {
                vec![0x1B, 0x4F, 0x46] // SS3 F
            } else {
                vec![0x1B, 0x5B, 0x46] // CSI F
            }
        }
        "pageup" => vec![0x1B, 0x5B, 0x35, 0x7E],
        "pagedown" => vec![0x1B, 0x5B, 0x36, 0x7E],

        // ── Function keys (normal mode: CSI 1n ~) ──────────────────────────
        // Note: F1-F4 are SS3 P-Q/R/S in app mode, CSI 1n ~ in normal mode.
        // We use normal mode sequences.
        "f1" => vec![0x1B, 0x5B, 0x31, 0x31, 0x7E], // CSI 11 ~
        "f2" => vec![0x1B, 0x5B, 0x31, 0x32, 0x7E], // CSI 12 ~
        "f3" => vec![0x1B, 0x5B, 0x31, 0x33, 0x7E], // CSI 13 ~
        "f4" => vec![0x1B, 0x5B, 0x31, 0x34, 0x7E], // CSI 14 ~
        "f5" => vec![0x1B, 0x5B, 0x31, 0x35, 0x7E], // CSI 15 ~
        "f6" => vec![0x1B, 0x5B, 0x31, 0x37, 0x7E], // CSI 17 ~
        "f7" => vec![0x1B, 0x5B, 0x31, 0x38, 0x7E], // CSI 18 ~
        "f8" => vec![0x1B, 0x5B, 0x31, 0x39, 0x7E], // CSI 19 ~
        "f9" => vec![0x1B, 0x5B, 0x32, 0x30, 0x7E], // CSI 20 ~
        "f10" => vec![0x1B, 0x5B, 0x32, 0x31, 0x7E], // CSI 21 ~
        "f11" => vec![0x1B, 0x5B, 0x32, 0x33, 0x7E], // CSI 23 ~
        "f12" => vec![0x1B, 0x5B, 0x32, 0x34, 0x7E], // CSI 24 ~

        // ── Delete / Insert ─────────────────────────────────────────────────
        "insert" => vec![0x1B, 0x5B, 0x32, 0x7E],

        _ => Vec::new(),
    }
}

/// Encode a mouse event in SGR or X10 format.
///
/// SGR format: `\x1b[<Cb;Cx;CyM` (press) / `\x1b[<Cb;Cx;Cyr` (release)
/// X10 format: `\x1b[M` + three bytes (button, col+32, row+32)
///
/// `button`: 0=left, 1=middle, 2=right. Add 32 for motion, 64/65 for scroll.
/// `modifiers`: OR with Shift=4, Meta=8, Ctrl=16.
pub fn encode_mouse_event(
    kind: MouseEventKind,
    button: u8,
    col: u16,
    row: u16,
    modifiers: u8,
    sgr_mode: bool,
) -> Vec<u8> {
    let base_button = match kind {
        MouseEventKind::Press => button,
        MouseEventKind::Release => button,
        MouseEventKind::Move => button | 32,
    };

    let cb = base_button | modifiers;

    if sgr_mode {
        let suffix = match kind {
            MouseEventKind::Release => b"r",
            _ => b"M",
        };
        format!("\x1b[<{};{};{}{}", cb, col, row, unsafe {
            std::str::from_utf8_unchecked(suffix)
        })
        .into_bytes()
    } else {
        // X10 legacy — only works up to col/row 223
        let cx = (col.min(223) as u8 + 32).max(32);
        let cy = (row.min(223) as u8 + 32).max(32);
        vec![0x1B, b'M', base_button + 32, cx, cy]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventKind {
    Press,
    Release,
    Move,
}
