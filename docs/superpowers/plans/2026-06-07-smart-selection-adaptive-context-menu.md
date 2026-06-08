# Smart Selection + Adaptive Context Menu Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Double-click on terminal output classifies the token (URL/email/path/hex/word) and selects+copies it; right-click context menu adapts to the classification with relevant actions (Open link, Cd here, Open in editor, Open email, Copy hex).

**Architecture:** Pure-Rust classifier module `src/selection_classifier.rs` exposes a `Classification` ADT and pure functions `is_url/is_email/is_path/is_hex/classify_token`. Double-click is detected by timing the terminal-area mouse-down; right-click runs the classifier and stores the result so the click-dispatch can act on it. No new crates, no atlas/icon changes (Unicode emoji like existing menu items).

**Tech Stack:** Rust 2021, `alacritty_terminal 0.24` (`Grid`/`Point`/`Line`/`Column`), existing winit mouse pipeline. Per-tab cwd lookup already exists at `src/main.rs:301` (`tab_live_cwd`, Linux-only); used to resolve relative paths before spawning a new tab.

**Scope note:** This plan covers the menu + selection UX. OSC 7 cwd tracking and OSC 133 exit code are out of scope (separate plan). Path resolution falls back to absolute-only on Windows/macOS, full resolution on Linux.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/selection_classifier.rs` (new) | `Classification` ADT + pure detector fns + inline `#[cfg(test)]` tests |
| `src/lib.rs` (modify) | Declare `pub mod selection_classifier;` |
| `src/renderer/mod.rs` (modify) | Extend `ContextMenuItem` enum with 7 new variants |
| `src/main.rs` (modify) | State (`last_term_click_*`, `context_menu_classification`); double-click branch; right-click classifier call; `build_smart_menu`; dispatch arms for new items |
| `src/renderer/pipeline.rs` (modify) | Label/icon map for the 7 new `ContextMenuItem` variants |

No new files in `src/renderer/` and no changes to atlas, config, or session.

---

## Task 1: Classification ADT + pure detector helpers

**Files:**
- Create: `src/selection_classifier.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/selection_classifier.rs
use alacritty_terminal::index::{Column, Line, Point};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Classification {
    Url(String),
    Email(String),
    Path(String),
    Hex(String),
    Word(String),
}

pub fn is_url(token: &str) -> bool {
    todo!()
}

pub fn is_email(token: &str) -> bool {
    todo!()
}

pub fn is_path(token: &str) -> bool {
    todo!()
}

pub fn is_hex(token: &str) -> bool {
    todo!()
}

pub fn classify_token(token: &str) -> Option<Classification> {
    todo!()
}

#[allow(dead_code)]
pub fn classify_at_point(
    grid: &alacritty_terminal::grid::Grid,
    point: Point,
    shell_cols: usize,
) -> Option<Classification> {
    let _ = (grid, point, shell_cols);
    todo!()
}

pub fn extract_token(
    grid: &alacritty_terminal::grid::Grid,
    point: Point,
    shell_cols: usize,
) -> Option<(String, usize, usize)> {
    let _ = (grid, point, shell_cols);
    todo!()
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
        assert!(matches!(classify_token("https://x.com"), Some(Classification::Url(_))));
        assert!(matches!(classify_token("a@b.co"), Some(Classification::Email(_))));
        assert!(matches!(classify_token("/usr/bin"), Some(Classification::Path(_))));
        assert!(matches!(classify_token("deadbeef"), Some(Classification::Hex(_))));
        assert!(matches!(classify_token("hello"), Some(Classification::Word(_))));
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --lib selection_classifier 2>&1 | tail -20`
Expected: compile errors because `is_url` etc. are `todo!()`.

- [ ] **Step 3: Implement the helpers**

Replace the `todo!()` bodies:

```rust
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
    if !local.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')) {
        return false;
    }
    let dot = domain.find('.');
    match dot {
        Some(i) if i > 0 && i < domain.len() - 1 => true,
        _ => false,
    }
}

pub fn is_path(token: &str) -> bool {
    if token.is_empty() {
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
        let last_dot = token.rfind('.').unwrap();
        let ext = &token[last_dot + 1..];
        if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.len() <= 8 {
            return true;
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
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --lib selection_classifier 2>&1 | tail -20`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add src/selection_classifier.rs
git commit -m "feat(selection): add Classification ADT and pure detector helpers"
```

---

## Task 2: `extract_token` from grid (word boundaries)

**Files:**
- Modify: `src/selection_classifier.rs`

- [ ] **Step 1: Add the failing test**

Add inside the `#[cfg(test)] mod tests` block:

```rust
    use alacritty_terminal::index::{Column, Line, Point};
    use alacritty_terminal::term::Config;
    use alacritty_terminal::Term;

    fn grid_with_row(text: &str) -> (alacritty_terminal::grid::Grid, usize) {
        let size = alacritty_terminal::index::Point::new(Line(0), Column(80));
        let config = Config::default();
        let mut term = Term::new(config, &size, alacritty_terminal::term::TermDamage::Full);
        let bytes: Vec<u8> = text.bytes().chain(std::iter::repeat(b' ')).take(80).collect();
        for (i, &b) in bytes.iter().enumerate() {
            term.input(b);
            let _ = i;
        }
        let cols = 80;
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
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test --lib selection_classifier::tests::extract_token_picks_word_around_point 2>&1 | tail -15`
Expected: compile error — `extract_token` not found.

- [ ] **Step 3: Implement `extract_token`**

Add above the `mod tests`:

```rust
const DELIMITERS: &[char] = &[' ', '\0', '"', '\'', '`', '<', '>', '(', ')', '{', '}', '[', ']'];
const TRAILING_PUNCT: &[char] = &[',', '.', ';', ':', '?', '!', ')', ']', '}'];

pub fn extract_token(grid: &alacritty_terminal::grid::Grid, point: Point, shell_cols: usize) -> Option<(String, usize, usize)> {
    let line_idx = point.line.0;
    if line_idx < 0 {
        return None;
    }
    let line_us = line_idx as usize;
    if line_us >= grid.len() {
        return None;
    }
    let row = &grid[alacritty_terminal::index::Line(line_us)];
    let col = point.column.0.min(shell_cols.saturating_sub(1));

    let line_str: String = (0..shell_cols)
        .map(|i| row[alacritty_terminal::index::Column(i)].c)
        .collect();

    let bytes = line_str.as_bytes();
    let mut start = col;
    while start > 0 && !DELIMITERS.contains(&bytes[start - 1] as &char) {
        start -= 1;
    }
    let mut end = col;
    while end < shell_cols && !DELIMITERS.contains(&bytes[end] as &char) {
        end += 1;
    }
    if end > start {
        while end > start && TRAILING_PUNCT.contains(&bytes[end - 1] as &char) {
            end -= 1;
        }
    }
    if start == end {
        return None;
    }
    Some((line_str[start..end].to_string(), start, end))
}
```

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test --lib selection_classifier::tests::extract_token_picks_word_around_point 2>&1 | tail -15`
Expected: 1 passed. If signature of `Term::input` or `Config::default` differs in 0.24, adjust to match; do NOT skip the test.

- [ ] **Step 5: Commit**

```bash
git add src/selection_classifier.rs
git commit -m "feat(selection): extract token at grid point with word boundaries"
```

---

## Task 3: `classify_at_point` orchestrator

**Files:**
- Modify: `src/selection_classifier.rs`

- [ ] **Step 1: Add the failing test**

Inside `mod tests`:

```rust
    #[test]
    fn classify_at_point_returns_url() {
        let (grid, cols) = grid_with_row("visit https://example.com today");
        let p = Point::new(Line(0), Column(10));
        match super::classify_at_point(&grid, p, cols) {
            Some(Classification::Url(s)) => assert_eq!(s, "https://example.com"),
            other => panic!("expected Url, got {:?}", other),
        }
    }
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test --lib selection_classifier::tests::classify_at_point_returns_url 2>&1 | tail -15`
Expected: `classify_at_point` not implemented.

- [ ] **Step 3: Implement orchestrator**

Replace the `todo!()` body of `classify_at_point`:

```rust
pub fn classify_at_point(
    grid: &alacritty_terminal::grid::Grid,
    point: Point,
    shell_cols: usize,
) -> Option<Classification> {
    let (token, _, _) = extract_token(grid, point, shell_cols)?;
    classify_token(&token)
}
```

- [ ] **Step 4: Run all module tests**

Run: `cargo test --lib selection_classifier 2>&1 | tail -15`
Expected: 7 passed (5 from Task 1 + 1 extract + 1 classify_at_point).

- [ ] **Step 5: Commit**

```bash
git add src/selection_classifier.rs
git commit -m "feat(selection): add classify_at_point orchestrator"
```

---

## Task 4: Wire module into `src/lib.rs`

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Add the module declaration**

Add `pub mod selection_classifier;` to `src/lib.rs` in alphabetical order (between `renderer` and `session`):

```rust
//! fasty library - winit/wgpu terminal emulator

pub mod config;
pub mod event_listener;
pub mod git;
pub mod keybindings;
pub mod pty;
pub mod renderer;
pub mod selection_classifier;
pub mod session;
pub mod terminal_state;
```

- [ ] **Step 2: Compile**

Run: `cargo build 2>&1 | tail -10`
Expected: clean build, no warnings from the new module.

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "chore: declare selection_classifier module"
```

---

## Task 5: Extend `ContextMenuItem` enum

**Files:**
- Modify: `src/renderer/mod.rs:621-629` (the enum)
- Modify: `src/renderer/pipeline.rs:2311-2318` (label/icon map; add default Unicode arms so compile is clean)

- [ ] **Step 1: Add new variants**

In `src/renderer/mod.rs`, replace the enum:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuItem {
    Copy,
    Paste,
    Separator,
    NewTab,
    CloseTab,
    About,
    OpenLink,
    CopyWord,
    CopyLine,
    CdHere,
    OpenInEditor,
    OpenEmail,
    CopyHex,
}
```

- [ ] **Step 2: Add label/icon map arms**

In `src/renderer/pipeline.rs` at the `match item` (around line 2311), add a `match` arm that covers the new variants with Unicode placeholders (we'll refine labels in Task 10):

```rust
let (icon, label, shortcut) = match item {
    ContextMenuItem::Copy          => ("📋", "Copiar",          Some("⌘C")),
    ContextMenuItem::Paste         => ("📋", "Pegar",           Some("⌘V")),
    ContextMenuItem::NewTab        => ("+",  "Nueva pestaña",   None),
    ContextMenuItem::CloseTab      => ("\u{2715}", "Cerrar pestaña", None),
    ContextMenuItem::About         => ("",   "About",           None),
    ContextMenuItem::OpenLink      => ("🔗", "Open link",       Some("⌘-click")),
    ContextMenuItem::CopyWord      => ("📋", "Copy word",       None),
    ContextMenuItem::CopyLine      => ("📋", "Copy line",       None),
    ContextMenuItem::CdHere        => ("📁", "cd here in new tab", None),
    ContextMenuItem::OpenInEditor  => ("✏️",  "Open in editor",  None),
    ContextMenuItem::OpenEmail     => ("✉️",  "Compose email",   None),
    ContextMenuItem::CopyHex       => ("#",  "Copy hex",        None),
    ContextMenuItem::Separator     => ("",   "",                None),
};
```

- [ ] **Step 3: Compile**

Run: `cargo build 2>&1 | tail -15`
Expected: clean build. (Exhaustive match — no `_` needed once all variants are listed.)

- [ ] **Step 4: Commit**

```bash
git add src/renderer/mod.rs src/renderer/pipeline.rs
git commit -m "feat(menu): extend ContextMenuItem with smart-selection variants"
```

---

## Task 6: Add double-click state variables

**Files:**
- Modify: `src/main.rs` (state block around line 706)

- [ ] **Step 1: Add state**

Add immediately after `let mut last_click_time: Option<std::time::Instant> = None;` (around line 706):

```rust
let mut last_term_click_time: Option<std::time::Instant> = None;
let mut last_term_click_cell: Option<(i32, usize)> = None;
```

- [ ] **Step 2: Compile**

Run: `cargo build 2>&1 | tail -10`
Expected: clean (unused-warning is fine; will be used in Task 7).

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "chore(selection): add double-click detection state"
```

---

## Task 7: Double-click → classify → set selection + auto-copy

**Files:**
- Modify: `src/main.rs` (the "arm selection" `else` branch around line 2791)

- [ ] **Step 1: Replace the `else` arm**

Find the `else` branch at the end of the left-button handling (around `src/main.rs:2791-2794`):

```rust
} else {
    tabs[active_tab_index].selection_start_pos = Some((current_mouse_x, current_mouse_y));
    tabs[active_tab_index].is_selecting_text = false;
}
```

Replace with:

```rust
} else {
    let click_point = mouse_to_grid_point(
        current_mouse_x, current_mouse_y,
        cell_width, cell_height,
        tabs[active_tab_index].scroll_target,
        tabs[active_tab_index].last_actual_offset,
        shell_cols, shell_rows,
        padding_top,
    );
    let in_history = click_point.line.0 < -(tabs[active_tab_index].terminal_state.lock().term().lock().history_size() as i32);
    if !in_history {
        let now = std::time::Instant::now();
        let same_cell = last_term_click_cell == Some((click_point.line.0, click_point.column.0));
        let is_double = same_cell
            && last_term_click_time
                .map(|t| now.duration_since(t) < std::time::Duration::from_millis(300))
                .unwrap_or(false);
        last_term_click_time = Some(now);
        last_term_click_cell = Some((click_point.line.0, click_point.column.0));

        if is_double {
            use crate::selection_classifier::extract_token;
            let token_info = {
                let term = tabs[active_tab_index].terminal_state.lock();
                let term_guard = term.term().lock();
                extract_token(term_guard.grid(), click_point, shell_cols)
            };
            if let Some((token, start_col, end_col)) = token_info {
                let start = alacritty_terminal::index::Point::new(click_point.line, alacritty_terminal::index::Column(start_col));
                let end = alacritty_terminal::index::Point::new(click_point.line, alacritty_terminal::index::Column(end_col));
                tabs[active_tab_index].selection = Some(renderer::Selection { start, end });
                copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, tabs[active_tab_index].selection.unwrap(), shell_cols, shell_rows, &mut clipboard);
                toast = Some(("\u{2713}  Text copied".to_string(), std::time::Instant::now(), 1920));
                let _ = token;
            }
        } else {
            tabs[active_tab_index].selection_start_pos = Some((current_mouse_x, current_mouse_y));
            tabs[active_tab_index].is_selecting_text = false;
        }
    } else {
        tabs[active_tab_index].selection_start_pos = Some((current_mouse_x, current_mouse_y));
        tabs[active_tab_index].is_selecting_text = false;
    }
}
```

Note: `mouse_to_grid_point` already exists in `src/main.rs` (line 4430). No import is required since the call site is in the same file.

- [ ] **Step 2: Compile**

Run: `cargo build 2>&1 | tail -20`
Expected: clean build. If `Point::line` field-access issues arise, wrap with `Point::line.0`; should be field access on the public `line` field.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs src/renderer/mod.rs
git commit -m "feat(selection): double-click classifies token and auto-copies"
```

---

## Task 8: Right-click classification + smart menu builder

**Files:**
- Modify: `src/main.rs` (state block; right-click branch; new `build_smart_menu`)

- [ ] **Step 1: Add menu state**

After the `context_menu_visible` block (around line 728), add:

```rust
let mut context_menu_classification: Option<crate::selection_classifier::Classification> = None;
```

- [ ] **Step 2: Add `build_smart_menu`**

Add below `get_context_menu_items` (around line 4818):

```rust
fn build_smart_menu(
    classification: Option<&crate::selection_classifier::Classification>,
    has_selection: bool,
    tabs_len: usize,
    cwd_resolvable: bool,
) -> Vec<renderer::ContextMenuItem> {
    use crate::selection_classifier::Classification;
    use renderer::ContextMenuItem::*;
    let mut items = Vec::new();
    if has_selection {
        items.push(Copy);
    }
    match classification {
        Some(Classification::Url(_))   => items.push(OpenLink),
        Some(Classification::Email(_)) => items.push(OpenEmail),
        Some(Classification::Path(p))  => {
            items.push(CopyWord);
            if cwd_resolvable || p.starts_with('/') {
                items.push(CdHere);
            }
            items.push(OpenInEditor);
        }
        Some(Classification::Hex(_))   => items.push(CopyHex),
        Some(Classification::Word(_))  => items.push(CopyWord),
        None => {}
    }
    items.push(Paste);
    items.push(Separator);
    items.push(NewTab);
    if tabs_len > 1 {
        items.push(CloseTab);
    }
    items
}
```

- [ ] **Step 3: Wire right-click branch**

In the right-click branch (around `src/main.rs:2953-2976`), replace the line that builds the menu:

```rust
let menu_items = get_context_menu_items(&tabs, active_tab_index, false);
```

with:

```rust
let click_point = mouse_to_grid_point(
    current_mouse_x, current_mouse_y,
    cell_width, cell_height,
    tabs[active_tab_index].scroll_target,
    tabs[active_tab_index].last_actual_offset,
    shell_cols, shell_rows,
    padding_top,
);
let in_history = click_point.line.0 < -(tabs[active_tab_index].terminal_state.lock().term().lock().history_size() as i32);
let classification = if in_history {
    None
} else {
    let term = tabs[active_tab_index].terminal_state.lock();
    let term_guard = term.term().lock();
    let grid = term_guard.grid();
    crate::selection_classifier::classify_at_point(grid, click_point, shell_cols)
};
let cwd_resolvable = tab_live_cwd(&tabs[active_tab_index]).is_some();
let menu_items = build_smart_menu(
    classification.as_ref(),
    tabs[active_tab_index].selection.is_some(),
    tabs.len(),
    cwd_resolvable,
);
context_menu_classification = classification;
let (menu_w, menu_h) = get_context_menu_size(&menu_items);
```

- [ ] **Step 4: Compile**

Run: `cargo build 2>&1 | tail -20`
Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(menu): classify right-click and build adaptive menu"
```

---

## Task 9: Dispatch arms for new menu items

**Files:**
- Modify: `src/main.rs` (the click-dispatch `match` around line 2183)

- [ ] **Step 1: Add new arms**

Find the dispatch `match item { ... }` and add arms for the new variants. Locate the existing `Copy` arm (around `src/main.rs:2237`) and the `NewTab` arm (around `src/main.rs:2292`). Add new arms in between, reusing the candidate text stored in `context_menu_classification`:

```rust
ContextMenuItem::OpenLink => {
    if let Some(crate::selection_classifier::Classification::Url(u)) = context_menu_classification.as_ref() {
        open_url(u);
    }
    context_menu_visible = false;
}
ContextMenuItem::CopyWord | ContextMenuItem::CopyHex => {
    if let Some(sel) = tabs[active_tab_index].selection {
        copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
        toast = Some(("\u{2713}  Text copied".to_string(), std::time::Instant::now(), 1920));
    }
    context_menu_visible = false;
}
ContextMenuItem::CopyLine => {
    if let Some(p) = last_line_point {
        let row_start = alacritty_terminal::index::Point::new(p.line, alacritty_terminal::index::Column(0));
        let row_end = alacritty_terminal::index::Point::new(p.line, alacritty_terminal::index::Column(shell_cols.saturating_sub(1)));
        let sel = renderer::Selection { start: row_start, end: row_end };
        copy_selection_to_clipboard(&tabs[active_tab_index].terminal_state, sel, shell_cols, shell_rows, &mut clipboard);
        toast = Some(("\u{2713}  Line copied".to_string(), std::time::Instant::now(), 1920));
    }
    context_menu_visible = false;
}
ContextMenuItem::CdHere => {
    if let Some(crate::selection_classifier::Classification::Path(p)) = context_menu_classification.as_ref() {
        let resolved = if p.starts_with('/') || p.starts_with('~') {
            Some(p.clone())
        } else if let Some(cwd) = tab_live_cwd(&tabs[active_tab_index]) {
            Some(cwd.join(p).to_string_lossy().into_owned())
        } else {
            None
        };
        if let Some(cwd) = resolved {
            if let Ok(new_tab) = create_new_tab(&shell, &[], Some(&cwd), config.scrollback, config.font.clone(), cell_width, cell_height, shell_cols, shell_rows, proxy.clone()) {
                tabs.push(new_tab);
                active_tab_index = tabs.len() - 1;
            }
        }
    }
    context_menu_visible = false;
}
ContextMenuItem::OpenInEditor => {
    if let Some(crate::selection_classifier::Classification::Path(p)) = context_menu_classification.as_ref() {
        let resolved = if p.starts_with('/') || p.starts_with('~') {
            p.clone()
        } else if let Some(cwd) = tab_live_cwd(&tabs[active_tab_index]) {
            cwd.join(p).to_string_lossy().into_owned()
        } else {
            p.clone()
        };
        open_file_in_editor(&resolved);
    }
    context_menu_visible = false;
}
ContextMenuItem::OpenEmail => {
    if let Some(crate::selection_classifier::Classification::Email(e)) = context_menu_classification.as_ref() {
        let target = if e.starts_with("mailto:") { e.clone() } else { format!("mailto:{}", e) };
        open_url(&target);
    }
    context_menu_visible = false;
}
```

- [ ] **Step 2: Track `last_line_point` for `CopyLine`**

Right-click branch stores the click point into `last_line_point: Option<alacritty_terminal::index::Point>` (state variable, added near `context_menu_classification`):

```rust
let mut last_line_point: Option<alacritty_terminal::index::Point> = None;
```

In the right-click branch, set it:

```rust
last_line_point = Some(click_point);
```

Note: `CopyLine` only appears in the menu when classification is `Word` and the user may want the full row. For the initial cut, keep `CopyLine` in the menu only if you want it; the simplest path is to remove `CopyLine` from `build_smart_menu` for now and reserve it for a future triple-click feature. **Recommended: remove `CopyLine` from `build_smart_menu` to keep the first cut small.**

- [ ] **Step 3: Compile**

Run: `cargo build 2>&1 | tail -20`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(menu): dispatch actions for smart-selection items"
```

---

## Task 10: Final cleanup + tests + manual smoke

**Files:**
- Modify: `src/renderer/pipeline.rs` (refine icon glyphs if you want)
- Modify: `src/selection_classifier.rs` (no changes expected)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: all selection_classifier tests pass, no regressions in other modules.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets 2>&1 | tail -30`
Expected: no new warnings. If there are warnings, fix them in the touched files only.

- [ ] **Step 3: Manual smoke checklist**

Build and run: `cargo run --release`

- [ ] Run a shell, type `echo https://example.com`, double-click on the URL — selection covers the URL, toast "Text copied" appears.
- [ ] Right-click on a URL in the output — menu shows "Open link" above Paste.
- [ ] Click "Open link" — default browser opens.
- [ ] Right-click on a path like `/usr/bin/env` — menu shows "cd here in new tab" and "Open in editor".
- [ ] Click "cd here in new tab" (Linux) — new tab opens in `/usr/bin`; `pwd` confirms.
- [ ] Right-click on `user@example.com` — "Compose email" appears; clicking it opens the OS mail handler.
- [ ] Right-click on `0xdeadbeef` — "Copy hex" appears.
- [ ] Drag-select still works (unchanged behavior).
- [ ] Single-click arming still works (no auto-copy, no classification menu).

- [ ] **Step 4: Commit any final polish**

```bash
git add -A
git commit -m "chore(selection): final polish for smart-selection feature"
```

---

## Out of Scope (Future Plans)

- OSC 7 cwd tracking via PTY escape sequences (currently `tab_live_cwd` Linux-only, no live updates).
- OSC 133 prompt marks (command start/end, exit code in statusbar).
- Triple-click for full-line selection (only double-click in this plan).
- Editor preference config (currently always uses `open_file_in_editor`).

Each of these is a small follow-up plan building on the same `Classification` infrastructure.
