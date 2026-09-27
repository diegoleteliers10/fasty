use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::{Cell, Flags};

const DELIMITERS: &[char] = &[
    ' ', '\0', '"', '\'', '`', '<', '>', '(', ')', '{', '}', '[', ']',
];
const TRAILING_PUNCT: &[char] = &[',', '.', ';', ':', '?', '!', ')', ']', '}'];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Classification {
    Url(String),
    Email(String),
    Path(String),
    Hex(String),
    Word(String),
}

pub fn is_url(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.starts_with("http://")
        || token.starts_with("https://")
        || token.starts_with("ftp://")
        || token.starts_with("mailto:")
    {
        return true;
    }
    if token.starts_with("www.") && token.contains('.') {
        return true;
    }
    false
}

pub fn is_email(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let parts: Vec<&str> = token.splitn(2, '@').collect();
    if parts.len() != 2 {
        return false;
    }
    let (local, domain) = (parts[0], parts[1]);
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !local
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
    {
        return false;
    }
    domain.find('.').is_some_and(|i| i > 0 && i < domain.len() - 1)
}

pub fn is_path(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if is_url(token) {
        return false;
    }
    if token.starts_with('/') || token.starts_with("./") || token.starts_with("../") || token == ".." {
        return true;
    }
    if token.starts_with("~/") || token == "~" {
        return true;
    }
    if token.contains('/') {
        return true;
    }
    if token.contains('.') && !token.starts_with('.') {
        if let Some(last_dot) = token.rfind('.') {
            let ext = &token[last_dot + 1..];
            if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.len() <= 8 {
                return true;
            }
        }
    }
    false
}

pub fn is_hex(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let core = token.strip_prefix("0x").unwrap_or(token);
    core.len() >= 8 && core.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn classify_token(token: &str) -> Option<Classification> {
    if token.is_empty() {
        return None;
    }
    if is_url(token) {
        return Some(Classification::Url(token.to_string()));
    }
    if is_email(token) {
        return Some(Classification::Email(token.to_string()));
    }
    if is_path(token) {
        return Some(Classification::Path(token.to_string()));
    }
    if is_hex(token) {
        return Some(Classification::Hex(token.to_string()));
    }
    Some(Classification::Word(token.to_string()))
}

pub fn classify_at_point(
    grid: &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell>,
    point: Point,
    shell_cols: usize,
) -> Option<Classification> {
    let (token, _, _) = extract_token(grid, point, shell_cols)?;
    classify_token(&token)
}

pub fn extract_token(
    grid: &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell>,
    point: Point,
    shell_cols: usize,
) -> Option<(String, usize, usize)> {
    let line_idx = point.line.0;
    let screen_lines = grid.screen_lines() as i32;
    let history_size = grid.history_size() as i32;
    if screen_lines == 0 || line_idx < -history_size || line_idx >= screen_lines {
        return None;
    }
    let row = &grid[alacritty_terminal::index::Line(line_idx)];
    let shell_cols = shell_cols.min(grid.columns());
    if shell_cols == 0 {
        return None;
    }
    let col = point.column.0.min(shell_cols - 1);

    let chars: Vec<char> = (0..shell_cols)
        .map(|i| row[alacritty_terminal::index::Column(i)].c)
        .collect();

    let mut start = col;
    while start > 0 && !DELIMITERS.contains(&chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < shell_cols && !DELIMITERS.contains(&chars[end]) {
        end += 1;
    }
    if end > start {
        while end > start && TRAILING_PUNCT.contains(&chars[end - 1]) {
            end -= 1;
        }
    }
    if start == end {
        return None;
    }
    Some((chars[start..end].iter().collect(), start, end))
}

pub fn extract_hyperlink(
    grid: &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell>,
    point: Point,
    shell_cols: usize,
) -> Option<(String, usize, usize)> {
    let line_idx = point.line.0;
    let screen_lines = grid.screen_lines() as i32;
    let history_size = grid.history_size() as i32;
    if screen_lines == 0 || line_idx < -history_size || line_idx >= screen_lines {
        return None;
    }
    let row = &grid[alacritty_terminal::index::Line(line_idx)];
    let shell_cols = shell_cols.min(grid.columns());
    if shell_cols == 0 {
        return None;
    }
    let col = point.column.0.min(shell_cols - 1);
    let cell = &row[alacritty_terminal::index::Column(col)];
    let hyperlink = cell.hyperlink()?;
    let uri = hyperlink.uri().to_string();

    let mut start = col;
    while start > 0 {
        if let Some(h) = row[alacritty_terminal::index::Column(start - 1)].hyperlink() {
            if h.uri() == uri {
                start -= 1;
                continue;
            }
        }
        break;
    }
    let mut end = col + 1;
    while end < shell_cols {
        if let Some(h) = row[alacritty_terminal::index::Column(end)].hyperlink() {
            if h.uri() == uri {
                end += 1;
                continue;
            }
        }
        break;
    }
    Some((uri, start, end))
}

/// True when a line in the terminal grid contains only whitespace or null characters.
pub fn is_line_empty(grid: &Grid<Cell>, line_idx: i32, shell_cols: usize) -> bool {
    let screen_lines = grid.screen_lines() as i32;
    let history_size = grid.history_size() as i32;
    if screen_lines == 0 || line_idx < -history_size || line_idx >= screen_lines {
        return true;
    }
    let row = &grid[Line(line_idx)];
    let cols = shell_cols.min(grid.columns()).min(row.len());
    for col in 0..cols {
        let c = row[Column(col)].c;
        if !c.is_whitespace() && c != '\0' {
            return false;
        }
    }
    true
}

/// True when the given line soft-wrapped into the subsequent line.
pub fn is_line_wrapped(grid: &Grid<Cell>, line_idx: i32) -> bool {
    let screen_lines = grid.screen_lines() as i32;
    let history_size = grid.history_size() as i32;
    if screen_lines == 0 || line_idx < -history_size || line_idx >= screen_lines {
        return false;
    }
    let row = &grid[Line(line_idx)];
    if row.len() == 0 {
        return false;
    }
    row[Column(row.len() - 1)].flags.contains(Flags::WRAPLINE)
}

/// Selects the full logical line at `line_idx`, expanding upwards and downwards
/// across any lines joined by soft-wrapping (`WRAPLINE`).
pub fn extract_logical_line(
    grid: &Grid<Cell>,
    line_idx: i32,
    shell_cols: usize,
) -> (Point, Point) {
    if grid.screen_lines() == 0 {
        return (
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(0)),
        );
    }
    let screen_lines = grid.screen_lines() as i32;
    let history_size = grid.history_size() as i32;
    let min_line = -history_size;
    let max_line = screen_lines - 1;

    let clamped_line = line_idx.clamp(min_line, max_line);
    let mut start_line = clamped_line;
    let mut end_line = clamped_line;

    while start_line > min_line && is_line_wrapped(grid, start_line - 1) {
        start_line -= 1;
    }

    while end_line < max_line && is_line_wrapped(grid, end_line) {
        end_line += 1;
    }

    let end_col = shell_cols.saturating_sub(1);
    (
        Point::new(Line(start_line), Column(0)),
        Point::new(Line(end_line), Column(end_col)),
    )
}

/// Selects the full paragraph at `line_idx`, expanding upwards and downwards
/// through contiguous non-empty lines until reaching a blank line or screen edge.
pub fn extract_paragraph(
    grid: &Grid<Cell>,
    line_idx: i32,
    shell_cols: usize,
) -> (Point, Point) {
    if grid.screen_lines() == 0 {
        return (
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(0)),
        );
    }
    let screen_lines = grid.screen_lines() as i32;
    let history_size = grid.history_size() as i32;
    let min_line = -history_size;
    let max_line = screen_lines - 1;

    let clamped_line = line_idx.clamp(min_line, max_line);

    if is_line_empty(grid, clamped_line, shell_cols) {
        let end_col = shell_cols.saturating_sub(1);
        return (
            Point::new(Line(clamped_line), Column(0)),
            Point::new(Line(clamped_line), Column(end_col)),
        );
    }

    let mut start_line = clamped_line;
    let mut end_line = clamped_line;

    while start_line > min_line {
        if is_line_empty(grid, start_line - 1, shell_cols) {
            break;
        }
        start_line -= 1;
    }

    while end_line < max_line {
        if is_line_empty(grid, end_line + 1, shell_cols) {
            break;
        }
        end_line += 1;
    }

    let end_col = shell_cols.saturating_sub(1);
    (
        Point::new(Line(start_line), Column(0)),
        Point::new(Line(end_line), Column(end_col)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_detection() {
        assert!(is_url("https://example.com"));
        assert!(is_url("http://foo.bar/path?q=1"));
        assert!(is_url("https://example.com/path_with_(parens)"));
        assert!(is_url("www.example.com"));
        assert!(!is_url(""));
        assert!(!is_url("hello"));
        assert!(!is_url("/usr/bin"));
        assert!(!is_url("user@example.com"));
    }

    #[test]
    fn email_detection() {
        assert!(is_email("user@example.com"));
        assert!(is_email("a.b+tag@sub.example.co"));
        assert!(!is_email("user@"));
        assert!(!is_email("@example.com"));
        assert!(!is_email("user@example"));
        assert!(!is_email("https://example.com"));
        assert!(!is_email(""));
    }

    #[test]
    fn path_detection() {
        assert!(is_path("/usr/local/bin"));
        assert!(is_path("./relative"));
        assert!(is_path("../up/here"));
        assert!(is_path("~/dotfiles"));
        assert!(is_path("src/main.rs"));
        assert!(is_path("Cargo.toml"));
        assert!(!is_path("hello"));
        assert!(!is_path("https://x.com"));
        assert!(!is_path(""));
    }

    #[test]
    fn hex_detection() {
        assert!(is_hex("deadbeef"));
        assert!(is_hex("DEADBEEF1234"));
        assert!(is_hex("0xdeadbeef"));
        assert!(!is_hex("abcd"));
        assert!(!is_hex("hello"));
        assert!(!is_hex("/usr/bin"));
        assert!(!is_hex(""));
    }

    #[test]
    fn classify_dispatches_to_specific_variant() {
        assert!(matches!(
            classify_token("https://x.com"),
            Some(Classification::Url(_))
        ));
        assert!(matches!(
            classify_token("a@b.co"),
            Some(Classification::Email(_))
        ));
        assert!(matches!(
            classify_token("/usr/bin"),
            Some(Classification::Path(_))
        ));
        assert!(matches!(
            classify_token("deadbeef"),
            Some(Classification::Hex(_))
        ));
        assert!(matches!(classify_token("hello"), Some(Classification::Word(_))));
    }

    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column, Line, Point};
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::cell::Cell;
    use alacritty_terminal::vte::ansi::Handler;

    fn grid_with_row(text: &str) -> (alacritty_terminal::grid::Grid<Cell>, usize) {
        let cols: usize = 80;
        let rows: usize = 1;
        let size = TermSize::new(cols, rows);
        let config = Config::default();
        let mut term: Term<VoidListener> = Term::new(config, &size, VoidListener);
        for ch in text.chars() {
            term.input(ch);
        }
        (term.grid().clone(), cols)
    }

    #[test]
    fn extract_token_picks_word_around_point() {
        let (grid, cols) = grid_with_row("hello world foo");
        let p = Point::new(Line(0), Column(8));
        let (tok, start, end) = super::extract_token(&grid, p, cols).unwrap();
        assert_eq!(tok, "world");
        assert_eq!(start, 6);
        assert_eq!(end, 11);
    }

    #[test]
    fn extract_token_on_whitespace_returns_left_word() {
        let (grid, cols) = grid_with_row("hello world foo");
        let p = Point::new(Line(0), Column(5));
        let (tok, start, end) = super::extract_token(&grid, p, cols).unwrap();
        assert_eq!(tok, "hello");
        assert_eq!(start, 0);
        assert_eq!(end, 5);
    }

    #[test]
    fn extract_token_handles_click_at_col_zero() {
        let (grid, cols) = grid_with_row("hello world foo");
        let p = Point::new(Line(0), Column(0));
        let (tok, start, end) = super::extract_token(&grid, p, cols).unwrap();
        assert_eq!(tok, "hello");
        assert_eq!(start, 0);
        assert_eq!(end, 5);
    }

    #[test]
    fn extract_token_strips_trailing_punctuation() {
        let (grid, cols) = grid_with_row("foo, bar;");
        let p = Point::new(Line(0), Column(1));
        let (tok, start, end) = super::extract_token(&grid, p, cols).unwrap();
        assert_eq!(tok, "foo");
        assert_eq!(start, 0);
        assert_eq!(end, 3);
    }

    #[test]
    fn classify_at_point_returns_url() {
        let (grid, cols) = grid_with_row("visit https://example.com today");
        let p = Point::new(Line(0), Column(10));
        match super::classify_at_point(&grid, p, cols) {
            Some(Classification::Url(s)) => assert_eq!(s, "https://example.com"),
            other => panic!("expected Url, got {:?}", other),
        }
    }

    fn grid_with_multi_rows(lines: &[&str], cols: usize) -> alacritty_terminal::grid::Grid<Cell> {
        let rows = lines.len();
        let size = TermSize::new(cols, rows);
        let config = Config::default();
        let mut term: Term<VoidListener> = Term::new(config, &size, VoidListener);
        for (i, line) in lines.iter().enumerate() {
            for ch in line.chars() {
                term.input(ch);
            }
            if i + 1 < lines.len() {
                term.linefeed();
                term.carriage_return();
            }
        }
        term.grid().clone()
    }

    #[test]
    fn test_extract_paragraph_bounds() {
        let lines = [
            "# Changelog",
            "",
            "Notable changes per Fastty release. The newest section ships inside the app and",
            "appears in the \"What's new\" dialog after an update.",
            "",
            "## 0.14.4",
        ];
        let cols = 80;
        let grid = grid_with_multi_rows(&lines, cols);

        // Clicking on line 2 (first line of the paragraph)
        let (start, end) = super::extract_paragraph(&grid, 2, cols);
        assert_eq!(start.line, Line(2));
        assert_eq!(start.column, Column(0));
        assert_eq!(end.line, Line(3));
        assert_eq!(end.column, Column(cols - 1));

        // Clicking on line 3 (second line of the paragraph)
        let (start2, end2) = super::extract_paragraph(&grid, 3, cols);
        assert_eq!(start2.line, Line(2));
        assert_eq!(end2.line, Line(3));

        // Clicking on empty line 1
        let (start_empty, end_empty) = super::extract_paragraph(&grid, 1, cols);
        assert_eq!(start_empty.line, Line(1));
        assert_eq!(end_empty.line, Line(1));
    }

    #[test]
    fn test_extract_logical_line_wrapped() {
        let lines = ["first row", "second row", "third row"];
        let cols = 20;
        let mut grid = grid_with_multi_rows(&lines, cols);

        // Simulate soft-wrap between line 0 and line 1
        grid[Line(0)][Column(cols - 1)].flags.insert(Flags::WRAPLINE);

        let (start, end) = super::extract_logical_line(&grid, 0, cols);
        assert_eq!(start.line, Line(0));
        assert_eq!(end.line, Line(1));

        let (start_from_1, end_from_1) = super::extract_logical_line(&grid, 1, cols);
        assert_eq!(start_from_1.line, Line(0));
        assert_eq!(end_from_1.line, Line(1));

        // Line 2 is not wrapped
        let (start_2, end_2) = super::extract_logical_line(&grid, 2, cols);
        assert_eq!(start_2.line, Line(2));
        assert_eq!(end_2.line, Line(2));
    }

    #[test]
    fn test_empty_screen_lines_does_not_panic() {
        let size = TermSize::new(80, 0);
        let config = Config::default();
        let term: Term<VoidListener> = Term::new(config, &size, VoidListener);
        let grid = term.grid();

        let (p1, p2) = super::extract_logical_line(grid, 0, 80);
        assert_eq!(p1, Point::new(Line(0), Column(0)));
        assert_eq!(p2, Point::new(Line(0), Column(0)));

        let (p3, p4) = super::extract_paragraph(grid, 0, 80);
        assert_eq!(p3, Point::new(Line(0), Column(0)));
        assert_eq!(p4, Point::new(Line(0), Column(0)));
    }
}
