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

        for &b in b"Hello, fastty Wasm!\r\nSecond Line" {
            parser.advance(&mut handler, b);
        }

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
        for &b in b"\x1b[1;31mBoldRed\x1b[0m" {
            parser.advance(&mut handler, b);
        }

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
            for &b in line.as_bytes() {
                parser.advance(&mut handler, b);
            }
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
        for &b in b"\x1b[5;10H" {
            parser.advance(&mut handler, b);
        }

        assert_eq!(term.cursor.row, 4);
        assert_eq!(term.cursor.col, 9);
    }
}
