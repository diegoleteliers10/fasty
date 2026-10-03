#[cfg(test)]
mod tests {
    use crate::vt::{Terminal, VtHandler, DEFAULT_FG, FLAG_BOLD};

    #[test]
    fn test_terminal_basic_echo() {
        let mut term = Terminal::new(80, 24, 100);
        let mut parser = vte::Parser::new();
        let mut handler = VtHandler {
            term: &mut term,
            osc_buf: Vec::new(),
        };

        parser.advance(&mut handler, b"Hello, fastty Wasm!\r\nSecond Line");

        assert_eq!(term.grid().cells[0][0].c, 'H');
        assert_eq!(term.grid().cells[0][1].c, 'e');
        assert_eq!(term.grid().cells[1][0].c, 'S');
        assert_eq!(term.cursor.row, 1);
        assert_eq!(term.cursor.col, 11);
    }

    #[test]
    fn test_sgr_colors_and_flags() {
        let mut term = Terminal::new(80, 24, 100);
        let mut parser = vte::Parser::new();
        let mut handler = VtHandler {
            term: &mut term,
            osc_buf: Vec::new(),
        };

        // Bold red text: \x1b[1;31mBoldRed\x1b[0m
        parser.advance(&mut handler, b"\x1b[1;31mBoldRed\x1b[0m");

        let cell = term.grid().cells[0][0];
        assert_eq!(cell.c, 'B');
        assert_ne!(cell.fg, DEFAULT_FG);
        assert_eq!(cell.flags & FLAG_BOLD, FLAG_BOLD);

        // Reset
        let reset_cell = term.grid().cells[0][7];
        assert_eq!(reset_cell.flags & FLAG_BOLD, 0);
    }

    #[test]
    fn test_scrollback() {
        let mut term = Terminal::new(80, 5, 50);
        let mut parser = vte::Parser::new();
        let mut handler = VtHandler {
            term: &mut term,
            osc_buf: Vec::new(),
        };

        for i in 0..10 {
            let line = format!("Line {}\r\n", i);
            parser.advance(&mut handler, line.as_bytes());
        }

        assert!(!term.main_grid.scrollback.is_empty());
    }

    #[test]
    fn test_cursor_movement() {
        let mut term = Terminal::new(80, 24, 100);
        let mut parser = vte::Parser::new();
        let mut handler = VtHandler {
            term: &mut term,
            osc_buf: Vec::new(),
        };

        // Move to row 5, col 10 (1-based: \x1b[5;10H)
        parser.advance(&mut handler, b"\x1b[5;10H");

        assert_eq!(term.cursor.row, 4);
        assert_eq!(term.cursor.col, 9);
    }

    #[test]
    fn test_restore_binary_snapshot() {
        let mut term = Terminal::new(80, 24, 100);
        let mut data = Vec::new();
        // Header (32 bytes)
        data.extend_from_slice(b"FST1"); // magic
        data.extend_from_slice(&1u16.to_le_bytes()); // version
        data.extend_from_slice(&2u16.to_le_bytes()); // flags (cursor visible)
        data.extend_from_slice(&2u16.to_le_bytes()); // cols = 2
        data.extend_from_slice(&1u16.to_le_bytes()); // rows = 1
        data.extend_from_slice(&1u16.to_le_bytes()); // cursor_col = 1
        data.extend_from_slice(&0u16.to_le_bytes()); // cursor_row = 0
        data.extend_from_slice(&2u32.to_le_bytes()); // cell_count = 2
        data.push(0); // cursor_style
        data.push(0); // reserved1
        data.extend_from_slice(&[0u8; 10]); // reserved2

        // Cell 0: 'A'
        data.extend_from_slice(&('A' as u32).to_le_bytes());
        data.extend_from_slice(&0x00FF0000u32.to_le_bytes());
        data.extend_from_slice(&0x00000000u32.to_le_bytes());
        data.extend_from_slice(&(1u16).to_le_bytes()); // bold
        data.extend_from_slice(&0u16.to_le_bytes());

        // Cell 1: 'B'
        data.extend_from_slice(&('B' as u32).to_le_bytes());
        data.extend_from_slice(&0x0000FF00u32.to_le_bytes());
        data.extend_from_slice(&0x00000000u32.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());

        assert!(term.restore_binary_snapshot(&data));
        assert_eq!(term.grid().cols, 2);
        assert_eq!(term.grid().rows, 1);
        assert_eq!(term.grid().cells[0][0].c, 'A');
        assert_eq!(term.grid().cells[0][0].fg, 0x00FF0000);
        assert_eq!(term.grid().cells[0][0].flags, FLAG_BOLD);
        assert_eq!(term.grid().cells[0][1].c, 'B');
        assert_eq!(term.cursor.col, 1);
    }

    #[test]
    fn test_restore_binary_snapshot_compressed() {
        let mut term = Terminal::new(80, 24, 100);
        let mut data = Vec::new();
        // Header (32 bytes) with flags: cursor visible (2) | deflate (4) = 6
        data.extend_from_slice(b"FST1"); // magic
        data.extend_from_slice(&1u16.to_le_bytes()); // version
        data.extend_from_slice(&6u16.to_le_bytes()); // flags: cursor visible | deflate
        data.extend_from_slice(&2u16.to_le_bytes()); // cols = 2
        data.extend_from_slice(&1u16.to_le_bytes()); // rows = 1
        data.extend_from_slice(&1u16.to_le_bytes()); // cursor_col = 1
        data.extend_from_slice(&0u16.to_le_bytes()); // cursor_row = 0
        data.extend_from_slice(&2u32.to_le_bytes()); // cell_count = 2
        data.push(0); // cursor_style
        data.push(0); // reserved1
        data.extend_from_slice(&[0u8; 10]); // reserved2

        // Raw cells (32 bytes)
        let mut raw_cells = Vec::new();
        // Cell 0: 'X'
        raw_cells.extend_from_slice(&('X' as u32).to_le_bytes());
        raw_cells.extend_from_slice(&0x00FF0000u32.to_le_bytes());
        raw_cells.extend_from_slice(&0x00000000u32.to_le_bytes());
        raw_cells.extend_from_slice(&(1u16).to_le_bytes()); // bold
        raw_cells.extend_from_slice(&0u16.to_le_bytes());

        // Cell 1: 'Y'
        raw_cells.extend_from_slice(&('Y' as u32).to_le_bytes());
        raw_cells.extend_from_slice(&0x0000FF00u32.to_le_bytes());
        raw_cells.extend_from_slice(&0x00000000u32.to_le_bytes());
        raw_cells.extend_from_slice(&0u16.to_le_bytes());
        raw_cells.extend_from_slice(&0u16.to_le_bytes());

        let compressed_cells = miniz_oxide::deflate::compress_to_vec(&raw_cells, 6);
        data.extend_from_slice(&compressed_cells);

        assert!(term.restore_binary_snapshot(&data));
        assert_eq!(term.grid().cols, 2);
        assert_eq!(term.grid().rows, 1);
        assert_eq!(term.grid().cells[0][0].c, 'X');
        assert_eq!(term.grid().cells[0][0].fg, 0x00FF0000);
        assert_eq!(term.grid().cells[0][0].flags, FLAG_BOLD);
        assert_eq!(term.grid().cells[0][1].c, 'Y');
        assert_eq!(term.cursor.col, 1);
    }

    /// Builds an FST1 v2 header for a 2-col grid with `history_rows`
    /// scrollback rows and 1 screen row.
    fn v2_header(history_rows: u32, cell_count: u32, flags: u16) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"FST1");
        data.extend_from_slice(&2u16.to_le_bytes()); // version
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes()); // cols
        data.extend_from_slice(&1u16.to_le_bytes()); // rows
        data.extend_from_slice(&1u16.to_le_bytes()); // cursor_col
        data.extend_from_slice(&0u16.to_le_bytes()); // cursor_row
        data.extend_from_slice(&cell_count.to_le_bytes());
        data.push(0); // cursor_style
        data.push(0); // reserved1
        data.extend_from_slice(&history_rows.to_le_bytes()); // history rows
        data.extend_from_slice(&[0u8; 6]); // reserved
        data
    }

    fn cell_bytes(c: char) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(c as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn test_restore_binary_snapshot_v2_with_history() {
        let mut term = Terminal::new(80, 24, 100);
        // 2 history rows + 1 screen row, 2 cols each, deflated.
        let mut raw = Vec::new();
        for ch in ['a', '0', 'b', '1'] {
            raw.extend(cell_bytes(ch));
        }
        raw.extend(cell_bytes('A'));
        raw.extend(cell_bytes('B'));

        let mut data = v2_header(2, 6, 6); // flags: cursor visible | deflate
        data.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(&raw, 6));

        assert!(term.restore_binary_snapshot(&data));
        assert_eq!(term.main_grid.scrollback.len(), 2);
        // Oldest first: row 0 is 'a0', the newest history row is 'b1'.
        assert_eq!(term.main_grid.scrollback[0][0].c, 'a');
        assert_eq!(term.main_grid.scrollback[0][1].c, '0');
        assert_eq!(term.main_grid.scrollback[1][0].c, 'b');
        // Screen cells land after the history rows in the payload.
        assert_eq!(term.grid().cells[0][0].c, 'A');
        assert_eq!(term.grid().cells[0][1].c, 'B');
    }

    #[test]
    fn test_restore_binary_snapshot_v2_caps_history() {
        let mut term = Terminal::new(80, 24, 3); // max_scrollback = 3
        // 5 history rows + 1 screen row.
        let mut raw = Vec::new();
        for ch in ['0', 'x', '1', 'x', '2', 'x', '3', 'x', '4', 'x'] {
            raw.extend(cell_bytes(ch));
        }
        raw.extend(cell_bytes('S'));
        raw.extend(cell_bytes('S'));

        let mut data = v2_header(5, 12, 6);
        data.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(&raw, 6));

        assert!(term.restore_binary_snapshot(&data));
        // Only the newest 3 history rows fit.
        assert_eq!(term.main_grid.scrollback.len(), 3);
        assert_eq!(term.main_grid.scrollback[0][0].c, '2');
        assert_eq!(term.main_grid.scrollback[2][0].c, '4');
        assert_eq!(term.grid().cells[0][0].c, 'S');
    }
}
