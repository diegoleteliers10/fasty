use gpui::{div, px, Div, ParentElement, SharedString, Styled, Window};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextInputState {
    pub text: String,
    pub cursor: usize, // Character offset (0..=char_count)
    pub selection: Option<(usize, usize)>, // (start, end) where start < end
    pub drag_anchor: Option<usize>,
    pub last_bounds: Option<gpui::Bounds<gpui::Pixels>>,
}

impl TextInputState {
    pub fn new(text: String) -> Self {
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            selection: None,
            drag_anchor: None,
            last_bounds: None,
        }
    }

    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.cursor.min(self.char_count());
        self.selection = None;
        self.drag_anchor = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.selection = None;
        self.drag_anchor = None;
    }

    pub fn set_text_with_cursor(&mut self, text: impl Into<String>, cursor: usize) {
        self.text = text.into();
        self.cursor = cursor.min(self.char_count());
        self.selection = None;
        self.drag_anchor = None;
    }

    pub fn set_cursor(&mut self, idx: usize) {
        self.cursor = idx.min(self.char_count());
        self.selection = None;
        self.drag_anchor = None;
    }

    pub fn start_drag(&mut self, idx: usize) {
        let clamped = idx.min(self.char_count());
        self.cursor = clamped;
        self.drag_anchor = Some(clamped);
        self.selection = None;
    }

    pub fn update_drag(&mut self, idx: usize) {
        if let Some(anchor) = self.drag_anchor {
            let clamped = idx.min(self.char_count());
            self.cursor = clamped;
            if clamped != anchor {
                self.selection = Some((anchor.min(clamped), anchor.max(clamped)));
            } else {
                self.selection = None;
            }
        }
    }

    pub fn end_drag(&mut self) {
        self.drag_anchor = None;
    }

    pub fn select_all(&mut self) {
        let count = self.char_count();
        if count > 0 {
            self.selection = Some((0, count));
            self.cursor = count;
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection.and_then(|(start, end)| {
            if start < end {
                Some(self.text.chars().skip(start).take(end - start).collect())
            } else {
                None
            }
        })
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some_and(|(s, e)| s < e)
    }

    pub fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selection {
            if start < end {
                let chars: Vec<char> = self.text.chars().collect();
                let clamped_end = end.min(chars.len());
                let clamped_start = start.min(clamped_end);
                let mut new_chars: Vec<char> = Vec::with_capacity(chars.len().saturating_sub(clamped_end - clamped_start));
                new_chars.extend(&chars[..clamped_start]);
                if clamped_end < chars.len() {
                    new_chars.extend(&chars[clamped_end..]);
                }
                self.text = new_chars.into_iter().collect();
                self.cursor = clamped_start;
                self.selection = None;
                self.drag_anchor = None;
                return true;
            }
        }
        false
    }

    pub fn insert_str(&mut self, s: &str) {
        let _ = self.delete_selection();
        let chars: Vec<char> = self.text.chars().collect();
        let cur = self.cursor.min(chars.len());
        let ins: Vec<char> = s.chars().collect();
        let mut new_chars: Vec<char> = Vec::with_capacity(chars.len() + ins.len());
        new_chars.extend(&chars[..cur]);
        new_chars.extend(&ins);
        new_chars.extend(&chars[cur..]);
        self.text = new_chars.into_iter().collect();
        self.cursor = cur + ins.len();
        self.selection = None;
        self.drag_anchor = None;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let cur = self.cursor.min(chars.len());
        if cur > 0 {
            let mut new_chars: Vec<char> = Vec::with_capacity(chars.len() - 1);
            new_chars.extend(&chars[..cur - 1]);
            new_chars.extend(&chars[cur..]);
            self.text = new_chars.into_iter().collect();
            self.cursor = cur - 1;
        }
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let cur = self.cursor.min(chars.len());
        if cur < chars.len() {
            let mut new_chars: Vec<char> = Vec::with_capacity(chars.len() - 1);
            new_chars.extend(&chars[..cur]);
            new_chars.extend(&chars[cur + 1..]);
            self.text = new_chars.into_iter().collect();
        }
    }

    pub fn move_left(&mut self, extend: bool) {
        let cur = self.cursor;
        if extend {
            let anchor = self.drag_anchor.unwrap_or(cur);
            self.drag_anchor = Some(anchor);
            let next_cur = cur.saturating_sub(1);
            self.cursor = next_cur;
            self.selection = if next_cur == anchor {
                None
            } else {
                Some((anchor.min(next_cur), anchor.max(next_cur)))
            };
        } else if let Some((start, _)) = self.selection {
            self.cursor = start;
            self.selection = None;
            self.drag_anchor = None;
        } else {
            self.cursor = cur.saturating_sub(1);
            self.selection = None;
            self.drag_anchor = None;
        }
    }

    pub fn move_right(&mut self, extend: bool) {
        let cur = self.cursor;
        let count = self.char_count();
        if extend {
            let anchor = self.drag_anchor.unwrap_or(cur);
            self.drag_anchor = Some(anchor);
            let next_cur = (cur + 1).min(count);
            self.cursor = next_cur;
            self.selection = if next_cur == anchor {
                None
            } else {
                Some((anchor.min(next_cur), anchor.max(next_cur)))
            };
        } else if let Some((_, end)) = self.selection {
            self.cursor = end;
            self.selection = None;
            self.drag_anchor = None;
        } else {
            self.cursor = (cur + 1).min(count);
            self.selection = None;
            self.drag_anchor = None;
        }
    }

    pub fn move_home(&mut self, extend: bool) {
        if extend {
            let anchor = self.drag_anchor.unwrap_or(self.cursor);
            self.drag_anchor = Some(anchor);
            self.cursor = 0;
            self.selection = if anchor > 0 { Some((0, anchor)) } else { None };
        } else {
            self.cursor = 0;
            self.selection = None;
            self.drag_anchor = None;
        }
    }

    pub fn move_end(&mut self, extend: bool) {
        let count = self.char_count();
        if extend {
            let anchor = self.drag_anchor.unwrap_or(self.cursor);
            self.drag_anchor = Some(anchor);
            self.cursor = count;
            self.selection = if anchor < count { Some((anchor, count)) } else { None };
        } else {
            self.cursor = count;
            self.selection = None;
            self.drag_anchor = None;
        }
    }

    pub fn index_for_position(
        &self,
        position: gpui::Point<gpui::Pixels>,
        font_size: f32,
        padding_left: f32,
        window: &Window,
    ) -> usize {
        if self.text.is_empty() {
            return 0;
        }
        let rel_x = if let Some(bounds) = self.last_bounds {
            (position.x.to_f64() as f32 - bounds.origin.x.to_f64() as f32 - padding_left).max(0.0)
        } else {
            0.0
        };
        index_for_x(&self.text, rel_x, font_size, window)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedVisualLine {
    pub line_idx: usize,
    pub start_char: usize,
    pub text: String,
}

pub fn wrap_text_into_lines(text: &str, max_cols: usize) -> Vec<WrappedVisualLine> {
    let max_cols = max_cols.max(8);
    let mut lines = Vec::new();
    let mut global_char_offset = 0;

    let raw_lines: Vec<&str> = text.split('\n').collect();
    for (raw_idx, raw_line) in raw_lines.iter().enumerate() {
        if raw_line.is_empty() {
            lines.push(WrappedVisualLine {
                line_idx: lines.len(),
                start_char: global_char_offset,
                text: String::new(),
            });
        } else {
            let chars: Vec<char> = raw_line.chars().collect();
            let mut pos = 0;
            while pos < chars.len() {
                let remaining_count = chars.len() - pos;
                if remaining_count <= max_cols {
                    let chunk: String = chars[pos..].iter().collect();
                    lines.push(WrappedVisualLine {
                        line_idx: lines.len(),
                        start_char: global_char_offset,
                        text: chunk,
                    });
                    global_char_offset += remaining_count;
                    pos = chars.len();
                } else {
                    let slice = &chars[pos..pos + max_cols];
                    let break_rel = slice
                        .iter()
                        .rposition(|&c| c.is_whitespace())
                        .map(|idx| idx + 1)
                        .unwrap_or(max_cols);

                    let chunk: String = chars[pos..pos + break_rel].iter().collect();
                    lines.push(WrappedVisualLine {
                        line_idx: lines.len(),
                        start_char: global_char_offset,
                        text: chunk,
                    });
                    global_char_offset += break_rel;
                    pos += break_rel;
                }
            }
        }
        if raw_idx + 1 < raw_lines.len() {
            global_char_offset += 1; // Count newline character
        }
    }

    if lines.is_empty() {
        lines.push(WrappedVisualLine {
            line_idx: 0,
            start_char: 0,
            text: String::new(),
        });
    }

    lines
}

pub fn selection_highlight_color() -> gpui::Hsla {
    gpui::Hsla {
        h: 215.0 / 360.0,
        s: 0.88,
        l: 0.52,
        a: 0.48,
    }
}

pub fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

pub fn byte_to_char_idx(text: &str, byte_idx: usize) -> usize {
    let mut count = 0;
    for (b, _) in text.char_indices() {
        if b >= byte_idx {
            return count;
        }
        count += 1;
    }
    count
}

pub fn index_for_x(
    text: &str,
    rel_x: f32,
    font_size: f32,
    window: &Window,
) -> usize {
    if text.is_empty() || rel_x <= 0.0 {
        return 0;
    }
    let char_count = text.chars().count();
    let text_run = window.text_style().to_run(text.len());
    let layout = window.text_system().layout_line(text, px(font_size), &[text_run], None);
    let total_width = layout.width.to_f64() as f32;
    if rel_x >= total_width {
        return char_count;
    }

    let mut best_idx = 0;
    let mut best_dist = rel_x.abs();

    for i in 1..=char_count {
        let x = if i == char_count {
            total_width
        } else {
            let byte_idx = char_to_byte_idx(text, i);
            layout.x_for_index(byte_idx).to_f64() as f32
        };
        let dist = (rel_x - x).abs();
        if dist < best_dist {
            best_dist = dist;
            best_idx = i;
        } else if x > rel_x {
            break;
        }
    }

    best_idx
}

pub fn x_for_index(
    text: &str,
    char_idx: usize,
    font_size: f32,
    window: Option<&Window>,
) -> f32 {
    if char_idx == 0 || text.is_empty() {
        return 0.0;
    }
    let char_count = text.chars().count();
    if let Some(w) = window {
        let text_run = w.text_style().to_run(text.len());
        let layout = w.text_system().layout_line(text, px(font_size), &[text_run], None);
        if char_idx >= char_count {
            layout.width.to_f64() as f32
        } else {
            let byte_idx = char_to_byte_idx(text, char_idx);
            layout.x_for_index(byte_idx).to_f64() as f32
        }
    } else {
        (char_idx as f32) * (font_size * 0.55)
    }
}

pub fn render_line_spans(
    line_text: &str,
    start_char: usize,
    cursor: Option<usize>,
    selection: Option<(usize, usize)>,
    font_size: f32,
    theme: &crate::ui::Theme,
    window: Option<&Window>,
) -> Div {
    let line_len = line_text.chars().count();
    let line_end = start_char + line_len;
    let sel_bg = selection_highlight_color();

    // Local cursor position in 0..=line_len
    let local_cursor = cursor.and_then(|c| {
        if c >= start_char && c <= line_end {
            Some(c - start_char)
        } else {
            None
        }
    });

    // Local selection range in 0..=line_len
    let local_sel = selection.and_then(|(s, e)| {
        let s_norm = s.min(e);
        let e_norm = s.max(e);
        let s_clamped = s_norm.max(start_char).min(line_end);
        let e_clamped = e_norm.max(start_char).min(line_end);
        if s_clamped < e_clamped {
            Some((s_clamped - start_char, e_clamped - start_char))
        } else {
            None
        }
    });

    let mut container = div()
        .relative()
        .flex_shrink_0()
        .h(px(font_size + 4.0))
        .flex()
        .items_center();

    // 1. Selection highlight rectangle (under text, absolute, takes 0 flow width)
    if let Some((s, e)) = local_sel {
        let sel_start_x = x_for_index(line_text, s, font_size, window);
        let sel_end_x = x_for_index(line_text, e, font_size, window);
        let sel_w = (sel_end_x - sel_start_x).max(1.0);
        container = container.child(
            div()
                .absolute()
                .left(px(sel_start_x))
                .top(px(1.))
                .w(px(sel_w))
                .h(px(font_size + 2.0))
                .bg(sel_bg)
                .rounded(px(2.)),
        );
    }

    // 2. Cursor indicator (absolute, takes 0 flow width)
    if let Some(c) = local_cursor {
        let cursor_x = x_for_index(line_text, c, font_size, window);
        let snapped_x = if cursor_x > 0.0 { (cursor_x - 0.75).max(0.0) } else { 0.0 };
        container = container.child(
            div()
                .absolute()
                .left(px(snapped_x))
                .top(px(1.))
                .w(px(1.5))
                .h(px(font_size + 2.0))
                .bg(theme.accent)
                .rounded(px(1.)),
        );
    }

    // 3. Text content (single continuous element, NEVER split, NEVER shifts)
    if line_len > 0 {
        container = container.child(
            div()
                .text_size(px(font_size))
                .text_color(theme.foreground)
                .child(SharedString::from(line_text.to_string())),
        );
    }

    container
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_input_editing_and_selection() {
        let mut state = TextInputState::new("hello world".into());
        assert_eq!(state.cursor, 11);

        state.start_drag(5);
        state.update_drag(11);
        state.end_drag();
        assert_eq!(state.selection, Some((5, 11)));
        assert_eq!(state.selected_text().as_deref(), Some(" world"));

        state.insert_str(" rust!");
        assert_eq!(state.text, "hello rust!");
        assert_eq!(state.cursor, 11);
        assert_eq!(state.selection, None);

        state.backspace();
        assert_eq!(state.text, "hello rust");
    }

    #[test]
    fn test_text_wrapping() {
        let text = "this is a very long line that should wrap cleanly into multiple lines";
        let lines = wrap_text_into_lines(text, 20);
        assert!(lines.len() >= 3);
        let reconstructed = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("");
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn test_render_line_spans_partitions() {
        let theme = crate::ui::Theme::default();
        let _ = render_line_spans("hello world", 0, Some(5), Some((2, 8)), 12.0, &theme, None);
        let _ = render_line_spans("", 0, Some(0), None, 12.0, &theme, None);
        let _ = render_line_spans("abc", 0, None, Some((0, 3)), 12.0, &theme, None);
    }
}
