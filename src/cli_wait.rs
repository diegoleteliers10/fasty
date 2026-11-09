//! `fastty wait`: block until a pane prints a literal string.
//!
//! The scan runs over raw PTY bytes, which carry ANSI escape sequences, so the
//! text is filtered before matching. Two rules keep it cheap:
//!
//! - Only the last [`WINDOW_BYTES`] of filtered text are kept. A wait that runs
//!   for an hour uses the same memory as one that runs for a second.
//! - Only output produced after the wait starts is matched. Matching a string
//!   that was already on screen would return immediately and hide a hang.
//!
//! A sequence split across two reads is handled by keeping a small carry: the
//! filter never drops bytes it cannot classify yet, and the window keeps a
//! prefix from the previous pass so a match spanning the boundary is found.

/// Filtered text kept for matching. Long enough for a compiler banner, small
/// enough that an eight-hour wait stays flat.
pub const WINDOW_BYTES: usize = 8 * 1024;

/// Where the ANSI filter is inside an escape sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FilterState {
    #[default]
    Text,
    /// Saw ESC; the next byte decides between CSI, OSC and a two-byte escape.
    Escape,
    /// Inside `ESC [ ... final`.
    Csi,
    /// Inside `ESC ] ... BEL|ST`.
    Osc,
    /// Inside an OSC terminated by BEL.
    OscBell,
    /// Saw ESC inside an OSC, expecting `\\`.
    OscEscape,
}

/// Strips ANSI escape sequences and normalises line endings.
///
/// Bytes that belong to an incomplete sequence stay in `pending` until the next
/// call, so a sequence split across two PTY reads is still removed.
#[derive(Debug, Default)]
pub struct TextFilter {
    state: FilterState,
    pending: Vec<u8>,
}

impl TextFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter one chunk and append the visible text to `out`.
    pub fn push(&mut self, chunk: &[u8], out: &mut String) {
        let mut bytes = std::mem::take(&mut self.pending);
        bytes.extend_from_slice(chunk);
        self.pending.clear();

        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            match self.state {
                FilterState::Text => match byte {
                    0x1b => {
                        self.state = FilterState::Escape;
                        index += 1;
                    }
                    b'\r' => {
                        // A PTY sends CR before LF. Drop the CR so a match sees
                        // one line break, not two.
                        index += 1;
                    }
                    0x08 => {
                        // Backspace edits the line; keep the text simple.
                        out.pop();
                        index += 1;
                    }
                    _ if byte < 0x20 && byte != b'\n' && byte != b'\t' => {
                        index += 1;
                    }
                    _ => {
                        // Copy the whole UTF-8 sequence so multibyte text stays valid.
                        let width = utf8_width(byte);
                        if index + width > bytes.len() {
                            self.pending.extend_from_slice(&bytes[index..]);
                            return;
                        }
                        out.push_str(&String::from_utf8_lossy(&bytes[index..index + width]));
                        index += width;
                    }
                },
                FilterState::Escape => match byte {
                    b'[' => {
                        self.state = FilterState::Csi;
                        index += 1;
                    }
                    b']' => {
                        self.state = FilterState::Osc;
                        index += 1;
                    }
                    _ => {
                        // Two-byte escape such as ESC ( B.
                        self.state = FilterState::Text;
                        index += 1;
                    }
                },
                FilterState::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.state = FilterState::Text;
                    }
                    index += 1;
                }
                FilterState::Osc => match byte {
                    0x07 => {
                        self.state = FilterState::Text;
                        index += 1;
                    }
                    0x1b => {
                        self.state = FilterState::OscEscape;
                        index += 1;
                    }
                    _ => index += 1,
                },
                FilterState::OscBell => {
                    self.state = FilterState::Text;
                    index += 1;
                }
                FilterState::OscEscape => {
                    if byte == b'\\' {
                        self.state = FilterState::Text;
                    } else {
                        self.state = FilterState::Osc;
                    }
                    index += 1;
                }
            }
        }
    }
}

fn utf8_width(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        // A continuation byte out of place: emit it alone so the parser moves.
        _ => 1,
    }
}

/// The result of a match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitMatch {
    /// The line holding the match, trimmed. Useful to report what happened.
    pub line: String,
}

/// Rolling window of filtered text that answers "has this string appeared".
#[derive(Debug)]
pub struct WaitScanner {
    needle: String,
    window: String,
}

impl WaitScanner {
    pub fn new(needle: &str) -> Self {
        Self {
            needle: needle.to_string(),
            window: String::new(),
        }
    }

    /// Add text and return the match the first time the needle appears.
    pub fn push(&mut self, text: &str) -> Option<WaitMatch> {
        if text.is_empty() {
            return None;
        }
        self.window.push_str(text);
        let trimmed = self.window.len().saturating_sub(WINDOW_BYTES);
        if trimmed > 0 {
            // Cut on a character boundary so the window stays valid UTF-8.
            let mut cut = trimmed;
            while cut < self.window.len() && !self.window.is_char_boundary(cut) {
                cut += 1;
            }
            self.window.drain(..cut);
        }
        let index = self.window.find(&self.needle)?;
        let line_start = self.window[..index].rfind('\n').map_or(0, |at| at + 1);
        let line_end = self.window[index..]
            .find('\n')
            .map_or(self.window.len(), |at| index + at);
        Some(WaitMatch {
            line: self.window[line_start..line_end].trim().to_string(),
        })
    }

    pub fn window(&self) -> &str {
        &self.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtered(chunks: &[&str]) -> String {
        let mut filter = TextFilter::new();
        let mut out = String::new();
        for chunk in chunks {
            filter.push(chunk.as_bytes(), &mut out);
        }
        out
    }

    #[test]
    fn colour_sequences_are_removed() {
        assert_eq!(filtered(&["\x1b[31mred\x1b[0m"]), "red");
    }

    #[test]
    fn a_sequence_split_across_chunks_is_still_removed() {
        assert_eq!(filtered(&["\x1b[3", "1mred"]), "red");
    }

    #[test]
    fn an_osc_title_is_removed() {
        assert_eq!(filtered(&["\x1b]0;my title\x07done"]), "done");
    }

    #[test]
    fn multibyte_text_survives_a_split() {
        let mut filter = TextFilter::new();
        let mut out = String::new();
        filter.push(b"caf\xc3", &mut out);
        filter.push(b"\xa9", &mut out);
        assert_eq!(out, "café");
    }

    #[test]
    fn carriage_return_does_not_double_the_line_break() {
        assert_eq!(filtered(&["one\r\ntwo\r\n"]), "one\ntwo\n");
    }

    #[test]
    fn a_match_is_found_across_two_chunks() {
        let mut scanner = WaitScanner::new("BUILD SUCCESS");
        assert!(scanner.push("starting\nBUILD ").is_none());
        let found = scanner.push("SUCCESS\n").expect("split match");
        assert_eq!(found.line, "BUILD SUCCESS");
    }

    #[test]
    fn the_window_stays_bounded() {
        let mut scanner = WaitScanner::new("never");
        for _ in 0..100 {
            scanner.push(&"x".repeat(1_000));
        }
        assert!(
            scanner.window().len() <= WINDOW_BYTES,
            "window grew to {}",
            scanner.window().len()
        );
    }

    #[test]
    fn a_match_at_the_window_edge_is_still_reported_with_its_line() {
        let mut scanner = WaitScanner::new("done");
        scanner.push(&"a".repeat(WINDOW_BYTES - 8));
        let found = scanner.push("\nall done\n").expect("match near the edge");
        assert_eq!(found.line, "all done");
    }

    #[test]
    fn the_first_match_wins() {
        let mut scanner = WaitScanner::new("ok");
        let found = scanner.push("first ok\nsecond ok\n").expect("match");
        assert_eq!(found.line, "first ok");
    }
}
