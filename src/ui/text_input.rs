use gpui::{div, px, Div, ParentElement, SharedString, Styled};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInputState {
    pub text: String,
    pub cursor: usize, // Character offset (0..=char_count)
    pub selection: Option<(usize, usize)>, // (start, end) where start < end
    pub drag_anchor: Option<usize>,
}

impl TextInputState {
    pub fn new(text: String) -> Self {
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            selection: None,
            drag_anchor: None,
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

pub fn render_line_spans(
    line_text: &str,
    start_char: usize,
    cursor: Option<usize>,
    selection: Option<(usize, usize)>,
    font_size: f32,
    theme: &crate::ui::Theme,
) -> Div {
    let line_len = line_text.chars().count();
    let line_end = start_char + line_len;
    let chars: Vec<char> = line_text.chars().collect();

    let sel_range = selection.and_then(|(s, e)| {
        let s_clamped = s.max(start_char).min(line_end);
        let e_clamped = e.max(start_char).min(line_end);
        if s_clamped < e_clamped {
            Some((s_clamped - start_char, e_clamped - start_char))
        } else {
            None
        }
    });

    let local_cursor = cursor.and_then(|c| {
        if c >= start_char && c <= line_end {
            Some(c - start_char)
        } else {
            None
        }
    });

    let mut children: Vec<Div> = Vec::new();

    let mut render_segment = |text: &str, is_selected: bool, seg_start: usize| {
        let seg_len = text.chars().count();
        let seg_end = seg_start + seg_len;
        let cur_in_seg = local_cursor.and_then(|c| {
            if c >= seg_start && c <= seg_end {
                Some(c - seg_start)
            } else {
                None
            }
        });

        if let Some(c_rel) = cur_in_seg {
            let seg_chars: Vec<char> = text.chars().collect();
            let before: String = seg_chars[..c_rel].iter().collect();
            let after: String = seg_chars[c_rel..].iter().collect();

            if !before.is_empty() {
                let mut d = div()
                    .text_size(px(font_size))
                    .text_color(theme.foreground)
                    .child(SharedString::from(before));
                if is_selected {
                    d = d.bg(theme.selected).rounded(px(2.));
                }
                children.push(d);
            }

            children.push(
                div()
                    .w(px(2.))
                    .h(px(font_size + 2.0))
                    .rounded(px(1.))
                    .bg(theme.accent)
                    .flex_shrink_0(),
            );

            if !after.is_empty() {
                let mut d = div()
                    .text_size(px(font_size))
                    .text_color(theme.foreground)
                    .child(SharedString::from(after));
                if is_selected {
                    d = d.bg(theme.selected).rounded(px(2.));
                }
                children.push(d);
            }
        } else if !text.is_empty() {
            let mut d = div()
                .text_size(px(font_size))
                .text_color(theme.foreground)
                .child(SharedString::from(text.to_string()));
            if is_selected {
                d = d.bg(theme.selected).rounded(px(2.));
            }
            children.push(d);
        }
    };

    if let Some((s, e)) = sel_range {
        let pre: String = chars[..s].iter().collect();
        let sel: String = chars[s..e].iter().collect();
        let post: String = chars[e..].iter().collect();

        render_segment(&pre, false, 0);
        render_segment(&sel, true, s);
        render_segment(&post, false, e);
    } else {
        render_segment(line_text, false, 0);
    }

    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .children(children)
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
}
