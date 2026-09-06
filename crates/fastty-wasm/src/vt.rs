use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;

pub const DEFAULT_FG: u32 = 0xCDD6F4; // Catppuccin / Modern theme foreground
pub const DEFAULT_BG: u32 = 0x1E1E2E; // Dark background
pub const DEFAULT_CURSOR_COLOR: u32 = 0xF5E0DC;

pub const FLAG_BOLD: u8 = 1 << 0;
pub const FLAG_DIM: u8 = 1 << 1;
pub const FLAG_ITALIC: u8 = 1 << 2;
pub const FLAG_UNDERLINE: u8 = 1 << 3;
pub const FLAG_INVERSE: u8 = 1 << 4;
pub const FLAG_HIDDEN: u8 = 1 << 5;
pub const FLAG_STRIKETHROUGH: u8 = 1 << 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub fg: u32,
    pub bg: u32,
    pub flags: u8,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            flags: 0,
        }
    }
}

pub type Row = Vec<Cell>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Debug)]
pub struct Cursor {
    pub col: usize,
    pub row: usize,
    pub visible: bool,
    pub style: CursorStyle,
    pub blink: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            col: 0,
            row: 0,
            visible: true,
            style: CursorStyle::Block,
            blink: true,
        }
    }
}

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Row>,
    pub scrollback: VecDeque<Row>,
    pub max_scrollback: usize,
}

impl Grid {
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        let cells = vec![vec![Cell::default(); cols]; rows];
        Self {
            cols,
            rows,
            cells,
            scrollback: VecDeque::with_capacity(max_scrollback.min(1000)),
            max_scrollback,
        }
    }

    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        if new_cols == self.cols && new_rows == self.rows {
            return;
        }
        let mut new_cells = vec![vec![Cell::default(); new_cols]; new_rows];
        for (r, row) in new_cells.iter_mut().enumerate().take(self.rows.min(new_rows)) {
            for (c, cell) in row.iter_mut().enumerate().take(self.cols.min(new_cols)) {
                *cell = self.cells[r][c];
            }
        }
        self.cells = new_cells;
        self.cols = new_cols;
        self.rows = new_rows;
    }

    pub fn scroll_up(&mut self, top: usize, bottom: usize) {
        if top >= bottom || bottom >= self.rows {
            return;
        }
        if top == 0 {
            let row = self.cells.remove(0);
            if self.scrollback.len() >= self.max_scrollback {
                self.scrollback.pop_front();
            }
            self.scrollback.push_back(row);
        } else {
            self.cells.remove(top);
        }
        self.cells.insert(bottom, vec![Cell::default(); self.cols]);
    }

    pub fn scroll_down(&mut self, top: usize, bottom: usize) {
        if top >= bottom || bottom >= self.rows {
            return;
        }
        self.cells.remove(bottom);
        self.cells.insert(top, vec![Cell::default(); self.cols]);
    }

    pub fn clear(&mut self) {
        for row in &mut self.cells {
            for cell in row {
                *cell = Cell::default();
            }
        }
    }
}

pub struct Terminal {
    pub main_grid: Grid,
    pub alt_grid: Grid,
    pub is_alt: bool,
    pub cursor: Cursor,
    pub saved_cursor: Cursor,
    pub saved_alt_cursor: Cursor,
    pub scroll_top: usize,
    pub scroll_bottom: usize,
    pub cur_fg: u32,
    pub cur_bg: u32,
    pub cur_flags: u8,
    pub title: String,
    pub cwd: String,
    pub scroll_offset: usize, // 0 = at bottom, > 0 = viewing history
}

const ANSI_PALETTE: [u32; 16] = [
    0x45475A, // Black (subtext0)
    0xF38BA8, // Red
    0xA6E3A1, // Green
    0xF9E2AF, // Yellow
    0x89B4FA, // Blue
    0xF5C2E7, // Magenta
    0x94E2D5, // Cyan
    0xBAC2DE, // White
    0x585B70, // Bright Black
    0xF38BA8, // Bright Red
    0xA6E3A1, // Bright Green
    0xF9E2AF, // Bright Yellow
    0x89B4FA, // Bright Blue
    0xF5C2E7, // Bright Magenta
    0x94E2D5, // Bright Cyan
    0xA6ADC8, // Bright White
];

pub fn get_256_color(index: u8) -> u32 {
    if index < 16 {
        return ANSI_PALETTE[index as usize];
    }
    if index >= 232 {
        // Grayscale 232-255
        let gray = (index - 232) * 10 + 8;
        return ((gray as u32) << 16) | ((gray as u32) << 8) | (gray as u32);
    }
    // 6x6x6 color cube 16-231
    let i = index - 16;
    let r = (i / 36) % 6;
    let g = (i / 6) % 6;
    let b = i % 6;
    let r_val = if r > 0 { (r * 40 + 55) as u32 } else { 0 };
    let g_val = if g > 0 { (g * 40 + 55) as u32 } else { 0 };
    let b_val = if b > 0 { (b * 40 + 55) as u32 } else { 0 };
    (r_val << 16) | (g_val << 8) | b_val
}

impl Terminal {
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        let scroll_bottom = rows.saturating_sub(1);
        Self {
            main_grid: Grid::new(cols, rows, scrollback),
            alt_grid: Grid::new(cols, rows, 0),
            is_alt: false,
            cursor: Cursor::default(),
            saved_cursor: Cursor::default(),
            saved_alt_cursor: Cursor::default(),
            scroll_top: 0,
            scroll_bottom,
            cur_fg: DEFAULT_FG,
            cur_bg: DEFAULT_BG,
            cur_flags: 0,
            title: "fastty".to_string(),
            cwd: "".to_string(),
            scroll_offset: 0,
        }
    }

    pub fn grid(&self) -> &Grid {
        if self.is_alt {
            &self.alt_grid
        } else {
            &self.main_grid
        }
    }

    pub fn grid_mut(&mut self) -> &mut Grid {
        if self.is_alt {
            &mut self.alt_grid
        } else {
            &mut self.main_grid
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        self.main_grid.resize(cols, rows);
        self.alt_grid.resize(cols, rows);
        self.scroll_bottom = rows.saturating_sub(1);
        if self.scroll_top >= rows {
            self.scroll_top = 0;
        }
    }

    /// Restore full terminal state in microseconds from raw binary snapshot.
    pub fn restore_binary_snapshot(&mut self, data: &[u8]) -> bool {
        if data.len() < 32 {
            return false;
        }
        if &data[0..4] != b"FST1" {
            return false;
        }
        let cols = u16::from_le_bytes([data[8], data[9]]) as usize;
        let rows = u16::from_le_bytes([data[10], data[11]]) as usize;
        let cursor_col = u16::from_le_bytes([data[12], data[13]]) as usize;
        let cursor_row = u16::from_le_bytes([data[14], data[15]]) as usize;
        let cell_count = u32::from_le_bytes([data[16], data[17], data[18], data[19]]) as usize;
        let flags = u16::from_le_bytes([data[6], data[7]]);
        let is_alt = (flags & (1 << 0)) != 0;
        let cursor_visible = (flags & (1 << 1)) != 0;
        let is_deflated = (flags & (1 << 2)) != 0;

        if cols == 0 || rows == 0 || cell_count != cols * rows {
            return false;
        }

        let decompressed_buf: Vec<u8>;
        let payload: &[u8] = if is_deflated {
            decompressed_buf = match miniz_oxide::inflate::decompress_to_vec(&data[32..]) {
                Ok(b) => b,
                Err(_) => return false,
            };
            let required_len = cell_count.checked_mul(16);
            if required_len.map_or(true, |len| decompressed_buf.len() < len) {
                return false;
            }
            &decompressed_buf[..cell_count * 16]
        } else {
            let required_len = cell_count.checked_mul(16).and_then(|n| n.checked_add(32));
            if required_len.map_or(true, |len| data.len() < len) {
                return false;
            }
            &data[32..32 + cell_count * 16]
        };

        self.resize(cols, rows);
        self.is_alt = is_alt;
        self.cursor.col = cursor_col.min(cols.saturating_sub(1));
        self.cursor.row = cursor_row.min(rows.saturating_sub(1));
        self.cursor.visible = cursor_visible;

        let grid = self.grid_mut();

        for r in 0..rows {
            for c in 0..cols {
                let offset = (r * cols + c) * 16;
                let chunk = &payload[offset..offset + 16];
                let cp = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let fg = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                let bg = u32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
                let cell_flags_raw = u16::from_le_bytes([chunk[12], chunk[13]]);

                let mut flags = 0u8;
                if (cell_flags_raw & (1 << 0)) != 0 { flags |= FLAG_BOLD; }
                if (cell_flags_raw & (1 << 1)) != 0 { flags |= FLAG_DIM; }
                if (cell_flags_raw & (1 << 2)) != 0 { flags |= FLAG_ITALIC; }
                if (cell_flags_raw & (1 << 3)) != 0 { flags |= FLAG_UNDERLINE; }
                if (cell_flags_raw & (1 << 4)) != 0 { flags |= FLAG_INVERSE; }
                if (cell_flags_raw & (1 << 5)) != 0 { flags |= FLAG_HIDDEN; }
                if (cell_flags_raw & (1 << 6)) != 0 { flags |= FLAG_STRIKETHROUGH; }

                let ch = std::char::from_u32(cp).unwrap_or(' ');
                grid.cells[r][c] = Cell {
                    c: ch,
                    fg,
                    bg,
                    flags,
                };
            }
        }

        true
    }

    pub fn line_feed(&mut self) {
        if self.cursor.row >= self.scroll_bottom {
            let top = self.scroll_top;
            let bottom = self.scroll_bottom;
            self.grid_mut().scroll_up(top, bottom);
        } else if self.cursor.row + 1 < self.grid().rows {
            self.cursor.row += 1;
        }
    }

    pub fn put_char(&mut self, c: char) {
        let cols = self.grid().cols;
        let width = c.width().unwrap_or(1);
        if width == 0 {
            return;
        }
        if self.cursor.col >= cols {
            self.cursor.col = 0;
            self.line_feed();
        }
        let cell = Cell {
            c,
            fg: self.cur_fg,
            bg: self.cur_bg,
            flags: self.cur_flags,
        };
        let r = self.cursor.row;
        let c_pos = self.cursor.col;
        let cur_fg = self.cur_fg;
        let cur_bg = self.cur_bg;
        let cur_flags = self.cur_flags;
        if r < self.grid().rows && c_pos < cols {
            self.grid_mut().cells[r][c_pos] = cell;
            self.cursor.col += 1;
            if width == 2 && self.cursor.col < cols {
                let spacer_c = self.cursor.col;
                self.grid_mut().cells[r][spacer_c] = Cell {
                    c: ' ',
                    fg: cur_fg,
                    bg: cur_bg,
                    flags: cur_flags,
                };
                self.cursor.col += 1;
            }
        }
    }

    pub fn handle_sgr(&mut self, params: &[i64]) {
        if params.is_empty() {
            self.cur_fg = DEFAULT_FG;
            self.cur_bg = DEFAULT_BG;
            self.cur_flags = 0;
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = params[i];
            match p {
                0 => {
                    self.cur_fg = DEFAULT_FG;
                    self.cur_bg = DEFAULT_BG;
                    self.cur_flags = 0;
                }
                1 => self.cur_flags |= FLAG_BOLD,
                2 => self.cur_flags |= FLAG_DIM,
                3 => self.cur_flags |= FLAG_ITALIC,
                4 => self.cur_flags |= FLAG_UNDERLINE,
                7 => self.cur_flags |= FLAG_INVERSE,
                8 => self.cur_flags |= FLAG_HIDDEN,
                9 => self.cur_flags |= FLAG_STRIKETHROUGH,
                22 => self.cur_flags &= !(FLAG_BOLD | FLAG_DIM),
                23 => self.cur_flags &= !FLAG_ITALIC,
                24 => self.cur_flags &= !FLAG_UNDERLINE,
                27 => self.cur_flags &= !FLAG_INVERSE,
                28 => self.cur_flags &= !FLAG_HIDDEN,
                29 => self.cur_flags &= !FLAG_STRIKETHROUGH,
                30..=37 => self.cur_fg = ANSI_PALETTE[(p - 30) as usize],
                38 => {
                    if i + 2 < params.len() && params[i + 1] == 5 {
                        self.cur_fg = get_256_color(params[i + 2] as u8);
                        i += 2;
                    } else if i + 4 < params.len() && params[i + 1] == 2 {
                        let r = (params[i + 2] as u32).min(255);
                        let g = (params[i + 3] as u32).min(255);
                        let b = (params[i + 4] as u32).min(255);
                        self.cur_fg = (r << 16) | (g << 8) | b;
                        i += 4;
                    }
                }
                39 => self.cur_fg = DEFAULT_FG,
                40..=47 => self.cur_bg = ANSI_PALETTE[(p - 40) as usize],
                48 => {
                    if i + 2 < params.len() && params[i + 1] == 5 {
                        self.cur_bg = get_256_color(params[i + 2] as u8);
                        i += 2;
                    } else if i + 4 < params.len() && params[i + 1] == 2 {
                        let r = (params[i + 2] as u32).min(255);
                        let g = (params[i + 3] as u32).min(255);
                        let b = (params[i + 4] as u32).min(255);
                        self.cur_bg = (r << 16) | (g << 8) | b;
                        i += 4;
                    }
                }
                49 => self.cur_bg = DEFAULT_BG,
                90..=97 => self.cur_fg = ANSI_PALETTE[(p - 90 + 8) as usize],
                100..=107 => self.cur_bg = ANSI_PALETTE[(p - 100 + 8) as usize],
                _ => {}
            }
            i += 1;
        }
    }
}

pub struct VtHandler<'a> {
    pub term: &'a mut Terminal,
    pub osc_buf: Vec<u8>,
}

impl<'a> vte::Perform for VtHandler<'a> {
    fn print(&mut self, c: char) {
        self.term.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0b | 0x0c => self.term.line_feed(),
            b'\r' => self.term.cursor.col = 0,
            0x08 => self.term.cursor.col = self.term.cursor.col.saturating_sub(1),
            b'\t' => {
                let tab_stop = (self.term.cursor.col / 8 + 1) * 8;
                let cols = self.term.grid().cols;
                self.term.cursor.col = tab_stop.min(cols.saturating_sub(1));
            }
            0x07 => {} // Bell
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let is_private = intermediates.first() == Some(&b'?');
        let p_vec: Vec<i64> = params
            .iter()
            .map(|p| p.first().copied().unwrap_or(0) as i64)
            .collect();
        let p1 = p_vec.first().copied().unwrap_or(0).max(1) as usize;
        let cols = self.term.grid().cols;
        let rows = self.term.grid().rows;

        if is_private {
            match action {
                'h' => {
                    for &p in &p_vec {
                        match p {
                            25 => self.term.cursor.visible = true,
                            47 | 1049 if !self.term.is_alt => {
                                self.term.saved_cursor = self.term.cursor.clone();
                                self.term.is_alt = true;
                                self.term.alt_grid.clear();
                            }
                            _ => {}
                        }
                    }
                }
                'l' => {
                    for &p in &p_vec {
                        match p {
                            25 => self.term.cursor.visible = false,
                            47 | 1049 if self.term.is_alt => {
                                self.term.is_alt = false;
                                self.term.cursor = self.term.saved_cursor.clone();
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        match action {
            'A' => self.term.cursor.row = self.term.cursor.row.saturating_sub(p1),
            'B' => self.term.cursor.row = (self.term.cursor.row + p1).min(rows.saturating_sub(1)),
            'C' => self.term.cursor.col = (self.term.cursor.col + p1).min(cols.saturating_sub(1)),
            'D' => self.term.cursor.col = self.term.cursor.col.saturating_sub(p1),
            'E' => {
                self.term.cursor.col = 0;
                self.term.cursor.row = (self.term.cursor.row + p1).min(rows.saturating_sub(1));
            }
            'F' => {
                self.term.cursor.col = 0;
                self.term.cursor.row = self.term.cursor.row.saturating_sub(p1);
            }
            'G' | '`' => self.term.cursor.col = (p1.saturating_sub(1)).min(cols.saturating_sub(1)),
            'H' | 'f' => {
                let r = p_vec.first().copied().unwrap_or(1).max(1).saturating_sub(1) as usize;
                let c = p_vec.get(1).copied().unwrap_or(1).max(1).saturating_sub(1) as usize;
                self.term.cursor.row = r.min(rows.saturating_sub(1));
                self.term.cursor.col = c.min(cols.saturating_sub(1));
            }
            'J' => {
                let mode = p_vec.first().copied().unwrap_or(0);
                let cur_r = self.term.cursor.row;
                let cur_c = self.term.cursor.col;
                match mode {
                    0 => {
                        // cursor to end of screen
                        for c in cur_c..cols {
                            if cur_r < rows {
                                self.term.grid_mut().cells[cur_r][c] = Cell::default();
                            }
                        }
                        for r in (cur_r + 1)..rows {
                            for c in 0..cols {
                                self.term.grid_mut().cells[r][c] = Cell::default();
                            }
                        }
                    }
                    1 => {
                        // start of screen to cursor
                        for r in 0..cur_r {
                            for c in 0..cols {
                                self.term.grid_mut().cells[r][c] = Cell::default();
                            }
                        }
                        for c in 0..=cur_c.min(cols - 1) {
                            if cur_r < rows {
                                self.term.grid_mut().cells[cur_r][c] = Cell::default();
                            }
                        }
                    }
                    2 | 3 => {
                        self.term.grid_mut().clear();
                        if mode == 3 {
                            self.term.main_grid.scrollback.clear();
                        }
                    }
                    _ => {}
                }
            }
            'K' => {
                let mode = p_vec.first().copied().unwrap_or(0);
                let r = self.term.cursor.row;
                let cur_c = self.term.cursor.col;
                if r < rows {
                    match mode {
                        0 => {
                            for c in cur_c..cols {
                                self.term.grid_mut().cells[r][c] = Cell::default();
                            }
                        }
                        1 => {
                            for c in 0..=cur_c.min(cols - 1) {
                                self.term.grid_mut().cells[r][c] = Cell::default();
                            }
                        }
                        2 => {
                            for c in 0..cols {
                                self.term.grid_mut().cells[r][c] = Cell::default();
                            }
                        }
                        _ => {}
                    }
                }
            }
            'L' => {
                // Insert lines
                let top = self.term.cursor.row;
                let bottom = self.term.scroll_bottom;
                for _ in 0..p1 {
                    self.term.grid_mut().scroll_down(top, bottom);
                }
            }
            'M' => {
                // Delete lines
                let top = self.term.cursor.row;
                let bottom = self.term.scroll_bottom;
                for _ in 0..p1 {
                    self.term.grid_mut().scroll_up(top, bottom);
                }
            }
            'P' => {
                // Delete characters
                let r = self.term.cursor.row;
                let c = self.term.cursor.col;
                if r < rows && c < cols {
                    for col in c..cols {
                        let next_c = col + p1;
                        self.term.grid_mut().cells[r][col] = if next_c < cols {
                            self.term.grid().cells[r][next_c]
                        } else {
                            Cell::default()
                        };
                    }
                }
            }
            'S' => {
                // Scroll up
                let top = self.term.scroll_top;
                let bottom = self.term.scroll_bottom;
                for _ in 0..p1 {
                    self.term.grid_mut().scroll_up(top, bottom);
                }
            }
            'T' => {
                // Scroll down
                let top = self.term.scroll_top;
                let bottom = self.term.scroll_bottom;
                for _ in 0..p1 {
                    self.term.grid_mut().scroll_down(top, bottom);
                }
            }
            'X' => {
                // Erase characters
                let r = self.term.cursor.row;
                let c = self.term.cursor.col;
                if r < rows {
                    for col in c..(c + p1).min(cols) {
                        self.term.grid_mut().cells[r][col] = Cell::default();
                    }
                }
            }
            'm' => self.term.handle_sgr(&p_vec),
            'r' => {
                // Set scrolling margins
                let top = p_vec.first().copied().unwrap_or(1).max(1).saturating_sub(1) as usize;
                let bottom = p_vec
                    .get(1)
                    .copied()
                    .unwrap_or(rows as i64)
                    .max(1)
                    .saturating_sub(1) as usize;
                if top < bottom && bottom < rows {
                    self.term.scroll_top = top;
                    self.term.scroll_bottom = bottom;
                }
            }
            's' => self.term.saved_cursor = self.term.cursor.clone(),
            'u' => self.term.cursor = self.term.saved_cursor.clone(),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }
        let code = std::str::from_utf8(params[0])
            .unwrap_or("")
            .parse::<u32>()
            .unwrap_or(999);
        match code {
            0 | 2 => {
                if let Some(title_bytes) = params.get(1) {
                    if let Ok(title) = std::str::from_utf8(title_bytes) {
                        self.term.title = title.to_string();
                    }
                }
            }
            7 => {
                if let Some(uri_bytes) = params.get(1) {
                    if let Ok(uri) = std::str::from_utf8(uri_bytes) {
                        self.term.cwd = uri.to_string();
                    }
                }
            }
            _ => {}
        }
    }
}
