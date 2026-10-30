//! Kitty keyboard protocol (https://sw.kovidgoyal.net/kitty/keyboard-protocol/).
//!
//! This module owns the two halves of the protocol:
//!
//! - State: the progressive-enhancement flag stack a program controls with
//!   `CSI > u`, `CSI < u`, and `CSI = u`, plus the `CSI ? u` query.
//! - Encoding: turning a key event into the bytes the program expects,
//!   either the legacy encoding (flags = 0) or one of the CSI u forms.
//!
//! The encoder is a pure function with no GPUI or terminal types, so tests
//! can cover the full matrix without a window.

/// Disambiguate escape codes: route Esc, alt+key, ctrl+key through CSI u.
pub const FLAG_DISAMBIGUATE: u8 = 0b00001;
/// Report event types: repeat and release events carry `:2` / `:3`.
pub const FLAG_REPORT_EVENTS: u8 = 0b00010;
/// Report alternate keys: shifted key in the first sub-field.
pub const FLAG_REPORT_ALTERNATE: u8 = 0b00100;
/// Report all keys as escape codes, including plain text keys.
pub const FLAG_REPORT_ALL: u8 = 0b01000;
/// Report associated text: text as code points in the third field.
/// Only meaningful together with [`FLAG_REPORT_ALL`].
pub const FLAG_REPORT_ASSOCIATED_TEXT: u8 = 0b10000;

/// All valid flag bits. Anything else in a `CSI = flags u` request is
/// rejected so a malformed sequence cannot poison the state.
pub const ALL_FLAGS: u8 = FLAG_DISAMBIGUATE
    | FLAG_REPORT_EVENTS
    | FLAG_REPORT_ALTERNATE
    | FLAG_REPORT_ALL
    | FLAG_REPORT_ASSOCIATED_TEXT;

/// Maximum depth of the flag stack. The spec asks terminals to bound it.
pub const MAX_STACK: usize = 16;

pub const EVENT_PRESS: u8 = 1;
pub const EVENT_REPEAT: u8 = 2;
pub const EVENT_RELEASE: u8 = 3;

/// What a key maps to on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyForm {
    /// Encoded as `CSI number ; mods u` (C0 keys, text keys, PUA keys).
    Unicode(u32),
    /// Encoded as `CSI number ; mods ~`.
    Tilde(u16),
    /// Encoded as `CSI 1 ; mods <letter>`.
    Letter(u8),
}

/// The protocol state for one terminal. Separate instances must exist for
/// the main and alternate screens; fastty keeps one per `TerminalState`.
#[derive(Debug, Default)]
pub struct KittyKeyboard {
    current: u8,
    stack: Vec<u8>,
}

impl KittyKeyboard {
    pub fn flags(&self) -> u8 {
        self.current
    }

    /// `CSI > flags u`. A missing or malformed flags field means zero.
    pub fn push(&mut self, flags: u8) {
        self.stack.push(self.current);
        if self.stack.len() > MAX_STACK {
            self.stack.remove(0);
        }
        self.current = flags & ALL_FLAGS;
    }

    /// `CSI < number u`. Popping an empty stack resets all flags.
    pub fn pop(&mut self, count: u32) {
        let count = count.clamp(1, MAX_STACK as u32) as usize;
        for _ in 0..count {
            match self.stack.pop() {
                Some(flags) => self.current = flags,
                None => {
                    self.stack.clear();
                    self.current = 0;
                    break;
                }
            }
        }
    }

    /// `CSI = flags ; mode u`. Mode 1 replaces, 2 ORs, 3 resets.
    /// Unknown modes are ignored, as the spec requires.
    pub fn set(&mut self, flags: u8, mode: u32) {
        let flags = flags & ALL_FLAGS;
        self.current = match mode {
            1 => flags,
            2 => self.current | flags,
            3 => self.current & !flags,
            _ => self.current,
        };
    }
}

/// One key event, already free of windowing-system types.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent<'a> {
    /// The GPUI key name, lowercased: `"a"`, `"1"`, `"escape"`, `"f5"`…
    pub key: &'a str,
    /// The text the platform produced for this event, if any.
    pub text: Option<&'a str>,
    /// The shifted character for this key, when known.
    pub shifted: Option<&'a str>,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub super_key: bool,
    /// [`EVENT_PRESS`], [`EVENT_REPEAT`], or [`EVENT_RELEASE`].
    pub event_type: u8,
}

/// Encode one key event under the given flags.
///
/// Returns `None` when the event must be swallowed: releases without
/// [`FLAG_REPORT_EVENTS`], or keys the protocol cannot represent. Plain
/// text under a flags set that keeps text keys legacy returns the raw
/// UTF-8 bytes, exactly like the legacy encoder would.
pub fn encode(event: &KeyEvent, flags: u8) -> Option<Vec<u8>> {
    let flags = flags & ALL_FLAGS;
    // Flags zero means the program never opted in; the caller's legacy
    // encoder owns every key in that case.
    if flags == 0 {
        return None;
    }
    let is_modifier_only = matches!(
        event.key,
        "shift" | "alt" | "control" | "ctrl" | "super" | "cmd" | "platform" | "hyper" | "meta"
    );
    if is_modifier_only && flags & FLAG_REPORT_ALL == 0 {
        return None;
    }

    let release = event.event_type == EVENT_RELEASE;
    if release && flags & FLAG_REPORT_EVENTS == 0 {
        return None;
    }

    let form = functional_key(event.key);
    let csi_u_for_functional = matches!(form, Some(KeyForm::Unicode(_)));
    let is_exception_key = matches!(event.key, "enter" | "return" | "tab" | "backspace");

    // Does this event go out as a CSI sequence?
    let as_csi = if form.is_some() {
        if is_exception_key {
            // Enter, Tab, and Backspace stay legacy on press/repeat unless
            // the program asked for every key as escape codes.
            flags & FLAG_REPORT_ALL != 0 || release
        } else if csi_u_for_functional {
            // C0/PUA keys: disambiguate routes them through CSI u; without
            // it only alt+functional goes CSI u (legacy prefix form).
            flags & FLAG_DISAMBIGUATE != 0
                || flags & FLAG_REPORT_ALL != 0
                || event.alt
                || event.ctrl
        } else {
            flags & FLAG_DISAMBIGUATE != 0 || flags & FLAG_REPORT_ALL != 0 || event.alt || event.ctrl
        }
    } else {
        let text = event.text.or(Some(event.key));
        let produces_text = text.is_some_and(|text| !text.is_empty());
        if flags & FLAG_REPORT_ALL != 0 {
            true
        } else if flags & FLAG_REPORT_ASSOCIATED_TEXT != 0 {
            produces_text
        } else if flags & FLAG_DISAMBIGUATE != 0 {
            // Plain text without modifiers stays legacy. Alt, ctrl, or
            // shift+alt on an ASCII key goes CSI u.
            event.ctrl || event.alt || (event.super_key && produces_text)
        } else {
            false
        }
    };

    if !as_csi {
        // Legacy bytes cannot express release or repeat as distinct events;
        // emitting one would arrive as a phantom press of the same key.
        // Drop the event instead (kitty spec: hosts encode releases as CSI u
        // or not at all). Repeat is indistinguishable from press.
        if release {
            return None;
        }
        return legacy_bytes(event, form);
    }

    let key_number = match form {
        Some(KeyForm::Unicode(code)) => code,
        Some(_) => {
            // ~ and letter forms carry their number in the sequence itself;
            // the unicode-key-code field stays 1 per the spec's second form.
            1
        }
        None => {
            let first = event
                .key
                .chars()
                .next()
                .map(|c| c.to_ascii_lowercase() as u32)?;
            if event.shift {
                first
            } else {
                first
            }
        }
    };

    let mods = 1
        + u32::from(event.shift)
        + 2 * u32::from(event.alt)
        + 4 * u32::from(event.ctrl)
        + 8 * u32::from(event.super_key);

    let mut seq = String::from("\x1b[");
    let report_event = flags & FLAG_REPORT_EVENTS != 0 && event.event_type != EVENT_PRESS;
    let wants_params = report_event || mods != 1;
    match form {
        Some(KeyForm::Letter(letter)) => {
            // With no modifiers and no event type, the parameters are
            // omitted entirely: `CSI A`, not `CSI 1;1 A`.
            if wants_params {
                seq.push_str(&key_number.to_string());
                seq.push(';');
                seq.push_str(&mods.to_string());
                if report_event {
                    seq.push(':');
                    seq.push_str(&event.event_type.to_string());
                }
            }
            seq.push(letter as char);
            return Some(seq.into_bytes());
        }
        Some(KeyForm::Tilde(number)) => {
            seq.push_str(&number.to_string());
            if wants_params {
                seq.push(';');
                seq.push_str(&mods.to_string());
                if report_event {
                    seq.push(':');
                    seq.push_str(&event.event_type.to_string());
                }
            }
            seq.push('~');
            return Some(seq.into_bytes());
        }
        Some(KeyForm::Unicode(code)) => {
            seq.push_str(&code.to_string());
        }
        None => {
            seq.push_str(&key_number.to_string());
            // Alternate keys: shifted sub-field, only with shift held.
            if flags & FLAG_REPORT_ALTERNATE != 0 {
                if let (true, Some(shifted)) = (event.shift, event.shifted.and_then(first_codepoint))
                {
                    seq.push(':');
                    seq.push_str(&shifted.to_string());
                }
            }
        }
    }

    let text = if flags & FLAG_REPORT_ASSOCIATED_TEXT != 0 && !release {
        event.text.and_then(text_codepoints)
    } else {
        None
    };

    if mods != 1 || report_event || text.is_some() {
        seq.push(';');
        seq.push_str(&mods.to_string());
        if report_event {
            seq.push(':');
            seq.push_str(&event.event_type.to_string());
        }
        if let Some(text) = text {
            seq.push(';');
            seq.push_str(&text);
        }
    }
    seq.push('u');
    Some(seq.into_bytes())
}

fn first_codepoint(value: &str) -> Option<u32> {
    value.chars().next().map(|c| c as u32)
}

fn text_codepoints(value: &str) -> Option<String> {
    let points: Vec<String> = value.chars().map(|c| (c as u32).to_string()).collect();
    if points.is_empty() {
        None
    } else {
        Some(points.join(":"))
    }
}

/// Legacy bytes for the keys that keep them, mirroring the C0 table of
/// the kitty spec. Returns `None` for anything the legacy encoding cannot
/// express; the caller then drops the event.
fn legacy_bytes(event: &KeyEvent, form: Option<KeyForm>) -> Option<Vec<u8>> {
    let esc: &[u8] = b"\x1b";
    match form {
        Some(KeyForm::Unicode(27)) => {
            if event.alt {
                let mut out = esc.to_vec();
                out.push(0x1b);
                Some(out)
            } else {
                Some(b"\x1b".to_vec())
            }
        }
        Some(KeyForm::Unicode(13)) => {
            if event.alt || event.ctrl && false {
                let mut out = esc.to_vec();
                out.push(b'\r');
                Some(out)
            } else {
                Some(b"\r".to_vec())
            }
        }
        Some(KeyForm::Unicode(9)) => {
            if event.shift {
                Some(b"\x1b[Z".to_vec())
            } else if event.alt {
                let mut out = esc.to_vec();
                out.push(b'\t');
                Some(out)
            } else {
                Some(b"\t".to_vec())
            }
        }
        Some(KeyForm::Unicode(127)) => {
            if event.ctrl {
                Some(b"\x08".to_vec())
            } else if event.alt {
                let mut out = esc.to_vec();
                out.push(0x7f);
                Some(out)
            } else {
                Some(b"\x7f".to_vec())
            }
        }
        Some(KeyForm::Unicode(_)) | Some(KeyForm::Tilde(_)) | Some(KeyForm::Letter(_)) => None,
        None => {
            if event.alt || event.ctrl || event.super_key {
                return None;
            }
            event.text.map(|text| text.as_bytes().to_vec())
        }
    }
}

/// Map a GPUI key name to its wire form.
fn functional_key(key: &str) -> Option<KeyForm> {
    let form = match key {
        "escape" | "esc" => KeyForm::Unicode(27),
        "enter" | "return" => KeyForm::Unicode(13),
        "tab" => KeyForm::Unicode(9),
        "backspace" => KeyForm::Unicode(127),
        "insert" => KeyForm::Tilde(2),
        "delete" => KeyForm::Tilde(3),
        "pageup" => KeyForm::Tilde(5),
        "pagedown" => KeyForm::Tilde(6),
        "up" => KeyForm::Letter(b'A'),
        "down" => KeyForm::Letter(b'B'),
        "right" => KeyForm::Letter(b'C'),
        "left" => KeyForm::Letter(b'D'),
        "home" => KeyForm::Letter(b'H'),
        "end" => KeyForm::Letter(b'F'),
        "f1" => KeyForm::Letter(b'P'),
        "f2" => KeyForm::Letter(b'Q'),
        "f3" => KeyForm::Tilde(13),
        "f4" => KeyForm::Letter(b'S'),
        "f5" => KeyForm::Tilde(15),
        "f6" => KeyForm::Tilde(17),
        "f7" => KeyForm::Tilde(18),
        "f8" => KeyForm::Tilde(19),
        "f9" => KeyForm::Tilde(20),
        "f10" => KeyForm::Tilde(21),
        "f11" => KeyForm::Tilde(23),
        "f12" => KeyForm::Tilde(24),
        "f13" => KeyForm::Unicode(57376),
        "f14" => KeyForm::Unicode(57377),
        "f15" => KeyForm::Unicode(57378),
        "f16" => KeyForm::Unicode(57379),
        "f17" => KeyForm::Unicode(57380),
        "f18" => KeyForm::Unicode(57381),
        "f19" => KeyForm::Unicode(57382),
        "f20" => KeyForm::Unicode(57383),
        "menu" => KeyForm::Unicode(57363),
        _ => return None,
    };
    Some(form)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: &str) -> KeyEvent {
        KeyEvent {
            key,
            text: None,
            shifted: None,
            shift: false,
            alt: false,
            ctrl: false,
            super_key: false,
            event_type: EVENT_PRESS,
        }
    }

    fn text_event<'a>(key: &'a str, text: &'a str, shifted: &'a str, shift: bool) -> KeyEvent<'a> {
        KeyEvent {
            key,
            text: Some(text),
            shifted: Some(shifted),
            shift,
            alt: false,
            ctrl: false,
            super_key: false,
            event_type: EVENT_PRESS,
        }
    }

    fn with_mods(mut event: KeyEvent, shift: bool, alt: bool, ctrl: bool) -> KeyEvent {
        event.shift = shift;
        event.alt = alt;
        event.ctrl = ctrl;
        event
    }

    #[test]
    fn zero_flags_leaves_everything_to_the_legacy_encoder() {
        // The caller's hand-written legacy path owns all keys when no
        // flags are set; this encoder must not double-encode.
        assert_eq!(encode(&press("escape"), 0), None);
        assert_eq!(encode(&press("enter"), 0), None);
        assert_eq!(encode(&press("backspace"), 0), None);
        assert_eq!(encode(&press("up"), 0), None);
        assert_eq!(encode(&text_event("a", "a", "A", false), 0), None);
    }

    #[test]
    fn disambiguate_routes_ctrl_and_alt_through_csi_u() {
        let flags = FLAG_DISAMBIGUATE;
        let ctrl_i = with_mods(text_event("i", "\t", "I", false), false, false, true);
        assert_eq!(encode(&ctrl_i, flags).as_deref(), Some(b"\x1b[105;5u".as_slice()));

        let alt_b = with_mods(text_event("b", "b", "B", false), false, true, false);
        assert_eq!(encode(&alt_b, flags).as_deref(), Some(b"\x1b[98;3u".as_slice()));

        // ctrl+space is a key event, not the NUL byte.
        let ctrl_space = with_mods(text_event(" ", " ", " ", false), false, false, true);
        assert_eq!(encode(&ctrl_space, flags).as_deref(), Some(b"\x1b[32;5u".as_slice()));

        // Unmodified text stays legacy so shells keep working.
        assert_eq!(encode(&text_event("a", "a", "A", false), flags), Some(b"a".to_vec()));
    }

    #[test]
    fn disambiguate_keeps_exception_keys_legacy_on_press() {
        let flags = FLAG_DISAMBIGUATE;
        assert_eq!(encode(&press("enter"), flags).as_deref(), Some(b"\r".as_slice()));
        assert_eq!(encode(&press("tab"), flags).as_deref(), Some(b"\t".as_slice()));
        assert_eq!(encode(&press("backspace"), flags).as_deref(), Some(b"\x7f".as_slice()));
    }

    #[test]
    fn functional_keys_use_their_wire_forms() {
        let flags = FLAG_DISAMBIGUATE;
        assert_eq!(encode(&press("up"), flags).as_deref(), Some(b"\x1b[A".as_slice()));
        assert_eq!(encode(&press("f5"), flags).as_deref(), Some(b"\x1b[15~".as_slice()));
        assert_eq!(encode(&press("escape"), flags).as_deref(), Some(b"\x1b[27u".as_slice()));

        let shift_up = with_mods(press("up"), true, false, false);
        assert_eq!(encode(&shift_up, flags).as_deref(), Some(b"\x1b[1;2A".as_slice()));
    }

    #[test]
    fn report_all_turns_text_into_escape_codes() {
        let flags = FLAG_REPORT_ALL | FLAG_DISAMBIGUATE;
        assert_eq!(encode(&text_event("a", "a", "A", false), flags).as_deref(), Some(b"\x1b[97u".as_slice()));

        let all_text = FLAG_REPORT_ALL | FLAG_DISAMBIGUATE | FLAG_REPORT_ASSOCIATED_TEXT;
        let shift_a = text_event("a", "A", "A", true);
        assert_eq!(encode(&shift_a, all_text).as_deref(), Some(b"\x1b[97;2;65u".as_slice()));
    }

    #[test]
    fn event_types_report_repeat_and_release() {
        let flags = FLAG_DISAMBIGUATE | FLAG_REPORT_EVENTS;
        let mut repeat = press("up");
        repeat.event_type = EVENT_REPEAT;
        assert_eq!(encode(&repeat, flags).as_deref(), Some(b"\x1b[1;1:2A".as_slice()));

        let mut release = press("up");
        release.event_type = EVENT_RELEASE;
        assert_eq!(encode(&release, flags).as_deref(), Some(b"\x1b[1;1:3A".as_slice()));

        // Releases without the flag are swallowed.
        assert_eq!(encode(&release, FLAG_DISAMBIGUATE), None);
    }

    #[test]
    fn release_never_falls_back_to_legacy_bytes() {
        // herdr-style flag set (disambiguate | events | alternate, no
        // report_all): a plain key stays legacy on press, but a release
        // emitted as legacy bytes arrives as a phantom press ("a" -> "aa").
        let flags = FLAG_DISAMBIGUATE | FLAG_REPORT_EVENTS | FLAG_REPORT_ALTERNATE;
        let mut release = press("a");
        release.event_type = EVENT_RELEASE;
        assert_eq!(encode(&release, flags), None);
        // Press still takes the legacy path.
        assert_eq!(
            encode(&text_event("a", "a", "A", false), flags).as_deref(),
            Some(b"a".as_slice())
        );
    }

    #[test]
    fn alternate_keys_report_the_shifted_codepoint() {
        // Alternate keys only affect events already going out as escape
        // codes, so this needs REPORT_ALL alongside REPORT_ALTERNATE.
        let flags = FLAG_DISAMBIGUATE | FLAG_REPORT_ALL | FLAG_REPORT_ALTERNATE;
        // No shift: no alternate sub-field.
        assert_eq!(
            encode(&text_event("a", "a", "A", false), flags).as_deref(),
            Some(b"\x1b[97u".as_slice())
        );
        // shift+2 produces '#' on US layouts; the key stays 2.
        let shift_two = text_event("2", "#", "#", true);
        assert_eq!(
            encode(&shift_two, flags).as_deref(),
            Some(b"\x1b[50:35;2u".as_slice())
        );
    }

    #[test]
    fn stack_push_pop_and_set_follow_the_spec() {
        let mut state = KittyKeyboard::default();
        assert_eq!(state.flags(), 0);

        state.push(FLAG_DISAMBIGUATE);
        assert_eq!(state.flags(), FLAG_DISAMBIGUATE);
        state.push(FLAG_REPORT_ALL);
        assert_eq!(state.flags(), FLAG_REPORT_ALL);

        state.set(FLAG_REPORT_EVENTS, 2);
        assert_eq!(state.flags(), FLAG_REPORT_ALL | FLAG_REPORT_EVENTS);
        state.set(FLAG_REPORT_EVENTS, 3);
        assert_eq!(state.flags(), FLAG_REPORT_ALL);

        state.pop(1);
        assert_eq!(state.flags(), FLAG_DISAMBIGUATE);
        state.pop(5);
        assert_eq!(state.flags(), 0);

        // Unknown set modes do nothing.
        state.set(FLAG_REPORT_ALL, 9);
        assert_eq!(state.flags(), 0);

        // Invalid bits are masked, not stored.
        state.set(0b1000_0000, 1);
        assert_eq!(state.flags(), 0);
    }

    #[test]
    fn pop_on_empty_stack_resets_flags() {
        let mut state = KittyKeyboard::default();
        state.set(FLAG_DISAMBIGUATE, 1);
        state.pop(1);
        assert_eq!(state.flags(), 0);
    }
}
