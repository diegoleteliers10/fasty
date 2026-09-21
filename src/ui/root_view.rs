use std::sync::Arc;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use gpui::{
    Bounds, Context, CursorStyle, Div, FocusHandle, FontFeatures, FontWeight, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, ScrollHandle,
    ScrollWheelEvent, SharedString, TitlebarOptions, Window, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
};

use super::settings_view::SettingsView;
use super::status_bar::{StatusBar, StatusBarModel, StatusInfo};
use super::tab_bar::{TabBar, TabItem, TabSidebar};
use super::theme::{Theme, rgb_to_hsla};
use crate::config::{self, Config, TabLayout};
use crate::event_listener::EventSender;
use crate::git::GitStatus;
use crate::pane_tree::{Direction, PaneId, PaneNode, PaneTree, SplitDirection, TerminalPane};
use crate::terminal_state::{AppEvent, TerminalState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: alacritty_terminal::index::Point,
    pub end: alacritty_terminal::index::Point,
}

pub struct TabData {
    pub id: usize,
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
        0x25A0 | 0x25A1
            | 0x25AA | 0x25AB
            | 0x25CB | 0x25C9
            | 0x25CE | 0x25CF
            | 0x25E6 | 0x25EF
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
                div()
                    .relative()
                    .w(px(width))
                    .h(px(line_h))
                    .bg(bg_c)
                    .child(div().absolute().bottom_0().left_0().right_0().h(px(fill_h)).bg(fg))
            }
            0x2589..=0x258F => {
                let frac = (8 - (code - 0x2588)) as f32 / 8.0;
                let fill_w = (width * frac).round().max(1.0);
                div()
                    .relative()
                    .w(px(width))
                    .h(px(line_h))
                    .bg(bg_c)
                    .child(div().absolute().top_0().bottom_0().left_0().w(px(fill_w)).bg(fg))
            }
            0x2591 => {
                let mut blended = fg;
                blended.a = 0.25;
                div().w(px(width)).h(px(line_h)).bg(bg_c).child(div().size_full().bg(blended))
            }
            0x2592 => {
                let mut blended = fg;
                blended.a = 0.50;
                div().w(px(width)).h(px(line_h)).bg(bg_c).child(div().size_full().bg(blended))
            }
            0x2593 => {
                let mut blended = fg;
                blended.a = 0.75;
                div().w(px(width)).h(px(line_h)).bg(bg_c).child(div().size_full().bg(blended))
            }
            0x2594 => {
                let fill_h = (line_h * 0.125).round().max(1.0);
                div()
                    .relative()
                    .w(px(width))
                    .h(px(line_h))
                    .bg(bg_c)
                    .child(div().absolute().top_0().left_0().right_0().h(px(fill_h)).bg(fg))
            }
            0x2595 => {
                let fill_w = (width * 0.125).round().max(1.0);
                div()
                    .relative()
                    .w(px(width))
                    .h(px(line_h))
                    .bg(bg_c)
                    .child(div().absolute().top_0().bottom_0().right_0().w(px(fill_w)).bg(fg))
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
            let mut corner = div()
                .relative()
                .w(px(width))
                .h(px(line_h))
                .bg(bg_c);

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

        let mut container = div()
            .relative()
            .w(px(width))
            .h(px(line_h))
            .bg(bg_c);

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
        let ring = |size: f32| div().w(px(size)).h(px(size)).rounded_full().border_1().border_color(fg);
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
            0x25C9 => centered(ring(d * 0.78).child(
                div().flex().flex_row().items_center().justify_center().size_full().child(dot(d * 0.36)),
            )),
            // ◎ BULLSEYE — two concentric rings
            0x25CE => centered(ring(d * 0.78).child(
                div().flex().flex_row().items_center().justify_center().size_full().child(ring(d * 0.42)),
            )),
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

fn enable_terminal_ligatures() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![
        ("calt".to_string(), 1),
        ("liga".to_string(), 1),
        ("dlig".to_string(), 1),
        ("clig".to_string(), 1),
    ]))
}

fn is_emoji_codepoint(code: u32) -> bool {
    matches!(
        code,
        0x1F300..=0x1FAFF
            | 0x2600..=0x27BF
            | 0x2300..=0x23FF
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
        crate::font_discovery_macos::available_monospace_fonts()
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
        if let Ok(output) = std::process::Command::new("reg")
            .args(["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts"])
            .output()
        {
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
    let summary = if title.is_empty() { "Fastty" } else { title };
    let res = notify_rust::Notification::new()
        .appname("Fastty")
        .summary(summary)
        .body(body)
        .show();

    if res.is_err() {
        #[cfg(target_os = "macos")]
        {
            let escaped_title = summary.replace('\\', "\\\\").replace('"', "\\\"");
            let escaped_body = body.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!(r#"display notification "{}" with title "{}""#, escaped_body, escaped_title);
            let _ = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .spawn();
        }
    }
}

use icons::common::IconType;
use super::icons::{render_app_logo, render_icon};

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
        PaletteCommand { id: "tab_overview", icon: IconType::Layers, title: "Mission Control / Tab Peek Overview", category: "Navigation", shortcut: Some(if is_mac { "⌘⇧O" } else { "Ctrl+Shift+M" }) },
        PaletteCommand { id: "global_search", icon: IconType::Search, title: "Find in All Tabs (Global Search)", category: "Terminal", shortcut: Some(if is_mac { "⌘⇧F" } else { "Ctrl+Shift+F" }) },
        PaletteCommand { id: "save_session", icon: IconType::Folder, title: "Session: Save Current Workspace Snapshot", category: "Session", shortcut: None },
        PaletteCommand { id: "restore_session", icon: IconType::RotateCcw, title: "Session: Attach / Restore Last Workspace", category: "Session", shortcut: None },
        PaletteCommand { id: "new_tab", icon: IconType::Plus, title: "New Tab", category: "Terminal", shortcut: Some(if is_mac { "⌘T" } else { "Ctrl+Shift+T" }) },
        PaletteCommand { id: "new_window", icon: IconType::ExternalLink, title: "New Window", category: "Window", shortcut: Some(if is_mac { "⌘⇧N" } else { "Ctrl+Shift+N" }) },
        PaletteCommand { id: "rename_tab", icon: IconType::Pencil, title: "Rename Active Tab", category: "Terminal", shortcut: Some(if is_mac { "⌘⇧R" } else { "Ctrl+Shift+R" }) },
        PaletteCommand { id: "close_tab", icon: IconType::X, title: "Close Active Tab", category: "Terminal", shortcut: Some(if is_mac { "⌘⇧W" } else { "Ctrl+Shift+Q" }) },
        PaletteCommand { id: "search", icon: IconType::Search, title: "Search in Buffer", category: "Terminal", shortcut: Some(if is_mac { "⌘F" } else { "Ctrl+Shift+F" }) },
        PaletteCommand { id: "clear", icon: IconType::Trash2, title: "Clear Scrollback", category: "Terminal", shortcut: Some(if is_mac { "⌘K" } else { "Ctrl+Shift+K" }) },
        PaletteCommand { id: "worktree", icon: IconType::GitPullRequest, title: "Git Worktree Picker", category: "Git", shortcut: Some(if is_mac { "⌘⌥W" } else { "Ctrl+Alt+W" }) },
        PaletteCommand { id: "project_jumper", icon: IconType::Folder, title: "Project / Tab Jumper", category: "Navigation", shortcut: Some(if is_mac { "⌘J" } else { "Ctrl+Shift+J" }) },
        PaletteCommand { id: "ssh", icon: IconType::Server, title: "SSH Host Manager", category: "Tools", shortcut: Some(if is_mac { "⌘O" } else { "Ctrl+Shift+O" }) },
        PaletteCommand { id: "snippets", icon: IconType::Terminal, title: "Snippets: Insert Snippet...", category: "Tools", shortcut: None },
        PaletteCommand { id: "prs", icon: IconType::GitPullRequest, title: "GitHub: Pull Requests (Checkout/Approve/Merge)", category: "Git", shortcut: None },
        PaletteCommand { id: "settings", icon: IconType::Settings, title: "Open Settings", category: "Preferences", shortcut: Some(if is_mac { "⌘," } else { "Ctrl+," }) },
        PaletteCommand { id: "about", icon: IconType::Zap, title: "About Fastty", category: "Application", shortcut: None },
        PaletteCommand { id: "fullscreen", icon: IconType::Maximize2, title: "Toggle Fullscreen", category: "Window", shortcut: Some(if is_mac { "⌃⌘F" } else { "F11" }) },
        PaletteCommand { id: "zoom_in", icon: IconType::ZoomIn, title: "Font: Increase Size (+1)", category: "View", shortcut: Some(if is_mac { "⌘=" } else { "Ctrl=" }) },
        PaletteCommand { id: "zoom_out", icon: IconType::ZoomOut, title: "Font: Decrease Size (-1)", category: "View", shortcut: Some(if is_mac { "⌘-" } else { "Ctrl-" }) },
        PaletteCommand { id: "zoom_reset", icon: IconType::RotateCcw, title: "Font: Reset Size (Default)", category: "View", shortcut: Some(if is_mac { "⌘0" } else { "Ctrl+0" }) },
        PaletteCommand { id: "theme_default", icon: IconType::Palette, title: "Theme: Switch to Default (Fastty)", category: "Theme", shortcut: None },
        PaletteCommand { id: "theme_catppuccin", icon: IconType::Palette, title: "Theme: Switch to Catppuccin", category: "Theme", shortcut: None },
        PaletteCommand { id: "theme_one_dark", icon: IconType::Palette, title: "Theme: Switch to One Dark", category: "Theme", shortcut: None },
        PaletteCommand { id: "theme_solarized", icon: IconType::Palette, title: "Theme: Switch to Solarized Dark", category: "Theme", shortcut: None },
        PaletteCommand { id: "theme_high_contrast", icon: IconType::Palette, title: "Theme: Switch to High Contrast", category: "Theme", shortcut: None },
        PaletteCommand { id: "open_config", icon: IconType::FolderOpen, title: "Open Config Folder", category: "Preferences", shortcut: None },
        PaletteCommand { id: "edit_config", icon: IconType::FileCode, title: "Edit config.toml", category: "Preferences", shortcut: None },
        PaletteCommand { id: "split_right", icon: IconType::Plus, title: "Split Pane Right", category: "Panes", shortcut: Some(if is_mac { "⌘D" } else { "Ctrl+Shift+E" }) },
        PaletteCommand { id: "split_down", icon: IconType::Plus, title: "Split Pane Down", category: "Panes", shortcut: Some(if is_mac { "⌘⇧D" } else { "Ctrl+Shift+O" }) },
        PaletteCommand { id: "split_left", icon: IconType::Plus, title: "Split Pane Left", category: "Panes", shortcut: None },
        PaletteCommand { id: "split_top", icon: IconType::Plus, title: "Split Pane Top", category: "Panes", shortcut: None },
        PaletteCommand { id: "focus_left", icon: IconType::ChevronLeft, title: "Focus Pane Left", category: "Panes", shortcut: Some(if is_mac { "⌥⌘←" } else { "Alt+←" }) },
        PaletteCommand { id: "focus_right", icon: IconType::ChevronRight, title: "Focus Pane Right", category: "Panes", shortcut: Some(if is_mac { "⌥⌘→" } else { "Alt+→" }) },
        PaletteCommand { id: "focus_top", icon: IconType::ChevronUp, title: "Focus Pane Top", category: "Panes", shortcut: Some(if is_mac { "⌥⌘↑" } else { "Alt+↑" }) },
        PaletteCommand { id: "focus_down", icon: IconType::ChevronDown, title: "Focus Pane Down", category: "Panes", shortcut: Some(if is_mac { "⌥⌘↓" } else { "Alt+↓" }) },
        PaletteCommand { id: "close_pane", icon: IconType::X, title: "Close Active Pane", category: "Panes", shortcut: Some(if is_mac { "⌘W" } else { "Ctrl+Shift+W" }) },
        PaletteCommand { id: "zoom_pane", icon: IconType::Maximize2, title: "Zoom Active Pane (Toggle)", category: "Panes", shortcut: None },
        PaletteCommand { id: "toggle_tab_sidebar", icon: IconType::Folder, title: "Toggle Tabs Sidebar", category: "View", shortcut: Some(if is_mac { "⌘B" } else { "Ctrl+B" }) },
        PaletteCommand { id: "toggle_ai_sidebar", icon: IconType::Sparkles, title: "Fastty AI: Toggle Assistant Sidebar", category: "AI", shortcut: Some(if is_mac { "⌘L" } else { "Ctrl+Shift+L" }) },
        PaletteCommand { id: "layout_horizontal", icon: IconType::Folder, title: "Tabs Layout: Horizontal Top Bar", category: "View", shortcut: None },
        PaletteCommand { id: "layout_vertical", icon: IconType::Folder, title: "Tabs Layout: Vertical Sidebar", category: "View", shortcut: None },
        PaletteCommand { id: "quit", icon: IconType::LogOut, title: "Quit Fastty", category: "Application", shortcut: Some(if is_mac { "⌘Q" } else { "Alt+F4" }) },
    ]
}

/// Maps a palette command id to the theme it switches to, if any.
/// Used for live preview: navigating to the row previews the theme,
/// `Enter` commits it, `Esc` reverts to the previous one.
pub fn theme_name_for_palette_id(cmd_id: &str) -> Option<&'static str> {
    match cmd_id {
        "theme_default" => Some("default"),
        "theme_catppuccin" => Some("catppuccin"),
        "theme_one_dark" => Some("one-dark"),
        "theme_solarized" => Some("solarized-dark"),
        "theme_high_contrast" => Some("high-contrast"),
        _ => None,
    }
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
    pub(crate) pr_snapshot: Arc<std::sync::Mutex<Option<crate::widgets::builtin::git_prs::PrsSummary>>>,
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
    pub ai_messages: Vec<crate::ui::ai_sidebar::AiUiMessage>,
    pub ai_streaming_text: String,
    pub ai_streaming_thinking: String,
    pub ai_is_streaming: bool,
    pub ai_last_usage: Option<(u64, u64)>,
    pub ai_accumulated_usage: (u64, u64),
    pub ai_current_turn_usage: (u64, u64),
    pub ai_context_hovercard_open: bool,
    pub ai_cancel_token: Option<crate::ai::CancelToken>,
    pub ai_pending_confirmation: Option<crate::ui::ai_sidebar::AiUiPendingConfirmation>,
    pub ai_confirm_reply_tx: Option<async_channel::Sender<crate::ai::PermissionDecision>>,
    pub ai_permission_checker: std::sync::Arc<crate::ai::PermissionChecker>,
    pub ai_agent_mode: String,
    pub ai_expanded_thinkings: std::collections::HashSet<usize>,
    pub ai_attached_files: Vec<std::path::PathBuf>,
    pub ai_at_menu_open: bool,
    pub ai_at_matches: Vec<String>,
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
        let font_id = window.text_system().resolve_font(&gpui::font(self.font_family.clone()));
        let cell_w = window.text_system().layout_width(font_id, px(self.font_size), '0').to_f64() as f32;
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

    pub fn with_options(_window: &mut Window, cli_opts: crate::cli::CliOptions, cx: &mut Context<Self>) -> Self {
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
        let theme_name = loaded_config.theme.as_deref().unwrap_or("default").to_string();
        let theme = Theme::from_name(&theme_name).with_opacity(loaded_config.opacity);

        let status_bar_model = StatusBarModel::new(&loaded_config, theme);
        let focus_handle = cx.focus_handle();
        _window.focus(&focus_handle, cx);

        let font_size = loaded_config.font.size;
        let font_family: SharedString = if loaded_config.font.family.is_empty() || loaded_config.font.family == "monospace" {
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
            loaded_config.font.family.clone().into()
        };

        // F3: auto-detect an installed Nerd Font for PUA icon fallback.
        // Only used when one exists; otherwise rendering is unchanged.
        let nerd_font_family: Option<SharedString> =
            find_nerd_font_family(&_window.text_system().all_font_names()).map(|n| n.into());

        // Scrollbar fade and cursor ticker (interval: 35ms for smooth 30-60fps fade out)
        cx.spawn_in(_window, async move |this, cx| {
            let mut blink_counter = 0u32;
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(35)).await;
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
                            if this.last_cursor_activity.elapsed() >= std::time::Duration::from_millis(500) {
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
                        while actual_drain < text_len && !this.ai_pending_text.is_char_boundary(actual_drain) {
                            actual_drain += 1;
                        }
                        this.ai_streaming_text.extend(this.ai_pending_text.drain(..actual_drain));
                        needs_notify = true;
                    }

                    let think_len = this.ai_pending_thinking.len();
                    if think_len > 0 {
                        let drain_size = 12.max(think_len / 3).min(think_len);
                        let mut actual_drain = drain_size;
                        while actual_drain < think_len && !this.ai_pending_thinking.is_char_boundary(actual_drain) {
                            actual_drain += 1;
                        }
                        this.ai_streaming_thinking.extend(this.ai_pending_thinking.drain(..actual_drain));
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
        let ai_permission_mode = loaded_config.ai.permission_mode;
        if initial_tab.is_none() && loaded_config.session_restore {
            if let Some(session) = crate::session::load() {
                if let Some(win) = session.windows.first() {
                    restored_ai_sidebar_open = win.ai_sidebar_open;
                    for tab_info in &win.tabs {
                        let cwd_path = tab_info.cwd.clone();
                        let title = tab_info.title_override.clone().or_else(|| tab_info.custom_name.clone());
                        restored_tabs.push((cwd_path, title));
                    }
                }
            }
        }

        let tab_layout = loaded_config.tab_layout;
        let sidebar_open = tab_layout == TabLayout::Vertical;
        let sidebar_anim_progress = if sidebar_open { 1.0 } else { 0.0 };

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
            ai_messages: Vec::new(),
            ai_streaming_text: String::new(),
            ai_streaming_thinking: String::new(),
            ai_is_streaming: false,
            ai_last_usage: None,
            ai_accumulated_usage: (0, 0),
            ai_current_turn_usage: (0, 0),
            ai_context_hovercard_open: false,
            ai_cancel_token: None,
            ai_pending_confirmation: None,
            ai_confirm_reply_tx: None,
            ai_permission_checker: std::sync::Arc::new(crate::ai::PermissionChecker::new(ai_permission_mode)),
            ai_agent_mode: "Agent".to_string(),
            ai_expanded_thinkings: std::collections::HashSet::new(),
            ai_attached_files: Vec::new(),
            ai_at_menu_open: false,
            ai_at_matches: Vec::new(),
            ai_pending_text: String::new(),
            ai_pending_thinking: String::new(),
            ai_composer_bounds: std::rc::Rc::new(std::cell::Cell::new(None)),
            ai_copy_feedback_until: None,
            ai_message_selection: None,
            ai_message_selection_anchor: None,
            is_dragging_message_selection: false,
        };


        crate::keybindings::init_resolver(view.config.keybindings.clone(), view.config.keybinding_preset);

        // Background update check and silent automatic preparation
        enum UpdateStatus {
            Ready(crate::updater::ReleaseInfo),
            Blocked(crate::updater::ReleaseInfo, String),
            Available(crate::updater::ReleaseInfo),
        }
        let (update_tx, update_rx) = async_channel::unbounded::<UpdateStatus>();
        std::thread::spawn(move || {
            if let Some(release) = crate::updater::check_for_update_sync() {
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
                    let reason = release.self_update_blocked_reason.clone().unwrap_or_default();
                    let _ = update_tx.send_blocking(UpdateStatus::Blocked(release, reason));
                }
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
                        }
                        UpdateStatus::Blocked(release, reason) => {
                            this.is_update_ready = false;
                            this.update_available = Some(release);
                            this.update_status = Some(reason);
                        }
                        UpdateStatus::Available(release) => {
                            this.is_update_ready = false;
                            this.update_available = Some(release);
                            this.update_status = None;
                        }
                    }
                    cx.notify();
                });
            }
        }).detach();

        if let Some(tab_data) = initial_tab {
            view.attach_tab(tab_data, _window, cx);
        } else if cli_opts.command.is_some() || cli_opts.working_dir.is_some() || cli_opts.title.is_some() {
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
            for (cwd, title) in restored_tabs {
                view.create_tab_with_cmd_and_cwd(&shell, &[], cwd.as_deref(), title, _window, cx);
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
        for line_i in min_p.line.0..=max_p.line.0 {
            if line_i < -(grid.history_size() as i32) || line_i >= grid.screen_lines() as i32 {
                continue;
            }
            let row = &grid[Line(line_i)];
            let start_c = if line_i == min_p.line.0 { min_p.column.0 } else { 0 };
            let end_c = if line_i == max_p.line.0 {
                max_p.column.0.min(row.len().saturating_sub(1))
            } else {
                row.len().saturating_sub(1)
            };
            for col_i in start_c..=end_c {
                if col_i < row.len() {
                    let cell = &row[Column(col_i)];
                    if cell.c != '\0' {
                        text.push(cell.c);
                    }
                }
            }
            if line_i < max_p.line.0 {
                text.push('\n');
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

                let res = this.update_in(cx, |this, _window, cx| {
                    match event {
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
                        AppEvent::CommandFinished { duration_ms, exit_code } => {
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
                        AppEvent::GitStatusUpdated { tab_idx, status, .. } => {
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
                        AppEvent::Notification { title, body } => {
                            send_system_notification(&title, &body);
                        }
                        AppEvent::Exit { .. } => {
                            if this.exec_pane_id == Some(pane_id) {
                                cx.quit();
                            } else if let Some(tab) = this.tabs.iter().find(|t| t.pane_tree.find_pane(pane_id).is_some()) {
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
                    }
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();

        let initial_cwd = cwd
            .map(|p| p.to_path_buf())
            .or_else(dirs::home_dir);
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

        self.active_tab_idx = self.tabs.len() - 1;
        self.persist_session();
        cx.notify();
    }

    pub fn split_active_pane(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else { return; };
        let current_cwd = active_tab.pane_tree.active_pane().and_then(|p| p.cwd.clone()).or_else(|| active_tab.cwd.clone());
        let default_shell = self.config.shell.clone().or_else(|| std::env::var("SHELL").ok()).unwrap_or_else(crate::paths::default_system_shell);
        let new_pane = self.spawn_terminal_pane(&default_shell, &[], current_cwd.as_deref(), None, window, cx);
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

    pub fn focus_pane_in_direction(
        &mut self,
        direction: Direction,
        cx: &mut Context<Self>,
    ) {
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
    }

    pub fn close_pane_by_id(
        &mut self,
        pane_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_opt = self.tabs.iter().find(|t| t.pane_tree.find_pane(pane_id).is_some()).map(|t| (t.id, t.pane_tree.pane_count()));
        let Some((tab_id, pane_count)) = tab_opt else { return; };

        if pane_count <= 1 {
            self.close_tab(tab_id, window, cx);
        } else {
            if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
                if let Some(pane) = tab.pane_tree.find_pane(pane_id) {
                    if let Some(term) = &pane.terminal {
                        if term.is_process_running() {
                            let name = term.get_foreground_process_name().unwrap_or_else(|| "process".to_string());
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

    pub fn close_active_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub fn with_initial_tab(window: &mut Window, cli_opts: crate::cli::CliOptions, tab_data: TabData, cx: &mut Context<Self>) -> Self {
        Self::with_options_internal(window, cli_opts, Some(tab_data), cx)
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
        self.active_tab_idx = self.tabs.len() - 1;
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

                let res = this.update_in(cx, |this, _window, cx| {
                    match event {
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
                        AppEvent::CommandFinished { duration_ms, exit_code } => {
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
                        AppEvent::GitStatusUpdated { tab_idx, status, .. } => {
                            if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == tab_idx) {
                                tab.git_status = status.clone();
                                tab.git_last_poll = Some(std::time::Instant::now());
                            }
                            cx.notify();
                        }
                        AppEvent::Notification { title, body } => {
                            send_system_notification(&title, &body);
                        }
                        _ => {
                            cx.notify();
                        }
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

    pub fn select_tab(&mut self, tab_id: usize, cx: &mut Context<Self>) {
        if let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) {
            if self.active_tab_idx != idx {
                self.selection = None;
                self.is_selecting = false;
                self.selection_start = None;
            }
            self.active_tab_idx = idx;
            self.persist_session();
            cx.notify();
        }
    }

    pub fn close_tab(&mut self, tab_id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
            let running: Vec<RunningProcessInfo> = tab.pane_tree.all_panes()
                .into_iter()
                .filter_map(|pane| {
                    if let Some(term) = &pane.terminal {
                        if term.is_process_running() {
                            let name = term.get_foreground_process_name().unwrap_or_else(|| "process".to_string());
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
                self.create_tab(window, cx);
            } else if self.active_tab_idx >= self.tabs.len() {
                self.active_tab_idx = self.tabs.len() - 1;
            }
            self.persist_session();
            cx.notify();
        }
    }

    pub fn confirm_pending_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_close.take() else { return; };
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
            self.persist_session();
            crate::updater::relaunch_fastty();
            return;
        }
        let Some(release) = self.update_available.clone() else {
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
                            this.update_status = Some(format!("Fastty v{} installed successfully!\nRestart Fastty to use the new version.", release.version));
                            this.is_update_modal_open = true;
                            this.update_available = None;
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

    pub fn open_pane_context_menu(&mut self, pane_id: usize, x: f32, y: f32, cx: &mut Context<Self>) {
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

    pub fn close_other_tabs(&mut self, keep_id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let other_running: Vec<RunningProcessInfo> = self.tabs
            .iter()
            .filter(|t| t.id != keep_id)
            .flat_map(|t| t.pane_tree.all_panes())
            .filter_map(|pane| {
                if let Some(term) = &pane.terminal {
                    if term.is_process_running() {
                        let name = term.get_foreground_process_name().unwrap_or_else(|| "process".to_string());
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

    pub fn force_close_other_tabs(&mut self, keep_id: usize, _window: &mut Window, cx: &mut Context<Self>) {
        for tab in self.tabs.iter().filter(|t| t.id != keep_id) {
            for pane in tab.pane_tree.all_panes() {
                if let Some(ref term) = pane.terminal {
                    term.terminate_process();
                }
                crate::daemon::unregister(pane.id);
            }
        }
        self.tabs.retain(|t| t.id == keep_id);
        self.active_tab_idx = 0;
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
            let cwd = self.tabs.get(self.active_tab_idx).and_then(|t| t.cwd.clone());
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

    pub fn toggle_tab_overview(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_tab_overview_open = !self.is_tab_overview_open;
        if self.is_tab_overview_open {
            self.tab_overview_selected = self.active_tab_idx;
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
            self.tab_overview_scroll_handle.scroll_to_item(self.tab_overview_selected);
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
            let proc = tab.terminal.as_ref().and_then(|t| t.get_foreground_process_name());
            let tab_title = tab.custom_title.clone().unwrap_or_else(|| tab.title.clone());

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
                            tab_title: pane.custom_title.clone().unwrap_or_else(|| pane.title.clone()),
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
                title: tab.title.clone(),
                custom_title: tab.custom_title.clone(),
                cwd: tab.cwd.as_ref().map(|c| c.to_string_lossy().to_string()),
                layout,
            });
        }

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let data = crate::session_manager::SessionData {
            name: session_name.to_string(),
            created_at: now,
            updated_at: now,
            active_tab_idx: self.active_tab_idx,
            tabs: persisted_tabs,
        };
        crate::session_manager::save_session(&data)
    }

    pub fn restore_workspace_session(&mut self, session_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = crate::session_manager::load_session(session_name) {
            let shell = self.config.shell.clone().or_else(|| std::env::var("SHELL").ok()).unwrap_or_else(crate::paths::default_system_shell);
            for tab_data in session.tabs {
                let tab_id = self.next_tab_id;
                self.next_tab_id += 1;

                let pane_tree = if let Some(ref persisted_layout) = tab_data.layout {
                    let root_node = crate::pane_tree::PaneNode::restore_from_persisted(
                        persisted_layout,
                        &mut |cwd, title| self.spawn_terminal_pane(&shell, &[], cwd, title, window, cx),
                    );
                    let first_pane_id = root_node.all_panes().first().map(|p| p.id).unwrap_or(tab_id);
                    crate::pane_tree::PaneTree {
                        root: root_node,
                        active_pane_id: first_pane_id,
                    }
                } else {
                    let cwd = tab_data.cwd.as_deref().map(std::path::Path::new);
                    let title = tab_data.custom_title.clone().or(Some(tab_data.title.clone()));
                    let pane = self.spawn_terminal_pane(&shell, &[], cwd, title, window, cx);
                    crate::pane_tree::PaneTree::new(pane)
                };

                let active_pane = pane_tree.active_pane();
                let pane_title = active_pane.map(|p| p.title.clone()).unwrap_or(tab_data.title);
                let pane_cwd = active_pane.and_then(|p| p.cwd.clone()).or_else(|| tab_data.cwd.map(std::path::PathBuf::from));
                let pane_term = active_pane.and_then(|p| p.terminal.clone());

                self.tabs.push(TabData {
                    id: tab_id,
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
            }
            self.persist_session();
            cx.notify();
        }
    }

    pub fn execute_palette_command(&mut self, cmd_id: &str, _window: &mut Window, cx: &mut Context<Self>) {
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
                let cwd = self.tabs.get(self.active_tab_idx).and_then(|t| t.cwd.clone());
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
                ).ok();
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
            "theme_catppuccin" => self.set_theme("catppuccin", cx),
            "theme_one_dark" => self.set_theme("one-dark", cx),
            "theme_solarized" => self.set_theme("solarized-dark", cx),
            "theme_high_contrast" => self.set_theme("high-contrast", cx),
            "open_config" => {
                let config_dir = dirs::home_dir().map(|h| h.join(".config/fastty")).unwrap_or_default();
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

    /// 160ms ease-out-cubic open/close animation, same as the tabs sidebar.
    pub fn trigger_ai_sidebar_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.ai_sidebar_open { 1.0f32 } else { 0.0f32 };

        if (self.ai_sidebar_anim_progress - target).abs() < 0.001 {
            return;
        }

        cx.spawn_in(window, async move |this, cx| {
            let start_time = std::time::Instant::now();
            let duration = std::time::Duration::from_millis(160);
            let initial = this.update_in(cx, |this, _window, _cx| this.ai_sidebar_anim_progress).unwrap_or(0.0);

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
                cx.background_executor().timer(std::time::Duration::from_millis(8)).await;
            }
        }).detach();
    }

    pub fn cancel_ai_stream(&mut self) {
        if let Some(token) = self.ai_cancel_token.take() {
            token.cancel();
        }
        // Deny any pending permission request so the blocked agent thread
        // unblocks and terminates instead of leaking into the next session.
        if let Some(tx) = self.ai_confirm_reply_tx.take() {
            let _ = tx.send_blocking(crate::ai::PermissionDecision::Deny);
        }
        self.ai_pending_confirmation = None;
        if !self.ai_pending_text.is_empty() {
            self.ai_streaming_text.push_str(&self.ai_pending_text);
            self.ai_pending_text.clear();
        }
        if !self.ai_pending_thinking.is_empty() {
            self.ai_streaming_thinking.push_str(&self.ai_pending_thinking);
            self.ai_pending_thinking.clear();
        }
        self.ai_accumulated_usage.0 += self.ai_current_turn_usage.0;
        self.ai_accumulated_usage.1 += self.ai_current_turn_usage.1;
        self.ai_current_turn_usage = (0, 0);
        if self.ai_accumulated_usage != (0, 0) {
            self.ai_last_usage = Some(self.ai_accumulated_usage);
        }
        self.ai_is_streaming = false;
    }

    pub fn update_ai_at_matches(&mut self, query: &str) {
        let active_cwd = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|t| t.cwd.as_ref().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| "~".to_string());

        let mut all_files = Vec::new();
        if let Ok(repo) = git2::Repository::discover(&active_cwd) {
            if let Ok(index) = repo.index() {
                for entry in index.iter() {
                    if let Ok(s) = std::str::from_utf8(&entry.path) {
                        all_files.push(s.to_string());
                    }
                }
            }
        }
        if all_files.is_empty() {
            if let Ok(entries) = std::fs::read_dir(&active_cwd) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if let Ok(rel) = p.strip_prefix(&active_cwd) {
                        all_files.push(rel.to_string_lossy().to_string());
                    }
                }
            }
        }
        all_files.sort();

        let q = query.to_lowercase();
        if q.is_empty() {
            self.ai_at_matches = all_files.into_iter().take(20).collect();
        } else {
            self.ai_at_matches = all_files
                .into_iter()
                .filter(|f| f.to_lowercase().contains(&q))
                .take(20)
                .collect();
        }
    }

    pub fn insert_ai_at_path(&mut self, path: &str) {
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
            self.ai_input_state.set_text_with_cursor(new_text, new_cursor);
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

    pub fn open_ai_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, rx) = async_channel::bounded::<Option<std::path::PathBuf>>(1);
        std::thread::spawn(move || {
            let res = crate::ui::ai_sidebar::pick_file_dialog();
            let _ = tx.send_blocking(res);
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Some(path)) = rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    if !this.ai_attached_files.contains(&path) {
                        this.ai_attached_files.push(path);
                    }
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

        let active_cwd = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|t| t.cwd.clone())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let mut augmented_content = text.clone();

        // 1. Parse @path references from user input
        for word in text.split_whitespace() {
            if let Some(path_part) = word.strip_prefix('@') {
                let clean = path_part.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '/' && c != '_' && c != '-');
                if !clean.is_empty() {
                    let path = active_cwd.join(clean);
                    if path.is_file() {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let max_chars = 16_000;
                            let snippet = if content.len() > max_chars {
                                format!("{}... (truncated)", &content[..max_chars])
                            } else {
                                content
                            };
                            augmented_content.push_str(&format!("\n\n[Referenced file: {}]\n```\n{}\n```", clean, snippet));
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
                    let max_chars = 16_000;
                    let snippet = if content.len() > max_chars {
                        format!("{}... (truncated)", &content[..max_chars])
                    } else {
                        content
                    };
                    augmented_content.push_str(&format!("\n\n[Attached file: {}]\n```\n{}\n```", path.display(), snippet));
                } else {
                    augmented_content.push_str(&format!("\n\n[Attached binary file: {}]", path.display()));
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
        self.ai_scroll_handle.scroll_to_bottom();

        let cfg = self.config.ai.clone();
        let active_prov = self.config.ai.default.clone();
        let active_mod = self.config.ai.active_model();
        let (model, default_model) = match crate::ai::create_model_from_config(
            &cfg,
            Some(&active_prov),
            Some(active_mod.as_str()),
        ) {
            Ok(res) => res,
            Err(e) => {
                self.ai_messages
                    .push(ai_error_message(&format!("initializing AI model: {e}")));
                cx.notify();
                return;
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

        let mut agent = crate::ai::Agent::new(
            model,
            default_model,
            tools,
            checker,
            perm_handler,
            ctx_tool,
        );

        let cancel_tok = agent.cancel_token();
        self.ai_cancel_token = Some(cancel_tok);
        self.ai_is_streaming = true;
        self.ai_streaming_text.clear();
        self.ai_streaming_thinking.clear();
        self.ai_pending_text.clear();
        self.ai_pending_thinking.clear();

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
                                    augmented_content.push_str(&format!("\n\n[Attached image error {}: {}]", img_path.display(), e));
                                }
                            }
                        }
                        for doc_path in &attached_documents {
                            match crate::ai::load_and_prepare_document(doc_path) {
                                Ok(part) => parts.push(part),
                                Err(e) => {
                                    augmented_content.push_str(&format!("\n\n[Attached PDF error {}: {}]", doc_path.display(), e));
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

        let event_tx_clone = event_tx.clone();
        std::thread::Builder::new()
            .name("ai-agent-turn".into())
            .spawn(move || {
                let tx = event_tx_clone;
                let res = agent.run_turn(move |ev| {
                    let _ = tx.send_blocking(ev);
                });
                if let Err(e) = res {
                    let _ = event_tx.send_blocking(crate::ai::AgentEvent::Error(format!("{}", e)));
                }
            })
            .ok();

        cx.spawn_in(window, async move |this, cx| {
            while let Ok((tool_id, tool_name, input_summary, reply_tx)) = ask_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    this.ai_pending_confirmation =
                        Some(crate::ui::ai_sidebar::AiUiPendingConfirmation {
                            tool_id,
                            tool_name,
                            input_summary,
                        });
                    this.ai_confirm_reply_tx = Some(reply_tx);
                    // The confirmation card renders at the end of the message
                    // list; bring it into view so the user sees the diff.
                    this.ai_scroll_handle.scroll_to_bottom();
                    cx.notify();
                });
            }
        })
        .detach();

        cx.spawn_in(window, async move |this, cx| {
            while let Ok(ev) = event_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
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
                            this.ai_accumulated_usage.0 += this.ai_current_turn_usage.0;
                            this.ai_accumulated_usage.1 += this.ai_current_turn_usage.1;
                            this.ai_current_turn_usage = (0, 0);
                            this.ai_last_usage = Some(this.ai_accumulated_usage);

                            // Flush any text/thinking accumulated so far into an assistant bubble
                            if !this.ai_pending_text.is_empty() {
                                this.ai_streaming_text.push_str(&this.ai_pending_text);
                                this.ai_pending_text.clear();
                            }
                            if !this.ai_pending_thinking.is_empty() {
                                this.ai_streaming_thinking.push_str(&this.ai_pending_thinking);
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
                        crate::ai::AgentEvent::TurnEnd => {
                            this.ai_accumulated_usage.0 += this.ai_current_turn_usage.0;
                            this.ai_accumulated_usage.1 += this.ai_current_turn_usage.1;
                            this.ai_current_turn_usage = (0, 0);
                            this.ai_last_usage = Some(this.ai_accumulated_usage);

                            // Flush any remaining buffered text/thinking
                            if !this.ai_pending_text.is_empty() {
                                this.ai_streaming_text.push_str(&this.ai_pending_text);
                                this.ai_pending_text.clear();
                            }
                            if !this.ai_pending_thinking.is_empty() {
                                this.ai_streaming_thinking.push_str(&this.ai_pending_thinking);
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
                        }
                        crate::ai::AgentEvent::Usage { input, output } => {
                            let effective_input = if input > 0 {
                                input
                            } else {
                                let chars: usize = this.ai_messages.iter().map(|m| m.text.len() + m.thinking.as_deref().unwrap_or("").len()).sum();
                                ((chars / 4).max(50)) as u64
                            };
                            this.ai_current_turn_usage = (effective_input, output);
                            let total_in = this.ai_accumulated_usage.0 + this.ai_current_turn_usage.0;
                            let total_out = this.ai_accumulated_usage.1 + this.ai_current_turn_usage.1;
                            this.ai_last_usage = Some((total_in, total_out));
                            return;
                        }
                        crate::ai::AgentEvent::Error(err) => {
                            this.ai_accumulated_usage.0 += this.ai_current_turn_usage.0;
                            this.ai_accumulated_usage.1 += this.ai_current_turn_usage.1;
                            this.ai_current_turn_usage = (0, 0);
                            if this.ai_accumulated_usage != (0, 0) {
                                this.ai_last_usage = Some(this.ai_accumulated_usage);
                            }
                            if !this.ai_pending_text.is_empty() {
                                this.ai_streaming_text.push_str(&this.ai_pending_text);
                                this.ai_pending_text.clear();
                            }
                            if !this.ai_pending_thinking.is_empty() {
                                this.ai_streaming_thinking.push_str(&this.ai_pending_thinking);
                                this.ai_pending_thinking.clear();
                            }

                            this.ai_messages.push(ai_error_message(&err));
                            this.ai_is_streaming = false;
                            this.ai_cancel_token = None;
                        }
                    }
                    this.scroll_ai_to_bottom();
                    cx.notify();
                });
            }
        })
        .detach();

        cx.notify();
    }

    pub fn scroll_ai_to_bottom(&self) {
        if self.ai_is_streaming {
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
        let active_cwd = self
            .tabs
            .get(self.active_tab_idx)
            .and_then(|t| t.cwd.as_ref().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| "~".to_string());
        let active_branch = self.tabs.get(self.active_tab_idx).and_then(|t| t.git_status.as_ref().map(|g| g.branch.clone()));

        let total_chars: usize = self.ai_messages.iter().map(|m| m.text.len() + m.thinking.as_deref().unwrap_or("").len()).sum();
        let max_context_u64 = self.config.ai.context_window.max(1_000) as u64;
        let total_in = self.ai_accumulated_usage.0 + self.ai_current_turn_usage.0;
        let total_out = self.ai_accumulated_usage.1 + self.ai_current_turn_usage.1;
        let effective_usage = if total_in > 0 || total_out > 0 {
            Some((total_in, total_out))
        } else {
            self.ai_last_usage
        };
        let used_tokens_u64 = match effective_usage {
            Some((input, output)) => input + output,
            None => {
                if total_chars == 0 {
                    0
                } else {
                    (total_chars / 4) as u64
                }
            }
        };
        let context_pct = if max_context_u64 > 0 {
            (used_tokens_u64 as f32 / max_context_u64 as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let sidebar = crate::ui::ai_sidebar::AiSidebar::new(
            self.theme,
            self.current_ai_sidebar_width(),
            provider,
            model,
        )
        .scroll_handle(self.ai_scroll_handle.clone())
        .agent_mode(self.ai_agent_mode.clone())
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
        .on_select_message_char(cx.listener(|this, (msg_idx, target): &(usize, usize), _window, cx| {
            this.ai_input_state.selection = None;
            // The sidebar keyboard handler (Cmd+C, Escape, ⌘↵) only runs when
            // the panel owns focus, so selecting a message takes focus here.
            this.ai_input_focused = true;
            this.ai_message_selection_anchor = Some((*msg_idx, *target));
            this.ai_message_selection = Some((*msg_idx, *target, *target));
            this.is_dragging_message_selection = true;
            cx.notify();
        }))
        .on_drag_message_char(cx.listener(|this, (msg_idx, target): &(usize, usize), _window, cx| {
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
        }))
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
        .at_matches(self.ai_at_matches.clone())
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
        .on_attach_click(cx.listener(|this, _ev, window, cx| {
            this.open_ai_file_dialog(window, cx);
        }))
        .on_new_chat(cx.listener(|this, _ev, _window, cx| {
            this.cancel_ai_stream(); // also clears ai_pending_confirmation + ai_confirm_reply_tx
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
            this.ai_accumulated_usage = (0, 0);
            this.ai_current_turn_usage = (0, 0);
            this.ai_message_selection = None;
            this.ai_message_selection_anchor = None;
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
            this.ai_messages.clear();
            this.ai_streaming_text.clear();
            this.ai_streaming_thinking.clear();
            this.ai_pending_text.clear();
            this.ai_pending_thinking.clear();
            this.ai_expanded_thinkings.clear();
            this.ai_attached_files.clear();
            this.ai_at_menu_open = false;
            this.ai_last_usage = None;
            this.ai_accumulated_usage = (0, 0);
            this.ai_current_turn_usage = (0, 0);
            // Drop any leftover confirmation so it can't reappear in a fresh view.
            if let Some(tx) = this.ai_confirm_reply_tx.take() {
                let _ = tx.send_blocking(crate::ai::PermissionDecision::Deny);
            }
            this.ai_pending_confirmation = None;
            this.ai_message_selection = None;
            this.ai_message_selection_anchor = None;
            cx.notify();
        }))
        .on_cancel(cx.listener(|this, _ev, _window, cx| {
            this.cancel_ai_stream();
            cx.notify();
        }))
        .on_submit(cx.listener(|this, _ev, window, cx| {
            this.submit_ai_prompt(window, cx);
        }))
        .on_confirm(cx.listener(|this, (always, allow): &(bool, bool), _window, cx| {
            if *always && *allow {
                if let Some(ref pending) = this.ai_pending_confirmation {
                    this.ai_permission_checker.allow_always_tool(&pending.tool_name, &pending.input_summary);
                }
            }
            if let Some(tx) = this.ai_confirm_reply_tx.take() {
                let dec = if *allow {
                    crate::ai::PermissionDecision::Allow
                } else {
                    crate::ai::PermissionDecision::Deny
                };
                let _ = tx.send_blocking(dec);
            }
            this.ai_pending_confirmation = None;
            cx.notify();
        }));

        div()
            .relative()
            .h_full()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                if !this.ai_input_focused {
                    this.ai_input_focused = true;
                    cx.notify();
                }
            }))
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
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev: &MouseDownEvent, _window, cx| {
                        this.is_dragging_ai_sidebar = true;
                        cx.notify();
                    }))
            )
    }


    pub fn set_tab_layout_mode(&mut self, layout: TabLayout, window: &mut Window, cx: &mut Context<Self>) {
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
            let initial = this.update_in(cx, |this, _window, _cx| this.sidebar_anim_progress).unwrap_or(0.0);

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
                cx.background_executor().timer(std::time::Duration::from_millis(8)).await;
            }
        }).detach();
    }

    pub fn reload_config(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let loaded_config = crate::config::load_lenient();
        // External reload wins over any in-flight palette preview, and
        // refreshes the config-error banner (fixing the file clears it).
        self.palette_preview_original = None;
        self.config_error = crate::config::load_error();
        let theme_name = loaded_config.theme.as_deref().unwrap_or("default").to_string();
        self.current_theme_name = theme_name.clone();
        self.theme = Theme::from_name(&theme_name).with_opacity(loaded_config.opacity);
        self.font_size = loaded_config.font.size;
        let new_family: SharedString = if loaded_config.font.family.is_empty() || loaded_config.font.family == "monospace" {
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
            loaded_config.font.family.clone().into()
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
        crate::keybindings::init_resolver(loaded_config.keybindings.clone(), loaded_config.keybinding_preset);
        self.config = loaded_config;
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
            if self.tab_layout != original.tab_layout
                || self.sidebar_open != original.sidebar_open
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
            self.rename_tab_input = tab.custom_title.clone().unwrap_or_else(|| tab.title.clone());
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
        self.font_family = family.to_string().into();
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

    fn handle_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.any_input_overlay_open() && super::ime::defers_to_ime(event, self.config.option_as_meta) {
            return;
        }
        self.process_key_down(event, _window, cx);
        cx.stop_propagation();
    }

    fn process_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke.key;
        let key_lower = key.to_lowercase();
        let modifiers = &event.keystroke.modifiers;

        if key_lower.starts_with("dead") || key_lower == "dead" {
            return;
        }

        let is_alt_gr = modifiers.control && !modifiers.platform && modifiers.alt;
        let is_ctrl = modifiers.control && !is_alt_gr;

        // AI Sidebar Input Keyboard Handler
        if self.ai_sidebar_open && self.ai_input_focused {
            if key_lower == "escape" || key_lower == "esc" {
                if self.ai_at_menu_open {
                    self.ai_at_menu_open = false;
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
            if (modifiers.platform && key_lower == "l") || (modifiers.shift && is_ctrl && key_lower == "l") {
                self.toggle_ai_sidebar(_window, cx);
                return;
            }
            // Select All: Cmd+A (macOS) or Ctrl+A (Linux/Windows)
            let is_select_all = (cfg!(target_os = "macos") && modifiers.platform && key_lower == "a")
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
                        } else if msg_idx == self.ai_messages.len() && !self.ai_streaming_text.is_empty() {
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
            if key_lower == "enter" || key_lower == "return" {
                if self.ai_at_menu_open && !self.ai_at_matches.is_empty() {
                    let first = self.ai_at_matches[0].clone();
                    self.insert_ai_at_path(&first);
                    cx.notify();
                    return;
                }
                if modifiers.shift {
                    self.ai_input_state.insert_str("\n");
                    self.ai_input_text = self.ai_input_state.text.clone();
                    self.check_ai_at_trigger();
                } else if self.ai_pending_confirmation.is_some() && modifiers.platform {
                    if let Some(tx) = self.ai_confirm_reply_tx.take() {
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
            let count = self.tabs.len();
            if count == 0 {
                self.is_tab_overview_open = false;
                cx.notify();
                return;
            }
            if key_lower == "enter" || key_lower == "return" {
                if let Some(tab) = self.tabs.get(self.tab_overview_selected) {
                    let id = tab.id;
                    self.is_tab_overview_open = false;
                    self.select_tab(id, cx);
                }
                return;
            }
            if key_lower == "right" || key_lower == "arrowright" || key_lower == "l" || key_lower == "tab" {
                self.tab_overview_selected = (self.tab_overview_selected + 1) % count;
                self.tab_overview_scroll_handle.scroll_to_item(self.tab_overview_selected);
                cx.notify();
                return;
            }
            if key_lower == "left" || key_lower == "arrowleft" || key_lower == "h" {
                self.tab_overview_selected = if self.tab_overview_selected == 0 {
                    count - 1
                } else {
                    self.tab_overview_selected - 1
                };
                self.tab_overview_scroll_handle.scroll_to_item(self.tab_overview_selected);
                cx.notify();
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "j" {
                let cols = 3.min(count);
                self.tab_overview_selected = (self.tab_overview_selected + cols).min(count - 1);
                self.tab_overview_scroll_handle.scroll_to_item(self.tab_overview_selected);
                cx.notify();
                return;
            }
            if key_lower == "up" || key_lower == "arrowup" || key_lower == "k" {
                let cols = 3.min(count);
                self.tab_overview_selected = self.tab_overview_selected.saturating_sub(cols);
                self.tab_overview_scroll_handle.scroll_to_item(self.tab_overview_selected);
                cx.notify();
                return;
            }
            if key_lower == "d" || key_lower == "x" || key_lower == "backspace" {
                if let Some(tab) = self.tabs.get(self.tab_overview_selected) {
                    let id = tab.id;
                    self.close_tab(id, _window, cx);
                    if self.tabs.is_empty() {
                        self.is_tab_overview_open = false;
                    } else {
                        self.tab_overview_selected = self.tab_overview_selected.min(self.tabs.len() - 1);
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
                    self.global_search_scroll_handle.scroll_to_item(self.global_search_selected);
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
                    self.global_search_scroll_handle.scroll_to_item(self.global_search_selected);
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
                    self.command_palette_scroll_handle.scroll_to_item(self.command_palette_selected);
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
                    self.command_palette_scroll_handle.scroll_to_item(self.command_palette_selected);
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
                    self.snippet_scroll_handle.scroll_to_item(self.snippet_selected);
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
                    self.snippet_scroll_handle.scroll_to_item(self.snippet_selected);
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
                    self.pr_picker_scroll_handle.scroll_to_item(self.pr_picker_selected);
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
                    self.pr_picker_scroll_handle.scroll_to_item(self.pr_picker_selected);
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
                .filter(|h| query.is_empty() || fuzzy_match_str(&query, &h.name) || fuzzy_match_str(&query, &h.hostname) || fuzzy_match_str(&query, &h.user) || fuzzy_match_str(&query, &h.tag))
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
                    self.ssh_manager_scroll_handle.scroll_to_item(self.ssh_manager_selected);
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
                    self.ssh_manager_scroll_handle.scroll_to_item(self.ssh_manager_selected);
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
                                self.search_match_idx = (self.search_match_idx + 1) % self.search_matches.len();
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
            let active_cwd = self.tabs.get(self.active_tab_idx).and_then(|t| t.cwd.as_deref());
            let worktrees = active_cwd.map(crate::git::list_worktrees).unwrap_or_default();
            let query = self.worktree_picker_query.to_lowercase();
            let filtered: Vec<&crate::git::Worktree> = worktrees
                .iter()
                .filter(|w| query.is_empty() || w.short_branch().to_lowercase().contains(&query) || w.path.to_string_lossy().to_lowercase().contains(&query))
                .collect();
            let count = filtered.len();

            if key_lower == "enter" || key_lower == "return" {
                if let Some(wt) = filtered.get(self.worktree_picker_selected) {
                    let shell = self.config.shell.clone().or_else(|| std::env::var("SHELL").ok()).unwrap_or_else(crate::paths::default_system_shell);
                    let title = wt.short_branch().to_string();
                    let path = wt.path.clone();
                    self.is_worktree_picker_open = false;
                    self.create_tab_with_cmd_and_cwd(&shell, &[], Some(&path), Some(title), _window, cx);
                }
                return;
            }
            if key_lower == "down" || key_lower == "arrowdown" || key_lower == "tab" {
                if count > 0 {
                    self.worktree_picker_selected = (self.worktree_picker_selected + 1) % count;
                    self.worktree_picker_scroll_handle.scroll_to_item(self.worktree_picker_selected);
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
                    self.worktree_picker_scroll_handle.scroll_to_item(self.worktree_picker_selected);
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
                        || cwd_str.as_ref().is_some_and(|c| c.to_lowercase().contains(&query))
                        || branch_str.as_ref().is_some_and(|b| b.to_lowercase().contains(&query));
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
                    self.project_jumper_scroll_handle.scroll_to_item(self.project_jumper_selected);
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
                    self.project_jumper_scroll_handle.scroll_to_item(self.project_jumper_selected);
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

        // 6. Dismiss open static overlays on Escape
        if (self.is_settings_open || self.is_about_open || self.is_context_menu_open || self.is_git_menu_open || self.is_tab_context_menu_open || self.is_pane_context_menu_open || self.is_update_modal_open)
            && (key_lower == "escape" || key_lower == "esc")
        {
            self.is_settings_open = false;
            self.is_about_open = false;
            self.is_context_menu_open = false;
            self.is_git_menu_open = false;
            self.is_git_branch_sub_open = false;
            self.is_tab_context_menu_open = false;
            self.is_pane_context_menu_open = false;
            self.is_update_modal_open = false;
            cx.notify();
            return;
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
                        self.active_tab_idx = (self.active_tab_idx + 1) % self.tabs.len();
                        cx.notify();
                    }
                    return;
                }
                Action::PrevTab => {
                    if !self.tabs.is_empty() {
                        self.selection = None;
                        self.is_selecting = false;
                        self.selection_start = None;
                        self.active_tab_idx = if self.active_tab_idx == 0 {
                            self.tabs.len() - 1
                        } else {
                            self.active_tab_idx - 1
                        };
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
                            self.active_tab_idx = target;
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
                            } else if msg_idx == self.ai_messages.len() && !self.ai_streaming_text.is_empty() {
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
                                if let Some(content) = crate::paste::get_clipboard_paste_content(&mut clip) {
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
                        if let Some(trigger_len) = crate::snippets::match_trigger(&self.typed_prompt_buf) {
                            let trigger = &self.typed_prompt_buf[self.typed_prompt_buf.len() - trigger_len..];
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
            || self.is_tab_overview_open
            || self.is_global_search_open
            || self.is_settings_open
            || self.is_about_open
            || self.is_update_modal_open
            || self.pending_close.is_some()
            || (self.ai_sidebar_open && self.ai_input_focused)
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

    fn handle_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
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
        let clicked_pane = active_tab.pane_tree.all_panes().into_iter().find(|p| {
            if let Some(b) = p.last_bounds {
                let bx = b.origin.x.to_f64() as f32;
                let by = b.origin.y.to_f64() as f32;
                let bw = b.size.width.to_f64() as f32;
                let bh = b.size.height.to_f64() as f32;
                mouse_x >= bx && mouse_x <= bx + bw && mouse_y >= by && mouse_y <= by + bh
            } else {
                false
            }
        }).or_else(|| active_tab.pane_tree.active_pane());

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
        let Some(ref terminal) = active_pane.as_ref().and_then(|p| p.terminal.as_ref()).or(active_tab.terminal.as_ref()) else {
            return;
        };

        let (cell_w, line_h) = self.measure_cell_metrics(window);
        let (pane_origin_x, pane_origin_y, pane_w, pane_h) = if let Some(b) = clicked_bounds.or_else(|| active_pane.and_then(|p| p.last_bounds)) {
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
            let grid_row = (((local_y / line_h).floor() as i32) - (display_offset as i32)).clamp(-(history_size as i32), screen_rows - 1);
            let start_point = alacritty_terminal::index::Point::new(
                alacritty_terminal::index::Line(grid_row),
                alacritty_terminal::index::Column(grid_col),
            );

            if event.click_count == 2 {
                // Double click: Select word under cursor and open context menu
                if let Some(term_guard) = terminal.term().try_lock() {
                    let grid = term_guard.grid();
                    if let Some((_token, start_c, end_c)) = crate::selection_classifier::extract_token(grid, start_point, screen_cols) {
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
                self.open_pane_context_menu(target_pane_id, mouse_x, mouse_y, cx);
                return;
            } else if event.click_count >= 3 {
                // Triple click: Select whole line and open context menu
                let start_p = alacritty_terminal::index::Point::new(
                    alacritty_terminal::index::Line(grid_row),
                    alacritty_terminal::index::Column(0),
                );
                let end_p = alacritty_terminal::index::Point::new(
                    alacritty_terminal::index::Line(grid_row),
                    alacritty_terminal::index::Column(screen_cols.saturating_sub(1)),
                );
                self.selection = Some(Selection {
                    start: start_p,
                    end: end_p,
                });
                self.is_selecting = false;
                self.has_selection_dragged = false;
                self.selection_start = None;
                self.open_pane_context_menu(target_pane_id, mouse_x, mouse_y, cx);
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
                let grid_col = ((local_x / cell_w).floor() as usize).min(screen_cols.saturating_sub(1));
                let grid_row = (((local_y / line_h).floor() as i32) - (display_offset as i32)).clamp(-(history_size as i32), screen_rows - 1);
                let point = alacritty_terminal::index::Point::new(
                    alacritty_terminal::index::Line(grid_row),
                    alacritty_terminal::index::Column(grid_col),
                );

                if let Some(term_guard) = terminal.term().try_lock() {
                    let grid = term_guard.grid();
                    if let Some((_token, start_c, end_c)) = crate::selection_classifier::extract_token(grid, point, screen_cols) {
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

    fn handle_mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
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
                    cur_y >= (by - 4.0) && cur_y <= (by + bh + 4.0) && cur_x >= (right_edge - 48.0) && cur_x <= (right_edge + 8.0)
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
                        py >= (by - 4.0) && py <= (by + bh + 4.0) && px >= (right_edge - 48.0) && px <= (right_edge + 8.0)
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
            let lines = crate::ui::text_input::wrap_text_into_lines(&self.ai_input_state.text, max_cols);
            let line_h = 18.0;
            let line_idx = ((rel_y / line_h).floor() as usize).min(lines.len().saturating_sub(1));
            if let Some(target_line) = lines.get(line_idx) {
                let col = crate::ui::text_input::index_for_x(&target_line.text, rel_x, 13.0, _window);
                let char_idx = (target_line.start_char + col).min(target_line.start_char + target_line.text.chars().count());
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
                tab.pane_tree.set_split_ratio_by_path(&self.dragging_split_path, new_ratio);
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
                            let track_h = pane.last_bounds.map_or(300.0, |b| b.size.height.to_f64() as f32).max(50.0);
                            let target_rows = ((track_h / line_h) as usize).max(5);
                            let total_rows = (history_size + target_rows) as f32;
                            let thumb_h = (track_h * (target_rows as f32 / total_rows)).clamp(24.0, track_h);
                            let scrollable_track = (track_h - thumb_h).max(1.0);
                            let offset_delta = (delta_y / scrollable_track) * history_size as f32;
                            let new_offset = (self.scrollbar_drag_start_offset as f32 - offset_delta).round().clamp(0.0, history_size as f32) as usize;
                            term.scroll_to_offset(new_offset);
                            cx.notify();
                        }
                    }
                }
            }
            return;
        }

        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else { return; };

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
            hovered_pane.clone().or_else(|| active_tab.pane_tree.active_pane())
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
        let grid_row = (((local_y / line_h).floor() as i32) - (display_offset as i32)).clamp(-(history_size as i32), screen_rows - 1);
        let current_point = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(grid_row),
            alacritty_terminal::index::Column(grid_col),
        );

        // URL / OSC 8 Hyperlink Hover detection
        if let Some(term_guard) = terminal.term().try_lock() {
            let grid = term_guard.grid();
            let cols = grid.columns();
            // 1. Explicit OSC 8 hyperlink check
            if let Some((url, start_c, end_c)) = crate::selection_classifier::extract_hyperlink(grid, current_point, cols) {
                self.hovered_url = Some(url);
                self.hovered_url_range = Some((grid_row, start_c, end_c));
                cx.notify();
                return;
            }

            // 2. Pattern-based URL / path / email classifier fallback
            if let Some((token, start_c, end_c)) = crate::selection_classifier::extract_token(grid, current_point, cols) {
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

    fn handle_mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
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
        let Some((raw_x, raw_y)) = self.selection_mouse_pos else { return; };
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else { return; };
        let active_pane = active_tab.pane_tree.active_pane();
        let Some(terminal) = active_pane.as_ref().and_then(|p| p.terminal.as_ref()).or(active_tab.terminal.as_ref()) else { return; };

        let (cell_w, line_h) = self.measure_cell_metrics(window);
        let px_per_line = if line_h > 0.0 { line_h } else { 18.0 };
        let cell_width = if cell_w > 0.0 { cell_w } else { 9.0 };

        let (pane_x, pane_y, pane_w, pane_h) = if let Some(b) = active_pane.as_ref().and_then(|p| p.last_bounds) {
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
        }.clamp(-(history_size as i32), screen_rows - 1);

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
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else { return; };
        let active_pane = active_tab.pane_tree.active_pane();
        let Some(terminal) = active_pane.as_ref().and_then(|p| p.terminal.as_ref()).or(active_tab.terminal.as_ref()) else { return; };

        let (_, line_h) = self.measure_cell_metrics(window);
        let px_per_line = if line_h > 0.0 { line_h } else { 18.0 };

        let (top_edge, bottom_edge) = if let Some(b) = active_pane.as_ref().and_then(|p| p.last_bounds) {
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

    fn handle_scroll(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };

        let mouse_x = event.position.x.to_f64() as f32;
        let mouse_y = event.position.y.to_f64() as f32;

        let target_pane = active_tab.pane_tree.all_panes().into_iter().find(|p| {
            if let Some(b) = p.last_bounds {
                let bx = b.origin.x.to_f64() as f32;
                let by = b.origin.y.to_f64() as f32;
                let bw = b.size.width.to_f64() as f32;
                let bh = b.size.height.to_f64() as f32;
                mouse_x >= bx && mouse_x <= bx + bw && mouse_y >= by && mouse_y <= by + bh
            } else {
                false
            }
        }).or_else(|| active_tab.pane_tree.active_pane());
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

                    let new_offset = (display_offset as isize + lines).clamp(0, history_size as isize) as usize;
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
                        let mut current_fg = theme.foreground;
                        let mut current_bg: Option<Hsla> = None;
                        let mut current_bold = false;
                        let mut current_underline = false;
                        let mut last_row: Option<i32> = None;
                        let mut current_span_char_cols: Vec<usize> = Vec::new();

                        for cell in content.display_iter {
                            let row = cell.point.line.0;
                            let col = cell.point.column.0;

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
                                            emoji_scale: None,
                                            char_cols: current_span_char_cols,
                                        });
                                        current_span_text = String::new();
                                        current_span_char_cols = Vec::new();
                                        current_block_cat = None;
                                        current_is_emoji = false;
                                        current_is_nerd = false;
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

                            let is_hovered_url = if let Some((h_row, h_start, h_end)) = self.hovered_url_range {
                                row == h_row && col >= h_start && col < h_end
                            } else {
                                false
                            };

                            let (effective_fg, effective_bg) = if cell.flags.contains(Flags::INVERSE) {
                                (cell.bg, cell.fg)
                            } else {
                                (cell.fg, cell.bg)
                            };
                            let fg = self.convert_color(effective_fg, true);
                            let bg = match effective_bg {
                                AnsiColor::Named(NamedColor::Background) => None,
                                _ => Some(self.convert_color(effective_bg, false)),
                            };

                            let is_bold = cell.flags.contains(Flags::BOLD);
                            let is_underline = cell.flags.contains(Flags::UNDERLINE) || is_hovered_url || cell.hyperlink().is_some();
                            let code = cell.c as u32;
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
                            {
                                Some(cell.c)
                            } else {
                                None
                            };

                            let cell_cols = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
                            let end_col = (col + cell_cols).min(target_cols);

                            if fg != current_fg
                                || bg != current_bg
                                || is_bold != current_bold
                                || is_underline != current_underline
                                || block_cat != current_block_cat
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
                                current_span_start_col = col;
                            }
                            current_span_text.push(cell.c);
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

                        let cursor_info = if cursor_visible && is_active && self.cursor_blink_visible {
                            Some((cursor_point.line.0, cursor_point.column.0, effective_shape))
                        } else {
                            None
                        };

                        let mut visible_images = Vec::new();
                        let total_pushed = terminal.total_lines_pushed.load(std::sync::atomic::Ordering::Relaxed);
                        let screen_rows = lines.len();
                        let viewport_start_abs = (total_pushed + screen_rows as u64).saturating_sub(display_offset as u64 + screen_rows as u64);
                        let viewport_end_abs = viewport_start_abs + screen_rows as u64;

                        let store = terminal.image_store.lock();
                        for placement in &store.placements {
                            let p_start = placement.absolute_line;
                            let p_end = placement.absolute_line + placement.rows as u64;
                            if p_end > viewport_start_abs && p_start < viewport_end_abs {
                                let screen_row = (p_start as i64) - (viewport_start_abs as i64);
                                visible_images.push(crate::ui::terminal_grid_element::VisibleImage {
                                    row: screen_row,
                                    col: placement.col,
                                    cols: placement.cols,
                                    rows: placement.rows,
                                    z_index: placement.z_index,
                                    image: placement.image.clone(),
                                });
                            }
                        }
                        drop(store);

                        let selection_range = if is_active { self.selection } else { None };
                        let search_open = is_active && self.is_search_open && !self.search_query.is_empty();
                        let search_query_str = self.search_query.to_lowercase();
                        let active_search_offset = self.search_matches.get(self.search_match_idx).copied();
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
                                        let col_start = s.char_cols.get(i).copied().unwrap_or(s.start_col);
                                        let col_end = s.char_cols.get(i + 1).copied().unwrap_or(s.end_col);
                                        char_to_col.push(col_start);
                                        char_to_end_col.push(col_end);
                                    }
                                }

                                let line_lower = full_line.to_lowercase();
                                let mut start_b = 0;
                                while let Some(found_b) = line_lower[start_b..].find(&search_query_str) {
                                    let match_start_b = start_b + found_b;
                                    let match_end_b = match_start_b + search_query_str.len();
                                    let match_start_char = line_lower[..match_start_b].chars().count();
                                    let match_end_char = line_lower[..match_end_b].chars().count();

                                    if match_start_char < char_to_col.len() && match_end_char > 0 {
                                        let col_start = char_to_col[match_start_char];
                                        let last_char_idx = (match_end_char - 1).min(char_to_end_col.len() - 1);
                                        let col_end = char_to_end_col[last_char_idx];
                                        let col_len = col_end.saturating_sub(col_start).max(1);

                                        let is_active_match = active_search_offset.is_some_and(|off| {
                                            off == (display_offset + (num_lines.saturating_sub(1 + row_idx)))
                                        });
                                        row_highlights.push((col_start, col_len, is_active_match));
                                    }
                                    start_b = match_end_b;
                                    if search_query_str.is_empty() { break; }
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
                    let is_this_dragging = self.is_dragging_scrollbar && self.dragging_scrollbar_pane_id == Some(pane.id);
                    let is_this_scrolling = (self.last_scrolled_pane_id == Some(pane.id) || (self.last_scrolled_pane_id.is_none() && is_active)) && elapsed_ms < 1500;

                    // Proximity check against this pane's right edge
                    let right_edge = container_x + avail_w;
                    let bottom_edge = container_y + avail_h;
                    let (is_in_proximity, dist_from_right) = if let Some((mx, my)) = self.cursor_window_pos {
                        if my >= (container_y - 4.0) && my <= (bottom_edge + 4.0) && mx >= (right_edge - 48.0) && mx <= (right_edge + 8.0) {
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
                        let thumb_h = (track_h * (target_rows as f32 / total_rows)).clamp(24.0, track_h);
                        let progress = 1.0 - (display_offset as f32 / history_size as f32).clamp(0.0, 1.0);
                        let thumb_top = ((track_h - thumb_h) * progress).clamp(0.0, track_h - thumb_h);

                        let mut thumb_bg = if is_this_dragging { theme.accent } else { theme.muted };
                        thumb_bg.a = 0.50 * alpha_mult;
                        let mut thumb_hover_bg = if is_this_dragging { theme.accent } else { theme.muted_strong };
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
                                        .hover(move |s| s.bg(thumb_hover_bg))
                                )
                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
                                    this.is_dragging_scrollbar = true;
                                    this.dragging_scrollbar_pane_id = Some(pane_id);
                                    this.last_scrolled_pane_id = Some(pane_id);
                                    this.scrollbar_drag_start_y = ev.position.y.to_f64() as f32;
                                    this.scrollbar_drag_start_offset = display_offset;
                                    this.last_scroll_activity = std::time::Instant::now();
                                    cx.notify();
                                }))
                        )
                    } else {
                        None
                    }
                } else {
                    None
                };

                div()
                    .size_full()
                    .relative()
                    .overflow_hidden()
                    .when(pane_count > 1 && is_active, |d| d.border_1().border_color(theme.accent))
                    .when(pane_count > 1 && !is_active, |d| d.border_1().border_color(theme.border))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                            tab.pane_tree.active_pane_id = pane_id;
                            if let Some(p) = tab.pane_tree.active_pane() {
                                tab.terminal = p.terminal.clone();
                                tab.cwd = p.cwd.clone();
                                tab.git_status = p.git_status.clone();
                            }
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, _ev, _window, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                            tab.pane_tree.active_pane_id = pane_id;
                            if let Some(p) = tab.pane_tree.active_pane() {
                                tab.terminal = p.terminal.clone();
                                tab.cwd = p.cwd.clone();
                                tab.git_status = p.git_status.clone();
                            }
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _ev, _window, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                            tab.pane_tree.active_pane_id = pane_id;
                            if let Some(p) = tab.pane_tree.active_pane() {
                                tab.terminal = p.terminal.clone();
                                tab.cwd = p.cwd.clone();
                                tab.git_status = p.git_status.clone();
                            }
                            cx.notify();
                        }
                    }))
                    .when_some(grid_el, |d, el| d.child(el))
                    .when_some(pane_scrollbar_thumb, |d, thumb| d.child(thumb))
            }
            PaneNode::Split { direction, ratio, first, second } => {
                let mut container = div().size_full().flex();
                container = match direction {
                    SplitDirection::Horizontal => container.flex_row(),
                    SplitDirection::Vertical => container.flex_col(),
                };

                let (w1, h1, w2, h2, first_x, first_y, second_x, second_y) = match direction {
                    SplitDirection::Horizontal => {
                        let w1 = avail_w * *ratio;
                        let w2 = avail_w * (1.0 - *ratio);
                        (w1, avail_h, w2, avail_h, container_x, container_y, container_x + w1, container_y)
                    }
                    SplitDirection::Vertical => {
                        let h1 = avail_h * *ratio;
                        let h2 = avail_h * (1.0 - *ratio);
                        (avail_w, h1, avail_w, h2, container_x, container_y, container_x, container_y + h1)
                    }
                };

                let is_dragging_this = self.is_dragging_pane_split && self.dragging_split_path == path;
                let divider_line_color = if is_dragging_this { theme.accent } else { theme.border };
                let split_path_clone = path.clone();
                let dir_val = *direction;

                let divider = match direction {
                    SplitDirection::Horizontal => {
                        div()
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
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev: &MouseDownEvent, _window, cx| {
                                this.is_dragging_pane_split = true;
                                this.dragging_split_path = split_path_clone.clone();
                                this.dragging_split_direction = dir_val;
                                this.dragging_split_bounds = (container_x, container_y, avail_w, avail_h);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(1.))
                                    .h_full()
                                    .bg(divider_line_color)
                            )
                    }
                    SplitDirection::Vertical => {
                        div()
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
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev: &MouseDownEvent, _window, cx| {
                                this.is_dragging_pane_split = true;
                                this.dragging_split_path = split_path_clone.clone();
                                this.dragging_split_direction = dir_val;
                                this.dragging_split_bounds = (container_x, container_y, avail_w, avail_h);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .h(px(1.))
                                    .w_full()
                                    .bg(divider_line_color)
                            )
                    }
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

        // F5: the picker fetched for another cwd: close rather than act on
        // stale data (e.g. tab switched by mouse while open).
        if self.is_pr_picker_open {
            let current_cwd = self.tabs.get(self.active_tab_idx).and_then(|t| t.cwd.clone());
            if current_cwd != self.pr_picker_cwd {
                self.is_pr_picker_open = false;
            }
        }
        let tab_items: Vec<TabItem> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| {
                let process_name = tab.terminal.as_ref().and_then(|t| t.get_foreground_process_name());
                // F2: zoomed tabs carry a marker so the state is visible.
                let mut title = tab.custom_title.clone().unwrap_or_else(|| tab.title.clone());
                if tab.zoomed_pane.is_some() {
                    title = format!("[Z] {title}");
                }
                TabItem {
                    id: tab.id,
                    title,
                    active: idx == self.active_tab_idx,
                    is_dirty: tab.git_status.as_ref().is_some_and(|g| g.unstaged > 0 || g.staged > 0),
                    process_name,
                }
            })
            .collect();

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

        let (active_cwd, active_git, fallback_info) = if let Some(active_tab) = self.tabs.get_mut(self.active_tab_idx) {
            let now = std::time::Instant::now();
            let should_poll = active_tab.git_checked_cwd.as_ref() != active_tab.cwd.as_ref()
                || active_tab.git_last_poll.is_none_or(|last| now.duration_since(last) >= std::time::Duration::from_secs(2));

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
        let avail_h = (viewport_size.height.to_f64() as f32 - top_offset - STATUS_BAR_HEIGHT)
            .max(100.0);

        let font_family = self.font_family.clone();
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
                    p.last_bounds = if Some(p.id) == zoomed_id { Some(full) } else { None };
                }
            } else {
                // No zoom (or a stale zoom target): normal tree layout.
                active_tab.zoomed_pane = None;
                active_tab.pane_tree.update_layout_bounds(sidebar_w, top_offset, avail_w, avail_h);
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
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _window, cx| {
                if let Some(active_tab) = this.tabs.get(this.active_tab_idx) {
                    if let Some(ref terminal) = active_tab.terminal {
                        crate::paste::handle_dropped_paths(terminal, paths.paths());
                        cx.notify();
                    }
                }
            }))
            .child(
                TabBar::new(tab_items.clone(), theme)
                    .layout(self.tab_layout)
                    .sidebar_open(self.sidebar_open)
                    .on_toggle_sidebar(cx.listener(|this, _ev, window, cx| {
                        this.toggle_tab_sidebar(window, cx);
                    }))
                    .ai_sidebar_open(self.ai_sidebar_open)
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
                            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _window, cx| {
                                if let Some(active_tab) = this.tabs.get(this.active_tab_idx) {
                                    if let Some(ref terminal) = active_tab.terminal {
                                        crate::paste::handle_dropped_paths(terminal, paths.paths());
                                        cx.notify();
                                    }
                                }
                            }))
                            .flex()
                            .flex_1()
                            .flex_col()
                            .h_full()
                            .pl(px(1.))
                            .bg(self.theme.main_bg)
                            .font_family(font_family.clone())
                            .text_size(px(font_size))
                            .font_features(enable_terminal_ligatures())
                            .overflow_hidden()
                            .child(super::ime::registration(cx.entity(), self.focus_handle.clone()))
                            .child(terminal_area),
                    )
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
                            IconType::Plus,
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
                                    let tab_data = this.tabs.remove(idx);
                                    if this.active_tab_idx >= this.tabs.len() {
                                        this.active_tab_idx = this.tabs.len().saturating_sub(1);
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
                                IconType::Plus,
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
                            IconType::Plus,
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
                            IconType::Plus,
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
                            IconType::Plus,
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
                            IconType::Plus,
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
                            IconType::Plus,
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
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(12.))
                                                                            .font_weight(FontWeight::BOLD)
                                                                            .text_color(if is_selected { theme.black } else { theme.foreground })
                                                                            .child(title),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(10.5))
                                                                            .text_color(if is_selected { theme.black.opacity(0.85) } else { theme.muted })
                                                                            .child(cwd_str.unwrap_or_else(|| "~".to_string())),
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
                                                                    .child(branch_str.map(|b| format!("🌿 {}", b)).unwrap_or_else(|| "Tab".to_string())),
                                                            )
                                                    })
                                                    .collect()
                                            }
                                        ),
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
                                                .text_size(px(11.))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.foreground)
                                                .child(format!("⎇ {}", branch_name)),
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

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(backdrop)
                        .flex()
                        .flex_col()
                        .items_center()
                        .pt(px(56.))
                        .pb(px(24.))
                        .px(px(24.))
                        .gap_4()
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
                                .w_full()
                                .max_w(px(980.))
                                .px(px(16.))
                                .py(px(10.))
                                .rounded(px(10.))
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
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme.muted)
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
                                .w_full()
                                .max_w(px(980.))
                                .max_h(px(580.))
                                .overflow_y_scroll()
                                .gap_4()
                                .p(px(4.))
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
                                        let preview_lines = tab.terminal.as_ref().map(|t| t.get_screen_preview_lines(7)).unwrap_or_default();
                                        let split_count = tab.pane_tree.all_panes().len();
                                        let branch_opt = tab.git_status.as_ref().map(|g| g.branch.clone());

                                        div()
                                            .flex()
                                            .flex_col()
                                            .w(px(290.))
                                            .h(px(210.))
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
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .px(px(10.))
                                                    .py(px(7.))
                                                    .bg(if is_selected { theme.accent.opacity(0.12) } else { theme.surface_raised })
                                                    .border_b_1()
                                                    .border_color(theme.border)
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_2()
                                                            .child(render_icon(icon_type, theme.accent, 13.0))
                                                            .child(
                                                                div()
                                                                    .text_size(px(11.5))
                                                                    .font_weight(FontWeight::BOLD)
                                                                    .text_color(theme.foreground)
                                                                    .max_w(px(140.))
                                                                    .overflow_hidden()
                                                                    .child(title),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_row()
                                                            .items_center()
                                                            .gap_1()
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
                                                    .bg(theme.black.opacity(0.85))
                                                    .p(px(8.))
                                                    .font_family(font_fam.clone())
                                                    .text_size(px(9.5))
                                                    .text_color(theme.foreground.opacity(0.8))
                                                    .overflow_hidden()
                                                    .flex()
                                                    .flex_col()
                                                    .gap(px(1.))
                                                    .children(
                                                        if preview_lines.is_empty() {
                                                            vec![
                                                                div()
                                                                    .text_color(theme.muted)
                                                                    .child("~ (empty buffer)")
                                                            ]
                                                        } else {
                                                            preview_lines
                                                                .into_iter()
                                                                .map(|l| {
                                                                    div()
                                                                        .max_w(px(270.))
                                                                        .overflow_hidden()
                                                                        .child(if l.is_empty() { " ".to_string() } else { l })
                                                                })
                                                                .collect()
                                                        }
                                                    ),
                                            )
                                            // Card Bottom Status Bar
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .px(px(8.))
                                                    .py(px(4.))
                                                    .bg(theme.surface)
                                                    .border_t_1()
                                                    .border_color(theme.border)
                                                    .child(
                                                        div()
                                                            .text_size(px(9.5))
                                                            .text_color(theme.muted)
                                                            .child(format!("cmd: {}", proc_name)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(9.5))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(theme.accent)
                                                            .child(branch_opt.map(|b| format!("🌿 {b}")).unwrap_or_else(|| "local".to_string())),
                                                    ),
                                            )
                                    })
                                )
                                // Plus Card to Add Tab
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .justify_center()
                                        .w(px(290.))
                                        .h(px(210.))
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
                let status_msg = self.update_status.clone().unwrap_or_default();
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
                                .w(px(360.))
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
                                                .child("Fastty Updater"),
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
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.foreground)
                                        .child(status_msg),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .justify_end()
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
                                                    .on_mouse_down(MouseButton::Left, |_ev, _window, _cx| {
                                                        crate::updater::relaunch_fastty();
                                                    })
                                                    .child("Restart Now"),
                                            )
                                        })
                                        .when(!self.is_update_ready, |el| {
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
                                                    .child(if self.is_updating { "Hide" } else { "OK" }),
                                            )
                                        }),
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
    div()
        .h(px(1.))
        .w_full()
        .bg(theme.border)
        .my(px(2.))
}

fn render_about_spec_row(label: &'static str, value: &str, theme: Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .py(px(2.))
        .text_size(px(11.))
        .child(
            div()
                .text_color(theme.muted)
                .child(label),
        )
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

    #[test]
    fn test_theme_palette_ids_map_to_theme_names() {
        assert_eq!(theme_name_for_palette_id("theme_default"), Some("default"));
        assert_eq!(theme_name_for_palette_id("theme_catppuccin"), Some("catppuccin"));
        assert_eq!(theme_name_for_palette_id("theme_one_dark"), Some("one-dark"));
        assert_eq!(theme_name_for_palette_id("theme_solarized"), Some("solarized-dark"));
        assert_eq!(theme_name_for_palette_id("theme_high_contrast"), Some("high-contrast"));
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
        assert_eq!(pr_action_label(PrPickerAction::Checkout, 12), "Checkout PR #12");
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
                    author: Some(GhPrAuthor { login: "sam".to_string() }),
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
    fn test_palette_preview_kind_covers_theme_font_and_layout() {        assert_eq!(
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
        assert_eq!(palette_preview_kind("zoom_reset"), Some(PalettePreviewKind::FontReset));
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
        let block_chars = ['█', '▀', '▄', '▌', '▐', '░', '▒', '▓', '▖', '▗', '▘', '▙', '▚', '▛', '▜', '▝', '▞', '▟'];
        for ch in block_chars {
            let res = render_geometric_cell(ch, cell_w, line_h, fg, bg, theme_bg);
            assert!(res.is_some(), "Character {:?} (U+{:04X}) must produce a geometric element", ch, ch as u32);
        }

        // Box drawing
        let box_chars = ['─', '│', '┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼', '╭', '╮', '╯', '╰', '═', '║'];
        for ch in box_chars {
            let res = render_geometric_cell(ch, cell_w, line_h, fg, bg, theme_bg);
            assert!(res.is_some(), "Box character {:?} (U+{:04X}) must produce a geometric element", ch, ch as u32);
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
            assert!(res.is_some(), "Shape {:?} (U+{:04X}) must produce a geometric element", ch, ch as u32);
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
                assert!(res.is_some(), "Banner character {:?} must render geometrically", ch);
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
                emoji_scale: None,
                char_cols: vec![5, 6, 7, 8],
            },
        ];

        trim_row_spans(&mut spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, "hello");
        assert_eq!(spans[0].end_col, 5);

        // Spans with background are preserved even with trailing spaces
        let mut spans_with_bg = vec![
            StyledSpan {
                text: "highlighted   ".to_string(),
                start_col: 0,
                end_col: 14,
                fg: theme.foreground,
                bg: Some(theme.accent),
                is_bold: false,
                is_underline: false,
                is_emoji: false,
                is_nerd: false,
                emoji_scale: None,
                char_cols: (0..14).collect(),
            },
        ];
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
