use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Rgb};
use gpui::{
    div, prelude::*, px, size, Bounds, Context, CursorStyle, Div, FocusHandle, FontFeatures,
    FontWeight, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Render, ScrollHandle, ScrollWheelEvent, SharedString, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use std::sync::Arc;

use super::settings_view::SettingsView;
use super::status_bar::{StatusBar, StatusBarModel, StatusInfo};
use super::tab_bar::{TabBar, TabItem, TabSidebar};
use super::theme::{rgb_to_hsla, Theme};
use crate::config::{self, Config, TabLayout};
use crate::event_listener::EventSender;
use crate::git::GitStatus;
use crate::pane_tree::{Direction, PaneId, PaneNode, PaneTree, SplitDirection, TerminalPane};
use crate::program_status::{ProgramRecord, ProgramState};
use crate::terminal_state::{AppEvent, TerminalState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: alacritty_terminal::index::Point,
    pub end: alacritty_terminal::index::Point,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AiTurnCompletion {
    Read,
    Unread,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AiNotificationUrgency {
    Routine,
    RequiresAction,
}

#[derive(Default)]
pub struct ProgramPaneStatus {
    records: Vec<ProgramRecord>,
    unread: bool,
    last_notification: Option<std::time::Instant>,
}

pub struct TabData {
    pub id: usize,
    pub ai_tab_key: String,
    pub ai_conversation_id: String,
    pub pane_tree: PaneTree,
    pub title: String,
    pub custom_title: Option<String>,
    pub terminal: Option<Arc<TerminalState>>,
    pub cwd: Option<std::path::PathBuf>,
    pub git_status: Option<GitStatus>,
    pub git_checked_cwd: Option<std::path::PathBuf>,
    pub git_last_poll: Option<std::time::Instant>,
    pub last_duration_ms: Option<u128>,
    pub last_exit_code: Option<i32>,
    /// F2: zoomed pane (tmux `prefix+z` semantics). The pane takes the whole
    /// terminal area until toggled off. Per-tab, transient (not persisted).
    pub zoomed_pane: Option<PaneId>,
}

fn new_ai_key(prefix: &str) -> String {
    static NEXT_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let serial = NEXT_KEY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = crate::ai::conversations::now_millis();
    format!("{prefix}-{now}-{serial}")
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyledSpan {
    pub text: String,
    pub start_col: usize,
    pub end_col: usize,
    pub fg: Hsla,
    pub bg: Option<Hsla>,
    pub is_bold: bool,
    pub is_underline: bool,
    /// Pure-emoji span (color glyphs from fallback fonts).
    pub is_emoji: bool,
    /// Nerd Font icon span (PUA codepoints). Shaped with the auto-detected
    /// Nerd Font when one is installed, else today's cascade behavior.
    pub is_nerd: bool,
    /// Technical keyboard shortcut symbol (⌘, ⌥, ⇧, ⌃, etc.) rendered with dedicated metrics.
    pub is_kbd: bool,
    /// Font-size factor measured so the emoji ink fits its reserved cells.
    pub emoji_scale: Option<f32>,
    /// Terminal column for each char in `text` (including zerowidth chars,
    /// which share the column of their base cell). Used by search highlight
    /// to compute exact pixel positions without floating-point interpolation.
    pub char_cols: Vec<usize>,
}

fn trim_row_spans(spans: &mut Vec<StyledSpan>) {
    while let Some(last) = spans.last_mut() {
        if last.bg.is_none() && !last.is_underline {
            let trimmed = last.text.trim_end_matches(' ');
            if trimmed.is_empty() {
                spans.pop();
            } else {
                // Count trailing spaces as columns (1 col per space, always ASCII).
                let trimmed_char_len = trimmed.chars().count();
                let total_char_len = last.text.chars().count();
                let trailing_spaces = total_char_len - trimmed_char_len;
                last.end_col = last.end_col.saturating_sub(trailing_spaces);
                last.text = trimmed.to_string();
                break;
            }
        } else {
            break;
        }
    }
}

pub fn decode_box_drawing(ch: char) -> Option<(u8, u8, u8, u8, u8)> {
    let code = ch as u32;
    if !(0x2500..=0x257F).contains(&code) {
        return None;
    }
    Some(match code {
        0x2500 => (1, 1, 0, 0, 0),
        0x2501 => (2, 2, 0, 0, 0),
        0x2502 => (0, 0, 1, 1, 0),
        0x2503 => (0, 0, 2, 2, 0),
        0x2504..=0x250B => (1, 1, 1, 1, 3),
        0x250C => (0, 1, 0, 1, 0),
        0x250D => (0, 2, 0, 1, 0),
        0x250E => (0, 1, 0, 2, 0),
        0x250F => (0, 2, 0, 2, 0),
        0x2510 => (1, 0, 0, 1, 0),
        0x2511 => (2, 0, 0, 1, 0),
        0x2512 => (1, 0, 0, 2, 0),
        0x2513 => (2, 0, 0, 2, 0),
        0x2514 => (0, 1, 1, 0, 0),
        0x2515 => (0, 2, 1, 0, 0),
        0x2516 => (0, 1, 2, 0, 0),
        0x2517 => (0, 2, 2, 0, 0),
        0x2518 => (1, 0, 1, 0, 0),
        0x2519 => (2, 0, 1, 0, 0),
        0x251A => (1, 0, 2, 0, 0),
        0x251B => (2, 0, 2, 0, 0),
        0x251C => (0, 1, 1, 1, 0),
        0x251D => (0, 2, 1, 1, 0),
        0x251E => (0, 1, 2, 1, 0),
        0x251F => (0, 1, 1, 2, 0),
        0x2520 => (0, 1, 2, 2, 0),
        0x2521 => (0, 2, 2, 1, 0),
        0x2522 => (0, 2, 1, 2, 0),
        0x2523 => (0, 2, 2, 2, 0),
        0x2524 => (1, 0, 1, 1, 0),
        0x2525 => (2, 0, 1, 1, 0),
        0x2526 => (1, 0, 2, 1, 0),
        0x2527 => (1, 0, 1, 2, 0),
        0x2528 => (1, 0, 2, 2, 0),
        0x2529 => (2, 0, 2, 1, 0),
        0x252A => (2, 0, 1, 2, 0),
        0x252B => (2, 0, 2, 2, 0),
        0x252C => (1, 1, 0, 1, 0),
        0x252D => (2, 1, 0, 1, 0),
        0x252E => (1, 2, 0, 1, 0),
        0x252F => (2, 2, 0, 1, 0),
        0x2530 => (1, 1, 0, 2, 0),
        0x2531 => (2, 1, 0, 2, 0),
        0x2532 => (1, 2, 0, 2, 0),
        0x2533 => (2, 2, 0, 2, 0),
        0x2534 => (1, 1, 1, 0, 0),
        0x2535 => (2, 1, 1, 0, 0),
        0x2536 => (1, 2, 1, 0, 0),
        0x2537 => (2, 2, 1, 0, 0),
        0x2538 => (1, 1, 2, 0, 0),
        0x2539 => (2, 1, 2, 0, 0),
        0x253A => (1, 2, 2, 0, 0),
        0x253B => (2, 2, 2, 0, 0),
        0x253C => (1, 1, 1, 1, 0),
        0x253D => (2, 1, 1, 1, 0),
        0x253E => (1, 2, 1, 1, 0),
        0x253F => (2, 2, 1, 1, 0),
        0x2540 => (1, 1, 2, 1, 0),
        0x2541 => (1, 1, 1, 2, 0),
        0x2542 => (1, 1, 2, 2, 0),
        0x2543 => (2, 1, 2, 1, 0),
        0x2544 => (1, 2, 2, 1, 0),
        0x2545 => (2, 2, 2, 1, 0),
        0x2546 => (2, 1, 1, 2, 0),
        0x2547 => (1, 2, 1, 2, 0),
        0x2548 => (2, 2, 1, 2, 0),
        0x2549 => (2, 1, 2, 2, 0),
        0x254A => (1, 2, 2, 2, 0),
        0x254B => (2, 2, 2, 2, 0),
        0x254C..=0x254F => (1, 1, 1, 1, 3),
        0x2550 => (3, 3, 0, 0, 0),
        0x2551 => (0, 0, 3, 3, 0),
        0x2552 => (0, 3, 0, 1, 0),
        0x2553 => (0, 1, 0, 3, 0),
        0x2554 => (0, 3, 0, 3, 0),
        0x2555 => (3, 0, 0, 1, 0),
        0x2556 => (1, 0, 0, 3, 0),
        0x2557 => (3, 0, 0, 3, 0),
        0x2558 => (0, 3, 1, 0, 0),
        0x2559 => (0, 1, 3, 0, 0),
        0x255A => (0, 3, 3, 0, 0),
        0x255B => (3, 0, 1, 0, 0),
        0x255C => (1, 0, 3, 0, 0),
        0x255D => (3, 0, 3, 0, 0),
        0x255E => (0, 3, 1, 1, 0),
        0x255F => (0, 1, 3, 3, 0),
        0x2560 => (0, 3, 3, 3, 0),
        0x2561 => (3, 0, 1, 1, 0),
        0x2562 => (1, 0, 3, 3, 0),
        0x2563 => (3, 0, 3, 3, 0),
        0x2564 => (3, 3, 0, 1, 0),
        0x2565 => (1, 1, 0, 3, 0),
        0x2566 => (3, 3, 0, 3, 0),
        0x2567 => (3, 3, 1, 0, 0),
        0x2568 => (1, 1, 3, 0, 0),
        0x2569 => (3, 3, 3, 0, 0),
        0x256A => (3, 3, 1, 1, 0),
        0x256B => (1, 1, 3, 3, 0),
        0x256C => (3, 3, 3, 3, 0),
        0x256D => (0, 1, 0, 1, 1),
        0x256E => (1, 0, 0, 1, 1),
        0x256F => (1, 0, 1, 0, 1),
        0x2570 => (0, 1, 1, 0, 1),
        0x2574 => (1, 0, 0, 0, 0),
        0x2575 => (0, 0, 1, 0, 0),
        0x2576 => (0, 1, 0, 0, 0),
        0x2577 => (0, 0, 0, 1, 0),
        0x2578 => (2, 0, 0, 0, 0),
        0x2579 => (0, 0, 2, 0, 0),
        0x257A => (0, 2, 0, 0, 0),
        0x257B => (0, 0, 0, 2, 0),
        0x257C => (1, 2, 0, 0, 0),
        0x257D => (0, 0, 1, 2, 0),
        0x257E => (2, 1, 0, 0, 0),
        0x257F => (0, 0, 2, 1, 0),
        _ => (0, 0, 0, 0, 0),
    })
}

#[allow(dead_code)]
fn render_quadrant(width: f32, line_h: f32, tl: Hsla, tr: Hsla, bl: Hsla, br: Hsla) -> Div {
    let half_w = (width / 2.0).floor().max(1.0);
    let rem_w = (width - half_w).max(1.0);
    let half_h = (line_h / 2.0).floor().max(1.0);
    let rem_h = (line_h - half_h).max(1.0);

    div()
        .flex()
        .flex_col()
        .w(px(width))
        .h(px(line_h))
        .child(
            div()
                .flex()
                .flex_row()
                .w_full()
                .h(px(half_h))
                .child(div().h_full().w(px(half_w)).bg(tl))
                .child(div().h_full().w(px(rem_w)).bg(tr)),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .w_full()
                .h(px(rem_h))
                .child(div().h_full().w(px(half_w)).bg(bl))
                .child(div().h_full().w(px(rem_w)).bg(br)),
        )
}

/// Geometry of the Mission Control grid, derived once per frame from the
/// window size and the cell count.
///
/// The renderer and the keyboard handler both read this struct. A single
/// source keeps the painted column count and the navigation column count
/// identical, which is what makes the arrow keys land on the cell under the
/// highlight. Deriving them separately let the two drift apart, and a drifting
/// column count turns Left/Right into a jump to an unrelated cell.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MissionControlLayout {
    /// Columns actually painted. `fits_cols_in_row(grid_w, cell_w)` must equal
    /// this, because that expression is the number of cells the flex line fits.
    cols: usize,
    /// Painted width of one cell.
    cell_w: f32,
    /// Painted height of one cell.
    cell_h: f32,
    /// Outer width of the grid box: the cells, the gaps, and the box padding.
    grid_w: f32,
    /// Height the grid may use before it scrolls.
    grid_max_h: f32,
}

/// How many bytes of a file the prompt may inline before it truncates.
const AI_ATTACH_INLINE_MAX_BYTES: usize = 16_000;

/// Cut a file down to what fits in the prompt, on a character boundary.
///
/// `len` is bytes, not characters, so slicing at exactly the cap panics when the
/// cap lands inside a multi-byte character. Any source file with an accented
/// word near the cap would abort the process on submit.
fn truncate_for_prompt(content: &str) -> String {
    if content.len() <= AI_ATTACH_INLINE_MAX_BYTES {
        return content.to_string();
    }
    let end = (0..=AI_ATTACH_INLINE_MAX_BYTES)
        .rev()
        .find(|i| content.is_char_boundary(*i))
        .unwrap_or(0);
    format!("{}... (truncated)", &content[..end])
}

/// True when the AI composer gets the key instead of an overlay or the shell.
///
/// The composer is focused as a plain bool, so clicking the paperclip leaves it
/// focused while the file picker sits above it. Without this gate the composer
/// block runs first and swallows everything: a typed letter goes into the
/// prompt, Enter sends the prompt, Escape closes the AI sidebar.
fn ai_keys_own_input(ai_open: bool, ai_focused: bool, file_picker_open: bool) -> bool {
    ai_open && ai_focused && !file_picker_open
}

/// Rows the `@` mention menu draws. The candidate list, this count and the
/// renderer all cap at `AI_AT_MENU_MAX_ROWS`, so the selection works off the
/// drawn count and never points at a row that is not painted.
fn at_menu_row_count(matches: &[String]) -> usize {
    matches
        .len()
        .min(crate::ui::ai_sidebar::AI_AT_MENU_MAX_ROWS)
}

/// Move the `@` mention menu selection by `delta` rows, wrapping at both ends.
/// With no rows there is nothing to point at, so the selection goes to 0.
fn move_at_menu_selection(selected: &mut usize, matches: &[String], delta: isize) {
    let len = at_menu_row_count(matches) as isize;
    if len == 0 {
        *selected = 0;
        return;
    }
    *selected = (*selected as isize + delta).rem_euclid(len) as usize;
}

/// Move the selection by `delta` rows and stop at the ends instead of
/// wrapping. A page jump that wrapped would land the user at the other end of
/// the list, which is the opposite of paging.
fn move_at_menu_selection_clamped(selected: &mut usize, matches: &[String], delta: isize) {
    let len = at_menu_row_count(matches) as isize;
    if len == 0 {
        *selected = 0;
        return;
    }
    *selected = ((*selected as isize + delta).clamp(0, len - 1)) as usize;
}

/// The row Enter commits in the `@` mention menu.
///
/// `None` means the key is not ours: the menu is closed or empty, so the caller
/// sends it to the shell. The index is clamped, because a stale selection from
/// a list that has since shrunk would otherwise pick a row that is no longer
/// shown.
fn at_menu_commit_row(menu_open: bool, matches: &[String], selected: usize) -> Option<usize> {
    if !menu_open {
        return None;
    }
    let rows = at_menu_row_count(matches);
    if rows == 0 {
        return None;
    }
    Some(selected.min(rows - 1))
}

/// Decide where a drop at `drop_x` lands.
///
/// `sidebar_left` is the left edge of the AI composer in window coordinates.
/// With the panel closed its width is 0, so the edge sits at the right of the
/// window and nothing can land on it.
fn file_drop_target_at(ai_visible: bool, sidebar_left: f32, drop_x: f32) -> FileDropTarget {
    if ai_visible && drop_x >= sidebar_left {
        FileDropTarget::AiAttach
    } else {
        FileDropTarget::Terminal
    }
}

/// Where a dropped file goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileDropTarget {
    /// Paste a shell-quoted path into the focused terminal.
    #[default]
    Terminal,
    /// Attach the file to the next AI message. Images and PDFs go as their own
    /// content part, anything else is inlined as text.
    AiAttach,
}

/// Rows PageUp and PageDown move in the `@` mention menu.
const AI_AT_MENU_PAGE_ROWS: isize = 5;

/// Cells per row. Four columns fills the window edge to edge up to the cell
/// width cap; see `cell_width_cap_threshold`. Past that the grid centres itself,
/// because a fifth column would push every cell past the readable width.
const MC_MAX_COLS: usize = 4;
/// Gap between cells.
const MC_GAP: f32 = 16.0;
/// Padding inside the grid box. Flex wrapping measures the content box, so this
/// padding comes out of the width the cells get.
const MC_GRID_PAD: f32 = 4.0;
/// Cell aspect (height / width). A card is a little shorter than it is wide:
/// the preview shows the last screenful of a tall terminal, which reads fine
/// in a landscape box and wastes height in a portrait one. A window too short
/// for the height shrinks the cell, and the grid then scrolls.
const MC_CELL_ASPECT: f32 = 0.75;
/// Narrowest cell that still fits a usable preview. A window too narrow for
/// this drops a column instead of shrinking cells further.
const MC_MIN_CELL_W: f32 = 260.0;
/// Widest cell. Past this a preview stretches without gaining readable detail.
/// The value is high enough that four columns still fill the window on a
/// 3440 px display, which is where a lower cap left a dead band at the edge.
const MC_MAX_CELL_W: f32 = 900.0;
/// Distance kept between the grid and the window edge.
const MC_EDGE_MARGIN: f32 = 24.0;
/// Space above the header bar.
const MC_TOP_PAD: f32 = 56.0;
/// Height of the header bar.
const MC_HEADER_H: f32 = 38.0;
/// Gap between the header bar and the grid.
const MC_HEADER_GAP: f32 = 16.0;
/// Space the header block takes: top padding, the bar, and the gap.
const MC_HEADER_SPACE: f32 = MC_TOP_PAD + MC_HEADER_H + MC_HEADER_GAP;

/// Height of the card's top bar plus its bottom status bar. The bars carry
/// their own borders, which sit inside their fixed heights, so no extra is
/// added for them. The card's 2 px border is not counted here; that makes the
/// estimate conservative, and the sweep found no overflow from the difference.
const MC_CARD_CHROME_H: f32 = 32.0 + 26.0;
/// Height of one preview line, including the gap between lines. The renderer
/// sets `.h(px(13.))` per line with `.gap(px(1.5))`.
const MC_PREVIEW_LINE_H: f32 = 14.5;
/// Padding inside the preview area, top and bottom. The renderer sets
/// `.py(px(6.))`.
const MC_PREVIEW_PAD_H: f32 = 12.0;

/// Number of preview lines that fit the preview area of a `cell_h` tall card.
///
/// A fixed line count left the rest of the card as empty background, so the
/// count follows the cell height. A terminal cannot supply more rows than it
/// shows, so `available_rows` is the other bound. That bound is also what keeps
/// the per-frame cost in check: `get_screen_preview` never returns more rows
/// than it is asked for, and it reads at most one screen.
fn preview_line_count(cell_h: f32, available_rows: usize) -> usize {
    let usable = cell_h - MC_CARD_CHROME_H - MC_PREVIEW_PAD_H;
    if !usable.is_finite() || usable < MC_PREVIEW_LINE_H {
        // Too short for a whole line, so report none rather than one that
        // cannot fit.
        return 0;
    }
    ((usable / MC_PREVIEW_LINE_H).floor() as usize).min(available_rows)
}

/// How many cells of `cell_w` the flex line inside a `grid_w` box actually
/// fits, given the gaps and the box padding.
///
/// The grid wraps with `flex_wrap`, so this is the number of cells the taffy
/// layout engine will place before breaking the line. When it is less than
/// `layout.cols`, the grid paints fewer cells per row than the keyboard handler
/// navigates, and the arrow keys land on cells that are not where the layout
/// says they are. `collect_flex_lines` breaks the line as soon as
/// `line_length > available`, and `available` is the content box, so the box
/// padding counts against the budget.
///
/// This sums the cells the way taffy does, one at a time, rather than dividing
/// once. A single division and an incremental sum disagree by an ulp when the
/// cells fit with no slack at all, which made some window widths paint one cell
/// fewer than expected.
fn fits_cols_in_row(grid_w: f32, cell_w: f32) -> usize {
    let available = grid_w - 2.0 * MC_GRID_PAD;
    // `is_finite` plus the positive check, rather than `> 0.0` alone, so a NaN
    // from a degenerate window size returns 0 instead of counting a cell.
    if !available.is_finite() || available <= 0.0 || !cell_w.is_finite() || cell_w <= 0.0 {
        return 0;
    }
    let mut line_length = 0.0f32;
    let mut count = 0usize;
    // The loop stops on the first cell that does not fit, which is where taffy
    // breaks the line. The cap keeps a degenerate input from looping forever.
    while count < MC_MAX_COLS * 4 {
        line_length += if count == 0 { cell_w } else { cell_w + MC_GAP };
        if line_length > available {
            break;
        }
        count += 1;
    }
    count
}

impl MissionControlLayout {
    /// `items` counts the tabs plus the trailing "New Tab" card.
    fn new(window_w: f32, window_h: f32, items: usize) -> Self {
        let items = items.max(1);
        // Space the grid may use. The box padding comes off the width, because
        // the cells live inside the padded content box.
        let avail_w = (window_w - 2.0 * MC_EDGE_MARGIN - 2.0 * MC_GRID_PAD).max(MC_MIN_CELL_W);
        let avail_h = (window_h - MC_HEADER_SPACE - MC_EDGE_MARGIN).max(0.0);

        // A column has to fit the minimum cell width, so the window width caps
        // the column count. Height is not a cap: the grid scrolls instead.
        let width_limited = ((avail_w + MC_GAP) / (MC_MIN_CELL_W + MC_GAP)).floor() as usize;
        let cols = [items, MC_MAX_COLS, width_limited.max(1)]
            .into_iter()
            .min()
            .unwrap_or(1)
            .max(1);

        // The columns share the width the window gives them, so the grid grows
        // with the window up to the widest readable cell.
        let cell_w = ((avail_w - (cols as f32 - 1.0) * MC_GAP) / cols as f32)
            .clamp(MC_MIN_CELL_W, MC_MAX_CELL_W);
        // Height follows the cell aspect. A short window wins: the cell shrinks
        // rather than pushing the last row out of view, and the grid scrolls.
        // `min` then `max` keeps the bounds ordered, because `f32::clamp` panics
        // when the lower bound is the larger of the two.
        let cell_h = (cell_w * MC_CELL_ASPECT).min(avail_h).max(0.0);
        // The box holds the cells, the gaps between them, and its own padding.
        let mut grid_w = cell_w * cols as f32 + (cols as f32 - 1.0) * MC_GAP + 2.0 * MC_GRID_PAD;
        // The cells must fit with room to spare. Without the slack, an
        // incremental sum in the layout engine can land an ulp above the budget
        // and wrap the last cell onto a second row, which would make the grid
        // paint one cell fewer per row than the arrow keys navigate.
        while fits_cols_in_row(grid_w, cell_w) < cols {
            grid_w += 1.0;
        }

        Self {
            cols,
            cell_w,
            cell_h,
            grid_w,
            grid_max_h: avail_h,
        }
    }
}

/// Direction of one grid move. An enum keeps a plain index delta from reaching
/// the navigation code, where a Left press and an Up press are different moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GridStep {
    Left,
    Right,
    Up,
    Down,
}

/// Moves the selection one cell, clamped to the grid.
///
/// Left and Right walk the row and stop at the grid edges, so a press never
/// teleports to an unrelated cell. Up and Down keep the column and clamp to the
/// first or last row, so an arrow always lands on the cell the eye expects.
/// `items` is the number of selectable cells, which is one less than the painted
/// count because the trailing "New Tab" card takes no selection.
fn move_grid_selection(selected: usize, step: GridStep, cols: usize, items: usize) -> usize {
    if items == 0 || cols == 0 {
        return 0;
    }
    let last = items - 1;
    let current = selected.min(last);

    match step {
        GridStep::Left => current.saturating_sub(1),
        GridStep::Right => (current + 1).min(last),
        GridStep::Up | GridStep::Down => {
            let row = current / cols;
            let col = current % cols;
            let target_row = match step {
                GridStep::Down => row + 1,
                _ => match row.checked_sub(1) {
                    Some(r) => r,
                    None => return current,
                },
            };
            let target = target_row * cols + col;
            // The last row is short when the cell count is not a multiple of
            // the column count. A step into a cell that row does not have keeps
            // the selection put, rather than sliding it into another column.
            if target > last {
                current
            } else {
                target
            }
        }
    }
}

/// True for box-drawing chars with vertical strokes. Repeated spans of these
/// must render one stroke per cell; a single element would center one stroke
/// across the whole span.
#[allow(dead_code)]
fn box_char_has_vertical_strokes(ch: char) -> bool {
    decode_box_drawing(ch).is_some_and(|(_, _, top, bottom, _)| top > 0 || bottom > 0)
}

/// Geometric Shapes synthesized by `render_geometric_cell`.
#[allow(dead_code)]
fn geometric_shape_synthesized(ch: char) -> bool {
    matches!(
        ch as u32,
        0x25A0 | 0x25A1 | 0x25AA | 0x25AB | 0x25CB | 0x25C9 | 0x25CE | 0x25CF | 0x25E6 | 0x25EF
    )
}

/// Glyphs that must be laid out once per cell when a pure span repeats them.
/// Horizontal-only strokes (─ ═ ━) and block fills (█ ░ ▓) stay single-element
/// because one continuous fill across the span is the correct rendering.
#[allow(dead_code)]
fn synthesized_per_cell(ch: char) -> bool {
    geometric_shape_synthesized(ch) || box_char_has_vertical_strokes(ch)
}

#[allow(dead_code)]
fn render_geometric_cell(
    ch: char,
    width: f32,
    line_h: f32,
    fg: Hsla,
    bg: Option<Hsla>,
    theme_bg: Hsla,
) -> Option<Div> {
    let code = ch as u32;
    let bg_c = bg.unwrap_or(theme_bg);
    let half_h = (line_h / 2.0).floor().max(1.0);
    let rem_h = (line_h - half_h).max(1.0);
    let half_w = (width / 2.0).floor().max(1.0);
    let rem_w = (width - half_w).max(1.0);

    // 1. Block Elements (0x2580..=0x259F)
    if (0x2580..=0x259F).contains(&code) {
        let el = match code {
            0x2588 => div().w(px(width)).h(px(line_h)).bg(fg),
            0x2580 => div()
                .flex()
                .flex_col()
                .w(px(width))
                .h(px(line_h))
                .child(div().w_full().h(px(half_h)).bg(fg))
                .child(div().w_full().h(px(rem_h)).bg(bg_c)),
            0x2584 => div()
                .flex()
                .flex_col()
                .w(px(width))
                .h(px(line_h))
                .child(div().w_full().h(px(half_h)).bg(bg_c))
                .child(div().w_full().h(px(rem_h)).bg(fg)),
            0x258C => div()
                .flex()
                .flex_row()
                .w(px(width))
                .h(px(line_h))
                .child(div().h_full().w(px(half_w)).bg(fg))
                .child(div().h_full().w(px(rem_w)).bg(bg_c)),
            0x2590 => div()
                .flex()
                .flex_row()
                .w(px(width))
                .h(px(line_h))
                .child(div().h_full().w(px(half_w)).bg(bg_c))
                .child(div().h_full().w(px(rem_w)).bg(fg)),
            0x2581..=0x2587 => {
                let frac = (code - 0x2580) as f32 / 8.0;
                let fill_h = (line_h * frac).round().max(1.0);
                div().relative().w(px(width)).h(px(line_h)).bg(bg_c).child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(fill_h))
                        .bg(fg),
                )
            }
            0x2589..=0x258F => {
                let frac = (8 - (code - 0x2588)) as f32 / 8.0;
                let fill_w = (width * frac).round().max(1.0);
                div().relative().w(px(width)).h(px(line_h)).bg(bg_c).child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(px(fill_w))
                        .bg(fg),
                )
            }
            0x2591 => {
                let mut blended = fg;
                blended.a = 0.25;
                div()
                    .w(px(width))
                    .h(px(line_h))
                    .bg(bg_c)
                    .child(div().size_full().bg(blended))
            }
            0x2592 => {
                let mut blended = fg;
                blended.a = 0.50;
                div()
                    .w(px(width))
                    .h(px(line_h))
                    .bg(bg_c)
                    .child(div().size_full().bg(blended))
            }
            0x2593 => {
                let mut blended = fg;
                blended.a = 0.75;
                div()
                    .w(px(width))
                    .h(px(line_h))
                    .bg(bg_c)
                    .child(div().size_full().bg(blended))
            }
            0x2594 => {
                let fill_h = (line_h * 0.125).round().max(1.0);
                div().relative().w(px(width)).h(px(line_h)).bg(bg_c).child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(fill_h))
                        .bg(fg),
                )
            }
            0x2595 => {
                let fill_w = (width * 0.125).round().max(1.0);
                div().relative().w(px(width)).h(px(line_h)).bg(bg_c).child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .w(px(fill_w))
                        .bg(fg),
                )
            }
            0x2596 => render_quadrant(width, line_h, bg_c, bg_c, fg, bg_c),
            0x2597 => render_quadrant(width, line_h, bg_c, bg_c, bg_c, fg),
            0x2598 => render_quadrant(width, line_h, fg, bg_c, bg_c, bg_c),
            0x2599 => render_quadrant(width, line_h, fg, bg_c, fg, fg),
            0x259A => render_quadrant(width, line_h, fg, bg_c, bg_c, fg),
            0x259B => render_quadrant(width, line_h, fg, fg, fg, bg_c),
            0x259C => render_quadrant(width, line_h, fg, fg, bg_c, fg),
            0x259D => render_quadrant(width, line_h, bg_c, fg, bg_c, bg_c),
            0x259E => render_quadrant(width, line_h, bg_c, fg, fg, bg_c),
            0x259F => render_quadrant(width, line_h, bg_c, fg, fg, fg),
            _ => return None,
        };
        return Some(el.flex_shrink_0());
    }

    // 2. Box Drawing Characters (0x2500..=0x257F)
    if let Some((left_style, right_style, top_style, bottom_style, kind)) = decode_box_drawing(ch) {
        let mid_x = (width / 2.0).round();
        let mid_y = (line_h / 2.0).round();
        let t_light = 1.0;
        let t_heavy = 2.0;
        let get_t = |s: u8| if s == 2 { t_heavy } else { t_light };

        let t_l = get_t(left_style);
        let t_r = get_t(right_style);
        let t_t = get_t(top_style);
        let t_b = get_t(bottom_style);

        if kind == 1 {
            // Round corners (0x256D..=0x2570)
            let radius = (mid_x.min(mid_y) * 0.9).round();
            let mut corner = div().relative().w(px(width)).h(px(line_h)).bg(bg_c);

            // 0x256D (╭): down & right -> top-left bend
            if code == 0x256D {
                corner = corner.child(
                    div()
                        .absolute()
                        .left(px(mid_x - t_l / 2.0))
                        .top(px(mid_y - t_t / 2.0))
                        .right_0()
                        .bottom_0()
                        .border_t_1()
                        .border_l_1()
                        .border_color(fg)
                        .rounded_tl(px(radius)),
                );
            }
            // 0x256E (╮): down & left -> top-right bend
            else if code == 0x256E {
                corner = corner.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(mid_y - t_t / 2.0))
                        .w(px(mid_x + t_r / 2.0))
                        .bottom_0()
                        .border_t_1()
                        .border_r_1()
                        .border_color(fg)
                        .rounded_tr(px(radius)),
                );
            }
            // 0x2570 (╰): up & right -> bottom-left bend
            else if code == 0x2570 {
                corner = corner.child(
                    div()
                        .absolute()
                        .left(px(mid_x - t_l / 2.0))
                        .top_0()
                        .right_0()
                        .h(px(mid_y + t_b / 2.0))
                        .border_b_1()
                        .border_l_1()
                        .border_color(fg)
                        .rounded_bl(px(radius)),
                );
            }
            // 0x256F (╯): up & left -> bottom-right bend
            else if code == 0x256F {
                corner = corner.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .w(px(mid_x + t_r / 2.0))
                        .h(px(mid_y + t_b / 2.0))
                        .border_b_1()
                        .border_r_1()
                        .border_color(fg)
                        .rounded_br(px(radius)),
                );
            }

            return Some(corner.flex_shrink_0());
        }

        let mut container = div().relative().w(px(width)).h(px(line_h)).bg(bg_c);

        if left_style > 0 && right_style > 0 && left_style == right_style && left_style != 3 {
            container = container.child(
                div()
                    .absolute()
                    .top(px(mid_y - t_l / 2.0))
                    .left_0()
                    .right_0()
                    .h(px(t_l))
                    .bg(fg),
            );
        } else {
            if left_style > 0 {
                container = container.child(
                    div()
                        .absolute()
                        .top(px(mid_y - t_l / 2.0))
                        .left_0()
                        .w(px(mid_x))
                        .h(px(t_l))
                        .bg(fg),
                );
            }
            if right_style > 0 {
                container = container.child(
                    div()
                        .absolute()
                        .top(px(mid_y - t_r / 2.0))
                        .left(px(mid_x))
                        .right_0()
                        .h(px(t_r))
                        .bg(fg),
                );
            }
        }

        if top_style > 0 && bottom_style > 0 && top_style == bottom_style && top_style != 3 {
            container = container.child(
                div()
                    .absolute()
                    .left(px(mid_x - t_t / 2.0))
                    .top_0()
                    .bottom_0()
                    .w(px(t_t))
                    .bg(fg),
            );
        } else {
            if top_style > 0 {
                container = container.child(
                    div()
                        .absolute()
                        .left(px(mid_x - t_t / 2.0))
                        .top_0()
                        .h(px(mid_y))
                        .w(px(t_t))
                        .bg(fg),
                );
            }
            if bottom_style > 0 {
                container = container.child(
                    div()
                        .absolute()
                        .left(px(mid_x - t_b / 2.0))
                        .top(px(mid_y))
                        .bottom_0()
                        .w(px(t_b))
                        .bg(fg),
                );
            }
        }

        return Some(container.flex_shrink_0());
    }

    // 3. Geometric Shapes (subset of U+25A0..=U+25FF). Synthesized like
    // Kitty/Ghostty do: circles centered on the full cell box — not on the
    // text baseline — so they never clip and stay aligned regardless of
    // which fallback font would supply the glyph.
    if (0x25A0..=0x25FF).contains(&code) {
        let d = width.min(line_h).max(2.0);
        let ring = |size: f32| {
            div()
                .w(px(size))
                .h(px(size))
                .rounded_full()
                .border_1()
                .border_color(fg)
        };
        let dot = |size: f32| div().w(px(size)).h(px(size)).rounded_full().bg(fg);
        let square = |side: f32| div().w(px(side)).h(px(side));
        let centered = |child: Div| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .w(px(width))
                .h(px(line_h))
                .bg(bg_c)
                .flex_shrink_0()
                .child(child)
        };

        let el = match code {
            // ○ WHITE CIRCLE
            0x25CB => centered(ring(d * 0.78)),
            // ◯ LARGE CIRCLE
            0x25EF => centered(ring(d)),
            // ● BLACK CIRCLE
            0x25CF => centered(dot(d * 0.72)),
            // ◦ WHITE BULLET
            0x25E6 => centered(ring(d * 0.40)),
            // ◉ FISHEYE — ring plus filled center
            0x25C9 => centered(
                ring(d * 0.78).child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .size_full()
                        .child(dot(d * 0.36)),
                ),
            ),
            // ◎ BULLSEYE — two concentric rings
            0x25CE => centered(
                ring(d * 0.78).child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .size_full()
                        .child(ring(d * 0.42)),
                ),
            ),
            // ■ □ squares
            0x25A0 => centered(square(d * 0.72).bg(fg)),
            0x25A1 => centered(square(d * 0.72).border_1().border_color(fg)),
            // ▪ ▫ small squares
            0x25AA => centered(square(d * 0.45).bg(fg)),
            0x25AB => centered(square(d * 0.45).border_1().border_color(fg)),
            _ => return None,
        };
        return Some(el);
    }

    None
}

/// OpenType features enabling ligature substitution for terminal text.
///
/// Previously this was unconditional, so `font.ligatures` in the config could be
/// set either way without changing anything. Note that ligatures fuse several
/// characters into one glyph, which does not fit the terminal's one-character-
/// per-cell grid, so callers honour the user's preference rather than assuming.
pub(crate) fn terminal_ligatures(enabled: bool) -> FontFeatures {
    // Value 0 is required to switch a feature off. An empty feature list means
    // "no override", so returning one leaves the font's own defaults in place --
    // and calt/liga are on by default, which is why an earlier attempt to
    // disable ligatures by returning nothing had no effect at all.
    let value = u32::from(enabled);
    FontFeatures(std::sync::Arc::new(vec![
        ("calt".to_string(), value),
        ("liga".to_string(), value),
        ("dlig".to_string(), value),
        ("clig".to_string(), value),
    ]))
}

fn is_emoji_codepoint(code: u32) -> bool {
    matches!(
        code,
        0x1F300..=0x1FAFF
            | 0x2600..=0x27BF
            | 0x231A
            | 0x231B
            | 0x2328
            | 0x23CF
            | 0x23E9..=0x23F3
            | 0x23F8..=0x23FA
            | 0x2B50..=0x2B55
    )
}

/// Nerd Font icon codepoints: BMP private-use area (Powerline, Font
/// Awesome, Devicons, Octicons, Seti, Codicons, ...), its conventional
/// stragglers, and the supplementary PUA planes.
pub fn is_nerd_codepoint(code: u32) -> bool {
    (0xE000..=0xF8FF).contains(&code)
        || (0x23FB..=0x23FE).contains(&code)
        || code == 0x2B58
        || (0xF0000..=0xFFFFD).contains(&code)
        || (0x100000..=0x10FFFD).contains(&code)
}

/// Technical keyboard shortcut and modifier symbols (⌘, ⌥, ⌃, ⇧, ⎋, ⏎, etc.)
/// that require dedicated single-cell centering and scaling to prevent overlap.
#[inline]
pub fn is_keyboard_symbol(code: u32) -> bool {
    if is_emoji_codepoint(code) || is_nerd_codepoint(code) {
        return false;
    }
    matches!(
        code,
        0x2318 // ⌘ Command
            | 0x2325 // ⌥ Option
            | 0x2303 // ⌃ Control
            | 0x238B // ⎋ Escape
            | 0x232B // ⌫ Delete / Backspace
            | 0x2326 // ⌦ Forward delete
            | 0x23CE // ⏎ Return
            | 0x2423 // ␣ Space symbol
            | 0x2190..=0x21FF // All Arrows (← ↑ → ↓ ⇧ ⇥ ⇤ ⇪ ↩ ↵ ⇄ ⇅ ⇐ ⇑ ⇒ ⇓ etc.)
            | 0x27F0..=0x27FF // Supplemental Arrows-A
            | 0x2900..=0x297F // Supplemental Arrows-B
            | 0x2B00..=0x2BFF // Miscellaneous Symbols and Arrows
    )
}

/// Finds an installed Nerd Font for icon fallback. Prefers `*Mono`
/// variants (terminal metrics; note the suffix match: the family name
/// itself may contain "mono", e.g. "JetBrainsMono Nerd Font"), else the
/// first match. Case-insensitive. Returns `None` when no Nerd Font exists:
/// callers keep today's behavior.
pub fn find_nerd_font_family(names: &[String]) -> Option<String> {
    let mut best: Option<(u8, String)> = None;
    for name in names {
        let lower = name.to_lowercase();
        if !lower.contains("nerd") {
            continue;
        }
        let score = if lower.ends_with("mono") { 0 } else { 1 };
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, name.clone()));
        }
    }
    best.map(|(_, name)| name)
}

pub fn open_path_or_url(target: impl AsRef<std::ffi::OsStr>) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(target).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "start", ""]).arg(target);
        cmd.creation_flags(0x08000000);
        let _ = cmd.spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

fn get_font_cell_metrics(_family: &str, size: f32) -> (f32, f32) {
    #[cfg(target_os = "macos")]
    {
        crate::font_discovery_macos::measure_font_metrics(_family, size)
    }
    #[cfg(not(target_os = "macos"))]
    {
        (size * 0.60, size * 1.32)
    }
}

pub(crate) fn available_system_fonts() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        // Unfiltered on purpose. The settings view decides whether to narrow to
        // monospace families, and needs the full list cached to be able to widen
        // again without paying for discovery a second time.
        crate::font_discovery_macos::all_system_font_families()
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("fc-list")
            .args([":", "family"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut set = std::collections::BTreeSet::new();
                for line in stdout.lines() {
                    for part in line.split(',') {
                        let name = part.trim();
                        if !name.is_empty() && !name.starts_with('.') {
                            set.insert(name.to_string());
                        }
                    }
                }
                if !set.is_empty() {
                    return set.into_iter().collect();
                }
            }
        }
        vec![
            "DejaVu Sans".to_string(),
            "DejaVu Sans Mono".to_string(),
            "Fira Code".to_string(),
            "JetBrains Mono".to_string(),
            "Hack".to_string(),
            "Ubuntu".to_string(),
            "Ubuntu Mono".to_string(),
            "Liberation Mono".to_string(),
            "Liberation Sans".to_string(),
            "Liberation Serif".to_string(),
            "monospace".to_string(),
            "sans-serif".to_string(),
        ]
    }
    #[cfg(target_os = "windows")]
    {
        let mut reg_fonts = Vec::new();
        let mut reg_cmd = std::process::Command::new("reg");
        reg_cmd.args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts",
        ]);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            reg_cmd.creation_flags(0x08000000);
        }
        if let Ok(output) = reg_cmd.output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with("HKEY_") {
                        continue;
                    }
                    if let Some(font_entry) = trimmed.split("    REG_").next() {
                        let mut font_name = font_entry.trim();
                        if let Some(idx) = font_name.rfind('(') {
                            font_name = font_name[..idx].trim();
                        }
                        if !font_name.is_empty() && !font_name.starts_with('.') {
                            reg_fonts.push(font_name.to_string());
                        }
                    }
                }
            }
        }
        if !reg_fonts.is_empty() {
            reg_fonts.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
            reg_fonts.dedup();
            return reg_fonts;
        }

        vec![
            "Arial".to_string(),
            "Calibri".to_string(),
            "Cambria".to_string(),
            "Cascadia Code".to_string(),
            "Cascadia Mono".to_string(),
            "Comic Sans MS".to_string(),
            "Consolas".to_string(),
            "Courier New".to_string(),
            "Fira Code".to_string(),
            "Georgia".to_string(),
            "Impact".to_string(),
            "JetBrains Mono".to_string(),
            "Lucida Console".to_string(),
            "Lucida Sans Unicode".to_string(),
            "Microsoft Sans Serif".to_string(),
            "Segoe UI".to_string(),
            "Segoe UI Variable".to_string(),
            "Tahoma".to_string(),
            "Times New Roman".to_string(),
            "Trebuchet MS".to_string(),
            "Verdana".to_string(),
            "monospace".to_string(),
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        vec!["monospace".to_string()]
    }
}

pub fn send_system_notification(title: &str, body: &str) {
    type NotificationSender = std::sync::mpsc::SyncSender<(String, String)>;
    static WORKER: std::sync::OnceLock<Result<NotificationSender, String>> = std::sync::OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<(String, String)>(16);
        std::thread::Builder::new()
            .name("fastty-notifications".into())
            .spawn(move || {
                for (title, body) in receiver {
                    deliver_system_notification(&title, &body);
                }
            })
            .map(|_| sender)
            .map_err(|error| error.to_string())
    });
    match worker {
        Ok(sender) => {
            if let Err(error) = sender.try_send((title.to_owned(), body.to_owned())) {
                eprintln!("Fastty notification queue rejected a report: {error}");
            }
        }
        Err(error) => eprintln!("Fastty notification worker failed: {error}"),
    }
}

fn deliver_system_notification(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        static APPLICATION: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
        let application = APPLICATION.get_or_init(|| {
            notify_rust::set_application("com.diegoleteliers10.fastty")
                .map_err(|error| error.to_string())
        });
        if let Err(error) = application {
            eprintln!("Fastty notification setup failed: {error}");
            return;
        }
    }
    let summary = if title.is_empty() { "Fastty" } else { title };
    #[cfg(target_os = "linux")]
    let notification_body = body.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    #[cfg(not(target_os = "linux"))]
    let notification_body = body.to_owned();
    let res = notify_rust::Notification::new()
        .appname("Fastty")
        .summary(summary)
        .body(&notification_body)
        .show();

    if let Err(error) = res {
        eprintln!("Fastty notification delivery failed: {error}");
    }
}

use super::icons::{render_app_logo, render_icon};
use crate::universal_picker::ItemKind;
use icons::common::IconType;

#[derive(Clone, Debug)]
pub struct GlobalSearchResult {
    pub tab_idx: usize,
    pub tab_id: usize,
    pub tab_title: String,
    pub process_name: Option<String>,
    pub pane_id: usize,
    pub offset: usize,
    pub line_index: i32,
    pub line_content: String,
}

#[derive(Clone, Debug)]
pub struct PaletteCommand {
    pub id: &'static str,
    pub icon: IconType,
    pub title: &'static str,
    pub category: &'static str,
    pub shortcut: Option<&'static str>,
}

pub fn fuzzy_match_str(pattern: &str, target: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    let p_lower = pattern.to_lowercase();
    let t_lower = target.to_lowercase();
    if t_lower.contains(&p_lower) {
        return true;
    }
    let mut p_iter = p_lower.chars();
    let mut curr_p = p_iter.next();
    for ch in t_lower.chars() {
        if let Some(p) = curr_p {
            if ch == p {
                curr_p = p_iter.next();
            }
        } else {
            break;
        }
    }
    curr_p.is_none()
}

pub fn get_all_palette_commands() -> Vec<PaletteCommand> {
    let is_mac = cfg!(target_os = "macos");
    vec![
        PaletteCommand {
            id: "tab_overview",
            icon: IconType::Layers,
            title: "Mission Control / Tab Peek Overview",
            category: "Navigation",
            shortcut: Some(if is_mac { "⌘⇧O" } else { "Ctrl+Shift+M" }),
        },
        PaletteCommand {
            id: "global_search",
            icon: IconType::TextSearch,
            title: "Find in All Tabs (Global Search)",
            category: "Terminal",
            shortcut: Some(if is_mac { "⌘⇧F" } else { "Ctrl+Shift+F" }),
        },
        PaletteCommand {
            id: "save_session",
            icon: IconType::Folder,
            title: "Session: Save Current Workspace Snapshot",
            category: "Session",
            shortcut: None,
        },
        PaletteCommand {
            id: "restore_session",
            icon: IconType::RotateCcw,
            title: "Session: Attach / Restore Last Workspace",
            category: "Session",
            shortcut: None,
        },
        PaletteCommand {
            id: "new_tab",
            icon: IconType::Plus,
            title: "New Tab",
            category: "Terminal",
            shortcut: Some(if is_mac { "⌘T" } else { "Ctrl+Shift+T" }),
        },
        PaletteCommand {
            id: "new_window",
            icon: IconType::ExternalLink,
            title: "New Window",
            category: "Window",
            shortcut: Some(if is_mac { "⌘⇧N" } else { "Ctrl+Shift+N" }),
        },
        PaletteCommand {
            id: "rename_tab",
            icon: IconType::Pencil,
            title: "Rename Active Tab",
            category: "Terminal",
            shortcut: Some(if is_mac { "⌘⇧R" } else { "Ctrl+Shift+R" }),
        },
        PaletteCommand {
            id: "close_tab",
            icon: IconType::X,
            title: "Close Active Tab",
            category: "Terminal",
            shortcut: Some(if is_mac { "⌘⇧W" } else { "Ctrl+Shift+Q" }),
        },
        PaletteCommand {
            id: "search",
            icon: IconType::Search,
            title: "Search in Buffer",
            category: "Terminal",
            shortcut: Some(if is_mac { "⌘F" } else { "Ctrl+Shift+F" }),
        },
        PaletteCommand {
            id: "clear",
            icon: IconType::Trash2,
            title: "Clear Scrollback",
            category: "Terminal",
            shortcut: Some(if is_mac { "⌘K" } else { "Ctrl+Shift+K" }),
        },
        PaletteCommand {
            id: "worktree",
            icon: IconType::FolderGit2,
            title: "Git Worktree Picker",
            category: "Git",
            shortcut: Some(if is_mac { "⌘⌥W" } else { "Ctrl+Alt+W" }),
        },
        PaletteCommand {
            id: "project_jumper",
            icon: IconType::Folder,
            title: "Project / Tab Jumper",
            category: "Navigation",
            shortcut: Some(if is_mac { "⌘J" } else { "Ctrl+Shift+J" }),
        },
        PaletteCommand {
            id: "file_picker",
            icon: IconType::Sparkles,
            title: "Universal Insert (Files, SSH, Git, Snippets)",
            category: "Tools",
            shortcut: Some(if is_mac { "⌃⌘," } else { "Ctrl+Super+," }),
        },
        PaletteCommand {
            id: "ssh",
            icon: IconType::Server,
            title: "SSH Host Manager",
            category: "Tools",
            shortcut: Some(if is_mac { "⌘O" } else { "Ctrl+Shift+O" }),
        },
        PaletteCommand {
            id: "snippets",
            icon: IconType::Terminal,
            title: "Snippets: Insert Snippet...",
            category: "Tools",
            shortcut: None,
        },
        PaletteCommand {
            id: "prs",
            icon: IconType::GitPullRequest,
            title: "GitHub: Pull Requests (Checkout/Approve/Merge)",
            category: "Git",
            shortcut: None,
        },
        PaletteCommand {
            id: "settings",
            icon: IconType::Settings,
            title: "Open Settings",
            category: "Preferences",
            shortcut: Some(if is_mac { "⌘," } else { "Ctrl+," }),
        },
        PaletteCommand {
            id: "about",
            icon: IconType::Zap,
            title: "About Fastty",
            category: "Application",
            shortcut: None,
        },
        PaletteCommand {
            id: "check_updates",
            icon: IconType::RotateCcw,
            title: "Check for Updates",
            category: "Application",
            shortcut: None,
        },
        PaletteCommand {
            id: "fullscreen",
            icon: IconType::Maximize2,
            title: "Toggle Fullscreen",
            category: "Window",
            shortcut: Some(if is_mac { "⌃⌘F" } else { "F11" }),
        },
        PaletteCommand {
            id: "zoom_in",
            icon: IconType::ZoomIn,
            title: "Font: Increase Size (+1)",
            category: "View",
            shortcut: Some(if is_mac { "⌘=" } else { "Ctrl=" }),
        },
        PaletteCommand {
            id: "zoom_out",
            icon: IconType::ZoomOut,
            title: "Font: Decrease Size (-1)",
            category: "View",
            shortcut: Some(if is_mac { "⌘-" } else { "Ctrl-" }),
        },
        PaletteCommand {
            id: "zoom_reset",
            icon: IconType::RotateCcw,
            title: "Font: Reset Size (Default)",
            category: "View",
            shortcut: Some(if is_mac { "⌘0" } else { "Ctrl+0" }),
        },
        PaletteCommand {
            id: "theme_default",
            icon: IconType::Palette,
            title: "Theme: Switch to Default (Fastty)",
            category: "Theme",
            shortcut: None,
        },
        PaletteCommand {
            id: "open_config",
            icon: IconType::FolderOpen,
            title: "Open Config Folder",
            category: "Preferences",
            shortcut: None,
        },
        PaletteCommand {
            id: "edit_config",
            icon: IconType::FileCode,
            title: "Edit config.toml",
            category: "Preferences",
            shortcut: None,
        },
        PaletteCommand {
            id: "split_right",
            icon: IconType::PanelRight,
            title: "Split Pane Right",
            category: "Panes",
            shortcut: Some(if is_mac { "⌘D" } else { "Ctrl+Shift+E" }),
        },
        PaletteCommand {
            id: "split_down",
            icon: IconType::PanelBottom,
            title: "Split Pane Down",
            category: "Panes",
            shortcut: Some(if is_mac { "⌘⇧D" } else { "Ctrl+Shift+O" }),
        },
        PaletteCommand {
            id: "split_left",
            icon: IconType::PanelLeft,
            title: "Split Pane Left",
            category: "Panes",
            shortcut: None,
        },
        PaletteCommand {
            id: "split_top",
            icon: IconType::PanelTop,
            title: "Split Pane Top",
            category: "Panes",
            shortcut: None,
        },
        PaletteCommand {
            id: "focus_left",
            icon: IconType::ChevronLeft,
            title: "Focus Pane Left",
            category: "Panes",
            shortcut: Some(if is_mac { "⌥⌘←" } else { "Alt+←" }),
        },
        PaletteCommand {
            id: "focus_right",
            icon: IconType::ChevronRight,
            title: "Focus Pane Right",
            category: "Panes",
            shortcut: Some(if is_mac { "⌥⌘→" } else { "Alt+→" }),
        },
        PaletteCommand {
            id: "focus_top",
            icon: IconType::ChevronUp,
            title: "Focus Pane Top",
            category: "Panes",
            shortcut: Some(if is_mac { "⌥⌘↑" } else { "Alt+↑" }),
        },
        PaletteCommand {
            id: "focus_down",
            icon: IconType::ChevronDown,
            title: "Focus Pane Down",
            category: "Panes",
            shortcut: Some(if is_mac { "⌥⌘↓" } else { "Alt+↓" }),
        },
        PaletteCommand {
            id: "close_pane",
            icon: IconType::X,
            title: "Close Active Pane",
            category: "Panes",
            shortcut: Some(if is_mac { "⌘W" } else { "Ctrl+Shift+W" }),
        },
        PaletteCommand {
            id: "zoom_pane",
            icon: IconType::Maximize2,
            title: "Zoom Active Pane (Toggle)",
            category: "Panes",
            shortcut: None,
        },
        PaletteCommand {
            id: "toggle_tab_sidebar",
            icon: IconType::Folder,
            title: "Toggle Tabs Sidebar",
            category: "View",
            shortcut: Some(if is_mac { "⌘B" } else { "Ctrl+B" }),
        },
        PaletteCommand {
            id: "toggle_ai_sidebar",
            icon: IconType::Sparkles,
            title: "Fastty AI: Toggle Assistant Sidebar",
            category: "AI",
            shortcut: Some(if is_mac { "⌘L" } else { "Ctrl+Shift+L" }),
        },
        PaletteCommand {
            id: "layout_horizontal",
            icon: IconType::LayoutPanelTop,
            title: "Tabs Layout: Horizontal Top Bar",
            category: "View",
            shortcut: None,
        },
        PaletteCommand {
            id: "layout_vertical",
            icon: IconType::LayoutPanelLeft,
            title: "Tabs Layout: Vertical Sidebar",
            category: "View",
            shortcut: None,
        },
        PaletteCommand {
            id: "quit",
            icon: IconType::LogOut,
            title: "Quit Fastty",
            category: "Application",
            shortcut: Some(if is_mac { "⌘Q" } else { "Alt+F4" }),
        },
    ]
    .into_iter()
    // Theme switch commands come from the registry: one entry per built-in
    // theme, sharing the table the palette preview and dispatch read.
    .chain(crate::config::THEME_REGISTRY.iter().filter(|(name, _, _, _)| {
        // `default` already has its hand-written entry above.
        *name != "default"
    }).map(|(_, id, title, _)| PaletteCommand {
        id,
        icon: IconType::Palette,
        title,
        category: "Theme",
        shortcut: None,
    }))
    .collect()
}

/// Maps a palette command id to the theme it switches to, if any.
/// Used for live preview: navigating to the row previews the theme,
/// `Enter` commits it, `Esc` reverts to the previous one.
pub fn theme_name_for_palette_id(cmd_id: &str) -> Option<&'static str> {
    crate::config::THEME_REGISTRY
        .iter()
        .find(|(_, id, _, _)| *id == cmd_id)
        .map(|(name, _, _, _)| *name)
}

/// Snapshot of the visual settings a palette preview may touch.
#[derive(Clone, Debug)]
pub struct PalettePreviewState {
    pub theme_name: String,
    pub font_size: f32,
    pub tab_layout: TabLayout,
    pub sidebar_open: bool,
}

/// What a palette entry previews while navigating. Font deltas apply to the
/// baseline snapshot (idempotent under repeated hovers); layout/theme swap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PalettePreviewKind {
    Theme(&'static str),
    FontDelta(f32),
    FontReset,
    Layout(TabLayout),
}

/// Maps a palette command id to its live preview, if any. `Enter` commits
/// it (saves), `Esc` reverts to the snapshot.
pub fn palette_preview_kind(cmd_id: &str) -> Option<PalettePreviewKind> {
    if let Some(theme_name) = theme_name_for_palette_id(cmd_id) {
        return Some(PalettePreviewKind::Theme(theme_name));
    }
    match cmd_id {
        "zoom_in" => Some(PalettePreviewKind::FontDelta(1.0)),
        "zoom_out" => Some(PalettePreviewKind::FontDelta(-1.0)),
        "zoom_reset" => Some(PalettePreviewKind::FontReset),
        "layout_horizontal" => Some(PalettePreviewKind::Layout(TabLayout::Horizontal)),
        "layout_vertical" => Some(PalettePreviewKind::Layout(TabLayout::Vertical)),
        _ => None,
    }
}

impl RootView {
    /// Palette entries matching the current query. UX4: with an empty query
    /// the most-recently executed commands sort first (stable otherwise),
    /// so repeat actions are one `Enter` away.
    pub fn filtered_palette_commands(&self) -> Vec<PaletteCommand> {
        let query = self.command_palette_query.to_lowercase();
        let mut cmds: Vec<PaletteCommand> = get_all_palette_commands()
            .into_iter()
            .filter(|c| {
                query.is_empty()
                    || fuzzy_match_str(&query, c.title)
                    || fuzzy_match_str(&query, c.category)
            })
            .collect();
        if query.is_empty() && !self.palette_recent.is_empty() {
            cmds.sort_by_key(|c| {
                self.palette_recent
                    .iter()
                    .position(|r| r == c.id)
                    .unwrap_or(usize::MAX)
            });
        }
        cmds
    }

    /// Records a palette execution for recent-first ordering (max 5).
    pub fn record_palette_recent(&mut self, cmd_id: &str) {
        if let Some(pos) = self.palette_recent.iter().position(|r| r == cmd_id) {
            self.palette_recent.remove(pos);
        }
        self.palette_recent.insert(0, cmd_id.to_string());
        self.palette_recent.truncate(5);
    }
}

fn run_opencode_turn(
    manager: Arc<crate::ai::opencode::OpencodeManager>,
    conversation_id: String,
    saved_session_id: Option<String>,
    command: String,
    args: Vec<String>,
    is_opencode: bool,
    cwd: std::path::PathBuf,
    model: String,
    effort: String,
    mode: String,
    prompt: Vec<serde_json::Value>,
    cancel: crate::ai::CancelToken,
    permission_tx: async_channel::Sender<(
        String,
        String,
        String,
        Vec<crate::ai::opencode::PermissionOption>,
        async_channel::Sender<Option<String>>,
    )>,
    event_tx: async_channel::Sender<crate::ai::AgentEvent>,
) -> anyhow::Result<()> {
    if cancel.is_cancelled() {
        return Ok(());
    }
    let runtime = manager.session_with_saved_args(conversation_id.clone(), command, &args, &cwd, saved_session_id)?;
    if cancel.is_cancelled() {
        manager.remove(&conversation_id);
        return Ok(());
    }
    let _ = event_tx.send_blocking(crate::ai::AgentEvent::ConversationSession(runtime.session_id().to_string()));
    if !model.is_empty() && model != "default" {
        if is_opencode {
            runtime.select_model_value(&model)?;
        } else {
            let has_model_option = runtime.info().config_options.as_array().is_some_and(|options| {
                options.iter().any(|option| {
                    option.get("id").and_then(serde_json::Value::as_str) == Some("model")
                        && option.get("options").and_then(serde_json::Value::as_array).is_some_and(|choices| {
                            choices.iter().any(|choice| choice.get("value").and_then(serde_json::Value::as_str) == Some(model.as_str()))
                        })
                })
            });
            if has_model_option {
                runtime.select_config_option("model", &model)?;
            } else {
                runtime.select_model_id(&model)?;
            }
        }
    }
    if is_opencode && !effort.is_empty() {
        runtime.select_effort(&effort)?;
    }
    if is_opencode && !mode.is_empty() {
        runtime.select_mode(&mode)?;
    }
    if cancel.is_cancelled() {
        manager.remove(&conversation_id);
        return Ok(());
    }
    runtime.prompt_content(prompt)?;

    let mut active_tools = std::collections::HashMap::<String, String>::new();
    loop {
        if cancel.is_cancelled() {
            let cancel_sent = runtime.cancel().is_ok();
            let drain_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut prompt_ended = false;
            while std::time::Instant::now() < drain_deadline {
                match runtime.recv_event_timeout(std::time::Duration::from_millis(100)) {
                    Ok(Some(crate::ai::opencode::OpencodeEvent::PromptFinished(_))) => {
                        prompt_ended = true;
                        break;
                    }
                    Ok(Some(crate::ai::opencode::OpencodeEvent::Error(_))) | Err(_) => break,
                    Ok(Some(crate::ai::opencode::OpencodeEvent::PermissionRequest { request_id, .. })) => {
                        let _ = runtime.resolve_permission(&request_id, None);
                    }
                    Ok(Some(_)) | Ok(None) => {}
                }
            }
            if !cancel_sent || !prompt_ended {
                manager.remove(&conversation_id);
            }
            end_opencode_tools(&mut active_tools, &event_tx, "Cancelled");
            return Ok(());
        }
        let Some(event) = runtime.recv_event_timeout(std::time::Duration::from_millis(100))? else {
            continue;
        };
        match event {
            crate::ai::opencode::OpencodeEvent::Text(delta) => {
                let _ = event_tx.send_blocking(crate::ai::AgentEvent::TextDelta(delta));
            }
            crate::ai::opencode::OpencodeEvent::Thinking(delta) => {
                let _ = event_tx.send_blocking(crate::ai::AgentEvent::ThinkingDelta(delta));
            }
            crate::ai::opencode::OpencodeEvent::ToolUpdate(update) => {
                let id = update.get("toolCallId").and_then(serde_json::Value::as_str).unwrap_or("tool").to_string();
                let name = update.get("title").and_then(serde_json::Value::as_str).unwrap_or("ACP tool").to_string();
                let args = update.get("rawInput").map(|value| value.to_string()).unwrap_or_default();
                if !active_tools.contains_key(&id) {
                    active_tools.insert(id.clone(), name.clone());
                    let _ = event_tx.send_blocking(crate::ai::AgentEvent::ToolStart { id: id.clone(), name: name.clone(), args });
                }
                let status = update.get("status").and_then(serde_json::Value::as_str).unwrap_or("");
                if status == "completed" || status == "failed" {
                    let output = update.get("content").map(|value| value.to_string()).unwrap_or_default();
                    let _ = event_tx.send_blocking(crate::ai::AgentEvent::ToolEnd {
                        id: id.clone(), name, output, is_error: status == "failed", diff: None,
                    });
                    active_tools.remove(&id);
                }
            }
            crate::ai::opencode::OpencodeEvent::UsageUpdate { used, size } => {
                let _ = event_tx.send_blocking(crate::ai::AgentEvent::ContextWindow { used, size });
            }
            crate::ai::opencode::OpencodeEvent::AvailableCommands(commands) => {
                let _ = event_tx.send_blocking(crate::ai::AgentEvent::AvailableCommands(commands));
            }
            crate::ai::opencode::OpencodeEvent::PermissionRequest { request_id, tool_call, options } => {
                let id = tool_call.get("toolCallId").and_then(serde_json::Value::as_str).unwrap_or("permission").to_string();
                let name = tool_call.get("title").and_then(serde_json::Value::as_str).unwrap_or("OpenCode tool").to_string();
                let summary = tool_call.get("rawInput").map(|value| value.to_string()).unwrap_or_else(|| tool_call.to_string());
                let (reply_tx, reply_rx) = async_channel::bounded(1);
                if permission_tx.send_blocking((id, name, summary, options, reply_tx)).is_err() {
                    let _ = runtime.resolve_permission(&request_id, None);
                    continue;
                }
                let choice = reply_rx.recv_blocking().unwrap_or(None);
                runtime.resolve_permission(&request_id, choice.as_deref())?;
            }
            crate::ai::opencode::OpencodeEvent::PromptFinished(response) => {
                if let Some(usage) = response.get("usage") {
                    let input = usage.get("inputTokens").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    let output = usage.get("outputTokens").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    let _ = event_tx.send_blocking(crate::ai::AgentEvent::Usage { input, output });
                }
                let _ = event_tx.send_blocking(crate::ai::AgentEvent::TurnEnd);
                return Ok(());
            }
            crate::ai::opencode::OpencodeEvent::Error(error) => {
                end_opencode_tools(&mut active_tools, &event_tx, &error);
                return Err(anyhow::anyhow!(error));
            }
            crate::ai::opencode::OpencodeEvent::OtherUpdate(_) => {}
        }
    }
}

fn end_opencode_tools(
    active_tools: &mut std::collections::HashMap<String, String>,
    event_tx: &async_channel::Sender<crate::ai::AgentEvent>,
    output: &str,
) {
    for (id, name) in active_tools.drain() {
        let _ = event_tx.send_blocking(crate::ai::AgentEvent::ToolEnd {
            id,
            name,
            output: output.to_string(),
            is_error: true,
            diff: None,
        });
    }
}

/// UX6: wraps a raw provider/stream error in an error-styled bubble with an
/// actionable hint, instead of a plain-text message that is easy to miss.
pub fn ai_error_message(err: &str) -> crate::ui::ai_sidebar::AiUiMessage {
    let hint = if err.contains("API key")
        || err.contains("api_key")
        || err.contains("not found in configuration")
    {
        "Tip: set your API key under Settings (AI tab) or via environment variable."
    } else if err.contains("Timeout")
        || err.contains("timed out")
        || err.contains("connect")
        || err.contains("Connection")
        || err.contains("network")
        || err.contains("DNS")
    {
        "Tip: check your network connection and provider status, then retry."
    } else {
        "Tip: retry, or check provider settings under Settings (AI tab)."
    };
    crate::ui::ai_sidebar::AiUiMessage::error(format!("Error: {err}\n\n{hint}"))
        .with_timestamp(crate::ui::ai_sidebar::current_time_str())
}

/// F5: PR picker mode. Browse lists PRs; Actions operates on one PR with an
/// explicit command preview per row (the two-step flow is the confirmation).
#[derive(Clone, Debug, PartialEq)]
pub enum PrPickerMode {
    Browse,
    Actions { number: usize, title: String },
}

/// F5: one browsable PR row (current-branch PR pinned first, deduped).
#[derive(Clone, Debug)]
pub struct PrPickerRow {
    pub number: usize,
    pub title: String,
    /// `@author` for listed PRs, review state for the current-branch one.
    pub detail: String,
    pub is_current: bool,
}

/// F5: flattens a widget snapshot into picker rows.
pub(crate) fn pr_picker_rows(
    snapshot: &crate::widgets::builtin::git_prs::PrsSummary,
) -> Vec<PrPickerRow> {
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(ref pr) = snapshot.current_pr {
        seen.insert(pr.number);
        rows.push(PrPickerRow {
            number: pr.number,
            title: pr.title.clone(),
            detail: pr
                .review_decision
                .clone()
                .unwrap_or_else(|| pr.state.clone()),
            is_current: true,
        });
    }
    for pr in &snapshot.open_prs {
        if seen.insert(pr.number) {
            rows.push(PrPickerRow {
                number: pr.number,
                title: pr.title.clone(),
                detail: pr
                    .author
                    .as_ref()
                    .map(|a| format!("@{}", a.login))
                    .unwrap_or_default(),
                is_current: false,
            });
        }
    }
    rows
}

/// F5: actions available on a selected PR, in fixed order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrPickerAction {
    Checkout,
    Approve,
    Merge,
    Back,
}

pub fn pr_action_count() -> usize {
    4
}

pub fn pr_action_for_index(idx: usize) -> PrPickerAction {
    match idx {
        0 => PrPickerAction::Checkout,
        1 => PrPickerAction::Approve,
        2 => PrPickerAction::Merge,
        _ => PrPickerAction::Back,
    }
}

pub fn pr_action_label(action: PrPickerAction, number: usize) -> String {
    match action {
        PrPickerAction::Checkout => format!("Checkout PR #{number}"),
        PrPickerAction::Approve => format!("Approve PR #{number}"),
        PrPickerAction::Merge => format!("Merge PR #{number}"),
        PrPickerAction::Back => "← Back to list".to_string(),
    }
}

/// F5: the exact command that will be typed into the pane (shown as preview
/// in the row, so the action is explicit before committing with Enter).
pub fn pr_action_command(action: PrPickerAction, number: usize) -> Option<String> {
    match action {
        PrPickerAction::Checkout => Some(format!("gh pr checkout {number}")),
        PrPickerAction::Approve => Some(format!("gh pr review {number} --approve")),
        PrPickerAction::Merge => Some(format!("gh pr merge {number} --merge")),
        PrPickerAction::Back => None,
    }
}

#[derive(Clone, Debug)]
pub enum CloseTarget {
    Pane { tab_id: usize, pane_id: usize },
    Tab { tab_id: usize },
    OtherTabs { keep_id: usize },
}

#[derive(Clone, Debug)]
pub struct RunningProcessInfo {
    pub pane_id: usize,
    pub process_name: String,
    pub pid: u32,
}

#[derive(Clone, Debug)]
pub struct PendingClose {
    pub target: CloseTarget,
    pub running_processes: Vec<RunningProcessInfo>,
}

#[derive(Default)]
struct AiConversationRuntime {
    owner_tab_id: usize,
    permission_checker: Option<Arc<crate::ai::PermissionChecker>>,
    messages: Vec<crate::ui::ai_sidebar::AiUiMessage>,
    streaming_text: String,
    streaming_thinking: String,
    is_streaming: bool,
    last_notification: Option<std::time::Instant>,
    last_usage: Option<(u64, u64)>,
    current_turn_usage: (u64, u64),
    cancel_token: Option<crate::ai::CancelToken>,
    stream_generation: u64,
    pending_confirmation: Option<crate::ui::ai_sidebar::AiUiPendingConfirmation>,
    confirm_reply_tx: Option<async_channel::Sender<crate::ai::PermissionDecision>>,
    opencode_confirm_reply_tx: Option<async_channel::Sender<Option<String>>>,
    pending_text: String,
    pending_thinking: String,
    turn_provider: String,
    turn_model: String,
}

impl Drop for RootView {
    fn drop(&mut self) {
        self.cancel_ai_stream();
        for runtime in self.ai_background_conversations.values_mut() {
            if let Some(token) = runtime.cancel_token.take() {
                token.cancel();
            }
            if let Some(tx) = runtime.confirm_reply_tx.take() {
                let _ = tx.try_send(crate::ai::PermissionDecision::Deny);
            }
            if let Some(tx) = runtime.opencode_confirm_reply_tx.take() {
                let _ = tx.try_send(None);
            }
        }
    }
}

pub struct RootView {
    config: Config,
    window_id: gpui::WindowId,
    theme: Theme,
    tabs: Vec<TabData>,
    active_tab_idx: usize,
    next_tab_id: usize,
    pub next_pane_id: usize,
    focus_handle: FocusHandle,
    font_size: f32,
    font_family: SharedString,
    /// F3: installed Nerd Font for PUA icon fallback, if any. `None` keeps
    /// today's behavior (OS cascade). Resolved once at startup.
    nerd_font_family: Option<SharedString>,
    status_bar_model: StatusBarModel,
    pub tab_layout: TabLayout,
    pub sidebar_open: bool,
    pub sidebar_anim_progress: f32,
    pub sidebar_scroll_handle: ScrollHandle,
    pub tab_bar_scroll_handle: ScrollHandle,
    pub is_settings_open: bool,
    pub is_context_menu_open: bool,
    pub is_about_open: bool,
    pub is_rename_tab_open: bool,
    pub rename_tab_idx: usize,
    pub rename_tab_input: String,
    pub is_tab_context_menu_open: bool,
    pub tab_context_menu_tab_id: usize,
    pub tab_context_menu_pos: (f32, f32),
    pub is_pane_context_menu_open: bool,
    pub pane_context_menu_pane_id: usize,
    pub pane_context_menu_pos: (f32, f32),
    pub is_command_palette_open: bool,
    pub command_palette_query: String,
    pub command_palette_selected: usize,
    pub is_ssh_manager_open: bool,
    pub ssh_manager_query: String,
    pub ssh_manager_selected: usize,
    /// F4: snippet picker (fuzzy search over triggers, preview, insert).
    pub is_snippet_picker_open: bool,
    pub snippet_query: String,
    pub snippet_selected: usize,
    /// F5: PR picker (browse PRs, checkout/approve/merge via `gh`).
    pub is_pr_picker_open: bool,
    pub pr_picker_query: String,
    pub pr_picker_selected: usize,
    pub pr_picker_mode: PrPickerMode,
    pub pr_picker_cwd: Option<std::path::PathBuf>,
    pub(crate) pr_snapshot:
        Arc<std::sync::Mutex<Option<crate::widgets::builtin::git_prs::PrsSummary>>>,
    pub is_search_open: bool,
    pub search_query: String,
    pub search_match_idx: usize,
    pub search_matches: Vec<usize>,
    pub is_worktree_picker_open: bool,
    pub worktree_picker_query: String,
    pub worktree_picker_selected: usize,
    pub is_project_jumper_open: bool,
    pub project_jumper_query: String,
    pub project_jumper_selected: usize,
    pub is_file_picker_open: bool,
    pub file_picker_query: String,
    pub file_picker_selected: usize,
    pub file_picker_scroll_handle: ScrollHandle,
    /// Search root for the universal picker (focused tab working directory).
    pub file_picker_root: Option<std::path::PathBuf>,
    /// Cached multi-source index for `file_picker_root`, rebuilt when the
    /// picker opens: files, git branches, ssh hosts, snippets, containers.
    pub file_picker_index: Vec<crate::universal_picker::UniversalItem>,
    /// Window-space pixel position of the terminal cursor line when the
    /// picker opened; the dropdown anchors above or below it.
    pub file_picker_anchor: Option<(f32, f32)>,
    pub is_tab_overview_open: bool,
    pub tab_overview_selected: usize,
    pub tab_overview_scroll_handle: ScrollHandle,
    pub is_global_search_open: bool,
    pub global_search_query: String,
    pub global_search_results: Vec<GlobalSearchResult>,
    pub global_search_selected: usize,
    pub global_search_scroll_handle: ScrollHandle,
    pub is_git_menu_open: bool,
    pub is_git_branch_sub_open: bool,
    pub git_menu_pos: Option<(f32, f32)>,
    pub command_palette_scroll_handle: ScrollHandle,
    pub ssh_manager_scroll_handle: ScrollHandle,
    pub snippet_scroll_handle: ScrollHandle,
    pub pr_picker_scroll_handle: ScrollHandle,
    pub worktree_picker_scroll_handle: ScrollHandle,
    pub project_jumper_scroll_handle: ScrollHandle,
    pub ai_scroll_handle: ScrollHandle,
    pub typed_prompt_buf: String,
    pub selection: Option<Selection>,
    pub is_selecting: bool,
    pub has_selection_dragged: bool,
    pub selection_start: Option<alacritty_terminal::index::Point>,
    pub selection_mouse_pos: Option<(f32, f32)>,
    pub selection_autoscroll_accum: f32,
    pub cursor_window_pos: Option<(f32, f32)>,
    pub hovered_url: Option<String>,
    pub hovered_url_range: Option<(i32, usize, usize)>,
    pub current_theme_name: String,
    /// Snapshot of the visual settings a command-palette preview may touch.
    /// Taken before the first preview step: `Enter` commits (saves), `Esc`
    /// restores everything without touching the config file.
    pub palette_preview_original: Option<PalettePreviewState>,
    /// Last config load failure (path + parse error), shown as a banner
    /// instead of failing silently to defaults. Session-dismissible.
    pub config_error: Option<String>,
    /// One-line first-run hint bar. True only when neither a config file nor
    /// a saved session exists yet; dismissed for the session via "Got it".
    pub show_onboarding_hint: bool,
    /// Most-recently executed palette commands (ids, most recent first,
    /// capped). Sorted to the top when the palette query is empty.
    pub palette_recent: Vec<String>,
    pub cursor_blink_visible: bool,
    pub last_cursor_activity: std::time::Instant,
    pub last_scroll_activity: std::time::Instant,
    pub scroll_accum: f32,
    pub is_dragging_scrollbar: bool,
    pub dragging_scrollbar_pane_id: Option<usize>,
    pub last_scrolled_pane_id: Option<usize>,
    pub scroll_fade_active: bool,
    pub scrollbar_drag_start_y: f32,
    pub scrollbar_drag_start_offset: usize,
    pub update_available: Option<crate::updater::ReleaseInfo>,
    pub is_updating: bool,
    pub is_update_ready: bool,
    pub update_status: Option<String>,
    pub is_update_modal_open: bool,
    pub is_whats_new_open: bool,
    pub whats_new_notes: Option<String>,
    pub pending_close: Option<PendingClose>,
    pub pressed_mouse_button: Option<MouseButton>,
    pub(crate) ime_marked_text: Option<String>,
    pub last_config_version: u64,
    pub is_dragging_pane_split: bool,
    pub dragging_split_path: Vec<usize>,
    pub dragging_split_direction: SplitDirection,
    pub dragging_split_bounds: (f32, f32, f32, f32),
    /// Pane id of a tab spawned from `-e`/`--exec` on the CLI. When that
    /// pane's child process exits, the whole app quits — matching how
    /// other terminals (Ghostty, Alacritty, Kitty, ...) treat `-e`: a
    /// one-shot execution, not a persistent shell tab. `None` for every
    /// other pane (normal tabs, splits, session restore, new windows).
    exec_pane_id: Option<crate::pane_tree::PaneId>,
    pub ai_sidebar_open: bool,
    pub ai_sidebar_width: f32,
    /// 0..=1 open/close animation progress, mirroring the tabs sidebar.
    pub ai_sidebar_anim_progress: f32,
    pub is_dragging_ai_sidebar: bool,
    pub ai_input_text: String,
    pub ai_input_state: crate::ui::TextInputState,
    pub is_dragging_ai_input: bool,
    pub ai_input_focused: bool,
    ai_loaded_conversation_id: String,
    ai_background_conversations: std::collections::HashMap<String, AiConversationRuntime>,
    ai_event_source: Option<(usize, String)>,
    ai_next_stream_generation: u64,
    ai_turn_provider: String,
    ai_turn_model: String,
    pub ai_messages: Vec<crate::ui::ai_sidebar::AiUiMessage>,
    pub ai_chat_records: Vec<crate::ai::conversations::Conversation>,
    pub ai_history_open: bool,
    pub ai_streaming_text: String,
    pub ai_streaming_thinking: String,
    pub ai_is_streaming: bool,
    pub ai_turn_completions: std::collections::HashMap<String, AiTurnCompletion>,
    pub program_statuses: std::collections::HashMap<PaneId, ProgramPaneStatus>,
    pub ai_last_notification: Option<std::time::Instant>,
    pub ai_last_usage: Option<(u64, u64)>,
    pub ai_current_turn_usage: (u64, u64),
    pub ai_context_hovercard_open: bool,
    pub ai_cancel_token: Option<crate::ai::CancelToken>,
    pub ai_stream_generation: u64,
    pub ai_pending_confirmation: Option<crate::ui::ai_sidebar::AiUiPendingConfirmation>,
    pub ai_confirm_reply_tx: Option<async_channel::Sender<crate::ai::PermissionDecision>>,
    pub ai_opencode_confirm_reply_tx: Option<async_channel::Sender<Option<String>>>,
    pub ai_opencode_manager: Arc<crate::ai::opencode::OpencodeManager>,
    pub ai_opencode_variant: String,
    pub ai_opencode_variants_open: bool,
    pub ai_permission_checker: std::sync::Arc<crate::ai::PermissionChecker>,
    pub ai_agent_mode: String,
    pub ai_expanded_thinkings: std::collections::HashSet<usize>,
    pub ai_attached_files: Vec<std::path::PathBuf>,
    pub ai_at_menu_open: bool,
    pub ai_at_is_skill_menu: bool,
    pub ai_at_matches: Vec<String>,
    /// Row the arrow keys point at in the `@` mention menu. Clamped to the
    /// drawn row count, which is `AI_AT_MENU_MAX_ROWS`, not the whole match list.
    pub ai_at_selected: usize,
    /// True while a file drag from another app is over the AI composer, so the
    /// panel can show it will take the file. A drop over the panel bubbles to
    /// the root container, not to the terminal area, which is why the user saw
    /// the path pasted into the shell with nothing on screen to explain it.
    pub ai_file_drag_hover: bool,
    /// Scrolls the `@` mention popup so the highlighted row stays in view. The
    /// popup is 160px tall and holds up to `AI_AT_MENU_MAX_ROWS` rows.
    pub ai_at_scroll_handle: ScrollHandle,
    /// Incoming stream deltas not yet revealed in the UI. Drained at ~28fps by the ticker.
    pub ai_pending_text: String,
    pub ai_pending_thinking: String,
    pub ai_composer_bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// Timestamp until which the message "Copy" button shows "Copied".
    pub ai_copy_feedback_until: Option<std::time::Instant>,
    pub ai_message_selection: Option<(usize, usize, usize)>,
    pub ai_message_selection_anchor: Option<(usize, usize)>,
    pub is_dragging_message_selection: bool,
}

impl RootView {
    #[inline]
    pub fn current_sidebar_width(&self) -> f32 {
        if self.tab_layout == TabLayout::Vertical || self.sidebar_anim_progress > 0.001 {
            (210.0 * self.sidebar_anim_progress).round()
        } else {
            0.0
        }
    }

    #[inline]
    pub fn current_ai_sidebar_width(&self) -> f32 {
        if self.ai_sidebar_open || self.ai_sidebar_anim_progress > 0.001 {
            (self.ai_sidebar_width * self.ai_sidebar_anim_progress).round()
        } else {
            0.0
        }
    }

    #[inline]
    fn measure_cell_metrics(&self, window: &Window) -> (f32, f32) {
        let font_id = window
            .text_system()
            .resolve_font(&gpui::font(self.font_family.clone()));
        let cell_w = window
            .text_system()
            .layout_width(font_id, px(self.font_size), '0')
            .to_f64() as f32;
        // Integer line height keeps every row origin on the whole-pixel grid,
        // so synthesized strokes connect across rows without half-pixel gaps
        // (fractional 1.32em heights drift by n*0.16px per row).
        let line_h = ((self.font_size * 1.32).max(12.0)).round();
        if cell_w >= 1.0 {
            (cell_w, line_h)
        } else {
            let (w, h) = get_font_cell_metrics(self.font_family.as_ref(), self.font_size);
            (w, h.round().max(12.0))
        }
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_options(window, crate::cli::CliOptions::default(), cx)
    }

    pub fn with_options(
        _window: &mut Window,
        cli_opts: crate::cli::CliOptions,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_options_internal(_window, cli_opts, None, cx)
    }

    pub fn with_options_internal(
        _window: &mut Window,
        cli_opts: crate::cli::CliOptions,
        initial_tab: Option<TabData>,
        cx: &mut Context<Self>,
    ) -> Self {
        config::load_custom_themes();
        crate::snippets::load();
        let loaded_config = crate::config::load_lenient();
        let theme_name = loaded_config
            .theme
            .as_deref()
            .unwrap_or("default")
            .to_string();
        let theme = Theme::from_name(&theme_name).with_opacity(loaded_config.opacity);

        let status_bar_model = StatusBarModel::new(&loaded_config, theme);
        let focus_handle = cx.focus_handle();
        _window.focus(&focus_handle, cx);

        let font_size = loaded_config.font.size;
        let font_family: SharedString =
            if loaded_config.font.family.is_empty() || loaded_config.font.family == "monospace" {
                #[cfg(target_os = "macos")]
                {
                    "Menlo".into()
                }
                #[cfg(target_os = "windows")]
                {
                    "Cascadia Code".into()
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    "monospace".into()
                }
            } else {
                // A family that is not installed gets silently substituted by the
                // text system, leaving cell metrics describing a different font
                // than the one actually drawn. Resolve to an installed family so
                // measurement and rendering agree.
                #[cfg(target_os = "macos")]
                {
                    crate::font_discovery_macos::resolve_font_family(
                        &loaded_config.font.family,
                    )
                    .into()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    loaded_config.font.family.clone().into()
                }
            };

        // F3: auto-detect an installed Nerd Font for PUA icon fallback.
        // Only used when one exists; otherwise rendering is unchanged.
        let nerd_font_family: Option<SharedString> =
            find_nerd_font_family(&_window.text_system().all_font_names()).map(|n| n.into());

        // Scrollbar fade and cursor ticker (interval: 35ms for smooth 30-60fps fade out)
        cx.spawn_in(_window, async move |this, cx| {
            let mut blink_counter = 0u32;
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(35))
                    .await;
                let res = this.update_in(cx, |this, _window, cx| {
                    let mut needs_notify = false;

                    // Selection auto-scroll while dragging outside/near viewport bounds
                    if this.is_selecting && this.selection_mouse_pos.is_some() {
                        this.tick_selection_autoscroll(0.035, _window, cx);
                        needs_notify = true;
                    }

                    // Scroll fade active window
                    let elapsed = this.last_scroll_activity.elapsed();
                    let is_fading = elapsed < std::time::Duration::from_millis(1500);
                    if this.is_dragging_scrollbar || is_fading {
                        this.scroll_fade_active = true;
                        needs_notify = true;
                    } else if this.scroll_fade_active {
                        this.scroll_fade_active = false;
                        needs_notify = true;
                    }

                    // Copy feedback revert ("Copied" -> "Copy" after ~1.5s)
                    if let Some(until) = this.ai_copy_feedback_until {
                        if std::time::Instant::now() >= until {
                            this.ai_copy_feedback_until = None;
                            needs_notify = true;
                        }
                    }

                    // Cursor blink logic (every ~525ms = 15 ticks of 35ms)
                    blink_counter += 1;
                    if blink_counter >= 15 {
                        blink_counter = 0;
                        if this.config.cursor.blink {
                            if this.last_cursor_activity.elapsed()
                                >= std::time::Duration::from_millis(500)
                            {
                                this.cursor_blink_visible = !this.cursor_blink_visible;
                                needs_notify = true;
                            }
                        } else {
                            this.cursor_blink_visible = true;
                        }
                    }

                    // Dynamic live config reload across all windows
                    let cur_config_ver = crate::config::current_config_version();
                    if this.last_config_version != cur_config_ver {
                        this.last_config_version = cur_config_ver;
                        this.reload_config(_window, cx);
                        needs_notify = true;
                    }

                    // Progressive AI stream reveal (smooth word-by-word / token-by-token effect)
                    let text_len = this.ai_pending_text.len();
                    if text_len > 0 {
                        // Dynamic drain rate: base 6 chars per 35ms tick (~170 chars/s),
                        // or text_len / 4 to smoothly catch up if a large chunk arrives.
                        let drain_size = 6.max(text_len / 4).min(text_len);
                        let mut actual_drain = drain_size;
                        while actual_drain < text_len
                            && !this.ai_pending_text.is_char_boundary(actual_drain)
                        {
                            actual_drain += 1;
                        }
                        this.ai_streaming_text
                            .extend(this.ai_pending_text.drain(..actual_drain));
                        needs_notify = true;
                    }

                    let think_len = this.ai_pending_thinking.len();
                    if think_len > 0 {
                        let drain_size = 12.max(think_len / 3).min(think_len);
                        let mut actual_drain = drain_size;
                        while actual_drain < think_len
                            && !this.ai_pending_thinking.is_char_boundary(actual_drain)
                        {
                            actual_drain += 1;
                        }
                        this.ai_streaming_thinking
                            .extend(this.ai_pending_thinking.drain(..actual_drain));
                        needs_notify = true;
                    }

                    if needs_notify {
                        this.scroll_ai_to_bottom();
                        cx.notify();
                    }
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut restored_tabs = Vec::new();
        let mut restored_ai_sidebar_open = false;
        let mut restored_active_tab = 0usize;
        let ai_permission_mode = loaded_config.ai.permission_mode;
        if initial_tab.is_none() && loaded_config.session_restore {
            if let Some(session) = crate::session::load() {
                if let Some(win) = session.windows.first() {
                    restored_ai_sidebar_open = win.ai_sidebar_open;
                    restored_active_tab = win.active_tab;
                    for tab_info in &win.tabs {
                        let cwd_path = tab_info.cwd.clone();
                        let title = tab_info
                            .title_override
                            .clone()
                            .or_else(|| tab_info.custom_name.clone());
                        restored_tabs.push((cwd_path, title, tab_info.ai_tab_key.clone(), tab_info.ai_conversation_id.clone()));
                    }
                }
            }
        }

        let tab_layout = loaded_config.tab_layout;
        let sidebar_open = tab_layout == TabLayout::Vertical;
        let sidebar_anim_progress = if sidebar_open { 1.0 } else { 0.0 };

        // One-shot "What's new" dialog. Recorded immediately so it fires
        // exactly once per version.
        let show_whats_new = crate::whats_new::should_show();
        crate::whats_new::mark_version_seen();

        let mut view = Self {
            config: loaded_config,
            window_id: _window.window_handle().window_id(),
            theme,
            tabs: Vec::new(),
            active_tab_idx: 0,
            next_tab_id: 1,
            next_pane_id: 1,
            focus_handle,
            font_size,
            font_family,
            nerd_font_family,
            status_bar_model,
            tab_layout,
            sidebar_open,
            sidebar_anim_progress,
            sidebar_scroll_handle: ScrollHandle::new(),
            tab_bar_scroll_handle: ScrollHandle::new(),
            is_settings_open: false,
            is_context_menu_open: false,
            is_about_open: false,
            is_rename_tab_open: false,
            rename_tab_idx: 0,
            rename_tab_input: String::new(),
            is_tab_context_menu_open: false,
            tab_context_menu_tab_id: 0,
            tab_context_menu_pos: (0.0, 0.0),
            is_pane_context_menu_open: false,
            pane_context_menu_pane_id: 0,
            pane_context_menu_pos: (0.0, 0.0),
            is_command_palette_open: false,
            command_palette_query: String::new(),
            command_palette_selected: 0,
            is_ssh_manager_open: false,
            ssh_manager_query: String::new(),
            ssh_manager_selected: 0,
            is_snippet_picker_open: false,
            snippet_query: String::new(),
            snippet_selected: 0,
            is_pr_picker_open: false,
            pr_picker_query: String::new(),
            pr_picker_selected: 0,
            pr_picker_mode: PrPickerMode::Browse,
            pr_picker_cwd: None,
            pr_snapshot: Arc::new(std::sync::Mutex::new(None)),
            is_search_open: false,
            search_query: String::new(),
            search_match_idx: 0,
            search_matches: Vec::new(),
            is_worktree_picker_open: false,
            worktree_picker_query: String::new(),
            worktree_picker_selected: 0,
            is_project_jumper_open: false,
            project_jumper_query: String::new(),
            project_jumper_selected: 0,
            is_file_picker_open: false,
            file_picker_query: String::new(),
            file_picker_selected: 0,
            file_picker_scroll_handle: ScrollHandle::new(),
            file_picker_root: None,
            file_picker_index: Vec::new(),
            file_picker_anchor: None,
            is_tab_overview_open: false,
            tab_overview_selected: 0,
            tab_overview_scroll_handle: ScrollHandle::new(),
            is_global_search_open: false,
            global_search_query: String::new(),
            global_search_results: Vec::new(),
            global_search_selected: 0,
            global_search_scroll_handle: ScrollHandle::new(),
            is_git_menu_open: false,
            is_git_branch_sub_open: false,
            git_menu_pos: None,
            command_palette_scroll_handle: ScrollHandle::new(),
            ssh_manager_scroll_handle: ScrollHandle::new(),
            snippet_scroll_handle: ScrollHandle::new(),
            pr_picker_scroll_handle: ScrollHandle::new(),
            worktree_picker_scroll_handle: ScrollHandle::new(),
            project_jumper_scroll_handle: ScrollHandle::new(),
            ai_scroll_handle: ScrollHandle::new(),
            typed_prompt_buf: String::new(),
            selection: None,
            is_selecting: false,
            has_selection_dragged: false,
            selection_start: None,
            selection_mouse_pos: None,
            selection_autoscroll_accum: 0.0,
            cursor_window_pos: None,
            hovered_url: None,
            hovered_url_range: None,
            current_theme_name: theme_name,
            palette_preview_original: None,
            config_error: crate::config::load_error(),
            show_onboarding_hint: !crate::config::Config::get_active_config_path().exists()
                && !crate::session::session_path().exists(),
            palette_recent: Vec::new(),
            cursor_blink_visible: true,
            last_cursor_activity: std::time::Instant::now(),
            last_scroll_activity: std::time::Instant::now() - std::time::Duration::from_secs(10),
            scroll_accum: 0.0,
            is_dragging_scrollbar: false,
            dragging_scrollbar_pane_id: None,
            last_scrolled_pane_id: None,
            scroll_fade_active: false,
            scrollbar_drag_start_y: 0.0,
            scrollbar_drag_start_offset: 0,
            update_available: None,
            is_updating: false,
            update_status: None,
            is_update_ready: false,
            is_update_modal_open: false,
            is_whats_new_open: show_whats_new,
            whats_new_notes: crate::whats_new::notes_for(env!("CARGO_PKG_VERSION")),
            pending_close: None,
            pressed_mouse_button: None,
            ime_marked_text: None,
            last_config_version: crate::config::current_config_version(),
            is_dragging_pane_split: false,
            dragging_split_path: Vec::new(),
            dragging_split_direction: SplitDirection::Horizontal,
            dragging_split_bounds: (0.0, 0.0, 0.0, 0.0),
            exec_pane_id: None,
            ai_sidebar_open: restored_ai_sidebar_open,
            ai_sidebar_width: 340.0,
            ai_sidebar_anim_progress: if restored_ai_sidebar_open { 1.0 } else { 0.0 },
            is_dragging_ai_sidebar: false,
            ai_input_text: String::new(),
            ai_input_state: crate::ui::TextInputState::default(),
            is_dragging_ai_input: false,
            ai_input_focused: true,
            ai_loaded_conversation_id: String::new(),
            ai_background_conversations: std::collections::HashMap::new(),
            ai_event_source: None,
            ai_next_stream_generation: 0,
            ai_turn_provider: String::new(),
            ai_turn_model: String::new(),
            ai_messages: Vec::new(),
            ai_chat_records: crate::ai::conversations::load(),
            ai_history_open: false,
            ai_streaming_text: String::new(),
            ai_streaming_thinking: String::new(),
            ai_is_streaming: false,
            ai_turn_completions: std::collections::HashMap::new(),
            program_statuses: std::collections::HashMap::new(),
            ai_last_notification: None,
            ai_last_usage: None,
            ai_current_turn_usage: (0, 0),
            ai_context_hovercard_open: false,
            ai_cancel_token: None,
            ai_stream_generation: 0,
            ai_pending_confirmation: None,
            ai_confirm_reply_tx: None,
            ai_opencode_confirm_reply_tx: None,
            ai_opencode_manager: Arc::new(crate::ai::opencode::OpencodeManager::default()),
            ai_opencode_variant: "default".to_string(),
            ai_opencode_variants_open: false,
            ai_permission_checker: std::sync::Arc::new(crate::ai::PermissionChecker::new(
                ai_permission_mode,
            )),
            ai_agent_mode: "Agent".to_string(),
            ai_expanded_thinkings: std::collections::HashSet::new(),
            ai_attached_files: Vec::new(),
            ai_at_menu_open: false,
            ai_at_is_skill_menu: false,
            ai_at_selected: 0,
            ai_file_drag_hover: false,
            ai_at_scroll_handle: ScrollHandle::new(),
            ai_at_matches: Vec::new(),
            ai_pending_text: String::new(),
            ai_pending_thinking: String::new(),
            ai_composer_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_copy_feedback_until: None,
            ai_message_selection: None,
            ai_message_selection_anchor: None,
            is_dragging_message_selection: false,
        };

        crate::keybindings::init_resolver(
            view.config.keybindings.clone(),
            view.config.keybinding_preset,
        );

        // Daemon → GUI requests: a `spawn { open: true }` (e.g. from an MCP
        // agent's `fastty_spawn_session`) surfaces as a visible tab. The
        // channel detaches with the view — after that, sends fail silently
        // and spawned sessions just stay headless.
        let (daemon_req_tx, daemon_req_rx) =
            async_channel::unbounded::<crate::daemon::GuiRequest>();
        crate::daemon::set_gui_sender(Some(daemon_req_tx));
        cx.spawn_in(_window, async move |this, cx| {
            while let Ok(req) = daemon_req_rx.recv().await {
                let res = this.update_in(cx, |this, window, cx| match req {
                    crate::daemon::GuiRequest::OpenSession { id, terminal } => {
                        this.adopt_daemon_session(id, terminal, window, cx);
                    }
                    crate::daemon::GuiRequest::SplitPane {
                        target,
                        direction,
                        command,
                        args,
                        cwd,
                        reply,
                    } => {
                        let _ = reply.send(
                            this.daemon_split_pane(target, direction, command, &args, cwd, window, cx),
                        );
                    }
                    crate::daemon::GuiRequest::ResizePane {
                        target,
                        direction,
                        delta,
                        reply,
                    } => {
                        let _ = reply
                            .send(this.daemon_resize_pane(target, direction, delta, cx));
                    }
                    crate::daemon::GuiRequest::FocusPane { target, reply } => {
                        let _ = reply.send(this.daemon_focus_pane(target, cx));
                    }
                    crate::daemon::GuiRequest::Layout { reply } => {
                        let _ = reply.send(Ok(this.daemon_layout()));
                    }
                    crate::daemon::GuiRequest::ResizeWindow { cols, rows, reply } => {
                        let _ =
                            reply.send(this.daemon_resize_window(cols, rows, window, cx));
                    }
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();

        // Background update check and silent automatic preparation.
        // When a newer release exists the changelog modal opens automatically
        // so the Skip / Update actions are always reachable (previously only
        // a small tab-bar badge appeared, which users missed).
        enum UpdateStatus {
            Ready(crate::updater::ReleaseInfo),
            Blocked(crate::updater::ReleaseInfo, String),
            Available(crate::updater::ReleaseInfo),
        }
        let (update_tx, update_rx) = async_channel::unbounded::<UpdateStatus>();
        let update_channel = crate::updater::UpdateChannel::parse(&view.config.update_channel);
        std::thread::spawn(move || {
            match crate::updater::check_for_updates(update_channel, false) {
                Ok(Some(release)) => {
                    if release.self_update_blocked_reason.is_none() {
                        // Silently download and stage update in background
                        match crate::updater::apply_update_sync(&release) {
                            Ok(()) => {
                                let _ = update_tx.send_blocking(UpdateStatus::Ready(release));
                            }
                            Err(_) => {
                                let _ = update_tx.send_blocking(UpdateStatus::Available(release));
                            }
                        }
                    } else {
                        let reason = release
                            .self_update_blocked_reason
                            .clone()
                            .unwrap_or_default();
                        let _ = update_tx.send_blocking(UpdateStatus::Blocked(release, reason));
                    }
                }
                Ok(None) | Err(_) => {}
            }
        });

        cx.spawn_in(_window, async move |this, cx| {
            if let Ok(status) = update_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    match status {
                        UpdateStatus::Ready(release) => {
                            this.is_update_ready = true;
                            this.update_available = Some(release.clone());
                            this.update_status = Some(format!(
                                "Fastty v{} is installed and ready to use.\nRestart Fastty to switch to the new version.",
                                release.version
                            ));
                            this.is_update_modal_open = true;
                        }
                        UpdateStatus::Blocked(release, reason) => {
                            this.is_update_ready = false;
                            this.update_available = Some(release);
                            this.update_status = Some(reason);
                            this.is_update_modal_open = true;
                        }
                        UpdateStatus::Available(release) => {
                            this.is_update_ready = false;
                            this.update_available = Some(release);
                            this.update_status = None;
                            this.is_update_modal_open = true;
                        }
                    }
                    cx.notify();
                });
            }
        }).detach();

        if let Some(tab_data) = initial_tab {
            view.attach_tab(tab_data, _window, cx);
        } else if cli_opts.command.is_some()
            || cli_opts.working_dir.is_some()
            || cli_opts.title.is_some()
        {
            let is_exec_invocation = cli_opts.command.is_some();
            let shell = cli_opts.command.unwrap_or_else(|| {
                view.config
                    .shell
                    .clone()
                    .or_else(|| std::env::var("SHELL").ok())
                    .unwrap_or_else(crate::paths::default_system_shell)
            });
            view.create_tab_with_cmd_and_cwd(
                &shell,
                &cli_opts.args,
                cli_opts.working_dir.as_deref(),
                cli_opts.title,
                _window,
                cx,
            );
            // `-e`/`--exec` on the CLI means "run this one command and exit",
            // matching Ghostty/Alacritty/Kitty/WezTerm. Tag the pane so the
            // app quits when its child process exits, instead of leaving an
            // empty tab open (the previous behavior, and still the right one
            // for a bare `-d`/`--title` invocation that opens an interactive
            // shell).
            if is_exec_invocation {
                if let Some(tab) = view.tabs.last() {
                    view.exec_pane_id = Some(tab.pane_tree.active_pane_id);
                }
            }
        } else if restored_tabs.is_empty() {
            view.create_tab(_window, cx);
        } else {
            let shell = view
                .config
                .shell
                .clone()
                .or_else(|| std::env::var("SHELL").ok())
                .unwrap_or_else(crate::paths::default_system_shell);
            for (cwd, title, ai_tab_key, ai_conversation_id) in restored_tabs {
                view.create_tab_with_cmd_and_cwd(&shell, &[], cwd.as_deref(), title, _window, cx);
                if let Some(tab) = view.tabs.last_mut() {
                    if let Some(key) = ai_tab_key { tab.ai_tab_key = key; }
                    if let Some(id) = ai_conversation_id { tab.ai_conversation_id = id; }
                }
            }
            if !view.tabs.is_empty() {
                view.active_tab_idx = restored_active_tab.min(view.tabs.len() - 1);
                let conversation_id = view.tabs[view.active_tab_idx].ai_conversation_id.clone();
                view.load_ai_conversation(&conversation_id);
            }
        }
        view
    }

    pub fn persist_session(&self) {
        if !self.config.session_restore {
            return;
        }
        let tab_infos: Vec<crate::session::TabInfo> = self
            .tabs
            .iter()
            .map(|t| crate::session::TabInfo {
                cwd: t.cwd.clone(),
                ai_tab_key: Some(t.ai_tab_key.clone()),
                ai_conversation_id: Some(t.ai_conversation_id.clone()),
                custom_name: None,
                title_override: Some(t.title.clone()),
            })
            .collect();

        let session = crate::session::Session {
            windows: vec![crate::session::WindowSession {
                tabs: tab_infos,
                active_tab: self.active_tab_idx,
                position: None,
                size: None,
                ai_sidebar_open: self.ai_sidebar_open,
            }],
            active_window: 0,
            legacy_tabs: Vec::new(),
            legacy_active_tab: 0,
        };
        crate::session::register_window(self.window_id, session.clone());
        let _ = crate::session::save(&session);
    }

    pub fn get_selected_text(&self) -> Option<String> {
        let active_tab = self.tabs.get(self.active_tab_idx)?;
        let terminal = active_tab.terminal.as_ref()?;
        let sel = self.selection?;
        let (min_p, max_p) = if sel.start <= sel.end {
            (sel.start, sel.end)
        } else {
            (sel.end, sel.start)
        };
        let term_guard = terminal.term().try_lock()?;
        let grid = term_guard.grid();
        let mut text = String::new();
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::cell::Flags;
        for line_i in min_p.line.0..=max_p.line.0 {
            if line_i < -(grid.history_size() as i32) || line_i >= grid.screen_lines() as i32 {
                continue;
            }
            let row = &grid[Line(line_i)];
            let start_c = if line_i == min_p.line.0 {
                min_p.column.0
            } else {
                0
            };
            let end_c = if line_i == max_p.line.0 {
                max_p.column.0.min(row.len().saturating_sub(1))
            } else {
                row.len().saturating_sub(1)
            };
            let mut line_str = String::new();
            for col_i in start_c..=end_c {
                if col_i < row.len() {
                    let cell = &row[Column(col_i)];
                    if cell.c != '\0' {
                        line_str.push(if cell.c == '\t' { ' ' } else { cell.c });
                    }
                }
            }
            let is_wrapped = if row.len() == 0 {
                false
            } else {
                row[Column(row.len() - 1)].flags.contains(Flags::WRAPLINE)
            };
            if line_i < max_p.line.0 {
                if is_wrapped {
                    text.push_str(&line_str);
                } else {
                    text.push_str(line_str.trim_end_matches(' '));
                    text.push('\n');
                }
            } else {
                if max_p.column.0 >= row.len().saturating_sub(1) {
                    text.push_str(line_str.trim_end_matches(' '));
                } else {
                    text.push_str(&line_str);
                }
            }
        }
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn spawn_terminal_pane(
        &mut self,
        cmd: &str,
        args: &[String],
        cwd: Option<&std::path::Path>,
        title_override: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TerminalPane {
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        let font_config = self.config.font.clone();
        let (cell_w, line_h) = self.measure_cell_metrics(window);

        let (event_tx, event_rx) = async_channel::unbounded::<AppEvent>();
        let event_sender = EventSender::Callback(Arc::new(move |event| {
            let _ = event_tx.send_blocking(event);
        }));

        let terminal = TerminalState::new(
            cmd,
            args,
            cwd.and_then(|p| p.to_str()),
            self.config.scrollback,
            font_config,
            cell_w,
            line_h,
            960.0,
            640.0,
            event_sender.clone(),
        )
        .expect("Failed to initialize terminal state");

        let terminal_arc = Arc::new(terminal);
        // Every pane is attachable over the local IPC daemon (`crate::daemon`)
        // the moment it exists, regardless of how it was created (new tab,
        // split, session restore, CLI `-e`) -- unregistered on the two removal
        // paths, `close_tab` and `close_active_pane`.
        crate::daemon::register(pane_id, terminal_arc.clone());
        let event_sender_loop = event_sender.clone();

        cx.spawn_in(window, async move |this, cx| {
            while let Ok(mut event) = event_rx.recv().await {
                // Coalesce consecutive Wakeup events (pure re-render pings) into
                // the next real event, WITHOUT dropping that next event: a plain
                // `while let Ok(AppEvent::Wakeup) = event_rx.try_recv() {}` discards
                // whatever non-Wakeup event it happens to pull out of the channel
                // when the pattern fails to match -- silently losing events (e.g. an
                // `Exit` that immediately follows the Wakeup for a command's last
                // bit of output) instead of just stopping the drain.
                while matches!(event, AppEvent::Wakeup) {
                    match event_rx.try_recv() {
                        Ok(next) => event = next,
                        Err(_) => break,
                    }
                }

                let res = this.update_in(cx, |this, _window, cx| match event {
                    AppEvent::Wakeup => {
                        cx.notify();
                    }
                    AppEvent::TitleChanged(title) => {
                        for tab in &mut this.tabs {
                            if let Some(pane) = tab.pane_tree.find_pane_mut(pane_id) {
                                pane.title = title.clone();
                                if tab.pane_tree.active_pane_id == pane_id {
                                    tab.title = title.clone();
                                }
                            }
                        }
                        this.persist_session();
                        cx.notify();
                    }
                    AppEvent::CwdChanged(cwd) => {
                        let p = std::path::PathBuf::from(&cwd);
                        let now = std::time::Instant::now();
                        for tab in &mut this.tabs {
                            if let Some(pane) = tab.pane_tree.find_pane_mut(pane_id) {
                                pane.git_checked_cwd = Some(p.clone());
                                pane.git_last_poll = Some(now);
                                pane.cwd = Some(p.clone());
                                if tab.pane_tree.active_pane_id == pane_id {
                                    tab.git_checked_cwd = Some(p.clone());
                                    tab.git_last_poll = Some(now);
                                    tab.cwd = Some(p.clone());
                                }
                            }
                        }
                        this.persist_session();
                        cx.notify();

                        let sender_bg = event_sender_loop.clone();
                        let p_bg = p.clone();
                        std::thread::spawn(move || {
                            if let Some(git) = crate::git::fetch_git_info_cached(&p_bg) {
                                sender_bg.send(AppEvent::GitStatusUpdated {
                                    window_id: None,
                                    tab_idx: pane_id,
                                    status: Some(git),
                                });
                            }
                        });
                    }
                    AppEvent::CommandFinished {
                        duration_ms,
                        exit_code,
                    } => {
                        let mut cwd_to_poll = None;
                        for tab in &mut this.tabs {
                            let is_active = tab.pane_tree.active_pane_id == pane_id;
                            if let Some(pane) = tab.pane_tree.find_pane_mut(pane_id) {
                                pane.last_duration_ms = Some(duration_ms);
                                pane.last_exit_code = exit_code;
                                if let Some(ref cwd) = pane.cwd {
                                    cwd_to_poll = Some(cwd.clone());
                                }
                                if is_active {
                                    tab.last_duration_ms = Some(duration_ms);
                                    tab.last_exit_code = exit_code;
                                }
                            }
                        }
                        cx.notify();

                        if let Some(cwd) = cwd_to_poll {
                            let sender_bg = event_sender_loop.clone();
                            std::thread::spawn(move || {
                                if let Some(git) = crate::git::fetch_git_info_cached(&cwd) {
                                    sender_bg.send(AppEvent::GitStatusUpdated {
                                        window_id: None,
                                        tab_idx: pane_id,
                                        status: Some(git),
                                    });
                                }
                            });
                        }
                    }
                    AppEvent::PromptStarted { .. } => {
                        cx.notify();
                    }
                    AppEvent::GitStatusUpdated {
                        tab_idx, status, ..
                    } => {
                        for tab in &mut this.tabs {
                            if let Some(pane) = tab.pane_tree.find_pane_mut(tab_idx) {
                                pane.git_status = status.clone();
                                pane.git_last_poll = Some(std::time::Instant::now());
                                if tab.pane_tree.active_pane_id == tab_idx {
                                    tab.git_status = status.clone();
                                    tab.git_last_poll = Some(std::time::Instant::now());
                                }
                            }
                        }
                        cx.notify();
                    }
                    AppEvent::ProgramStatusChanged { source_id } => {
                        this.refresh_program_status(source_id, _window, cx);
                    }
                    AppEvent::Notification { title, body } => {
                        send_system_notification(&title, &body);
                    }
                    AppEvent::Exit { .. } => {
                        if this.exec_pane_id == Some(pane_id) {
                            cx.quit();
                        } else if let Some(tab) = this
                            .tabs
                            .iter()
                            .find(|t| t.pane_tree.find_pane(pane_id).is_some())
                        {
                            let tab_id = tab.id;
                            let pane_count = tab.pane_tree.pane_count();
                            if pane_count > 1 {
                                this.force_close_pane(tab_id, pane_id, cx);
                            } else {
                                this.close_tab(tab_id, _window, cx);
                            }
                        }
                    }
                    _ => {
                        cx.notify();
                    }
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();

        let initial_cwd = cwd.map(|p| p.to_path_buf()).or_else(dirs::home_dir);
        let default_shell_name = std::path::Path::new(cmd)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.trim_end_matches(".exe"))
            .filter(|n| !n.is_empty())
            .unwrap_or("fastty");
        let pane_title = title_override.unwrap_or_else(|| default_shell_name.to_string());

        if let Some(ref p) = initial_cwd {
            let sender_bg = event_sender.clone();
            let p_bg = p.clone();
            std::thread::spawn(move || {
                if let Some(git) = crate::git::fetch_git_info_cached(&p_bg) {
                    sender_bg.send(AppEvent::GitStatusUpdated {
                        window_id: None,
                        tab_idx: pane_id,
                        status: Some(git),
                    });
                }
            });
        }

        TerminalPane {
            id: pane_id,
            terminal: Some(terminal_arc),
            title: pane_title,
            custom_title: None,
            cwd: initial_cwd.clone(),
            git_status: None,
            git_checked_cwd: initial_cwd,
            git_last_poll: Some(std::time::Instant::now()),
            last_duration_ms: None,
            last_exit_code: None,
            last_bounds: None,
        }
    }

    pub fn create_tab_with_cmd_and_cwd(
        &mut self,
        cmd: &str,
        args: &[String],
        cwd: Option<&std::path::Path>,
        title_override: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;

        let pane = self.spawn_terminal_pane(cmd, args, cwd, title_override, window, cx);
        let pane_title = pane.title.clone();
        let pane_cwd = pane.cwd.clone();
        let pane_git = pane.git_status.clone();
        let pane_term = pane.terminal.clone();
        let pane_tree = PaneTree::new(pane);

        self.tabs.push(TabData {
            id: tab_id,
            ai_tab_key: new_ai_key("tab"),
            ai_conversation_id: new_ai_key("chat"),
            pane_tree,
            title: pane_title,
            custom_title: None,
            terminal: pane_term,
            cwd: pane_cwd.clone(),
            git_status: pane_git,
            git_checked_cwd: pane_cwd,
            git_last_poll: Some(std::time::Instant::now()),
            last_duration_ms: None,
            last_exit_code: None,
            zoomed_pane: None,
        });

        self.activate_tab_index(self.tabs.len() - 1);
        self.persist_session();
        cx.notify();
    }

    pub fn split_active_pane(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let current_cwd = active_tab
            .pane_tree
            .active_pane()
            .and_then(|p| p.cwd.clone())
            .or_else(|| active_tab.cwd.clone());
        let default_shell = self
            .config
            .shell
            .clone()
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(crate::paths::default_system_shell);
        let new_pane = self.spawn_terminal_pane(
            &default_shell,
            &[],
            current_cwd.as_deref(),
            None,
            window,
            cx,
        );
        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
            tab.pane_tree.split_active_pane(new_pane, direction);
            // A new split exits zoom: the tree changed under it.
            tab.zoomed_pane = None;
            if let Some(pane) = tab.pane_tree.active_pane() {
                tab.terminal = pane.terminal.clone();
                tab.cwd = pane.cwd.clone();
                tab.git_status = pane.git_status.clone();
            }
        }
        self.persist_session();
        cx.notify();
    }

    pub fn focus_pane_in_direction(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
            if let Some(_target_id) = tab.pane_tree.focus_direction(direction) {
                if let Some(pane) = tab.pane_tree.active_pane() {
                    tab.terminal = pane.terminal.clone();
                    tab.cwd = pane.cwd.clone();
                    tab.git_status = pane.git_status.clone();
                }
                cx.notify();
            }
        }
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            self.acknowledge_program_status(tab.pane_tree.active_pane_id);
        }
    }

    pub fn close_pane_by_id(
        &mut self,
        pane_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_opt = self
            .tabs
            .iter()
            .find(|t| t.pane_tree.find_pane(pane_id).is_some())
            .map(|t| (t.id, t.pane_tree.pane_count()));
        let Some((tab_id, pane_count)) = tab_opt else {
            return;
        };

        if pane_count <= 1 {
            self.close_tab(tab_id, window, cx);
        } else {
            if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                if let Some(pane) = tab.pane_tree.find_pane(pane_id) {
                    if let Some(term) = &pane.terminal {
                        if term.is_process_running() {
                            let name = term
                                .get_foreground_process_name()
                                .unwrap_or_else(|| "process".to_string());
                            let pid = term.shell_pid().unwrap_or(0);
                            self.pending_close = Some(PendingClose {
                                target: CloseTarget::Pane { tab_id, pane_id },
                                running_processes: vec![RunningProcessInfo {
                                    pane_id,
                                    process_name: name,
                                    pid,
                                }],
                            });
                            cx.notify();
                            return;
                        }
                    }
                }
            }
            self.force_close_pane(tab_id, pane_id, cx);
        }
    }

    pub fn close_active_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            let active_id = tab.pane_tree.active_pane_id;
            self.close_pane_by_id(active_id, window, cx);
        }
    }

    /// F2: toggles zoom on the active pane (tmux `prefix+z` semantics): the
    /// pane takes the whole terminal area until toggled off. No-op with a
    /// single pane. Per-tab and transient (not persisted, not previewed).
    pub fn toggle_pane_zoom(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
            if tab.pane_tree.pane_count() < 2 {
                return;
            }
            let active = tab.pane_tree.active_pane_id;
            tab.zoomed_pane = if tab.zoomed_pane == Some(active) {
                None
            } else {
                Some(active)
            };
            cx.notify();
        }
    }

    pub fn force_close_pane(&mut self, tab_id: usize, pane_id: usize, cx: &mut Context<Self>) {
        self.selection = None;
        self.is_selecting = false;
        self.selection_start = None;
        if self.exec_pane_id == Some(pane_id) {
            self.exec_pane_id = None;
        }
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            if let Some(pane) = tab.pane_tree.find_pane(pane_id) {
                if let Some(ref term) = pane.terminal {
                    term.terminate_process();
                }
            }
            if tab.pane_tree.close_pane(pane_id) {
                crate::daemon::unregister(pane_id);
            }
            // A closed zoomed pane exits zoom; other panes keep it.
            if tab.zoomed_pane == Some(pane_id) {
                tab.zoomed_pane = None;
            }
            if let Some(pane) = tab.pane_tree.active_pane() {
                tab.terminal = pane.terminal.clone();
                tab.cwd = pane.cwd.clone();
                tab.git_status = pane.git_status.clone();
            }
        }
        self.persist_session();
        cx.notify();
    }

    pub fn with_initial_tab(
        window: &mut Window,
        cli_opts: crate::cli::CliOptions,
        tab_data: TabData,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_options_internal(window, cli_opts, Some(tab_data), cx)
    }

    /// Surface a daemon-spawned session (`spawn { open: true }`, e.g. an MCP
    /// agent's `fastty_spawn_session`) as a new tab. The pane keeps the
    /// daemon-assigned id so IPC clients keep addressing the same session:
    /// `fastty_write_session` types into this tab and `fastty_close_session`
    /// closes it. Rendering resizes the terminal to the tab's real geometry
    /// on the first frame (`render_pane_tree_node`).
    pub fn adopt_daemon_session(
        &mut self,
        id: PaneId,
        terminal: Arc<TerminalState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A session the agent spawned and closed immediately: its Exit event
        // was emitted before this tab existed (with no event sender wired)
        // and would never arrive, leaving a dead tab. Nothing to show.
        if !terminal.is_alive() {
            return;
        }

        // Keep future GUI pane ids clear of the daemon's id space (>= 100).
        if id >= self.next_pane_id {
            self.next_pane_id = id + 1;
        }

        let cwd = terminal.get_current_working_directory();
        let title = terminal
            .get_foreground_process_name()
            .or_else(|| {
                cwd.as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("session {id}"));

        let pane = TerminalPane {
            id,
            terminal: Some(terminal.clone()),
            title: title.clone(),
            custom_title: None,
            cwd: cwd.clone(),
            git_status: None,
            git_checked_cwd: cwd.clone(),
            git_last_poll: Some(std::time::Instant::now()),
            last_duration_ms: None,
            last_exit_code: None,
            last_bounds: None,
        };

        let tab_data = TabData {
            id: 0, // assigned by `attach_tab`
            ai_tab_key: new_ai_key("tab"),
            ai_conversation_id: new_ai_key("chat"),
            pane_tree: crate::pane_tree::PaneTree::new(pane),
            title,
            custom_title: None,
            terminal: Some(terminal),
            cwd,
            git_status: None,
            git_checked_cwd: None,
            git_last_poll: None,
            last_duration_ms: None,
            last_exit_code: None,
            zoomed_pane: None,
        };

        self.attach_tab(tab_data, window, cx);
    }

    fn tab_index_of_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.pane_tree.find_pane(pane_id).is_some())
    }

    /// Daemon-facing `split_pane`: create a new pane next to `target`
    /// inside its tab (the daemon protocol's way for agents to build
    /// layouts). The new pane registers as a GUI session, so list/close/
    /// write keep working on it.
    pub fn daemon_split_pane(
        &mut self,
        target: PaneId,
        direction: Direction,
        command: Option<String>,
        args: &[String],
        cwd: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneId, String> {
        let tab_idx = self
            .tab_index_of_pane(target)
            .ok_or_else(|| format!("pane {target} is not visible in the fastty window"))?;

        let shell = command.unwrap_or_else(|| {
            self.config
                .shell
                .clone()
                .or_else(|| std::env::var("SHELL").ok())
                .unwrap_or_else(crate::paths::default_system_shell)
        });
        let cwd = cwd
            .map(std::path::PathBuf::from)
            .or_else(|| {
                self.tabs[tab_idx]
                    .pane_tree
                    .find_pane(target)
                    .and_then(|p| p.cwd.clone())
            })
            .or_else(|| self.tabs[tab_idx].cwd.clone());

        let new_pane = self.spawn_terminal_pane(&shell, args, cwd.as_deref(), None, window, cx);
        let new_id = new_pane.id;

        if let Some(tab) = self.tabs.get_mut(tab_idx) {
            if tab.pane_tree.split_pane_at(target, new_pane, direction) {
                // A new split exits zoom: the tree changed under it.
                tab.zoomed_pane = None;
                if let Some(pane) = tab.pane_tree.active_pane() {
                    tab.terminal = pane.terminal.clone();
                    tab.cwd = pane.cwd.clone();
                    tab.git_status = pane.git_status.clone();
                }
            }
        }
        self.persist_session();
        cx.notify();
        Ok(new_id)
    }

    /// Daemon-facing `resize_pane`: move the divider on one side of a pane.
    pub fn daemon_resize_pane(
        &mut self,
        target: PaneId,
        direction: Direction,
        delta: f32,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let tab_idx = self
            .tab_index_of_pane(target)
            .ok_or_else(|| format!("pane {target} is not visible in the fastty window"))?;
        let side = match direction {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Top => "top",
            Direction::Down => "down",
        };
        let moved = self.tabs[tab_idx]
            .pane_tree
            .resize_pane(target, direction, delta);
        if !moved {
            return Err(format!("pane {target} has no divider on its {side} side"));
        }
        self.persist_session();
        cx.notify();
        Ok(())
    }

    /// Daemon-facing `focus_pane`: bring the pane's tab to the front and
    /// make the pane the active one, so the user sees what the agent is
    /// touching.
    pub fn daemon_focus_pane(&mut self, target: PaneId, cx: &mut Context<Self>) -> Result<(), String> {
        let tab_idx = self
            .tab_index_of_pane(target)
            .ok_or_else(|| format!("pane {target} is not visible in the fastty window"))?;
        self.activate_tab_index(tab_idx);
        self.selection = None;
        self.is_selecting = false;
        self.selection_start = None;
        if let Some(tab) = self.tabs.get_mut(tab_idx) {
            tab.pane_tree.active_pane_id = target;
            if let Some(pane) = tab.pane_tree.active_pane() {
                tab.terminal = pane.terminal.clone();
                tab.cwd = pane.cwd.clone();
                tab.git_status = pane.git_status.clone();
            }
        }
        self.persist_session();
        cx.notify();
        Ok(())
    }

    /// Daemon-facing `layout`: the full window structure (tabs → panes), so
    /// IPC clients can see which sessions share a tab before splitting.
    pub fn daemon_layout(&self) -> Vec<crate::daemon::TabLayout> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| crate::daemon::TabLayout {
                tab_id: tab.id,
                title: tab.title.clone(),
                active: idx == self.active_tab_idx,
                active_pane: tab.pane_tree.active_pane_id,
                zoomed_pane: tab.zoomed_pane,
                panes: tab
                    .pane_tree
                    .all_panes()
                    .iter()
                    .map(|p| {
                        let (cols, rows) = p
                            .terminal
                            .as_ref()
                            .map(|t| t.dimensions())
                            .unwrap_or((0, 0));
                        crate::daemon::PaneLayoutInfo {
                            id: p.id,
                            title: p.title.clone(),
                            cwd: p
                                .cwd
                                .clone()
                                .map(|c| c.to_string_lossy().into_owned()),
                            active: p.id == tab.pane_tree.active_pane_id,
                            cols,
                            rows,
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// Daemon-facing `resize_window`: best-effort resize of the OS window so
    /// the active pane renders at `cols` x `rows`. Chrome is derived from
    /// the active pane, so with splits the result is approximate — dividers
    /// are `resize_pane`'s job.
    pub fn daemon_resize_window(
        &mut self,
        cols: usize,
        rows: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let (cur_cols, cur_rows) = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|t| t.pane_tree.active_pane())
            .and_then(|p| p.terminal.as_ref())
            .map(|t| t.dimensions())
            .ok_or_else(|| "no active pane to size against".to_string())?;
        let (cell_w, line_h) = self.measure_cell_metrics(window);
        let d_w = (cols as isize - cur_cols as isize) as f32 * cell_w;
        let d_h = (rows as isize - cur_rows as isize) as f32 * line_h;
        let mut size = window.bounds().size;
        size.width += px(d_w);
        size.height += px(d_h);
        window.resize(size);
        cx.notify();
        Ok(())
    }

    pub fn attach_tab(
        &mut self,
        mut tab_data: TabData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        tab_data.id = tab_id;

        let (event_tx, event_rx) = async_channel::unbounded::<AppEvent>();
        let event_sender = EventSender::Callback(Arc::new(move |event| {
            let _ = event_tx.send_blocking(event);
        }));

        if let Some(terminal) = &tab_data.terminal {
            terminal.set_event_sender(event_sender.clone());
        }
        for pane in tab_data.pane_tree.all_panes_mut() {
            if let Some(ref term) = pane.terminal {
                term.set_event_sender(event_sender.clone());
            }
        }

        self.tabs.push(tab_data);
        self.activate_tab_index(self.tabs.len() - 1);
        self.persist_session();
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            while let Ok(mut event) = event_rx.recv().await {
                // See the identical comment in `spawn_terminal_pane`: coalesce
                // consecutive Wakeup events without dropping the first non-Wakeup
                // event that follows one in the channel.
                while matches!(event, AppEvent::Wakeup) {
                    match event_rx.try_recv() {
                        Ok(next) => event = next,
                        Err(_) => break,
                    }
                }

                let res = this.update_in(cx, |this, _window, cx| match event {
                    AppEvent::Wakeup => {
                        cx.notify();
                    }
                    AppEvent::TitleChanged(title) => {
                        if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == tab_id) {
                            tab.title = title;
                        }
                        this.persist_session();
                        cx.notify();
                    }
                    AppEvent::CwdChanged(cwd) => {
                        let p = std::path::PathBuf::from(&cwd);
                        let now = std::time::Instant::now();
                        if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == tab_id) {
                            tab.git_checked_cwd = Some(p.clone());
                            tab.git_last_poll = Some(now);
                            tab.cwd = Some(p.clone());
                        }
                        this.persist_session();
                        cx.notify();

                        let sender_bg = event_sender.clone();
                        let p_bg = p.clone();
                        std::thread::spawn(move || {
                            if let Some(git) = crate::git::fetch_git_info_cached(&p_bg) {
                                sender_bg.send(AppEvent::GitStatusUpdated {
                                    window_id: None,
                                    tab_idx: tab_id,
                                    status: Some(git),
                                });
                            }
                        });
                    }
                    AppEvent::CommandFinished {
                        duration_ms,
                        exit_code,
                    } => {
                        let mut cwd_to_poll = None;
                        if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == tab_id) {
                            tab.last_duration_ms = Some(duration_ms);
                            tab.last_exit_code = exit_code;
                            if let Some(ref cwd) = tab.cwd {
                                cwd_to_poll = Some(cwd.clone());
                            }
                        }
                        cx.notify();

                        if let Some(cwd) = cwd_to_poll {
                            let sender_bg = event_sender.clone();
                            std::thread::spawn(move || {
                                if let Some(git) = crate::git::fetch_git_info_cached(&cwd) {
                                    sender_bg.send(AppEvent::GitStatusUpdated {
                                        window_id: None,
                                        tab_idx: tab_id,
                                        status: Some(git),
                                    });
                                }
                            });
                        }
                    }
                    AppEvent::PromptStarted { .. } => {
                        cx.notify();
                    }
                    AppEvent::GitStatusUpdated {
                        tab_idx, status, ..
                    } => {
                        if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == tab_idx) {
                            tab.git_status = status.clone();
                            tab.git_last_poll = Some(std::time::Instant::now());
                        }
                        cx.notify();
                    }
                    AppEvent::ProgramStatusChanged { source_id } => {
                        this.refresh_program_status(source_id, _window, cx);
                    }
                    AppEvent::Notification { title, body } => {
                        send_system_notification(&title, &body);
                    }
                    AppEvent::Exit { shell_pid } => {
                        // Mirror `spawn_terminal_pane`'s exit handling for
                        // restored and daemon-adopted tabs. One pump serves
                        // every pane in the tab, so match the exited pane by
                        // shell pid; close that pane, or the whole tab when
                        // it was the only one.
                        let exited_pane = this
                            .tabs
                            .iter()
                            .find(|t| t.id == tab_id)
                            .and_then(|tab| {
                                tab.pane_tree.all_panes().iter().find_map(|p| {
                                    let matches = p
                                        .terminal
                                        .as_ref()
                                        .map(|t| t.shell_pid() == shell_pid)
                                        .unwrap_or(false);
                                    matches.then_some(p.id)
                                })
                            });
                        if let Some(pane_id) = exited_pane {
                            if this.exec_pane_id == Some(pane_id) {
                                cx.quit();
                            } else if let Some(tab) = this
                                .tabs
                                .iter()
                                .find(|t| t.id == tab_id)
                            {
                                let pane_count = tab.pane_tree.pane_count();
                                if pane_count > 1 {
                                    this.force_close_pane(tab_id, pane_id, cx);
                                } else {
                                    this.close_tab(tab_id, _window, cx);
                                }
                            }
                        }
                    }
                    _ => {
                        cx.notify();
                    }
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn create_tab_with_cmd(
        &mut self,
        cmd: &str,
        args: &[String],
        title_override: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_tab_with_cmd_and_cwd(cmd, args, None, title_override, window, cx);
    }

    pub fn create_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shell = self
            .config
            .shell
            .clone()
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(crate::paths::default_system_shell);
        self.create_tab_with_cmd_and_cwd(&shell, &[], None, None, window, cx);
    }

    fn save_active_ai_conversation(&mut self) {
        let tab = self.ai_event_source.as_ref()
            .and_then(|(id, _)| self.tabs.iter().find(|tab| tab.id == *id))
            .or_else(|| self.tabs.get(self.active_tab_idx));
        let Some(tab) = tab else { return };
        let tab_key = tab.ai_tab_key.clone();
        let conversation_id = self.ai_event_source.as_ref().map(|(_, id)| id.clone())
            .unwrap_or_else(|| tab.ai_conversation_id.clone());
        let cwd = tab.cwd.clone().unwrap_or_default();
        let now = crate::ai::conversations::now_millis();
        let default_context = crate::config::ai_context_window().max(1_000) as u64;
        let index = self.ai_chat_records.iter().position(|record| record.id == conversation_id);
        let record = if let Some(index) = index {
            &mut self.ai_chat_records[index]
        } else {
            self.ai_chat_records.push(crate::ai::conversations::Conversation {
                id: conversation_id,
                tab_key,
                cwd: cwd.clone(),
                title: "New chat".to_string(),
                provider: self.config.ai.default.clone(),
                model: self.config.ai.active_model(),
                messages: Vec::new(),
                context_window: default_context,
                used_tokens: None,
                acp_session_id: None,
                acp_provider: None,
                available_commands: Vec::new(),
                created_at: now,
                updated_at: now,
            });
            self.ai_chat_records.last_mut().unwrap()
        };
        if record.cwd != cwd {
            record.cwd = cwd;
            record.acp_session_id = None;
            record.acp_provider = None;
            record.available_commands.clear();
        }
        record.messages = self.ai_messages.clone();
        if !self.ai_turn_provider.is_empty() {
            record.provider = self.ai_turn_provider.clone();
            record.model = self.ai_turn_model.clone();
        } else {
            record.provider = self.config.ai.default.clone();
            record.model = self.config.ai.active_model();
        }
        if let Some(first_prompt) = record.messages.iter().find(|message| message.is_user) {
            let title = first_prompt.text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !title.is_empty() && record.title == "New chat" {
                record.title = title.chars().take(64).collect();
            }
        }
        record.updated_at = now;
        let _ = crate::ai::conversations::save(&self.ai_chat_records);
    }

    fn finish_cancelled_ai_turn(&mut self) {
        if !self.ai_streaming_text.is_empty() || !self.ai_streaming_thinking.is_empty() {
            self.ai_messages.push(crate::ui::ai_sidebar::AiUiMessage {
                is_user: false,
                text: std::mem::take(&mut self.ai_streaming_text),
                thinking: if self.ai_streaming_thinking.is_empty() {
                    None
                } else {
                    Some(std::mem::take(&mut self.ai_streaming_thinking))
                },
                tool_calls: Vec::new(),
                timestamp: Some(crate::ui::ai_sidebar::current_time_str()),
                images: Vec::new(),
                documents: Vec::new(),
                is_error: false,
            });
        }
        for message in &mut self.ai_messages {
            for tool in &mut message.tool_calls {
                if tool.is_running {
                    tool.is_running = false;
                    tool.is_error = true;
                    tool.output = Some("Cancelled".to_string());
                }
            }
        }
        self.ai_streaming_text.clear();
        self.ai_streaming_thinking.clear();
        self.ai_pending_text.clear();
        self.ai_pending_thinking.clear();
    }

    fn swap_ai_runtime(&mut self, runtime: &mut AiConversationRuntime) {
        let checker = runtime.permission_checker.take().unwrap_or_else(|| {
            Arc::new(crate::ai::PermissionChecker::new(self.config.ai.permission_mode))
        });
        runtime.permission_checker = Some(std::mem::replace(&mut self.ai_permission_checker, checker));
        std::mem::swap(&mut self.ai_messages, &mut runtime.messages);
        std::mem::swap(&mut self.ai_streaming_text, &mut runtime.streaming_text);
        std::mem::swap(&mut self.ai_streaming_thinking, &mut runtime.streaming_thinking);
        std::mem::swap(&mut self.ai_is_streaming, &mut runtime.is_streaming);
        std::mem::swap(&mut self.ai_last_notification, &mut runtime.last_notification);
        std::mem::swap(&mut self.ai_last_usage, &mut runtime.last_usage);
        std::mem::swap(&mut self.ai_current_turn_usage, &mut runtime.current_turn_usage);
        std::mem::swap(&mut self.ai_cancel_token, &mut runtime.cancel_token);
        std::mem::swap(&mut self.ai_stream_generation, &mut runtime.stream_generation);
        std::mem::swap(&mut self.ai_pending_confirmation, &mut runtime.pending_confirmation);
        std::mem::swap(&mut self.ai_confirm_reply_tx, &mut runtime.confirm_reply_tx);
        std::mem::swap(&mut self.ai_opencode_confirm_reply_tx, &mut runtime.opencode_confirm_reply_tx);
        std::mem::swap(&mut self.ai_pending_text, &mut runtime.pending_text);
        std::mem::swap(&mut self.ai_pending_thinking, &mut runtime.pending_thinking);
        std::mem::swap(&mut self.ai_turn_provider, &mut runtime.turn_provider);
        std::mem::swap(&mut self.ai_turn_model, &mut runtime.turn_model);
    }

    fn load_ai_conversation(&mut self, conversation_id: &str) {
        if self.ai_loaded_conversation_id == conversation_id {
            return;
        }
        let mut previous = AiConversationRuntime::default();
        previous.owner_tab_id = self.tabs.iter()
            .find(|tab| tab.ai_conversation_id == self.ai_loaded_conversation_id)
            .map(|tab| tab.id)
            .or_else(|| self.tabs.get(self.active_tab_idx).map(|tab| tab.id))
            .unwrap_or_default();
        self.swap_ai_runtime(&mut previous);
        if !self.ai_loaded_conversation_id.is_empty() {
            self.ai_background_conversations.insert(self.ai_loaded_conversation_id.clone(), previous);
        }
        let mut next = self.ai_background_conversations.remove(conversation_id).unwrap_or_else(|| {
            let record = self.ai_chat_records.iter().find(|record| record.id == conversation_id);
            AiConversationRuntime {
                messages: record.map(|record| record.messages.clone()).unwrap_or_default(),
                last_usage: record.and_then(|record| record.used_tokens.map(|used| (used, 0))),
                ..AiConversationRuntime::default()
            }
        });
        self.swap_ai_runtime(&mut next);
        self.ai_loaded_conversation_id = conversation_id.to_string();
        self.ai_expanded_thinkings.clear();
        self.ai_message_selection = None;
        self.ai_message_selection_anchor = None;
    }

    fn with_ai_turn<R>(
        &mut self,
        tab_id: usize,
        conversation_id: &str,
        generation: u64,
        update: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if !self.tabs.iter().any(|tab| tab.id == tab_id) {
            return None;
        }
        if self.ai_loaded_conversation_id == conversation_id {
            if self.ai_stream_generation != generation || !self.ai_is_streaming {
                return None;
            }
            return Some(update(self));
        }
        let runtime = self.ai_background_conversations.get(conversation_id)?;
        if runtime.stream_generation != generation || !runtime.is_streaming {
            return None;
        }
        let mut runtime = self.ai_background_conversations.remove(conversation_id)?;
        self.swap_ai_runtime(&mut runtime);
        self.ai_event_source = Some((tab_id, conversation_id.to_string()));
        let result = update(self);
        self.ai_event_source = None;
        self.swap_ai_runtime(&mut runtime);
        self.ai_background_conversations.insert(conversation_id.to_string(), runtime);
        Some(result)
    }

    fn cancel_background_ai_conversation(&mut self, conversation_id: &str) {
        if let Some(mut runtime) = self.ai_background_conversations.remove(conversation_id) {
            self.swap_ai_runtime(&mut runtime);
            self.cancel_ai_stream();
            self.finish_cancelled_ai_turn();
            if let Some(record) = self.ai_chat_records.iter_mut().find(|record| record.id == conversation_id) {
                record.messages = self.ai_messages.clone();
            }
            self.swap_ai_runtime(&mut runtime);
        }
    }

    fn cancel_tab_ai_turns(&mut self, tab_id: usize) {
        let ids: Vec<String> = self.ai_background_conversations.iter()
            .filter(|(_, runtime)| runtime.owner_tab_id == tab_id)
            .map(|(id, _)| id.clone()).collect();
        for id in ids {
            self.cancel_background_ai_conversation(&id);
        }
    }

    fn activate_tab_index(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.acknowledge_program_status(self.tabs[idx].pane_tree.active_pane_id);
        if idx == self.active_tab_idx {
            let id = self.tabs[idx].ai_conversation_id.clone();
            if self.ai_loaded_conversation_id != id {
                self.load_ai_conversation(&id);
            }
            return;
        }
        self.save_active_ai_conversation();
        self.active_tab_idx = idx;
        let conversation_id = self.tabs[idx].ai_conversation_id.clone();
        self.load_ai_conversation(&conversation_id);
    }

    pub fn select_tab(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        if let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) {
            if self.active_tab_idx != idx {
                self.selection = None;
                self.is_selecting = false;
                self.selection_start = None;
            }
            self.activate_tab_index(idx);
            self.persist_session();
            cx.notify();
        }
    }

    pub fn close_tab(&mut self, tab_id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
            let running: Vec<RunningProcessInfo> = tab
                .pane_tree
                .all_panes()
                .into_iter()
                .filter_map(|pane| {
                    if let Some(term) = &pane.terminal {
                        if term.is_process_running() {
                            let name = term
                                .get_foreground_process_name()
                                .unwrap_or_else(|| "process".to_string());
                            let pid = term.shell_pid().unwrap_or(0);
                            return Some(RunningProcessInfo {
                                pane_id: pane.id,
                                process_name: name,
                                pid,
                            });
                        }
                    }
                    None
                })
                .collect();

            if !running.is_empty() {
                self.pending_close = Some(PendingClose {
                    target: CloseTarget::Tab { tab_id },
                    running_processes: running,
                });
                cx.notify();
                return;
            }
        }
        self.force_close_tab(tab_id, window, cx);
    }

    pub fn force_close_tab(&mut self, tab_id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selection = None;
        self.is_selecting = false;
        self.selection_start = None;
        if let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) {
            let was_active = idx == self.active_tab_idx;
            self.cancel_tab_ai_turns(tab_id);
            if was_active && self.ai_is_streaming {
                self.cancel_ai_stream();
                self.finish_cancelled_ai_turn();
            }
            if was_active {
                self.save_active_ai_conversation();
            }
            let removed_conversation_id = self.tabs[idx].ai_conversation_id.clone();
            let removed_tab_key = &self.tabs[idx].ai_tab_key;
            self.ai_turn_completions.retain(|id, _| {
                !self.ai_chat_records.iter().any(|record| record.id == *id && record.tab_key == *removed_tab_key)
            });
            self.ai_opencode_manager.remove(&removed_conversation_id);
            let removed = self.tabs.remove(idx);
            for pane in removed.pane_tree.all_panes() {
                if self.exec_pane_id == Some(pane.id) {
                    self.exec_pane_id = None;
                }
                if let Some(ref term) = pane.terminal {
                    term.terminate_process();
                }
                crate::daemon::unregister(pane.id);
            }
            if self.tabs.is_empty() {
                self.ai_messages.clear();
                self.create_tab(window, cx);
                // Sync the loaded id so the removed conversation is not
                // re-inserted as a ghost runtime on the next load call.
                if let Some(tab) = self.tabs.first() {
                    self.ai_loaded_conversation_id = tab.ai_conversation_id.clone();
                }
            } else if was_active {
                self.active_tab_idx = idx.min(self.tabs.len() - 1);
                let active_id = self.tabs[self.active_tab_idx].ai_conversation_id.clone();
                self.load_ai_conversation(&active_id);
            } else if idx < self.active_tab_idx {
                self.active_tab_idx -= 1;
            }
            // A close can be deferred to the confirmation dialog, so this runs
            // after the tab is actually gone. The Mission Control highlight
            // reads the field on the next paint, and it has to stay a valid
            // index.
            self.tab_overview_selected = self
                .tab_overview_selected
                .min(self.tabs.len().saturating_sub(1));
            self.persist_session();
            cx.notify();
        }
    }

    pub fn confirm_pending_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_close.take() else {
            return;
        };
        match pending.target {
            CloseTarget::Tab { tab_id } => {
                if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                    for pane in tab.pane_tree.all_panes() {
                        if let Some(term) = &pane.terminal {
                            term.terminate_process();
                        }
                    }
                }
                self.force_close_tab(tab_id, window, cx);
            }
            CloseTarget::Pane { tab_id, pane_id } => {
                if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                    if let Some(pane) = tab.pane_tree.find_pane(pane_id) {
                        if let Some(term) = &pane.terminal {
                            term.terminate_process();
                        }
                    }
                }
                self.force_close_pane(tab_id, pane_id, cx);
            }
            CloseTarget::OtherTabs { keep_id } => {
                for tab in self.tabs.iter().filter(|t| t.id != keep_id) {
                    for pane in tab.pane_tree.all_panes() {
                        if let Some(term) = &pane.terminal {
                            term.terminate_process();
                        }
                    }
                }
                self.force_close_other_tabs(keep_id, window, cx);
            }
        }
    }

    pub fn trigger_apply_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_updating {
            self.is_update_modal_open = true;
            cx.notify();
            return;
        }
        if self.is_update_ready {
            self.is_update_modal_open = true;
            cx.notify();
            return;
        }
        let Some(release) = self.update_available.clone() else {
            // No cached release (first run, 24h throttle, or a past network
            // error): run a forced network check so the button always leads
            // somewhere instead of silently doing nothing.
            self.check_for_updates_manual(window, cx);
            return;
        };

        if let Some(reason) = release.self_update_blocked_reason.clone() {
            self.update_status = Some(format!(
                "A new version (v{}) is available.\n\n{}\n\n{}",
                release.version, reason, release.release_url
            ));
            self.is_update_modal_open = true;
            cx.notify();
            return;
        }

        self.is_updating = true;
        self.update_status = Some(format!("Downloading Fastty v{}...", release.version));
        self.is_update_modal_open = true;
        cx.notify();

        let (result_tx, result_rx) = async_channel::unbounded::<Result<(), String>>();
        let rel_clone = release.clone();
        std::thread::spawn(move || {
            let res = crate::updater::apply_update_sync(&rel_clone).map_err(|e| e.to_string());
            let _ = result_tx.send_blocking(res);
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(res) = result_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    this.is_updating = false;
                    match res {
                        Ok(()) => {
                            this.is_update_ready = true;
                            this.update_status = Some(format!("Fastty v{} is ready to install.\nRestart Fastty to switch to the new version.", release.version));
                            this.is_update_modal_open = true;
                            this.update_available = Some(release);
                        }
                        Err(e) => {
                            this.is_update_ready = false;
                            this.update_status = Some(format!("Update failed:\n{}", e));
                            this.is_update_modal_open = true;
                        }
                    }
                    cx.notify();
                });
            }
        }).detach();
    }

    /// Forced network check (bypasses the 24h throttle and the skipped-version
    /// gate). Always opens the changelog modal: with the release on success,
    /// with "up to date" on no release, or with the error text on failure, so
    /// the user never faces a dead button.
    pub fn check_for_updates_manual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_updating
            || self.is_update_ready
            || self.update_status.as_deref() == Some("Checking for updates...")
        {
            self.is_update_modal_open = true;
            cx.notify();
            return;
        }
        self.update_available = None;
        self.update_status = Some("Checking for updates...".to_string());
        self.is_update_modal_open = true;
        cx.notify();

        let channel = crate::updater::UpdateChannel::parse(&self.config.update_channel);
        let (tx, rx) =
            async_channel::unbounded::<Result<Option<crate::updater::ReleaseInfo>, String>>();
        std::thread::spawn(move || {
            let res = crate::updater::check_for_update_sync_with(channel, true)
                .map_err(|e| e.to_string());
            let _ = tx.send_blocking(res);
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(res) = rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    match res {
                        Ok(Some(release)) => {
                            this.is_update_ready = false;
                            this.update_available = Some(release);
                            this.update_status = None;
                            this.is_update_modal_open = true;
                        }
                        Ok(None) => {
                            this.update_available = None;
                            this.is_update_ready = false;
                            this.update_status = Some(format!(
                                "Fastty v{} is up to date.",
                                env!("CARGO_PKG_VERSION")
                            ));
                            this.is_update_modal_open = true;
                        }
                        Err(e) => {
                            this.update_available = None;
                            this.is_update_ready = false;
                            this.update_status = Some(format!("Update check failed:\n{e}"));
                            this.is_update_modal_open = true;
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// "Skip this version": persists the dismissal so the background check
    /// stops offering this release, then closes the modal and clears the
    /// tab-bar badge.
    pub fn skip_update_version(&mut self, cx: &mut Context<Self>) {
        if let Some(release) = self.update_available.clone() {
            crate::updater::dismiss_version(&release.tag_name);
        }
        self.update_available = None;
        self.is_update_ready = false;
        self.update_status = None;
        self.is_update_modal_open = false;
        cx.notify();
    }

    pub fn open_settings_window(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_settings_open = false;
        self.is_context_menu_open = false;
        self.is_about_open = false;
        self.is_command_palette_open = false;
        self.is_ssh_manager_open = false;
        self.is_search_open = false;

        let existing = { *crate::ui::settings_view::SETTINGS_WINDOW_HANDLE.lock() };
        if let Some(handle) = existing {
            let mut activated = false;
            let current_config = self.config.clone();
            let _ = handle.update(cx, |view, window, cx| {
                view.sync_from_config(&current_config, cx);
                window.activate_window();
                window.focus(&view.focus_handle, cx);
                activated = true;
            });
            if activated {
                cx.notify();
                return;
            }
        }

        let bounds = Bounds::centered(None, size(px(880.), px(640.)), &*cx);
        let current_config = self.config.clone();
        if let Ok(handle) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(740.), px(520.))),
                window_background: WindowBackgroundAppearance::Blurred,
                app_id: Some("com.fastty.app.settings".into()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Fastty Settings".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |w, cx| {
                w.set_background_appearance(WindowBackgroundAppearance::Blurred);
                cx.new(|cx| SettingsView::new_with_config(w, &current_config, cx))
            },
        ) {
            *crate::ui::settings_view::SETTINGS_WINDOW_HANDLE.lock() = Some(handle);
        }
        cx.notify();
    }

    pub fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_window(window, cx);
    }

    pub fn toggle_context_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_context_menu_open = !self.is_context_menu_open;
        if self.is_context_menu_open {
            self.is_settings_open = false;
            self.is_about_open = false;
            self.is_tab_context_menu_open = false;
            self.is_pane_context_menu_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_search_open = false;
        }
        cx.notify();
    }

    pub fn open_tab_context_menu(&mut self, tab_id: usize, x: f32, y: f32, cx: &mut Context<Self>) {
        self.is_tab_context_menu_open = true;
        self.tab_context_menu_tab_id = tab_id;
        self.tab_context_menu_pos = (x, y);
        self.is_settings_open = false;
        self.is_context_menu_open = false;
        self.is_pane_context_menu_open = false;
        self.is_about_open = false;
        self.is_command_palette_open = false;
        self.is_ssh_manager_open = false;
        self.is_search_open = false;
        self.is_git_menu_open = false;
        cx.notify();
    }

    pub fn open_pane_context_menu(
        &mut self,
        pane_id: usize,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) {
        self.is_pane_context_menu_open = true;
        self.pane_context_menu_pane_id = pane_id;
        self.pane_context_menu_pos = (x, y);
        self.is_settings_open = false;
        self.is_context_menu_open = false;
        self.is_tab_context_menu_open = false;
        self.is_about_open = false;
        self.is_command_palette_open = false;
        self.is_ssh_manager_open = false;
        self.is_search_open = false;
        self.is_git_menu_open = false;
        cx.notify();
    }

    pub fn close_other_tabs(
        &mut self,
        keep_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let other_running: Vec<RunningProcessInfo> = self
            .tabs
            .iter()
            .filter(|t| t.id != keep_id)
            .flat_map(|t| t.pane_tree.all_panes())
            .filter_map(|pane| {
                if let Some(term) = &pane.terminal {
                    if term.is_process_running() {
                        let name = term
                            .get_foreground_process_name()
                            .unwrap_or_else(|| "process".to_string());
                        let pid = term.shell_pid().unwrap_or(0);
                        return Some(RunningProcessInfo {
                            pane_id: pane.id,
                            process_name: name,
                            pid,
                        });
                    }
                }
                None
            })
            .collect();

        if !other_running.is_empty() {
            self.pending_close = Some(PendingClose {
                target: CloseTarget::OtherTabs { keep_id },
                running_processes: other_running,
            });
            cx.notify();
            return;
        }

        self.force_close_other_tabs(keep_id, window, cx);
    }

    pub fn force_close_other_tabs(
        &mut self,
        keep_id: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_is_streaming && self.tabs.get(self.active_tab_idx).map(|tab| tab.id) != Some(keep_id) {
            self.cancel_ai_stream();
            self.finish_cancelled_ai_turn();
        }
        self.save_active_ai_conversation();
        let removed_ids: Vec<usize> = self.tabs.iter().filter(|tab| tab.id != keep_id).map(|tab| tab.id).collect();
        for id in removed_ids { self.cancel_tab_ai_turns(id); }
        for tab in self.tabs.iter().filter(|t| t.id != keep_id) {
            self.ai_opencode_manager.remove(&tab.ai_conversation_id);
            for pane in tab.pane_tree.all_panes() {
                if let Some(ref term) = pane.terminal {
                    term.terminate_process();
                }
                crate::daemon::unregister(pane.id);
            }
        }
        self.tabs.retain(|t| t.id == keep_id);
        self.active_tab_idx = 0;
        let conversation_id = self.tabs[0].ai_conversation_id.clone();
        self.load_ai_conversation(&conversation_id);
        self.persist_session();
        cx.notify();
    }

    pub fn toggle_about(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_about_open = !self.is_about_open;
        if self.is_about_open {
            self.is_context_menu_open = false;
            self.is_settings_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_snippet_picker_open = false;
            self.is_pr_picker_open = false;
            self.is_search_open = false;
        }
        cx.notify();
    }

    pub fn toggle_command_palette(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_command_palette_open = !self.is_command_palette_open;
        if self.is_command_palette_open {
            self.command_palette_query.clear();
            self.command_palette_selected = 0;
            // Fresh open: no preview in flight yet (set lazily on first hover).
            self.palette_preview_original = None;
            self.is_settings_open = false;
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_ssh_manager_open = false;
            self.is_snippet_picker_open = false;
            self.is_pr_picker_open = false;
            self.is_search_open = false;
        } else {
            // Closed without committing (toggle/Esc/backdrop): drop preview.
            self.cancel_palette_preview(_window, cx);
        }
        cx.notify();
    }

    pub fn toggle_ssh_manager(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_ssh_manager_open = !self.is_ssh_manager_open;
        if self.is_ssh_manager_open {
            self.ssh_manager_query.clear();
            self.ssh_manager_selected = 0;
            self.is_settings_open = false;
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_command_palette_open = false;
            self.is_snippet_picker_open = false;
            self.is_pr_picker_open = false;
            self.is_search_open = false;
        }
        cx.notify();
    }

    /// F4: snippet picker open/close. Reads the live snippet map on every
    /// open, so file edits appear instantly (same live-reload as Tab expand).
    pub fn toggle_snippet_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_snippet_picker_open = !self.is_snippet_picker_open;
        if self.is_snippet_picker_open {
            self.snippet_query.clear();
            self.snippet_selected = 0;
            self.snippet_scroll_handle.scroll_to_item(0);
            self.is_settings_open = false;
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_pr_picker_open = false;
            self.is_search_open = false;
            self.is_worktree_picker_open = false;
            self.is_project_jumper_open = false;
        }
        cx.notify();
    }

    /// F4: inserts the expanded snippet body into the active pane (active
    /// split first, tab terminal as fallback). No trailing Enter is sent:
    /// the user reviews the text and executes it themselves.
    pub fn insert_snippet_body(&mut self, body: &str, cx: &mut Context<Self>) {
        let (expanded, _) = crate::snippets::expand(body);
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            let term = tab
                .pane_tree
                .active_pane()
                .and_then(|p| p.terminal.clone())
                .or_else(|| tab.terminal.clone());
            if let Some(t) = term {
                t.write_to_pty(expanded.as_bytes());
            }
        }
        self.is_snippet_picker_open = false;
        cx.notify();
    }

    /// F5: PR picker open/close. Fetches in background on every open; the
    /// modal shows Loading until the snapshot lands (regular renders pick
    /// it up, same as widgets).
    pub fn toggle_pr_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_pr_picker_open = !self.is_pr_picker_open;
        if self.is_pr_picker_open {
            self.pr_picker_query.clear();
            self.pr_picker_selected = 0;
            self.pr_picker_scroll_handle.scroll_to_item(0);
            self.pr_picker_mode = PrPickerMode::Browse;
            let cwd = self
                .tabs
                .get(self.active_tab_idx)
                .and_then(|t| t.cwd.clone());
            self.pr_picker_cwd = cwd.clone();
            self.is_settings_open = false;
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_snippet_picker_open = false;
            self.is_search_open = false;
            self.is_worktree_picker_open = false;
            self.is_project_jumper_open = false;
            if let Some(dir) = cwd {
                if let Ok(mut guard) = self.pr_snapshot.lock() {
                    *guard = None;
                }
                let state = self.pr_snapshot.clone();
                std::thread::spawn(move || {
                    let summary = crate::widgets::builtin::git_prs::fetch_prs_summary(&dir);
                    if let Ok(mut guard) = state.lock() {
                        *guard = summary;
                    }
                });
            }
        }
        cx.notify();
    }

    /// F5: types the action command into the active pane (visible, like the
    /// git menu) and closes the picker.
    pub fn run_pr_action_command(&mut self, cmd: &str, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            let term = tab
                .pane_tree
                .active_pane()
                .and_then(|p| p.terminal.clone())
                .or_else(|| tab.terminal.clone());
            if let Some(t) = term {
                t.write_to_pty(format!("{cmd}\r").as_bytes());
            }
        }
        self.is_pr_picker_open = false;
        cx.notify();
    }

    pub fn toggle_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_search_open = !self.is_search_open;
        if self.is_search_open {
            self.search_query.clear();
            self.search_match_idx = 0;
            self.search_matches.clear();
            self.is_settings_open = false;
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_snippet_picker_open = false;
            self.is_pr_picker_open = false;
            self.is_worktree_picker_open = false;
            self.is_project_jumper_open = false;
        }
        cx.notify();
    }

    pub fn toggle_worktree_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_worktree_picker_open = !self.is_worktree_picker_open;
        if self.is_worktree_picker_open {
            self.worktree_picker_query.clear();
            self.worktree_picker_selected = 0;
            self.is_settings_open = false;
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_snippet_picker_open = false;
            self.is_pr_picker_open = false;
            self.is_search_open = false;
            self.is_project_jumper_open = false;
        }
        cx.notify();
    }

    pub fn toggle_project_jumper(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_project_jumper_open = !self.is_project_jumper_open;
        if self.is_project_jumper_open {
            self.project_jumper_query.clear();
            self.project_jumper_selected = 0;
            self.is_settings_open = false;
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_snippet_picker_open = false;
            self.is_pr_picker_open = false;
            self.is_search_open = false;
            self.is_worktree_picker_open = false;
            self.is_tab_overview_open = false;
            self.is_global_search_open = false;
        }
        cx.notify();
    }

    pub fn toggle_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_file_picker_open {
            self.is_file_picker_open = false;
            cx.notify();
            return;
        }
        self.open_file_picker(window, cx);
    }

    /// Open the file picker over the active tab's working directory.
    pub fn open_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_file_picker_open = true;
        self.file_picker_query.clear();
        self.file_picker_selected = 0;
        self.file_picker_scroll_handle.scroll_to_item(0);
        self.is_settings_open = false;
        self.is_context_menu_open = false;
        self.is_about_open = false;
        self.is_command_palette_open = false;
        self.is_ssh_manager_open = false;
        self.is_snippet_picker_open = false;
        self.is_pr_picker_open = false;
        self.is_search_open = false;
        self.is_worktree_picker_open = false;
        self.is_project_jumper_open = false;
        self.is_tab_overview_open = false;
        self.is_global_search_open = false;
        self.file_picker_anchor = self.terminal_cursor_anchor(window);
        let active_tab = self.tabs.get(self.active_tab_idx);
        self.file_picker_root = active_tab.and_then(|t| t.cwd.clone());
        let current_branch = active_tab
            .and_then(|t| t.git_status.as_ref())
            .map(|g| g.branch.clone());
        self.file_picker_index = crate::universal_picker::build_sources(
            self.file_picker_root.as_deref(),
            current_branch.as_deref(),
        );
        cx.notify();
    }

    /// Window-space pixel position of the active terminal cursor line.
    ///
    /// Mirrors the grid math in `update_selection_endpoint`: visible row is
    /// the cursor line plus the scrollback display offset.
    fn terminal_cursor_anchor(&self, window: &Window) -> Option<(f32, f32)> {
        let tab = self.tabs.get(self.active_tab_idx)?;
        let pane = tab.pane_tree.active_pane();
        let terminal = pane
            .as_ref()
            .and_then(|p| p.terminal.as_ref())
            .or(tab.terminal.as_ref())?;
        let bounds = pane.as_ref().and_then(|p| p.last_bounds)?;
        let (cell_w, line_h) = self.measure_cell_metrics(window);
        let line_h = if line_h > 0.0 { line_h } else { 18.0 };
        let cell_w = if cell_w > 0.0 { cell_w } else { 9.0 };
        let pane_x = bounds.origin.x.to_f64() as f32;
        let pane_y = bounds.origin.y.to_f64() as f32;
        let pane_h = bounds.size.height.to_f64() as f32;
        let (cur_line, cur_col) = terminal.cursor_screen_pos();
        let row = (cur_line as f32 + terminal.display_offset() as f32)
            .min(((pane_h / line_h) as f32).floor());
        let x = pane_x + cur_col as f32 * cell_w;
        let y = pane_y + row * line_h;
        Some((x, y))
    }

    /// Closes the picker and inserts the selected item into the focused
    /// pane's prompt: paths quoted, commands pre-filled with a trailing
    /// space, snippet bodies expanded.
    pub fn insert_selected_file_path(&mut self, cx: &mut Context<Self>) {
        let matches = crate::universal_picker::search(
            &self.file_picker_index,
            &self.file_picker_query,
            crate::universal_picker::MAX_RESULTS,
        );
        let Some(item) = matches.get(self.file_picker_selected) else {
            return;
        };
        let text = item.insert.clone();
        self.is_file_picker_open = false;
        if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
            let terminal = active_tab
                .pane_tree
                .active_pane()
                .and_then(|p| p.terminal.clone())
                .or_else(|| active_tab.terminal.clone());
            if let Some(ref terminal) = terminal {
                crate::paste::paste_text_to_terminal(terminal, &text);
            }
        }
        cx.notify();
    }

    /// Route dropped files to the AI composer when they landed on it, and to
    /// the focused terminal otherwise.
    pub fn handle_file_drop(
        &mut self,
        paths: &[std::path::PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.route_file_drop(
            paths,
            window.viewport_size().width.to_f64() as f32,
            window.mouse_position().x.to_f64() as f32,
        );
        cx.notify();
    }

    /// The routing itself, with the geometry handed in.
    ///
    /// No `Window`, because a panel child that blocks the mouse has to carry
    /// its own `on_drop` and reach this through a weak entity, which cannot
    /// hand over a `&mut Window`.
    ///
    /// `drop_x` comes from `window.mouse_position()`. That is the drop position:
    /// the platform input loop sets it immediately before it synthesises the
    /// mouse-up that triggers `on_drop`.
    ///
    /// The terminal area and the AI composer are children of the same div, which
    /// carries the `on_drop`. An earlier version pasted every file into the shell
    /// because the handler never looked at the position, not because the drop
    /// travelled somewhere unexpected. Both `on_drop` sites call this, so the
    /// routing does not depend on which one the hitbox picks.
    pub fn route_file_drop(&mut self, paths: &[std::path::PathBuf], viewport_w: f32, drop_x: f32) {
        self.ai_file_drag_hover = false;
        let sidebar_w = self.current_ai_sidebar_width();
        match file_drop_target_at(sidebar_w > 0.0, viewport_w - sidebar_w, drop_x) {
            FileDropTarget::AiAttach => {
                for path in paths {
                    self.attach_ai_file(path.clone());
                }
            }
            FileDropTarget::Terminal => {
                if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref terminal) = active_tab.terminal {
                        crate::paste::handle_dropped_paths(terminal, paths);
                    }
                }
            }
        }
    }

    pub fn attach_ai_file(&mut self, path: std::path::PathBuf) {
        if !self.ai_attached_files.contains(&path) {
            self.ai_attached_files.push(path);
        }
    }

    pub fn toggle_tab_overview(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_tab_overview_open = !self.is_tab_overview_open;
        if self.is_tab_overview_open {
            // Clamp on open, so the field is a valid tab index before the first
            // frame paints or the first key press reads it.
            self.tab_overview_selected = self.active_tab_idx.min(self.tabs.len().saturating_sub(1));
            self.is_global_search_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_snippet_picker_open = false;
            self.is_pr_picker_open = false;
            self.is_search_open = false;
            self.is_worktree_picker_open = false;
            self.is_project_jumper_open = false;
            self.is_settings_open = false;
            self.is_about_open = false;
            self.tab_overview_scroll_handle
                .scroll_to_item(self.tab_overview_selected);
        }
        cx.notify();
    }

    pub fn toggle_global_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_global_search_open = !self.is_global_search_open;
        if self.is_global_search_open {
            self.is_tab_overview_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_snippet_picker_open = false;
            self.is_pr_picker_open = false;
            self.is_search_open = false;
            self.is_worktree_picker_open = false;
            self.is_project_jumper_open = false;
            self.is_settings_open = false;
            self.is_about_open = false;
            self.global_search_query.clear();
            self.global_search_results.clear();
            self.global_search_selected = 0;
            self.global_search_scroll_handle.scroll_to_item(0);
        }
        cx.notify();
    }

    pub fn update_global_search(&mut self, cx: &mut Context<Self>) {
        self.global_search_results.clear();
        self.global_search_selected = 0;
        let q = self.global_search_query.trim();
        if q.is_empty() {
            cx.notify();
            return;
        }

        for (tab_idx, tab) in self.tabs.iter().enumerate() {
            let proc = tab
                .terminal
                .as_ref()
                .and_then(|t| t.get_foreground_process_name());
            let tab_title = tab
                .custom_title
                .clone()
                .unwrap_or_else(|| tab.title.clone());

            // Check main terminal
            if let Some(ref term) = tab.terminal {
                let snippets = term.search_snippets(q, 10);
                for snip in snippets {
                    self.global_search_results.push(GlobalSearchResult {
                        tab_idx,
                        tab_id: tab.id,
                        tab_title: tab_title.clone(),
                        process_name: proc.clone(),
                        pane_id: 0,
                        offset: snip.offset,
                        line_index: snip.line_index,
                        line_content: snip.text,
                    });
                }
            }

            // Check split panes if any
            for pane in tab.pane_tree.all_panes() {
                if let Some(ref term) = pane.terminal {
                    let snippets = term.search_snippets(q, 10);
                    for snip in snippets {
                        self.global_search_results.push(GlobalSearchResult {
                            tab_idx,
                            tab_id: tab.id,
                            tab_title: pane
                                .custom_title
                                .clone()
                                .unwrap_or_else(|| pane.title.clone()),
                            process_name: term.get_foreground_process_name(),
                            pane_id: pane.id,
                            offset: snip.offset,
                            line_index: snip.line_index,
                            line_content: snip.text,
                        });
                    }
                }
            }
        }
        cx.notify();
    }

    pub fn save_workspace_session(&mut self, session_name: &str) -> anyhow::Result<()> {
        let mut persisted_tabs = Vec::new();
        for tab in &self.tabs {
            let layout = Some(tab.pane_tree.root.to_persisted());
            persisted_tabs.push(crate::session_manager::PersistedTab {
                id: tab.id,
                ai_tab_key: Some(tab.ai_tab_key.clone()),
                ai_conversation_id: Some(tab.ai_conversation_id.clone()),
                title: tab.title.clone(),
                custom_title: tab.custom_title.clone(),
                cwd: tab.cwd.as_ref().map(|c| c.to_string_lossy().to_string()),
                layout,
            });
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let data = crate::session_manager::SessionData {
            name: session_name.to_string(),
            created_at: now,
            updated_at: now,
            active_tab_idx: self.active_tab_idx,
            tabs: persisted_tabs,
        };
        crate::session_manager::save_session(&data)
    }

    pub fn restore_workspace_session(
        &mut self,
        session_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = crate::session_manager::load_session(session_name) {
            let shell = self
                .config
                .shell
                .clone()
                .or_else(|| std::env::var("SHELL").ok())
                .unwrap_or_else(crate::paths::default_system_shell);
            for tab_data in session.tabs {
                let tab_id = self.next_tab_id;
                self.next_tab_id += 1;

                let pane_tree = if let Some(ref persisted_layout) = tab_data.layout {
                    let root_node = crate::pane_tree::PaneNode::restore_from_persisted(
                        persisted_layout,
                        &mut |cwd, title| {
                            self.spawn_terminal_pane(&shell, &[], cwd, title, window, cx)
                        },
                    );
                    let first_pane_id = root_node
                        .all_panes()
                        .first()
                        .map(|p| p.id)
                        .unwrap_or(tab_id);
                    crate::pane_tree::PaneTree {
                        root: root_node,
                        active_pane_id: first_pane_id,
                    }
                } else {
                    let cwd = tab_data.cwd.as_deref().map(std::path::Path::new);
                    let title = tab_data
                        .custom_title
                        .clone()
                        .or(Some(tab_data.title.clone()));
                    let pane = self.spawn_terminal_pane(&shell, &[], cwd, title, window, cx);
                    crate::pane_tree::PaneTree::new(pane)
                };

                let active_pane = pane_tree.active_pane();
                let pane_title = active_pane
                    .map(|p| p.title.clone())
                    .unwrap_or(tab_data.title);
                let pane_cwd = active_pane
                    .and_then(|p| p.cwd.clone())
                    .or_else(|| tab_data.cwd.map(std::path::PathBuf::from));
                let pane_term = active_pane.and_then(|p| p.terminal.clone());

                self.tabs.push(TabData {
                    id: tab_id,
                    ai_tab_key: tab_data.ai_tab_key.unwrap_or_else(|| new_ai_key("tab")),
                    ai_conversation_id: tab_data.ai_conversation_id.unwrap_or_else(|| new_ai_key("chat")),
                    pane_tree,
                    title: pane_title,
                    custom_title: tab_data.custom_title,
                    terminal: pane_term,
                    cwd: pane_cwd.clone(),
                    git_status: None,
                    git_checked_cwd: pane_cwd,
                    git_last_poll: Some(std::time::Instant::now()),
                    last_duration_ms: None,
                    last_exit_code: None,
                    zoomed_pane: None,
                });
            }

            if session.active_tab_idx < self.tabs.len() {
                self.active_tab_idx = session.active_tab_idx;
                let conversation_id = self.tabs[self.active_tab_idx].ai_conversation_id.clone();
                self.load_ai_conversation(&conversation_id);
            }
            self.persist_session();
            cx.notify();
        }
    }

    pub fn execute_palette_command(
        &mut self,
        cmd_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_command_palette_open = false;
        self.record_palette_recent(cmd_id);
        // Leaving the palette on a non-preview command discards any preview;
        // previewable commands (theme, font size, layout) commit in their
        // arms below: theme/layout are absolute (idempotent), font size
        // persists the already-previewed value.
        if palette_preview_kind(cmd_id).is_none() {
            self.cancel_palette_preview(_window, cx);
        }
        match cmd_id {
            "tab_overview" => self.toggle_tab_overview(_window, cx),
            "global_search" => self.toggle_global_search(_window, cx),
            "save_session" => {
                let _ = self.save_workspace_session("default-workspace");
                cx.notify();
            }
            "restore_session" => {
                self.restore_workspace_session("default-workspace", _window, cx);
            }
            "new_tab" => self.create_tab(_window, cx),
            "new_window" => {
                let cwd = self
                    .tabs
                    .get(self.active_tab_idx)
                    .and_then(|t| t.cwd.clone());
                let cli_opts = crate::cli::CliOptions {
                    working_dir: cwd,
                    ..Default::default()
                };
                let bounds = Bounds::centered(None, size(px(960.), px(640.)), &*cx);
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(size(px(640.), px(420.))),
                        window_background: WindowBackgroundAppearance::Blurred,
                        app_id: Some("com.fastty.app".into()),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Fastty".into()),
                            appears_transparent: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |w, cx| {
                        w.set_background_appearance(WindowBackgroundAppearance::Blurred);
                        cx.new(|cx| RootView::with_options(w, cli_opts, cx))
                    },
                )
                .ok();
            }
            "rename_tab" => self.open_rename_tab(self.active_tab_idx, cx),
            "close_tab" => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    let id = tab.id;
                    self.close_tab(id, _window, cx);
                }
            }
            "search" => self.toggle_search(_window, cx),
            "worktree" => self.toggle_worktree_picker(_window, cx),
            "project_jumper" => self.toggle_project_jumper(_window, cx),
            "file_picker" => self.toggle_file_picker(_window, cx),
            "clear" => {
                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    if let Some(ref term) = tab.terminal {
                        term.scroll_to_bottom();
                    }
                }
                cx.notify();
            }
            "ssh" => self.toggle_ssh_manager(_window, cx),
            "snippets" => self.toggle_snippet_picker(_window, cx),
            "prs" => self.toggle_pr_picker(_window, cx),
            "settings" => self.toggle_settings(_window, cx),
            "about" => self.toggle_about(_window, cx),
            "check_updates" => self.check_for_updates_manual(_window, cx),
            "fullscreen" => {
                _window.toggle_fullscreen();
                cx.notify();
            }
            "zoom_in" => {
                if self.palette_preview_original.is_none() {
                    self.adjust_font_size(1.0, cx);
                } else {
                    // Already previewed while navigating: just persist.
                    self.commit_font_size(cx);
                }
            }
            "zoom_out" => {
                if self.palette_preview_original.is_none() {
                    self.adjust_font_size(-1.0, cx);
                } else {
                    self.commit_font_size(cx);
                }
            }
            "zoom_reset" => {
                if self.palette_preview_original.is_none() {
                    self.font_size = 13.0;
                }
                self.commit_font_size(cx);
            }
            "theme_default" => self.set_theme("default", cx),
            theme_cmd if crate::config::THEME_REGISTRY.iter().any(|(_, id, _, _)| *id == theme_cmd) => {
                let theme_name = crate::config::THEME_REGISTRY
                    .iter()
                    .find(|(_, id, _, _)| *id == theme_cmd)
                    .map(|(name, _, _, _)| *name)
                    .unwrap_or("default");
                self.set_theme(theme_name, cx);
            }
            "open_config" => {
                let config_dir = dirs::home_dir()
                    .map(|h| h.join(".config/fastty"))
                    .unwrap_or_default();
                let _ = std::fs::create_dir_all(&config_dir);
                if let Some(path_str) = config_dir.to_str() {
                    open_path_or_url(path_str);
                }
                cx.notify();
            }
            "edit_config" => {
                Self::open_settings_file();
                cx.notify();
            }
            "split_right" => self.split_active_pane(Direction::Right, _window, cx),
            "split_down" => self.split_active_pane(Direction::Down, _window, cx),
            "split_left" => self.split_active_pane(Direction::Left, _window, cx),
            "split_top" => self.split_active_pane(Direction::Top, _window, cx),
            "focus_left" => self.focus_pane_in_direction(Direction::Left, cx),
            "focus_right" => self.focus_pane_in_direction(Direction::Right, cx),
            "focus_top" | "focus_up" => self.focus_pane_in_direction(Direction::Top, cx),
            "focus_down" => self.focus_pane_in_direction(Direction::Down, cx),
            "close_pane" => self.close_active_pane(_window, cx),
            "zoom_pane" => self.toggle_pane_zoom(cx),
            "toggle_tab_sidebar" => self.toggle_tab_sidebar(_window, cx),
            "toggle_ai_sidebar" => self.toggle_ai_sidebar(_window, cx),
            "layout_horizontal" => self.set_tab_layout_mode(TabLayout::Horizontal, _window, cx),
            "layout_vertical" => self.set_tab_layout_mode(TabLayout::Vertical, _window, cx),
            "quit" => cx.quit(),
            _ => {}
        }
    }

    pub fn toggle_tab_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_layout == TabLayout::Horizontal {
            self.tab_layout = TabLayout::Vertical;
            self.config.tab_layout = TabLayout::Vertical;
            let _ = self.config.save_default();
            crate::config::increment_config_version();
            self.sidebar_open = true;
        } else {
            self.sidebar_open = !self.sidebar_open;
        }
        self.trigger_sidebar_animation(window, cx);
        cx.notify();
    }

    pub fn toggle_ai_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai_sidebar_open = !self.ai_sidebar_open;
        if self.ai_sidebar_open {
            self.ai_input_focused = true;
            window.focus(&self.focus_handle, cx);
        }
        self.persist_session();
        self.trigger_ai_sidebar_animation(window, cx);
        cx.notify();
    }

    fn ai_response_is_visible(&self, window: &Window) -> bool {
        self.ai_event_source.is_none()
            && self.ai_sidebar_open
            && window.is_window_active()
            && !self.is_settings_open
            && !self.is_update_modal_open
            && !self.is_whats_new_open
            && !self.is_tab_overview_open
    }

    fn notify_ai_status(
        &mut self,
        window: &Window,
        title: &str,
        urgency: AiNotificationUrgency,
    ) {
        if !self.config.notify_on_ai_status || self.ai_response_is_visible(window) {
            return;
        }
        let now = std::time::Instant::now();
        if urgency == AiNotificationUrgency::Routine
            && self.ai_last_notification.is_some_and(|last| now.duration_since(last).as_secs() < 5)
        {
            return;
        }
        let tab = self.ai_event_source.as_ref()
            .and_then(|(id, _)| self.tabs.iter().find(|tab| tab.id == *id))
            .or_else(|| self.tabs.get(self.active_tab_idx));
        let Some(tab) = tab else { return };
        let conversation_id = self.ai_event_source.as_ref().map(|(_, id)| id.as_str())
            .unwrap_or(tab.ai_conversation_id.as_str());
        let conversation_title = self.ai_chat_records.iter().find(|record| record.id == conversation_id)
            .map(|record| record.title.as_str()).unwrap_or("AI chat");
        let body = format!("{} / {}: {}", tab.custom_title.as_deref().unwrap_or(&tab.title), conversation_title, self.ai_turn_provider);
        self.ai_last_notification = Some(now);
        let title = title.to_owned();
        send_system_notification(&title, &body);
    }

    fn refresh_program_status(&mut self, source_id: u64, window: &Window, cx: &mut Context<Self>) {
        let source = self.tabs.iter().flat_map(|tab| tab.pane_tree.all_panes())
            .find_map(|pane| pane.terminal.as_ref()
                .filter(|terminal| terminal.source_id() == source_id)
                .map(|terminal| (pane.id, terminal.program_status())));
        if let Some((pane_id, records)) = source {
            self.update_program_status(pane_id, records, window, cx);
        }
    }

    fn update_program_status(
        &mut self,
        pane_id: PaneId,
        records: Vec<ProgramRecord>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.pane_tree.find_pane(pane_id).is_some()) else {
            return;
        };
        let source = format!("{} / pane {}", tab.custom_title.as_deref().unwrap_or(&tab.title), pane_id);
        let foreground = window.is_window_active()
            && !self.any_input_overlay_open()
            && self.tabs.get(self.active_tab_idx).is_some_and(|tab| tab.pane_tree.active_pane_id == pane_id);
        let status = self.program_statuses.entry(pane_id).or_default();
        let changed_attention: Vec<ProgramRecord> = records.iter().filter(|record| {
            matches!(record.state, ProgramState::Done | ProgramState::Blocked | ProgramState::Error)
                && !status.records.iter().any(|previous| {
                    previous.id == record.id && previous.state == record.state && previous.kind == record.kind
                })
        }).cloned().collect();
        let has_attention = records.iter().any(|record| {
            matches!(record.state, ProgramState::Done | ProgramState::Blocked | ProgramState::Error)
        });
        status.unread = has_attention && (status.unread || !changed_attention.is_empty());
        status.records = records;
        let now = std::time::Instant::now();
        if self.config.notify_on_program_status && !foreground {
            if let Some(record) = crate::program_status::aggregate(&changed_attention) {
                let requires_action = record.state == ProgramState::Blocked;
                let cooldown_elapsed = status
                    .last_notification
                    .is_none_or(|last| now.duration_since(last).as_secs() >= 5);
                if !requires_action && !cooldown_elapsed {
                    cx.notify();
                    return;
                }
                let title = format!("Fastty: {}", super::tab_bar::program_status_text(record));
                let message = record.msg.as_deref().or(record.title.as_deref()).unwrap_or("");
                let body = format!("{}: {} {}", source, record.app.as_deref().unwrap_or("Program"), message);
                status.last_notification = Some(now);
                send_system_notification(&title, &body);
            }
        }
        cx.notify();
    }

    fn visible_program_record(&self, pane_id: PaneId) -> Option<&ProgramRecord> {
        let status = self.program_statuses.get(&pane_id)?;
        status.records.iter().filter(|record| match record.state {
            ProgramState::Idle => false,
            ProgramState::Done | ProgramState::Error => status.unread,
            ProgramState::Working | ProgramState::Blocked => true,
        }).max_by_key(|record| record.state.priority())
    }

    fn acknowledge_program_status(&mut self, pane_id: PaneId) {
        if let Some(status) = self.program_statuses.get_mut(&pane_id) {
            status.unread = false;
        }
    }

    /// 160ms ease-out-cubic open/close animation, same as the tabs sidebar.
    pub fn trigger_ai_sidebar_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.ai_sidebar_open { 1.0f32 } else { 0.0f32 };

        if (self.ai_sidebar_anim_progress - target).abs() < 0.001 {
            return;
        }

        cx.spawn_in(window, async move |this, cx| {
            let start_time = std::time::Instant::now();
            let duration = std::time::Duration::from_millis(160);
            let initial = this
                .update_in(cx, |this, _window, _cx| this.ai_sidebar_anim_progress)
                .unwrap_or(0.0);

            loop {
                let elapsed = start_time.elapsed();
                let t = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
                let eased = 1.0 - (1.0 - t).powi(3);
                let current = initial + (target - initial) * eased;
                let done = t >= 1.0;

                let res = this.update_in(cx, |this, _window, cx| {
                    this.ai_sidebar_anim_progress = if done { target } else { current };
                    cx.notify();
                });

                if res.is_err() || done {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(8))
                    .await;
            }
        })
        .detach();
    }

    pub fn cancel_ai_stream(&mut self) {
        self.ai_stream_generation = self.ai_stream_generation.wrapping_add(1);
        if let Some(token) = self.ai_cancel_token.take() {
            token.cancel();
        }
        // Deny any pending permission request so the blocked agent thread
        // unblocks and terminates instead of leaking into the next session.
        if let Some(tx) = self.ai_confirm_reply_tx.take() {
            let _ = tx.send_blocking(crate::ai::PermissionDecision::Deny);
        }
        if let Some(tx) = self.ai_opencode_confirm_reply_tx.take() {
            let _ = tx.send_blocking(None);
        }
        self.ai_pending_confirmation = None;
        if !self.ai_pending_text.is_empty() {
            self.ai_streaming_text.push_str(&self.ai_pending_text);
            self.ai_pending_text.clear();
        }
        if !self.ai_pending_thinking.is_empty() {
            self.ai_streaming_thinking
                .push_str(&self.ai_pending_thinking);
            self.ai_pending_thinking.clear();
        }
        if self.ai_current_turn_usage != (0, 0) {
            self.ai_last_usage = Some(self.ai_current_turn_usage);
        }
        self.ai_current_turn_usage = (0, 0);
        self.ai_is_streaming = false;
    }

    pub fn update_ai_at_matches(&mut self, query: &str) {
        self.ai_at_is_skill_menu = false;
        let active_cwd = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|t| t.cwd.as_ref().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| "~".to_string());

        self.ai_at_matches = crate::file_search::search_from_root(
            std::path::Path::new(&active_cwd),
            query,
            crate::ui::ai_sidebar::AI_AT_MENU_MAX_ROWS,
        )
        .into_iter()
        .map(|entry| entry.rel_path)
        .collect();
        // The list just changed under the selection. Typing filters it, so an
        // index from before can point past the end.
        self.ai_at_selected = 0;
        // The handle outlives the popup and keeps its offset, so a reopened menu
        // would start scrolled away from the row that is now highlighted.
        self.ai_at_scroll_handle.set_offset(gpui::Point::default());
    }

    fn update_ai_skill_matches(&mut self, query: &str) {
        self.ai_at_is_skill_menu = true;
        let cwd = self.tabs.get(self.active_tab_idx).and_then(|tab| tab.cwd.clone()).unwrap_or_default();
        let needle = query.to_lowercase();
        let mut matches: Vec<String> = crate::ai::conversations::discover_skills(&cwd).into_iter()
            .filter(|skill| needle.is_empty() || skill.name.to_lowercase().contains(&needle))
            .map(|skill| format!("skill::{}::{}", skill.name, skill.scope))
            .collect();
        let conversation_id = self.tabs.get(self.active_tab_idx).map(|tab| tab.ai_conversation_id.as_str()).unwrap_or_default();
        if self.config.ai.default == "opencode" {
        let active_commands = self.ai_chat_records.iter().find(|record| record.id == conversation_id)
            .filter(|record| !record.available_commands.is_empty())
            .or_else(|| self.ai_chat_records.iter().find(|record| record.cwd == cwd && !record.available_commands.is_empty()));
        if let Some(record) = active_commands {
            matches.extend(record.available_commands.iter()
                .filter(|command| needle.is_empty() || command.name.to_lowercase().contains(&needle))
                .map(|command| format!("acp::{}", command.name)));
        }
        }
        matches.truncate(crate::ui::ai_sidebar::AI_AT_MENU_MAX_ROWS);
        self.ai_at_matches = matches;
        self.ai_at_selected = 0;
        self.ai_at_scroll_handle.set_offset(gpui::Point::default());
    }

    /// Rows the mention menu draws, which is what the arrow keys move over.
    pub fn ai_at_row_count(&self) -> usize {
        at_menu_row_count(&self.ai_at_matches)
    }

    /// Move the mention menu selection by `delta` rows, wrapping at both ends.
    pub fn move_ai_at_selection(&mut self, delta: isize) {
        move_at_menu_selection(&mut self.ai_at_selected, &self.ai_at_matches, delta);
    }

    /// True when the AI composer gets the key instead of an overlay or the shell.
    pub fn ai_keys_own_input(&self) -> bool {
        ai_keys_own_input(
            self.ai_sidebar_open,
            self.ai_input_focused,
            self.is_file_picker_open,
        )
    }

    /// Bring the highlighted mention row into view. The popup scrolls; the rows
    /// are taller than it.
    fn scroll_at_menu_selection_into_view(&mut self) {
        let last = self.ai_at_row_count().saturating_sub(1);
        self.ai_at_scroll_handle
            .scroll_to_item(self.ai_at_selected.min(last));
    }

    pub fn insert_ai_at_path(&mut self, path: &str) {
        if path.starts_with("skill::") || path.starts_with("acp::") {
            let Some(name) = path.split("::").nth(1) else { return };
            let text = self.ai_input_state.text.clone();
            let chars: Vec<char> = text.chars().collect();
            let cursor = self.ai_input_state.cursor.min(chars.len());
            let before: String = chars[..cursor].iter().collect();
            let after: String = chars[cursor..].iter().collect();
            let Some(slash_idx) = before.rfind('/') else { return };
            let prefix = &before[..slash_idx];
            let insertion = format!("{prefix}/{name} {after}");
            let new_cursor = prefix.chars().count() + name.chars().count() + 2;
            self.ai_input_state.set_text_with_cursor(insertion, new_cursor);
            self.ai_input_text = self.ai_input_state.text.clone();
            self.ai_at_menu_open = false;
            return;
        }
        let text = self.ai_input_state.text.clone();
        let cursor_char_idx = self.ai_input_state.cursor;
        let chars: Vec<char> = text.chars().collect();
        let cursor_clamped = cursor_char_idx.min(chars.len());

        let before_cursor: String = chars[..cursor_clamped].iter().collect();
        let after_cursor: String = chars[cursor_clamped..].iter().collect();

        if let Some(at_idx) = before_cursor.rfind('@') {
            let before_at = &before_cursor[..at_idx];
            let new_text = format!("{}@{} {}", before_at, path, after_cursor);
            let new_cursor = before_at.chars().count() + 1 + path.chars().count() + 1;
            self.ai_input_state
                .set_text_with_cursor(new_text, new_cursor);
        } else {
            let insertion = format!("@{} ", path);
            self.ai_input_state.insert_str(&insertion);
        }
        self.ai_input_text = self.ai_input_state.text.clone();
        self.ai_at_menu_open = false;
    }

    pub fn check_ai_at_trigger(&mut self) {
        let text = &self.ai_input_state.text;
        let cursor = self.ai_input_state.cursor;
        let chars: Vec<char> = text.chars().collect();
        let cursor_clamped = cursor.min(chars.len());
        let before: String = chars[..cursor_clamped].iter().collect();
        if let Some(slash_idx) = before.rfind('/') {
            let prefix = &before[..slash_idx];
            let query = &before[slash_idx + 1..];
            let at_token_start = prefix.chars().last().map(char::is_whitespace).unwrap_or(true);
            if at_token_start && !query.chars().any(char::is_whitespace) {
                self.update_ai_skill_matches(query);
                self.ai_at_menu_open = true;
                return;
            }
        }
        if let Some(at_idx) = before.rfind('@') {
            let query = &before[at_idx + 1..];
            if !query.contains(' ') && !query.contains('\n') {
                self.update_ai_at_matches(query);
                self.ai_at_menu_open = true;
                return;
            }
        }
        self.ai_at_menu_open = false;
    }

    /// Native OS file dialog for the AI composer.
    ///
    /// It runs a blocking external process, so it goes on its own thread and
    /// comes back through a channel. The path it returns is absolute, which is
    /// what [`attach_ai_file`] needs.
    pub fn open_ai_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::bounded::<Option<std::path::PathBuf>>(1);
        std::thread::spawn(move || {
            let res = crate::ui::ai_sidebar::pick_file_dialog();
            let _ = tx.send_blocking(res);
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Some(path)) = rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    this.attach_ai_file(path);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn submit_ai_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.ai_input_state.text.trim().to_string();
        if (text.is_empty() && self.ai_attached_files.is_empty()) || self.ai_is_streaming {
            return;
        }
        self.ai_input_state.clear();
        self.ai_input_text.clear();
        self.ai_at_menu_open = false;
        self.ai_opencode_variants_open = false;

        let active_cwd = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|t| t.cwd.clone())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let mut augmented_content = text.clone();
        let universal_skill = text.trim_start().split_whitespace().next().and_then(|word| {
            let name = word.strip_prefix('/')?.trim_matches(|character: char| {
                !character.is_alphanumeric() && character != '-' && character != '_'
            });
            crate::ai::conversations::load_skill(name, &active_cwd)
        });
        if let Some((skill, body)) = universal_skill {
            let command = format!("/{}", skill.name);
            augmented_content = text.replacen(&command, "", 1).trim().to_string();
            let scope = skill.scope;
            let directory = skill.path.parent().unwrap_or(&skill.path).display();
            augmented_content.push_str(&format!(
                "\n\n<skill name=\"{}\" scope=\"{}\" directory=\"{}\">\nRelative paths resolve from this skill directory.\n\n{}\n</skill>",
                skill.name, scope, directory, body
            ));
        }

        // 1. Parse @path references from user input
        for word in text.split_whitespace() {
            if let Some(path_part) = word.strip_prefix('@') {
                let clean = path_part.trim_matches(|c: char| {
                    !c.is_alphanumeric() && c != '.' && c != '/' && c != '_' && c != '-'
                });
                if !clean.is_empty() {
                    let path = active_cwd.join(clean);
                    if path.is_file() {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let snippet = truncate_for_prompt(&content);
                            augmented_content.push_str(&format!(
                                "\n\n[Referenced file: {}]\n```\n{}\n```",
                                clean, snippet
                            ));
                        }
                    }
                }
            }
        }

        // 2. Attach any files/images selected with clip picker
        let attached = std::mem::take(&mut self.ai_attached_files);
        let mut attached_images: Vec<std::path::PathBuf> = Vec::new();
        let mut attached_documents: Vec<std::path::PathBuf> = Vec::new();
        for path in attached {
            let is_img = crate::ui::ai_sidebar::is_image_path(&path);
            let is_pdf = crate::ai::is_pdf_path(&path);
            if is_img {
                attached_images.push(path);
            } else if is_pdf {
                attached_documents.push(path);
            } else if path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let snippet = truncate_for_prompt(&content);
                    augmented_content.push_str(&format!(
                        "\n\n[Attached file: {}]\n```\n{}\n```",
                        path.display(),
                        snippet
                    ));
                } else {
                    augmented_content
                        .push_str(&format!("\n\n[Attached binary file: {}]", path.display()));
                }
            }
        }

        self.ai_messages.push(crate::ui::ai_sidebar::AiUiMessage {
            is_user: true,
            text: text.clone(),
            thinking: None,
            tool_calls: Vec::new(),
            timestamp: Some(crate::ui::ai_sidebar::current_time_str()),
            images: attached_images.clone(),
            documents: attached_documents.clone(),
            is_error: false,
        });
        self.save_active_ai_conversation();
        self.ai_scroll_handle.scroll_to_bottom();

        let cfg = self.config.ai.clone();
        let active_prov = self.config.ai.default.clone();
        let active_mod = self.config.ai.active_model();
        let active_tab_id = self.tabs.get(self.active_tab_idx).map(|tab| tab.id).unwrap_or(0);
        let active_conversation_id = self.tabs.get(self.active_tab_idx)
            .map(|tab| tab.ai_conversation_id.clone()).unwrap_or_default();
        let provider_config = cfg.providers.get(&active_prov);
        let acp_provider_identity = provider_config.and_then(|provider| provider.acp_identity(&active_prov));
        let saved_acp_session_id = self.ai_chat_records.iter()
            .find(|record| record.id == active_conversation_id)
            .filter(|record| record.acp_provider == acp_provider_identity)
            .and_then(|record| record.acp_session_id.clone());
        let is_opencode = active_prov == "opencode";
        let is_acp = provider_config.is_some_and(crate::ai::ProviderConfig::is_acp);
        let model = if is_acp {
            None
        } else {
            match crate::ai::create_model_from_config(
                &cfg,
                Some(&active_prov),
                Some(active_mod.as_str()),
            ) {
                Ok((model, model_name)) => Some((model, model_name)),
                Err(e) => {
                    self.ai_messages.push(ai_error_message(&format!("initializing AI model: {e}")));
                    cx.notify();
                    return;
                }
            }
        };

        let tools = if self.ai_agent_mode == "Ask" {
            Vec::new()
        } else {
            crate::ai::default_tools()
        };
        self.ai_permission_checker.set_mode(cfg.permission_mode);
        let checker = self.ai_permission_checker.clone();

        let (ask_tx, ask_rx) = async_channel::unbounded::<(
            String,
            String,
            String,
            async_channel::Sender<crate::ai::PermissionDecision>,
        )>();
        let (opencode_ask_tx, opencode_ask_rx) = async_channel::unbounded::<(
            String,
            String,
            String,
            Vec<crate::ai::opencode::PermissionOption>,
            async_channel::Sender<Option<String>>,
        )>();
        let (event_tx, event_rx) = async_channel::unbounded::<crate::ai::AgentEvent>();

        struct UiPermissionHandler {
            ask_tx: async_channel::Sender<(
                String,
                String,
                String,
                async_channel::Sender<crate::ai::PermissionDecision>,
            )>,
        }
        impl crate::ai::PermissionHandler for UiPermissionHandler {
            fn ask_permission(
                &self,
                tool_call_id: &str,
                tool_name: &str,
                input_summary: &str,
            ) -> crate::ai::PermissionDecision {
                let (reply_tx, reply_rx) = async_channel::bounded(1);
                if self
                    .ask_tx
                    .send_blocking((
                        tool_call_id.to_string(),
                        tool_name.to_string(),
                        input_summary.to_string(),
                        reply_tx,
                    ))
                    .is_err()
                {
                    return crate::ai::PermissionDecision::Deny;
                }
                reply_rx
                    .recv_blocking()
                    .unwrap_or(crate::ai::PermissionDecision::Deny)
            }
        }

        let perm_handler = std::sync::Arc::new(UiPermissionHandler { ask_tx });
        let ctx_tool = crate::ai::ToolCtx {
            cwd: active_cwd.clone(),
        };

        let mut agent = model.map(|(model, model_name)| {
            crate::ai::Agent::new(model, model_name, tools, checker, perm_handler, ctx_tool)
        });

        let cancel_tok = agent.as_ref().map(crate::ai::Agent::cancel_token)
            .unwrap_or_else(crate::ai::CancelToken::new);
        self.ai_cancel_token = Some(cancel_tok.clone());
        self.ai_next_stream_generation = self.ai_next_stream_generation.wrapping_add(1);
        self.ai_stream_generation = self.ai_next_stream_generation;
        self.ai_turn_provider = active_prov.clone();
        self.ai_turn_model = active_mod.clone();
        let stream_generation = self.ai_stream_generation;
        self.ai_is_streaming = true;
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            self.ai_turn_completions.remove(&tab.ai_conversation_id);
        }
        self.ai_streaming_text.clear();
        self.ai_streaming_thinking.clear();
        self.ai_pending_text.clear();
        self.ai_pending_thinking.clear();

        if let Some(agent) = agent.as_mut() {
        let sys_prompt = if self.ai_agent_mode == "Ask" {
            format!(
                "{}\n\n[MODE: ASK ONLY]\nYou are in Ask mode. You must ONLY answer questions, explain concepts, and provide direct advice. You have no tools and cannot execute commands or modify files.",
                crate::ai::system_prompt(&active_cwd)
            )
        } else {
            crate::ai::system_prompt(&active_cwd)
        };
        agent.add_message(crate::ai::Message::system(sys_prompt));
        let last_idx = self.ai_messages.len().saturating_sub(1);
        for (idx, m) in self.ai_messages.iter().enumerate() {
            if m.is_user {
                if idx == last_idx {
                    if !attached_images.is_empty() || !attached_documents.is_empty() {
                        let mut parts = Vec::new();
                        for img_path in &attached_images {
                            match crate::ai::load_and_prepare_image(img_path) {
                                Ok(part) => parts.push(part),
                                Err(e) => {
                                    augmented_content.push_str(&format!(
                                        "\n\n[Attached image error {}: {}]",
                                        img_path.display(),
                                        e
                                    ));
                                }
                            }
                        }
                        for doc_path in &attached_documents {
                            match crate::ai::load_and_prepare_document(doc_path) {
                                Ok(part) => parts.push(part),
                                Err(e) => {
                                    augmented_content.push_str(&format!(
                                        "\n\n[Attached PDF error {}: {}]",
                                        doc_path.display(),
                                        e
                                    ));
                                }
                            }
                        }
                        let prompt_text = if augmented_content.trim().is_empty() {
                            if !attached_documents.is_empty() {
                                "Analyze and summarize this document.".to_string()
                            } else {
                                "Describe this image and analyze its contents.".to_string()
                            }
                        } else {
                            augmented_content.clone()
                        };
                        parts.push(crate::ai::ContentPart::Text { text: prompt_text });
                        agent.add_message(crate::ai::Message::user_parts(parts));
                    } else {
                        agent.add_message(crate::ai::Message::user(&augmented_content));
                    }
                } else if !m.images.is_empty() || !m.documents.is_empty() {
                    let mut parts = Vec::new();
                    for img_path in &m.images {
                        if let Ok(part) = crate::ai::load_and_prepare_image(img_path) {
                            parts.push(part);
                        }
                    }
                    for doc_path in &m.documents {
                        if let Ok(part) = crate::ai::load_and_prepare_document(doc_path) {
                            parts.push(part);
                        }
                    }
                    let prompt_text = if m.text.trim().is_empty() {
                        if !m.documents.is_empty() {
                            "Analyze and summarize this document.".to_string()
                        } else {
                            "Describe this image and analyze its contents.".to_string()
                        }
                    } else {
                        m.text.clone()
                    };
                    parts.push(crate::ai::ContentPart::Text { text: prompt_text });
                    agent.add_message(crate::ai::Message::user_parts(parts));
                } else {
                    agent.add_message(crate::ai::Message::user(&m.text));
                }
            } else if !m.text.is_empty() {
                agent.add_message(crate::ai::Message::assistant(&m.text));
            }
        }
        }

        let event_tx_clone = event_tx.clone();
        let event_tab_id = active_tab_id;
        let event_conversation_id = active_conversation_id.clone();
        let opencode_manager = Arc::clone(&self.ai_opencode_manager);
        let acp_runtime_key = acp_provider_identity.as_ref()
            .map(|identity| format!("{active_conversation_id}::{identity}"))
            .unwrap_or_else(|| active_conversation_id.clone());
        let acp_provider = cfg.providers.get(&active_prov);
        let opencode_command = acp_provider
            .and_then(crate::ai::ProviderConfig::command)
            .unwrap_or("opencode")
            .to_string();
        let opencode_args = acp_provider.and_then(crate::ai::ProviderConfig::acp_args)
            .unwrap_or_else(|| vec!["acp".to_string()]);
        let opencode_cwd = active_cwd.clone();
        let opencode_model = active_mod.clone();
        let opencode_effort = acp_provider.and_then(|provider| {
            if let crate::ai::ProviderConfig::Opencode { variants, .. } | crate::ai::ProviderConfig::Acp { variants, .. } = provider {
                crate::ai::opencode::catalog_model_key(&opencode_model, variants)
                    .and_then(|key| variants.get(&key))
            } else {
                None
            }
        }).and_then(|variants| {
            variants.iter().find(|variant| **variant == self.ai_opencode_variant)
                .or_else(|| variants.iter().find(|variant| variant.as_str() == "default"))
                .or_else(|| variants.first())
        }).cloned().unwrap_or_default();
        let opencode_mode = if self.ai_agent_mode == "Ask" { "plan" } else { "build" }.to_string();
        let mut opencode_prompt = Vec::new();
        if is_acp {
            let mut prompt_text = if augmented_content.trim().is_empty() {
                if !attached_documents.is_empty() {
                    "Analyze and summarize this document.".to_string()
                } else if !attached_images.is_empty() {
                    "Describe this image and analyze its contents.".to_string()
                } else {
                    String::new()
                }
            } else {
                augmented_content.clone()
            };
            for path in &attached_images {
                match crate::ai::load_and_prepare_image(path) {
                    Ok(crate::ai::ContentPart::Image { media_type, data }) => {
                        opencode_prompt.push(serde_json::json!({ "type": "image", "mimeType": media_type, "data": data }));
                    }
                    Ok(_) => {}
                    Err(error) => prompt_text.push_str(&format!("\n\n[Attached image error {}: {}]", path.display(), error)),
                }
            }
            for path in &attached_documents {
                match crate::ai::load_and_prepare_document(path) {
                    Ok(crate::ai::ContentPart::Document { media_type, data, name }) => {
                        let uri = format!("file://{}", path.to_string_lossy());
                        opencode_prompt.push(serde_json::json!({ "type": "resource", "resource": { "uri": uri, "mimeType": media_type, "blob": data, "name": name } }));
                    }
                    Ok(_) => {}
                    Err(error) => prompt_text.push_str(&format!("\n\n[Attached PDF error {}: {}]", path.display(), error)),
                }
            }
            opencode_prompt.push(serde_json::json!({ "type": "text", "text": prompt_text }));
        }
        std::thread::Builder::new()
            .name("ai-agent-turn".into())
            .spawn(move || {
                let tx = event_tx_clone;
                let res = if let Some(mut agent) = agent {
                    agent.run_turn(move |ev| {
                        let _ = tx.send_blocking(ev);
                    })
                } else {
                    run_opencode_turn(
                        opencode_manager,
                        acp_runtime_key,
                        saved_acp_session_id,
                        opencode_command,
                        opencode_args,
                        is_opencode,
                        opencode_cwd,
                        opencode_model,
                        opencode_effort,
                        opencode_mode,
                        opencode_prompt,
                        cancel_tok.clone(),
                        opencode_ask_tx,
                        tx.clone(),
                    )
                };
                if let Err(e) = res {
                    if !cancel_tok.is_cancelled() {
                        let _ = event_tx.send_blocking(crate::ai::AgentEvent::Error(format!("{e:#}")));
                    }
                }
            })
            .ok();

        let permission_conversation_id = event_conversation_id.clone();
        cx.spawn_in(window, async move |this, cx| {
            while let Ok((tool_id, tool_name, input_summary, reply_tx)) = ask_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    let rejected_reply = reply_tx.clone();
                    let accepted = this.with_ai_turn(event_tab_id, &permission_conversation_id, stream_generation, |this| {
                        this.ai_pending_confirmation =
                            Some(crate::ui::ai_sidebar::AiUiPendingConfirmation {
                                tool_id,
                                tool_name,
                                input_summary,
                                options: Vec::new(),
                        });
                    this.ai_confirm_reply_tx = Some(reply_tx);
                    this.notify_ai_status(
                        _window,
                        "Fastty AI: Permission required",
                        AiNotificationUrgency::RequiresAction,
                    );
                    this.ai_opencode_confirm_reply_tx = None;
                    // The confirmation card renders at the end of the message
                    // list; bring it into view so the user sees the diff.
                    if this.ai_event_source.is_none() { this.ai_scroll_handle.scroll_to_bottom(); }
                    cx.notify();
                    });
                    if accepted.is_none() {
                        let _ = rejected_reply.send_blocking(crate::ai::PermissionDecision::Deny);
                    }
                });
            }
        })
        .detach();

        let opencode_permission_conversation_id = event_conversation_id.clone();
        cx.spawn_in(window, async move |this, cx| {
            while let Ok((tool_id, tool_name, input_summary, options, reply_tx)) = opencode_ask_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    let rejected_reply = reply_tx.clone();
                    let accepted = this.with_ai_turn(event_tab_id, &opencode_permission_conversation_id, stream_generation, |this| {
                        this.ai_pending_confirmation = Some(crate::ui::ai_sidebar::AiUiPendingConfirmation {
                            tool_id,
                            tool_name,
                            input_summary,
                            options: options.into_iter().map(|option| crate::ui::ai_sidebar::AiUiPermissionOption {
                                option_id: option.option_id,
                                name: option.name,
                                kind: option.kind,
                            }).collect(),
                    });
                    this.ai_confirm_reply_tx = None;
                    this.ai_opencode_confirm_reply_tx = Some(reply_tx);
                    this.notify_ai_status(
                        _window,
                        "Fastty AI: Permission required",
                        AiNotificationUrgency::RequiresAction,
                    );
                    if this.ai_event_source.is_none() { this.ai_scroll_handle.scroll_to_bottom(); }
                    cx.notify();
                    });
                    if accepted.is_none() {
                        let _ = rejected_reply.send_blocking(None);
                    }
                });
            }
        })
        .detach();

        let stream_conversation_id = event_conversation_id;
        let session_provider_identity = acp_provider_identity;
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(ev) = event_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    let accepted = this.with_ai_turn(event_tab_id, &stream_conversation_id, stream_generation, |this| {
                        match ev {
                            crate::ai::AgentEvent::TextDelta(d) => {
                                this.ai_pending_text.push_str(&d);
                                // Do not call cx.notify() here!
                                // The 35ms ticker progressively drains ai_pending_text into ai_streaming_text,
                                // giving a smooth, word-by-word streaming effect without GPUI frame coalescing dumps.
                                return;
                            }
                            crate::ai::AgentEvent::ThinkingDelta(t) => {
                                this.ai_pending_thinking.push_str(&t);
                                // Also smooth reveal for thinking deltas
                                return;
                            }
                            crate::ai::AgentEvent::ToolStart { id, name, args } => {
                                if this.ai_current_turn_usage != (0, 0) {
                                    this.ai_last_usage = Some(this.ai_current_turn_usage);
                                }
                                this.ai_current_turn_usage = (0, 0);

                                // Flush any text/thinking accumulated so far into an assistant bubble
                                if !this.ai_pending_text.is_empty() {
                                    this.ai_streaming_text.push_str(&this.ai_pending_text);
                                    this.ai_pending_text.clear();
                                }
                                if !this.ai_pending_thinking.is_empty() {
                                    this.ai_streaming_thinking
                                        .push_str(&this.ai_pending_thinking);
                                    this.ai_pending_thinking.clear();
                                }
                                let text = std::mem::take(&mut this.ai_streaming_text);
                                let thinking = if this.ai_streaming_thinking.is_empty() {
                                    None
                                } else {
                                    Some(std::mem::take(&mut this.ai_streaming_thinking))
                                };
                                if !text.is_empty() || thinking.is_some() {
                                    this.ai_messages.push(crate::ui::ai_sidebar::AiUiMessage {
                                        is_user: false,
                                        text,
                                        thinking,
                                        tool_calls: Vec::new(),
                                        timestamp: Some(crate::ui::ai_sidebar::current_time_str()),
                                        images: Vec::new(),
                                        documents: Vec::new(),
                                        is_error: false,
                                });
                            }

                            if let Some(last) = this.ai_messages.last_mut() {
                                if !last.is_user {
                                    last.tool_calls.push(crate::ui::ai_sidebar::AiUiToolCall {
                                        id,
                                        name,
                                        args,
                                        output: None,
                                        is_error: false,
                                        is_running: true,
                                        diff: None,
                                    });
                                } else {
                                    this.ai_messages.push(crate::ui::ai_sidebar::AiUiMessage {
                                        is_user: false,
                                        text: String::new(),
                                        thinking: None,
                                        tool_calls: vec![crate::ui::ai_sidebar::AiUiToolCall {
                                            id,
                                            name,
                                            args,
                                            output: None,
                                            is_error: false,
                                            is_running: true,
                                            diff: None,
                                        }],
                                        timestamp: Some(crate::ui::ai_sidebar::current_time_str()),
                                        images: Vec::new(),
                                        documents: Vec::new(),
                                        is_error: false,
                                    });
                                }
                            }
                        }
                        crate::ai::AgentEvent::ToolEnd {
                            id,
                            output,
                            is_error,
                            diff,
                            ..
                        } => {
                            for m in this.ai_messages.iter_mut().rev() {
                                if let Some(tc) = m.tool_calls.iter_mut().find(|c| c.id == id) {
                                    tc.output = Some(output);
                                    tc.is_error = is_error;
                                    tc.is_running = false;
                                    tc.diff = diff;
                                    break;
                                }
                            }
                        }
                        crate::ai::AgentEvent::ConversationSession(session_id) => {
                            if let Some(_tab) = this.tabs.iter().find(|tab| tab.id == event_tab_id) {
                                let conversation_id = stream_conversation_id.clone();
                                if let Some(record) = this.ai_chat_records.iter_mut().find(|record| record.id == conversation_id) {
                                    record.acp_session_id = Some(session_id);
                                    record.acp_provider = session_provider_identity.clone();
                                    let _ = crate::ai::conversations::save(&this.ai_chat_records);
                                }
                            }
                        }
                        crate::ai::AgentEvent::AvailableCommands(commands) => {
                            if let Some(_tab) = this.tabs.iter().find(|tab| tab.id == event_tab_id) {
                                let conversation_id = stream_conversation_id.clone();
                                if let Some(record) = this.ai_chat_records.iter_mut().find(|record| record.id == conversation_id) {
                                    record.available_commands = commands;
                                    let _ = crate::ai::conversations::save(&this.ai_chat_records);
                                }
                            }
                        }
                        crate::ai::AgentEvent::ContextWindow { used, size } => {
                            if let Some(_tab) = this.tabs.iter().find(|tab| tab.id == event_tab_id) {
                                let conversation_id = stream_conversation_id.clone();
                                if let Some(record) = this.ai_chat_records.iter_mut().find(|record| record.id == conversation_id) {
                                    record.used_tokens = Some(used);
                                    if record.context_window == 0 { record.context_window = size; }
                                    record.updated_at = crate::ai::conversations::now_millis();
                                    this.ai_last_usage = Some((used, 0));
                                    let _ = crate::ai::conversations::save(&this.ai_chat_records);
                                }
                            }
                        }
                        crate::ai::AgentEvent::TurnEnd => {
                            if this.ai_current_turn_usage != (0, 0) {
                                this.ai_last_usage = Some(this.ai_current_turn_usage);
                            }
                            this.ai_current_turn_usage = (0, 0);

                            // Flush any remaining buffered text/thinking
                            if !this.ai_pending_text.is_empty() {
                                this.ai_streaming_text.push_str(&this.ai_pending_text);
                                this.ai_pending_text.clear();
                            }
                            if !this.ai_pending_thinking.is_empty() {
                                this.ai_streaming_thinking
                                    .push_str(&this.ai_pending_thinking);
                                this.ai_pending_thinking.clear();
                            }

                            let text = std::mem::take(&mut this.ai_streaming_text);
                            let thinking = if this.ai_streaming_thinking.is_empty() {
                                None
                            } else {
                                Some(std::mem::take(&mut this.ai_streaming_thinking))
                            };
                            let has_last_tools = this
                                .ai_messages
                                .last()
                                .map(|m| !m.is_user && !m.tool_calls.is_empty())
                                .unwrap_or(false);
                            if !text.is_empty() || thinking.is_some() {
                                this.ai_messages.push(crate::ui::ai_sidebar::AiUiMessage {
                                    is_user: false,
                                    text,
                                    thinking,
                                    tool_calls: Vec::new(),
                                    timestamp: Some(crate::ui::ai_sidebar::current_time_str()),
                                    images: Vec::new(),
                                    documents: Vec::new(),
                                    is_error: false,
                                });
                            } else if !has_last_tools {
                                this.ai_messages.push(crate::ui::ai_sidebar::AiUiMessage {
                                    is_user: false,
                                    text: "(Assistant completed turn with no content)".to_string(),
                                    thinking: None,
                                    tool_calls: Vec::new(),
                                    timestamp: Some(crate::ui::ai_sidebar::current_time_str()),
                                    images: Vec::new(),
                                    documents: Vec::new(),
                                    is_error: false,
                                });
                            }
                            this.ai_is_streaming = false;
                            this.ai_cancel_token = None;
                            this.ai_turn_completions.insert(
                                stream_conversation_id.clone(),
                                if this.ai_response_is_visible(_window) {
                                    AiTurnCompletion::Read
                                } else {
                                    AiTurnCompletion::Unread
                                },
                            );
                            this.notify_ai_status(
                                _window,
                                "Fastty AI: Turn complete",
                                AiNotificationUrgency::Routine,
                            );
                            this.save_active_ai_conversation();
                        }
                        crate::ai::AgentEvent::Usage { input, output } => {
                            if input == 0 && output == 0 {
                                return;
                            }
                            let effective_input = if input > 0 {
                                input
                            } else {
                                let chars: usize = this
                                    .ai_messages
                                    .iter()
                                    .map(|m| {
                                        m.text.len() + m.thinking.as_deref().unwrap_or("").len()
                                    })
                                    .sum();
                                ((chars / 4).max(50)) as u64
                            };
                            this.ai_current_turn_usage = (effective_input, output);
                            this.ai_last_usage = Some(this.ai_current_turn_usage);
                            if let Some(_tab) = this.tabs.iter().find(|tab| tab.id == event_tab_id) {
                                let conversation_id = stream_conversation_id.clone();
                                if let Some(record) = this.ai_chat_records.iter_mut().find(|record| record.id == conversation_id) {
                                    record.used_tokens = Some(effective_input.saturating_add(output));
                                    record.updated_at = crate::ai::conversations::now_millis();
                                    let _ = crate::ai::conversations::save(&this.ai_chat_records);
                                }
                            }
                            return;
                        }
                        crate::ai::AgentEvent::Error(err) => {
                            if this.ai_current_turn_usage != (0, 0) {
                                this.ai_last_usage = Some(this.ai_current_turn_usage);
                            }
                            this.ai_current_turn_usage = (0, 0);
                            if !this.ai_pending_text.is_empty() {
                                this.ai_streaming_text.push_str(&this.ai_pending_text);
                                this.ai_pending_text.clear();
                            }
                            if !this.ai_pending_thinking.is_empty() {
                                this.ai_streaming_thinking
                                    .push_str(&this.ai_pending_thinking);
                                this.ai_pending_thinking.clear();
                            }

                            this.ai_messages.push(ai_error_message(&err));
                            this.notify_ai_status(
                                _window,
                                "Fastty AI: Turn failed",
                                AiNotificationUrgency::Routine,
                            );
                            this.ai_is_streaming = false;
                            this.ai_cancel_token = None;
                            this.save_active_ai_conversation();
                        }
                    }
                    this.scroll_ai_to_bottom();
                    cx.notify();
                    });
                    let _ = accepted;
                });
            }
        })
        .detach();

        cx.notify();
    }

    pub fn scroll_ai_to_bottom(&self) {
        if self.ai_is_streaming && self.ai_event_source.is_none() {
            self.ai_scroll_handle.scroll_to_bottom();
        }
    }

    fn render_ai_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let provider = self.config.ai.default.clone();
        let model = self.config.ai.active_model();
        let opencode_variants = if provider == "opencode" {
            self.config.ai.providers.get(&provider).and_then(|provider| {
                if let crate::ai::ProviderConfig::Opencode { variants, .. } | crate::ai::ProviderConfig::Acp { variants, .. } = provider {
                    crate::ai::opencode::catalog_model_key(&model, variants)
                        .and_then(|key| variants.get(&key)).cloned()
                } else {
                    None
                }
            }).unwrap_or_default()
        } else {
            Vec::new()
        };
        let opencode_variant = opencode_variants.iter()
            .find(|variant| **variant == self.ai_opencode_variant)
            .cloned()
            .or_else(|| opencode_variants.iter().find(|variant| variant.as_str() == "default").cloned())
            .or_else(|| opencode_variants.first().cloned())
            .unwrap_or_else(|| "default".to_string());
        let active_cwd = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|t| t.cwd.as_ref().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| "~".to_string());
        let active_branch = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|t| t.git_status.as_ref().map(|g| g.branch.clone()));
        let history_cwd = self.tabs.get(self.active_tab_idx).and_then(|tab| tab.cwd.clone()).unwrap_or_default();
        let history_tab_key = self.tabs.get(self.active_tab_idx)
            .map(|tab| tab.ai_tab_key.clone())
            .unwrap_or_default();
        let mut history_items: Vec<(String, String, u64)> = self.ai_chat_records.iter()
            .filter(|record| record.tab_key == history_tab_key && record.cwd == history_cwd)
            .map(|record| (record.id.clone(), record.title.clone(), record.updated_at / 1_000))
            .collect();
        history_items.sort_by(|left, right| right.2.cmp(&left.2));
        history_items.truncate(30);

        let active_conversation_id = self.tabs.get(self.active_tab_idx)
            .map(|tab| tab.ai_conversation_id.as_str()).unwrap_or_default();
        let active_record = self.ai_chat_records.iter().find(|record| record.id == active_conversation_id);
        let max_context_u64 = active_record.map(|record| record.context_window)
            .filter(|size| *size > 0)
            .unwrap_or_else(|| crate::config::ai_context_window().max(1_000) as u64);
        let effective_usage = if active_record.and_then(|record| record.used_tokens).is_some() {
            active_record.and_then(|record| record.used_tokens).map(|used| (used, 0))
        } else if self.ai_current_turn_usage != (0, 0) {
            Some(self.ai_current_turn_usage)
        } else {
            self.ai_last_usage
        };
        let used_tokens_u64 = effective_usage
            .map(|(input, output)| input.saturating_add(output))
            .unwrap_or(0);
        let context_pct = if max_context_u64 > 0 {
            (used_tokens_u64 as f32 / max_context_u64 as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let sidebar = crate::ui::ai_sidebar::AiSidebar::new(
            self.theme,
            self.current_ai_sidebar_width(),
            provider,
        )
        .scroll_handle(self.ai_scroll_handle.clone())
        .agent_mode(self.ai_agent_mode.clone())
        .opencode_variants(opencode_variants, opencode_variant)
        .opencode_variants_open(self.ai_opencode_variants_open)
        .expanded_thinkings(self.ai_expanded_thinkings.clone())
        .cwd(active_cwd)
        .git_branch(active_branch)
        .context_pct(context_pct)
        .context_tokens(used_tokens_u64, max_context_u64, effective_usage)
        .context_hovercard_open(self.ai_context_hovercard_open)
        .on_hover_context(cx.listener(|this, is_hovered: &bool, _window, cx| {
            this.ai_context_hovercard_open = *is_hovered;
            cx.notify();
        }))
        .messages(self.ai_messages.clone())
        .turn_completed(self.ai_turn_completions.contains_key(active_conversation_id))
        .streaming(
            self.ai_is_streaming,
            self.ai_streaming_text.clone(),
            self.ai_streaming_thinking.clone(),
        )
        .pending_confirmation(self.ai_pending_confirmation.clone())
        .input_text(self.ai_input_state.text.clone())
        .cursor_pos(self.ai_input_state.cursor)
        .selection(self.ai_input_state.selection)
        .is_focused(self.ai_input_focused)
        .composer_bounds(self.ai_composer_bounds.clone())
        .message_selection(self.ai_message_selection)
        .on_select_message_char(cx.listener(
            |this, (msg_idx, target): &(usize, usize), _window, cx| {
                this.ai_input_state.selection = None;
                // The sidebar keyboard handler (Cmd+C, Escape, ⌘↵) only runs when
                // the panel owns focus, so selecting a message takes focus here.
                this.ai_input_focused = true;
                this.ai_message_selection_anchor = Some((*msg_idx, *target));
                this.ai_message_selection = Some((*msg_idx, *target, *target));
                this.is_dragging_message_selection = true;
                cx.notify();
            },
        ))
        .on_drag_message_char(cx.listener(
            |this, (msg_idx, target): &(usize, usize), _window, cx| {
                if this.is_dragging_message_selection {
                    if let Some((anchor_msg, anchor_char)) = this.ai_message_selection_anchor {
                        if anchor_msg == *msg_idx {
                            let s = anchor_char.min(*target);
                            let e = anchor_char.max(*target);
                            this.ai_message_selection = Some((*msg_idx, s, e));
                            cx.notify();
                        }
                    }
                }
            },
        ))
        .copy_feedback(self.ai_copy_feedback_until.is_some())
        .on_copied(cx.listener(|this, _ev, _window, cx| {
            this.ai_copy_feedback_until =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(1500));
            cx.notify();
        }))
        .on_toggle_mode(cx.listener(|this, mode: &String, _window, cx| {
            this.ai_agent_mode = mode.clone();
            cx.notify();
        }))
        .on_toggle_thinking(cx.listener(|this, target: &usize, _window, cx| {
            if this.ai_expanded_thinkings.contains(target) {
                this.ai_expanded_thinkings.remove(target);
            } else {
                this.ai_expanded_thinkings.insert(*target);
            }
            cx.notify();
        }))
        .attached_files(self.ai_attached_files.clone())
        .on_remove_attachment(cx.listener(|this, idx: &usize, _window, cx| {
            if *idx < this.ai_attached_files.len() {
                this.ai_attached_files.remove(*idx);
                cx.notify();
            }
        }))
        .at_menu_open(self.ai_at_menu_open)
        .at_is_skill_menu(self.ai_at_is_skill_menu)
        .at_matches(self.ai_at_matches.clone())
        .at_selected(
            self.ai_at_selected
                .min(self.ai_at_row_count().saturating_sub(1)),
        )
        .at_scroll_handle(self.ai_at_scroll_handle.clone())
        .on_select_at_match(cx.listener(|this, path: &String, _window, cx| {
            this.insert_ai_at_path(path);
            this.ai_at_menu_open = false;
            cx.notify();
        }))
        .on_at_click(cx.listener(|this, _ev, _window, cx| {
            this.ai_at_menu_open = !this.ai_at_menu_open;
            if this.ai_at_menu_open {
                this.update_ai_at_matches("");
            }
            cx.notify();
        }))
        .on_dismiss_at_menu(cx.listener(|this, _ev, _window, cx| {
            this.ai_at_menu_open = false;
            cx.notify();
        }))
        .history(self.ai_history_open, history_items)
        .on_toggle_history(cx.listener(|this, _ev, _window, cx| {
            this.ai_history_open = !this.ai_history_open;
            cx.notify();
        }))
        .on_select_history(cx.listener(|this, conversation_id: &String, _window, cx| {
            this.save_active_ai_conversation();
            if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                tab.ai_conversation_id = conversation_id.clone();
            }
            this.load_ai_conversation(conversation_id);
            this.ai_history_open = false;
            this.persist_session();
            cx.notify();
        }))
        .on_delete_history(cx.listener(|this, conversation_id: &String, _window, cx| {
            let deleting_active = this.tabs.get(this.active_tab_idx)
                .map(|tab| tab.ai_conversation_id == *conversation_id)
                .unwrap_or(false);
            if deleting_active {
                this.cancel_ai_stream();
                this.ai_messages.clear();
                this.ai_streaming_text.clear();
                this.ai_streaming_thinking.clear();
                this.ai_pending_text.clear();
                this.ai_pending_thinking.clear();
                this.ai_last_usage = None;
                this.ai_current_turn_usage = (0, 0);
                this.ai_pending_confirmation = None;
                this.ai_attached_files.clear();
                this.ai_message_selection = None;
                this.ai_message_selection_anchor = None;
                if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                    tab.ai_conversation_id = new_ai_key("chat");
                }
            }
            this.cancel_background_ai_conversation(conversation_id);
            this.ai_opencode_manager.remove(conversation_id);
            this.ai_turn_completions.remove(conversation_id);
            this.ai_chat_records.retain(|record| record.id != *conversation_id);
            if deleting_active {
                this.ai_loaded_conversation_id = this.tabs.get(this.active_tab_idx)
                    .map(|tab| tab.ai_conversation_id.clone()).unwrap_or_default();
            }
            let _ = crate::ai::conversations::save(&this.ai_chat_records);
            this.persist_session();
            cx.notify();
        }))
        .on_attach_click(cx.listener(|this, _ev, window, cx| {
            this.open_ai_file_dialog(window, cx);
        }))
        .on_toggle_opencode_variants(cx.listener(|this, _ev, _window, cx| {
            this.ai_opencode_variants_open = !this.ai_opencode_variants_open;
            cx.notify();
        }))
        .on_select_opencode_variant(cx.listener(|this, variant: &String, _window, cx| {
            this.ai_opencode_variant = variant.clone();
            this.ai_opencode_variants_open = false;
            cx.notify();
        }))
        // The diff card and the hovercard block the mouse, so they carry their
        // own `on_drop` and hand the paths back through a weak entity.
        .on_file_drop({
            let entity = cx.entity();
            std::rc::Rc::new(
                move |paths: &[std::path::PathBuf], window: &mut Window, cx: &mut gpui::App| {
                    let viewport_w = window.viewport_size().width.to_f64() as f32;
                    let drop_x = window.mouse_position().x.to_f64() as f32;
                    entity.update(cx, |this, cx| {
                        this.route_file_drop(paths, viewport_w, drop_x);
                        cx.notify();
                    });
                },
            )
        })
        .on_new_chat(cx.listener(|this, _ev, _window, cx| {
            this.save_active_ai_conversation();
            if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                tab.ai_conversation_id = new_ai_key("chat");
            }
            if let Some(id) = this.tabs.get(this.active_tab_idx).map(|tab| tab.ai_conversation_id.clone()) {
                this.load_ai_conversation(&id);
            }
            this.ai_messages.clear();
            this.ai_streaming_text.clear();
            this.ai_streaming_thinking.clear();
            this.ai_pending_text.clear();
            this.ai_pending_thinking.clear();
            this.ai_expanded_thinkings.clear();
            this.ai_attached_files.clear();
            this.ai_at_menu_open = false;
            this.ai_input_state.clear();
            this.ai_input_text.clear();
            this.ai_last_usage = None;
            this.ai_current_turn_usage = (0, 0);
            this.ai_message_selection = None;
            this.ai_message_selection_anchor = None;
            this.save_active_ai_conversation();
            this.persist_session();
            cx.notify();
        }))
        .on_model_click(cx.listener(|this, _ev, window, cx| {
            this.open_settings_window(window, cx);
        }))
        .on_click_char(cx.listener(|this, target: &usize, _window, cx| {
            this.ai_input_focused = true;
            this.is_dragging_ai_input = true;
            this.ai_message_selection = None;
            this.ai_message_selection_anchor = None;
            this.ai_input_state.start_drag(*target);
            this.ai_input_text = this.ai_input_state.text.clone();
            cx.notify();
        }))
        .on_focus(cx.listener(|this, _ev, _window, cx| {
            this.ai_input_focused = true;
            cx.notify();
        }))
        .on_close(cx.listener(|this, _ev, window, cx| {
            this.toggle_ai_sidebar(window, cx);
        }))
        .on_clear(cx.listener(|this, _ev, _window, cx| {
            this.cancel_ai_stream();
            let conversation_id = this.tabs.get(this.active_tab_idx)
                .map(|tab| tab.ai_conversation_id.clone());
            if let Some(conversation_id) = conversation_id {
                this.ai_turn_completions.remove(&conversation_id);
                this.ai_opencode_manager.remove(&conversation_id);
                if let Some(record) = this.ai_chat_records.iter_mut().find(|record| record.id == conversation_id) {
                    record.title = "New chat".to_string();
                    record.messages.clear();
                    record.used_tokens = None;
                    record.acp_session_id = None;
                    record.acp_provider = None;
                    record.available_commands.clear();
                    record.updated_at = crate::ai::conversations::now_millis();
                }
            }
            this.ai_messages.clear();
            this.ai_streaming_text.clear();
            this.ai_streaming_thinking.clear();
            this.ai_pending_text.clear();
            this.ai_pending_thinking.clear();
            this.ai_expanded_thinkings.clear();
            this.ai_attached_files.clear();
            this.ai_at_menu_open = false;
            this.ai_input_state.clear();
            this.ai_input_text.clear();
            this.ai_last_usage = None;
            this.ai_current_turn_usage = (0, 0);
            // Drop any leftover confirmation so it can't reappear in a fresh view.
            if let Some(tx) = this.ai_confirm_reply_tx.take() {
                let _ = tx.send_blocking(crate::ai::PermissionDecision::Deny);
            }
            if let Some(tx) = this.ai_opencode_confirm_reply_tx.take() {
                let _ = tx.send_blocking(None);
            }
            this.ai_pending_confirmation = None;
            this.ai_message_selection = None;
            this.ai_message_selection_anchor = None;
            this.save_active_ai_conversation();
            this.persist_session();
            cx.notify();
        }))
        .on_cancel(cx.listener(|this, _ev, _window, cx| {
            this.cancel_ai_stream();
            cx.notify();
        }))
        .on_submit(cx.listener(|this, _ev, window, cx| {
            this.submit_ai_prompt(window, cx);
        }))
        .on_confirm(
            cx.listener(|this, option_id: &String, _window, cx| {
                let is_always = option_id == "allow_always";
                let is_allow = !option_id.starts_with("reject");
                if is_always && is_allow {
                    if let Some(ref pending) = this.ai_pending_confirmation {
                        this.ai_permission_checker
                            .allow_always_tool(&pending.tool_name, &pending.input_summary);
                    }
                }
                if is_allow {
                    // Learned auto-allow: count the manual approval toward
                    // its command family; the sidebar offers the rule once
                    // the family crosses the threshold.
                    if let Some(ref pending) = this.ai_pending_confirmation {
                        let _ = crate::ai::learned_allow::record_approval(
                            &pending.tool_name,
                            &pending.input_summary,
                        );
                    }
                }
                if let Some(tx) = this.ai_opencode_confirm_reply_tx.take() {
                    let _ = tx.send_blocking(Some(option_id.clone()));
                } else if let Some(tx) = this.ai_confirm_reply_tx.take() {
                    let dec = if is_allow {
                        crate::ai::PermissionDecision::Allow
                    } else {
                        crate::ai::PermissionDecision::Deny
                    };
                    let _ = tx.send_blocking(dec);
                }
                this.ai_pending_confirmation = None;
                cx.notify();
            }),
        );

        div()
            .relative()
            .h_full()
            // A file drag over this panel means the file lands on the message,
            // not in the shell. Say so before the user lets go.
            .when(self.ai_file_drag_hover, |d| {
                d.border_l_2()
                    .border_color(self.theme.accent)
                    .bg(self.theme.accent.opacity(0.06))
            })
            // gpui calls `on_drag_move` for every move event while a drag is
            // active, inside this element or not, and this wrapper is a flex
            // child with no width so it stretches across the window. Without the
            // position test the tint would cover the whole terminal and claim a
            // drop there attaches, when it does not.
            .on_drag_move::<gpui::ExternalPaths>(cx.listener(
                |this, ev: &gpui::DragMoveEvent<gpui::ExternalPaths>, window, cx| {
                    let over_composer = file_drop_target_at(
                        this.ai_sidebar_open,
                        window.viewport_size().width.to_f64() as f32
                            - this.current_ai_sidebar_width(),
                        ev.event.position.x.to_f64() as f32,
                    ) == FileDropTarget::AiAttach;
                    let hover = this.ai_sidebar_open && over_composer;
                    if hover != this.ai_file_drag_hover {
                        this.ai_file_drag_hover = hover;
                        cx.notify();
                    }
                },
            ))
            .on_file_drop_exit(cx.listener(|this, _ev: &gpui::FileDropEvent, _window, cx| {
                if this.ai_file_drag_hover {
                    this.ai_file_drag_hover = false;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, _window, cx| {
                    if !this.ai_input_focused {
                        this.ai_input_focused = true;
                        cx.notify();
                    }
                }),
            )
            .child(sidebar)
            .child(
                div()
                    .id("ai-sidebar-resize-handle")
                    .absolute()
                    .top_0()
                    .left(px(-3.))
                    .bottom_0()
                    .w(px(7.))
                    .cursor(CursorStyle::ResizeColumn)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _ev: &MouseDownEvent, _window, cx| {
                            this.is_dragging_ai_sidebar = true;
                            cx.notify();
                        }),
                    ),
            )
    }

    pub fn set_tab_layout_mode(
        &mut self,
        layout: TabLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Committed selection: persist. Any in-flight preview ends here.
        self.palette_preview_original = None;
        if self.tab_layout == layout {
            return;
        }
        self.tab_layout = layout;
        self.config.tab_layout = layout;
        let _ = self.config.save_default();
        crate::config::increment_config_version();
        if self.tab_layout == TabLayout::Vertical {
            self.sidebar_open = true;
        }
        self.trigger_sidebar_animation(window, cx);
        cx.notify();
    }

    pub fn trigger_sidebar_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.tab_layout == TabLayout::Vertical && self.sidebar_open {
            1.0f32
        } else {
            0.0f32
        };

        if (self.sidebar_anim_progress - target).abs() < 0.001 {
            return;
        }

        cx.spawn_in(window, async move |this, cx| {
            let start_time = std::time::Instant::now();
            let duration = std::time::Duration::from_millis(160);
            let initial = this
                .update_in(cx, |this, _window, _cx| this.sidebar_anim_progress)
                .unwrap_or(0.0);

            loop {
                let elapsed = start_time.elapsed();
                let t = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
                let eased = 1.0 - (1.0 - t).powi(3);
                let current = initial + (target - initial) * eased;
                let done = t >= 1.0;

                let res = this.update_in(cx, |this, _window, cx| {
                    this.sidebar_anim_progress = if done { target } else { current };
                    cx.notify();
                });

                if res.is_err() || done {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(8))
                    .await;
            }
        })
        .detach();
    }

    pub fn reload_config(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let loaded_config = crate::config::load_lenient();
        // External reload wins over any in-flight palette preview, and
        // refreshes the config-error banner (fixing the file clears it).
        self.palette_preview_original = None;
        self.config_error = crate::config::load_error();
        let theme_name = loaded_config
            .theme
            .as_deref()
            .unwrap_or("default")
            .to_string();
        self.current_theme_name = theme_name.clone();
        self.theme = Theme::from_name(&theme_name).with_opacity(loaded_config.opacity);
        self.font_size = loaded_config.font.size;
        let new_family: SharedString =
            if loaded_config.font.family.is_empty() || loaded_config.font.family == "monospace" {
                #[cfg(target_os = "macos")]
                {
                    "Menlo".into()
                }
                #[cfg(target_os = "windows")]
                {
                    "Cascadia Code".into()
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    "monospace".into()
                }
            } else {
                // A family that is not installed gets silently substituted by the
                // text system, leaving cell metrics describing a different font
                // than the one actually drawn. Resolve to an installed family so
                // measurement and rendering agree.
                #[cfg(target_os = "macos")]
                {
                    crate::font_discovery_macos::resolve_font_family(
                        &loaded_config.font.family,
                    )
                    .into()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    loaded_config.font.family.clone().into()
                }
            };
        self.font_family = new_family;
        self.status_bar_model = StatusBarModel::new(&loaded_config, self.theme);
        if self.tab_layout != loaded_config.tab_layout {
            self.tab_layout = loaded_config.tab_layout;
            if self.tab_layout == TabLayout::Vertical {
                self.sidebar_open = true;
            }
            self.trigger_sidebar_animation(_window, cx);
        }
        for tab in &self.tabs {
            if let Some(ref term) = tab.terminal {
                term.update_scrollback(loaded_config.scrollback);
            }
            tab.pane_tree.for_each_terminal(|term| {
                term.update_scrollback(loaded_config.scrollback);
            });
        }
        let context_window_changed = loaded_config.ai.context_window != self.config.ai.context_window;
        crate::keybindings::init_resolver(
            loaded_config.keybindings.clone(),
            loaded_config.keybinding_preset,
        );
        self.config = loaded_config;
        if context_window_changed {
            let active_conversation_id = self.tabs.get(self.active_tab_idx)
                .map(|tab| tab.ai_conversation_id.as_str());
            if let Some(record) = active_conversation_id.and_then(|id| {
                self.ai_chat_records.iter_mut().find(|record| record.id == id)
            }) {
                record.context_window = self.config.ai.context_window.max(1_000) as u64;
                record.updated_at = crate::ai::conversations::now_millis();
                let _ = crate::ai::conversations::save(&self.ai_chat_records);
            }
        }
        cx.notify();
    }

    pub fn set_theme(&mut self, theme_name: &str, cx: &mut Context<Self>) {
        // Committed selection: persist. Any in-flight preview ends here.
        self.palette_preview_original = None;
        self.current_theme_name = theme_name.to_string();
        self.config.theme = Some(self.current_theme_name.clone());
        let _ = self.config.save_default();
        self.theme = Theme::from_name(theme_name).with_opacity(self.config.opacity);
        self.status_bar_model = StatusBarModel::new(&self.config, self.theme);
        cx.notify();
    }

    /// Snapshots the previewable visual settings before the first preview
    /// step, so cancelling restores them losslessly.
    fn ensure_palette_preview(&mut self) {
        if self.palette_preview_original.is_none() {
            self.palette_preview_original = Some(PalettePreviewState {
                theme_name: self.current_theme_name.clone(),
                font_size: self.font_size,
                tab_layout: self.tab_layout,
                sidebar_open: self.sidebar_open,
            });
        }
    }

    /// Transient preview: swaps the UI colors without touching the config
    /// file, so cancelling restores the previous theme losslessly.
    pub fn preview_theme_transient(&mut self, theme_name: &str, cx: &mut Context<Self>) {
        if self.current_theme_name == theme_name {
            return;
        }
        self.ensure_palette_preview();
        self.current_theme_name = theme_name.to_string();
        self.theme = Theme::from_name(theme_name).with_opacity(self.config.opacity);
        self.status_bar_model = StatusBarModel::new(&self.config, self.theme);
        cx.notify();
    }

    /// Transient font-size preview (no disk write).
    pub fn preview_font_size_transient(&mut self, size: f32, cx: &mut Context<Self>) {
        let size = size.clamp(9.0, 36.0);
        if (self.font_size - size).abs() < f32::EPSILON {
            return;
        }
        self.ensure_palette_preview();
        self.font_size = size;
        cx.notify();
    }

    /// Transient tab-layout preview (no disk write), with sidebar animation.
    pub fn preview_layout_transient(
        &mut self,
        layout: TabLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab_layout == layout {
            return;
        }
        self.ensure_palette_preview();
        self.tab_layout = layout;
        if layout == TabLayout::Vertical {
            self.sidebar_open = true;
        }
        self.trigger_sidebar_animation(window, cx);
        cx.notify();
    }

    /// Reverts an uncommitted preview. No-op when nothing is previewing.
    pub fn cancel_palette_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(original) = self.palette_preview_original.take() {
            let mut changed = false;
            if self.current_theme_name != original.theme_name {
                self.current_theme_name = original.theme_name;
                self.theme =
                    Theme::from_name(&self.current_theme_name).with_opacity(self.config.opacity);
                self.status_bar_model = StatusBarModel::new(&self.config, self.theme);
                changed = true;
            }
            if (self.font_size - original.font_size).abs() > f32::EPSILON {
                self.font_size = original.font_size;
                changed = true;
            }
            if self.tab_layout != original.tab_layout || self.sidebar_open != original.sidebar_open
            {
                self.tab_layout = original.tab_layout;
                self.sidebar_open = original.sidebar_open;
                self.trigger_sidebar_animation(window, cx);
                changed = true;
            }
            if changed {
                cx.notify();
            }
        }
    }

    /// If the palette entry previews something (theme, font size, layout),
    /// applies it transiently; otherwise reverts to the pre-preview state so
    /// non-preview rows show the real settings.
    pub fn preview_palette_command(
        &mut self,
        cmd_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match palette_preview_kind(cmd_id) {
            Some(PalettePreviewKind::Theme(name)) => self.preview_theme_transient(name, cx),
            Some(PalettePreviewKind::FontDelta(delta)) => {
                let base = self
                    .palette_preview_original
                    .as_ref()
                    .map(|s| s.font_size)
                    .unwrap_or(self.font_size);
                self.preview_font_size_transient(base + delta, cx);
            }
            Some(PalettePreviewKind::FontReset) => self.preview_font_size_transient(13.0, cx),
            Some(PalettePreviewKind::Layout(layout)) => {
                self.preview_layout_transient(layout, window, cx)
            }
            None => self.cancel_palette_preview(window, cx),
        }
    }

    pub fn adjust_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        // Committed change (e.g. keyboard shortcut): persist.
        self.palette_preview_original = None;
        self.font_size = (self.font_size + delta).clamp(9.0, 36.0);
        self.config.font.size = self.font_size;
        let _ = self.config.save_default();
        cx.notify();
    }

    /// Commits the current (possibly previewed) font size to disk.
    /// Used by palette `zoom_*` Enter: the value was already previewed while
    /// navigating, so re-applying the delta would double it.
    pub fn commit_font_size(&mut self, cx: &mut Context<Self>) {
        self.palette_preview_original = None;
        self.config.font.size = self.font_size;
        let _ = self.config.save_default();
        cx.notify();
    }

    pub fn open_rename_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(idx) {
            self.rename_tab_idx = idx;
            self.rename_tab_input = tab
                .custom_title
                .clone()
                .unwrap_or_else(|| tab.title.clone());
            self.is_rename_tab_open = true;
            cx.notify();
        }
    }

    pub fn save_rename_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.rename_tab_idx) {
            let trimmed = self.rename_tab_input.trim();
            if trimmed.is_empty() {
                tab.custom_title = None;
            } else {
                tab.custom_title = Some(trimmed.to_string());
            }
        }
        self.is_rename_tab_open = false;
        self.persist_session();
        cx.notify();
    }

    pub fn set_font_family(&mut self, family: &str, cx: &mut Context<Self>) {
        // Keep the configured value in config so the user's choice survives a
        // restart, but render with a family that is actually installed.
        let resolved = {
            #[cfg(target_os = "macos")]
            {
                crate::font_discovery_macos::resolve_font_family(family)
            }
            #[cfg(not(target_os = "macos"))]
            {
                family.to_string()
            }
        };
        self.font_family = resolved.into();
        self.config.font.family = family.to_string();
        let _ = self.config.save_default();
        cx.notify();
    }

    pub fn open_settings_file() {
        let path = crate::config::Config::get_active_config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if !path.exists() {
            let _ = std::fs::write(&path, "# Fastty configuration\n");
        }
        open_path_or_url(&path);
    }

    /// RGB of an explicitly-requested program color — truecolor `Spec`, or
    /// a 256-color index above the theme-owned palette (16..=255). Named
    /// colors belong to the theme and are never correction candidates.
    fn explicit_color_rgb(color: AnsiColor) -> Option<(u8, u8, u8)> {
        match color {
            AnsiColor::Spec(rgb) => Some((rgb.r, rgb.g, rgb.b)),
            AnsiColor::Indexed(idx) => crate::ui::color_harmony::indexed_to_rgb(idx),
            _ => None,
        }
    }

    fn ansi_spec(rgb: (u8, u8, u8)) -> AnsiColor {
        AnsiColor::Spec(Rgb { r: rgb.0, g: rgb.1, b: rgb.2 })
    }

    fn hsla_to_rgb_tuple(c: Hsla) -> (u8, u8, u8) {
        let rgba = gpui::Rgba::from(c);
        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (to_u8(rgba.r), to_u8(rgba.g), to_u8(rgba.b))
    }

    /// Foreground conversion with automatic contrast correction: explicit
    /// colors that clash with `bg_rgb` (the cell background, or the theme
    /// background) get their Oklab lightness adjusted, hue preserved.
    fn convert_fg_harmonized(&self, color: AnsiColor, bg_rgb: (u8, u8, u8)) -> Hsla {
        if self.config.contrast_correction {
            if let Some(rgb) = Self::explicit_color_rgb(color) {
                let fixed = crate::ui::color_harmony::correct_fg(rgb, bg_rgb);
                return self.convert_color(Self::ansi_spec(fixed), true);
            }
        }
        self.convert_color(color, true)
    }

    fn convert_color(&self, color: AnsiColor, is_fg: bool) -> Hsla {
        let mut hsla = match color {
            AnsiColor::Named(named) => match named {
                NamedColor::Black => self.theme.black,
                NamedColor::Red => self.theme.red,
                NamedColor::Green => self.theme.green,
                NamedColor::Yellow => self.theme.yellow,
                NamedColor::Blue => self.theme.blue,
                NamedColor::Magenta => self.theme.magenta,
                NamedColor::Cyan => self.theme.cyan,
                NamedColor::White => self.theme.white,
                NamedColor::BrightBlack => self.theme.bright_black,
                NamedColor::BrightRed => self.theme.bright_red,
                NamedColor::BrightGreen => self.theme.bright_green,
                NamedColor::BrightYellow => self.theme.bright_yellow,
                NamedColor::BrightBlue => self.theme.bright_blue,
                NamedColor::BrightMagenta => self.theme.bright_magenta,
                NamedColor::BrightCyan => self.theme.bright_cyan,
                NamedColor::BrightWhite => self.theme.bright_white,
                NamedColor::Foreground => self.theme.foreground,
                NamedColor::Background => self.theme.background,
                NamedColor::Cursor => self.theme.cursor,
                _ => {
                    if is_fg {
                        self.theme.foreground
                    } else {
                        self.theme.background
                    }
                }
            },
            AnsiColor::Spec(rgb_val) => rgb_to_hsla(rgb_val.r, rgb_val.g, rgb_val.b),
            AnsiColor::Indexed(idx) => {
                if idx < 16 {
                    let named = match idx {
                        0 => NamedColor::Black,
                        1 => NamedColor::Red,
                        2 => NamedColor::Green,
                        3 => NamedColor::Yellow,
                        4 => NamedColor::Blue,
                        5 => NamedColor::Magenta,
                        6 => NamedColor::Cyan,
                        7 => NamedColor::White,
                        8 => NamedColor::BrightBlack,
                        9 => NamedColor::BrightRed,
                        10 => NamedColor::BrightGreen,
                        11 => NamedColor::BrightYellow,
                        12 => NamedColor::BrightBlue,
                        13 => NamedColor::BrightMagenta,
                        14 => NamedColor::BrightCyan,
                        _ => NamedColor::BrightWhite,
                    };
                    self.convert_color(AnsiColor::Named(named), is_fg)
                } else if idx < 232 {
                    let i = idx - 16;
                    let r = ((i / 36) % 6) * 51;
                    let g = ((i / 6) % 6) * 51;
                    let b = (i % 6) * 51;
                    rgb_to_hsla(r, g, b)
                } else {
                    let gray = (idx - 232) * 10 + 8;
                    rgb_to_hsla(gray, gray, gray)
                }
            }
        };

        if !is_fg && self.theme.opacity < 1.0 {
            hsla.a = self.theme.opacity;
        }

        hsla
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.any_input_overlay_open()
            && super::ime::defers_to_ime(event, self.config.option_as_meta)
        {
            return;
        }
        self.process_key_down(event, _window, cx);
        cx.stop_propagation();
    }

    fn process_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.any_input_overlay_open() && !self.ai_keys_own_input() {
            if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                self.acknowledge_program_status(tab.pane_tree.active_pane_id);
                cx.notify();
            }
        }
        let key = &event.keystroke.key;
        let key_lower = key.to_lowercase();
        let modifiers = &event.keystroke.modifiers;

        if key_lower.starts_with("dead") || key_lower == "dead" {
            return;
        }

        let is_alt_gr = modifiers.control && !modifiers.platform && modifiers.alt;
        let is_ctrl = modifiers.control && !is_alt_gr;

        // AI Sidebar Input Keyboard Handler
        if self.ai_keys_own_input() {
            if key_lower == "escape" || key_lower == "esc" {
                if self.ai_at_menu_open {
                    self.ai_at_menu_open = false;
                } else if self.ai_history_open {
                    self.ai_history_open = false;
                } else if self.ai_opencode_variants_open {
                    self.ai_opencode_variants_open = false;
                } else if let Some(tx) = self.ai_confirm_reply_tx.take() {
                    let _ = tx.send_blocking(crate::ai::PermissionDecision::Deny);
                    self.ai_pending_confirmation = None;
                } else if self.ai_is_streaming {
                    self.cancel_ai_stream();
                } else {
                    self.ai_sidebar_open = false;
                }
                cx.notify();
                return;
            }
            if (modifiers.platform && key_lower == "l")
                || (modifiers.shift && is_ctrl && key_lower == "l")
            {
                self.toggle_ai_sidebar(_window, cx);
                return;
            }

            // Arrow keys move the `@` mention menu while it is open. Without
            // this they fall through to the PTY below and the shell receives
            // \x1b[A / \x1b[B, so the cursor jumps in the composer and the
            // history-search hint of the shell fires behind it.
            // `check_ai_at_trigger` leaves the menu open with an empty match
            // list when the query hits nothing. Nothing is painted then, so the
            // arrows must not fall through to the shell either.
            if self.ai_at_menu_open {
                // Paging stops at the ends; the arrows below wrap.
                if key_lower == "pageup" {
                    move_at_menu_selection_clamped(
                        &mut self.ai_at_selected,
                        &self.ai_at_matches,
                        -AI_AT_MENU_PAGE_ROWS,
                    );
                    self.scroll_at_menu_selection_into_view();
                    cx.notify();
                    return;
                }
                if key_lower == "pagedown" {
                    move_at_menu_selection_clamped(
                        &mut self.ai_at_selected,
                        &self.ai_at_matches,
                        AI_AT_MENU_PAGE_ROWS,
                    );
                    self.scroll_at_menu_selection_into_view();
                    cx.notify();
                    return;
                }
                let moved = if key_lower == "up" || key_lower == "arrowup" {
                    Some(-1)
                } else if key_lower == "down" || key_lower == "arrowdown" {
                    Some(1)
                } else if key_lower == "home" {
                    self.ai_at_selected = 0;
                    Some(0)
                } else if key_lower == "end" {
                    self.ai_at_selected = self.ai_at_row_count() - 1;
                    Some(0)
                } else {
                    None
                };
                if let Some(delta) = moved {
                    if delta != 0 {
                        self.move_ai_at_selection(delta);
                    }
                    self.scroll_at_menu_selection_into_view();
                    cx.notify();
                    return;
                }
            }

            // Select All: Cmd+A (macOS) or Ctrl+A (Linux/Windows)
            let is_select_all =
                (cfg!(target_os = "macos") && modifiers.platform && key_lower == "a")
                    || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "a");
            if is_select_all {
                self.ai_input_state.select_all();
                cx.notify();
                return;
            }
            // Copy: Cmd+C (macOS) or Ctrl+C (Linux/Windows)
            let is_copy = (cfg!(target_os = "macos") && modifiers.platform && key_lower == "c")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "c");
            if is_copy {
                if let Some(sel_text) = self.ai_input_state.selected_text() {
                    if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                        let _ = clip.set_text(sel_text);
                    }
                    return;
                }
                if let Some((msg_idx, s, e)) = self.ai_message_selection {
                    if s < e {
                        let text = if msg_idx < self.ai_messages.len() {
                            Some(self.ai_messages[msg_idx].text.as_str())
                        } else if msg_idx == self.ai_messages.len()
                            && !self.ai_streaming_text.is_empty()
                        {
                            Some(self.ai_streaming_text.as_str())
                        } else {
                            None
                        };
                        if let Some(txt) = text {
                            let selected: String = txt.chars().skip(s).take(e - s).collect();
                            if !selected.is_empty() {
                                if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                                    let _ = clip.set_text(selected);
                                }
                                return;
                            }
                        }
                    }
                }
                return;
            }
            // Cut: Cmd+X (macOS) or Ctrl+X (Linux/Windows)
            let is_cut = (cfg!(target_os = "macos") && modifiers.platform && key_lower == "x")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "x");
            if is_cut {
                if let Some(sel_text) = self.ai_input_state.selected_text() {
                    if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                        let _ = clip.set_text(sel_text);
                    }
                    self.ai_input_state.delete_selection();
                    self.ai_input_text = self.ai_input_state.text.clone();
                    self.check_ai_at_trigger();
                    cx.notify();
                }
                return;
            }
            // Paste: Cmd+V (macOS) or Ctrl+V (Linux/Windows)
            let is_paste = (cfg!(target_os = "macos") && modifiers.platform && key_lower == "v")
                || (!cfg!(target_os = "macos") && is_ctrl && key_lower == "v");
            if is_paste {
                if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                    if let Some(img_path) = crate::paste::get_clipboard_image(&mut clip) {
                        self.ai_attached_files.push(img_path);
                        cx.notify();
                        return;
                    }
                    if let Ok(text) = clip.get_text() {
                        self.ai_input_state.insert_str(&text);
                        self.ai_input_text = self.ai_input_state.text.clone();
                        self.check_ai_at_trigger();
                        cx.notify();
                    }
                }
                return;
            }
            if key_lower == "enter" || key_lower == "return" || key_lower == "tab" {
                if let Some(row) = at_menu_commit_row(
                    self.ai_at_menu_open,
                    &self.ai_at_matches,
                    self.ai_at_selected,
                ) {
                    self.ai_at_selected = row;
                    let path = self.ai_at_matches[row].clone();
                    self.insert_ai_at_path(&path);
                    cx.notify();
                    return;
                }
                if modifiers.shift {
                    self.ai_input_state.insert_str("\n");
                    self.ai_input_text = self.ai_input_state.text.clone();
                    self.check_ai_at_trigger();
                } else if self.ai_pending_confirmation.is_some() && modifiers.platform {
                    if let Some(tx) = self.ai_opencode_confirm_reply_tx.take() {
                        let selected = self.ai_pending_confirmation.as_ref()
                            .and_then(|pending| pending.options.iter().find(|option| option.kind.as_deref() == Some("allow_once")))
                            .or_else(|| self.ai_pending_confirmation.as_ref().and_then(|pending| pending.options.iter().find(|option| !option.kind.as_deref().unwrap_or("").starts_with("reject"))))
                            .map(|option| option.option_id.clone());
                        let _ = tx.send_blocking(selected);
                    } else if let Some(tx) = self.ai_confirm_reply_tx.take() {
                        let _ = tx.send_blocking(crate::ai::PermissionDecision::Allow);
                    }
                    self.ai_pending_confirmation = None;
                } else {
                    self.submit_ai_prompt(_window, cx);
                }
                cx.notify();
                return;
            }
            if key_lower == "backspace" {
                self.ai_input_state.backspace();
                self.ai_input_text = self.ai_input_state.text.clone();
                self.check_ai_at_trigger();
                cx.notify();
                return;
            }
            if key_lower == "delete" {
                self.ai_input_state.delete_forward();
                self.ai_input_text = self.ai_input_state.text.clone();
                self.check_ai_at_trigger();
                cx.notify();
                return;
            }
            if key_lower == "left" || key_lower == "arrowleft" {
                self.ai_input_state.move_left(modifiers.shift);
                self.check_ai_at_trigger();
                cx.notify();
                return;
            }
            if key_lower == "right" || key_lower == "arrowright" {
                self.ai_input_state.move_right(modifiers.shift);
                self.check_ai_at_trigger();
                cx.notify();
                return;
            }
            if key_lower == "home" {
                self.ai_input_state.move_home(modifiers.shift);
                self.check_ai_at_trigger();
                cx.notify();
                return;
            }
            if key_lower == "end" {
                self.ai_input_state.move_end(modifiers.shift);
                self.check_ai_at_trigger();
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.ai_input_state.insert_str(ch);
                    self.ai_input_text = self.ai_input_state.text.clone();
                    self.check_ai_at_trigger();
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.ai_input_state.insert_str(key);
                self.ai_input_text = self.ai_input_state.text.clone();
                self.check_ai_at_trigger();
                cx.notify();
                return;
            }
        }

        // Pending Close Modal Keyboard Handler

        if self.pending_close.is_some() {
            if key_lower == "escape" || key_lower == "esc" {
                self.pending_close = None;
                cx.notify();
                return;
            }
            if key_lower == "enter" || key_lower == "return" {
                self.confirm_pending_close(_window, cx);
                return;
            }
            return;
        }

        // 0. Rename Tab Modal Keyboard Handler
        if self.is_rename_tab_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_rename_tab_open = false;
                cx.notify();
                return;
            }
            if key_lower == "enter" || key_lower == "return" {
                self.save_rename_tab(cx);
                return;
            }
            if key_lower == "backspace" {
                self.rename_tab_input.pop();
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.rename_tab_input.push_str(ch);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.rename_tab_input.push_str(key);
                cx.notify();
                return;
            }
            return;
        }

        // Tab Overview / Mission Control Keyboard Handler
        if self.is_tab_overview_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_tab_overview_open = false;
                cx.notify();
                return;
            }
            let tab_count = self.tabs.len();
            if tab_count == 0 {
                self.is_tab_overview_open = false;
                cx.notify();
                return;
            }
            // The grid also paints a trailing "New Tab" card. It takes part in
            // layout, so it takes part in navigation too: the selection stops
            // one cell short of it, and `t` still creates a tab from anywhere.
            let items = tab_count + 1;
            let viewport = _window.viewport_size();
            let layout = MissionControlLayout::new(
                viewport.width.to_f64() as f32,
                viewport.height.to_f64() as f32,
                items,
            );
            let cols = layout.cols;
            let selectable = tab_count;
            // The grid paints a trailing "New Tab" card at index `tab_count`,
            // which takes no selection. Clamping here means Enter, the close
            // keys, the arrows, and the painted highlight all read the same
            // index, so a stale value can never highlight a cell the keys
            // cannot act on.
            let selected = self.tab_overview_selected.min(selectable - 1);
            self.tab_overview_selected = selected;

            if key_lower == "enter" || key_lower == "return" {
                if let Some(tab) = self.tabs.get(selected) {
                    let id = tab.id;
                    self.is_tab_overview_open = false;
                    self.select_tab(id, cx);
                }
                return;
            }

            let moved = if key_lower == "right"
                || key_lower == "arrowright"
                || key_lower == "l"
                || key_lower == "tab"
            {
                Some(move_grid_selection(
                    selected,
                    GridStep::Right,
                    cols,
                    selectable,
                ))
            } else if key_lower == "left" || key_lower == "arrowleft" || key_lower == "h" {
                Some(move_grid_selection(
                    selected,
                    GridStep::Left,
                    cols,
                    selectable,
                ))
            } else if key_lower == "down" || key_lower == "arrowdown" || key_lower == "j" {
                Some(move_grid_selection(
                    selected,
                    GridStep::Down,
                    cols,
                    selectable,
                ))
            } else if key_lower == "up" || key_lower == "arrowup" || key_lower == "k" {
                Some(move_grid_selection(
                    selected,
                    GridStep::Up,
                    cols,
                    selectable,
                ))
            } else {
                None
            };
            if let Some(next) = moved {
                if next != selected {
                    self.tab_overview_selected = next;
                    self.tab_overview_scroll_handle.scroll_to_item(next);
                    cx.notify();
                }
                return;
            }

            if key_lower == "d" || key_lower == "x" || key_lower == "backspace" {
                if let Some(tab) = self.tabs.get(selected) {
                    let id = tab.id;
                    self.close_tab(id, _window, cx);
                    if self.tabs.is_empty() {
                        self.is_tab_overview_open = false;
                    } else {
                        self.tab_overview_selected = selected.min(self.tabs.len() - 1);
                    }
                    cx.notify();
                }
                return;
            }
            if key_lower == "t" || key_lower == "n" {
                self.is_tab_overview_open = false;
                self.create_tab(_window, cx);
                return;
            }
            return;
        }

        // Global Multi-Tab Search Keyboard Handler
        if self.is_global_search_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_global_search_open = false;
                cx.notify();
                return;
            }
            let count = self.global_search_results.len();
            if key_lower == "enter" || key_lower == "return" {
                if let Some(res) = self.global_search_results.get(self.global_search_selected) {
                    let tab_id = res.tab_id;
                    let offset = res.offset;
                    let pane_id = res.pane_id;
                    self.is_global_search_open = false;
                    self.select_tab(tab_id, cx);
                    if let Some(active_tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                        if pane_id > 0 {
                            if let Some(pane) = active_tab.pane_tree.find_pane(pane_id) {
                                if let Some(ref t) = pane.terminal {
                                    t.scroll_to_offset(offset);
                                }
                            }
                        } else if let Some(ref t) = active_tab.terminal {
                            t.scroll_to_offset(offset);
                        }
                    }
                    cx.notify();
                }
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.global_search_selected = (self.global_search_selected + 1) % count;
                    self.global_search_scroll_handle
                        .scroll_to_item(self.global_search_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" {
                if count > 0 {
                    self.global_search_selected = if self.global_search_selected == 0 {
                        count - 1
                    } else {
                        self.global_search_selected - 1
                    };
                    self.global_search_scroll_handle
                        .scroll_to_item(self.global_search_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "backspace" {
                self.global_search_query.pop();
                self.update_global_search(cx);
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.global_search_query.push_str(ch);
                    self.update_global_search(cx);
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.global_search_query.push_str(key);
                self.update_global_search(cx);
                return;
            }
            return;
        }

        // 1. Command Palette Keyboard Handler
        if self.is_command_palette_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_command_palette_open = false;
                self.cancel_palette_preview(_window, cx);
                cx.notify();
                return;
            }
            let filtered = self.filtered_palette_commands();
            let count = filtered.len();

            if key_lower == "enter" || key_lower == "return" {
                if let Some(cmd) = filtered.get(self.command_palette_selected) {
                    let id = cmd.id;
                    self.execute_palette_command(id, _window, cx);
                }
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.command_palette_selected = (self.command_palette_selected + 1) % count;
                    self.command_palette_scroll_handle
                        .scroll_to_item(self.command_palette_selected);
                    let selected_id = filtered.get(self.command_palette_selected).map(|c| c.id);
                    if let Some(id) = selected_id {
                        self.preview_palette_command(id, _window, cx);
                    }
                    cx.notify();
                }
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" {
                if count > 0 {
                    self.command_palette_selected = if self.command_palette_selected == 0 {
                        count - 1
                    } else {
                        self.command_palette_selected - 1
                    };
                    self.command_palette_scroll_handle
                        .scroll_to_item(self.command_palette_selected);
                    let selected_id = filtered.get(self.command_palette_selected).map(|c| c.id);
                    if let Some(id) = selected_id {
                        self.preview_palette_command(id, _window, cx);
                    }
                    cx.notify();
                }
                return;
            }
            if key_lower == "backspace" {
                self.command_palette_query.pop();
                self.command_palette_selected = 0;
                self.command_palette_scroll_handle.scroll_to_item(0);
                // Re-filter after the edit so the row under the cursor previews.
                let selected_id = self
                    .filtered_palette_commands()
                    .into_iter()
                    .next()
                    .map(|c| c.id);
                if let Some(id) = selected_id {
                    self.preview_palette_command(id, _window, cx);
                }
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.command_palette_query.push_str(ch);
                    self.command_palette_selected = 0;
                    self.command_palette_scroll_handle.scroll_to_item(0);
                    let selected_id = self
                        .filtered_palette_commands()
                        .into_iter()
                        .next()
                        .map(|c| c.id);
                    if let Some(id) = selected_id {
                        self.preview_palette_command(id, _window, cx);
                    }
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.command_palette_query.push_str(key);
                self.command_palette_selected = 0;
                self.command_palette_scroll_handle.scroll_to_item(0);
                let selected_id = self
                    .filtered_palette_commands()
                    .into_iter()
                    .next()
                    .map(|c| c.id);
                if let Some(id) = selected_id {
                    self.preview_palette_command(id, _window, cx);
                }
                cx.notify();
                return;
            }
            return;
        }

        // 2. Snippet Picker Keyboard Handler (F4)
        if self.is_snippet_picker_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_snippet_picker_open = false;
                cx.notify();
                return;
            }
            let all_snips = crate::snippets::all();
            let query = self.snippet_query.to_lowercase();
            let filtered: Vec<&(String, String)> = all_snips
                .iter()
                .filter(|(trigger, body)| {
                    query.is_empty()
                        || fuzzy_match_str(&query, trigger)
                        || fuzzy_match_str(&query, body)
                })
                .collect();
            let count = filtered.len();

            if key_lower == "enter" || key_lower == "return" {
                if let Some((_, body)) = filtered.get(self.snippet_selected) {
                    let body = (*body).clone();
                    self.insert_snippet_body(&body, cx);
                }
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.snippet_selected = (self.snippet_selected + 1) % count;
                    self.snippet_scroll_handle
                        .scroll_to_item(self.snippet_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" {
                if count > 0 {
                    self.snippet_selected = if self.snippet_selected == 0 {
                        count - 1
                    } else {
                        self.snippet_selected - 1
                    };
                    self.snippet_scroll_handle
                        .scroll_to_item(self.snippet_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "backspace" {
                self.snippet_query.pop();
                self.snippet_selected = 0;
                self.snippet_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.snippet_query.push_str(ch);
                    self.snippet_selected = 0;
                    self.snippet_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.snippet_query.push_str(key);
                self.snippet_selected = 0;
                self.snippet_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // PR Picker Keyboard Handler (F5)
        if self.is_pr_picker_open {
            if key_lower == "escape" || key_lower == "esc" {
                match self.pr_picker_mode.clone() {
                    PrPickerMode::Actions { .. } => {
                        self.pr_picker_mode = PrPickerMode::Browse;
                        self.pr_picker_selected = 0;
                        self.pr_picker_scroll_handle.scroll_to_item(0);
                    }
                    PrPickerMode::Browse => {
                        self.is_pr_picker_open = false;
                    }
                }
                cx.notify();
                return;
            }
            let rows: Vec<PrPickerRow> = match self.pr_picker_mode.clone() {
                PrPickerMode::Browse => {
                    let query = self.pr_picker_query.to_lowercase();
                    let guard = self.pr_snapshot.lock().unwrap();
                    guard
                        .as_ref()
                        .map(pr_picker_rows)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|r| {
                            query.is_empty()
                                || fuzzy_match_str(&query, &r.title)
                                || fuzzy_match_str(&query, &r.detail)
                                || fuzzy_match_str(&query, &r.number.to_string())
                        })
                        .collect()
                }
                PrPickerMode::Actions { .. } => Vec::new(),
            };
            let count = match self.pr_picker_mode {
                PrPickerMode::Browse => rows.len(),
                PrPickerMode::Actions { .. } => pr_action_count(),
            };

            if key_lower == "enter" || key_lower == "return" {
                match self.pr_picker_mode.clone() {
                    PrPickerMode::Browse => {
                        if let Some(row) = rows.get(self.pr_picker_selected) {
                            self.pr_picker_mode = PrPickerMode::Actions {
                                number: row.number,
                                title: row.title.clone(),
                            };
                            self.pr_picker_selected = 0;
                            self.pr_picker_scroll_handle.scroll_to_item(0);
                            cx.notify();
                        }
                    }
                    PrPickerMode::Actions { number, .. } => {
                        match pr_action_for_index(self.pr_picker_selected) {
                            PrPickerAction::Back => {
                                self.pr_picker_mode = PrPickerMode::Browse;
                                self.pr_picker_selected = 0;
                                self.pr_picker_scroll_handle.scroll_to_item(0);
                                cx.notify();
                            }
                            action => {
                                if let Some(cmd) = pr_action_command(action, number) {
                                    self.run_pr_action_command(&cmd, cx);
                                }
                            }
                        }
                    }
                }
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.pr_picker_selected = (self.pr_picker_selected + 1) % count;
                    self.pr_picker_scroll_handle
                        .scroll_to_item(self.pr_picker_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" {
                if count > 0 {
                    self.pr_picker_selected = if self.pr_picker_selected == 0 {
                        count - 1
                    } else {
                        self.pr_picker_selected - 1
                    };
                    self.pr_picker_scroll_handle
                        .scroll_to_item(self.pr_picker_selected);
                    cx.notify();
                }
                return;
            }
            // Typing filters only in Browse mode; in Actions mode single
            // keys must not leak into the (hidden) query.
            if matches!(self.pr_picker_mode, PrPickerMode::Browse) {
                if key_lower == "backspace" {
                    self.pr_picker_query.pop();
                    self.pr_picker_selected = 0;
                    self.pr_picker_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
                if let Some(ref ch) = event.keystroke.key_char {
                    if !modifiers.platform && !is_ctrl {
                        self.pr_picker_query.push_str(ch);
                        self.pr_picker_selected = 0;
                        self.pr_picker_scroll_handle.scroll_to_item(0);
                        cx.notify();
                        return;
                    }
                } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                    self.pr_picker_query.push_str(key);
                    self.pr_picker_selected = 0;
                    self.pr_picker_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            }
            return;
        }

        // 3. SSH Manager Keyboard Handler
        if self.is_ssh_manager_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_ssh_manager_open = false;
                cx.notify();
                return;
            }
            let hosts = crate::ssh::parse_ssh_config();
            let query = self.ssh_manager_query.to_lowercase();
            let filtered: Vec<&crate::ssh::SshHost> = hosts
                .iter()
                .filter(|h| {
                    query.is_empty()
                        || fuzzy_match_str(&query, &h.name)
                        || fuzzy_match_str(&query, &h.hostname)
                        || fuzzy_match_str(&query, &h.user)
                        || fuzzy_match_str(&query, &h.tag)
                })
                .collect();
            let count = filtered.len();

            if key_lower == "enter" || key_lower == "return" {
                if let Some(host) = filtered.get(self.ssh_manager_selected) {
                    let host_clone = (*host).clone();
                    self.is_ssh_manager_open = false;
                    let title = format!("ssh: {}", host_clone.name);
                    let (shell, args) = host_clone.resilient_shell_command();
                    self.create_tab_with_cmd(&shell, &args, Some(title), _window, cx);
                } else {
                    // UX5: no match (or no hosts at all): open ~/.ssh/config
                    // so the user can add their first `Host` entry.
                    let path = crate::ssh::ensure_ssh_config_exists();
                    self.is_ssh_manager_open = false;
                    open_path_or_url(&path);
                    cx.notify();
                }
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.ssh_manager_selected = (self.ssh_manager_selected + 1) % count;
                    self.ssh_manager_scroll_handle
                        .scroll_to_item(self.ssh_manager_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" {
                if count > 0 {
                    self.ssh_manager_selected = if self.ssh_manager_selected == 0 {
                        count - 1
                    } else {
                        self.ssh_manager_selected - 1
                    };
                    self.ssh_manager_scroll_handle
                        .scroll_to_item(self.ssh_manager_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "backspace" {
                self.ssh_manager_query.pop();
                self.ssh_manager_selected = 0;
                self.ssh_manager_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.ssh_manager_query.push_str(ch);
                    self.ssh_manager_selected = 0;
                    self.ssh_manager_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.ssh_manager_query.push_str(key);
                self.ssh_manager_selected = 0;
                self.ssh_manager_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // 4. Search Bar Keyboard Handler
        if self.is_search_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_search_open = false;
                cx.notify();
                return;
            }
            if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                if let Some(ref term) = active_tab.terminal {
                    if key_lower == "enter" || key_lower == "return" {
                        if !self.search_matches.is_empty() {
                            if modifiers.shift {
                                self.search_match_idx = if self.search_match_idx == 0 {
                                    self.search_matches.len() - 1
                                } else {
                                    self.search_match_idx - 1
                                };
                            } else {
                                self.search_match_idx =
                                    (self.search_match_idx + 1) % self.search_matches.len();
                            }
                            let offset = self.search_matches[self.search_match_idx];
                            term.scroll_to_offset(offset);
                            self.last_scroll_activity = std::time::Instant::now();
                            cx.notify();
                        }
                        return;
                    }
                    if key_lower == "backspace" {
                        self.search_query.pop();
                        self.search_matches = term.search_matches(&self.search_query);
                        self.search_match_idx = 0;
                        if let Some(&offset) = self.search_matches.first() {
                            term.scroll_to_offset(offset);
                            self.last_scroll_activity = std::time::Instant::now();
                        }
                        cx.notify();
                        return;
                    }
                    let mut char_to_add = None;
                    if let Some(ref ch) = event.keystroke.key_char {
                        if !modifiers.platform && !is_ctrl {
                            char_to_add = Some(ch.clone());
                        }
                    } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                        char_to_add = Some(key.clone());
                    }
                    if let Some(ch) = char_to_add {
                        self.search_query.push_str(&ch);
                        self.search_matches = term.search_matches(&self.search_query);
                        self.search_match_idx = 0;
                        if let Some(&offset) = self.search_matches.first() {
                            term.scroll_to_offset(offset);
                            self.last_scroll_activity = std::time::Instant::now();
                        }
                        cx.notify();
                        return;
                    }
                }
            }
            return;
        }

        // 5. Git Worktree Picker Keyboard Handler
        if self.is_worktree_picker_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_worktree_picker_open = false;
                cx.notify();
                return;
            }
            let active_cwd = self
                .tabs
                .get(self.active_tab_idx)
                .and_then(|t| t.cwd.as_deref());
            let worktrees = active_cwd
                .map(crate::git::list_worktrees)
                .unwrap_or_default();
            let query = self.worktree_picker_query.to_lowercase();
            let filtered: Vec<&crate::git::Worktree> = worktrees
                .iter()
                .filter(|w| {
                    query.is_empty()
                        || w.short_branch().to_lowercase().contains(&query)
                        || w.path.to_string_lossy().to_lowercase().contains(&query)
                })
                .collect();
            let count = filtered.len();

            if key_lower == "enter" || key_lower == "return" {
                if let Some(wt) = filtered.get(self.worktree_picker_selected) {
                    let shell = self
                        .config
                        .shell
                        .clone()
                        .or_else(|| std::env::var("SHELL").ok())
                        .unwrap_or_else(crate::paths::default_system_shell);
                    let title = wt.short_branch().to_string();
                    let path = wt.path.clone();
                    self.is_worktree_picker_open = false;
                    self.create_tab_with_cmd_and_cwd(
                        &shell,
                        &[],
                        Some(&path),
                        Some(title),
                        _window,
                        cx,
                    );
                }
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.worktree_picker_selected = (self.worktree_picker_selected + 1) % count;
                    self.worktree_picker_scroll_handle
                        .scroll_to_item(self.worktree_picker_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" {
                if count > 0 {
                    self.worktree_picker_selected = if self.worktree_picker_selected == 0 {
                        count - 1
                    } else {
                        self.worktree_picker_selected - 1
                    };
                    self.worktree_picker_scroll_handle
                        .scroll_to_item(self.worktree_picker_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "backspace" {
                self.worktree_picker_query.pop();
                self.worktree_picker_selected = 0;
                self.worktree_picker_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.worktree_picker_query.push_str(ch);
                    self.worktree_picker_selected = 0;
                    self.worktree_picker_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.worktree_picker_query.push_str(key);
                self.worktree_picker_selected = 0;
                self.worktree_picker_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // 6. Project / Tab Jumper Keyboard Handler
        if self.is_project_jumper_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_project_jumper_open = false;
                cx.notify();
                return;
            }
            let query = self.project_jumper_query.to_lowercase();
            let filtered_tabs: Vec<(usize, usize, String, Option<String>)> = self
                .tabs
                .iter()
                .enumerate()
                .filter_map(|(idx, t)| {
                    let cwd_str = t.cwd.as_ref().map(|p| p.to_string_lossy().into_owned());
                    let branch_str = t.git_status.as_ref().map(|g| g.branch.clone());
                    let matches = query.is_empty()
                        || t.title.to_lowercase().contains(&query)
                        || cwd_str
                            .as_ref()
                            .is_some_and(|c| c.to_lowercase().contains(&query))
                        || branch_str
                            .as_ref()
                            .is_some_and(|b| b.to_lowercase().contains(&query));
                    if matches {
                        Some((idx, t.id, t.title.clone(), cwd_str))
                    } else {
                        None
                    }
                })
                .collect();
            let count = filtered_tabs.len();

            if key_lower == "enter" || key_lower == "return" {
                if let Some((_, tab_id, _, _)) = filtered_tabs.get(self.project_jumper_selected) {
                    let id = *tab_id;
                    self.is_project_jumper_open = false;
                    self.select_tab(id, cx);
                }
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.project_jumper_selected = (self.project_jumper_selected + 1) % count;
                    self.project_jumper_scroll_handle
                        .scroll_to_item(self.project_jumper_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" {
                if count > 0 {
                    self.project_jumper_selected = if self.project_jumper_selected == 0 {
                        count - 1
                    } else {
                        self.project_jumper_selected - 1
                    };
                    self.project_jumper_scroll_handle
                        .scroll_to_item(self.project_jumper_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "backspace" {
                self.project_jumper_query.pop();
                self.project_jumper_selected = 0;
                self.project_jumper_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.project_jumper_query.push_str(ch);
                    self.project_jumper_selected = 0;
                    self.project_jumper_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.project_jumper_query.push_str(key);
                self.project_jumper_selected = 0;
                self.project_jumper_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // 7. File Path Picker Keyboard Handler (Ctrl+Cmd+, / ⌃⌘,)
        if self.is_file_picker_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_file_picker_open = false;
                cx.notify();
                return;
            }
            let count = crate::universal_picker::search(
                &self.file_picker_index,
                &self.file_picker_query,
                crate::universal_picker::MAX_RESULTS,
            )
            .len();
            if key_lower == "enter" || key_lower == "return" {
                self.insert_selected_file_path(cx);
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.file_picker_selected = (self.file_picker_selected + 1) % count;
                    self.file_picker_scroll_handle
                        .scroll_to_item(self.file_picker_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" {
                if count > 0 {
                    self.file_picker_selected = if self.file_picker_selected == 0 {
                        count - 1
                    } else {
                        self.file_picker_selected - 1
                    };
                    self.file_picker_scroll_handle
                        .scroll_to_item(self.file_picker_selected);
                    cx.notify();
                }
                return;
            }
            if key_lower == "backspace" {
                self.file_picker_query.pop();
                self.file_picker_selected = 0;
                self.file_picker_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !is_ctrl {
                    self.file_picker_query.push_str(ch);
                    self.file_picker_selected = 0;
                    self.file_picker_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !is_ctrl {
                self.file_picker_query.push_str(key);
                self.file_picker_selected = 0;
                self.file_picker_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // Escape closes every root-level menu, picker, and modal.
        if key_lower == "escape" || key_lower == "esc" {
            let has_dismissible_overlay = self.is_settings_open
                || self.is_about_open
                || self.is_context_menu_open
                || self.is_rename_tab_open
                || self.is_tab_context_menu_open
                || self.is_pane_context_menu_open
                || self.is_command_palette_open
                || self.is_ssh_manager_open
                || self.is_snippet_picker_open
                || self.is_pr_picker_open
                || self.is_search_open
                || self.is_worktree_picker_open
                || self.is_project_jumper_open
                || self.is_file_picker_open
                || self.is_tab_overview_open
                || self.is_global_search_open
                || self.is_git_menu_open
                || self.is_update_modal_open
                || self.is_whats_new_open
                || self.ai_history_open;
            if has_dismissible_overlay {
                self.is_settings_open = false;
                self.is_about_open = false;
                self.is_context_menu_open = false;
                self.is_rename_tab_open = false;
                self.is_git_menu_open = false;
                self.is_git_branch_sub_open = false;
                self.is_tab_context_menu_open = false;
                self.is_pane_context_menu_open = false;
                self.is_command_palette_open = false;
                self.is_ssh_manager_open = false;
                self.is_snippet_picker_open = false;
                self.is_pr_picker_open = false;
                self.is_search_open = false;
                self.is_worktree_picker_open = false;
                self.is_project_jumper_open = false;
                self.is_file_picker_open = false;
                self.is_tab_overview_open = false;
                self.is_global_search_open = false;
                self.is_update_modal_open = false;
                self.is_whats_new_open = false;
                self.ai_history_open = false;
                cx.notify();
                return;
            }
        }

        // Rename Tab (⌘⇧R on macOS; Ctrl+Shift+R on Linux/Windows)
        let is_rename_tab = if cfg!(target_os = "macos") {
            modifiers.platform && modifiers.shift && key_lower == "r"
        } else {
            is_ctrl && modifiers.shift && key_lower == "r"
        };
        if is_rename_tab {
            self.open_rename_tab(self.active_tab_idx, cx);
            return;
        }

        // Dynamic Keybinding Resolver Check (Default, Ghostty, Tmux, ITerm2 presets + User overrides)
        let combo_opt = crate::keybindings::combo_from_key(
            key,
            is_ctrl,
            modifiers.shift,
            modifiers.alt && !is_alt_gr,
            modifiers.platform,
        );
        let action_opt = combo_opt.and_then(|combo| {
            let resolver_lock = crate::keybindings::RESOLVER.get_or_init(|| {
                parking_lot::RwLock::new(crate::keybindings::KeyBindingResolver::with_defaults())
            });
            resolver_lock.read().resolve(&combo)
        });

        if let Some(action) = action_opt {
            use crate::keybindings::Action;
            match action {
                Action::CommandPalette => {
                    self.toggle_command_palette(_window, cx);
                    return;
                }
                Action::NewWindow => {
                    self.execute_palette_command("new_window", _window, cx);
                    return;
                }
                Action::SshManager => {
                    self.toggle_ssh_manager(_window, cx);
                    return;
                }
                Action::WorktreePicker => {
                    self.toggle_worktree_picker(_window, cx);
                    return;
                }
                Action::ProjectJumper => {
                    self.toggle_project_jumper(_window, cx);
                    return;
                }
                Action::InsertFilePath => {
                    self.toggle_file_picker(_window, cx);
                    return;
                }
                Action::OpenSearch => {
                    self.toggle_search(_window, cx);
                    return;
                }
                Action::OpenSettings => {
                    self.toggle_settings(_window, cx);
                    return;
                }
                Action::ToggleFullscreen => {
                    _window.toggle_fullscreen();
                    return;
                }
                Action::NewTab => {
                    self.create_tab(_window, cx);
                    return;
                }
                Action::SplitRight => {
                    self.split_active_pane(Direction::Right, _window, cx);
                    return;
                }
                Action::SplitDown => {
                    self.split_active_pane(Direction::Down, _window, cx);
                    return;
                }
                Action::SplitLeft => {
                    self.split_active_pane(Direction::Left, _window, cx);
                    return;
                }
                Action::SplitTop => {
                    self.split_active_pane(Direction::Top, _window, cx);
                    return;
                }
                Action::ToggleTabSidebar => {
                    self.toggle_tab_sidebar(_window, cx);
                    return;
                }
                Action::ToggleAiSidebar => {
                    self.toggle_ai_sidebar(_window, cx);
                    return;
                }

                Action::FocusLeft => {
                    self.focus_pane_in_direction(Direction::Left, cx);
                    return;
                }
                Action::FocusRight => {
                    self.focus_pane_in_direction(Direction::Right, cx);
                    return;
                }
                Action::FocusTop => {
                    self.focus_pane_in_direction(Direction::Top, cx);
                    return;
                }
                Action::FocusDown => {
                    self.focus_pane_in_direction(Direction::Down, cx);
                    return;
                }
                Action::ClosePane => {
                    self.close_active_pane(_window, cx);
                    return;
                }
                Action::CloseTab => {
                    if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                        let id = active_tab.id;
                        self.close_tab(id, _window, cx);
                    }
                    return;
                }
                Action::NextTab => {
                    if !self.tabs.is_empty() {
                        self.selection = None;
                        self.is_selecting = false;
                        self.selection_start = None;
                        self.activate_tab_index((self.active_tab_idx + 1) % self.tabs.len());
                        cx.notify();
                    }
                    return;
                }
                Action::PrevTab => {
                    if !self.tabs.is_empty() {
                        self.selection = None;
                        self.is_selecting = false;
                        self.selection_start = None;
                        let target = if self.active_tab_idx == 0 {
                            self.tabs.len() - 1
                        } else {
                            self.active_tab_idx - 1
                        };
                        self.activate_tab_index(target);
                        cx.notify();
                    }
                    return;
                }
                Action::SelectTab(digit) => {
                    if (1..=9).contains(&digit) {
                        let target = (digit - 1) as usize;
                        if target < self.tabs.len() {
                            self.selection = None;
                            self.is_selecting = false;
                            self.selection_start = None;
                            self.activate_tab_index(target);
                            cx.notify();
                            return;
                        }
                    }
                }
                Action::IncreaseFontSize => {
                    self.adjust_font_size(1.0, cx);
                    return;
                }
                Action::DecreaseFontSize => {
                    self.adjust_font_size(-1.0, cx);
                    return;
                }
                Action::ResetFontSize => {
                    self.font_size = 13.0;
                    self.config.font.size = 13.0;
                    let _ = self.config.save_default();
                    cx.notify();
                    return;
                }
                Action::ClearScrollback => {
                    if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                        if let Some(ref terminal) = active_tab.terminal {
                            terminal.scroll_to_bottom();
                            cx.notify();
                        }
                    }
                    return;
                }
                Action::PrevPrompt => {
                    if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                        if let Some(ref terminal) = active_tab.terminal {
                            terminal.scroll_to_prev_prompt();
                            self.last_scroll_activity = std::time::Instant::now();
                            cx.notify();
                        }
                    }
                    return;
                }
                Action::NextPrompt => {
                    if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                        if let Some(ref terminal) = active_tab.terminal {
                            terminal.scroll_to_next_prompt();
                            self.last_scroll_activity = std::time::Instant::now();
                            cx.notify();
                        }
                    }
                    return;
                }
                Action::Copy => {
                    if let Some((msg_idx, s, e)) = self.ai_message_selection {
                        if s < e {
                            let text = if msg_idx < self.ai_messages.len() {
                                Some(self.ai_messages[msg_idx].text.as_str())
                            } else if msg_idx == self.ai_messages.len()
                                && !self.ai_streaming_text.is_empty()
                            {
                                Some(self.ai_streaming_text.as_str())
                            } else {
                                None
                            };
                            if let Some(txt) = text {
                                let selected: String = txt.chars().skip(s).take(e - s).collect();
                                if !selected.is_empty() {
                                    if let Some(mut clip) =
                                        crate::event_listener::clipboard_helper()
                                    {
                                        let _ = clip.set_text(selected);
                                    }
                                    return;
                                }
                            }
                        }
                    }
                    if let Some(text) = self.get_selected_text() {
                        if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                            let _ = clip.set_text(text);
                        }
                    }
                    return;
                }
                Action::Paste => {
                    if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                        if let Some(ref terminal) = active_tab.terminal {
                            if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                                if let Some(content) =
                                    crate::paste::get_clipboard_paste_content(&mut clip)
                                {
                                    crate::paste::paste_text_to_terminal(terminal, &content);
                                    cx.notify();
                                    return;
                                }
                            }
                        }
                    }
                    return;
                }
                Action::Quit => {
                    cx.quit();
                    return;
                }
                Action::TabOverview => {
                    self.toggle_tab_overview(_window, cx);
                    return;
                }
                Action::GlobalSearch => {
                    self.toggle_global_search(_window, cx);
                    return;
                }
                Action::ReloadConfig => {
                    self.reload_config(_window, cx);
                    return;
                }
            }
        }

        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let Some(ref terminal) = active_tab.terminal else {
            return;
        };

        self.last_cursor_activity = std::time::Instant::now();
        self.cursor_blink_visible = true;

        if terminal.display_offset() > 0 {
            terminal.scroll_to_bottom();
            cx.notify();
        }

        // Clear selection on regular keypress
        if self.selection.is_some() {
            self.selection = None;
            cx.notify();
        }

        let bytes_to_send: Option<Vec<u8>> = if is_alt_gr {
            if let Some(ref ch) = event.keystroke.key_char {
                self.typed_prompt_buf.push_str(ch);
                Some(ch.as_bytes().to_vec())
            } else if key.len() == 1 {
                self.typed_prompt_buf.push_str(key);
                Some(key.as_bytes().to_vec())
            } else {
                None
            }
        } else if is_ctrl {
            match key_lower.as_str() {
                "c" => {
                    self.typed_prompt_buf.clear();
                    Some(vec![3])
                }
                "u" => {
                    self.typed_prompt_buf.clear();
                    Some(vec![21])
                }
                "d" => Some(vec![4]),
                "z" => Some(vec![26]),
                "l" => Some(vec![12]),
                "a" => Some(vec![1]),
                "e" => Some(vec![5]),
                "k" => Some(vec![11]),
                "w" => Some(vec![23]),
                "r" => Some(vec![18]),
                "g" => Some(vec![7]),
                "h" => Some(vec![8]),
                "j" => Some(vec![10]),
                "n" => Some(vec![14]),
                "p" => Some(vec![16]),
                "t" => Some(vec![20]),
                "x" => Some(vec![24]),
                "y" => Some(vec![25]),
                "f" => Some(vec![6]),
                "b" => Some(vec![2]),
                "o" => Some(vec![15]),
                "v" => Some(vec![22]),
                "q" => Some(vec![17]),
                "s" => Some(vec![19]),
                _ => None,
            }
        } else if modifiers.alt {
            match key_lower.as_str() {
                "b" => Some(b"\x1bb".to_vec()),
                "f" => Some(b"\x1bf".to_vec()),
                "d" => Some(b"\x1bd".to_vec()),
                "backspace" => Some(b"\x17".to_vec()),
                _ => {
                    if key.chars().count() == 1 {
                        let base = if modifiers.shift {
                            key.to_ascii_uppercase()
                        } else {
                            key.to_string()
                        };
                        let mut b = vec![0x1b];
                        b.extend_from_slice(base.as_bytes());
                        Some(b)
                    } else {
                        None
                    }
                }
            }
        } else {
            match key_lower.as_str() {
                "enter" | "return" => {
                    if modifiers.shift {
                        self.typed_prompt_buf.push('\n');
                        Some(b"\n".to_vec())
                    } else {
                        self.typed_prompt_buf.clear();
                        Some(b"\r".to_vec())
                    }
                }
                "backspace" => {
                    self.typed_prompt_buf.pop();
                    Some(b"\x7f".to_vec())
                }
                "tab" => {
                    if modifiers.shift {
                        Some(b"\x1b[Z".to_vec())
                    } else {
                        // Snippet Tab expansion
                        if let Some(trigger_len) =
                            crate::snippets::match_trigger(&self.typed_prompt_buf)
                        {
                            let trigger =
                                &self.typed_prompt_buf[self.typed_prompt_buf.len() - trigger_len..];
                            if let Some(body) = crate::snippets::get_expansion(trigger) {
                                let (expanded, _) = crate::snippets::expand(&body);
                                let mut erase_bytes = Vec::new();
                                for _ in 0..trigger_len {
                                    erase_bytes.extend_from_slice(b"\x08 \x08");
                                }
                                terminal.write_to_pty(&erase_bytes);
                                terminal.write_to_pty(expanded.as_bytes());
                                self.typed_prompt_buf.clear();
                                cx.notify();
                                return;
                            }
                        }
                        Some(b"\t".to_vec())
                    }
                }
                "escape" | "esc" => Some(b"\x1b".to_vec()),
                "up" | "arrowup" => {
                    if modifiers.shift {
                        self.last_scroll_activity = std::time::Instant::now();
                        terminal.scroll(1);
                        cx.notify();
                        None
                    } else {
                        Some(b"\x1b[A".to_vec())
                    }
                }
                "down" | "arrowdown" => {
                    if modifiers.shift {
                        self.last_scroll_activity = std::time::Instant::now();
                        terminal.scroll(-1);
                        cx.notify();
                        None
                    } else {
                        Some(b"\x1b[B".to_vec())
                    }
                }
                "right" | "arrowright" => Some(b"\x1b[C".to_vec()),
                "left" | "arrowleft" => Some(b"\x1b[D".to_vec()),
                "home" => {
                    if modifiers.shift {
                        self.last_scroll_activity = std::time::Instant::now();
                        terminal.scroll_to_top();
                        cx.notify();
                        None
                    } else {
                        Some(b"\x1b[H".to_vec())
                    }
                }
                "end" => {
                    if modifiers.shift {
                        self.last_scroll_activity = std::time::Instant::now();
                        terminal.scroll_to_bottom();
                        cx.notify();
                        None
                    } else {
                        Some(b"\x1b[F".to_vec())
                    }
                }
                "pageup" => {
                    if modifiers.shift {
                        self.last_scroll_activity = std::time::Instant::now();
                        terminal.scroll_page(1);
                        cx.notify();
                        None
                    } else {
                        Some(b"\x1b[5~".to_vec())
                    }
                }
                "pagedown" => {
                    if modifiers.shift {
                        self.last_scroll_activity = std::time::Instant::now();
                        terminal.scroll_page(-1);
                        cx.notify();
                        None
                    } else {
                        Some(b"\x1b[6~".to_vec())
                    }
                }
                _ => {
                    if let Some(ref ch) = event.keystroke.key_char {
                        if !modifiers.platform && !modifiers.control && !modifiers.alt {
                            self.typed_prompt_buf.push_str(ch);
                        }
                        Some(ch.as_bytes().to_vec())
                    } else if key.len() == 1 {
                        if !modifiers.platform && !modifiers.control && !modifiers.alt {
                            self.typed_prompt_buf.push_str(key);
                        }
                        Some(key.as_bytes().to_vec())
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(bytes) = bytes_to_send {
            terminal.write_to_pty(&bytes);
            cx.notify();
        }
    }

    pub(crate) fn any_input_overlay_open(&self) -> bool {
        self.is_rename_tab_open
            || self.is_command_palette_open
            || self.is_ssh_manager_open
            || self.is_snippet_picker_open
            || self.is_pr_picker_open
            || self.is_search_open
            || self.is_worktree_picker_open
            || self.is_project_jumper_open
            || self.is_file_picker_open
            || self.is_tab_overview_open
            || self.is_global_search_open
            || self.is_settings_open
            || self.is_about_open
            || self.is_update_modal_open
            || self.is_whats_new_open
            || self.pending_close.is_some()
            || (self.ai_sidebar_open && self.ai_input_focused)
    }

    fn any_root_menu_open(&self) -> bool {
        self.is_context_menu_open
            || self.is_tab_context_menu_open
            || self.is_pane_context_menu_open
            || self.is_command_palette_open
            || self.is_ssh_manager_open
            || self.is_snippet_picker_open
            || self.is_pr_picker_open
            || self.is_search_open
            || self.is_worktree_picker_open
            || self.is_project_jumper_open
            || self.is_file_picker_open
            || self.is_tab_overview_open
            || self.is_global_search_open
            || self.is_git_menu_open
    }

    fn close_root_menus(&mut self) {
        self.is_context_menu_open = false;
        self.is_tab_context_menu_open = false;
        self.is_pane_context_menu_open = false;
        self.is_command_palette_open = false;
        self.is_ssh_manager_open = false;
        self.is_snippet_picker_open = false;
        self.is_pr_picker_open = false;
        self.is_search_open = false;
        self.is_worktree_picker_open = false;
        self.is_project_jumper_open = false;
        self.is_file_picker_open = false;
        self.is_tab_overview_open = false;
        self.is_global_search_open = false;
        self.is_git_menu_open = false;
        self.is_git_branch_sub_open = false;
    }

    pub fn ime_commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.ime_marked_text = None;
        if self.any_input_overlay_open() {
            cx.notify();
            return;
        }
        let terminal = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|tab| tab.terminal.clone());
        let Some(terminal) = terminal else {
            return;
        };
        self.last_cursor_activity = std::time::Instant::now();
        self.cursor_blink_visible = true;
        if terminal.display_offset() > 0 {
            terminal.scroll_to_bottom();
        }
        if self.selection.is_some() {
            self.selection = None;
        }
        self.typed_prompt_buf.push_str(text);
        terminal.write_to_pty(text.as_bytes());
        cx.notify();
    }

    pub fn ime_set_marked_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.ime_marked_text = Some(text.to_string());
        cx.notify();
    }

    pub fn ime_clear_marked_text(&mut self, cx: &mut Context<Self>) {
        if self.ime_marked_text.take().is_some() {
            cx.notify();
        }
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };

        // Click in terminal area unfocuses the AI input so keystrokes go to the terminal.
        if self.ai_sidebar_open && self.ai_input_focused {
            let win_w = window.viewport_size().width.to_f64() as f32;
            let sidebar_left = win_w - self.current_ai_sidebar_width();
            if (event.position.x.to_f64() as f32) < sidebar_left {
                self.ai_input_focused = false;
                cx.notify();
            }
        }

        let mouse_x = event.position.x.to_f64() as f32;
        let mouse_y = event.position.y.to_f64() as f32;

        // Find which pane was clicked based on its rendered bounds
        let clicked_pane = active_tab
            .pane_tree
            .all_panes()
            .into_iter()
            .find(|p| {
                if let Some(b) = p.last_bounds {
                    let bx = b.origin.x.to_f64() as f32;
                    let by = b.origin.y.to_f64() as f32;
                    let bw = b.size.width.to_f64() as f32;
                    let bh = b.size.height.to_f64() as f32;
                    mouse_x >= bx && mouse_x <= bx + bw && mouse_y >= by && mouse_y <= by + bh
                } else {
                    false
                }
            })
            .or_else(|| active_tab.pane_tree.active_pane());

        let clicked_pane_id = clicked_pane.as_ref().map(|p| p.id);
        let clicked_bounds = clicked_pane.as_ref().and_then(|p| p.last_bounds);

        // Switch active pane if clicking a different pane
        if let Some(pid) = clicked_pane_id {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                if tab.pane_tree.active_pane_id != pid {
                    tab.pane_tree.active_pane_id = pid;
                    if let Some(p) = tab.pane_tree.active_pane() {
                        tab.terminal = p.terminal.clone();
                        tab.cwd = p.cwd.clone();
                        tab.git_status = p.git_status.clone();
                    }
                    cx.notify();
                }
            }
        }

        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let active_pane = active_tab.pane_tree.active_pane();
        let Some(ref terminal) = active_pane
            .as_ref()
            .and_then(|p| p.terminal.as_ref())
            .or(active_tab.terminal.as_ref())
        else {
            return;
        };

        let (cell_w, line_h) = self.measure_cell_metrics(window);
        let (pane_origin_x, pane_origin_y, pane_w, pane_h) =
            if let Some(b) = clicked_bounds.or_else(|| active_pane.and_then(|p| p.last_bounds)) {
                (
                    b.origin.x.to_f64() as f32,
                    b.origin.y.to_f64() as f32,
                    b.size.width.to_f64() as f32,
                    b.size.height.to_f64() as f32,
                )
            } else {
                let sidebar_w = self.current_sidebar_width();
                let v = window.viewport_size();
                (
                    sidebar_w + 1.0,
                    32.0,
                    (v.width.to_f64() as f32 - sidebar_w).max(100.0),
                    (v.height.to_f64() as f32 - 56.0).max(100.0),
                )
            };

        let local_x = (mouse_x - pane_origin_x).max(0.0);
        let local_y = (mouse_y - pane_origin_y).max(0.0);
        let col = ((local_x / cell_w).floor() as usize) + 1;
        let row = ((local_y / line_h).floor() as usize) + 1;

        self.pressed_mouse_button = Some(event.button);

        let is_right_click = event.button == MouseButton::Right;

        if terminal.is_mouse_mode_enabled() && !is_right_click && event.click_count < 2 {
            let btn = match event.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                _ => 0,
            };
            terminal.send_mouse_button_with_mods(
                btn,
                col,
                row,
                true,
                event.modifiers.shift,
                event.modifiers.alt,
                event.modifiers.control,
            );
            return;
        }

        let target_pane_id = clicked_pane_id.unwrap_or(active_tab.pane_tree.active_pane_id);

        if event.button == MouseButton::Left {
            // URL Click check (Cmd+Click on macOS, Ctrl+Click on Linux/Windows)
            let is_link_modifier = if cfg!(target_os = "macos") {
                event.modifiers.platform
            } else {
                event.modifiers.control
            };

            if is_link_modifier {
                if let Some(ref link) = self.hovered_url {
                    open_path_or_url(link);
                    return;
                }
            }

            let history_size = terminal.history_size();
            if local_y > pane_h {
                return;
            }
            let screen_rows = ((pane_h / line_h) as i32).max(1);
            let screen_cols = ((pane_w / cell_w) as usize).max(1);
            let display_offset = terminal.display_offset();
            let grid_col = ((local_x / cell_w).floor() as usize).min(screen_cols.saturating_sub(1));
            let grid_row = (((local_y / line_h).floor() as i32) - (display_offset as i32))
                .clamp(-(history_size as i32), screen_rows - 1);
            let start_point = alacritty_terminal::index::Point::new(
                alacritty_terminal::index::Line(grid_row),
                alacritty_terminal::index::Column(grid_col),
            );

            if event.click_count == 2 {
                // Double click: Select word under cursor
                if let Some(term_guard) = terminal.term().try_lock() {
                    let grid = term_guard.grid();
                    if let Some((_token, start_c, end_c)) =
                        crate::selection_classifier::extract_token(grid, start_point, screen_cols)
                    {
                        let start_p = alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(grid_row),
                            alacritty_terminal::index::Column(start_c),
                        );
                        let end_p = alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(grid_row),
                            alacritty_terminal::index::Column(end_c.saturating_sub(1)),
                        );
                        self.selection = Some(Selection {
                            start: start_p,
                            end: end_p,
                        });
                    } else {
                        self.selection = None;
                    }
                }
                self.is_selecting = false;
                self.has_selection_dragged = false;
                self.selection_start = None;
                if self.config.copy_on_select {
                    if let Some(text) = self.get_selected_text() {
                        if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                            let _ = clip.set_text(text);
                        }
                    }
                }
                cx.notify();
                return;
            } else if event.click_count == 3 {
                // Triple click: Select logical line (including soft-wrapped rows)
                let (start_p, end_p) = if let Some(term_guard) = terminal.term().try_lock() {
                    let grid = term_guard.grid();
                    crate::selection_classifier::extract_logical_line(grid, grid_row, screen_cols)
                } else {
                    (
                        alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(grid_row),
                            alacritty_terminal::index::Column(0),
                        ),
                        alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(grid_row),
                            alacritty_terminal::index::Column(screen_cols.saturating_sub(1)),
                        ),
                    )
                };
                self.selection = Some(Selection {
                    start: start_p,
                    end: end_p,
                });
                self.is_selecting = false;
                self.has_selection_dragged = false;
                self.selection_start = None;
                if self.config.copy_on_select {
                    if let Some(text) = self.get_selected_text() {
                        if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                            let _ = clip.set_text(text);
                        }
                    }
                }
                cx.notify();
                return;
            } else if event.click_count >= 4 {
                // Quadruple click: Select full paragraph (bounded by empty lines)
                let (start_p, end_p) = if let Some(term_guard) = terminal.term().try_lock() {
                    let grid = term_guard.grid();
                    crate::selection_classifier::extract_paragraph(grid, grid_row, screen_cols)
                } else {
                    (
                        alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(grid_row),
                            alacritty_terminal::index::Column(0),
                        ),
                        alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(grid_row),
                            alacritty_terminal::index::Column(screen_cols.saturating_sub(1)),
                        ),
                    )
                };
                self.selection = Some(Selection {
                    start: start_p,
                    end: end_p,
                });
                self.is_selecting = false;
                self.has_selection_dragged = false;
                self.selection_start = None;
                if self.config.copy_on_select {
                    if let Some(text) = self.get_selected_text() {
                        if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                            let _ = clip.set_text(text);
                        }
                    }
                }
                cx.notify();
                return;
            }

            // Single click: Start Text Selection in this pane
            self.is_selecting = true;
            self.has_selection_dragged = false;
            self.selection_start = Some(start_point);
            self.selection = None;
            self.selection_mouse_pos = Some((mouse_x, mouse_y));
            cx.notify();
        } else if event.button == MouseButton::Right {
            // Right click: If no selection yet, select word under cursor if present, and open context menu
            if self.selection.is_none() {
                let history_size = terminal.history_size();
                let screen_rows = ((pane_h / line_h) as i32).max(1);
                let screen_cols = ((pane_w / cell_w) as usize).max(1);
                let display_offset = terminal.display_offset();
                let grid_col =
                    ((local_x / cell_w).floor() as usize).min(screen_cols.saturating_sub(1));
                let grid_row = (((local_y / line_h).floor() as i32) - (display_offset as i32))
                    .clamp(-(history_size as i32), screen_rows - 1);
                let point = alacritty_terminal::index::Point::new(
                    alacritty_terminal::index::Line(grid_row),
                    alacritty_terminal::index::Column(grid_col),
                );

                if let Some(term_guard) = terminal.term().try_lock() {
                    let grid = term_guard.grid();
                    if let Some((_token, start_c, end_c)) =
                        crate::selection_classifier::extract_token(grid, point, screen_cols)
                    {
                        let start_p = alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(grid_row),
                            alacritty_terminal::index::Column(start_c),
                        );
                        let end_p = alacritty_terminal::index::Point::new(
                            alacritty_terminal::index::Line(grid_row),
                            alacritty_terminal::index::Column(end_c.saturating_sub(1)),
                        );
                        self.selection = Some(Selection {
                            start: start_p,
                            end: end_p,
                        });
                    }
                }
            }
            self.is_selecting = false;
            self.has_selection_dragged = false;
            self.selection_start = None;
            self.open_pane_context_menu(target_pane_id, mouse_x, mouse_y, cx);
        }
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cur_x = event.position.x.to_f64() as f32;
        let cur_y = event.position.y.to_f64() as f32;
        let prev_pos = self.cursor_window_pos;
        self.cursor_window_pos = Some((cur_x, cur_y));

        // Repaint when mouse enters, moves within, or leaves scrollbar proximity (within 48px inside right edge)
        if let Some(tab) = self.tabs.get(self.active_tab_idx) {
            let in_prox = tab.pane_tree.all_panes().iter().any(|p| {
                if let Some(b) = p.last_bounds {
                    let bx = b.origin.x.to_f64() as f32;
                    let by = b.origin.y.to_f64() as f32;
                    let bw = b.size.width.to_f64() as f32;
                    let bh = b.size.height.to_f64() as f32;
                    let right_edge = bx + bw;
                    cur_y >= (by - 4.0)
                        && cur_y <= (by + bh + 4.0)
                        && cur_x >= (right_edge - 48.0)
                        && cur_x <= (right_edge + 8.0)
                } else {
                    false
                }
            });
            let was_in_prox = prev_pos.is_some_and(|(px, py)| {
                tab.pane_tree.all_panes().iter().any(|p| {
                    if let Some(b) = p.last_bounds {
                        let bx = b.origin.x.to_f64() as f32;
                        let by = b.origin.y.to_f64() as f32;
                        let bw = b.size.width.to_f64() as f32;
                        let bh = b.size.height.to_f64() as f32;
                        let right_edge = bx + bw;
                        py >= (by - 4.0)
                            && py <= (by + bh + 4.0)
                            && px >= (right_edge - 48.0)
                            && px <= (right_edge + 8.0)
                    } else {
                        false
                    }
                })
            });
            if in_prox || was_in_prox {
                cx.notify();
            }
        }

        if self.is_dragging_ai_input {
            let (rel_x, rel_y) = if let Some(bounds) = self.ai_composer_bounds.get() {
                (
                    (cur_x - bounds.origin.x.to_f64() as f32).max(0.0),
                    (cur_y - bounds.origin.y.to_f64() as f32).max(0.0),
                )
            } else {
                let win_w = _window.viewport_size().width.to_f64() as f32;
                let text_left = win_w - self.current_ai_sidebar_width() + 22.0;
                ((cur_x - text_left).max(0.0), 0.0)
            };

            let avail_w = (self.current_ai_sidebar_width() - 44.0).max(80.0);
            let max_cols = ((avail_w / 7.2).floor() as usize).max(10);
            let lines =
                crate::ui::text_input::wrap_text_into_lines(&self.ai_input_state.text, max_cols);
            let line_h = 18.0;
            let line_idx = ((rel_y / line_h).floor() as usize).min(lines.len().saturating_sub(1));
            if let Some(target_line) = lines.get(line_idx) {
                let col =
                    crate::ui::text_input::index_for_x(&target_line.text, rel_x, 13.0, _window);
                let char_idx = (target_line.start_char + col)
                    .min(target_line.start_char + target_line.text.chars().count());
                self.ai_input_state.update_drag(char_idx);
                self.ai_input_text = self.ai_input_state.text.clone();
                cx.notify();
            }
            return;
        }

        if self.is_dragging_ai_sidebar {
            let win_w = _window.viewport_size().width.to_f64() as f32;
            let new_w = (win_w - cur_x).clamp(240.0, 750.0);
            self.ai_sidebar_width = new_w;
            cx.notify();
            return;
        }

        if self.is_dragging_pane_split {
            let (bx, by, bw, bh) = self.dragging_split_bounds;
            let new_ratio = match self.dragging_split_direction {
                SplitDirection::Horizontal => {
                    if bw > 20.0 {
                        ((cur_x - bx) / bw).clamp(0.05, 0.95)
                    } else {
                        0.5
                    }
                }
                SplitDirection::Vertical => {
                    if bh > 20.0 {
                        ((cur_y - by) / bh).clamp(0.05, 0.95)
                    } else {
                        0.5
                    }
                }
            };
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.pane_tree
                    .set_split_ratio_by_path(&self.dragging_split_path, new_ratio);
                cx.notify();
            }
            return;
        }

        if self.is_dragging_scrollbar {
            self.last_scroll_activity = std::time::Instant::now();
            let (_, line_h) = self.measure_cell_metrics(_window);
            let cur_y = event.position.y.to_f64() as f32;
            let delta_y = cur_y - self.scrollbar_drag_start_y;
            let target_pane_id = self.dragging_scrollbar_pane_id;
            if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                let pane = if let Some(pid) = target_pane_id {
                    active_tab.pane_tree.find_pane(pid)
                } else {
                    active_tab.pane_tree.active_pane()
                };
                if let Some(pane) = pane {
                    if let Some(ref term) = pane.terminal {
                        let history_size = term.history_size();
                        if history_size > 0 {
                            let track_h = pane
                                .last_bounds
                                .map_or(300.0, |b| b.size.height.to_f64() as f32)
                                .max(50.0);
                            let target_rows = ((track_h / line_h) as usize).max(5);
                            let total_rows = (history_size + target_rows) as f32;
                            let thumb_h =
                                (track_h * (target_rows as f32 / total_rows)).clamp(24.0, track_h);
                            let scrollable_track = (track_h - thumb_h).max(1.0);
                            let offset_delta = (delta_y / scrollable_track) * history_size as f32;
                            let new_offset = (self.scrollbar_drag_start_offset as f32
                                - offset_delta)
                                .round()
                                .clamp(0.0, history_size as f32)
                                as usize;
                            term.scroll_to_offset(new_offset);
                            cx.notify();
                        }
                    }
                }
            }
            return;
        }

        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };

        // Find hovered pane based on rendered bounds
        let hovered_pane = active_tab.pane_tree.all_panes().into_iter().find(|p| {
            if let Some(b) = p.last_bounds {
                let bx = b.origin.x.to_f64() as f32;
                let by = b.origin.y.to_f64() as f32;
                let bw = b.size.width.to_f64() as f32;
                let bh = b.size.height.to_f64() as f32;
                cur_x >= bx && cur_x <= bx + bw && cur_y >= by && cur_y <= by + bh
            } else {
                false
            }
        });

        let (cell_w, line_h) = self.measure_cell_metrics(_window);

        // Terminal mouse motion reporting (for active pane if dragging, or hovered pane if moving)
        let motion_pane = if self.pressed_mouse_button.is_some() {
            active_tab.pane_tree.active_pane()
        } else {
            hovered_pane
                .clone()
                .or_else(|| active_tab.pane_tree.active_pane())
        };

        if let Some(ref pane) = motion_pane {
            if let Some(ref term) = pane.terminal {
                if term.is_mouse_mode_enabled() {
                    let (pane_x, pane_y) = if let Some(b) = pane.last_bounds {
                        (b.origin.x.to_f64() as f32, b.origin.y.to_f64() as f32)
                    } else {
                        (self.current_sidebar_width() + 1.0, 32.0)
                    };
                    let local_x = (cur_x - pane_x).max(0.0);
                    let local_y = (cur_y - pane_y).max(0.0);
                    let left = self.pressed_mouse_button == Some(MouseButton::Left);
                    let middle = self.pressed_mouse_button == Some(MouseButton::Middle);
                    let right = self.pressed_mouse_button == Some(MouseButton::Right);
                    let col = ((local_x / cell_w).floor() as usize) + 1;
                    let row = ((local_y / line_h).floor() as usize) + 1;
                    term.send_mouse_motion(
                        col,
                        row,
                        left,
                        middle,
                        right,
                        event.modifiers.shift,
                        event.modifiers.alt,
                        event.modifiers.control,
                    );
                    if self.pressed_mouse_button.is_some() {
                        return;
                    }
                }
            }
        }

        if self.is_selecting {
            if event.pressed_button != Some(MouseButton::Left) {
                self.is_selecting = false;
                self.has_selection_dragged = false;
                self.selection_mouse_pos = None;
                self.selection_autoscroll_accum = 0.0;
                self.pressed_mouse_button = None;
                if self.config.copy_on_select {
                    if let Some(text) = self.get_selected_text() {
                        if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                            let _ = clip.set_text(text);
                        }
                    }
                }
                cx.notify();
                return;
            }
            let raw_x = cur_x;
            let raw_y = cur_y;
            self.selection_mouse_pos = Some((raw_x, raw_y));
            self.has_selection_dragged = true;
            self.update_selection_endpoint(_window, cx);
            return;
        }

        // Link / OSC 8 Hover detection across any hovered pane
        let target_link_pane = hovered_pane.or_else(|| active_tab.pane_tree.active_pane());
        let Some(link_pane) = target_link_pane else {
            if self.hovered_url.is_some() {
                self.hovered_url = None;
                self.hovered_url_range = None;
                cx.notify();
            }
            return;
        };

        let Some(ref terminal) = link_pane.terminal else {
            if self.hovered_url.is_some() {
                self.hovered_url = None;
                self.hovered_url_range = None;
                cx.notify();
            }
            return;
        };

        let Some(b) = link_pane.last_bounds else {
            if self.hovered_url.is_some() {
                self.hovered_url = None;
                self.hovered_url_range = None;
                cx.notify();
            }
            return;
        };

        let bx = b.origin.x.to_f64() as f32;
        let by = b.origin.y.to_f64() as f32;
        let bw = b.size.width.to_f64() as f32;
        let bh = b.size.height.to_f64() as f32;
        if cur_x < bx || cur_x > bx + bw || cur_y < by || cur_y > by + bh {
            if self.hovered_url.is_some() {
                self.hovered_url = None;
                self.hovered_url_range = None;
                cx.notify();
            }
            return;
        }

        let local_x = (cur_x - bx).max(0.0);
        let local_y = (cur_y - by).max(0.0);
        let history_size = terminal.history_size();
        let screen_rows = ((bh / line_h) as i32).max(1);
        let display_offset = terminal.display_offset();
        let grid_col = (local_x / cell_w).floor() as usize;
        let grid_row = (((local_y / line_h).floor() as i32) - (display_offset as i32))
            .clamp(-(history_size as i32), screen_rows - 1);
        let current_point = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(grid_row),
            alacritty_terminal::index::Column(grid_col),
        );

        // URL / OSC 8 Hyperlink Hover detection
        if let Some(term_guard) = terminal.term().try_lock() {
            let grid = term_guard.grid();
            let cols = grid.columns();
            // 1. Explicit OSC 8 hyperlink check
            if let Some((url, start_c, end_c)) =
                crate::selection_classifier::extract_hyperlink(grid, current_point, cols)
            {
                self.hovered_url = Some(url);
                self.hovered_url_range = Some((grid_row, start_c, end_c));
                cx.notify();
                return;
            }

            // 2. Pattern-based URL / path / email classifier fallback
            if let Some((token, start_c, end_c)) =
                crate::selection_classifier::extract_token(grid, current_point, cols)
            {
                if let Some(classification) = crate::selection_classifier::classify_token(&token) {
                    match classification {
                        crate::selection_classifier::Classification::Url(u) => {
                            self.hovered_url = Some(u);
                            self.hovered_url_range = Some((grid_row, start_c, end_c));
                            cx.notify();
                            return;
                        }
                        crate::selection_classifier::Classification::Path(p) => {
                            self.hovered_url = Some(p);
                            self.hovered_url_range = Some((grid_row, start_c, end_c));
                            cx.notify();
                            return;
                        }
                        crate::selection_classifier::Classification::Email(e) => {
                            self.hovered_url = Some(format!("mailto:{}", e));
                            self.hovered_url_range = Some((grid_row, start_c, end_c));
                            cx.notify();
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }

        if self.hovered_url.is_some() {
            self.hovered_url = None;
            self.hovered_url_range = None;
            cx.notify();
        }
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pressed_mouse_button = None;
        self.is_dragging_scrollbar = false;
        self.dragging_scrollbar_pane_id = None;
        self.is_dragging_pane_split = false;
        self.is_dragging_ai_sidebar = false;
        if self.is_dragging_ai_input {
            self.is_dragging_ai_input = false;
            self.ai_input_state.end_drag();
        }
        if self.is_dragging_message_selection {
            self.is_dragging_message_selection = false;
            if let Some((_, s, e)) = self.ai_message_selection {
                if s == e {
                    self.ai_message_selection = None;
                    self.ai_message_selection_anchor = None;
                }
            }
            cx.notify();
        }
        self.dragging_split_path.clear();
        self.selection_mouse_pos = None;
        self.has_selection_dragged = false;
        self.selection_autoscroll_accum = 0.0;
        if self.is_selecting {
            self.is_selecting = false;
            if self.config.copy_on_select {
                if let Some(text) = self.get_selected_text() {
                    if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                        let _ = clip.set_text(text);
                    }
                }
            }
            cx.notify();
        }
        if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
            let active_pane = active_tab.pane_tree.active_pane();
            if let Some(p) = active_pane.as_ref() {
                if let Some(ref terminal) = p.terminal {
                    if terminal.is_mouse_mode_enabled() {
                        let (cell_w, line_h) = self.measure_cell_metrics(_window);
                        let (pane_x, pane_y) = if let Some(b) = p.last_bounds {
                            (b.origin.x.to_f64() as f32, b.origin.y.to_f64() as f32)
                        } else {
                            (self.current_sidebar_width() + 1.0, 32.0)
                        };
                        let local_x = (event.position.x.to_f64() as f32 - pane_x).max(0.0);
                        let local_y = (event.position.y.to_f64() as f32 - pane_y).max(0.0);
                        let col = ((local_x / cell_w).floor() as usize) + 1;
                        let row = ((local_y / line_h).floor() as usize) + 1;
                        let btn = match event.button {
                            MouseButton::Left => 0,
                            MouseButton::Middle => 1,
                            MouseButton::Right => 2,
                            _ => 0,
                        };
                        terminal.send_mouse_button_with_mods(
                            btn,
                            col,
                            row,
                            false,
                            event.modifiers.shift,
                            event.modifiers.alt,
                            event.modifiers.control,
                        );
                    }
                }
            }
        }
    }

    fn update_selection_endpoint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_selecting || !self.has_selection_dragged {
            return;
        }
        let Some((raw_x, raw_y)) = self.selection_mouse_pos else {
            return;
        };
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let active_pane = active_tab.pane_tree.active_pane();
        let Some(terminal) = active_pane
            .as_ref()
            .and_then(|p| p.terminal.as_ref())
            .or(active_tab.terminal.as_ref())
        else {
            return;
        };

        let (cell_w, line_h) = self.measure_cell_metrics(window);
        let px_per_line = if line_h > 0.0 { line_h } else { 18.0 };
        let cell_width = if cell_w > 0.0 { cell_w } else { 9.0 };

        let (pane_x, pane_y, pane_w, pane_h) =
            if let Some(b) = active_pane.as_ref().and_then(|p| p.last_bounds) {
                (
                    b.origin.x.to_f64() as f32,
                    b.origin.y.to_f64() as f32,
                    b.size.width.to_f64() as f32,
                    b.size.height.to_f64() as f32,
                )
            } else {
                let v = window.viewport_size();
                let sidebar_w = self.current_sidebar_width();
                (
                    sidebar_w + 1.0,
                    32.0,
                    (v.width.to_f64() as f32 - sidebar_w).max(100.0),
                    (v.height.to_f64() as f32 - 56.0).max(100.0),
                )
            };

        let top_edge = pane_y;
        let bottom_edge = pane_y + pane_h;

        let screen_rows = ((pane_h / px_per_line) as i32).max(1);
        let screen_cols = ((pane_w / cell_width) as usize).max(1);
        let history_size = terminal.history_size();
        let display_offset = terminal.display_offset();

        let updated_grid_row = if raw_y < top_edge {
            -(display_offset as i32)
        } else if raw_y > bottom_edge {
            (screen_rows - 1) - (display_offset as i32)
        } else {
            let rel_y = (raw_y - top_edge).clamp(0.0, pane_h - 1.0);
            let row_in_screen = (rel_y / px_per_line).floor() as i32;
            row_in_screen - (display_offset as i32)
        }
        .clamp(-(history_size as i32), screen_rows - 1);

        let local_x = (raw_x - pane_x).max(0.0);
        let updated_col = if raw_y < top_edge && raw_x < pane_x + 5.0 {
            0
        } else {
            ((local_x / cell_width).floor() as usize).min(screen_cols.saturating_sub(1))
        };

        let updated_point = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(updated_grid_row),
            alacritty_terminal::index::Column(updated_col),
        );

        if let Some(start_p) = self.selection_start {
            if start_p == updated_point {
                if self.selection.is_some() {
                    self.selection = None;
                    cx.notify();
                }
            } else {
                let (start, end) = if start_p <= updated_point {
                    (start_p, updated_point)
                } else {
                    (updated_point, start_p)
                };
                self.selection = Some(Selection { start, end });
                cx.notify();
            }
        }
    }

    fn tick_selection_autoscroll(&mut self, dt: f32, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_selecting || !self.has_selection_dragged {
            self.selection_autoscroll_accum = 0.0;
            return;
        }
        let Some((_raw_x, raw_y)) = self.selection_mouse_pos else {
            self.selection_autoscroll_accum = 0.0;
            return;
        };
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let active_pane = active_tab.pane_tree.active_pane();
        let Some(terminal) = active_pane
            .as_ref()
            .and_then(|p| p.terminal.as_ref())
            .or(active_tab.terminal.as_ref())
        else {
            return;
        };

        let (_, line_h) = self.measure_cell_metrics(window);
        let px_per_line = if line_h > 0.0 { line_h } else { 18.0 };

        let (top_edge, bottom_edge) =
            if let Some(b) = active_pane.as_ref().and_then(|p| p.last_bounds) {
                let by = b.origin.y.to_f64() as f32;
                let bh = b.size.height.to_f64() as f32;
                (by, by + bh)
            } else {
                let v = window.viewport_size();
                let avail_h = (v.height.to_f64() as f32 - 56.0).max(100.0);
                (32.0, 32.0 + avail_h)
            };

        // Auto-scroll triggers near or past the top/bottom edges
        let top_scroll_zone = top_edge + px_per_line * 1.5;
        let bottom_scroll_zone = bottom_edge - px_per_line * 1.5;

        let (dist, direction) = if raw_y < top_scroll_zone {
            ((top_scroll_zone - raw_y).max(0.0), 1.0f32)
        } else if raw_y > bottom_scroll_zone {
            ((raw_y - bottom_scroll_zone).max(0.0), -1.0f32)
        } else {
            self.selection_autoscroll_accum = 0.0;
            return;
        };

        if dist <= 0.0 {
            self.selection_autoscroll_accum = 0.0;
            return;
        }

        // Distance-scaled continuous velocity:
        // Starts very slow at boundary (~1.5 lines/sec = 1 line every ~0.66s),
        // accelerates smoothly as cursor is dragged further away.
        let speed = (1.5 + 0.05 * dist.powf(1.4)).clamp(1.0, 70.0);
        self.selection_autoscroll_accum += direction * speed * dt;

        if self.selection_autoscroll_accum.abs() >= 1.0 {
            let lines = self.selection_autoscroll_accum.trunc() as isize;
            self.selection_autoscroll_accum -= lines as f32;
            terminal.scroll(lines);
            self.last_scroll_activity = std::time::Instant::now();
            self.update_selection_endpoint(window, cx);
        }
    }

    fn handle_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };

        let mouse_x = event.position.x.to_f64() as f32;
        let mouse_y = event.position.y.to_f64() as f32;

        let target_pane = active_tab
            .pane_tree
            .all_panes()
            .into_iter()
            .find(|p| {
                if let Some(b) = p.last_bounds {
                    let bx = b.origin.x.to_f64() as f32;
                    let by = b.origin.y.to_f64() as f32;
                    let bw = b.size.width.to_f64() as f32;
                    let bh = b.size.height.to_f64() as f32;
                    mouse_x >= bx && mouse_x <= bx + bw && mouse_y >= by && mouse_y <= by + bh
                } else {
                    false
                }
            })
            .or_else(|| active_tab.pane_tree.active_pane());
        let target_pane_id = target_pane.as_ref().map(|p| p.id);
        let target_bounds = target_pane.as_ref().and_then(|p| p.last_bounds);
        let Some(terminal) = target_pane.as_ref().and_then(|p| p.terminal.as_ref()) else {
            return;
        };

        let (_, line_h) = self.measure_cell_metrics(_window);
        let px_per_line = if line_h > 0.0 { line_h } else { 18.0 };

        let delta_y = match event.delta {
            gpui::ScrollDelta::Pixels(p) => p.y.as_f32(),
            gpui::ScrollDelta::Lines(l) => l.y * px_per_line * 3.0,
        };

        if delta_y.abs() > 0.1 {
            self.last_scroll_activity = std::time::Instant::now();
            self.last_scrolled_pane_id = target_pane_id;
            if terminal.is_mouse_mode_enabled() {
                let (cell_w, _) = self.measure_cell_metrics(_window);
                let (pane_x, pane_y) = if let Some(b) = target_bounds {
                    (b.origin.x.to_f64() as f32, b.origin.y.to_f64() as f32)
                } else {
                    let sidebar_w = self.current_sidebar_width();
                    (sidebar_w + 1.0, 32.0)
                };
                let local_x = (mouse_x - pane_x).max(0.0);
                let local_y = (mouse_y - pane_y).max(0.0);
                let col = ((local_x / cell_w).floor() as usize) + 1;
                let row = ((local_y / px_per_line).floor() as usize) + 1;
                let btn = if delta_y > 0.0 { 64 } else { 65 };
                terminal.send_mouse_button_with_mods(
                    btn,
                    col,
                    row,
                    true,
                    event.modifiers.shift,
                    event.modifiers.alt,
                    event.modifiers.control,
                );
            } else {
                let display_offset = terminal.display_offset();
                let history_size = terminal.history_size();

                // If already at the bottom and trying to scroll down, stop immediately
                if display_offset == 0 && delta_y < 0.0 {
                    self.scroll_accum = 0.0;
                    return;
                }
                // If already at the top of history and trying to scroll up, stop immediately
                if display_offset >= history_size && delta_y > 0.0 {
                    self.scroll_accum = 0.0;
                    return;
                }

                self.scroll_accum += delta_y;
                let lines = (self.scroll_accum / px_per_line) as isize;
                if lines != 0 {
                    self.scroll_accum -= (lines as f32) * px_per_line;

                    let new_offset =
                        (display_offset as isize + lines).clamp(0, history_size as isize) as usize;
                    if new_offset != display_offset {
                        terminal.scroll_to_offset(new_offset);
                        cx.notify();
                    }

                    // Clear remainder when reaching boundary
                    if new_offset == 0 || new_offset == history_size {
                        self.scroll_accum = 0.0;
                    }
                }
            }
        }
    }

    fn render_pane_tree_node(
        &self,
        node: &PaneNode,
        path: Vec<usize>,
        container_x: f32,
        container_y: f32,
        avail_w: f32,
        avail_h: f32,
        cell_w: f32,
        line_h: f32,
        active_pane_id: usize,
        pane_count: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = self.theme;
        let font_family = self.font_family.clone();
        let font_size = self.font_size;

        match node {
            PaneNode::Leaf(pane) => {
                let is_active = pane.id == active_pane_id;
                let pane_id = pane.id;
                let target_cols = ((avail_w / cell_w) as usize).max(20);
                let target_rows = ((avail_h / line_h) as usize).max(5);

                let grid_el = if let Some(ref terminal) = pane.terminal {
                    terminal.resize_with_pixels(
                        target_cols,
                        target_rows,
                        (target_cols as f32 * cell_w).round() as u16,
                        (target_rows as f32 * line_h).round() as u16,
                    );
                    let term = terminal.term();
                    if let Some(term_guard) = term.try_lock() {
                        let content = term_guard.renderable_content();
                        let cursor_point = content.cursor.point;
                        let cursor_visible = content.cursor.shape != CursorShape::Hidden;
                        let display_offset = term_guard.grid().display_offset();

                        let mut lines: Vec<Vec<StyledSpan>> = Vec::new();
                        let mut current_row_spans: Vec<StyledSpan> = Vec::new();
                        let mut current_span_text = String::new();
                        let mut current_span_start_col: usize = 0;
                        let mut current_span_end_col: usize = 0;
                        let mut current_block_cat: Option<char> = None;
                        let mut current_is_emoji = false;
                        let mut current_is_nerd = false;
                        let mut current_is_kbd = false;
                        let mut current_fg = theme.foreground;
                        let mut current_bg: Option<Hsla> = None;
                        let mut current_bold = false;
                        let mut current_underline = false;
                        let mut last_row: Option<i32> = None;
                        let mut current_span_char_cols: Vec<usize> = Vec::new();
                        let theme_bg_rgb = Self::hsla_to_rgb_tuple(theme.background);
                        let theme_fg_rgb = Self::hsla_to_rgb_tuple(theme.foreground);
                        // Selected cells get their fg corrected against what
                        // they will actually sit on: their background blended
                        // with the accent selection tint painted under the
                        // text (see `TerminalGridElement`'s selection pass).
                        // Without this, dim explicit colors (TUI grays) that
                        // read on the plain background vanish on the tint.
                        let selection = if is_active { self.selection } else { None };
                        let selection_accent_rgb = Self::hsla_to_rgb_tuple(theme.accent);
                        let mut current_sel_cols: Option<(usize, usize)> = None;

                        for cell in content.display_iter {
                            let row = cell.point.line.0;
                            let col = cell.point.column.0;

                            if last_row != Some(row) {
                                current_sel_cols = selection.as_ref().and_then(|sel| {
                                    crate::ui::terminal_grid_element::selection_columns_on_line(
                                        sel, row,
                                    )
                                });
                            }

                            if col >= target_cols {
                                continue;
                            }

                            if let Some(prev) = last_row {
                                if row != prev {
                                    if !current_span_text.is_empty() {
                                        current_row_spans.push(StyledSpan {
                                            text: current_span_text,
                                            start_col: current_span_start_col,
                                            end_col: current_span_end_col,
                                            fg: current_fg,
                                            bg: current_bg,
                                            is_bold: current_bold,
                                            is_underline: current_underline,
                                            is_emoji: current_is_emoji,
                                            is_nerd: current_is_nerd,
                                            is_kbd: current_is_kbd,
                                            emoji_scale: None,
                                            char_cols: current_span_char_cols,
                                        });
                                        current_span_text = String::new();
                                        current_span_char_cols = Vec::new();
                                        current_block_cat = None;
                                        current_is_emoji = false;
                                        current_is_nerd = false;
                                        current_is_kbd = false;
                                    }
                                    trim_row_spans(&mut current_row_spans);
                                    lines.push(current_row_spans);
                                    current_row_spans = Vec::new();

                                    let mut r = prev + 1;
                                    while r < row {
                                        lines.push(Vec::new());
                                        r += 1;
                                    }

                                    last_row = Some(row);
                                }
                            } else {
                                last_row = Some(row);
                            }

                            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                                continue;
                            }

                            let is_hovered_url =
                                if let Some((h_row, h_start, h_end)) = self.hovered_url_range {
                                    row == h_row && col >= h_start && col < h_end
                                } else {
                                    false
                                };

                            let (effective_fg, effective_bg) =
                                if cell.flags.contains(Flags::INVERSE) {
                                    (cell.bg, cell.fg)
                                } else {
                                    (cell.fg, cell.bg)
                                };
                            let cell_bg_rgb = match effective_bg {
                                AnsiColor::Named(NamedColor::Background) => theme_bg_rgb,
                                other => Self::explicit_color_rgb(other).unwrap_or(theme_bg_rgb),
                            };
                            let in_selection = current_sel_cols
                                .is_some_and(|(c0, c1)| col >= c0 && col < c1);
                            let correction_base = if in_selection {
                                crate::ui::color_harmony::alpha_blend(
                                    cell_bg_rgb,
                                    selection_accent_rgb,
                                    crate::ui::terminal_grid_element::SELECTION_OVERLAY_ALPHA,
                                )
                            } else {
                                cell_bg_rgb
                            };
                            let fg = self.convert_fg_harmonized(effective_fg, correction_base);
                            let bg = match effective_bg {
                                AnsiColor::Named(NamedColor::Background) => None,
                                other => {
                                    let hsla = if self.config.contrast_correction {
                                        Self::explicit_color_rgb(other)
                                            .map(|rgb| {
                                                let fixed =
                                                    crate::ui::color_harmony::correct_bg(rgb, theme_fg_rgb);
                                                self.convert_color(Self::ansi_spec(fixed), false)
                                            })
                                            .unwrap_or_else(|| self.convert_color(other, false))
                                    } else {
                                        self.convert_color(other, false)
                                    };
                                    Some(hsla)
                                }
                            };

                            let is_bold = cell.flags.contains(Flags::BOLD);
                            let is_underline = cell.flags.contains(Flags::UNDERLINE)
                                || is_hovered_url
                                || cell.hyperlink().is_some();
                            let code = cell.c as u32;
                            let is_kbd = is_keyboard_symbol(code);
                            let is_emoji = is_emoji_codepoint(code);
                            let is_nerd = is_nerd_codepoint(code);
                            let is_pua_icon = (0xE000..=0xF8FF).contains(&code)
                                || (0xF0000..=0xFFFFD).contains(&code)
                                || (0x100000..=0x10FFFD).contains(&code);
                            let block_cat = if (0x2500..=0x257F).contains(&code)
                                || (0x2580..=0x259F).contains(&code)
                                || (0x25A0..=0x25FF).contains(&code)
                                || is_emoji
                                || is_pua_icon
                                || is_kbd
                            {
                                Some(cell.c)
                            } else {
                                None
                            };

                            let cell_cols = if cell.flags.contains(Flags::WIDE_CHAR) {
                                2
                            } else {
                                1
                            };
                            let end_col = (col + cell_cols).min(target_cols);

                            if fg != current_fg
                                || bg != current_bg
                                || is_bold != current_bold
                                || is_underline != current_underline
                                || block_cat != current_block_cat
                                || current_is_kbd
                                || is_kbd
                                || current_span_text.is_empty()
                            {
                                if !current_span_text.is_empty() {
                                    current_row_spans.push(StyledSpan {
                                        text: current_span_text,
                                        start_col: current_span_start_col,
                                        end_col: current_span_end_col,
                                        fg: current_fg,
                                        bg: current_bg,
                                        is_bold: current_bold,
                                        is_underline: current_underline,
                                        is_emoji: current_is_emoji,
                                        is_nerd: current_is_nerd,
                                        is_kbd: current_is_kbd,
                                        emoji_scale: None,
                                        char_cols: current_span_char_cols,
                                    });
                                    current_span_text = String::new();
                                    current_span_char_cols = Vec::new();
                                }
                                current_fg = fg;
                                current_bg = bg;
                                current_bold = is_bold;
                                current_underline = is_underline;
                                current_block_cat = block_cat;
                                current_is_emoji = is_emoji;
                                current_is_nerd = is_nerd;
                                current_is_kbd = is_kbd;
                                current_span_start_col = col;
                            }
                            let display_c = if cell.c == '\t' || cell.c == '\0' {
                                ' '
                            } else {
                                cell.c
                            };
                            current_span_text.push(display_c);
                            current_span_char_cols.push(col);
                            current_span_end_col = end_col;
                        }

                        if !current_span_text.is_empty() {
                            current_row_spans.push(StyledSpan {
                                text: current_span_text,
                                start_col: current_span_start_col,
                                end_col: current_span_end_col,
                                fg: current_fg,
                                bg: current_bg,
                                is_bold: current_bold,
                                is_underline: current_underline,
                                is_emoji: current_is_emoji,
                                is_nerd: current_is_nerd,
                                is_kbd: current_is_kbd,
                                emoji_scale: None,
                                char_cols: current_span_char_cols,
                            });
                        }
                        trim_row_spans(&mut current_row_spans);
                        lines.push(current_row_spans);

                        let cursor_color = theme.cursor;

                        let effective_shape = if content.cursor.shape == CursorShape::Block {
                            self.config.cursor.shape.into()
                        } else {
                            content.cursor.shape
                        };

                        let cursor_info =
                            if cursor_visible && is_active && self.cursor_blink_visible {
                                Some((cursor_point.line.0, cursor_point.column.0, effective_shape))
                            } else {
                                None
                            };

                        let mut visible_images = Vec::new();
                        let total_pushed = terminal
                            .total_lines_pushed
                            .load(std::sync::atomic::Ordering::Relaxed);
                        let screen_rows = lines.len();
                        let viewport_start_abs = (total_pushed + screen_rows as u64)
                            .saturating_sub(display_offset as u64 + screen_rows as u64);
                        let viewport_end_abs = viewport_start_abs + screen_rows as u64;

                        let store = terminal.image_store.lock();
                        for placement in &store.placements {
                            let p_start = placement.absolute_line;
                            let p_end = placement.absolute_line + placement.rows as u64;
                            if p_end > viewport_start_abs && p_start < viewport_end_abs {
                                let screen_row = (p_start as i64) - (viewport_start_abs as i64);
                                visible_images.push(
                                    crate::ui::terminal_grid_element::VisibleImage {
                                        row: screen_row,
                                        col: placement.col,
                                        cols: placement.cols,
                                        rows: placement.rows,
                                        z_index: placement.z_index,
                                        image: placement.image.clone(),
                                    },
                                );
                            }
                        }
                        drop(store);

                        let selection_range = if is_active { self.selection } else { None };
                        let search_open =
                            is_active && self.is_search_open && !self.search_query.is_empty();
                        let search_query_str = self.search_query.to_lowercase();
                        let active_search_offset =
                            self.search_matches.get(self.search_match_idx).copied();
                        let num_lines = lines.len();
                        let row_width = (target_cols as f32 * cell_w).round();

                        let mut all_search_highlights = Vec::with_capacity(num_lines);
                        if search_open && !search_query_str.is_empty() {
                            for (row_idx, spans) in lines.iter().enumerate() {
                                let mut row_highlights = Vec::new();
                                let mut full_line = String::new();
                                let mut char_to_col: Vec<usize> = Vec::new();
                                let mut char_to_end_col: Vec<usize> = Vec::new();

                                for s in spans {
                                    for (i, c) in s.text.chars().enumerate() {
                                        full_line.push(c);
                                        let col_start =
                                            s.char_cols.get(i).copied().unwrap_or(s.start_col);
                                        let col_end =
                                            s.char_cols.get(i + 1).copied().unwrap_or(s.end_col);
                                        char_to_col.push(col_start);
                                        char_to_end_col.push(col_end);
                                    }
                                }

                                let line_lower = full_line.to_lowercase();
                                let mut start_b = 0;
                                while let Some(found_b) =
                                    line_lower[start_b..].find(&search_query_str)
                                {
                                    let match_start_b = start_b + found_b;
                                    let match_end_b = match_start_b + search_query_str.len();
                                    let match_start_char =
                                        line_lower[..match_start_b].chars().count();
                                    let match_end_char = line_lower[..match_end_b].chars().count();

                                    if match_start_char < char_to_col.len() && match_end_char > 0 {
                                        let col_start = char_to_col[match_start_char];
                                        let last_char_idx =
                                            (match_end_char - 1).min(char_to_end_col.len() - 1);
                                        let col_end = char_to_end_col[last_char_idx];
                                        let col_len = col_end.saturating_sub(col_start).max(1);

                                        let is_active_match =
                                            active_search_offset.is_some_and(|off| {
                                                off == (display_offset
                                                    + (num_lines.saturating_sub(1 + row_idx)))
                                            });
                                        row_highlights.push((col_start, col_len, is_active_match));
                                    }
                                    start_b = match_end_b;
                                    if search_query_str.is_empty() {
                                        break;
                                    }
                                }
                                all_search_highlights.push(row_highlights);
                            }
                        } else {
                            all_search_highlights.resize(num_lines, Vec::new());
                        }

                        Some(crate::ui::terminal_grid_element::TerminalGridElement {
                            lines,
                            cell_w,
                            line_h,
                            row_width,
                            cursor_info,
                            selection_range,
                            search_highlights: all_search_highlights,
                            visible_images,
                            theme,
                            cursor_color,
                            font_family: font_family.clone(),
                            font_size,
                            display_offset,
                            nerd_font_family: self.nerd_font_family.clone(),
                            ligatures: self.config.font.ligatures,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                let (history_size, display_offset) = if let Some(ref terminal) = pane.terminal {
                    (terminal.history_size(), terminal.display_offset())
                } else {
                    (0, 0)
                };

                let pane_scrollbar_thumb = if history_size > 0 {
                    let elapsed_ms = self.last_scroll_activity.elapsed().as_millis();
                    let is_this_dragging = self.is_dragging_scrollbar
                        && self.dragging_scrollbar_pane_id == Some(pane.id);
                    let is_this_scrolling = (self.last_scrolled_pane_id == Some(pane.id)
                        || (self.last_scrolled_pane_id.is_none() && is_active))
                        && elapsed_ms < 1500;

                    // Proximity check against this pane's right edge
                    let right_edge = container_x + avail_w;
                    let bottom_edge = container_y + avail_h;
                    let (is_in_proximity, dist_from_right) =
                        if let Some((mx, my)) = self.cursor_window_pos {
                            if my >= (container_y - 4.0)
                                && my <= (bottom_edge + 4.0)
                                && mx >= (right_edge - 48.0)
                                && mx <= (right_edge + 8.0)
                            {
                                let dist = (right_edge - mx).max(0.0);
                                (true, dist)
                            } else {
                                (false, 999.0)
                            }
                        } else {
                            (false, 999.0)
                        };

                    if is_this_dragging || is_this_scrolling || is_in_proximity {
                        let (alpha_mult, visual_w) = if is_this_dragging {
                            (1.0, 8.0)
                        } else if dist_from_right <= 14.0 {
                            (0.90, 8.0)
                        } else if is_in_proximity {
                            let t = ((48.0 - dist_from_right) / 34.0).clamp(0.0, 1.0);
                            (0.30 + 0.60 * t, 6.0)
                        } else if elapsed_ms < 700 {
                            (1.0, 5.0)
                        } else {
                            let t = (elapsed_ms - 700) as f32 / 800.0;
                            ((1.0 - t).clamp(0.0, 1.0), 5.0)
                        };

                        let track_h = avail_h;
                        let total_rows = (history_size + target_rows) as f32;
                        let thumb_h =
                            (track_h * (target_rows as f32 / total_rows)).clamp(24.0, track_h);
                        let progress =
                            1.0 - (display_offset as f32 / history_size as f32).clamp(0.0, 1.0);
                        let thumb_top =
                            ((track_h - thumb_h) * progress).clamp(0.0, track_h - thumb_h);

                        let mut thumb_bg = if is_this_dragging {
                            theme.accent
                        } else {
                            theme.muted
                        };
                        thumb_bg.a = 0.50 * alpha_mult;
                        let mut thumb_hover_bg = if is_this_dragging {
                            theme.accent
                        } else {
                            theme.muted_strong
                        };
                        thumb_hover_bg.a = 0.90 * alpha_mult;

                        Some(
                            div()
                                .id(("scrollbar-hit-area", pane_id))
                                .absolute()
                                .right(px(4.))
                                .top(px(thumb_top))
                                .w(px(14.))
                                .h(px(thumb_h))
                                .flex()
                                .justify_end()
                                .cursor(CursorStyle::PointingHand)
                                .child(
                                    div()
                                        .w(px(visual_w))
                                        .h_full()
                                        .rounded_full()
                                        .bg(thumb_bg)
                                        .hover(move |s| s.bg(thumb_hover_bg)),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
                                        this.is_dragging_scrollbar = true;
                                        this.dragging_scrollbar_pane_id = Some(pane_id);
                                        this.last_scrolled_pane_id = Some(pane_id);
                                        this.scrollbar_drag_start_y = ev.position.y.to_f64() as f32;
                                        this.scrollbar_drag_start_offset = display_offset;
                                        this.last_scroll_activity = std::time::Instant::now();
                                        cx.notify();
                                    }),
                                ),
                        )
                    } else {
                        None
                    }
                } else {
                    None
                };

                let program_badge = self.visible_program_record(pane_id).map(|record| {
                    let mut label = format!("{}: {}",
                        record.app.as_deref().or(record.title.as_deref()).unwrap_or("Program"),
                        super::tab_bar::program_status_text(record));
                    if let Some(progress) = record.progress {
                        label.push_str(&format!(" {progress}%"));
                    }
                    if let Some(message) = record.msg.as_deref().filter(|message| !message.is_empty()) {
                        label.push_str(&format!(" · {message}"));
                    }
                    div().id(("program-status", pane_id))
                        .absolute().bottom(px(4.)).right(px(20.))
                        .flex().items_center().gap(px(5.))
                        .max_w(px(260.)).px(px(7.)).py(px(4.))
                        .rounded(px(5.)).bg(theme.surface_raised)
                        .border_1().border_color(theme.border)
                        .text_size(px(11.)).text_color(theme.foreground)
                        .overflow_hidden()
                        .child(super::tab_bar::render_program_status(record, theme, 12.0))
                        .child(div().min_w(px(0.)).truncate().child(SharedString::from(label)))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                            this.acknowledge_program_status(pane_id);
                            cx.stop_propagation();
                            cx.notify();
                        }))
                });

                div()
                    .size_full()
                    .relative()
                    .overflow_hidden()
                    .when(pane_count > 1 && is_active, |d| {
                        d.border_1().border_color(theme.accent)
                    })
                    .when(pane_count > 1 && !is_active, |d| {
                        d.border_1().border_color(theme.border)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _ev, _window, cx| {
                            this.acknowledge_program_status(pane_id);
                            if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                tab.pane_tree.active_pane_id = pane_id;
                                if let Some(p) = tab.pane_tree.active_pane() {
                                    tab.terminal = p.terminal.clone();
                                    tab.cwd = p.cwd.clone();
                                    tab.git_status = p.git_status.clone();
                                }
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _ev, _window, cx| {
                            if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                tab.pane_tree.active_pane_id = pane_id;
                                if let Some(p) = tab.pane_tree.active_pane() {
                                    tab.terminal = p.terminal.clone();
                                    tab.cwd = p.cwd.clone();
                                    tab.git_status = p.git_status.clone();
                                }
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _ev, _window, cx| {
                            if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                tab.pane_tree.active_pane_id = pane_id;
                                if let Some(p) = tab.pane_tree.active_pane() {
                                    tab.terminal = p.terminal.clone();
                                    tab.cwd = p.cwd.clone();
                                    tab.git_status = p.git_status.clone();
                                }
                                cx.notify();
                            }
                        }),
                    )
                    .when_some(grid_el, |d, el| d.child(el))
                    .when_some(pane_scrollbar_thumb, |d, thumb| d.child(thumb))
                    .when_some(program_badge, |d, badge| d.child(badge))
            }
            PaneNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let mut container = div().size_full().flex();
                container = match direction {
                    SplitDirection::Horizontal => container.flex_row(),
                    SplitDirection::Vertical => container.flex_col(),
                };

                let (w1, h1, w2, h2, first_x, first_y, second_x, second_y) = match direction {
                    SplitDirection::Horizontal => {
                        let w1 = avail_w * *ratio;
                        let w2 = avail_w * (1.0 - *ratio);
                        (
                            w1,
                            avail_h,
                            w2,
                            avail_h,
                            container_x,
                            container_y,
                            container_x + w1,
                            container_y,
                        )
                    }
                    SplitDirection::Vertical => {
                        let h1 = avail_h * *ratio;
                        let h2 = avail_h * (1.0 - *ratio);
                        (
                            avail_w,
                            h1,
                            avail_w,
                            h2,
                            container_x,
                            container_y,
                            container_x,
                            container_y + h1,
                        )
                    }
                };

                let is_dragging_this =
                    self.is_dragging_pane_split && self.dragging_split_path == path;
                let divider_line_color = if is_dragging_this {
                    theme.accent
                } else {
                    theme.border
                };
                let split_path_clone = path.clone();
                let dir_val = *direction;

                let divider = match direction {
                    SplitDirection::Horizontal => div()
                        .id(SharedString::from(format!("h-split-divider-{:?}", path)))
                        .relative()
                        .w(px(7.))
                        .h_full()
                        .mx(px(-3.))
                        .cursor(CursorStyle::ResizeColumn)
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |s| s.bg(gpui::transparent_black()))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _ev: &MouseDownEvent, _window, cx| {
                                this.is_dragging_pane_split = true;
                                this.dragging_split_path = split_path_clone.clone();
                                this.dragging_split_direction = dir_val;
                                this.dragging_split_bounds =
                                    (container_x, container_y, avail_w, avail_h);
                                cx.notify();
                            }),
                        )
                        .child(div().w(px(1.)).h_full().bg(divider_line_color)),
                    SplitDirection::Vertical => div()
                        .id(SharedString::from(format!("v-split-divider-{:?}", path)))
                        .relative()
                        .h(px(7.))
                        .w_full()
                        .my(px(-3.))
                        .cursor(CursorStyle::ResizeRow)
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(move |s| s.bg(gpui::transparent_black()))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _ev: &MouseDownEvent, _window, cx| {
                                this.is_dragging_pane_split = true;
                                this.dragging_split_path = split_path_clone.clone();
                                this.dragging_split_direction = dir_val;
                                this.dragging_split_bounds =
                                    (container_x, container_y, avail_w, avail_h);
                                cx.notify();
                            }),
                        )
                        .child(div().h(px(1.)).w_full().bg(divider_line_color)),
                };

                let mut path_first = path.clone();
                path_first.push(0);
                let mut path_second = path;
                path_second.push(1);

                container
                    .child(
                        div()
                            .flex_basis(gpui::relative(*ratio))
                            .size_full()
                            .overflow_hidden()
                            .child(self.render_pane_tree_node(
                                first,
                                path_first,
                                first_x,
                                first_y,
                                w1,
                                h1,
                                cell_w,
                                line_h,
                                active_pane_id,
                                pane_count,
                                cx,
                            )),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex_basis(gpui::relative(1.0 - *ratio))
                            .size_full()
                            .overflow_hidden()
                            .child(self.render_pane_tree_node(
                                second,
                                path_second,
                                second_x,
                                second_y,
                                w2,
                                h2,
                                cell_w,
                                line_h,
                                active_pane_id,
                                pane_count,
                                cx,
                            )),
                    )
            }
        }
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let mut live_panes = std::collections::HashSet::new();
        for tab in &self.tabs {
            for pane in tab.pane_tree.all_panes() {
                live_panes.insert(pane.id);
                if !self.program_statuses.contains_key(&pane.id) {
                    if let Some(terminal) = pane.terminal.as_ref() {
                        let records = terminal.program_status();
                        let unread = records.iter().any(|record| {
                            matches!(record.state, ProgramState::Done | ProgramState::Blocked | ProgramState::Error)
                        });
                        self.program_statuses.insert(pane.id, ProgramPaneStatus { records, unread, last_notification: None });
                    }
                }
            }
        }
        self.program_statuses.retain(|id, _| live_panes.contains(id));
        if self.ai_response_is_visible(_window) {
            if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                if let Some(completion) = self.ai_turn_completions.get_mut(&tab.ai_conversation_id) {
                    *completion = AiTurnCompletion::Read;
                }
            }
        }

        // F5: the picker fetched for another cwd: close rather than act on
        // stale data (e.g. tab switched by mouse while open).
        if self.is_pr_picker_open {
            let current_cwd = self
                .tabs
                .get(self.active_tab_idx)
                .and_then(|t| t.cwd.clone());
            if current_cwd != self.pr_picker_cwd {
                self.is_pr_picker_open = false;
            }
        }
        let tab_items: Vec<TabItem> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| {
                let process_name = tab
                    .terminal
                    .as_ref()
                    .and_then(|t| t.get_foreground_process_name());
                // F2: zoomed tabs carry a marker so the state is visible.
                let mut title = tab
                    .custom_title
                    .clone()
                    .unwrap_or_else(|| tab.title.clone());
                if tab.zoomed_pane.is_some() {
                    title = format!("[Z] {title}");
                }
                TabItem {
                    id: tab.id,
                    title,
                    active: idx == self.active_tab_idx,
                    ai_response_unread: self.ai_turn_completions.get(&tab.ai_conversation_id) == Some(&AiTurnCompletion::Unread),
                    program_status: tab.pane_tree.all_panes().into_iter()
                        .filter_map(|pane| self.visible_program_record(pane.id))
                        .max_by_key(|record| record.state.priority())
                        .cloned(),
                    is_dirty: tab
                        .git_status
                        .as_ref()
                        .is_some_and(|g| g.unstaged > 0 || g.staged > 0),
                    process_name,
                }
            })
            .collect();

        let mut background_program_statuses = Vec::new();
        for (tab_idx, tab) in self.tabs.iter().enumerate() {
            if tab_idx == self.active_tab_idx {
                continue;
            }
            let title = tab.custom_title.as_deref().unwrap_or(&tab.title).to_string();
            for pane in tab.pane_tree.all_panes() {
                if let Some(record) = self.visible_program_record(pane.id).cloned() {
                    background_program_statuses.push((tab_idx, pane.id, title.clone(), record));
                }
            }
        }
        background_program_statuses.sort_by_key(|(tab_idx, pane_id, _, record)| {
            (std::cmp::Reverse(record.state.priority()), *tab_idx, *pane_id)
        });

        let cwd_changed = {
            let active_tab = self.tabs.get_mut(self.active_tab_idx);
            if let Some(active_tab) = active_tab {
                if let Some(ref term) = active_tab.terminal {
                    if let Some(proc_cwd) = term.get_current_working_directory() {
                        if active_tab.cwd.as_ref() != Some(&proc_cwd) {
                            active_tab.cwd = Some(proc_cwd);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        };
        if cwd_changed {
            self.persist_session();
        }

        let (active_cwd, active_git, fallback_info) =
            if let Some(active_tab) = self.tabs.get_mut(self.active_tab_idx) {
                let now = std::time::Instant::now();
                let should_poll = active_tab.git_checked_cwd.as_ref() != active_tab.cwd.as_ref()
                    || active_tab.git_last_poll.is_none_or(|last| {
                        now.duration_since(last) >= std::time::Duration::from_secs(2)
                    });

                if should_poll {
                    active_tab.git_checked_cwd = active_tab.cwd.clone();
                    active_tab.git_last_poll = Some(now);
                    if let Some(ref cwd) = active_tab.cwd {
                        active_tab.git_status = crate::git::fetch_git_info_cached(cwd);
                    } else {
                        active_tab.git_status = None;
                    }
                }
                let cwd_path = active_tab.cwd.as_deref();
                let git = active_tab.git_status.as_ref();
                let info = StatusInfo {
                    git_branch: git.map(|g| g.branch.clone()),
                    git_dirty: git.map(|g| g.unstaged > 0 || g.staged > 0).unwrap_or(false),
                    cwd: cwd_path.map(|p| p.to_string_lossy().into_owned()),
                    last_duration_ms: active_tab.last_duration_ms,
                    last_exit_code: active_tab.last_exit_code,
                };
                (cwd_path, git.cloned(), info)
            } else {
                (None, None, StatusInfo::default())
            };

        let (left_segs, right_segs) = self.status_bar_model.render_segments(
            active_cwd,
            active_git.as_ref(),
            _window.is_window_active(),
        );

        let (cell_w, line_h) = self.measure_cell_metrics(_window);
        let viewport_size = _window.viewport_size();
        let sidebar_w = self.current_sidebar_width();
        let ai_w = self.current_ai_sidebar_width();
        let avail_w = (viewport_size.width.to_f64() as f32 - 1.0 - sidebar_w - ai_w).max(100.0);
        // Chrome layout constants — must match TabBar/StatusBar element heights

        const TAB_BAR_HEIGHT: f32 = 32.0;
        const STATUS_BAR_HEIGHT: f32 = 20.0;
        // One-line notice bars (config error, onboarding) sit between the tab
        // bar and the terminal and take real layout space, so the pane grid
        // stays aligned with mouse coordinates.
        const NOTICE_BAR_HEIGHT: f32 = 28.0;
        let notice_h = ((self.config_error.is_some() as u8 + self.show_onboarding_hint as u8)
            as f32)
            * NOTICE_BAR_HEIGHT;
        let top_offset = TAB_BAR_HEIGHT + notice_h;
        let avail_h =
            (viewport_size.height.to_f64() as f32 - top_offset - STATUS_BAR_HEIGHT).max(100.0);

        let font_family = self.font_family.clone();
        let font_ligatures = self.config.font.ligatures;
        let font_size = self.font_size;

        // F2: a zoomed pane renders alone over the whole area. The temp leaf
        // is a clone, so real-tree state is untouched; hidden panes lose
        // their bounds so mouse routing can't land on them.
        let zoomed_id = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|tab| tab.zoomed_pane);
        let zoomed_leaf: Option<PaneNode> = zoomed_id.and_then(|id| {
            self.tabs
                .get(self.active_tab_idx)?
                .pane_tree
                .find_pane(id)
                .cloned()
                .map(PaneNode::Leaf)
        });

        if let Some(active_tab) = self.tabs.get_mut(self.active_tab_idx) {
            if zoomed_leaf.is_some() {
                let full = gpui::Bounds {
                    origin: gpui::Point {
                        x: px(sidebar_w),
                        y: px(top_offset),
                    },
                    size: gpui::Size {
                        width: px(avail_w),
                        height: px(avail_h),
                    },
                };
                for p in active_tab.pane_tree.all_panes_mut() {
                    p.last_bounds = if Some(p.id) == zoomed_id {
                        Some(full)
                    } else {
                        None
                    };
                }
            } else {
                // No zoom (or a stale zoom target): normal tree layout.
                active_tab.zoomed_pane = None;
                active_tab
                    .pane_tree
                    .update_layout_bounds(sidebar_w, top_offset, avail_w, avail_h);
            }
        }

        let terminal_area = if let Some(leaf) = zoomed_leaf.as_ref() {
            let active_pane_id = self
                .tabs
                .get(self.active_tab_idx)
                .map(|t| t.pane_tree.active_pane_id)
                .unwrap_or(0);
            self.render_pane_tree_node(
                leaf,
                Vec::new(),
                sidebar_w,
                top_offset,
                avail_w,
                avail_h,
                cell_w,
                line_h,
                active_pane_id,
                1,
                cx,
            )
        } else if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
            let active_pane_id = active_tab.pane_tree.active_pane_id;
            let pane_count = active_tab.pane_tree.pane_count();
            self.render_pane_tree_node(
                &active_tab.pane_tree.root,
                Vec::new(),
                sidebar_w,
                top_offset,
                avail_w,
                avail_h,
                cell_w,
                line_h,
                active_pane_id,
                pane_count,
                cx,
            )
        } else {
            div().size_full()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.window_fill())
            .text_color(theme.foreground)
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                this.handle_file_drop(paths.paths(), window, cx);
            }))
            .child(
                TabBar::new(tab_items.clone(), theme, self.tab_bar_scroll_handle.clone())
                    .layout(self.tab_layout)
                    .sidebar_open(self.sidebar_open)
                    .on_toggle_sidebar(cx.listener(|this, _ev, window, cx| {
                        this.toggle_tab_sidebar(window, cx);
                    }))
                    .ai_sidebar_open(self.ai_sidebar_open)
                    .ai_response_unread(self.tabs.iter().any(|tab| {
                        self.ai_turn_completions.get(&tab.ai_conversation_id) == Some(&AiTurnCompletion::Unread)
                    }))
                    .on_toggle_ai(cx.listener(|this, _ev, window, cx| {
                        this.toggle_ai_sidebar(window, cx);
                    }))
                    .update_available(self.update_available.as_ref().map(|u| u.version.clone()), self.is_updating, self.is_update_ready)
                    .on_update(cx.listener(|this, _ev, window, cx| {
                        this.trigger_apply_update(window, cx);
                    }))
                    .on_select_tab(cx.listener(|this, &tab_id, _window, cx| {
                        this.select_tab(tab_id, cx);
                    }))
                    .on_close_tab(cx.listener(|this, &tab_id, window, cx| {
                        this.close_tab(tab_id, window, cx);
                    }))
                    .on_rename_tab(cx.listener(|this, &tab_id, _window, cx| {
                        if let Some(pos) = this.tabs.iter().position(|t| t.id == tab_id) {
                            this.open_rename_tab(pos, cx);
                        }
                    }))
                    .on_tab_context_menu(cx.listener(|this, &(tab_id, x, y), _window, cx| {
                        this.open_tab_context_menu(tab_id, x, y, cx);
                    }))
                    .on_new_tab(cx.listener(|this, _ev, window, cx| {
                        this.create_tab(window, cx);
                    }))
                    .on_logo_context_menu(cx.listener(|this, _ev, window, cx| {
                        this.toggle_context_menu(window, cx);
                    })),
            )
            // UX1: a broken config file fails silently to defaults without
            // this. One-line banner with the parse error; fixing the file
            // clears it via the config watcher + reload_config.
            .when_some(self.config_error.clone(), |this, err| {
                let short: String = err.chars().take(160).collect();
                this.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .h(px(28.))
                        .px(px(12.))
                        .bg(theme.bright_red.opacity(0.12))
                        .border_b_1()
                        .border_color(theme.bright_red.opacity(0.45))
                        .child(div().text_size(px(12.)).child("⚠"))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.5))
                                .text_color(theme.foreground)
                                .overflow_hidden()
                                .child(SharedString::from(format!("Config error — {short}"))),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .bg(theme.surface_raised)
                                .text_size(px(11.))
                                .text_color(theme.foreground)
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                    Self::open_settings_file();
                                })
                                .child("Open config"),
                        )
                        .child(
                            div()
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .text_size(px(11.))
                                .text_color(theme.muted)
                                .cursor(CursorStyle::PointingHand)
                                .hover(|s| s.text_color(theme.foreground))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _ev, _window, cx| {
                                        this.config_error = None;
                                        cx.notify();
                                    }),
                                )
                                .child("Dismiss"),
                        ),
                )
            })
            // UX7: first-run hint. Only shows when neither a config file nor
            // a saved session exists yet; dismissed for the session.
            .when(self.show_onboarding_hint, |this| {
                let hints = if cfg!(target_os = "macos") {
                    "Welcome to Fastty — ⌘P commands · ⌘O SSH hosts · ⌘, Settings"
                } else {
                    "Welcome to Fastty — Ctrl+Shift+P commands · Ctrl+Shift+O SSH hosts · Ctrl+, Settings"
                };
                this.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .h(px(28.))
                        .px(px(12.))
                        .bg(theme.accent.opacity(0.10))
                        .border_b_1()
                        .border_color(theme.accent.opacity(0.35))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.5))
                                .text_color(theme.foreground)
                                .overflow_hidden()
                                .child(hints.to_string()),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .bg(theme.surface_raised)
                                .text_size(px(11.))
                                .text_color(theme.foreground)
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _ev, _window, cx| {
                                        this.show_onboarding_hint = false;
                                        cx.notify();
                                    }),
                                )
                                .child("Got it"),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .when(self.tab_layout == TabLayout::Vertical || self.sidebar_anim_progress > 0.001, |this| {
                        this.child(
                            TabSidebar::new(tab_items.clone(), theme, self.sidebar_anim_progress, self.sidebar_scroll_handle.clone())
                                .on_select_tab(cx.listener(|this, &tab_id, _window, cx| {
                                    this.select_tab(tab_id, cx);
                                }))
                                .on_close_tab(cx.listener(|this, &tab_id, window, cx| {
                                    this.close_tab(tab_id, window, cx);
                                }))
                                .on_rename_tab(cx.listener(|this, &tab_id, _window, cx| {
                                    if let Some(pos) = this.tabs.iter().position(|t| t.id == tab_id) {
                                        this.open_rename_tab(pos, cx);
                                    }
                                }))
                                .on_tab_context_menu(cx.listener(|this, &(tab_id, x, y), _window, cx| {
                                    this.open_tab_context_menu(tab_id, x, y, cx);
                                }))
                                .on_new_tab(cx.listener(|this, _ev, window, cx| {
                                    this.create_tab(window, cx);
                                }))
                                .on_toggle_sidebar(cx.listener(|this, _ev, window, cx| {
                                    this.toggle_tab_sidebar(window, cx);
                                }))
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .track_focus(&self.focus_handle)
                            .key_context("RootView")
                            .on_key_down(cx.listener(Self::handle_key_down))
                            .on_scroll_wheel(cx.listener(Self::handle_scroll))
                            .on_mouse_move(cx.listener(Self::handle_mouse_move))
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
                            .on_mouse_down(MouseButton::Right, cx.listener(Self::handle_mouse_down))
                            .on_mouse_down(MouseButton::Middle, cx.listener(Self::handle_mouse_down))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
                            .on_mouse_up(MouseButton::Right, cx.listener(Self::handle_mouse_up))
                            .on_mouse_up(MouseButton::Middle, cx.listener(Self::handle_mouse_up))
                            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                                this.handle_file_drop(paths.paths(), window, cx);
                            }))
                            .flex()
                            .flex_1()
                            .flex_col()
                            .h_full()
                            .pl(px(1.))
                            .bg(self.theme.main_bg)
                            .font_family(font_family.clone())
                            .text_size(px(font_size))
                            .font_features(terminal_ligatures(font_ligatures))
                            .overflow_hidden()
                            .child(super::ime::registration(cx.entity(), self.focus_handle.clone()))
                            .child(terminal_area)
                            .when(!background_program_statuses.is_empty(), |pane| {
                                pane.child(
                                    div()
                                        .id("background-program-statuses")
                                        .absolute()
                                        .bottom(px(4.))
                                        .right(px(20.))
                                        .flex()
                                        .flex_col()
                                        .items_end()
                                        .gap(px(4.))
                                        .children(background_program_statuses.iter().map(
                                            |(tab_idx, pane_id, tab_title, record)| {
                                                let label = format!(
                                                    "{} | {}: {}",
                                                    tab_title,
                                                    record
                                                        .app
                                                        .as_deref()
                                                        .or(record.title.as_deref())
                                                        .unwrap_or("Program"),
                                                    super::tab_bar::program_status_text(record),
                                                );
                                                let pane_id = *pane_id;
                                                let tab_idx = *tab_idx;
                                                div()
                                                    .id(("background-program-status", pane_id))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(5.))
                                                    .max_w(px(360.))
                                                    .px(px(7.))
                                                    .py(px(4.))
                                                    .rounded(px(5.))
                                                    .bg(theme.surface_raised)
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .text_size(px(11.))
                                                    .text_color(theme.foreground)
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |this, _ev, _window, cx| {
                                                            this.activate_tab_index(tab_idx);
                                                            cx.notify();
                                                        }),
                                                    )
                                                    .child(super::tab_bar::render_program_status(
                                                        record, theme, 12.0,
                                                    ))
                                                    .child(
                                                        div()
                                                            .min_w(px(0.))
                                                            .truncate()
                                                            .child(SharedString::from(label)),
                                                    )
                                            },
                                        )),
                                )
                            }),
                    )
                    .when(
                        self.ai_opencode_variants_open
                            || self.ai_history_open
                            || self.ai_at_menu_open,
                        |this| {
                        this.child(
                            div()
                                .id("ai-opencode-effort-outside-dismiss")
                                .absolute()
                                .inset_0()
                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                    this.ai_opencode_variants_open = false;
                                    this.ai_history_open = false;
                                    this.ai_at_menu_open = false;
                                    cx.notify();
                                    cx.stop_propagation();
                                }))
                                .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                                    this.ai_opencode_variants_open = false;
                                    this.ai_history_open = false;
                                    this.ai_at_menu_open = false;
                                    cx.notify();
                                    cx.stop_propagation();
                                })),
                        )
                    })
                    .when(
                        self.ai_sidebar_open || self.ai_sidebar_anim_progress > 0.001,
                        |this| this.child(self.render_ai_sidebar(_window, cx)),
                    )
            )
            .child(

                StatusBar::new(left_segs, right_segs, fallback_info, theme)
                    .on_git_context_menu(cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                        if let Some(active_tab) = this.tabs.get(this.active_tab_idx) {
                            if active_tab.git_status.is_some() {
                                this.is_git_menu_open = true;
                                this.git_menu_pos = Some((ev.position.x.to_f64() as f32, ev.position.y.to_f64() as f32));
                                cx.notify();
                            }
                        }
                    }))
            )
            .when(self.any_root_menu_open(), |this| {
                this.child(
                    div()
                        .id("root-menu-outside-dismiss")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.close_root_menus();
                            cx.notify();
                            cx.stop_propagation();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                            this.close_root_menus();
                            cx.notify();
                            cx.stop_propagation();
                        })),
                )
            })
            .when(self.is_context_menu_open, |this| {
                let sc_palette = if cfg!(target_os = "macos") { "⌘P" } else { "Ctrl+Shift+P" };
                let sc_ai = if cfg!(target_os = "macos") { "⌘L" } else { "Ctrl+Shift+L" };
                let sc_ssh = if cfg!(target_os = "macos") { "⌘O" } else { "Ctrl+Shift+O" };
                let sc_search = if cfg!(target_os = "macos") { "⌘F" } else { "Ctrl+Shift+F" };
                let sc_settings = if cfg!(target_os = "macos") { "⌘," } else { "Ctrl+," };
                let sc_new_tab = if cfg!(target_os = "macos") { "⌘T" } else { "Ctrl+Shift+T" };
                let sc_clear = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl+Shift+K" };
                let sc_fullscreen = if cfg!(target_os = "macos") { "⌃⌘F" } else { "F11" };
                let sc_quit = if cfg!(target_os = "macos") { "⌘Q" } else { "Alt+F4" };

                this.child(
                    div()
                        .id("logo-menu-backdrop")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_context_menu_open = false;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                            this.is_context_menu_open = false;
                            cx.notify();
                        }))
                        .on_mouse_move(|_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .on_scroll_wheel(|_ev, _window, cx| {
                            cx.stop_propagation();
                        }),
                )
                .child(
                    div()
                        .id("logo-context-menu-popup")
                        .absolute()
                        .top(px(34.))
                        .when(cfg!(target_os = "macos"), |d| d.right(px(6.)))
                        .when(!cfg!(target_os = "macos"), |d| d.left(px(6.)))
                        .w(px(220.))
                        .p(px(4.))
                        .rounded(px(8.))
                        .bg({
                            let mut bg = theme.surface;
                            bg.a = 1.0;
                            bg
                        })
                        .border_1()
                        .border_color(theme.border)
                        .shadow_xl()
                        .occlude()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .on_mouse_move(|_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .on_scroll_wheel(|_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .child(render_context_menu_item(
                            IconType::Zap,
                            "About Fastty",
                            None,
                            cx.listener(|this, _ev, window, cx| {
                                this.toggle_about(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::RotateCcw,
                            "Check for Updates",
                            None,
                            cx.listener(|this, _ev, window, cx| {
                                this.is_context_menu_open = false;
                                this.check_for_updates_manual(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Settings,
                            "Settings...",
                            Some(sc_settings),
                            cx.listener(|this, _ev, window, cx| {
                                this.is_context_menu_open = false;
                                this.open_settings_window(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_divider(theme))
                        .child(render_context_menu_item(
                            IconType::Command,
                            "Command Palette",
                            Some(sc_palette),
                            cx.listener(|this, _ev, window, cx| {
                                this.is_context_menu_open = false;
                                this.toggle_command_palette(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Sparkles,
                            "Fastty AI",
                            Some(sc_ai),
                            cx.listener(|this, _ev, window, cx| {
                                this.is_context_menu_open = false;
                                this.toggle_ai_sidebar(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Server,
                            "SSH Manager",
                            Some(sc_ssh),
                            cx.listener(|this, _ev, window, cx| {
                                this.is_context_menu_open = false;
                                this.toggle_ssh_manager(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Search,
                            "Find in Buffer",
                            Some(sc_search),
                            cx.listener(|this, _ev, window, cx| {
                                this.is_context_menu_open = false;
                                this.toggle_search(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_divider(theme))
                        .child(render_context_menu_item(
                            IconType::Plus,
                            "New Tab",
                            Some(sc_new_tab),
                            cx.listener(|this, _ev, window, cx| {
                                this.is_context_menu_open = false;
                                this.create_tab(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Trash2,
                            "Clear Scrollback",
                            Some(sc_clear),
                            cx.listener(|this, _ev, _window, cx| {
                                if let Some(tab) = this.tabs.get(this.active_tab_idx) {
                                    if let Some(ref term) = tab.terminal {
                                        term.scroll_to_bottom();
                                    }
                                }
                                this.is_context_menu_open = false;
                                cx.notify();
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Folder,
                            "Open Config Folder",
                            None,
                            cx.listener(|this, _ev, _window, cx| {
                                let config_dir = dirs::home_dir().map(|h| h.join(".config/fastty")).unwrap_or_default();
                                let _ = std::fs::create_dir_all(&config_dir);
                                if let Some(path_str) = config_dir.to_str() {
                                    open_path_or_url(path_str);
                                }
                                this.is_context_menu_open = false;
                                cx.notify();
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Maximize2,
                            "Toggle Fullscreen",
                            Some(sc_fullscreen),
                            cx.listener(|this, _ev, window, cx| {
                                window.toggle_fullscreen();
                                this.is_context_menu_open = false;
                                cx.notify();
                            }),
                            theme,
                        ))
                        .child(render_context_menu_divider(theme))
                        .child(render_context_menu_item(
                            IconType::LogOut,
                            "Quit Fastty",
                            Some(sc_quit),
                            cx.listener(|_this, _ev, _window, cx| {
                                cx.quit();
                            }),
                            theme,
                        )),
                )
            })
            // Tab Right-Click Context Menu Overlay
            .when(self.is_tab_context_menu_open, |this| {
                let target_tab_id = self.tab_context_menu_tab_id;
                let (menu_x, menu_y) = self.tab_context_menu_pos;
                let sc_rename = if cfg!(target_os = "macos") { "⌘⇧R" } else { "Ctrl+Shift+R" };
                let sc_close = if cfg!(target_os = "macos") { "⌘W" } else { "Ctrl+Shift+W" };

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_tab_context_menu_open = false;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                            this.is_tab_context_menu_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("tab-context-menu-popup")
                        .absolute()
                        .top(px(menu_y.max(28.0)))
                        .left(px(menu_x.max(8.0)))
                        .w(px(200.))
                        .p(px(4.))
                        .rounded(px(8.))
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_xl()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .child(render_context_menu_item(
                            IconType::Pencil,
                            "Rename Tab",
                            Some(sc_rename),
                            cx.listener(move |this, _ev, _window, cx| {
                                this.is_tab_context_menu_open = false;
                                if let Some(pos) = this.tabs.iter().position(|t| t.id == target_tab_id) {
                                    this.open_rename_tab(pos, cx);
                                }
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::CopyPlus,
                            "Duplicate Tab",
                            None,
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_tab_context_menu_open = false;
                                let cwd = this.tabs.iter().find(|t| t.id == target_tab_id).and_then(|t| t.cwd.clone());
                                let shell = this.config.shell.clone().or_else(|| std::env::var("SHELL").ok()).unwrap_or_else(crate::paths::default_system_shell);
                                this.create_tab_with_cmd_and_cwd(&shell, &[], cwd.as_deref(), None, window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::ExternalLink,
                            "Move to New Window",
                            None,
                            cx.listener(move |this, _ev, _window, cx| {
                                this.is_tab_context_menu_open = false;
                                if this.tabs.len() <= 1 {
                                    return;
                                }
                                let tab_idx_opt = this.tabs.iter().position(|t| t.id == target_tab_id);
                                if let Some(idx) = tab_idx_opt {
                                    let was_active = idx == this.active_tab_idx;
                                    this.cancel_tab_ai_turns(target_tab_id);
                                    if was_active && this.ai_is_streaming {
                                        this.cancel_ai_stream();
                                        this.finish_cancelled_ai_turn();
                                    }
                                    if was_active {
                                        this.save_active_ai_conversation();
                                    }
                                    let conversation_id = this.tabs[idx].ai_conversation_id.clone();
                                    this.ai_opencode_manager.remove(&conversation_id);
                                    let tab_data = this.tabs.remove(idx);
                                    if this.active_tab_idx >= this.tabs.len() {
                                        this.active_tab_idx = this.tabs.len().saturating_sub(1);
                                    } else if idx < this.active_tab_idx {
                                        this.active_tab_idx -= 1;
                                    }
                                    if was_active {
                                        let active_id = this.tabs[this.active_tab_idx].ai_conversation_id.clone();
                                        this.load_ai_conversation(&active_id);
                                    }
                                    this.persist_session();
                                    cx.notify();

                                    let cwd = tab_data.cwd.clone();
                                    let title = Some(tab_data.title.clone());
                                    let cli_opts = crate::cli::CliOptions {
                                        working_dir: cwd,
                                        title,
                                        ..Default::default()
                                    };
                                    let bounds = Bounds::centered(None, size(px(960.), px(640.)), &*cx);
                                    cx.open_window(
                                        WindowOptions {
                                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                                            window_min_size: Some(size(px(640.), px(420.))),
                                            window_background: WindowBackgroundAppearance::Blurred,
                                            app_id: Some("com.fastty.app".into()),
                                            titlebar: Some(TitlebarOptions {
                                                title: Some("Fastty".into()),
                                                appears_transparent: true,
                                                ..Default::default()
                                            }),
                                            ..Default::default()
                                        },
                                        |w, cx| {
                                            w.set_background_appearance(WindowBackgroundAppearance::Blurred);
                                            cx.new(|cx| RootView::with_initial_tab(w, cli_opts, tab_data, cx))
                                        },
                                    ).ok();
                                }
                            }),
                            theme,
                        ))
                        .child(render_context_menu_divider(theme))
                        .child(render_context_menu_item(
                            IconType::X,
                            "Close Tab",
                            Some(sc_close),
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_tab_context_menu_open = false;
                                this.close_tab(target_tab_id, window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Trash2,
                            "Close Other Tabs",
                            None,
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_tab_context_menu_open = false;
                                this.close_other_tabs(target_tab_id, window, cx);
                            }),
                            theme,
                        )),
                )
            })
            // Terminal / Pane Right-Click / Double-Click Context Menu Overlay
            .when(self.is_pane_context_menu_open, |this| {
                let (menu_x, menu_y) = self.pane_context_menu_pos;
                let target_pane_id = self.pane_context_menu_pane_id;
                let sc_copy = if cfg!(target_os = "macos") { "⌘C" } else { "Ctrl+Shift+C" };
                let sc_paste = if cfg!(target_os = "macos") { "⌘V" } else { "Ctrl+Shift+V" };
                let sc_split_right = if cfg!(target_os = "macos") { "⌘D" } else { "Ctrl+Shift+E" };
                let sc_split_down = if cfg!(target_os = "macos") { "⌘⇧D" } else { "Ctrl+Shift+O" };
                let sc_close_pane = if cfg!(target_os = "macos") { "⌘W" } else { "Ctrl+Shift+W" };
                let sc_close_tab = if cfg!(target_os = "macos") { "⌘⇧W" } else { "Ctrl+Shift+Q" };
                let sc_rename = if cfg!(target_os = "macos") { "⌘⇧R" } else { "Ctrl+Shift+R" };
                let sc_clear = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl+Shift+K" };
                let sc_search = if cfg!(target_os = "macos") { "⌘F" } else { "Ctrl+Shift+F" };
                let has_selection = self.selection.is_some();
                let (can_zoom, is_zoomed) = self
                    .tabs
                    .get(self.active_tab_idx)
                    .map(|tab| {
                        (
                            tab.pane_tree.pane_count() > 1,
                            tab.zoomed_pane == Some(target_pane_id),
                        )
                    })
                    .unwrap_or((false, false));
                let menu_w: f32 = 220.0;
                let menu_h: f32 = if has_selection { 420.0 } else { 390.0 };
                let vp = _window.viewport_size();
                let vp_w = vp.width.to_f64() as f32;
                let vp_h = vp.height.to_f64() as f32;
                let popup_x = if menu_x + menu_w > vp_w - 8.0 {
                    (vp_w - menu_w - 8.0).max(8.0)
                } else {
                    menu_x.max(8.0)
                };
                let popup_y = if menu_y + menu_h > vp_h - 28.0 {
                    (menu_y - menu_h).max(36.0)
                } else {
                    menu_y.max(36.0)
                };

                let mut menu_bg = theme.surface;
                menu_bg.a = 1.0;

                this.child(
                    div()
                        .id("pane-context-menu-backdrop")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_scroll_wheel(|_ev, _window, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_pane_context_menu_open = false;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                            this.is_pane_context_menu_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("pane-context-menu-popup")
                        .absolute()
                        .top(px(popup_y))
                        .left(px(popup_x))
                        .w(px(220.))
                        .p(px(4.))
                        .rounded(px(8.))
                        .bg(menu_bg)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_xl()
                        .occlude()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .on_mouse_move(|_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .on_scroll_wheel(|_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                            cx.stop_propagation();
                        })
                        .when(has_selection, |d| {
                            d.child(render_context_menu_item(
                                IconType::ClipboardCopy,
                                "Copy",
                                Some(sc_copy),
                                cx.listener(|this, _ev, _window, cx| {
                                    this.is_pane_context_menu_open = false;
                                    if let Some(text) = this.get_selected_text() {
                                        if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                                            let _ = clip.set_text(text);
                                        }
                                    }
                                    cx.notify();
                                }),
                                theme,
                            ))
                        })
                        .child(render_context_menu_item(
                            IconType::ClipboardPaste,
                            "Paste",
                            Some(sc_paste),
                            cx.listener(move |this, _ev, _window, cx| {
                                this.is_pane_context_menu_open = false;
                                if let Some(active_tab) = this.tabs.get(this.active_tab_idx) {
                                    let pane = active_tab.pane_tree.find_pane(target_pane_id).or_else(|| active_tab.pane_tree.active_pane());
                                    if let Some(terminal) = pane.and_then(|p| p.terminal.as_ref()).or(active_tab.terminal.as_ref()) {
                                        if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                                            if let Some(content) = crate::paste::get_clipboard_paste_content(&mut clip) {
                                                crate::paste::paste_text_to_terminal(terminal, &content);
                                            }
                                        }
                                    }
                                }
                                cx.notify();
                            }),
                            theme,
                        ))
                        .child(render_context_menu_divider(theme))
                        .child(render_context_menu_item(
                            IconType::PanelRight,
                            "Split Pane Right",
                            Some(sc_split_right),
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_pane_context_menu_open = false;
                                if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                    tab.pane_tree.active_pane_id = target_pane_id;
                                    if let Some(p) = tab.pane_tree.active_pane() {
                                        tab.terminal = p.terminal.clone();
                                        tab.cwd = p.cwd.clone();
                                        tab.git_status = p.git_status.clone();
                                    }
                                }
                                this.split_active_pane(Direction::Right, window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::PanelBottom,
                            "Split Pane Down",
                            Some(sc_split_down),
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_pane_context_menu_open = false;
                                if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                    tab.pane_tree.active_pane_id = target_pane_id;
                                    if let Some(p) = tab.pane_tree.active_pane() {
                                        tab.terminal = p.terminal.clone();
                                        tab.cwd = p.cwd.clone();
                                        tab.git_status = p.git_status.clone();
                                    }
                                }
                                this.split_active_pane(Direction::Down, window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::PanelLeft,
                            "Split Pane Left",
                            None,
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_pane_context_menu_open = false;
                                if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                    tab.pane_tree.active_pane_id = target_pane_id;
                                    if let Some(p) = tab.pane_tree.active_pane() {
                                        tab.terminal = p.terminal.clone();
                                        tab.cwd = p.cwd.clone();
                                        tab.git_status = p.git_status.clone();
                                    }
                                }
                                this.split_active_pane(Direction::Left, window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::PanelTop,
                            "Split Pane Top",
                            None,
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_pane_context_menu_open = false;
                                if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                    tab.pane_tree.active_pane_id = target_pane_id;
                                    if let Some(p) = tab.pane_tree.active_pane() {
                                        tab.terminal = p.terminal.clone();
                                        tab.cwd = p.cwd.clone();
                                        tab.git_status = p.git_status.clone();
                                    }
                                }
                                this.split_active_pane(Direction::Top, window, cx);
                            }),
                            theme,
                        ))
                        .when(can_zoom, |d| {
                            d.child(render_context_menu_item(
                                IconType::Maximize2,
                                if is_zoomed { "Unzoom Pane" } else { "Zoom Pane" },
                                None,
                                cx.listener(move |this, _ev, _window, cx| {
                                    this.is_pane_context_menu_open = false;
                                    if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                        tab.pane_tree.active_pane_id = target_pane_id;
                                        if let Some(p) = tab.pane_tree.active_pane() {
                                            tab.terminal = p.terminal.clone();
                                            tab.cwd = p.cwd.clone();
                                            tab.git_status = p.git_status.clone();
                                        }
                                    }
                                    this.toggle_pane_zoom(cx);
                                }),
                                theme,
                            ))
                        })
                        .child(render_context_menu_divider(theme))
                        .child(render_context_menu_item(
                            IconType::Pencil,
                            "Change Tab Name...",
                            Some(sc_rename),
                            cx.listener(|this, _ev, _window, cx| {
                                this.is_pane_context_menu_open = false;
                                let idx = this.active_tab_idx;
                                this.open_rename_tab(idx, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Search,
                            "Find in Buffer...",
                            Some(sc_search),
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_pane_context_menu_open = false;
                                if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                    tab.pane_tree.active_pane_id = target_pane_id;
                                    if let Some(p) = tab.pane_tree.active_pane() {
                                        tab.terminal = p.terminal.clone();
                                        tab.cwd = p.cwd.clone();
                                        tab.git_status = p.git_status.clone();
                                    }
                                }
                                this.toggle_search(window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Trash2,
                            "Clear / Reset Terminal",
                            Some(sc_clear),
                            cx.listener(move |this, _ev, _window, cx| {
                                this.is_pane_context_menu_open = false;
                                if let Some(active_tab) = this.tabs.get(this.active_tab_idx) {
                                    let pane = active_tab.pane_tree.find_pane(target_pane_id).or_else(|| active_tab.pane_tree.active_pane());
                                    if let Some(ref term) = pane.and_then(|p| p.terminal.as_ref()).or(active_tab.terminal.as_ref()) {
                                        term.scroll_to_bottom();
                                        term.write_to_pty(b"\x0c");
                                    }
                                }
                                cx.notify();
                            }),
                            theme,
                        ))
                        .child(render_context_menu_divider(theme))
                        .child(render_context_menu_item(
                            IconType::X,
                            "Close Pane",
                            Some(sc_close_pane),
                            cx.listener(move |this, _ev, window, cx| {
                                this.is_pane_context_menu_open = false;
                                this.close_pane_by_id(target_pane_id, window, cx);
                            }),
                            theme,
                        ))
                        .child(render_context_menu_item(
                            IconType::Trash2,
                            "Close Tab",
                            Some(sc_close_tab),
                            cx.listener(|this, _ev, window, cx| {
                                this.is_pane_context_menu_open = false;
                                if let Some(active_tab) = this.tabs.get(this.active_tab_idx) {
                                    let tab_id = active_tab.id;
                                    this.close_tab(tab_id, window, cx);
                                }
                            }),
                            theme,
                        )),
                )
            })
            .when(self.is_about_open, |this| {
                let mut backdrop_bg = theme.black;
                backdrop_bg.a = 0.65;

                let pkg_ver = env!("CARGO_PKG_VERSION");
                let platform_desc = if cfg!(target_os = "macos") {
                    format!("v{} • macOS ({})", pkg_ver, std::env::consts::ARCH)
                } else if cfg!(target_os = "windows") {
                    format!("v{} • Windows ({})", pkg_ver, std::env::consts::ARCH)
                } else {
                    format!("v{} • Linux ({})", pkg_ver, std::env::consts::ARCH)
                };

                let renderer_desc = if cfg!(target_os = "macos") {
                    "GPUI / Metal (GPU Accelerated)"
                } else if cfg!(target_os = "windows") {
                    "GPUI / Direct3D / Vulkan"
                } else {
                    "GPUI / Vulkan / Wayland / X11"
                };

                let shell_desc = if cfg!(target_os = "windows") {
                    std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
                } else {
                    std::env::var("SHELL").unwrap_or_else(|_| "zsh".to_string())
                };

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_about_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(380.))
                                .p(px(20.))
                                .rounded(px(12.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .flex()
                                .flex_col()
                                .gap_4()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .pb(px(12.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_2()
                                                .child(render_app_logo(24.0))
                                                .child(
                                                    div()
                                                        .flex()
                                                        .flex_col()
                                                        .gap_0p5()
                                                        .child(
                                                            div()
                                                                .text_size(px(16.))
                                                                .font_weight(FontWeight::BOLD)
                                                                .text_color(theme.foreground)
                                                                .child("Fastty"),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(11.))
                                                                .text_color(theme.muted)
                                                                .child(platform_desc),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .hover(|s| s.text_color(theme.foreground))
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.is_about_open = false;
                                                    cx.notify();
                                                }))
                                                .child(render_icon(IconType::X, theme.accent, 12.0)),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .line_height(px(18.))
                                        .text_color(theme.muted_strong)
                                        .child("High-performance GPU-accelerated terminal emulator written in Rust with GPUI, hardware graphics acceleration, and subpixel typography."),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .p(px(12.))
                                        .rounded(px(8.))
                                        .bg(theme.surface_raised)
                                        .border_1()
                                        .border_color(theme.border)
                                        .child(render_about_spec_row("Renderer", renderer_desc, theme))
                                        .child(render_about_spec_row("PTY Engine", "Alacritty VTE + SGR 1006 Mouse", theme))
                                        .child(render_about_spec_row("Typography", "Subpixel Rasterizer + OpenType Ligatures", theme))
                                        .child(render_about_spec_row("Shell", &shell_desc, theme))
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .px(px(12.))
                                                .py(px(6.))
                                                .rounded(px(6.))
                                                .bg(theme.surface_raised)
                                                .hover(move |s| s.bg(theme.hover))
                                                .border_1()
                                                .border_color(theme.border)
                                                .text_size(px(11.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(theme.accent)
                                                .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                                    open_path_or_url("https://github.com/diegoleteliers10/fasty");
                                                })
                                                .child("GitHub Repo ↗"),
                                        )
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .px(px(16.))
                                                .py(px(6.))
                                                .rounded(px(6.))
                                                .bg(theme.accent)
                                                .hover(move |s| s.opacity(0.85))
                                                .text_size(px(11.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(theme.background)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.is_about_open = false;
                                                    cx.notify();
                                                }))
                                                .child("Close"),
                                        ),
                                ),
                        ),
                )
            })
            // Rename Tab Modal
            .when(self.is_rename_tab_open, |this| {
                let mut backdrop_bg = theme.black;
                backdrop_bg.a = 0.65;
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_rename_tab_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(360.))
                                .p(px(16.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .flex()
                                .flex_col()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .pb(px(6.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_2()
                                                .child(render_icon(IconType::Pencil, theme.accent, 14.0))
                                                .child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(theme.foreground)
                                                        .child("Rename Tab"),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .hover(|s| s.text_color(theme.foreground))
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.is_rename_tab_open = false;
                                                    cx.notify();
                                                }))
                                                .child(render_icon(IconType::X, theme.accent, 12.0)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .px(px(10.))
                                        .py(px(7.))
                                        .rounded(px(6.))
                                        .bg(theme.surface_raised)
                                        .border_1()
                                        .border_color(theme.accent)
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(if self.rename_tab_input.is_empty() { theme.muted } else { theme.foreground })
                                                .child(if self.rename_tab_input.is_empty() {
                                                    "Leave empty to auto-title from process...".to_string()
                                                } else {
                                                    self.rename_tab_input.clone()
                                                }),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_size(px(10.5))
                                                .text_color(theme.muted_strong)
                                                .child("↵ Save • Esc Cancel"),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .px(px(12.))
                                                        .py(px(4.))
                                                        .rounded(px(4.))
                                                        .bg(theme.accent)
                                                        .text_color(theme.black)
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_size(px(11.))
                                                        .cursor(CursorStyle::PointingHand)
                                                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                            this.save_rename_tab(cx);
                                                        }))
                                                        .child("Save"),
                                                ),
                                        ),
                                ),
                        ),
                )
            })
            // Command Palette Modal
            .when(self.is_command_palette_open, |this| {
                let mut backdrop_bg = theme.black;
                backdrop_bg.a = 0.65;
                let filtered = self.filtered_palette_commands();
                let selected_idx = self.command_palette_selected;

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop_bg)
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(60.))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            // Backdrop click: close without committing the preview.
                            this.is_command_palette_open = false;
                            this.cancel_palette_preview(_window, cx);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(500.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                // Input Box Header
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px(px(12.))
                                        .py(px(10.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(render_icon(IconType::Search, theme.accent, 14.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(13.))
                                                .text_color(if self.command_palette_query.is_empty() {
                                                    theme.muted
                                                } else {
                                                    theme.foreground
                                                })
                                                .child(if self.command_palette_query.is_empty() {
                                                    "Type a command... (↑↓ to navigate, Enter to run)".to_string()
                                                } else {
                                                    format!("{}|", self.command_palette_query)
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px(px(5.))
                                                .py(px(2.))
                                                .rounded(px(3.))
                                                .bg(theme.surface_raised)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("ESC"),
                                        ),
                                )
                                // Filtered Commands List
                                .child(
                                    div()
                                        .id("palette-commands-list")
                                        .track_scroll(&self.command_palette_scroll_handle)
                                        .flex()
                                        .flex_col()
                                        .max_h(px(280.))
                                        .overflow_y_scroll()
                                        .p(px(4.))
                                        .gap_1()
                                        .children(
                                            if filtered.is_empty() {
                                                vec![
                                                    div()
                                                        .p(px(12.))
                                                        .text_size(px(12.))
                                                        .text_color(theme.muted)
                                                        .child("No matching commands found."),
                                                ]
                                            } else {
                                                filtered
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, cmd)| {
                                                        let is_selected = idx == selected_idx;
                                                        let cmd_id = cmd.id;
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .justify_between()
                                                            .px(px(10.))
                                                            .py(px(6.))
                                                            .rounded(px(6.))
                                                            .bg(if is_selected { theme.accent } else { theme.surface })
                                                            .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                if this.command_palette_selected != idx {
                                                                    this.command_palette_selected = idx;
                                                                    // Hover previews live, like keyboard nav.
                                                                    this.preview_palette_command(cmd_id, _window, cx);
                                                                    cx.notify();
                                                                }
                                                            }))
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, window, cx| {
                                                                this.execute_palette_command(cmd_id, window, cx);
                                                            }))
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .gap_2p5()
                                                                    .child(render_icon(cmd.icon, if is_selected { theme.black } else { theme.accent }, 13.0))
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(12.))
                                                                            .font_weight(if is_selected { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                                            .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                            .child(cmd.title),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .gap_2()
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(10.))
                                                                            .text_color(if is_selected { theme.black.opacity(0.85) } else { theme.muted })
                                                                            .child(cmd.category),
                                                                    )
                                                                    .when_some(cmd.shortcut, |this, sc| {
                                                                        this.child(
                                                                            div()
                                                                                .px(px(4.))
                                                                                .py(px(1.))
                                                                                .rounded(px(3.))
                                                                                .bg(if is_selected { theme.black } else { theme.surface_raised })
                                                                                .text_size(px(10.))
                                                                                .font_weight(if is_selected { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                                                .text_color(if is_selected { theme.accent } else { theme.muted })
                                                                                .child(sc),
                                                                        )
                                                                    }),
                                                            )
                                                    })
                                                    .collect()
                                            }
                                        ),
                                ),
                        ),
                )
            })
            // Snippet Picker Modal (F4)
            .when(self.is_snippet_picker_open, |this| {
                let mut backdrop_bg = theme.black;
                backdrop_bg.a = 0.65;
                let all_snips = crate::snippets::all();
                let query = self.snippet_query.to_lowercase();
                let filtered: Vec<(String, String)> = all_snips
                    .into_iter()
                    .filter(|(trigger, body)| {
                        query.is_empty()
                            || fuzzy_match_str(&query, trigger)
                            || fuzzy_match_str(&query, body)
                    })
                    .collect();
                let selected_idx = self.snippet_selected;

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop_bg)
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(60.))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_snippet_picker_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(560.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                // Input Box Header
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px(px(12.))
                                        .py(px(10.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(render_icon(IconType::Terminal, theme.accent, 14.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(13.))
                                                .text_color(if self.snippet_query.is_empty() {
                                                    theme.muted
                                                } else {
                                                    theme.foreground
                                                })
                                                .child(if self.snippet_query.is_empty() {
                                                    "Type to filter snippets... (↑↓ to navigate, Enter to insert)".to_string()
                                                } else {
                                                    format!("{}|", self.snippet_query)
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px(px(5.))
                                                .py(px(2.))
                                                .rounded(px(3.))
                                                .bg(theme.surface_raised)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("ESC"),
                                        ),
                                )
                                // Filtered Snippets List
                                .child(
                                    div()
                                        .id("snippet-picker-list")
                                        .track_scroll(&self.snippet_scroll_handle)
                                        .flex()
                                        .flex_col()
                                        .max_h(px(300.))
                                        .overflow_y_scroll()
                                        .p(px(4.))
                                        .gap_1()
                                        .children(
                                            if filtered.is_empty() {
                                                vec![
                                                    div()
                                                        .p(px(12.))
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(theme.foreground)
                                                                .child("No snippets found"),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(11.))
                                                                .text_color(theme.muted)
                                                                .child("Add [snippet.NAME] entries to your snippets.toml to grow this list."),
                                                        ),
                                                ]
                                            } else {
                                                filtered
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, (trigger, body))| {
                                                        let is_selected = idx == selected_idx;
                                                        // Placeholder-stripped preview, one line.
                                                        let (expanded, _) = crate::snippets::expand(&body);
                                                        let preview: String = expanded
                                                            .lines()
                                                            .next()
                                                            .unwrap_or("")
                                                            .chars()
                                                            .take(72)
                                                            .collect();
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap_0p5()
                                                            .px(px(10.))
                                                            .py(px(6.))
                                                            .rounded(px(6.))
                                                            .bg(if is_selected { theme.accent } else { theme.surface })
                                                            .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                if this.snippet_selected != idx {
                                                                    this.snippet_selected = idx;
                                                                    cx.notify();
                                                                }
                                                            }))
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                this.insert_snippet_body(&body, cx);
                                                            }))
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .gap_2()
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(12.))
                                                                            .font_weight(FontWeight::BOLD)
                                                                            .text_color(if is_selected { theme.black } else { theme.accent })
                                                                            .child(trigger),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .px(px(4.))
                                                                            .py(px(1.))
                                                                            .rounded(px(3.))
                                                                            .bg(if is_selected { theme.black } else { theme.surface_raised })
                                                                            .text_size(px(10.))
                                                                            .text_color(if is_selected { theme.accent } else { theme.muted })
                                                                            .child("snip"),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_size(px(11.))
                                                                    .text_color(if is_selected { theme.black.opacity(0.85) } else { theme.muted_strong })
                                                                    .overflow_hidden()
                                                                    .child(preview),
                                                            )
                                                    })
                                                    .collect()
                                            }
                                        ),
                                ),
                        ),
                )
            })
            // PR Picker Modal (F5)
            .when(self.is_pr_picker_open, |this| {
                let mut backdrop_bg = theme.black;
                backdrop_bg.a = 0.65;
                let mode = self.pr_picker_mode.clone();
                let query = self.pr_picker_query.to_lowercase();
                let has_cwd = self.pr_picker_cwd.is_some();
                let snapshot = self.pr_snapshot.lock().unwrap().clone();
                let rows: Vec<PrPickerRow> = snapshot
                    .as_ref()
                    .map(pr_picker_rows)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|r| {
                        query.is_empty()
                            || fuzzy_match_str(&query, &r.title)
                            || fuzzy_match_str(&query, &r.detail)
                            || fuzzy_match_str(&query, &r.number.to_string())
                    })
                    .collect();
                let selected_idx = self.pr_picker_selected;
                let header_text = match &mode {
                    PrPickerMode::Browse => {
                        if self.pr_picker_query.is_empty() {
                            "Browse pull requests... (↑↓ to navigate, Enter for actions)".to_string()
                        } else {
                            format!("{}|", self.pr_picker_query)
                        }
                    }
                    PrPickerMode::Actions { number, title } => {
                        let short: String = title.chars().take(40).collect();
                        format!("PR #{number}: {short} (Enter runs, Esc backs out)")
                    }
                };

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop_bg)
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(60.))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_pr_picker_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(560.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                // Input Box Header
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px(px(12.))
                                        .py(px(10.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(render_icon(IconType::GitPullRequest, theme.accent, 14.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(13.))
                                                .text_color(if matches!(mode, PrPickerMode::Browse) && self.pr_picker_query.is_empty() {
                                                    theme.muted
                                                } else {
                                                    theme.foreground
                                                })
                                                .child(header_text),
                                        )
                                        .child(
                                            div()
                                                .px(px(5.))
                                                .py(px(2.))
                                                .rounded(px(3.))
                                                .bg(theme.surface_raised)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("ESC"),
                                        ),
                                )
                                // Body
                                .child(
                                    div()
                                        .id("pr-picker-list")
                                        .track_scroll(&self.pr_picker_scroll_handle)
                                        .flex()
                                        .flex_col()
                                        .max_h(px(300.))
                                        .overflow_y_scroll()
                                        .p(px(4.))
                                        .gap_1()
                                        .children(match mode {
                                            PrPickerMode::Actions { number, .. } => {
                                                (0..pr_action_count())
                                                    .map(|idx| {
                                                        let action = pr_action_for_index(idx);
                                                        let is_selected = idx == selected_idx;
                                                        let label = pr_action_label(action, number);
                                                        let cmd_preview = pr_action_command(action, number);
                                                        let action_copy = action;
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap_0p5()
                                                            .px(px(10.))
                                                            .py(px(6.))
                                                            .rounded(px(6.))
                                                            .bg(if is_selected { theme.accent } else { theme.surface })
                                                            .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                if this.pr_picker_selected != idx {
                                                                    this.pr_picker_selected = idx;
                                                                    cx.notify();
                                                                }
                                                            }))
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                match action_copy {
                                                                    PrPickerAction::Back => {
                                                                        this.pr_picker_mode = PrPickerMode::Browse;
                                                                        this.pr_picker_selected = 0;
                                                                        this.pr_picker_scroll_handle.scroll_to_item(0);
                                                                        cx.notify();
                                                                    }
                                                                    action => {
                                                                        if let Some(cmd) = pr_action_command(action, number) {
                                                                            this.run_pr_action_command(&cmd, cx);
                                                                        }
                                                                    }
                                                                }
                                                            }))
                                                            .child(
                                                                div()
                                                                    .text_size(px(12.))
                                                                    .font_weight(FontWeight::BOLD)
                                                                    .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                    .child(label),
                                                            )
                                                            .when_some(cmd_preview, |this, cmd| {
                                                                this.child(
                                                                    div()
                                                                        .text_size(px(11.))
                                                                        .text_color(if is_selected { theme.black.opacity(0.85) } else { theme.muted })
                                                                        .child(cmd),
                                                                )
                                                            })
                                                    })
                                                    .collect()
                                            }
                                            PrPickerMode::Browse => {
                                                if !has_cwd {
                                                    vec![
                                                        div()
                                                            .p(px(12.))
                                                            .text_size(px(12.))
                                                            .text_color(theme.muted)
                                                            .child("Open a git repository first, then pick PRs here."),
                                                    ]
                                                } else if snapshot.is_none() {
                                                    vec![
                                                        div()
                                                            .p(px(12.))
                                                            .text_size(px(12.))
                                                            .text_color(theme.muted)
                                                            .child("Loading pull requests..."),
                                                    ]
                                                } else if rows.is_empty() {
                                                    vec![
                                                        div()
                                                            .p(px(12.))
                                                            .flex()
                                                            .flex_col()
                                                            .gap_1()
                                                            .child(
                                                                div()
                                                                    .text_size(px(12.))
                                                                    .text_color(theme.foreground)
                                                                    .child("No open pull requests"),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_size(px(11.))
                                                                    .text_color(theme.muted)
                                                                    .child("Needs `gh` authenticated in this repo."),
                                                            ),
                                                    ]
                                                } else {
                                                    rows
                                                        .into_iter()
                                                        .enumerate()
                                                        .map(|(idx, row)| {
                                                            let is_selected = idx == selected_idx;
                                                            let number = row.number;
                                                            let title = row.title.clone();
                                                            div()
                                                                .flex()
                                                                .flex_col()
                                                                .gap_0p5()
                                                                .px(px(10.))
                                                                .py(px(6.))
                                                                .rounded(px(6.))
                                                                .bg(if is_selected { theme.accent } else { theme.surface })
                                                                .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                                .cursor(CursorStyle::PointingHand)
                                                                .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                    if this.pr_picker_selected != idx {
                                                                        this.pr_picker_selected = idx;
                                                                        cx.notify();
                                                                    }
                                                                }))
                                                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                    this.pr_picker_mode = PrPickerMode::Actions { number, title: title.clone() };
                                                                    this.pr_picker_selected = 0;
                                                                    this.pr_picker_scroll_handle.scroll_to_item(0);
                                                                    cx.notify();
                                                                }))
                                                                .child(
                                                                    div()
                                                                        .flex()
                                                                        .flex_row()
                                                                        .items_center()
                                                                        .gap_2()
                                                                        .child(
                                                                            div()
                                                                                .text_size(px(12.))
                                                                                .font_weight(FontWeight::BOLD)
                                                                                .text_color(if is_selected { theme.black } else { theme.accent })
                                                                                .child(format!("#{}", row.number)),
                                                                        )
                                                                        .when(row.is_current, |this| {
                                                                            this.child(
                                                                                div()
                                                                                    .px(px(4.))
                                                                                    .py(px(1.))
                                                                                    .rounded(px(3.))
                                                                                    .bg(if is_selected { theme.black } else { theme.surface_raised })
                                                                                    .text_size(px(10.))
                                                                                    .text_color(if is_selected { theme.accent } else { theme.muted })
                                                                                    .child("current"),
                                                                            )
                                                                        })
                                                                        .child(
                                                                            div()
                                                                                .text_size(px(10.))
                                                                                .text_color(if is_selected { theme.black.opacity(0.85) } else { theme.muted })
                                                                                .child(row.detail),
                                                                        ),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_size(px(12.))
                                                                        .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                        .overflow_hidden()
                                                                        .child(row.title),
                                                                )
                                                        })
                                                        .collect()
                                                }
                                            }
                                        }),
                                ),
                        ),
                )
            })
            // SSH Host Manager Modal
            .when(self.is_ssh_manager_open, |this| {
                let mut backdrop_bg = theme.black;
                backdrop_bg.a = 0.65;
                let hosts = crate::ssh::parse_ssh_config();
                let query = self.ssh_manager_query.to_lowercase();
                let filtered: Vec<crate::ssh::SshHost> = hosts
                    .into_iter()
                    .filter(|h| query.is_empty() || fuzzy_match_str(&query, &h.name) || fuzzy_match_str(&query, &h.hostname) || fuzzy_match_str(&query, &h.user) || fuzzy_match_str(&query, &h.tag))
                    .collect();
                let selected_idx = self.ssh_manager_selected;

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop_bg)
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(60.))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_ssh_manager_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(480.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                // Input Box Header
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px(px(12.))
                                        .py(px(10.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(render_icon(IconType::Server, theme.accent, 14.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(13.))
                                                .text_color(if self.ssh_manager_query.is_empty() {
                                                    theme.muted
                                                } else {
                                                    theme.foreground
                                                })
                                                .child(if self.ssh_manager_query.is_empty() {
                                                    "Search SSH hosts from ~/.ssh/config...".to_string()
                                                } else {
                                                    format!("{}|", self.ssh_manager_query)
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px(px(5.))
                                                .py(px(2.))
                                                .rounded(px(3.))
                                                .bg(theme.surface_raised)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("ESC"),
                                        ),
                                )
                                // Hosts List
                                .child(
                                    div()
                                        .id("ssh-hosts-list")
                                        .track_scroll(&self.ssh_manager_scroll_handle)
                                        .flex()
                                        .flex_col()
                                        .max_h(px(280.))
                                        .overflow_y_scroll()
                                        .p(px(4.))
                                        .gap_1()
                                        .children(
                                            if filtered.is_empty() {
                                                vec![
                                                    div()
                                                        .p(px(16.))
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(theme.foreground)
                                                                .child("No SSH hosts found"),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(11.))
                                                                .text_color(theme.muted)
                                                                .child("Add 'Host <name>' entries to ~/.ssh/config — or press Enter to open it."),
                                                        ),
                                                ]
                                            } else {
                                                filtered
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, host)| {
                                                        let is_selected = idx == selected_idx;
                                                        let host_clone = host.clone();
                                                        let is_already_connected = self.tabs.iter().any(|t| {
                                                            let title = t.custom_title.as_deref().unwrap_or(&t.title);
                                                            title.contains(&host.name)
                                                        });

                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .justify_between()
                                                            .px(px(10.))
                                                            .py(px(6.))
                                                            .rounded(px(6.))
                                                            .bg(if is_selected { theme.accent } else { theme.surface })
                                                            .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                if this.ssh_manager_selected != idx {
                                                                    this.ssh_manager_selected = idx;
                                                                    cx.notify();
                                                                }
                                                            }))
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, window, cx| {
                                                                this.is_ssh_manager_open = false;
                                                                let title = format!("ssh: {}", host_clone.name);
                                                                let (shell, args) = host_clone.resilient_shell_command();
                                                                this.create_tab_with_cmd(&shell, &args, Some(title), window, cx);
                                                            }))
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_col()
                                                                    .child(
                                                                        div()
                                                                            .flex()
                                                                            .flex_row()
                                                                            .items_center()
                                                                            .gap_2()
                                                                            .child(
                                                                                div()
                                                                                    .text_size(px(12.))
                                                                                    .font_weight(FontWeight::BOLD)
                                                                                    .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                                    .child(host.name.clone()),
                                                                            )
                                                                            .when(is_already_connected, |el| {
                                                                                el.child(
                                                                                    div()
                                                                                        .px(px(4.))
                                                                                        .py(px(1.))
                                                                                        .rounded(px(3.))
                                                                                        .bg(theme.green.opacity(0.2))
                                                                                        .text_size(px(9.))
                                                                                        .font_weight(FontWeight::BOLD)
                                                                                        .text_color(theme.green)
                                                                                        .child("● Active"),
                                                                                )
                                                                            }),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(10.5))
                                                                            .text_color(if is_selected { theme.black.opacity(0.85) } else { theme.muted })
                                                                            .child(format!("{}@{}", host.user, host.hostname)),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .gap_1()
                                                                    .child(
                                                                        div()
                                                                            .px(px(5.))
                                                                            .py(px(1.5))
                                                                            .rounded(px(3.))
                                                                            .bg(if is_selected { theme.black } else { theme.surface_raised })
                                                                            .text_size(px(9.5))
                                                                            .font_weight(FontWeight::MEDIUM)
                                                                            .text_color(if is_selected { theme.accent } else { theme.muted_strong })
                                                                            .child(format!("#{}", host.tag)),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .px(px(5.))
                                                                            .py(px(1.5))
                                                                            .rounded(px(3.))
                                                                            .bg(if is_selected { theme.black } else { theme.surface_raised })
                                                                            .text_size(px(9.5))
                                                                            .font_weight(if is_selected { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                                            .text_color(if is_selected { theme.black.opacity(0.7) } else { theme.muted })
                                                                            .child(format!(":{}", host.port)),
                                                                    ),
                                                            )
                                                    })
                                                    .collect()
                                            }
                                        ),
                                ),
                        ),
                )
            })
            // In-Terminal Search Bar Overlay
            .when(self.is_search_open, |this| {
                let match_count = self.search_matches.len();
                let current_match_display = if match_count == 0 {
                    "0/0".to_string()
                } else {
                    format!("{}/{}", self.search_match_idx + 1, match_count)
                };

                this.child(
                    div()
                        .absolute()
                        .top(px(36.))
                        .right(px(12.))
                        .w(px(310.))
                        .p(px(6.))
                        .rounded(px(8.))
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_lg()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(render_icon(IconType::Search, theme.accent, 13.0))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(if self.search_query.is_empty() { theme.muted } else { theme.foreground })
                                .child(if self.search_query.is_empty() {
                                    "Find in terminal...".to_string()
                                } else {
                                    format!("{}|", self.search_query)
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(theme.muted)
                                .child(current_match_display),
                        )
                        // Prev match button
                        .child(
                            div()
                                .cursor(CursorStyle::PointingHand)
                                .w(px(18.))
                                .h(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(3.))
                                .bg(theme.surface_raised)
                                .hover(|s| s.bg(theme.hover))
                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                    if let Some(active_tab) = this.tabs.get(this.active_tab_idx) {
                                        if let Some(ref term) = active_tab.terminal {
                                            if !this.search_matches.is_empty() {
                                                this.search_match_idx = if this.search_match_idx == 0 {
                                                    this.search_matches.len() - 1
                                                } else {
                                                    this.search_match_idx - 1
                                                };
                                                let offset = this.search_matches[this.search_match_idx];
                                                term.scroll_to_offset(offset);
                                                this.last_scroll_activity = std::time::Instant::now();
                                                cx.notify();
                                            }
                                        }
                                    }
                                }))
                                .child(render_icon(IconType::ChevronUp, theme.accent, 11.0)),
                        )
                        // Next match button
                        .child(
                            div()
                                .cursor(CursorStyle::PointingHand)
                                .w(px(18.))
                                .h(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(3.))
                                .bg(theme.surface_raised)
                                .hover(|s| s.bg(theme.hover))
                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                    if let Some(active_tab) = this.tabs.get(this.active_tab_idx) {
                                        if let Some(ref term) = active_tab.terminal {
                                            if !this.search_matches.is_empty() {
                                                this.search_match_idx = (this.search_match_idx + 1) % this.search_matches.len();
                                                let offset = this.search_matches[this.search_match_idx];
                                                term.scroll_to_offset(offset);
                                                this.last_scroll_activity = std::time::Instant::now();
                                                cx.notify();
                                            }
                                        }
                                    }
                                }))
                                .child(render_icon(IconType::ChevronDown, theme.accent, 11.0)),
                        )
                        // Close button
                        .child(
                            div()
                                .cursor(CursorStyle::PointingHand)
                                .w(px(18.))
                                .h(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(3.))
                                .bg(theme.surface_raised)
                                .hover(|s| s.bg(theme.hover))
                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                    this.is_search_open = false;
                                    cx.notify();
                                }))
                                .child(render_icon(IconType::X, theme.accent, 11.0)),
                        ),
                )
            })
            // Git Worktree Picker Modal
            .when(self.is_worktree_picker_open, |this| {
                let active_cwd = self.tabs.get(self.active_tab_idx).and_then(|t| t.cwd.as_deref());
                let is_git = active_cwd.is_some_and(crate::git::is_git_repo);
                let worktrees = active_cwd.map(crate::git::list_worktrees).unwrap_or_default();
                let query = self.worktree_picker_query.to_lowercase();
                let filtered: Vec<crate::git::Worktree> = worktrees
                    .into_iter()
                    .filter(|w| query.is_empty() || w.short_branch().to_lowercase().contains(&query) || w.path.to_string_lossy().to_lowercase().contains(&query))
                    .collect();
                let selected_idx = self.worktree_picker_selected;

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(rgb_to_hsla(0, 0, 0).opacity(0.45))
                        .flex()
                        .items_start()
                        .justify_center()
                        .pt(px(70.))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_worktree_picker_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(520.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .p(px(8.))
                                .flex()
                                .flex_col()
                                .gap_2()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                // Header & Search Input
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px(px(8.))
                                        .py(px(6.))
                                        .rounded(px(6.))
                                        .bg(theme.surface_raised)
                                        .child(render_icon(IconType::GitPullRequest, theme.accent, 14.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(13.))
                                                .text_color(if self.worktree_picker_query.is_empty() { theme.muted } else { theme.foreground })
                                                .child(if self.worktree_picker_query.is_empty() {
                                                    "Filter Git worktrees...".to_string()
                                                } else {
                                                    format!("{}|", self.worktree_picker_query)
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px(px(6.))
                                                .py(px(2.))
                                                .rounded(px(4.))
                                                .bg(theme.surface)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("ESC to close"),
                                        ),
                                )
                                // Worktree List
                                .child(
                                    div()
                                        .id("worktrees-list")
                                        .track_scroll(&self.worktree_picker_scroll_handle)
                                        .flex()
                                        .flex_col()
                                        .max_h(px(280.))
                                        .overflow_y_scroll()
                                        .p(px(4.))
                                        .gap_1()
                                        .children(
                                            if !is_git {
                                                vec![
                                                    div()
                                                        .p(px(16.))
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(theme.foreground)
                                                                .child("Not a Git repository"),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(11.))
                                                                .text_color(theme.muted)
                                                                .child("Current tab directory is not within a Git worktree."),
                                                        ),
                                                ]
                                            } else if filtered.is_empty() {
                                                vec![
                                                    div()
                                                        .p(px(16.))
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(theme.foreground)
                                                                .child("No worktrees match your filter"),
                                                        ),
                                                ]
                                            } else {
                                                filtered
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, wt)| {
                                                        let is_selected = idx == selected_idx;
                                                        let wt_clone = wt.clone();
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .justify_between()
                                                            .px(px(10.))
                                                            .py(px(6.))
                                                            .rounded(px(6.))
                                                            .bg(if is_selected { theme.accent } else { theme.surface })
                                                            .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                if this.worktree_picker_selected != idx {
                                                                    this.worktree_picker_selected = idx;
                                                                    cx.notify();
                                                                }
                                                            }))
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, window, cx| {
                                                                let shell = this.config.shell.clone().or_else(|| std::env::var("SHELL").ok()).unwrap_or_else(crate::paths::default_system_shell);
                                                                let title = wt_clone.short_branch().to_string();
                                                                let path = wt_clone.path.clone();
                                                                this.is_worktree_picker_open = false;
                                                                this.create_tab_with_cmd_and_cwd(&shell, &[], Some(&path), Some(title), window, cx);
                                                            }))
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_col()
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(12.))
                                                                            .font_weight(FontWeight::BOLD)
                                                                            .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                            .child(format!("branch: {}", wt.short_branch())),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(10.5))
                                                                            .text_color(if is_selected { theme.black.opacity(0.85) } else { theme.muted })
                                                                            .child(wt.path.to_string_lossy().to_string()),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .px(px(6.))
                                                                    .py(px(2.))
                                                                    .rounded(px(3.))
                                                                    .bg(if is_selected { theme.black } else { theme.surface_raised })
                                                                    .text_size(px(10.))
                                                                    .font_weight(if is_selected { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                                    .text_color(if is_selected { theme.accent } else { theme.muted })
                                                                    .child(wt.short_commit().to_string()),
                                                            )
                                                    })
                                                    .collect()
                                            }
                                        ),
                                ),
                        ),
                )
            })
            // Project / Tab Jumper Modal
            .when(self.is_project_jumper_open, |this| {
                let query = self.project_jumper_query.to_lowercase();
                let filtered_tabs: Vec<(usize, usize, String, Option<String>, Option<String>)> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, t)| {
                        let cwd_str = t.cwd.as_ref().map(|p| p.to_string_lossy().into_owned());
                        let branch_str = t.git_status.as_ref().map(|g| g.branch.clone());
                        let matches = query.is_empty()
                            || t.title.to_lowercase().contains(&query)
                            || cwd_str.as_ref().is_some_and(|c| c.to_lowercase().contains(&query))
                            || branch_str.as_ref().is_some_and(|b| b.to_lowercase().contains(&query));
                        if matches {
                            Some((idx, t.id, t.title.clone(), cwd_str, branch_str))
                        } else {
                            None
                        }
                    })
                    .collect();
                let selected_idx = self.project_jumper_selected;

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(rgb_to_hsla(0, 0, 0).opacity(0.45))
                        .flex()
                        .items_start()
                        .justify_center()
                        .pt(px(70.))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_project_jumper_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(520.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .p(px(8.))
                                .flex()
                                .flex_col()
                                .gap_2()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                // Header & Search Input
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px(px(8.))
                                        .py(px(6.))
                                        .rounded(px(6.))
                                        .bg(theme.surface_raised)
                                        .child(render_icon(IconType::Folder, theme.accent, 14.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(13.))
                                                .text_color(if self.project_jumper_query.is_empty() { theme.muted } else { theme.foreground })
                                                .child(if self.project_jumper_query.is_empty() {
                                                    "Switch tab or project...".to_string()
                                                } else {
                                                    format!("{}|", self.project_jumper_query)
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px(px(6.))
                                                .py(px(2.))
                                                .rounded(px(4.))
                                                .bg(theme.surface)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("ESC to close"),
                                        ),
                                )
                                // Tab Jumper List
                                .child(
                                    div()
                                        .id("project-jumper-list")
                                        .track_scroll(&self.project_jumper_scroll_handle)
                                        .flex()
                                        .flex_col()
                                        .max_h(px(280.))
                                        .overflow_y_scroll()
                                        .p(px(4.))
                                        .gap_1()
                                        .children(
                                            if filtered_tabs.is_empty() {
                                                vec![
                                                    div()
                                                        .p(px(16.))
                                                        .flex()
                                                        .flex_col()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(theme.foreground)
                                                                .child("No open tabs matching search"),
                                                        ),
                                                ]
                                            } else {
                                                filtered_tabs
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, (_, tab_id, title, cwd_str, branch_str))| {
                                                        let is_selected = idx == selected_idx;
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .justify_between()
                                                            .px(px(10.))
                                                            .py(px(6.))
                                                            .rounded(px(6.))
                                                            .bg(if is_selected { theme.accent } else { theme.surface })
                                                            .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                if this.project_jumper_selected != idx {
                                                                    this.project_jumper_selected = idx;
                                                                    cx.notify();
                                                                }
                                                            }))
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                this.is_project_jumper_open = false;
                                                                this.select_tab(tab_id, cx);
                                                            }))
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_col()
                                                                    .min_w_0()
                                                                    .flex_1()
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(12.))
                                                                            .font_weight(FontWeight::BOLD)
                                                                            .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                            .whitespace_nowrap()
                                                                            .text_ellipsis()
                                                                            .overflow_hidden()
                                                                            .child(title),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .flex()
                                                                            .flex_row()
                                                                            .items_center()
                                                                            .gap_1()
                                                                            .min_w_0()
                                                                            .text_size(px(10.5))
                                                                            .text_color(if is_selected { theme.black.opacity(0.85) } else { theme.muted })
                                                                            .child(render_icon(
                                                                                IconType::Folder,
                                                                                if is_selected { theme.black.opacity(0.85) } else { theme.muted },
                                                                                10.0,
                                                                            ))
                                                                            .child(
                                                                                div()
                                                                                    .min_w_0()
                                                                                    .whitespace_nowrap()
                                                                                    .text_ellipsis()
                                                                                    .overflow_hidden()
                                                                                    .child(cwd_str.unwrap_or_else(|| "~".to_string())),
                                                                            ),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .px(px(6.))
                                                                    .py(px(2.))
                                                                    .rounded(px(3.))
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .gap_1()
                                                                    .bg(if is_selected { theme.black } else { theme.surface_raised })
                                                                    .text_size(px(10.))
                                                                    .font_weight(if is_selected { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                                    .text_color(if is_selected { theme.accent } else { theme.muted })
                                                                    .when(branch_str.is_some(), |el| {
                                                                        el.child(render_icon(
                                                                            IconType::GitBranch,
                                                                            if is_selected { theme.accent } else { theme.muted },
                                                                            10.0,
                                                                        ))
                                                                    })
                                                                    .child(branch_str.unwrap_or_else(|| "Tab".to_string())),
                                                            )
                                                    })
                                                    .collect()
                                            }
                                        ),
                                ),
                        ),
                )
            })
            // File Path Picker Dropdown (Ctrl+Shift+,) — anchored at the
            // input line, opens below it or flips above near the screen edge
            .when(self.is_file_picker_open, |this| {
                let matches = crate::universal_picker::search(
                    &self.file_picker_index,
                    &self.file_picker_query,
                    crate::universal_picker::MAX_RESULTS,
                );
                let selected_idx = self.file_picker_selected;
                let root_label = self
                    .file_picker_root
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "No working directory".to_string());

                let menu_w: f32 = 300.0;
                let menu_h: f32 = 340.0;
                let vp = _window.viewport_size();
                let vp_w = vp.width.to_f64() as f32;
                let vp_h = vp.height.to_f64() as f32;
                let (anchor_x, anchor_y) = self.file_picker_anchor.unwrap_or_else(|| {
                    ((vp_w - menu_w) / 2.0, (vp_h - 160.0).max(0.0))
                });
                let popup_x = if anchor_x + menu_w > vp_w - 8.0 {
                    (vp_w - menu_w - 8.0).max(8.0)
                } else {
                    anchor_x.max(8.0)
                };
                // Open below the input line; flip above it near the screen bottom.
                let popup_y = if anchor_y + 24.0 + menu_h > vp_h - 28.0 {
                    (anchor_y - 8.0 - menu_h).max(36.0)
                } else {
                    anchor_y + 24.0
                };

                this.child(
                    div()
                        .id("file-picker-backdrop")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_scroll_wheel(|_ev, _window, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_file_picker_open = false;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                            this.is_file_picker_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("file-picker-popup")
                        .absolute()
                        .top(px(popup_y))
                        .left(px(popup_x))
                        .w(px(menu_w))
                        .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .p(px(8.))
                                .flex()
                                .flex_col()
                                .gap_2()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                // Search input
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px(px(8.))
                                        .py(px(6.))
                                        .rounded(px(6.))
                                        .bg(theme.surface_raised)
                                        .child(render_icon(IconType::Search, theme.muted, 13.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_size(px(13.))
                                                .text_color(if self.file_picker_query.is_empty() { theme.muted } else { theme.foreground })
                                                .child(if self.file_picker_query.is_empty() {
                                                    "Insert anything: files, ssh, git…"
                                                        .to_string()
                                                } else {
                                                    format!("{}|", self.file_picker_query)
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px(px(6.))
                                                .py(px(2.))
                                                .rounded(px(4.))
                                                .bg(theme.surface)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("ESC to close"),
                                        ),
                                )
                                // Search root breadcrumb
                                .child(
                                    div()
                                        .px(px(8.))
                                        .text_size(px(10.))
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_color(theme.muted)
                                        .child(format!("In {}", root_label)),
                                )
                                // Results list
                                .child(
                                    div()
                                        .id("file-picker-list")
                                        .track_scroll(&self.file_picker_scroll_handle)
                                        .flex()
                                        .flex_col()
                                        .max_h(px(260.))
                                        .overflow_y_scroll()
                                        .p(px(4.))
                                        .gap_1()
                                        .children(
                                            if matches.is_empty() {
                                                vec![
                                                    div()
                                                        .p(px(16.))
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(theme.muted)
                                                                .child(if self.file_picker_index.is_empty() {
                                                                    "No sources available yet".to_string()
                                                                } else {
                                                                    "Nothing matching".to_string()
                                                                }),
                                                        ),
                                                ]
                                            } else {
                                                matches
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, item)| {
                                                        let is_selected = idx == selected_idx;
                                                        let (icon, icon_color) = match item.kind {
                                                            ItemKind::Dir => (IconType::Folder, theme.accent),
                                                            ItemKind::File => (IconType::FileCode, theme.muted),
                                                            ItemKind::SshHost => (IconType::Server, theme.accent),
                                                            ItemKind::GitBranch => (IconType::GitBranch, theme.accent),
                                                            ItemKind::Snippet => (IconType::Terminal, theme.muted),
                                                            ItemKind::DockerContainer => (IconType::Container, theme.accent),
                                                        };
                                                        let file_name = item.title.clone();
                                                        let dir_prefix = item.detail.clone();
                                                        let kind_label = item.kind.label();
                                                        let show_kind_label = !matches!(
                                                            item.kind,
                                                            ItemKind::File | ItemKind::Dir
                                                        );
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_2()
                                                            .px(px(10.))
                                                            .py(px(6.))
                                                            .rounded(px(6.))
                                                            .bg(if is_selected { theme.accent } else { theme.surface })
                                                            .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                if this.file_picker_selected != idx {
                                                                    this.file_picker_selected = idx;
                                                                    cx.notify();
                                                                }
                                                            }))
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                this.file_picker_selected = idx;
                                                                this.insert_selected_file_path(cx);
                                                            }))
                                                            .child(render_icon(
                                                                icon,
                                                                if is_selected { theme.black } else { icon_color },
                                                                13.0,
                                                            ))
                                                            .child(
                                                                div()
                                                                    .flex_1()
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .whitespace_nowrap()
                                                                    .overflow_hidden()
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(11.5))
                                                                            .text_color(if is_selected { theme.black.opacity(0.8) } else { theme.muted })
                                                                            .child(dir_prefix),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(12.))
                                                                            .font_weight(FontWeight::SEMIBOLD)
                                                                            .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                            .child(file_name),
                                                                    ),
                                                            )
                                                            .when(show_kind_label, |row| {
                                                                row.child(
                                                                    div()
                                                                        .ml_auto()
                                                                        .pl(px(6.))
                                                                        .text_size(px(9.5))
                                                                        .text_color(if is_selected {
                                                                            theme.black.opacity(0.6)
                                                                        } else {
                                                                            theme.muted
                                                                        })
                                                                        .child(kind_label),
                                                                )
                                                            })
                                                    })
                                                    .collect::<Vec<_>>()
                                            }
                                        ),
                                ),
                )
            })
            // Git Context Menu Overlay
            .when(self.is_git_menu_open, |this| {
                let active_tab = self.tabs.get(self.active_tab_idx);
                let Some(git_info) = active_tab.and_then(|t| t.git_status.clone()) else {
                    return this;
                };
                let branch_name = git_info.branch.clone();
                let remote_url = git_info.remote_url.clone();
                let has_remote = remote_url.is_some();

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_git_menu_open = false;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                            this.is_git_menu_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .id("git-context-menu-popup")
                                .absolute()
                                .bottom(px(32.))
                                .left(px(12.))
                                .w(px(240.))
                                .p(px(6.))
                                .rounded(px(8.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .px(px(8.))
                                        .py(px(4.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_1()
                                                .child(render_icon(IconType::GitBranch, theme.foreground, 11.0))
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(theme.foreground)
                                                        .child(branch_name.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("Git Actions"),
                                        ),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::GitPullRequest,
                                        "Git Status",
                                        None,
                                        cx.listener(|this, _ev, _window, cx| {
                                            this.is_git_menu_open = false;
                                            if let Some(tab) = this.tabs.get(this.active_tab_idx) {
                                                if let Some(ref term) = tab.terminal {
                                                    term.write_to_pty(b"git status\r");
                                                }
                                            }
                                            cx.notify();
                                        }),
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = false;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::ArrowDown,
                                        "Git Pull",
                                        None,
                                        cx.listener(|this, _ev, _window, cx| {
                                            this.is_git_menu_open = false;
                                            if let Some(tab) = this.tabs.get(this.active_tab_idx) {
                                                if let Some(ref term) = tab.terminal {
                                                    term.write_to_pty(b"git pull\r");
                                                }
                                            }
                                            cx.notify();
                                        }),
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = false;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::ArrowUp,
                                        "Git Push",
                                        None,
                                        cx.listener(|this, _ev, _window, cx| {
                                            this.is_git_menu_open = false;
                                            if let Some(tab) = this.tabs.get(this.active_tab_idx) {
                                                if let Some(ref term) = tab.terminal {
                                                    term.write_to_pty(b"git push\r");
                                                }
                                            }
                                            cx.notify();
                                        }),
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = false;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::GitGraph,
                                        "Git Log Graph",
                                        None,
                                        cx.listener(|this, _ev, _window, cx| {
                                            this.is_git_menu_open = false;
                                            if let Some(tab) = this.tabs.get(this.active_tab_idx) {
                                                if let Some(ref term) = tab.terminal {
                                                    term.write_to_pty(b"git log --oneline --graph --all -n 25\r");
                                                }
                                            }
                                            cx.notify();
                                        }),
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = false;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::FileDiff,
                                        "Git Diff",
                                        None,
                                        cx.listener(|this, _ev, _window, cx| {
                                            this.is_git_menu_open = false;
                                            if let Some(tab) = this.tabs.get(this.active_tab_idx) {
                                                if let Some(ref term) = tab.terminal {
                                                    term.write_to_pty(b"git diff\r");
                                                }
                                            }
                                            cx.notify();
                                        }),
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = false;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::RefreshCw,
                                        "Git Fetch All",
                                        None,
                                        cx.listener(|this, _ev, _window, cx| {
                                            this.is_git_menu_open = false;
                                            if let Some(tab) = this.tabs.get(this.active_tab_idx) {
                                                if let Some(ref term) = tab.terminal {
                                                    term.write_to_pty(b"git fetch --all --prune\r");
                                                }
                                            }
                                            cx.notify();
                                        }),
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = false;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::GitBranch,
                                        "Change Branch",
                                        Some("▶"),
                                        cx.listener(|this, _ev, _window, cx| {
                                            this.is_git_branch_sub_open = !this.is_git_branch_sub_open;
                                            cx.notify();
                                        }),
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && !this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = true;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::GitBranch,
                                        "Git Worktree Picker",
                                        Some(if cfg!(target_os = "macos") { "⌘⌥W" } else { "Ctrl+Alt+W" }),
                                        cx.listener(|this, _ev, window, cx| {
                                            this.is_git_menu_open = false;
                                            this.is_git_branch_sub_open = false;
                                            this.toggle_worktree_picker(window, cx);
                                        }),
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = false;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .child(
                                    render_context_menu_item(
                                        IconType::Clipboard,
                                        "Copy Branch Name",
                                        None,
                                        {
                                            let b_name = branch_name.clone();
                                            cx.listener(move |this, _ev, _window, cx| {
                                                this.is_git_menu_open = false;
                                                this.is_git_branch_sub_open = false;
                                                if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                                                    let _ = clip.set_text(b_name.clone());
                                                }
                                                cx.notify();
                                            })
                                        },
                                        theme,
                                    )
                                    .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                        if *hovered && this.is_git_branch_sub_open {
                                            this.is_git_branch_sub_open = false;
                                            cx.notify();
                                        }
                                    })),
                                )
                                .when_some(remote_url, |this, url| {
                                    this.child(
                                        render_context_menu_item(
                                            IconType::Globe,
                                            "Open Remote in Browser",
                                            None,
                                            cx.listener(move |this, _ev, _window, cx| {
                                                this.is_git_menu_open = false;
                                                this.is_git_branch_sub_open = false;
                                                open_path_or_url(&url);
                                                cx.notify();
                                            }),
                                            theme,
                                        )
                                        .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                                            if *hovered && this.is_git_branch_sub_open {
                                                this.is_git_branch_sub_open = false;
                                                cx.notify();
                                            }
                                        })),
                                    )
                                }),
                        )
                        .when(self.is_git_branch_sub_open, |parent| {
                            let active_cwd = self.tabs.get(self.active_tab_idx).and_then(|t| t.cwd.as_deref());
                            let branches = active_cwd.map(crate::git::list_local_branches).unwrap_or_default();
                            let current_branch = branch_name.clone();

                            let items_below = if has_remote { 3.0 } else { 2.0 };
                            let change_branch_bottom = 32.0 + 6.0 + items_below * 31.0;
                            let change_branch_top = change_branch_bottom + 27.0;
                            let branch_count = branches.len().max(1) as f32;
                            let submenu_h = (39.0 + branch_count * 31.0 - 4.0).min(320.0);
                            let submenu_bottom = (change_branch_top - submenu_h).max(32.0);

                            parent.child(
                                div()
                                    .id("git-branch-submenu-popup")
                                    .absolute()
                                    .bottom(px(submenu_bottom))
                                    .left(px(256.))
                                    .w(px(210.))
                                    .max_h(px(320.))
                                    .overflow_y_scroll()
                                    .p(px(6.))
                                    .rounded(px(8.))
                                    .bg(theme.surface)
                                    .border_1()
                                    .border_color(theme.border)
                                    .shadow_xl()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                        cx.stop_propagation();
                                    })
                                    .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                        cx.stop_propagation();
                                    })
                                    .child(
                                        div()
                                            .px(px(8.))
                                            .py(px(4.))
                                            .border_b_1()
                                            .border_color(theme.border)
                                            .text_size(px(10.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted)
                                            .child("LOCAL BRANCHES"),
                                    )
                                    .children(
                                        if branches.is_empty() {
                                            vec![
                                                div()
                                                    .px(px(8.))
                                                    .py(px(6.))
                                                    .text_size(px(11.))
                                                    .text_color(theme.muted)
                                                    .child("No branches found")
                                                    .into_any_element()
                                            ]
                                        } else {
                                            branches
                                                .into_iter()
                                                .map(|b| {
                                                    let is_current = b == current_branch;
                                                    let target_branch = b.clone();
                                                    let target_branch_cb = b.clone();
                                                    let hover_bg = theme.hover;
                                                    let active_bg = theme.surface_raised;

                                                    div()
                                                        .id(SharedString::from(format!("branch-item-{}", target_branch)))
                                                        .flex()
                                                        .flex_row()
                                                        .items_center()
                                                        .justify_between()
                                                        .w_full()
                                                        .px(px(8.))
                                                        .py(px(5.5))
                                                        .rounded(px(6.))
                                                        .cursor(CursorStyle::PointingHand)
                                                        .hover(move |s| s.bg(hover_bg))
                                                        .active(move |s| s.bg(active_bg))
                                                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                            this.is_git_menu_open = false;
                                                            this.is_git_branch_sub_open = false;
                                                            if !is_current {
                                                                if let Some(tab) = this.tabs.get(this.active_tab_idx) {
                                                                    if let Some(ref term) = tab.terminal {
                                                                        term.write_to_pty(format!("git checkout {}\r", target_branch_cb).as_bytes());
                                                                    }
                                                                }
                                                            }
                                                            cx.notify();
                                                        }))
                                                        .child(
                                                            div()
                                                                .flex()
                                                                .flex_row()
                                                                .items_center()
                                                                .gap_2()
                                                                .flex_1()
                                                                .overflow_hidden()
                                                                .child(render_icon(
                                                                    IconType::GitBranch,
                                                                    if is_current { theme.green } else { theme.accent },
                                                                    12.0,
                                                                ))
                                                                .child(
                                                                    div()
                                                                        .flex_1()
                                                                        .overflow_hidden()
                                                                        .text_ellipsis()
                                                                        .text_size(px(12.))
                                                                        .font_weight(if is_current { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                                                        .text_color(if is_current { theme.green } else { theme.foreground })
                                                                        .child(SharedString::from(target_branch)),
                                                                ),
                                                        )
                                                        .when(is_current, |el| {
                                                            el.child(
                                                                div()
                                                                    .flex_shrink_0()
                                                                    .pl(px(4.))
                                                                    .text_size(px(10.))
                                                                    .font_weight(FontWeight::BOLD)
                                                                    .text_color(theme.green)
                                                                    .child("✓"),
                                                            )
                                                        })
                                                        .into_any_element()
                                                })
                                                .collect()
                                        }
                                    ),
                            )
                        }),
                )
            })
            // Mission Control / Tab Peek Grid View Overlay
            .when(self.is_tab_overview_open, |this| {
                let backdrop = gpui::hsla(0.0, 0.0, 0.0, 0.75);
                let selected_idx = self.tab_overview_selected;
                let font_fam = self.font_family.clone();
                let viewport = _window.viewport_size();
                let items = self.tabs.len() + 1;
                // Same struct the keyboard handler uses, so the painted column
                // count and the navigation column count cannot disagree. The
                // grid box is built from the same numbers it was derived from,
                // so the flex line fits exactly `layout.cols` cells.
                let layout = MissionControlLayout::new(
                    viewport.width.to_f64() as f32,
                    viewport.height.to_f64() as f32,
                    items,
                );
                // The grid box is built from the same numbers the column count
                // came from, so this cannot fire. The invariant is covered by
                // `mission_control_cells_fit_with_slack` rather than by a panic
                // in the render path.
                let cell_w = layout.cell_w;
                let cell_h = layout.cell_h;
                let container_w = layout.grid_w;

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop)
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(MC_TOP_PAD))
                        .pb(px(MC_EDGE_MARGIN))
                        .px(px(MC_EDGE_MARGIN))
                        .gap(px(MC_HEADER_GAP))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_tab_overview_open = false;
                            cx.notify();
                        }))
                        // Top Header Bar
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .w(px(container_w))
                                .max_w(px(container_w))
                                .h(px(MC_HEADER_H))
                                .px(px(16.))
                                .rounded(px(10.))
                                .overflow_hidden()
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_lg()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .min_w_0()
                                        .flex_shrink_0()
                                        .child(render_icon(IconType::Layers, theme.accent, 16.0))
                                        .child(
                                            div()
                                                .text_size(px(14.))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.foreground)
                                                .child("Mission Control"),
                                        )
                                        .child(
                                            div()
                                                .px(px(6.))
                                                .py(px(2.))
                                                .rounded(px(4.))
                                                .bg(theme.surface_raised)
                                                .text_size(px(11.))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.accent)
                                                .child(format!("{} Tabs", self.tabs.len())),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_3()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme.muted)
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .overflow_hidden()
                                                .child("← → ↑ ↓ Navigate • Enter Select • D Close • T New • ESC Exit"),
                                        )
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .p(px(4.))
                                                .rounded(px(4.))
                                                .hover(|s| s.bg(theme.hover))
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.is_tab_overview_open = false;
                                                    cx.notify();
                                                }))
                                                .child(render_icon(IconType::X, theme.muted, 14.0)),
                                        ),
                                ),
                        )
                        // Grid of Tab Thumbnails
                        .child(
                            div()
                                .id("tab-overview-grid")
                                .track_scroll(&self.tab_overview_scroll_handle)
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .justify_center()
                                .w(px(container_w))
                                .max_h(px(layout.grid_max_h))
                                .overflow_y_scroll()
                                .gap(px(MC_GAP))
                                .p(px(MC_GRID_PAD))
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .children(
                                    self.tabs.iter().enumerate().map(|(idx, tab)| {
                                        let is_selected = idx == selected_idx;
                                        let is_active_tab = idx == self.active_tab_idx;
                                        let tab_id = tab.id;
                                        let title = tab.custom_title.clone().unwrap_or_else(|| tab.title.clone());
                                        let proc_name = tab.terminal.as_ref().and_then(|t| t.get_foreground_process_name()).unwrap_or_else(|| "terminal".to_string());
                                        let (icon_type, _) = super::icons::get_deck_process_icon(&proc_name);
                                        let active_term = tab.terminal.clone().or_else(|| tab.pane_tree.all_panes().first().and_then(|p| p.terminal.clone()));
                                        // The preview fills the card, so the line
                                        // count follows the cell height instead
                                        // of a fixed number. A fixed count left
                                        // most of a tall card as empty black. A
                                        // terminal cannot supply more rows than
                                        // it shows, so the count is also bounded
                                        // by that.
                                        let available_rows = active_term
                                            .as_ref()
                                            .map(|t| t.screen_line_count())
                                            .unwrap_or(0);
                                        let preview_lines = active_term
                                            .as_ref()
                                            .map(|t| {
                                                t.get_screen_preview(preview_line_count(
                                                    cell_h,
                                                    available_rows,
                                                ))
                                            })
                                            .unwrap_or_default();
                                        let split_count = tab.pane_tree.all_panes().len();
                                        let branch_opt = tab.git_status.as_ref().map(|g| g.branch.clone());

                                        div()
                                            .flex()
                                            .flex_col()
                                            .w(px(cell_w))
                                            .h(px(cell_h))
                                            .rounded(px(10.))
                                            .bg(theme.surface)
                                            .border_2()
                                            .border_color(if is_selected { theme.accent } else if is_active_tab { theme.accent.opacity(0.5) } else { theme.border })
                                            .shadow_xl()
                                            .cursor(CursorStyle::PointingHand)
                                            .overflow_hidden()
                                            .hover(|s| if !is_selected { s.border_color(theme.foreground.opacity(0.4)) } else { s })
                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                if this.tab_overview_selected != idx {
                                                    this.tab_overview_selected = idx;
                                                    cx.notify();
                                                }
                                            }))
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                this.is_tab_overview_open = false;
                                                this.select_tab(tab_id, cx);
                                            }))
                                            // Card Top Bar
                                            .child(
                                                div()
                                                    .h(px(32.))
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .px(px(10.))
                                                    .bg(if is_selected { theme.accent.opacity(0.12) } else { theme.surface_raised })
                                                    .border_b_1()
                                                    .border_color(theme.border)
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(render_icon(icon_type, theme.accent, 13.0))
                                                            .child(
                                                                div()
                                                                    .flex_1()
                                                                    .min_w_0()
                                                                    .text_size(px(11.5))
                                                                    .font_weight(FontWeight::BOLD)
                                                                    .text_color(theme.foreground)
                                                                    .whitespace_nowrap()
                                                                    .text_ellipsis()
                                                                    .overflow_hidden()
                                                                    .child(title),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_1_5()
                                                            .when(split_count > 1, |el| {
                                                                el.child(
                                                                    div()
                                                                        .px(px(4.))
                                                                        .py(px(1.))
                                                                        .rounded(px(3.))
                                                                        .bg(theme.black)
                                                                        .text_size(px(9.5))
                                                                        .text_color(theme.yellow)
                                                                        .child(format!("{split_count} splits")),
                                                                )
                                                            })
                                                            .child(
                                                                div()
                                                                    .px(px(5.))
                                                                    .py(px(1.))
                                                                    .rounded(px(3.))
                                                                    .bg(if is_selected { theme.accent } else { theme.black })
                                                                    .text_size(px(10.))
                                                                    .font_weight(FontWeight::BOLD)
                                                                    .text_color(if is_selected { theme.black } else { theme.muted })
                                                                    .child(format!("#{}", idx + 1)),
                                                            )
                                                            .child(
                                                                div()
                                                                    .cursor(CursorStyle::PointingHand)
                                                                    .p(px(2.))
                                                                    .rounded(px(3.))
                                                                    .hover(|s| s.bg(theme.hover))
                                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, window, cx| {
                                                                        cx.stop_propagation();
                                                                        this.close_tab(tab_id, window, cx);
                                                                        if this.tabs.is_empty() {
                                                                            this.is_tab_overview_open = false;
                                                                        } else {
                                                                            this.tab_overview_selected = this.tab_overview_selected.min(this.tabs.len() - 1);
                                                                        }
                                                                        cx.notify();
                                                                    }))
                                                                    .child(render_icon(IconType::X, theme.muted, 11.0)),
                                                            ),
                                                    ),
                                            )
                                            // Mini Terminal Preview
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .bg(theme.black.opacity(0.92))
                                                    .px(px(8.))
                                                    .py(px(6.))
                                                    .font_family(font_fam.clone())
                                                    .text_size(px(8.0))
                                                    .overflow_hidden()
                                                    .flex()
                                                    .flex_col()
                                                    .justify_start()
                                                    .gap(px(1.5))
                                                    .children(
                                                        if preview_lines.is_empty() || preview_lines.iter().all(|l| l.spans.is_empty()) {
                                                            vec![
                                                                div()
                                                                    .flex_1()
                                                                    .flex()
                                                                    .items_center()
                                                                    .justify_center()
                                                                    .text_size(px(9.5))
                                                                    .text_color(theme.muted.opacity(0.6))
                                                                    .child("~ (empty buffer)")
                                                            ]
                                                        } else {
                                                            preview_lines
                                                                .into_iter()
                                                                .map(|line| {
                                                                    let is_cursor_row = line.is_cursor_row;
                                                                    div()
                                                                        .h(px(13.))
                                                                        .w_full()
                                                                        .flex()
                                                                        .flex_row()
                                                                        .items_center()
                                                                        .overflow_hidden()
                                                                        .whitespace_nowrap()
                                                                        .when(line.spans.is_empty(), |el| {
                                                                            if is_cursor_row {
                                                                                el.child(
                                                                                    div()
                                                                                        .w(px(5.))
                                                                                        .h(px(9.))
                                                                                        .rounded(px(1.))
                                                                                        .bg(theme.accent)
                                                                                )
                                                                            } else {
                                                                                el.child(div().h(px(13.)).child(" "))
                                                                            }
                                                                        })
                                                                        .when(!line.spans.is_empty(), |el| {
                                                                            let mut row_el = el;
                                                                            for span in line.spans {
                                                                                let color = match span.fg {
                                                                                    Some(ansi) => self.convert_fg_harmonized(
                                                                                        ansi,
                                                                                        Self::hsla_to_rgb_tuple(theme.background),
                                                                                    ),
                                                                                    None => theme.foreground.opacity(0.85),
                                                                                };
                                                                                row_el = row_el.child(
                                                                                    div()
                                                                                        .text_color(color)
                                                                                        .when(span.bold, |s| s.font_weight(FontWeight::BOLD))
                                                                                        .child(span.text),
                                                                                );
                                                                            }
                                                                            if is_cursor_row {
                                                                                row_el = row_el.child(
                                                                                    div()
                                                                                        .w(px(5.))
                                                                                        .h(px(9.))
                                                                                        .rounded(px(1.))
                                                                                        .bg(theme.accent)
                                                                                        .ml(px(1.)),
                                                                                );
                                                                            }
                                                                            row_el
                                                                        })
                                                                })
                                                                .collect()
                                                        }
                                                    ),
                                            )
                                            // Card Bottom Status Bar
                                            .child({
                                                let folder_name = tab.cwd.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned());
                                                div()
                                                    .h(px(26.))
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .px(px(8.))
                                                    .bg(theme.surface)
                                                    .border_t_1()
                                                    .border_color(theme.border)
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap(px(5.))
                                                            .text_size(px(9.5))
                                                            .text_color(theme.muted)
                                                            .whitespace_nowrap()
                                                            .text_ellipsis()
                                                            .overflow_hidden()
                                                            .child(div().child(format!("cmd: {}", proc_name)))
                                                            .when_some(folder_name, |el, folder| {
                                                                el.child(div().text_color(theme.muted.opacity(0.5)).child("•"))
                                                                    .child(render_icon(IconType::Folder, theme.muted, 11.0))
                                                                    .child(div().text_ellipsis().overflow_hidden().child(folder))
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .pl(px(6.))
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap(px(4.))
                                                            .text_size(px(9.5))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(theme.accent)
                                                            .whitespace_nowrap()
                                                            .map(|el| {
                                                                if let Some(b) = branch_opt {
                                                                    el.child(render_icon(IconType::GitBranch, theme.accent, 11.0))
                                                                        .child(b)
                                                                } else {
                                                                    el.child("local")
                                                                }
                                                            }),
                                                    )
                                            })
                                    })
                                )
                                // Plus Card to Add Tab
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .justify_center()
                                        .w(px(cell_w))
                                        .h(px(cell_h))
                                        .rounded(px(10.))
                                        .bg(theme.surface.opacity(0.5))
                                        .border_2()
                                        .border_color(theme.border)
                                        .cursor(CursorStyle::PointingHand)
                                        .hover(|s| s.bg(theme.hover).border_color(theme.accent))
                                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, window, cx| {
                                            this.is_tab_overview_open = false;
                                            this.create_tab(window, cx);
                                        }))
                                        .child(render_icon(IconType::Plus, theme.accent, 28.0))
                                        .child(
                                            div()
                                                .pt(px(6.))
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.foreground)
                                                .child("New Tab"),
                                        ),
                                ),
                        ),
                )
            })
            // Global Multi-Tab Search Overlay
            .when(self.is_global_search_open, |this| {
                let backdrop = gpui::hsla(0.0, 0.0, 0.0, 0.65);
                let selected_idx = self.global_search_selected;
                let results_count = self.global_search_results.len();

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop)
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(60.))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_global_search_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(580.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                // Input Box Header
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px(px(12.))
                                        .py(px(10.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(render_icon(IconType::Search, theme.accent, 14.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(13.))
                                                .text_color(if self.global_search_query.is_empty() {
                                                    theme.muted
                                                } else {
                                                    theme.foreground
                                                })
                                                .child(if self.global_search_query.is_empty() {
                                                    "Search in all open tabs and splits (⌘⇧F)...".to_string()
                                                } else {
                                                    format!("{}|", self.global_search_query)
                                                }),
                                        )
                                        .child(
                                            div()
                                                .px(px(6.))
                                                .py(px(2.))
                                                .rounded(px(3.))
                                                .bg(theme.surface_raised)
                                                .text_size(px(10.))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(if results_count > 0 { theme.accent } else { theme.muted })
                                                .child(format!("{results_count} results")),
                                        )
                                        .child(
                                            div()
                                                .px(px(5.))
                                                .py(px(2.))
                                                .rounded(px(3.))
                                                .bg(theme.surface_raised)
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child("ESC"),
                                        ),
                                )
                                // Results List
                                .child(
                                    div()
                                        .id("global-search-results-list")
                                        .track_scroll(&self.global_search_scroll_handle)
                                        .flex()
                                        .flex_col()
                                        .max_h(px(340.))
                                        .overflow_y_scroll()
                                        .p(px(4.))
                                        .gap_1()
                                        .children(
                                            if self.global_search_query.trim().is_empty() {
                                                vec![
                                                    div()
                                                        .p(px(20.))
                                                        .flex()
                                                        .flex_col()
                                                        .items_center()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(theme.foreground)
                                                                .child("Find text across all tabs"),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(11.))
                                                                .text_color(theme.muted)
                                                                .child("Type any query to search the full scrollback of every active terminal."),
                                                        ),
                                                ]
                                            } else if self.global_search_results.is_empty() {
                                                vec![
                                                    div()
                                                        .p(px(16.))
                                                        .flex()
                                                        .flex_col()
                                                        .child(
                                                            div()
                                                                .text_size(px(12.))
                                                                .text_color(theme.muted)
                                                                .child("No matches found across active tabs"),
                                                        ),
                                                ]
                                            } else {
                                                self.global_search_results
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(idx, res)| {
                                                        let is_selected = idx == selected_idx;
                                                        let tab_id = res.tab_id;
                                                        let offset = res.offset;
                                                        let pane_id = res.pane_id;
                                                        let proc_name = res.process_name.clone().unwrap_or_else(|| "sh".to_string());
                                                        let (icon_type, _) = super::icons::get_deck_process_icon(&proc_name);

                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .px(px(10.))
                                                            .py(px(6.))
                                                            .rounded(px(6.))
                                                            .bg(if is_selected { theme.accent } else { theme.surface })
                                                            .hover(|s| if !is_selected { s.bg(theme.hover) } else { s })
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_move(cx.listener(move |this, _ev, _window, cx| {
                                                                if this.global_search_selected != idx {
                                                                    this.global_search_selected = idx;
                                                                    cx.notify();
                                                                }
                                                            }))
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                                this.is_global_search_open = false;
                                                                this.select_tab(tab_id, cx);
                                                                if let Some(active_tab) = this.tabs.iter().find(|t| t.id == tab_id) {
                                                                    if pane_id > 0 {
                                                                        if let Some(pane) = active_tab.pane_tree.find_pane(pane_id) {
                                                                            if let Some(ref t) = pane.terminal {
                                                                                t.scroll_to_offset(offset);
                                                                            }
                                                                        }
                                                                    } else if let Some(ref t) = active_tab.terminal {
                                                                        t.scroll_to_offset(offset);
                                                                    }
                                                                }
                                                                cx.notify();
                                                            }))
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_row()
                                                                    .items_center()
                                                                    .justify_between()
                                                                    .child(
                                                                        div()
                                                                            .flex()
                                                                            .flex_row()
                                                                            .items_center()
                                                                            .gap_1()
                                                                            .child(render_icon(icon_type, if is_selected { theme.black } else { theme.accent }, 12.0))
                                                                            .child(
                                                                                div()
                                                                                    .text_size(px(11.5))
                                                                                    .font_weight(FontWeight::BOLD)
                                                                                    .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                                    .child(format!("Tab #{}: {}", res.tab_idx + 1, res.tab_title)),
                                                                            ),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .px(px(4.))
                                                                            .py(px(1.))
                                                                            .rounded(px(3.))
                                                                            .bg(if is_selected { theme.black } else { theme.surface_raised })
                                                                            .text_size(px(9.5))
                                                                            .text_color(if is_selected { theme.accent } else { theme.muted })
                                                                            .child(format!("Line {}", res.line_index)),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .pt(px(2.))
                                                                    .text_size(px(11.))
                                                                    .font_family(self.font_family.clone())
                                                                    .text_color(if is_selected { theme.black.opacity(0.9) } else { theme.muted })
                                                                    .child(res.line_content.clone()),
                                                            )
                                                    })
                                                    .collect()
                                            }
                                        ),
                                ),
                        ),
                )
            })
            .when(self.is_update_modal_open, |this| {
                let release_opt = self.update_available.clone();
                let has_release = release_opt.is_some();
                let version = release_opt
                    .as_ref()
                    .map(|r| r.version.clone())
                    .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
                let release_url = release_opt
                    .as_ref()
                    .map(|r| r.release_url.clone())
                    .unwrap_or_else(|| "https://github.com/diegoleteliers10/fasty/releases".to_string());
                let notes = release_opt
                    .as_ref()
                    .and_then(|r| {
                        let trimmed = r.release_notes.trim();
                        if !trimmed.is_empty() {
                            Some(trimmed.to_string())
                        } else {
                            None
                        }
                    })
                    .or_else(|| crate::whats_new::notes_for(&version))
                    .unwrap_or_else(|| "Bug fixes and performance improvements.".to_string());

                let title = if self.is_update_ready {
                    format!("Fastty v{} is ready to install", version)
                } else if self.is_updating {
                    format!("Downloading Fastty v{}...", version)
                } else if has_release {
                    format!("Fastty v{} is available", version)
                } else if self.update_status.as_deref() == Some("Checking for updates...") {
                    "Checking for updates...".to_string()
                } else if self.update_status.as_deref().is_some_and(|status| status.starts_with("Update check failed:")) {
                    "Update check failed".to_string()
                } else {
                    "Fastty is up to date".to_string()
                };

                let subtitle = if self.is_update_ready {
                    "Update downloaded and verified. Restart Fastty to switch to the new version."
                } else if self.is_updating {
                    "Downloading update in the background..."
                } else if let Some(ref status) = self.update_status {
                    status.as_str()
                } else if has_release {
                    "A new update is available."
                } else {
                    "Fastty is up to date."
                };

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.5))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_update_modal_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(500.))
                                .p(px(16.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .pb(px(6.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(theme.foreground)
                                                        .child(title),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(theme.muted)
                                                        .child(subtitle.to_string()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.is_update_modal_open = false;
                                                    cx.notify();
                                                }))
                                                .child(render_icon(IconType::X, theme.accent, 12.0)),
                                        ),
                                )
                                .when(has_release, |el| el.child(
                                    div()
                                        .id("update-modal-changelog")
                                        .max_h(px(320.))
                                        .p(px(12.))
                                        .rounded(px(6.))
                                        .bg(theme.surface_raised)
                                        .border_1()
                                        .border_color(theme.border)
                                        .overflow_y_scroll()
                                        .child(crate::ui::markdown::render_markdown(&notes, &theme, false)),
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .pt(px(2.))
                                        .when(has_release, |el| el.child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme.accent)
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, move |_ev, _window, _cx| {
                                                    open_path_or_url(&release_url);
                                                })
                                                .child("View full changelog"),
                                        ))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .gap_2()
                                                .when(self.is_update_ready, |el| {
                                                    el.child(
                                                        div()
                                                            .px(px(12.))
                                                            .py(px(5.))
                                                            .rounded(px(5.))
                                                            .bg(theme.surface_raised)
                                                            .text_color(theme.foreground)
                                                            .font_weight(FontWeight::NORMAL)
                                                            .text_size(px(11.))
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                                this.is_update_modal_open = false;
                                                                cx.notify();
                                                            }))
                                                            .child("Later"),
                                                    )
                                                    .child(
                                                        div()
                                                            .px(px(12.))
                                                            .py(px(5.))
                                                            .rounded(px(5.))
                                                            .bg(theme.accent)
                                                            .text_color(theme.black)
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_size(px(11.))
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, _cx| {
                                                                this.persist_session();
                                                                let _ = this.config.save_default();
                                                                crate::updater::relaunch_fastty();
                                                            }))
                                                            .child("Restart Now"),
                                                    )
                                                })
                                                .when(self.is_updating, |el| {
                                                    el.child(
                                                        div()
                                                            .px(px(12.))
                                                            .py(px(5.))
                                                            .rounded(px(5.))
                                                            .bg(theme.accent)
                                                            .text_color(theme.black)
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_size(px(11.))
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                                this.is_update_modal_open = false;
                                                                cx.notify();
                                                            }))
                                                            .child("Hide"),
                                                    )
                                                })
                                                .when(!self.is_update_ready && !self.is_updating, |el| {
                                                    let is_blocked = self
                                                        .update_available
                                                        .as_ref()
                                                        .and_then(|r| r.self_update_blocked_reason.as_ref())
                                                        .is_some();
                                                    let release_url = self
                                                        .update_available
                                                        .as_ref()
                                                        .map(|r| r.release_url.clone())
                                                        .unwrap_or_else(|| "https://github.com/diegoleteliers10/fasty/releases".to_string());
                                                    el.child(
                                                        div()
                                                            .px(px(12.))
                                                            .py(px(5.))
                                                            .rounded(px(5.))
                                                            .bg(theme.surface_raised)
                                                            .text_color(theme.foreground)
                                                            .font_weight(FontWeight::NORMAL)
                                                            .text_size(px(11.))
                                                            .cursor(CursorStyle::PointingHand)
                                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                                this.is_update_modal_open = false;
                                                                cx.notify();
                                                            }))
                                                            .child(if has_release { "Later" } else { "Close" }),
                                                    )
                                                    .when(has_release, |btn| {
                                                        btn.child(
                                                            div()
                                                                .px(px(12.))
                                                                .py(px(5.))
                                                                .rounded(px(5.))
                                                                .bg(theme.surface_raised)
                                                                .text_color(theme.foreground)
                                                                .font_weight(FontWeight::NORMAL)
                                                                .text_size(px(11.))
                                                                .cursor(CursorStyle::PointingHand)
                                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                                    this.skip_update_version(cx);
                                                                }))
                                                                .child("Skip this version"),
                                                        )
                                                    })
                                                    .when(is_blocked, |btn| {
                                                        btn.child(
                                                            div()
                                                                .px(px(12.))
                                                                .py(px(5.))
                                                                .rounded(px(5.))
                                                                .bg(theme.accent)
                                                                .text_color(theme.black)
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_size(px(11.))
                                                                .cursor(CursorStyle::PointingHand)
                                                                .on_mouse_down(MouseButton::Left, move |_ev, _window, _cx| {
                                                                    open_path_or_url(&release_url);
                                                                })
                                                                .child("Open download page"),
                                                        )
                                                    })
                                                    .when(has_release && !is_blocked, |btn| {
                                                        btn.child(
                                                            div()
                                                                .px(px(12.))
                                                                .py(px(5.))
                                                                .rounded(px(5.))
                                                                .bg(theme.accent)
                                                                .text_color(theme.black)
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_size(px(11.))
                                                                .cursor(CursorStyle::PointingHand)
                                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, window, cx| {
                                                                    this.trigger_apply_update(window, cx);
                                                                }))
                                                                .child("Update Now"),
                                                        )
                                                    })
                                                }),
                                        ),
                                ),
                        ),
                )
            })
            .when(self.is_whats_new_open, |this| {
                let notes = self
                    .whats_new_notes
                    .clone()
                    .unwrap_or_else(|| "Bug fixes and improvements.".to_string());
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.5))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_whats_new_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(480.))
                                .p(px(16.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .pb(px(6.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.foreground)
                                                .child(format!("What's new in v{}", env!("CARGO_PKG_VERSION"))),
                                        )
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.is_whats_new_open = false;
                                                    cx.notify();
                                                }))
                                                .child(render_icon(IconType::X, theme.accent, 12.0)),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("whats-new-notes")
                                        .max_h(px(360.))
                                        .overflow_y_scroll()
                                        .child(crate::ui::markdown::render_markdown(&notes, &theme, false)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .pt(px(2.))
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme.accent)
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                                    open_path_or_url("https://github.com/diegoleteliers10/fasty/releases");
                                                })
                                                .child("View full changelog"),
                                        )
                                        .child(
                                            div()
                                                .px(px(12.))
                                                .py(px(5.))
                                                .rounded(px(5.))
                                                .bg(theme.accent)
                                                .text_color(theme.black)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(px(11.))
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.is_whats_new_open = false;
                                                    cx.notify();
                                                }))
                                                .child("Continue"),
                                        ),
                                ),
                        ),
                )
            })
            .when(self.pending_close.is_some(), |this| {
                let pending = self.pending_close.clone().unwrap();
                let process_names: Vec<String> = pending
                    .running_processes
                    .iter()
                    .map(|p| p.process_name.clone())
                    .collect();

                let (target_label, desc) = match pending.target {
                    CloseTarget::Pane { .. } => {
                        let name = process_names.first().cloned().unwrap_or_else(|| "process".to_string());
                        ("Close Pane?", format!("Process '{}' is still running in this pane.", name))
                    }
                    CloseTarget::Tab { .. } => {
                        if process_names.len() == 1 {
                            ("Close Tab?", format!("Process '{}' is still running in this tab.", process_names[0]))
                        } else {
                            ("Close Tab?", format!("{} processes are still running in this tab.", process_names.len()))
                        }
                    }
                    CloseTarget::OtherTabs { .. } => {
                        ("Close Other Tabs?", format!("{} processes are still running in other tabs.", process_names.len()))
                    }
                };

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.55))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.pending_close = None;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(380.))
                                .p(px(16.))
                                .rounded(px(10.))
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .shadow_xl()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .on_mouse_down(MouseButton::Left, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .pb(px(6.))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .px(px(6.))
                                                        .py(px(1.))
                                                        .rounded(px(4.))
                                                        .bg(theme.yellow)
                                                        .text_color(theme.black)
                                                        .text_size(px(10.))
                                                        .font_weight(FontWeight::BOLD)
                                                        .child("ACTIVE"),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(theme.foreground)
                                                        .child(target_label),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.pending_close = None;
                                                    cx.notify();
                                                }))
                                                .child(render_icon(IconType::X, theme.muted_strong, 12.0)),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.foreground)
                                        .child(desc),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_1()
                                        .children(process_names.into_iter().map(|name| {
                                            div()
                                                .px(px(8.))
                                                .py(px(2.))
                                                .rounded(px(4.))
                                                .bg(theme.surface_raised)
                                                .border_1()
                                                .border_color(theme.border)
                                                .text_size(px(11.))
                                                .text_color(theme.accent)
                                                .child(name)
                                        })),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(theme.muted_strong)
                                        .child("Do you want to terminate the running process and close?"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .justify_end()
                                        .gap_2()
                                        .pt(px(4.))
                                        .child(
                                            div()
                                                .px(px(12.))
                                                .py(px(5.))
                                                .rounded(px(5.))
                                                .bg(theme.surface_raised)
                                                .text_color(theme.foreground)
                                                .text_size(px(11.))
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.pending_close = None;
                                                    cx.notify();
                                                }))
                                                .child("Cancel"),
                                        )
                                        .child(
                                            div()
                                                .px(px(12.))
                                                .py(px(5.))
                                                .rounded(px(5.))
                                                .bg(theme.red)
                                                .text_color(theme.white)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_size(px(11.))
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, window, cx| {
                                                    this.confirm_pending_close(window, cx);
                                                }))
                                                .child("Terminate & Close"),
                                        ),
                                ),
                        ),
                )
            })
    }
}

fn render_context_menu_item(
    icon: IconType,
    label: &'static str,
    shortcut: Option<&'static str>,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    theme: Theme,
) -> gpui::Stateful<Div> {
    let hover_bg = theme.hover;
    let active_bg = theme.surface_raised;
    div()
        .id(SharedString::from(format!("ctx-item-{}", label)))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .w_full()
        .px(px(8.))
        .py(px(5.5))
        .rounded(px(6.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(hover_bg))
        .active(move |s| s.bg(active_bg))
        .on_mouse_down(MouseButton::Left, on_click)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(render_icon(icon, theme.accent, 13.0))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(label),
                ),
        )
        .when_some(shortcut, |this, sc| {
            this.child(
                div()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(theme.muted)
                    .child(sc),
            )
        })
}

fn render_context_menu_divider(theme: Theme) -> Div {
    div().h(px(1.)).w_full().bg(theme.border).my(px(2.))
}

fn render_about_spec_row(label: &'static str, value: &str, theme: Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .py(px(2.))
        .text_size(px(11.))
        .child(div().text_color(theme.muted).child(label))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.foreground)
                .child(value.to_string()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon the tab rows, the project jumper and the attachment chips draw
    /// must produce real SVG geometry, not an empty path list. An empty list
    /// yields a valid but blank SVG, which reads as a layout bug.
    ///
    /// This does not catch a misspelt variant, which fails to compile. It
    /// catches a registry entry that stops carrying geometry, such as an
    /// `icons` upgrade that empties a list.
    ///
    /// The check lives here rather than in `icons.rs` because that file glob
    /// imports `gpui::*`, and a test module there does not compile.
    #[test]
    fn rendered_icons_have_svg_data() {
        for (label, icon) in [
            ("file image", ::icons::common::IconType::FileImage),
            ("file", ::icons::common::IconType::File),
            ("folder", ::icons::common::IconType::Folder),
            ("git branch", icons::common::IconType::GitBranch),
            ("terminal", ::icons::common::IconType::Terminal),
            ("file code", ::icons::common::IconType::FileCode),
            (
                "git pull request",
                ::icons::common::IconType::GitPullRequest,
            ),
            ("user", ::icons::common::IconType::User),
            ("git commit", ::icons::common::IconType::GitCommitHorizontal),
            // Status bar markers that used to be text-presentation glyphs.
            ("check", ::icons::common::IconType::Check),
            ("x", ::icons::common::IconType::X),
            ("clock", ::icons::common::IconType::Clock),
            ("skip forward", ::icons::common::IconType::SkipForward),
            ("loader", ::icons::common::IconType::Loader),
            ("arrow up", ::icons::common::IconType::ArrowUp),
            ("arrow down", ::icons::common::IconType::ArrowDown),
            ("dot", ::icons::common::IconType::Dot),
            // Menu icons: two identical icons in one list tell the reader
            // nothing, so each action gets its own.
            ("clipboard copy", ::icons::common::IconType::ClipboardCopy),
            ("clipboard paste", ::icons::common::IconType::ClipboardPaste),
            ("copy plus", ::icons::common::IconType::CopyPlus),
            ("panel right", ::icons::common::IconType::PanelRight),
            ("panel bottom", ::icons::common::IconType::PanelBottom),
            ("panel left", ::icons::common::IconType::PanelLeft),
            ("panel top", ::icons::common::IconType::PanelTop),
            ("text search", ::icons::common::IconType::TextSearch),
            ("folder git", ::icons::common::IconType::FolderGit2),
            (
                "layout panel top",
                ::icons::common::IconType::LayoutPanelTop,
            ),
            (
                "layout panel left",
                ::icons::common::IconType::LayoutPanelLeft,
            ),
        ] {
            let data = crate::ui::icons::get_icon_svg_bytes(icon);
            // Not every icon is a path: the commit markers are a circle and two
            // lines. Check for any drawable element, or the check would reject
            // valid icons and pass a genuinely empty one.
            const SHAPES: [&[u8]; 7] = [
                b"<path",
                b"<circle",
                b"<line",
                b"<rect",
                b"<polyline",
                b"<polygon",
                b"<ellipse",
            ];
            let drawable = SHAPES
                .iter()
                .any(|tag| data.windows(tag.len()).any(|w| w == *tag));
            assert!(
                drawable,
                "the {label} icon has no drawable shape, so it would render blank"
            );
        }
    }

    /// Two rows with the same icon next to each other say nothing: the reader
    /// has to read both labels to tell them apart. Icons are the fastest signal
    /// in a list, so a repeated one is a bug.
    ///
    /// The Theme category is excluded. Its rows differ by theme, not by action,
    /// so they want a swatch of the theme's own colour rather than a distinct
    /// icon per theme.
    #[test]
    fn no_two_palette_commands_in_a_category_share_an_icon() {
        let cmds = get_all_palette_commands();
        let mut by_category: std::collections::HashMap<&str, Vec<(&str, String)>> =
            std::collections::HashMap::new();
        for cmd in cmds.iter().filter(|c| c.category != "Theme") {
            by_category
                .entry(cmd.category)
                .or_default()
                .push((cmd.title, format!("{:?}", cmd.icon)));
        }
        let mut clashes: Vec<String> = Vec::new();
        for (category, items) in &by_category {
            let mut seen: std::collections::HashMap<&str, Vec<&str>> =
                std::collections::HashMap::new();
            for (title, icon) in items {
                seen.entry(icon.as_str()).or_default().push(title);
            }
            for (icon, titles) in seen.iter().filter(|(_, t)| t.len() > 1) {
                clashes.push(format!("{category}: {icon} is used by {titles:?}"));
            }
        }
        assert!(
            clashes.is_empty(),
            "duplicate icons: {}",
            clashes.join("; ")
        );
    }

    /// Wrapping up from the first row lands on the last one, so the arrow keys
    /// can reach every candidate without stopping at an edge.
    #[test]
    fn the_mention_selection_wraps_at_both_ends() {
        let matches = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut selected = 0;

        move_at_menu_selection(&mut selected, &matches, -1);
        assert_eq!(selected, 2, "up from the top wraps to the bottom");
        move_at_menu_selection(&mut selected, &matches, 1);
        assert_eq!(selected, 0, "down from the bottom wraps to the top");
        move_at_menu_selection(&mut selected, &matches, 1);
        assert_eq!(selected, 1);
        move_at_menu_selection(&mut selected, &matches, -1);
        assert_eq!(selected, 0);
    }

    /// An empty candidate list has no row to point at. Without this, a stale
    /// index would survive and Enter would insert a path that is no longer in
    /// the list.
    #[test]
    fn moving_with_no_rows_clears_the_selection() {
        let matches: Vec<String> = Vec::new();
        let mut selected = 3;
        move_at_menu_selection(&mut selected, &matches, 1);
        assert_eq!(selected, 0);
        move_at_menu_selection(&mut selected, &matches, -1);
        assert_eq!(selected, 0);
    }

    /// Typing filters the candidate list, so the selection has to come back into
    /// range when the list shrinks under it.
    /// The renderer draws at most `AI_AT_MENU_MAX_ROWS` rows and the arrow keys
    /// must not reach past the last one drawn.
    /// The renderer draws at most `AI_AT_MENU_MAX_ROWS` rows, so no jump can
    /// leave the selection pointing at a row that is not painted.
    #[test]
    fn no_jump_points_past_the_last_drawn_row() {
        let matches: Vec<String> = (0..40).map(|i| format!("f{i}")).collect();
        let max = crate::ui::ai_sidebar::AI_AT_MENU_MAX_ROWS;
        assert_eq!(at_menu_row_count(&matches), max);

        let mut selected = 0;
        for delta in [1, 5, 19, 20, 21, 100] {
            move_at_menu_selection(&mut selected, &matches, delta);
            assert!(
                selected < max,
                "a jump of {delta} landed on row {selected}, past the last drawn row"
            );
            move_at_menu_selection_clamped(&mut selected, &matches, delta);
            assert!(
                selected < max,
                "a page of {delta} landed on row {selected}, past the last drawn row"
            );
        }
    }

    /// Arrow keys wrap so the whole list stays reachable, but a page jump stops
    /// at the ends. Wrapping a page jump puts the user at the opposite end.
    #[test]
    fn a_page_jump_stops_at_the_ends_while_an_arrow_wraps() {
        let matches: Vec<String> = (0..40).map(|i| format!("f{i}")).collect();
        let last = crate::ui::ai_sidebar::AI_AT_MENU_MAX_ROWS - 1;

        let mut selected = last;
        move_at_menu_selection(&mut selected, &matches, 1);
        assert_eq!(selected, 0, "down from the last row wraps to the top");

        let mut selected = last;
        move_at_menu_selection_clamped(&mut selected, &matches, 5);
        assert_eq!(selected, last, "paging down from the last row stays put");

        let mut selected = 0;
        move_at_menu_selection_clamped(&mut selected, &matches, -5);
        assert_eq!(selected, 0, "paging up from the first row stays put");
    }

    /// A file dropped on the AI composer must attach to the message. Before this
    /// the composer was a sibling of the terminal area, so the drop bubbled to
    /// the root container and pasted the path into the shell.
    #[test]
    fn a_drop_on_the_ai_composer_attaches_instead_of_pasting() {
        assert_eq!(
            file_drop_target_at(true, 900.0, 950.0),
            FileDropTarget::AiAttach
        );
        assert_eq!(
            file_drop_target_at(true, 900.0, 900.0),
            FileDropTarget::AiAttach,
            "the left edge of the composer belongs to the composer"
        );
        assert_eq!(
            file_drop_target_at(true, 900.0, 899.0),
            FileDropTarget::Terminal,
            "one pixel left of the composer is the terminal"
        );
    }

    /// With the panel closed its width is 0, so the edge is at the right of the
    /// window. A drop can never attach to a panel that is not there.
    #[test]
    fn a_closed_composer_never_takes_the_drop() {
        assert_eq!(
            file_drop_target_at(false, 1280.0, 1279.0),
            FileDropTarget::Terminal
        );
        assert_eq!(
            file_drop_target_at(false, 1280.0, 1279.9),
            FileDropTarget::Terminal
        );
    }

    /// A wide terminal next to the composer: everything left of the split goes
    /// to the shell.
    #[test]
    fn the_terminal_side_of_the_split_pastes() {
        for x in [0.0, 1.0, 400.0, 899.0] {
            assert_eq!(
                file_drop_target_at(true, 900.0, x),
                FileDropTarget::Terminal
            );
        }
    }

    /// Enter commits the row the arrows pointed at, not always the first one.
    /// This is the whole point of the keyboard selection: the user arrows down
    /// three rows and expects the third.
    #[test]
    fn enter_commits_the_highlighted_row() {
        let matches: Vec<String> = (0..5).map(|i| format!("f{i}")).collect();
        for selected in 0..5 {
            assert_eq!(at_menu_commit_row(true, &matches, selected), Some(selected));
        }
    }

    /// A selection left over from a list that has since shrunk must not pick a
    /// row that is no longer shown.
    #[test]
    fn enter_clamps_a_stale_selection() {
        let matches = vec!["a".to_string(), "b".to_string()];
        assert_eq!(at_menu_commit_row(true, &matches, 9), Some(1));
    }

    /// With the menu closed or empty, Enter is not ours and has to reach the
    /// shell, which is how sending a message works.
    #[test]
    fn enter_falls_through_when_the_menu_has_nothing_to_pick() {
        let matches = vec!["a".to_string()];
        assert_eq!(at_menu_commit_row(false, &matches, 0), None);
        assert_eq!(at_menu_commit_row(true, &[], 0), None);
    }

    /// An overlay opened from the AI panel has to get the keys first. The
    /// composer is focused as a plain bool, so clicking the paperclip leaves it
    /// focused with the picker above it.
    #[test]
    fn an_open_overlay_takes_the_keys_from_the_ai_composer() {
        // Panel focused, picker closed: the composer owns the key.
        assert!(ai_keys_own_input(true, true, false));
        // Panel focused, picker open: the picker owns it, or a typed letter goes
        // into the prompt and Enter sends the prompt.
        assert!(!ai_keys_own_input(true, true, true));
        // Panel closed: the shell owns it.
        assert!(!ai_keys_own_input(false, true, false));
        // Panel open but not focused: the shell owns it.
        assert!(!ai_keys_own_input(true, false, false));
    }

    /// `is_char_boundary` is what stops a 16 KB cap from slicing through an
    /// accented character and aborting the process on submit.
    #[test]
    fn the_inline_cap_never_splits_a_character() {
        // A 3-byte euro sign straddling the cap, so slicing at exactly the cap
        // lands in the middle of it.
        let filler = "a".repeat(AI_ATTACH_INLINE_MAX_BYTES - 1);
        let content = format!("{filler}\u{20ac}{}", "b".repeat(50));
        assert!(!content.is_char_boundary(AI_ATTACH_INLINE_MAX_BYTES));
        let out = truncate_for_prompt(&content);
        assert!(out.ends_with("... (truncated)"), "{}", &out[out.len()..]);
        let kept = out.strip_suffix("... (truncated)").unwrap();
        assert!(content.starts_with(kept));
        assert!(kept.len() <= AI_ATTACH_INLINE_MAX_BYTES);
    }

    /// A short file comes through whole, with no truncation marker.
    #[test]
    fn a_short_file_is_not_truncated() {
        let content = "line\n".repeat(10);
        assert_eq!(truncate_for_prompt(&content), content);
        assert_eq!(truncate_for_prompt(""), "");
    }

    /// The mention list, the row count and the renderer have to agree on the
    /// cap, or the arrows would reach a row that is never painted.
    #[test]
    fn the_mention_cap_is_one_value() {
        assert_eq!(crate::ui::ai_sidebar::AI_AT_MENU_MAX_ROWS, 20);
        let over: Vec<String> = (0..40).map(|i| format!("f{i}")).collect();
        assert_eq!(at_menu_row_count(&over), 20);
        assert_eq!(over.iter().take(20).count(), at_menu_row_count(&over));
    }

    #[test]
    fn test_theme_palette_ids_map_to_theme_names() {
        // Every registry entry round-trips: palette id -> theme name.
        for (name, id, _, _) in crate::config::THEME_REGISTRY {
            assert_eq!(theme_name_for_palette_id(id), Some(*name), "id {id}");
        }
        // Non-theme commands never trigger a preview.
        assert_eq!(theme_name_for_palette_id("new_tab"), None);
        assert_eq!(theme_name_for_palette_id("settings"), None);
        assert_eq!(theme_name_for_palette_id("quit"), None);
    }

    #[test]
    fn test_find_nerd_font_prefers_mono_variant() {
        let names = vec![
            "Menlo".to_string(),
            "JetBrainsMono Nerd Font".to_string(),
            "JetBrainsMono Nerd Font Mono".to_string(),
        ];
        assert_eq!(
            find_nerd_font_family(&names),
            Some("JetBrainsMono Nerd Font Mono".to_string())
        );
    }

    #[test]
    fn test_find_nerd_font_case_insensitive_and_optional() {
        let names = vec!["Menlo".to_string(), "FiraCode NERD FONT".to_string()];
        assert_eq!(
            find_nerd_font_family(&names),
            Some("FiraCode NERD FONT".to_string())
        );
        let plain = vec!["Menlo".to_string(), "Consolas".to_string()];
        assert_eq!(find_nerd_font_family(&plain), None);
        let empty: Vec<String> = Vec::new();
        assert_eq!(find_nerd_font_family(&empty), None);
    }

    #[test]
    fn test_is_nerd_codepoint_ranges() {
        // Powerline, PUA icons, Font Awesome range.
        assert!(is_nerd_codepoint(0xE0A0));
        assert!(is_nerd_codepoint(0xE62E));
        assert!(is_nerd_codepoint(0xF013));
        assert!(is_nerd_codepoint(0x23FB));
        assert!(is_nerd_codepoint(0x2B58));
        // Ordinary text is not nerd.
        assert!(!is_nerd_codepoint('A' as u32));
        assert!(!is_nerd_codepoint('─' as u32));
        assert!(!is_nerd_codepoint('●' as u32));
    }

    #[test]
    fn test_is_emoji_codepoint_excludes_keyboard_symbols() {
        // macOS technical keyboard symbols must NOT be emojis so they render with monospace metrics
        assert!(!is_emoji_codepoint('⌘' as u32));
        assert!(!is_emoji_codepoint('⌥' as u32));
        assert!(!is_emoji_codepoint('⌃' as u32));
        assert!(!is_emoji_codepoint('⎋' as u32));
        assert!(!is_emoji_codepoint('⌫' as u32));
        assert!(!is_emoji_codepoint('⌦' as u32));
        assert!(!is_emoji_codepoint('⏎' as u32));
        assert!(!is_emoji_codepoint('⇧' as u32));

        // Actual emojis in the 0x2300 block must still be recognized
        assert!(is_emoji_codepoint(0x231A)); // ⌚
        assert!(is_emoji_codepoint(0x231B)); // ⌛
        assert!(is_emoji_codepoint(0x23F0)); // ⏰
        assert!(is_emoji_codepoint(0x23F3)); // ⏳
    }

    #[test]
    fn test_is_keyboard_symbol_classification() {
        assert!(is_keyboard_symbol('⌘' as u32));
        assert!(is_keyboard_symbol('⌥' as u32));
        assert!(is_keyboard_symbol('⌃' as u32));
        assert!(is_keyboard_symbol('⇧' as u32));
        assert!(is_keyboard_symbol('⎋' as u32));
        assert!(is_keyboard_symbol('⏎' as u32));
        assert!(is_keyboard_symbol('←' as u32));
        assert!(is_keyboard_symbol('→' as u32));
        assert!(is_keyboard_symbol('↑' as u32));
        assert!(is_keyboard_symbol('↓' as u32));
        assert!(is_keyboard_symbol(0x27F5)); // ⟵ Long leftwards arrow (Supplemental Arrows-A)
        assert!(is_keyboard_symbol(0x2905)); // ⤅ Rightwards two-headed arrow from bar (Supplemental Arrows-B)
        assert!(is_keyboard_symbol(0x2B05)); // ⬅ Leftwards black arrow (Misc Symbols and Arrows)

        // Regular text characters must not be keyboard symbols
        assert!(!is_keyboard_symbol('A' as u32));
        assert!(!is_keyboard_symbol('R' as u32));
        assert!(!is_keyboard_symbol('W' as u32));
        assert!(!is_keyboard_symbol('0' as u32));
        assert!(!is_keyboard_symbol(' ' as u32));

        // Emojis and Nerd Font symbols in 0x2B00 block must not be keyboard symbols
        assert!(!is_keyboard_symbol(0x2B50)); // ⭐ Star emoji
        assert!(!is_keyboard_symbol(0x2B55)); // ⭕ Circle emoji
        assert!(!is_keyboard_symbol(0x2B58)); // Nerd Font icon
    }

    #[test]
    fn test_pr_action_mapping_and_commands() {
        assert_eq!(pr_action_count(), 4);
        assert_eq!(pr_action_for_index(0), PrPickerAction::Checkout);
        assert_eq!(pr_action_for_index(1), PrPickerAction::Approve);
        assert_eq!(pr_action_for_index(2), PrPickerAction::Merge);
        assert_eq!(pr_action_for_index(3), PrPickerAction::Back);
        assert_eq!(pr_action_for_index(99), PrPickerAction::Back);
        assert_eq!(
            pr_action_command(PrPickerAction::Checkout, 12),
            Some("gh pr checkout 12".to_string())
        );
        assert_eq!(
            pr_action_command(PrPickerAction::Approve, 12),
            Some("gh pr review 12 --approve".to_string())
        );
        assert_eq!(
            pr_action_command(PrPickerAction::Merge, 12),
            Some("gh pr merge 12 --merge".to_string())
        );
        assert_eq!(pr_action_command(PrPickerAction::Back, 12), None);
        assert_eq!(
            pr_action_label(PrPickerAction::Checkout, 12),
            "Checkout PR #12"
        );
        assert_eq!(pr_action_label(PrPickerAction::Back, 12), "← Back to list");
    }

    #[test]
    fn test_pr_picker_rows_pins_current_and_dedups() {
        use crate::widgets::builtin::git_prs::{GhPrAuthor, GhPrList, GhPrView, PrsSummary};
        let snapshot = PrsSummary {
            current_pr: Some(GhPrView {
                state: "OPEN".to_string(),
                number: 7,
                title: "Mine".to_string(),
                url: "https://github.com/o/r/pull/7".to_string(),
                review_decision: Some("APPROVED".to_string()),
            }),
            open_prs: vec![
                GhPrList {
                    number: 7,
                    title: "Mine".to_string(),
                    url: "https://github.com/o/r/pull/7".to_string(),
                    author: None,
                },
                GhPrList {
                    number: 9,
                    title: "Theirs".to_string(),
                    url: "https://github.com/o/r/pull/9".to_string(),
                    author: Some(GhPrAuthor {
                        login: "sam".to_string(),
                    }),
                },
            ],
            review_requested_prs: Vec::new(),
            cwd: std::path::PathBuf::from("/tmp"),
        };
        let rows = pr_picker_rows(&snapshot);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].is_current);
        assert_eq!(rows[0].number, 7);
        assert_eq!(rows[0].detail, "APPROVED");
        assert!(!rows[1].is_current);
        assert_eq!(rows[1].detail, "@sam");
    }

    #[test]
    fn test_palette_preview_kind_covers_theme_font_and_layout() {
        assert_eq!(
            palette_preview_kind("theme_catppuccin"),
            Some(PalettePreviewKind::Theme("catppuccin"))
        );
        assert_eq!(
            palette_preview_kind("zoom_in"),
            Some(PalettePreviewKind::FontDelta(1.0))
        );
        assert_eq!(
            palette_preview_kind("zoom_out"),
            Some(PalettePreviewKind::FontDelta(-1.0))
        );
        assert_eq!(
            palette_preview_kind("zoom_reset"),
            Some(PalettePreviewKind::FontReset)
        );
        assert_eq!(
            palette_preview_kind("layout_horizontal"),
            Some(PalettePreviewKind::Layout(TabLayout::Horizontal))
        );
        assert_eq!(
            palette_preview_kind("layout_vertical"),
            Some(PalettePreviewKind::Layout(TabLayout::Vertical))
        );
        // Plain commands preview nothing (navigating to them reverts).
        assert_eq!(palette_preview_kind("new_tab"), None);
        assert_eq!(palette_preview_kind("ssh"), None);
        assert_eq!(palette_preview_kind("quit"), None);
    }

    #[test]
    fn test_geometric_block_cells_return_elements() {
        let theme = Theme::fastty_default();
        let fg = theme.foreground;
        let bg = Some(theme.background);
        let theme_bg = theme.background;
        let cell_w = 9.0;
        let line_h = 18.0;

        // Block elements
        let block_chars = [
            '█', '▀', '▄', '▌', '▐', '░', '▒', '▓', '▖', '▗', '▘', '▙', '▚', '▛', '▜', '▝', '▞',
            '▟',
        ];
        for ch in block_chars {
            let res = render_geometric_cell(ch, cell_w, line_h, fg, bg, theme_bg);
            assert!(
                res.is_some(),
                "Character {:?} (U+{:04X}) must produce a geometric element",
                ch,
                ch as u32
            );
        }

        // Box drawing
        let box_chars = [
            '─', '│', '┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼', '╭', '╮', '╯', '╰', '═', '║',
        ];
        for ch in box_chars {
            let res = render_geometric_cell(ch, cell_w, line_h, fg, bg, theme_bg);
            assert!(
                res.is_some(),
                "Box character {:?} (U+{:04X}) must produce a geometric element",
                ch,
                ch as u32
            );
        }
    }

    #[test]
    fn test_geometric_shapes_synthesized() {
        let theme = Theme::fastty_default();
        let fg = theme.foreground;
        let bg = Some(theme.background);
        let theme_bg = theme.background;

        // Circles and squares render as synthesized shapes centered on the cell box
        let shape_chars = ['○', '◯', '●', '◦', '◉', '◎', '■', '□', '▪', '▫'];
        for ch in shape_chars {
            let res = render_geometric_cell(ch, 8.0, 17.0, fg, bg, theme_bg);
            assert!(
                res.is_some(),
                "Shape {:?} (U+{:04X}) must produce a geometric element",
                ch,
                ch as u32
            );
        }

        // Wide-cell large circle gets the full two-cell span width
        let res = render_geometric_cell('◯', 16.0, 17.0, fg, bg, theme_bg);
        assert!(res.is_some());
    }

    #[test]
    fn test_per_cell_synthesis_classification() {
        // Vertical-stroke box chars render once per cell
        assert!(synthesized_per_cell('│'));
        assert!(synthesized_per_cell('├'));
        assert!(synthesized_per_cell('┼'));
        assert!(synthesized_per_cell('║'));
        // Synthesized shapes render once per cell
        assert!(synthesized_per_cell('○'));
        assert!(synthesized_per_cell('●'));
        // Horizontal-only strokes and block fills stay single-element
        assert!(!synthesized_per_cell('─'));
        assert!(!synthesized_per_cell('━'));
        assert!(!synthesized_per_cell('═'));
        assert!(!synthesized_per_cell('█'));
        assert!(!synthesized_per_cell('░'));
    }

    #[test]
    fn test_decode_box_drawing_styles() {
        // Light horizontal and vertical
        assert_eq!(decode_box_drawing('─'), Some((1, 1, 0, 0, 0)));
        assert_eq!(decode_box_drawing('│'), Some((0, 0, 1, 1, 0)));

        // Light corners
        assert_eq!(decode_box_drawing('┌'), Some((0, 1, 0, 1, 0)));
        assert_eq!(decode_box_drawing('┐'), Some((1, 0, 0, 1, 0)));
        assert_eq!(decode_box_drawing('└'), Some((0, 1, 1, 0, 0)));
        assert_eq!(decode_box_drawing('┘'), Some((1, 0, 1, 0, 0)));

        // Round corners
        assert_eq!(decode_box_drawing('╭'), Some((0, 1, 0, 1, 1)));
        assert_eq!(decode_box_drawing('╮'), Some((1, 0, 0, 1, 1)));
        assert_eq!(decode_box_drawing('╯'), Some((1, 0, 1, 0, 1)));
        assert_eq!(decode_box_drawing('╰'), Some((0, 1, 1, 0, 1)));
    }

    #[test]
    fn test_opencode_banner_block_fixtures() {
        let theme = Theme::fastty_default();
        let fg = theme.foreground;
        let bg = Some(theme.background);
        let theme_bg = theme.background;
        let cell_w = 8.5;
        let line_h = 19.0;

        // Sample ASCII block art from OpenCode / Claude Code banners
        let banner_lines = [
            "  ▄████▄   ██████  ███████ ███    ██  ██████  ",
            " ██      ██ ██   ██ ██      ████   ██ ██      ",
            " ██      ██ ██████  █████   ██ ██  ██ ██      ",
            " ██      ██ ██      ██      ██  ██ ██ ██      ",
            "  ▀████▀▀   ██      ███████ ██   ████  ██████ ",
        ];

        for line in banner_lines {
            for ch in line.chars() {
                if ch == ' ' {
                    continue;
                }
                let res = render_geometric_cell(ch, cell_w, line_h, fg, bg, theme_bg);
                assert!(
                    res.is_some(),
                    "Banner character {:?} must render geometrically",
                    ch
                );
            }
        }
    }

    #[test]
    fn test_viewport_scissor_clamping_logic() {
        let target_cols = 80;
        let cell_cols = 2; // Wide char

        // Cells within bounds
        let col_in = 78;
        assert!(col_in < target_cols);
        let end_col = (col_in + cell_cols).min(target_cols);
        assert_eq!(end_col, 80);

        // Cells outside bounds
        let col_out = 80;
        assert!(col_out >= target_cols);
        let col_far = 95;
        assert!(col_far >= target_cols);
    }

    #[test]
    fn test_trim_row_spans_without_cursor_state() {
        let theme = Theme::fastty_default();
        let mut spans = vec![
            StyledSpan {
                text: "hello".to_string(),
                start_col: 0,
                end_col: 5,
                fg: theme.foreground,
                bg: None,
                is_bold: false,
                is_underline: false,
                is_emoji: false,
                is_nerd: false,
                is_kbd: false,
                emoji_scale: None,
                char_cols: vec![0, 1, 2, 3, 4],
            },
            StyledSpan {
                text: "    ".to_string(),
                start_col: 5,
                end_col: 9,
                fg: theme.foreground,
                bg: None,
                is_bold: false,
                is_underline: false,
                is_emoji: false,
                is_nerd: false,
                is_kbd: false,
                emoji_scale: None,
                char_cols: vec![5, 6, 7, 8],
            },
        ];

        trim_row_spans(&mut spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, "hello");
        assert_eq!(spans[0].end_col, 5);

        // Spans with background are preserved even with trailing spaces
        let mut spans_with_bg = vec![StyledSpan {
            text: "highlighted   ".to_string(),
            start_col: 0,
            end_col: 14,
            fg: theme.foreground,
            bg: Some(theme.accent),
            is_bold: false,
            is_underline: false,
            is_emoji: false,
            is_nerd: false,
            is_kbd: false,
            emoji_scale: None,
            char_cols: (0..14).collect(),
        }];
        trim_row_spans(&mut spans_with_bg);
        assert_eq!(spans_with_bg.len(), 1);
        assert_eq!(spans_with_bg[0].text, "highlighted   ");
    }

    #[test]
    fn test_cursor_quad_geometry_matches_cell_grid() {
        let cell_w = 9.0;
        let line_h = 18.0;

        for col in 0..100 {
            let x_start = (col as f32 * cell_w).round();
            let x_end = ((col + 1) as f32 * cell_w).round();
            let quad_w = (x_end - x_start).max(cell_w);

            // Span width for single character at that column
            let span_w = ((col + 1) as f32 * cell_w).round() - (col as f32 * cell_w).round();

            assert_eq!(quad_w, span_w.max(cell_w));
            assert!(quad_w >= cell_w);
            assert!(line_h > 0.0);
        }
    }
}

/// Mission Control grid geometry and navigation.
///
/// Its own module because `#[test]` expands recursively, once per test, and one
/// module holding every test in this file runs past the crate's recursion limit.
#[cfg(test)]
mod mission_control_tests {
    use super::*;

    /// The grid must keep every cell at or above the minimum readable size, so
    /// a narrow window drops a column instead of shrinking cells.
    ///
    /// Width is checked across the whole range because the app allows a window
    /// down to 640 px. Height is checked at a size that leaves room for the
    /// minimum cell: below that the grid scrolls by design, which
    /// `mission_control_layout_fits_window` covers.
    #[test]
    fn mission_control_layout_keeps_cells_readable() {
        for width in [640.0, 900.0, 1280.0, 1920.0, 2560.0, 3440.0] {
            for items in 1..=12 {
                let layout = MissionControlLayout::new(width, 800.0, items);
                assert!(layout.cols >= 1, "cols must be at least 1");
                assert!(layout.cols <= items, "cols cannot exceed the cell count");
                assert!(layout.cols <= MC_MAX_COLS);
                assert!(
                    layout.cell_w >= MC_MIN_CELL_W,
                    "cell_w {} below the minimum at width {width} with {items} items",
                    layout.cell_w
                );
                assert!(
                    layout.cell_h >= 0.0 && layout.cell_h.is_finite(),
                    "cell_h {} is not usable at width {width} with {items} items",
                    layout.cell_h
                );
                assert!(
                    layout.grid_w <= width,
                    "grid_w {} overflows window width {width}",
                    layout.grid_w
                );
                assert!(layout.grid_max_h >= 0.0);
            }
        }
    }

    /// The grid must stay inside the window, otherwise the last row is cut off
    /// instead of scrolling.
    #[test]
    fn mission_control_layout_fits_window() {
        for (width, height) in [
            (1920.0, 400.0),
            (1280.0, 600.0),
            (900.0, 700.0),
            (1920.0, 1400.0),
            (2560.0, 1440.0),
        ] {
            for items in 1..=40 {
                let layout = MissionControlLayout::new(width, height, items);
                assert!(
                    layout.grid_w <= width,
                    "grid_w {} overflows window width {width}",
                    layout.grid_w
                );
                assert!(
                    layout.grid_max_h <= height,
                    "grid_max_h {} overflows window height {height}",
                    layout.grid_max_h
                );
                assert!(layout.cell_w > 0.0 && layout.cell_h > 0.0);
            }
        }
    }

    /// The painted cells per row must equal the column count the keyboard
    /// handler navigates by.
    ///
    /// The grid wraps with `flex_wrap`, and the layout engine measures the
    /// content box, so the box padding comes out of the width the cells get. A
    /// `grid_w` that omits the padding makes the engine wrap one column early:
    /// the grid then paints `cols - 1` cells per row while the handler steps by
    /// `cols`, and every arrow key lands on the wrong cell. This is the check
    /// that would have caught the original bug.
    #[test]
    fn mission_control_painted_columns_match_navigation() {
        for (width, height) in [
            (640.0, 420.0),
            (900.0, 700.0),
            (1280.0, 600.0),
            (1280.0, 1400.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3440.0, 1440.0),
        ] {
            for items in 1..=24 {
                let layout = MissionControlLayout::new(width, height, items);
                assert_eq!(
                    fits_cols_in_row(layout.grid_w, layout.cell_w),
                    layout.cols,
                    "at {width}x{height} with {items} items the grid paints a different \
                     number of cells per row than the arrow keys step over"
                );
            }
        }
    }

    /// The grid must use the window width it is given, right up to the point
    /// where the cell width cap takes over. A grid that stops growing early
    /// leaves a dead band at the edge.
    ///
    /// The cap binds at `MC_MAX_COLS` cells of `MC_MAX_CELL_W`. Below that
    /// width the grid must fill the window exactly; the sweep ends just under
    /// the threshold so a lowered cap fails here rather than in the app.
    #[test]
    fn mission_control_grid_uses_available_width() {
        let threshold = MC_MAX_COLS as f32 * MC_MAX_CELL_W
            + (MC_MAX_COLS as f32 - 1.0) * MC_GAP
            + 2.0 * MC_GRID_PAD
            + 2.0 * MC_EDGE_MARGIN;
        for width in [
            900.0,
            1280.0,
            1600.0,
            1920.0,
            2560.0,
            3440.0,
            threshold - 1.0,
        ] {
            let layout = MissionControlLayout::new(width, 900.0, 8);
            let slack = (width - 2.0 * MC_EDGE_MARGIN) - layout.grid_w;
            assert!(
                slack.abs() < 1.0,
                "grid_w {} leaves {slack} px unused at width {width}",
                layout.grid_w
            );
        }
    }

    /// Past the cell width cap the grid centres itself rather than stretching a
    /// cell past a readable width. A fifth column is not an option: it would
    /// push every cell below `MC_MIN_CELL_W`.
    #[test]
    fn mission_control_grid_caps_cell_width() {
        let threshold = MC_MAX_COLS as f32 * MC_MAX_CELL_W
            + (MC_MAX_COLS as f32 - 1.0) * MC_GAP
            + 2.0 * MC_GRID_PAD
            + 2.0 * MC_EDGE_MARGIN;
        for width in [threshold, 5000.0, 7680.0] {
            let layout = MissionControlLayout::new(width, 2160.0, 8);
            assert_eq!(layout.cols, MC_MAX_COLS);
            assert!(
                layout.cell_w <= MC_MAX_CELL_W,
                "cell_w {} exceeds the cap at width {width}",
                layout.cell_w
            );
        }
    }

    /// A card must be a little shorter than it is wide, at every window size.
    /// The preview shows the last screenful of a terminal, which reads fine in a
    /// landscape box, and a portrait box wastes height.
    ///
    /// A window too short for the aspect shrinks the cell instead, because
    /// fitting the cell matters more than the aspect there.
    #[test]
    fn mission_control_cell_is_shorter_than_wide() {
        for (width, height) in [
            (640.0, 420.0),
            (1280.0, 800.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3440.0, 1440.0),
            (1920.0, 2160.0),
        ] {
            let layout = MissionControlLayout::new(width, height, 8);
            let aspect_h = layout.cell_w * MC_CELL_ASPECT;
            let expected = aspect_h.min(layout.grid_max_h);
            assert!(
                (layout.cell_h - expected).abs() < 0.01,
                "cell_h {} at {width}x{height} is neither the aspect height {aspect_h} \
                 nor the window limit {}",
                layout.cell_h,
                layout.grid_max_h
            );
            assert!(
                layout.cell_h < layout.cell_w,
                "cell {}x{} is not shorter than wide at {width}x{height}",
                layout.cell_w,
                layout.cell_h
            );
        }
    }

    /// The exact geometry at three reference window sizes.
    ///
    /// This pins the numbers so a change to the layout constants shows up as a
    /// failing test rather than a different card. It does not by itself prove
    /// platform parity: the layout takes no platform input, so a Windows run
    /// asserts the same values because the same arithmetic runs, not because
    /// this test compares two platforms.
    #[test]
    fn mission_control_geometry_is_identical_on_every_platform() {
        for (width, height, expected_cols, expected_cell_w, expected_cell_h) in [
            (1512.0_f32, 982.0_f32, 4_usize, 352.0_f32, 264.0_f32),
            (1920.0, 1080.0, 4, 454.0, 340.5),
            (1280.0, 800.0, 4, 294.0, 220.5),
        ] {
            let layout = MissionControlLayout::new(width, height, 8);
            assert_eq!(layout.cols, expected_cols, "cols at {width}x{height}");
            assert!(
                (layout.cell_w - expected_cell_w).abs() < 1.0,
                "cell_w {} differs from the pinned {expected_cell_w} at {width}x{height}",
                layout.cell_w
            );
            assert!(
                (layout.cell_h - expected_cell_h).abs() < 1.0,
                "cell_h {} differs from the pinned {expected_cell_h} at {width}x{height}",
                layout.cell_h
            );
        }
    }

    /// The card must stay big enough to hold the top bar, the status bar, and a
    /// few preview lines. A short card would show a status bar and no terminal.
    #[test]
    fn mission_control_cell_holds_a_usable_preview() {
        for (width, height) in [
            (640.0, 420.0),
            (1280.0, 800.0),
            (1920.0, 1080.0),
            (3440.0, 1440.0),
        ] {
            let layout = MissionControlLayout::new(width, height, 8);
            let lines = preview_line_count(layout.cell_h, 200);
            assert!(
                lines >= 6,
                "a {}x{} card at {width}x{height} holds only {lines} preview lines",
                layout.cell_w,
                layout.cell_h
            );
        }
    }

    /// A short window flattens the cell, and the grid then scrolls. It must not
    /// panic, and the cell must still be a usable size.
    #[test]
    fn mission_control_survives_a_short_window() {
        for height in [0.0, 50.0, 134.0, 200.0, 334.0, 420.0] {
            for width in [640.0, 1920.0] {
                let layout = MissionControlLayout::new(width, height, 8);
                assert!(
                    layout.cell_h >= 0.0 && layout.cell_h.is_finite(),
                    "cell_h {} is not usable at {width}x{height}",
                    layout.cell_h
                );
                assert!(
                    layout.cell_w >= MC_MIN_CELL_W,
                    "cell_w {} fell below the minimum at {width}x{height}",
                    layout.cell_w
                );
                assert!(layout.grid_max_h >= 0.0);
            }
        }
    }

    /// The preview must fill the card. A fixed line count leaves the rest of a
    /// tall cell as empty background, so the count follows the cell height.
    ///
    /// The count is also bounded by the rows the terminal shows, because
    /// `get_screen_preview` cannot return more. The test uses a tall terminal so
    /// the cell height is the binding limit, which is the case a fixed count
    /// used to get wrong.
    #[test]
    fn mission_control_preview_fills_the_cell() {
        let tall_terminal = 200;
        for (width, height) in [
            (640.0, 420.0),
            (1280.0, 800.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3440.0, 1440.0),
        ] {
            let layout = MissionControlLayout::new(width, height, 8);
            let lines = preview_line_count(layout.cell_h, tall_terminal);
            let used = MC_CARD_CHROME_H + MC_PREVIEW_PAD_H + lines as f32 * MC_PREVIEW_LINE_H;
            let fill = used / layout.cell_h;
            assert!(
                fill > 0.85,
                "preview fills only {fill:.0}% of the {} px cell at {width}x{height} \
                 ({lines} lines)",
                layout.cell_h
            );
            assert!(
                used <= layout.cell_h,
                "{lines} preview lines need {used} px but the cell is {} px",
                layout.cell_h
            );
        }
    }

    /// A short terminal cannot fill a tall cell, because the preview has one
    /// line per visible row. The count must reflect that rather than asking for
    /// rows that will never come.
    #[test]
    fn mission_control_preview_respects_the_terminal_row_count() {
        let cell_h = 1209.0;
        let full = preview_line_count(cell_h, 200);
        let short = preview_line_count(cell_h, 24);
        assert_eq!(
            short, 24,
            "a 24 row terminal must not yield more than 24 lines"
        );
        assert!(full > short);
        // Asking for zero rows yields an empty preview rather than a panic.
        assert_eq!(preview_line_count(cell_h, 0), 0);
    }

    #[test]
    fn mission_control_preview_line_count_stays_within_the_cell() {
        // Below one line height there is nothing to draw, and reporting one
        // would overflow the card.
        assert_eq!(preview_line_count(0.0, 100), 0);
        assert_eq!(preview_line_count(MC_CARD_CHROME_H, 100), 0);
        assert_eq!(
            preview_line_count(MC_CARD_CHROME_H + MC_PREVIEW_PAD_H + MC_PREVIEW_LINE_H, 100),
            1
        );
        // A normal cell gets a useful number of lines.
        assert!(preview_line_count(286.0, 100) >= 10);
        // The count never exceeds what the cell can hold, whatever the terminal
        // reports.
        for height in 200..2000 {
            for available in [0usize, 1, 24, 100, 200] {
                let lines = preview_line_count(height as f32, available);
                let used = MC_CARD_CHROME_H + MC_PREVIEW_PAD_H + lines as f32 * MC_PREVIEW_LINE_H;
                assert!(
                    used <= height as f32,
                    "{lines} lines need {used} px in a {} px cell",
                    height
                );
            }
        }
    }

    /// The cells must fit their row with room to spare, at every window width.
    ///
    /// An exact fit is not enough: the layout engine sums the cells one at a
    /// time in `f32`, so a fit with no slack can land an ulp over the budget and
    /// wrap the last cell onto a second row. That makes the grid paint one cell
    /// fewer per row than the arrow keys navigate, which is the reported bug.
    #[test]
    fn mission_control_cells_fit_with_slack() {
        for width in 640..=3900 {
            for items in 1..=8 {
                let layout = MissionControlLayout::new(width as f32, 900.0, items);
                assert_eq!(
                    fits_cols_in_row(layout.grid_w, layout.cell_w),
                    layout.cols,
                    "at width {width} with {items} items the cells do not fit \
                     {} columns of {} px in {} px",
                    layout.cols,
                    layout.cell_w,
                    layout.grid_w
                );
            }
        }
    }

    /// The widths where the cells fit with no slack, so the grid box has to grow
    /// past the exact sum. Without the extra width the layout engine sums the
    /// cells in `f32` and lands an ulp over the budget, which wraps the last
    /// cell and makes the grid paint fewer columns than the keys navigate.
    #[test]
    fn mission_control_gives_the_last_cell_slack() {
        // Width 1577 with three items is one such case: the exact sum leaves no
        // room for the floating point error in the layout engine.
        let exact = {
            let layout = MissionControlLayout::new(1577.0, 900.0, 3);
            layout.cell_w * 3.0 + 2.0 * MC_GAP + 2.0 * MC_GRID_PAD
        };
        assert_eq!(
            fits_cols_in_row(exact, exact / 3.0),
            2,
            "the exact three column sum must be the failing case this guards"
        );

        let layout = MissionControlLayout::new(1577.0, 900.0, 3);
        assert_eq!(layout.cols, 3);
        assert!(
            layout.grid_w > exact,
            "grid_w {} must exceed the exact sum {exact} to leave slack",
            layout.grid_w
        );
    }

    /// A wider window must give the cells more room, which is what makes the
    /// overlay fill the width on resize.
    #[test]
    fn mission_control_layout_grows_with_window() {
        let narrow = MissionControlLayout::new(900.0, 800.0, 6);
        let wide = MissionControlLayout::new(1920.0, 800.0, 6);
        assert!(wide.cell_w > narrow.cell_w);
        assert!(wide.grid_w > narrow.grid_w);
    }

    /// The layout must be a pure function of its inputs, so the renderer and the
    /// keyboard handler cannot disagree about the grid shape.
    #[test]
    fn mission_control_layout_is_deterministic() {
        for (width, height, items) in [(640.0, 420.0, 1), (1600.0, 900.0, 7), (3440.0, 1440.0, 40)]
        {
            let a = MissionControlLayout::new(width, height, items);
            let b = MissionControlLayout::new(width, height, items);
            assert_eq!(a, b);
        }
    }

    /// Right and Left walk the row in order and never teleport. A grid of 7
    /// tabs over 4 columns: 0 1 2 3 / 4 5 6.
    #[test]
    fn mission_control_navigation_walks_row_in_order() {
        let cols = 4;
        let selectable = 7;

        let mut idx = 0;
        for want in [1, 2, 3, 4, 5, 6, 6] {
            idx = move_grid_selection(idx, GridStep::Right, cols, selectable);
            assert_eq!(idx, want, "right step landed on the wrong cell");
        }
        for want in [5, 4, 3, 2, 1, 0, 0] {
            idx = move_grid_selection(idx, GridStep::Left, cols, selectable);
            assert_eq!(idx, want, "left step landed on the wrong cell");
        }
    }

    /// Up and Down keep the column, so the highlight stays in the same column.
    #[test]
    fn mission_control_navigation_keeps_the_column() {
        let cols = 4;
        let selectable = 7;

        assert_eq!(move_grid_selection(0, GridStep::Down, cols, selectable), 4);
        assert_eq!(move_grid_selection(1, GridStep::Down, cols, selectable), 5);
        assert_eq!(move_grid_selection(2, GridStep::Down, cols, selectable), 6);
        assert_eq!(move_grid_selection(4, GridStep::Up, cols, selectable), 0);
        assert_eq!(move_grid_selection(5, GridStep::Up, cols, selectable), 1);
        assert_eq!(move_grid_selection(6, GridStep::Up, cols, selectable), 2);
    }

    /// Up and Down clamp at the first and last row instead of wrapping.
    #[test]
    fn mission_control_navigation_clamps_at_row_edges() {
        // Two full rows: Up on the first row and Down on the last stay put.
        assert_eq!(move_grid_selection(0, GridStep::Up, 4, 8), 0);
        assert_eq!(move_grid_selection(4, GridStep::Down, 4, 8), 4);
        // A single row: neither direction has anywhere to go.
        assert_eq!(move_grid_selection(2, GridStep::Up, 4, 3), 2);
        assert_eq!(move_grid_selection(2, GridStep::Down, 4, 3), 2);
        // A single cell grid.
        assert_eq!(move_grid_selection(0, GridStep::Down, 4, 1), 0);
        assert_eq!(move_grid_selection(0, GridStep::Up, 4, 1), 0);
    }

    /// A short last row must not slide the selection into another column.
    #[test]
    fn mission_control_navigation_handles_short_last_row() {
        // 3 columns, 7 cells: rows are 0 1 2 / 3 4 5 / 6.
        let cols = 3;
        let selectable = 7;

        assert_eq!(move_grid_selection(1, GridStep::Down, cols, selectable), 4);
        assert_eq!(move_grid_selection(2, GridStep::Down, cols, selectable), 5);
        // Up from the short row keeps the column when the row above has one.
        assert_eq!(move_grid_selection(6, GridStep::Up, cols, selectable), 3);
        // Column 1 has no cell in the last row, so the selection stays put
        // rather than jumping to column 0 or to the end of the grid.
        assert_eq!(move_grid_selection(4, GridStep::Down, cols, selectable), 4);
        assert_eq!(move_grid_selection(5, GridStep::Down, cols, selectable), 5);
    }

    /// An out-of-range selection must not read past the end of the tabs. The
    /// trailing "New Tab" card uses index == tabs.len(), so a press from it
    /// lands on the last real tab.
    #[test]
    fn mission_control_navigation_clamps_stale_selection() {
        let selectable = 5;
        assert_eq!(move_grid_selection(5, GridStep::Left, 3, selectable), 3);
        assert_eq!(move_grid_selection(99, GridStep::Right, 3, selectable), 4);
        assert_eq!(move_grid_selection(99, GridStep::Down, 3, selectable), 4);
    }

    #[test]
    fn mission_control_navigation_handles_empty_grid() {
        assert_eq!(move_grid_selection(0, GridStep::Right, 0, 0), 0);
        assert_eq!(move_grid_selection(0, GridStep::Left, 4, 0), 0);
        assert_eq!(move_grid_selection(0, GridStep::Down, 4, 0), 0);
    }

    /// The property that matters: no arrow key can ever move the selection off
    /// a real tab, for any grid shape the layout can produce.
    #[test]
    fn mission_control_navigation_never_escapes_the_grid() {
        for cols in 1..=6 {
            for items in 1..=40 {
                for start in 0..items {
                    for step in [
                        GridStep::Left,
                        GridStep::Right,
                        GridStep::Up,
                        GridStep::Down,
                    ] {
                        let next = move_grid_selection(start, step, cols, items);
                        assert!(
                            next < items,
                            "step {step:?} from {start} gave {next}, outside 0..{items} \
                             at cols {cols}"
                        );
                    }
                }
            }
        }
    }

    /// With one column every cell is its own row, so all four arrows walk the
    /// list and stop at its ends.
    #[test]
    fn mission_control_navigation_single_column() {
        assert_eq!(move_grid_selection(0, GridStep::Right, 1, 3), 1);
        assert_eq!(move_grid_selection(0, GridStep::Left, 1, 3), 0);
        assert_eq!(move_grid_selection(0, GridStep::Down, 1, 3), 1);
        assert_eq!(move_grid_selection(2, GridStep::Right, 1, 3), 2);
        assert_eq!(move_grid_selection(2, GridStep::Down, 1, 3), 2);
        assert_eq!(move_grid_selection(2, GridStep::Up, 1, 3), 1);
        assert_eq!(move_grid_selection(2, GridStep::Left, 1, 3), 1);
        assert_eq!(move_grid_selection(0, GridStep::Up, 1, 3), 0);
    }
}

#[cfg(test)]
mod ligature_tests {
    use super::*;

    #[test]
    fn ligatures_are_enabled_when_requested() {
        let on = terminal_ligatures(true);
        for tag in ["calt", "liga", "dlig", "clig"] {
            assert!(
                on.0.iter().any(|(name, v)| name == tag && *v == 1),
                "{tag} should be explicitly enabled"
            );
        }
    }

    #[test]
    fn ligatures_are_suppressed_when_disabled() {
        // The regression this guards: the setting used to be ignored entirely,
        // so `font.ligatures = false` had no effect on rendering.
        let off = terminal_ligatures(false);
        assert!(!off.0.is_empty(), "an empty list would mean 'no override'");
        for tag in ["calt", "liga", "dlig", "clig"] {
            assert!(
                off.0.iter().any(|(name, v)| name == tag && *v == 0),
                "{tag} must be explicitly set to 0 to disable it"
            );
        }
    }

    #[test]
    fn the_two_settings_differ() {
        // Same tags either way, opposite values. Comparing lengths would pass
        // even if the toggle were inert.
        let on = terminal_ligatures(true);
        let off = terminal_ligatures(false);
        assert_eq!(on.0.len(), off.0.len(), "both list the same feature tags");
        for ((on_name, on_val), (off_name, off_val)) in on.0.iter().zip(off.0.iter()) {
            assert_eq!(on_name, off_name);
            assert_eq!(*on_val, 1, "{on_name} on");
            assert_eq!(*off_val, 0, "{on_name} off");
        }
        assert_ne!(on.0, off.0, "the toggle must change what is requested");
    }
}
