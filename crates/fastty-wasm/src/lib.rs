#[cfg(test)]
mod tests;
pub mod vt;

use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::vt::{
    Cell, Terminal, VtHandler, DEFAULT_BG, DEFAULT_CURSOR_COLOR, FLAG_BOLD, FLAG_DIM, FLAG_INVERSE,
    FLAG_UNDERLINE,
};

#[wasm_bindgen]
pub struct FasttyVt {
    terminal: Terminal,
    parser: vte::Parser,
    dirty: bool,
}

#[wasm_bindgen]
impl FasttyVt {
    #[wasm_bindgen(constructor)]
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        Self {
            terminal: Terminal::new(cols.max(1), rows.max(1), scrollback),
            parser: vte::Parser::new(),
            dirty: true,
        }
    }

    #[wasm_bindgen]
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        let mut handler = VtHandler {
            term: &mut self.terminal,
            osc_buf: Vec::new(),
        };
        for &b in bytes {
            self.parser.advance(&mut handler, b);
        }
        self.dirty = true;
    }

    #[wasm_bindgen]
    pub fn feed_str(&mut self, text: &str) {
        self.feed_bytes(text.as_bytes());
    }

    #[wasm_bindgen]
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.terminal.resize(cols, rows);
        self.dirty = true;
    }

    /// Restore the entire screen in microseconds from a binary snapshot (magic b"FST1").
    #[wasm_bindgen]
    pub fn restore_binary_snapshot(&mut self, data: &[u8]) -> bool {
        let restored = self.terminal.restore_binary_snapshot(data);
        if restored {
            self.dirty = true;
        }
        restored
    }

    /// Scroll display up into history (positive delta) or down towards bottom (negative delta).
    #[wasm_bindgen]
    pub fn scroll_display(&mut self, lines: i32) {
        let max_scroll = self.terminal.main_grid.scrollback.len();
        if lines > 0 {
            self.terminal.scroll_offset = self
                .terminal
                .scroll_offset
                .saturating_add(lines as usize)
                .min(max_scroll);
        } else if lines < 0 {
            self.terminal.scroll_offset = self
                .terminal
                .scroll_offset
                .saturating_sub((-lines) as usize);
        }
        self.dirty = true;
    }

    #[wasm_bindgen]
    pub fn scroll_to(&mut self, offset: usize) {
        let max_scroll = self.terminal.main_grid.scrollback.len();
        self.terminal.scroll_offset = offset.min(max_scroll);
        self.dirty = true;
    }

    #[wasm_bindgen]
    pub fn scroll_to_top(&mut self) {
        let max_scroll = self.terminal.main_grid.scrollback.len();
        self.terminal.scroll_offset = max_scroll;
        self.dirty = true;
    }

    #[wasm_bindgen]
    pub fn scroll_to_bottom(&mut self) {
        self.terminal.scroll_offset = 0;
        self.dirty = true;
    }

    #[wasm_bindgen]
    pub fn scroll_page_up(&mut self) {
        let page = self.terminal.grid().rows.saturating_sub(2).max(1);
        self.scroll_display(page as i32);
    }

    #[wasm_bindgen]
    pub fn scroll_page_down(&mut self) {
        let page = self.terminal.grid().rows.saturating_sub(2).max(1);
        self.scroll_display(-(page as i32));
    }

    #[wasm_bindgen]
    pub fn scroll_offset(&self) -> usize {
        self.terminal.scroll_offset
    }

    #[wasm_bindgen]
    pub fn max_scroll_offset(&self) -> usize {
        self.terminal.main_grid.scrollback.len()
    }

    #[wasm_bindgen]
    pub fn cols(&self) -> usize {
        self.terminal.grid().cols
    }

    #[wasm_bindgen]
    pub fn rows(&self) -> usize {
        self.terminal.grid().rows
    }

    #[wasm_bindgen]
    pub fn title(&self) -> String {
        self.terminal.title.clone()
    }

    #[wasm_bindgen]
    pub fn cwd(&self) -> String {
        self.terminal.cwd.clone()
    }

    #[wasm_bindgen]
    pub fn cursor_col(&self) -> usize {
        self.terminal.cursor.col
    }

    #[wasm_bindgen]
    pub fn cursor_row(&self) -> usize {
        self.terminal.cursor.row
    }

    #[wasm_bindgen]
    pub fn cursor_visible(&self) -> bool {
        self.terminal.cursor.visible
    }

    #[wasm_bindgen]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    #[wasm_bindgen]
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// High performance direct 2D Canvas renderer. Renders the full terminal grid,
    /// background color blocks, styled text, and cursor in batched single passes.
    #[wasm_bindgen]
    pub fn render_canvas(
        &mut self,
        canvas: &HtmlCanvasElement,
        font_family: &str,
        font_size_px: f64,
        dpr: f64,
    ) -> Result<(), JsValue> {
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("failed to get 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;

        let cols = self.terminal.grid().cols;
        let rows = self.terminal.grid().rows;

        // Font metrics computation (scaled by DPI)
        let char_width = (font_size_px * 0.60 * dpr).max(4.0).round();
        let char_height = (font_size_px * 1.25 * dpr).max(8.0).round();
        let width = cols as f64 * char_width;
        let height = rows as f64 * char_height;

        let cur_w = canvas.width() as f64;
        let cur_h = canvas.height() as f64;
        if cur_w != width || cur_h != height {
            canvas.set_width(width as u32);
            canvas.set_height(height as u32);
        }

        // Draw main background
        let bg_hex = format!("#{:06x}", DEFAULT_BG);
        context.set_fill_style_str(&bg_hex);
        context.fill_rect(0.0, 0.0, width, height);

        let font_str = format!("{}px {}", (font_size_px * dpr).round(), font_family);
        let bold_font_str = format!("bold {}px {}", (font_size_px * dpr).round(), font_family);
        context.set_font(&font_str);
        context.set_text_baseline("top");

        let scroll_offset = self.terminal.scroll_offset;
        let scrollback_len = self.terminal.main_grid.scrollback.len();

        for r in 0..rows {
            let row_cells: Option<&Vec<Cell>> = if scroll_offset > 0 && !self.terminal.is_alt {
                let history_idx = scrollback_len + r;
                if history_idx >= scroll_offset {
                    let sb_idx = history_idx - scroll_offset;
                    if sb_idx < scrollback_len {
                        self.terminal.main_grid.scrollback.get(sb_idx)
                    } else {
                        self.terminal.main_grid.cells.get(sb_idx - scrollback_len)
                    }
                } else {
                    None
                }
            } else {
                self.terminal.grid().cells.get(r)
            };

            let Some(row) = row_cells else { continue };
            let y = r as f64 * char_height;

            // 1. Batched background drawing
            let mut bg_start_col = 0;
            let mut cur_bg = DEFAULT_BG;
            let mut cur_bg_len = 0;

            for (c, cell) in row.iter().enumerate().take(cols.min(row.len())) {
                                let mut bg = cell.bg;
                if cell.flags & FLAG_INVERSE != 0 {
                    bg = cell.fg;
                }

                if bg == cur_bg {
                    cur_bg_len += 1;
                } else {
                    if cur_bg != DEFAULT_BG && cur_bg_len > 0 {
                        let x = bg_start_col as f64 * char_width;
                        let span_w = cur_bg_len as f64 * char_width;
                        let hex = format!("#{:06x}", cur_bg);
                        context.set_fill_style_str(&hex);
                        context.fill_rect(x, y, span_w, char_height);
                    }
                    cur_bg = bg;
                    bg_start_col = c;
                    cur_bg_len = 1;
                }
            }
            if cur_bg != DEFAULT_BG && cur_bg_len > 0 {
                let x = bg_start_col as f64 * char_width;
                let span_w = cur_bg_len as f64 * char_width;
                let hex = format!("#{:06x}", cur_bg);
                context.set_fill_style_str(&hex);
                context.fill_rect(x, y, span_w, char_height);
            }

            // 2. Batched text runs drawing
            let mut text_buf = String::with_capacity(cols);
            let mut run_start_col = 0;
            let mut run_fg = 0u32;
            let mut run_flags = 0u8;
            let mut has_run = false;

            for (c, cell) in row.iter().enumerate().take(cols.min(row.len())) {
                                let mut fg = cell.fg;
                if cell.flags & FLAG_INVERSE != 0 {
                    fg = cell.bg;
                }
                if cell.flags & FLAG_DIM != 0 {
                    let r = ((fg >> 16) & 0xFF) / 2;
                    let g = ((fg >> 8) & 0xFF) / 2;
                    let b = (fg & 0xFF) / 2;
                    fg = (r << 16) | (g << 8) | b;
                }

                let ch = if cell.c == '\0' { ' ' } else { cell.c };

                if !has_run {
                    if ch != ' ' {
                        has_run = true;
                        run_start_col = c;
                        run_fg = fg;
                        run_flags = cell.flags;
                        text_buf.push(ch);
                    }
                } else if fg == run_fg && cell.flags == run_flags {
                    text_buf.push(ch);
                } else {
                    // Flush existing run
                    let trimmed = text_buf.trim_end();
                    if !trimmed.is_empty() {
                        if run_flags & FLAG_BOLD != 0 {
                            context.set_font(&bold_font_str);
                        } else {
                            context.set_font(&font_str);
                        }
                        let hex = format!("#{:06x}", run_fg);
                        context.set_fill_style_str(&hex);
                        let x = run_start_col as f64 * char_width;
                        let _ = context.fill_text(trimmed, x, y + (char_height * 0.1));

                        if run_flags & FLAG_UNDERLINE != 0 {
                            let underline_w = text_buf.chars().count() as f64 * char_width;
                            context.fill_rect(x, y + char_height - 2.0, underline_w, 2.0);
                        }
                    }
                    text_buf.clear();

                    if ch != ' ' {
                        run_start_col = c;
                        run_fg = fg;
                        run_flags = cell.flags;
                        text_buf.push(ch);
                    } else {
                        has_run = false;
                    }
                }
            }

            // Flush tail run if any
            if has_run {
                let trimmed = text_buf.trim_end();
                if !trimmed.is_empty() {
                    if run_flags & FLAG_BOLD != 0 {
                        context.set_font(&bold_font_str);
                    } else {
                        context.set_font(&font_str);
                    }
                    let hex = format!("#{:06x}", run_fg);
                    context.set_fill_style_str(&hex);
                    let x = run_start_col as f64 * char_width;
                    let _ = context.fill_text(trimmed, x, y + (char_height * 0.1));

                    if run_flags & FLAG_UNDERLINE != 0 {
                        let underline_w = text_buf.chars().count() as f64 * char_width;
                        context.fill_rect(x, y + char_height - 2.0, underline_w, 2.0);
                    }
                }
            }
        }

        // 3. Draw cursor (only when viewing bottom & cursor visible)
        if scroll_offset == 0 && self.terminal.cursor.visible {
            let cx = self.terminal.cursor.col as f64 * char_width;
            let cy = self.terminal.cursor.row as f64 * char_height;
            let cursor_hex = format!("#{:06x}", DEFAULT_CURSOR_COLOR);
            context.set_fill_style_str(&cursor_hex);
            context.fill_rect(cx, cy, char_width, char_height);

            // Redraw character inside cursor block inverted
            let r = self.terminal.cursor.row;
            let c = self.terminal.cursor.col;
            if r < rows && c < cols {
                if let Some(row) = self.terminal.grid().cells.get(r) {
                    if let Some(cell) = row.get(c) {
                        if cell.c != ' ' && cell.c != '\0' {
                            context.set_fill_style_str(&bg_hex);
                            let text_s = cell.c.to_string();
                            let _ = context.fill_text(&text_s, cx, cy + (char_height * 0.1));
                        }
                    }
                }
            }
        }

        self.dirty = false;
        Ok(())
    }

    /// Encode browser KeyboardEvent details into standard ANSI/VT input sequence.
    #[wasm_bindgen]
    pub fn encode_key(
        key: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        _meta: bool,
    ) -> Option<String> {
        match key {
            "Enter" => Some("\r".to_string()),
            "Backspace" => Some(if ctrl { "\x08" } else { "\x7f" }.to_string()),
            "Tab" => Some(if shift { "\x1b[Z" } else { "\t" }.to_string()),
            "Escape" => Some("\x1b".to_string()),
            "ArrowUp" => Some(if alt { "\x1b\x1b[A" } else { "\x1b[A" }.to_string()),
            "ArrowDown" => Some(if alt { "\x1b\x1b[B" } else { "\x1b[B" }.to_string()),
            "ArrowRight" => Some(if alt { "\x1b\x1b[C" } else { "\x1b[C" }.to_string()),
            "ArrowLeft" => Some(if alt { "\x1b\x1b[D" } else { "\x1b[D" }.to_string()),
            "Home" => Some("\x1b[H".to_string()),
            "End" => Some("\x1b[F".to_string()),
            "PageUp" => Some("\x1b[5~".to_string()),
            "PageDown" => Some("\x1b[6~".to_string()),
            "Delete" => Some("\x1b[3~".to_string()),
            "Insert" => Some("\x1b[2~".to_string()),
            _ => {
                if ctrl && key.len() == 1 {
                    let b = key.as_bytes()[0];
                    if b.is_ascii_alphabetic() {
                        let code = (b.to_ascii_uppercase() - b'@') as char;
                        return Some(code.to_string());
                    }
                }
                if key.len() == 1 {
                    if alt {
                        return Some(format!("\x1b{}", key));
                    }
                    return Some(key.to_string());
                }
                None
            }
        }
    }
}
