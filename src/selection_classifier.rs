use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Point;

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
}
