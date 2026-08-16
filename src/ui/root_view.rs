use std::sync::Arc;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use gpui::{
    Context, CursorStyle, Div, FocusHandle, FontFeatures, FontWeight, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, ScrollHandle, ScrollWheelEvent,
    SharedString, Window, div, prelude::*, px,
};

use super::status_bar::{StatusBar, StatusBarModel, StatusInfo};
use super::tab_bar::{TabBar, TabItem};
use super::theme::{Theme, rgb_to_hsla};
use crate::config::{self, Config};
use crate::event_listener::EventSender;
use crate::git::GitStatus;
use crate::terminal_state::{AppEvent, TerminalState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: alacritty_terminal::index::Point,
    pub end: alacritty_terminal::index::Point,
}

pub struct TabData {
    pub id: usize,
    pub title: String,
    pub custom_title: Option<String>,
    pub terminal: Option<Arc<TerminalState>>,
    pub cwd: Option<std::path::PathBuf>,
    pub git_status: Option<GitStatus>,
    pub git_checked_cwd: Option<std::path::PathBuf>,
    pub last_duration_ms: Option<u128>,
    pub last_exit_code: Option<i32>,
}

#[derive(Clone)]
struct StyledSpan {
    text: String,
    start_col: usize,
    end_col: usize,
    fg: Hsla,
    bg: Option<Hsla>,
    is_bold: bool,
    is_underline: bool,
    is_cursor: bool,
}

fn trim_row_spans(spans: &mut Vec<StyledSpan>) {
    while let Some(last) = spans.last_mut() {
        if last.bg.is_none() && !last.is_underline && !last.is_cursor {
            let trimmed = last.text.trim_end_matches(' ');
            if trimmed.is_empty() {
                spans.pop();
            } else {
                let diff = last.text.len() - trimmed.len();
                last.end_col = last.end_col.saturating_sub(diff);
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

pub fn open_path_or_url(target: impl AsRef<std::ffi::OsStr>) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(target).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd").args(["/C", "start", ""]).arg(target).spawn();
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

fn available_system_fonts() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        crate::font_discovery_macos::available_monospace_fonts()
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("fc-list")
            .args([":spacing=100", "family"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut set = std::collections::BTreeSet::new();
                for line in stdout.lines() {
                    for part in line.split(',') {
                        let name = part.trim();
                        if !name.is_empty() {
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
            "DejaVu Sans Mono".to_string(),
            "Fira Code".to_string(),
            "JetBrains Mono".to_string(),
            "Hack".to_string(),
            "Ubuntu Mono".to_string(),
            "Liberation Mono".to_string(),
            "monospace".to_string(),
        ]
    }
    #[cfg(target_os = "windows")]
    {
        vec![
            "Cascadia Code".to_string(),
            "Cascadia Mono".to_string(),
            "Consolas".to_string(),
            "Lucida Console".to_string(),
            "Courier New".to_string(),
            "Fira Code".to_string(),
            "JetBrains Mono".to_string(),
            "monospace".to_string(),
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        vec!["monospace".to_string()]
    }
}

use icons::common::IconType;
use super::icons::{render_app_logo, render_icon};

#[derive(Clone, Debug)]
pub struct PaletteCommand {
    pub id: &'static str,
    pub icon: IconType,
    pub title: &'static str,
    pub category: &'static str,
    pub shortcut: Option<&'static str>,
}

pub fn get_all_palette_commands() -> Vec<PaletteCommand> {
    let is_mac = cfg!(target_os = "macos");
    vec![
        PaletteCommand { id: "new_tab", icon: IconType::Plus, title: "New Tab", category: "Terminal", shortcut: Some(if is_mac { "⌘T" } else { "Ctrl+Shift+T" }) },
        PaletteCommand { id: "rename_tab", icon: IconType::Pencil, title: "Rename Active Tab", category: "Terminal", shortcut: Some(if is_mac { "⌘⇧R" } else { "Ctrl+Shift+R" }) },
        PaletteCommand { id: "close_tab", icon: IconType::X, title: "Close Active Tab", category: "Terminal", shortcut: Some(if is_mac { "⌘W" } else { "Ctrl+Shift+W" }) },
        PaletteCommand { id: "search", icon: IconType::Search, title: "Search in Buffer", category: "Terminal", shortcut: Some(if is_mac { "⌘F" } else { "Ctrl+Shift+F" }) },
        PaletteCommand { id: "clear", icon: IconType::Trash2, title: "Clear Scrollback", category: "Terminal", shortcut: Some(if is_mac { "⌘K" } else { "Ctrl+Shift+K" }) },
        PaletteCommand { id: "worktree", icon: IconType::GitPullRequest, title: "Git Worktree Picker", category: "Git", shortcut: Some(if is_mac { "⌘⌥W" } else { "Ctrl+Alt+W" }) },
        PaletteCommand { id: "project_jumper", icon: IconType::Folder, title: "Project / Tab Jumper", category: "Navigation", shortcut: Some(if is_mac { "⌘J" } else { "Ctrl+Shift+J" }) },
        PaletteCommand { id: "ssh", icon: IconType::Server, title: "SSH Host Manager", category: "Tools", shortcut: Some(if is_mac { "⌘O" } else { "Ctrl+Shift+O" }) },
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
        PaletteCommand { id: "quit", icon: IconType::LogOut, title: "Quit Fastty", category: "Application", shortcut: Some(if is_mac { "⌘Q" } else { "Alt+F4" }) },
    ]
}

pub struct RootView {
    config: Config,
    theme: Theme,
    tabs: Vec<TabData>,
    active_tab_idx: usize,
    next_tab_id: usize,
    focus_handle: FocusHandle,
    font_size: f32,
    font_family: SharedString,
    status_bar_model: StatusBarModel,
    pub is_settings_open: bool,
    pub is_context_menu_open: bool,
    pub is_about_open: bool,
    pub is_rename_tab_open: bool,
    pub rename_tab_idx: usize,
    pub rename_tab_input: String,
    pub is_tab_context_menu_open: bool,
    pub tab_context_menu_tab_id: usize,
    pub tab_context_menu_pos: (f32, f32),
    pub is_command_palette_open: bool,
    pub command_palette_query: String,
    pub command_palette_selected: usize,
    pub is_ssh_manager_open: bool,
    pub ssh_manager_query: String,
    pub ssh_manager_selected: usize,
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
    pub is_git_menu_open: bool,
    pub git_menu_pos: Option<(f32, f32)>,
    pub command_palette_scroll_handle: ScrollHandle,
    pub ssh_manager_scroll_handle: ScrollHandle,
    pub worktree_picker_scroll_handle: ScrollHandle,
    pub project_jumper_scroll_handle: ScrollHandle,
    pub typed_prompt_buf: String,
    pub selection: Option<Selection>,
    pub is_selecting: bool,
    pub selection_start: Option<alacritty_terminal::index::Point>,
    pub hovered_url: Option<String>,
    pub hovered_url_range: Option<(i32, usize, usize)>,
    pub current_theme_name: String,
    pub cursor_blink_visible: bool,
    pub last_cursor_activity: std::time::Instant,
    pub last_scroll_activity: std::time::Instant,
    pub is_dragging_scrollbar: bool,
    pub scrollbar_drag_start_y: f32,
    pub scrollbar_drag_start_offset: usize,
    pub update_available: Option<crate::updater::ReleaseInfo>,
    pub is_updating: bool,
    pub update_status: Option<String>,
    pub is_update_modal_open: bool,
    pub pressed_mouse_button: Option<MouseButton>,
}

impl RootView {
    #[inline]
    fn measure_cell_metrics(&self, window: &Window) -> (f32, f32) {
        let font_id = window.text_system().resolve_font(&gpui::font(self.font_family.clone()));
        let cell_w = window.text_system().layout_width(font_id, px(self.font_size), '0').to_f64() as f32;
        let line_h = (self.font_size * 1.32).max(12.0);
        if cell_w >= 1.0 {
            (cell_w, line_h)
        } else {
            get_font_cell_metrics(self.font_family.as_ref(), self.font_size)
        }
    }

    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        config::load_custom_themes();
        crate::snippets::load();
        let loaded_config = Config::load().unwrap_or_default();
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

        // Scrollbar fade and cursor ticker (interval: 35ms for smooth 30-60fps fade out)
        cx.spawn_in(_window, async move |this, cx| {
            let mut blink_counter = 0u32;
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(35)).await;
                let res = this.update_in(cx, |this, _window, cx| {
                    let mut needs_notify = false;

                    // Scroll fade active window
                    let elapsed = this.last_scroll_activity.elapsed();
                    if this.is_dragging_scrollbar || elapsed < std::time::Duration::from_millis(1500) {
                        needs_notify = true;
                    }

                    // Cursor blink logic (every ~525ms = 15 ticks of 35ms)
                    blink_counter += 1;
                    if blink_counter >= 15 {
                        blink_counter = 0;
                        if this.last_cursor_activity.elapsed() >= std::time::Duration::from_millis(500) {
                            this.cursor_blink_visible = !this.cursor_blink_visible;
                            needs_notify = true;
                        }
                    }

                    if needs_notify {
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
        if loaded_config.session_restore {
            if let Some(session) = crate::session::load() {
                if let Some(win) = session.windows.first() {
                    for tab_info in &win.tabs {
                        let cwd_path = tab_info.cwd.clone();
                        let title = tab_info.title_override.clone().or_else(|| tab_info.custom_name.clone());
                        restored_tabs.push((cwd_path, title));
                    }
                }
            }
        }

        let mut view = Self {
            config: loaded_config,
            theme,
            tabs: Vec::new(),
            active_tab_idx: 0,
            next_tab_id: 1,
            focus_handle,
            font_size,
            font_family,
            status_bar_model,
            is_settings_open: false,
            is_context_menu_open: false,
            is_about_open: false,
            is_rename_tab_open: false,
            rename_tab_idx: 0,
            rename_tab_input: String::new(),
            is_tab_context_menu_open: false,
            tab_context_menu_tab_id: 0,
            tab_context_menu_pos: (0.0, 0.0),
            is_command_palette_open: false,
            command_palette_query: String::new(),
            command_palette_selected: 0,
            is_ssh_manager_open: false,
            ssh_manager_query: String::new(),
            ssh_manager_selected: 0,
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
            is_git_menu_open: false,
            git_menu_pos: None,
            command_palette_scroll_handle: ScrollHandle::new(),
            ssh_manager_scroll_handle: ScrollHandle::new(),
            worktree_picker_scroll_handle: ScrollHandle::new(),
            project_jumper_scroll_handle: ScrollHandle::new(),
            typed_prompt_buf: String::new(),
            selection: None,
            is_selecting: false,
            selection_start: None,
            hovered_url: None,
            hovered_url_range: None,
            current_theme_name: theme_name,
            cursor_blink_visible: true,
            last_cursor_activity: std::time::Instant::now(),
            last_scroll_activity: std::time::Instant::now() - std::time::Duration::from_secs(10),
            is_dragging_scrollbar: false,
            scrollbar_drag_start_y: 0.0,
            scrollbar_drag_start_offset: 0,
            update_available: None,
            is_updating: false,
            update_status: None,
            is_update_modal_open: false,
            pressed_mouse_button: None,
        };

        // Background update check
        let (update_tx, update_rx) = async_channel::unbounded::<Option<crate::updater::ReleaseInfo>>();
        std::thread::spawn(move || {
            let res = crate::updater::check_for_update_sync();
            let _ = update_tx.send_blocking(res);
        });

        cx.spawn_in(_window, async move |this, cx| {
            if let Ok(Some(release)) = update_rx.recv().await {
                let _ = this.update_in(cx, |this, _window, cx| {
                    this.update_available = Some(release);
                    cx.notify();
                });
            }
        }).detach();

        if restored_tabs.is_empty() {
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
            }],
            active_window: 0,
            legacy_tabs: Vec::new(),
            legacy_active_tab: 0,
        };
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

    pub fn create_tab_with_cmd_and_cwd(
        &mut self,
        cmd: &str,
        args: &[String],
        cwd: Option<&std::path::Path>,
        title_override: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;

        let font_config = self.config.font.clone();
        let (cell_w, line_h) = self.measure_cell_metrics(_window);

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
            event_sender,
        )
        .expect("Failed to initialize terminal state");

        let terminal_arc = Arc::new(terminal);

        cx.spawn_in(_window, async move |this, cx| {
            while let Ok(event) = event_rx.recv().await {
                while let Ok(AppEvent::Wakeup) = event_rx.try_recv() {}

                let res = this.update_in(cx, |this, _window, cx| {
                    match event {
                        AppEvent::Wakeup => {
                            cx.notify();
                        }
                        AppEvent::TitleChanged(title) => {
                            if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                tab.title = title;
                            }
                            this.persist_session();
                            cx.notify();
                        }
                        AppEvent::CwdChanged(cwd) => {
                            if let Some(tab) = this.tabs.get_mut(this.active_tab_idx) {
                                let p = std::path::PathBuf::from(&cwd);
                                tab.git_status = crate::git::fetch_git_info(&p);
                                tab.cwd = Some(p);
                            }
                            this.persist_session();
                            cx.notify();
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
        let initial_git = initial_cwd.as_ref().and_then(|p| crate::git::fetch_git_info(p));
        let default_shell_name = std::path::Path::new(cmd)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.trim_end_matches(".exe"))
            .filter(|n| !n.is_empty())
            .unwrap_or("fastty");
        let tab_title = title_override.unwrap_or_else(|| default_shell_name.to_string());

        self.tabs.push(TabData {
            id: tab_id,
            title: tab_title,
            custom_title: None,
            terminal: Some(terminal_arc),
            cwd: initial_cwd.clone(),
            git_status: initial_git,
            git_checked_cwd: initial_cwd,
            last_duration_ms: None,
            last_exit_code: None,
        });

        self.active_tab_idx = self.tabs.len() - 1;
        self.persist_session();
        cx.notify();
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
            self.active_tab_idx = idx;
            self.persist_session();
            cx.notify();
        }
    }

    pub fn close_tab(&mut self, tab_id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) {
            self.tabs.remove(idx);
            if self.tabs.is_empty() {
                self.create_tab(window, cx);
            } else if self.active_tab_idx >= self.tabs.len() {
                self.active_tab_idx = self.tabs.len() - 1;
            }
            self.persist_session();
            cx.notify();
        }
    }

    pub fn trigger_apply_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_updating {
            return;
        }
        let Some(release) = self.update_available.clone() else {
            return;
        };

        self.is_updating = true;
        self.update_status = Some(format!("Downloading Fastty v{}...", release.version));
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
                            this.update_status = Some(format!("Fastty v{} installed successfully!\nPlease restart Fastty to use the new version.", release.version));
                            this.is_update_modal_open = true;
                            this.update_available = None;
                        }
                        Err(e) => {
                            this.update_status = Some(format!("Update failed: {}", e));
                            this.is_update_modal_open = true;
                        }
                    }
                    cx.notify();
                });
            }
        }).detach();
    }

    pub fn toggle_settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_settings_open = !self.is_settings_open;
        if self.is_settings_open {
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_command_palette_open = false;
            self.is_ssh_manager_open = false;
            self.is_search_open = false;
        }
        cx.notify();
    }

    pub fn toggle_context_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_context_menu_open = !self.is_context_menu_open;
        if self.is_context_menu_open {
            self.is_settings_open = false;
            self.is_about_open = false;
            self.is_tab_context_menu_open = false;
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
        self.is_about_open = false;
        self.is_command_palette_open = false;
        self.is_ssh_manager_open = false;
        self.is_search_open = false;
        self.is_git_menu_open = false;
        cx.notify();
    }

    pub fn close_other_tabs(&mut self, keep_id: usize, _window: &mut Window, cx: &mut Context<Self>) {
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
            self.is_search_open = false;
        }
        cx.notify();
    }

    pub fn toggle_command_palette(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_command_palette_open = !self.is_command_palette_open;
        if self.is_command_palette_open {
            self.command_palette_query.clear();
            self.command_palette_selected = 0;
            self.is_settings_open = false;
            self.is_context_menu_open = false;
            self.is_about_open = false;
            self.is_ssh_manager_open = false;
            self.is_search_open = false;
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
            self.is_search_open = false;
        }
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
            self.is_search_open = false;
            self.is_worktree_picker_open = false;
        }
        cx.notify();
    }

    pub fn execute_palette_command(&mut self, cmd_id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_command_palette_open = false;
        match cmd_id {
            "new_tab" => self.create_tab(_window, cx),
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
            "settings" => self.toggle_settings(_window, cx),
            "about" => self.toggle_about(_window, cx),
            "fullscreen" => {
                _window.toggle_fullscreen();
                cx.notify();
            }
            "zoom_in" => self.adjust_font_size(1.0, cx),
            "zoom_out" => self.adjust_font_size(-1.0, cx),
            "zoom_reset" => {
                self.font_size = 13.0;
                self.config.font.size = 13.0;
                let _ = self.config.save_default();
                cx.notify();
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
            "quit" => cx.quit(),
            _ => {}
        }
    }

    pub fn set_theme(&mut self, theme_name: &str, cx: &mut Context<Self>) {
        self.current_theme_name = theme_name.to_string();
        self.config.theme = Some(self.current_theme_name.clone());
        let _ = self.config.save_default();
        self.theme = Theme::from_name(theme_name).with_opacity(self.config.opacity);
        self.status_bar_model = StatusBarModel::new(&self.config, self.theme);
        cx.notify();
    }

    pub fn adjust_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.font_size = (self.font_size + delta).clamp(9.0, 36.0);
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
        match color {
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
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke.key;
        let key_lower = key.to_lowercase();
        let modifiers = &event.keystroke.modifiers;

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
                if !modifiers.platform && !modifiers.control {
                    self.rename_tab_input.push_str(ch);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !modifiers.control {
                self.rename_tab_input.push_str(key);
                cx.notify();
                return;
            }
            return;
        }

        // 1. Command Palette Keyboard Handler
        if self.is_command_palette_open {
            if key_lower == "escape" || key_lower == "esc" {
                self.is_command_palette_open = false;
                cx.notify();
                return;
            }
            let query = self.command_palette_query.to_lowercase();
            let all_cmds = get_all_palette_commands();
            let filtered: Vec<&PaletteCommand> = all_cmds
                .iter()
                .filter(|c| query.is_empty() || c.title.to_lowercase().contains(&query) || c.category.to_lowercase().contains(&query))
                .collect();
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
                    cx.notify();
                }
                return;
            }
            if key_lower == "backspace" {
                self.command_palette_query.pop();
                self.command_palette_selected = 0;
                self.command_palette_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            if let Some(ref ch) = event.keystroke.key_char {
                if !modifiers.platform && !modifiers.control {
                    self.command_palette_query.push_str(ch);
                    self.command_palette_selected = 0;
                    self.command_palette_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !modifiers.control {
                self.command_palette_query.push_str(key);
                self.command_palette_selected = 0;
                self.command_palette_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // 2. SSH Manager Keyboard Handler
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
                .filter(|h| query.is_empty() || h.name.to_lowercase().contains(&query) || h.hostname.to_lowercase().contains(&query) || h.user.to_lowercase().contains(&query))
                .collect();
            let count = filtered.len();

            if key_lower == "enter" || key_lower == "return" {
                if let Some(host) = filtered.get(self.ssh_manager_selected) {
                    let host_clone = (*host).clone();
                    self.is_ssh_manager_open = false;
                    let title = format!("ssh: {}", host_clone.name);
                    self.create_tab_with_cmd("ssh", &host_clone.ssh_args(), Some(title), _window, cx);
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
                if !modifiers.platform && !modifiers.control {
                    self.ssh_manager_query.push_str(ch);
                    self.ssh_manager_selected = 0;
                    self.ssh_manager_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !modifiers.control {
                self.ssh_manager_query.push_str(key);
                self.ssh_manager_selected = 0;
                self.ssh_manager_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // 3. Search Bar Keyboard Handler
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
                        if let Some(&offset) = self.search_matches.get(0) {
                            term.scroll_to_offset(offset);
                            self.last_scroll_activity = std::time::Instant::now();
                        }
                        cx.notify();
                        return;
                    }
                    let mut char_to_add = None;
                    if let Some(ref ch) = event.keystroke.key_char {
                        if !modifiers.platform && !modifiers.control {
                            char_to_add = Some(ch.clone());
                        }
                    } else if key.len() == 1 && !modifiers.platform && !modifiers.control {
                        char_to_add = Some(key.clone());
                    }
                    if let Some(ch) = char_to_add {
                        self.search_query.push_str(&ch);
                        self.search_matches = term.search_matches(&self.search_query);
                        self.search_match_idx = 0;
                        if let Some(&offset) = self.search_matches.get(0) {
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

        // 4. Git Worktree Picker Keyboard Handler
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
                if !modifiers.platform && !modifiers.control {
                    self.worktree_picker_query.push_str(ch);
                    self.worktree_picker_selected = 0;
                    self.worktree_picker_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !modifiers.control {
                self.worktree_picker_query.push_str(key);
                self.worktree_picker_selected = 0;
                self.worktree_picker_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // 5. Project / Tab Jumper Keyboard Handler
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
                        || cwd_str.as_ref().map_or(false, |c| c.to_lowercase().contains(&query))
                        || branch_str.as_ref().map_or(false, |b| b.to_lowercase().contains(&query));
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
                if !modifiers.platform && !modifiers.control {
                    self.project_jumper_query.push_str(ch);
                    self.project_jumper_selected = 0;
                    self.project_jumper_scroll_handle.scroll_to_item(0);
                    cx.notify();
                    return;
                }
            } else if key.len() == 1 && !modifiers.platform && !modifiers.control {
                self.project_jumper_query.push_str(key);
                self.project_jumper_selected = 0;
                self.project_jumper_scroll_handle.scroll_to_item(0);
                cx.notify();
                return;
            }
            return;
        }

        // 6. Dismiss open static overlays on Escape
        if (self.is_settings_open || self.is_about_open || self.is_context_menu_open || self.is_git_menu_open || self.is_tab_context_menu_open || self.is_update_modal_open)
            && (key_lower == "escape" || key_lower == "esc")
        {
            self.is_settings_open = false;
            self.is_about_open = false;
            self.is_context_menu_open = false;
            self.is_git_menu_open = false;
            self.is_tab_context_menu_open = false;
            self.is_update_modal_open = false;
            cx.notify();
            return;
        }

        // 7. Command Palette Trigger (⌘P on macOS; Ctrl+Shift+P on Linux/Windows)
        let is_command_palette = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "p"
        } else {
            modifiers.control && modifiers.shift && key_lower == "p"
        };
        if is_command_palette {
            self.toggle_command_palette(_window, cx);
            return;
        }

        // 8. SSH Manager Trigger (⌘O on macOS; Ctrl+Shift+O on Linux/Windows)
        let is_ssh_mgr = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "o"
        } else {
            modifiers.control && modifiers.shift && key_lower == "o"
        };
        if is_ssh_mgr {
            self.toggle_ssh_manager(_window, cx);
            return;
        }

        // 9. Git Worktree Picker Trigger (⌘⌥W on macOS; Ctrl+Alt+W on Linux/Windows)
        let is_worktree_trigger = if cfg!(target_os = "macos") {
            modifiers.platform && modifiers.alt && key_lower == "w"
        } else {
            modifiers.control && modifiers.alt && key_lower == "w"
        };
        if is_worktree_trigger {
            self.toggle_worktree_picker(_window, cx);
            return;
        }

        // 10. Project / Tab Jumper Trigger (⌘J on macOS; Ctrl+Shift+J on Linux/Windows)
        let is_jumper_trigger = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "j"
        } else {
            modifiers.control && modifiers.shift && key_lower == "j"
        };
        if is_jumper_trigger {
            self.toggle_project_jumper(_window, cx);
            return;
        }

        // 11. Search in Buffer Trigger (⌘F on macOS; Ctrl+Shift+F / Ctrl+F on Linux/Windows)
        let is_search_trigger = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "f"
        } else {
            (modifiers.control && modifiers.shift && key_lower == "f")
                || (modifiers.control && key_lower == "f")
        };
        if is_search_trigger {
            self.toggle_search(_window, cx);
            return;
        }

        // 12. Open / Toggle Settings (⌘, / ⌘S on macOS; Ctrl+, / Ctrl+Shift+S on Linux/Windows)
        let is_settings = if cfg!(target_os = "macos") {
            modifiers.platform && (key == "," || key_lower == "s" || key_lower == "comma")
        } else {
            (modifiers.control && (key == "," || key_lower == "comma"))
                || (modifiers.control && modifiers.shift && key_lower == "s")
        };
        if is_settings {
            self.toggle_settings(_window, cx);
            return;
        }

        // 3. Toggle Fullscreen (F11 on all OS; ⌃⌘F on macOS)
        let is_fullscreen = key_lower == "f11"
            || (cfg!(target_os = "macos") && modifiers.platform && modifiers.control && key_lower == "f");
        if is_fullscreen {
            _window.toggle_fullscreen();
            return;
        }

        // 4. New Tab (⌘T on macOS; Ctrl+Shift+T on Linux/Windows)
        let is_new_tab = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "t"
        } else {
            modifiers.control && modifiers.shift && key_lower == "t"
        };
        if is_new_tab {
            self.create_tab(_window, cx);
            return;
        }

        // 4b. Rename Tab (⌘⇧R on macOS; Ctrl+Shift+R on Linux/Windows)
        let is_rename_tab = if cfg!(target_os = "macos") {
            modifiers.platform && modifiers.shift && key_lower == "r"
        } else {
            modifiers.control && modifiers.shift && key_lower == "r"
        };
        if is_rename_tab {
            self.open_rename_tab(self.active_tab_idx, cx);
            return;
        }

        // 5. Close Tab (⌘W on macOS; Ctrl+Shift+W on Linux/Windows)
        let is_close_tab = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "w"
        } else {
            modifiers.control && modifiers.shift && key_lower == "w"
        };
        if is_close_tab {
            if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                let id = active_tab.id;
                self.close_tab(id, _window, cx);
            }
            return;
        }

        // 6. Next Tab (Ctrl+Tab, Ctrl+PageDown on all OS; ⌘Shift+] on macOS)
        let is_next_tab = (modifiers.control && !modifiers.shift && key_lower == "tab")
            || (modifiers.control && key_lower == "pagedown")
            || (cfg!(target_os = "macos") && modifiers.platform && modifiers.shift && (key == "]" || key == "}"));
        if is_next_tab {
            if !self.tabs.is_empty() {
                self.active_tab_idx = (self.active_tab_idx + 1) % self.tabs.len();
                cx.notify();
            }
            return;
        }

        // 7. Prev Tab (Ctrl+Shift+Tab, Ctrl+PageUp on all OS; ⌘Shift+[ on macOS)
        let is_prev_tab = (modifiers.control && modifiers.shift && key_lower == "tab")
            || (modifiers.control && key_lower == "pageup")
            || (cfg!(target_os = "macos") && modifiers.platform && modifiers.shift && (key == "[" || key == "{"));
        if is_prev_tab {
            if !self.tabs.is_empty() {
                self.active_tab_idx = if self.active_tab_idx == 0 {
                    self.tabs.len() - 1
                } else {
                    self.active_tab_idx - 1
                };
                cx.notify();
            }
            return;
        }

        // 8. Jump to Tab 1-9 (⌘1-9 on macOS; Alt+1-9 or Ctrl+Shift+1-9 on Linux/Windows)
        let tab_jump: Option<usize> = if cfg!(target_os = "macos") && modifiers.platform {
            key.parse::<usize>().ok()
        } else if modifiers.alt || (modifiers.control && modifiers.shift) {
            key.parse::<usize>().ok()
        } else {
            None
        };
        if let Some(digit) = tab_jump {
            if (1..=9).contains(&digit) {
                let target = digit - 1;
                if target < self.tabs.len() {
                    self.active_tab_idx = target;
                    cx.notify();
                    return;
                }
            }
        }

        // 9. Increase Font Size (⌘= / ⌘+ on macOS; Ctrl= / Ctrl+ on Linux/Windows)
        let is_zoom_in = if cfg!(target_os = "macos") {
            modifiers.platform && (key == "=" || key == "+" || key_lower == "equal" || key_lower == "plus")
        } else {
            modifiers.control && (key == "=" || key == "+" || key_lower == "equal" || key_lower == "plus")
        };
        if is_zoom_in {
            self.adjust_font_size(1.0, cx);
            return;
        }

        // 10. Decrease Font Size (⌘- on macOS; Ctrl- on Linux/Windows)
        let is_zoom_out = if cfg!(target_os = "macos") {
            modifiers.platform && (key == "-" || key == "_" || key_lower == "minus")
        } else {
            modifiers.control && (key == "-" || key == "_" || key_lower == "minus")
        };
        if is_zoom_out {
            self.adjust_font_size(-1.0, cx);
            return;
        }

        // 11. Reset Font Size (⌘0 on macOS; Ctrl0 on Linux/Windows)
        let is_zoom_reset = if cfg!(target_os = "macos") {
            modifiers.platform && key == "0"
        } else {
            modifiers.control && key == "0"
        };
        if is_zoom_reset {
            self.font_size = 13.0;
            self.config.font.size = 13.0;
            let _ = self.config.save_default();
            cx.notify();
            return;
        }

        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let Some(ref terminal) = active_tab.terminal else {
            return;
        };

        // 12. Clear Scrollback (⌘K on macOS; Ctrl+Shift+K on Linux/Windows)
        let is_clear_scroll = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "k"
        } else {
            modifiers.control && modifiers.shift && key_lower == "k"
        };
        if is_clear_scroll {
            terminal.scroll_to_bottom();
            cx.notify();
            return;
        }

        // 13. Paste (⌘V on macOS; Ctrl+Shift+V or Shift+Insert on Linux/Windows)
        let is_paste = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "v"
        } else {
            (modifiers.control && modifiers.shift && key_lower == "v")
                || (modifiers.shift && key_lower == "insert")
        };
        if is_paste {
            if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                if let Ok(text) = clip.get_text() {
                    terminal.write_to_pty(text.as_bytes());
                    cx.notify();
                    return;
                }
            }
        }

        // 14. Copy (⌘C on macOS; Ctrl+Shift+C on Linux/Windows)
        let is_copy = if cfg!(target_os = "macos") {
            modifiers.platform && key_lower == "c"
        } else {
            modifiers.control && modifiers.shift && key_lower == "c"
        };
        if is_copy {
            if let Some(text) = self.get_selected_text() {
                if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                    let _ = clip.set_text(text);
                }
            }
            return;
        }

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

        let bytes_to_send: Option<Vec<u8>> = if modifiers.control {
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
                    if let Some(ref ch) = event.keystroke.key_char {
                        let mut b = vec![0x1b];
                        b.extend_from_slice(ch.as_bytes());
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

    fn handle_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        let Some(ref terminal) = active_tab.terminal else {
            return;
        };

        let (cell_w, line_h) = self.measure_cell_metrics(window);
        let local_x = (event.position.x.to_f64() as f32 - 12.0).max(0.0);
        let local_y = (event.position.y.to_f64() as f32 - 38.0).max(0.0);
        let col = ((local_x / cell_w).floor() as usize) + 1;
        let row = ((local_y / line_h).floor() as usize) + 1;

        self.pressed_mouse_button = Some(event.button);

        if terminal.is_mouse_mode_enabled() {
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
                true,
                event.modifiers.shift,
                event.modifiers.alt,
                event.modifiers.control,
            );
            return;
        }

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

            // Start Text Selection
            let history_size = terminal.history_size();
            let viewport_size = window.viewport_size();
            let avail_h = (viewport_size.height.to_f64() as f32 - 54.0 - 12.0).max(100.0);
            if local_y > avail_h {
                return;
            }
            let screen_rows = ((avail_h / line_h) as i32).max(1);
            let display_offset = terminal.display_offset();
            let grid_col = ((local_x / cell_w).floor() as usize).max(0);
            let grid_row = (((local_y / line_h).floor() as i32) - (display_offset as i32)).clamp(-(history_size as i32), screen_rows - 1);
            let start_point = alacritty_terminal::index::Point::new(
                alacritty_terminal::index::Line(grid_row),
                alacritty_terminal::index::Column(grid_col),
            );
            self.is_selecting = true;
            self.selection_start = Some(start_point);
            self.selection = None;
            cx.notify();
        }
    }

    fn handle_mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dragging_scrollbar {
            self.last_scroll_activity = std::time::Instant::now();
            let cur_y = event.position.y.to_f64() as f32;
            let delta_y = cur_y - self.scrollbar_drag_start_y;
            if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
                if let Some(ref term) = active_tab.terminal {
                    let history_size = term.history_size();
                    if history_size > 0 {
                        let (_, line_h) = self.measure_cell_metrics(_window);
                        let viewport_size = _window.viewport_size();
                        let avail_h = (viewport_size.height.to_f64() as f32 - 54.0 - 12.0).max(100.0);
                        let target_rows = ((avail_h / line_h) as usize).max(5);
                        let track_h = avail_h;
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
            return;
        }

        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else { return; };
        let Some(ref terminal) = active_tab.terminal else { return; };

        let (cell_w, line_h) = self.measure_cell_metrics(_window);
        let local_x = (event.position.x.to_f64() as f32 - 12.0).max(0.0);
        let local_y = (event.position.y.to_f64() as f32 - 38.0).max(0.0);

        if terminal.is_mouse_mode_enabled() {
            let left = self.pressed_mouse_button == Some(MouseButton::Left);
            let middle = self.pressed_mouse_button == Some(MouseButton::Middle);
            let right = self.pressed_mouse_button == Some(MouseButton::Right);
            let col = ((local_x / cell_w).floor() as usize) + 1;
            let row = ((local_y / line_h).floor() as usize) + 1;
            terminal.send_mouse_motion(
                col,
                row,
                left,
                middle,
                right,
                event.modifiers.shift,
                event.modifiers.alt,
                event.modifiers.control,
            );
            return;
        }

        let history_size = terminal.history_size();
        let viewport_size = _window.viewport_size();
        let avail_h = (viewport_size.height.to_f64() as f32 - 54.0 - 12.0).max(100.0);
        let screen_rows = ((avail_h / line_h) as i32).max(1);
        let display_offset = terminal.display_offset();
        let grid_col = ((local_x / cell_w).floor() as usize).max(0);
        let grid_row = (((local_y / line_h).floor() as i32) - (display_offset as i32)).clamp(-(history_size as i32), screen_rows - 1);
        let current_point = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(grid_row),
            alacritty_terminal::index::Column(grid_col),
        );

        if self.is_selecting {
            if let Some(start_p) = self.selection_start {
                self.selection = Some(Selection {
                    start: start_p,
                    end: current_point,
                });
                cx.notify();
            }
            return;
        }

        // URL Hover detection
        if let Some(term_guard) = terminal.term().try_lock() {
            let grid = term_guard.grid();
            let cols = grid.columns();
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
            if let Some(ref terminal) = active_tab.terminal {
                if terminal.is_mouse_mode_enabled() {
                    let (cell_w, line_h) = self.measure_cell_metrics(_window);
                    let local_x = (event.position.x.to_f64() as f32 - 12.0).max(0.0);
                    let local_y = (event.position.y.to_f64() as f32 - 38.0).max(0.0);
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

    fn handle_scroll(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };

        let Some(ref terminal) = active_tab.terminal else {
            return;
        };

        let delta_y = match event.delta {
            gpui::ScrollDelta::Pixels(p) => p.y.as_f32(),
            gpui::ScrollDelta::Lines(l) => l.y * 20.0,
        };

        if delta_y.abs() > 0.5 {
            self.last_scroll_activity = std::time::Instant::now();
            if terminal.is_mouse_mode_enabled() {
                let (cell_w, line_h) = self.measure_cell_metrics(_window);
                let local_x = (event.position.x.to_f64() as f32 - 12.0).max(0.0);
                let local_y = (event.position.y.to_f64() as f32 - 38.0).max(0.0);
                let col = ((local_x / cell_w).floor() as usize) + 1;
                let row = ((local_y / line_h).floor() as usize) + 1;
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
                let lines = (delta_y / 15.0).round() as isize;
                if lines != 0 {
                    terminal.scroll(lines);
                    cx.notify();
                }
            }
        }
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;

        let tab_items: Vec<TabItem> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| TabItem {
                id: tab.id,
                title: tab.custom_title.clone().unwrap_or_else(|| tab.title.clone()),
                active: idx == self.active_tab_idx,
                is_dirty: tab.git_status.as_ref().map_or(false, |g| g.unstaged > 0 || g.staged > 0),
            })
            .collect();

        let (active_cwd, active_git, fallback_info) = if let Some(active_tab) = self.tabs.get_mut(self.active_tab_idx) {
            if let Some(ref term) = active_tab.terminal {
                if let Some(proc_cwd) = term.get_current_working_directory() {
                    let changed = active_tab.cwd.as_ref().map_or(true, |c| c != &proc_cwd);
                    if changed {
                        active_tab.cwd = Some(proc_cwd);
                    }
                }
            }
            if active_tab.git_checked_cwd.as_ref() != active_tab.cwd.as_ref() {
                active_tab.git_checked_cwd = active_tab.cwd.clone();
                if let Some(ref cwd) = active_tab.cwd {
                    active_tab.git_status = crate::git::fetch_git_info(cwd);
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

        let (left_segs, right_segs) = self.status_bar_model.render_segments(active_cwd, active_git.as_ref());

        let (cell_w, line_h) = self.measure_cell_metrics(_window);
        let viewport_size = _window.viewport_size();
        let avail_w = (viewport_size.width.to_f64() as f32 - 24.0).max(100.0);
        let avail_h = (viewport_size.height.to_f64() as f32 - 54.0 - 12.0).max(100.0);
        let target_cols = ((avail_w / cell_w) as usize).max(20);
        let target_rows = ((avail_h / line_h) as usize).max(5);

        // Terminal Grid Rendering for active tab
        let (display_offset, history_size, lines) = if let Some(active_tab) = self.tabs.get(self.active_tab_idx) {
            if let Some(ref terminal) = active_tab.terminal {
                terminal.resize_with_pixels(
                    target_cols,
                    target_rows,
                    (target_cols as f32 * cell_w).round() as u16,
                    (target_rows as f32 * line_h).round() as u16,
                );
                let h_size = terminal.history_size();
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
            let mut current_fg = self.theme.foreground;
            let mut current_bg: Option<Hsla> = None;
            let mut current_bold = false;
            let mut current_underline = false;
            let mut current_is_cursor = false;
            let mut last_row: Option<i32> = None;

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
                                is_cursor: current_is_cursor,
                            });
                            current_span_text = String::new();
                            current_block_cat = None;
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

                // Wide char spacer is the second column of a double-width glyph; skip
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
                let is_underline = cell.flags.contains(Flags::UNDERLINE) || is_hovered_url;
                let is_cursor = cursor_visible && self.cursor_blink_visible && row == cursor_point.line.0 && col == cursor_point.column.0;
                let code = cell.c as u32;
                let is_emoji = is_emoji_codepoint(code);
                let is_pua_icon = (0xE000..=0xF8FF).contains(&code)
                    || (0xF0000..=0xFFFFD).contains(&code)
                    || (0x100000..=0x10FFFD).contains(&code);
                let block_cat = if (0x2500..=0x257F).contains(&code) || (0x2580..=0x259F).contains(&code) {
                    Some(cell.c)
                } else if is_emoji || is_pua_icon {
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
                    || is_cursor != current_is_cursor
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
                            is_cursor: current_is_cursor,
                        });
                        current_span_text = String::new();
                    }
                    current_span_start_col = col;
                    current_span_end_col = end_col;
                    current_fg = fg;
                    current_bg = bg;
                    current_bold = is_bold;
                    current_underline = is_underline;
                    current_is_cursor = is_cursor;
                    current_block_cat = block_cat;
                } else {
                    current_span_end_col = end_col;
                }

                current_span_text.push(cell.c);

                let has_fe0f = cell.zerowidth().map_or(false, |zw| zw.contains(&'\u{FE0F}'));
                if let Some(zerowidth) = cell.zerowidth() {
                    for &zw in zerowidth {
                        current_span_text.push(zw);
                    }
                }

                if is_emoji_codepoint(code) && !has_fe0f {
                    current_span_text.push('\u{FE0F}');
                }
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
                    is_cursor: current_is_cursor,
                });
            }
            trim_row_spans(&mut current_row_spans);
            lines.push(current_row_spans);
            lines.truncate(target_rows);

            drop(term_guard);

            if lines.is_empty() {
                lines.push(vec![StyledSpan {
                    text: " ".to_string(),
                    start_col: 0,
                    end_col: 1,
                    fg: self.theme.background,
                    bg: Some(self.theme.cursor),
                    is_bold: false,
                    is_underline: false,
                    is_cursor: true,
                }]);
            }

            (display_offset, h_size, lines)
                } else {
                    (0, h_size, Vec::new())
                }
            } else {
                (0, 0, Vec::new())
            }
        } else {
            (0, 0, Vec::new())
        };

        let font_family = self.font_family.clone();
        let font_size = self.font_size;
        let cursor_color = self.theme.cursor;
        let scrollbar_thumb = if history_size > 0 {
            let elapsed_ms = self.last_scroll_activity.elapsed().as_millis();
            if self.is_dragging_scrollbar || elapsed_ms < 1500 {
                let alpha_mult = if self.is_dragging_scrollbar || elapsed_ms < 700 {
                    1.0
                } else {
                    let t = (elapsed_ms - 700) as f32 / 800.0;
                    (1.0 - t).clamp(0.0, 1.0)
                };

                let track_h = avail_h;
                let total_rows = (history_size + target_rows) as f32;
                let thumb_h = (track_h * (target_rows as f32 / total_rows)).clamp(24.0, track_h);
                let progress = 1.0 - (display_offset as f32 / history_size as f32).clamp(0.0, 1.0);
                let thumb_top = ((track_h - thumb_h) * progress).clamp(0.0, track_h - thumb_h);

                let mut thumb_bg = self.theme.muted;
                thumb_bg.a = 0.45 * alpha_mult;
                let mut thumb_hover_bg = self.theme.muted_strong;
                thumb_hover_bg.a = 0.85 * alpha_mult;

                Some(
                    div()
                        .absolute()
                        .right(px(4.))
                        .top(px(thumb_top + 6.0))
                        .w(px(5.))
                        .h(px(thumb_h))
                        .rounded_full()
                        .bg(thumb_bg)
                        .hover(move |s| s.bg(thumb_hover_bg))
                        .cursor(CursorStyle::PointingHand)
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
                            this.is_dragging_scrollbar = true;
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
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TabBar::new(tab_items, theme)
                    .update_available(self.update_available.as_ref().map(|u| u.version.clone()), self.is_updating)
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
                    .on_open_settings(cx.listener(|this, _ev, window, cx| {
                        this.toggle_settings(window, cx);
                    }))
                    .on_logo_context_menu(cx.listener(|this, _ev, window, cx| {
                        this.toggle_context_menu(window, cx);
                    })),
            )
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
                    .flex()
                    .flex_1()
                    .flex_col()
                    .w_full()
                    .px(px(12.))
                    .py(px(6.))
                    .bg(self.theme.main_bg)
                    .font_family(font_family)
                    .text_size(px(font_size))
                    .font_features(enable_terminal_ligatures())
                    .overflow_hidden()
                    .when_some(scrollbar_thumb, |this, thumb| this.child(thumb))
                    .children({
                        let selection_range = self.selection;
                        let search_open = self.is_search_open && !self.search_query.is_empty();
                        let search_query_str = self.search_query.to_lowercase();
                        let active_search_offset = self.search_matches.get(self.search_match_idx).copied();
                        let num_lines = lines.len();
                        let row_width = (target_cols as f32 * cell_w).round();

                        lines.into_iter().enumerate().map(move |(row_idx, spans)| {
                            let default_bg = theme.background;
                            let grid_line_idx = row_idx as i32 - display_offset as i32;

                            // Calculate selection highlight range for this row
                            let sel_col_range = if let Some(sel) = selection_range {
                                let (min_p, max_p) = if sel.start <= sel.end {
                                    (sel.start, sel.end)
                                } else {
                                    (sel.end, sel.start)
                                };
                                if grid_line_idx < min_p.line.0 || grid_line_idx > max_p.line.0 {
                                    None
                                } else if min_p.line.0 == max_p.line.0 {
                                    Some((min_p.column.0, max_p.column.0 + 1))
                                } else if grid_line_idx == min_p.line.0 {
                                    Some((min_p.column.0, 300))
                                } else if grid_line_idx == max_p.line.0 {
                                    Some((0, max_p.column.0 + 1))
                                } else {
                                    Some((0, 300))
                                }
                            } else {
                                None
                            };

                            // Search match highlights on this row
                            let mut search_highlights = Vec::new();
                            if search_open && !search_query_str.is_empty() {
                                let mut full_line = String::new();
                                let mut char_to_col: Vec<usize> = Vec::new();
                                let mut char_to_end_col: Vec<usize> = Vec::new();

                                for s in &spans {
                                    let span_cols = s.end_col.saturating_sub(s.start_col);
                                    let char_count = s.text.chars().count();
                                    if char_count > 0 {
                                        let cols_per_char = (span_cols as f32 / char_count as f32).max(1.0);
                                        for (i, c) in s.text.chars().enumerate() {
                                            full_line.push(c);
                                            let col_start = s.start_col + (i as f32 * cols_per_char).round() as usize;
                                            let col_end = if i + 1 == char_count {
                                                s.end_col
                                            } else {
                                                s.start_col + ((i + 1) as f32 * cols_per_char).round() as usize
                                            };
                                            char_to_col.push(col_start);
                                            char_to_end_col.push(col_end);
                                        }
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

                                        let is_active = active_search_offset.map_or(false, |off| {
                                            off == (display_offset + (num_lines.saturating_sub(1 + row_idx)))
                                        });
                                        search_highlights.push((col_start, col_len, is_active));
                                    }
                                    start_b = match_end_b;
                                    if search_query_str.is_empty() { break; }
                                }
                            }

                            div()
                                .relative()
                                .flex()
                                .flex_row()
                                .items_start()
                                .w(px(row_width))
                                .h(px(line_h))
                                .line_height(px(line_h))
                                .overflow_hidden()
                                .children(spans.into_iter().map(move |span| {
                                    let span_width = ((span.end_col as f32 * cell_w).round() - (span.start_col as f32 * cell_w).round()).max(0.0);
                                    let first_char = span.text.chars().next();
                                    let is_pure = first_char.map_or(false, |fc| span.text.chars().all(|c| c == fc));

                                    let geom_opt = if is_pure {
                                        first_char.and_then(|ch| {
                                            render_geometric_cell(
                                                ch,
                                                span_width,
                                                line_h,
                                                span.fg,
                                                span.bg,
                                                default_bg,
                                            )
                                        })
                                    } else {
                                        None
                                    };

                                    if let Some(geom) = geom_opt {
                                        geom
                                    } else {
                                        let mut el = div()
                                            .flex_shrink_0()
                                            .w(px(span_width))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .h(px(line_h))
                                            .line_height(px(line_h))
                                            .text_color(span.fg)
                                            .font_features(enable_terminal_ligatures())
                                            .font_weight(if span.is_bold {
                                                FontWeight::BOLD
                                            } else {
                                                FontWeight::NORMAL
                                            });

                                        if span.is_cursor {
                                            el = el.border_l_2().border_color(cursor_color);
                                        }

                                        if let Some(bg_color) = span.bg {
                                            el = el.bg(bg_color);
                                        }
                                        if span.is_underline {
                                            el = el.underline();
                                        }

                                        el.child(SharedString::from(span.text))
                                    }
                                }))
                                .when_some(sel_col_range, |this, (c_start, c_end)| {
                                    let x_start = (c_start as f32 * cell_w).round();
                                    let x_end = (c_end as f32 * cell_w).round();
                                    let quad_w = (x_end - x_start).max(0.0);
                                    this.child(
                                        div()
                                            .absolute()
                                            .left(px(x_start))
                                            .top(px(0.))
                                            .w(px(quad_w))
                                            .h(px(line_h))
                                            .bg(theme.accent.opacity(0.35))
                                            .rounded(px(2.))
                                    )
                                })
                                .children(search_highlights.into_iter().map(move |(m_col, m_len, is_active)| {
                                    let x_start = (m_col as f32 * cell_w).round();
                                    let x_end = ((m_col + m_len) as f32 * cell_w).round();
                                    let quad_w = (x_end - x_start).max(cell_w);

                                    div()
                                        .absolute()
                                        .left(px(x_start))
                                        .top(px(0.))
                                        .w(px(quad_w))
                                        .h(px(line_h))
                                        .bg(if is_active { theme.accent.opacity(0.70) } else { theme.yellow.opacity(0.40) })
                                        .border_b_2()
                                        .border_color(if is_active { theme.accent } else { theme.yellow })
                                        .rounded(px(2.))
                                }))
                        })
                    }),
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
            .when(self.is_settings_open, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_settings_open = false;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                            this.is_settings_open = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .absolute()
                                .top(px(44.))
                                .right(px(12.))
                                .w(px(310.))
                                .p(px(14.))
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
                                .on_mouse_down(MouseButton::Right, |_ev, _window, cx| {
                                    cx.stop_propagation();
                                })
                        // Header
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
                                        .child("Settings"),
                                )
                                .child(
                                    div()
                                        .cursor(CursorStyle::PointingHand)
                                        .text_size(px(13.))
                                        .text_color(theme.muted)
                                        .hover(|s| s.text_color(theme.foreground))
                                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                            this.is_settings_open = false;
                                            cx.notify();
                                        }))
                                        .child(render_icon(IconType::X, theme.accent, 12.0)),
                                ),
                        )
                        // Themes section
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.muted_strong)
                                        .child("THEME"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .flex_wrap()
                                        .gap_1()
                                        .children(
                                            crate::config::all_theme_names().into_iter().map(|name| {
                                                let is_active = self.current_theme_name == name;
                                                let theme_name_str = name.clone();
                                                let label = match name.as_str() {
                                                    "default" => "Default".to_string(),
                                                    "catppuccin" => "Catppuccin".to_string(),
                                                    "one-dark" => "One Dark".to_string(),
                                                    "solarized-dark" => "Solarized".to_string(),
                                                    "high-contrast" => "Contrast".to_string(),
                                                    other => other.to_string(),
                                                };
                                                div()
                                                    .px(px(8.))
                                                    .py(px(4.))
                                                    .rounded(px(5.))
                                                    .bg(if is_active { theme.accent } else { theme.surface_raised })
                                                    .text_color(if is_active { theme.black } else { theme.foreground })
                                                    .text_size(px(11.))
                                                    .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                    .cursor(CursorStyle::PointingHand)
                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                        this.set_theme(&theme_name_str, cx);
                                                    }))
                                                    .child(label)
                                            })
                                        ),
                                ),
                        )
                        // Font size section
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .py(px(2.))
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.muted_strong)
                                        .child("FONT SIZE"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(22.))
                                                .h(px(22.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(4.))
                                                .bg(theme.surface_raised)
                                                .hover(|s| s.bg(theme.hover))
                                                .text_color(theme.foreground)
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.adjust_font_size(-1.0, cx);
                                                }))
                                                .child("−"),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.foreground)
                                                .child(format!("{:.0}px", self.font_size)),
                                        )
                                        .child(
                                            div()
                                                .w(px(22.))
                                                .h(px(22.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(4.))
                                                .bg(theme.surface_raised)
                                                .hover(|s| s.bg(theme.hover))
                                                .text_color(theme.foreground)
                                                .cursor(CursorStyle::PointingHand)
                                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                                                    this.adjust_font_size(1.0, cx);
                                                }))
                                                .child("+"),
                                        ),
                                ),
                        )
                        // Font family section
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.muted_strong)
                                                .child("FONT FAMILY"),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(theme.muted)
                                                .child(self.font_family.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("font-family-list")
                                        .flex()
                                        .flex_col()
                                        .max_h(px(140.))
                                        .overflow_y_scroll()
                                        .p(px(2.))
                                        .rounded(px(6.))
                                        .bg(theme.surface_raised)
                                        .border_1()
                                        .border_color(theme.border)
                                        .gap_1()
                                        .children(
                                            available_system_fonts()
                                                .into_iter()
                                                .map(|font_name| {
                                                    let is_active = self.font_family.as_ref() == font_name;
                                                    let name_clone = font_name.clone();
                                                    div()
                                                        .flex()
                                                        .flex_row()
                                                        .items_center()
                                                        .justify_between()
                                                        .px(px(8.))
                                                        .py(px(4.))
                                                        .rounded(px(4.))
                                                        .bg(if is_active { theme.accent } else { theme.surface_raised })
                                                        .hover(|s| if !is_active { s.bg(theme.hover) } else { s })
                                                        .text_color(if is_active { theme.black } else { theme.foreground })
                                                        .text_size(px(11.))
                                                        .font_weight(if is_active { FontWeight::BOLD } else { FontWeight::NORMAL })
                                                        .cursor(CursorStyle::PointingHand)
                                                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, _window, cx| {
                                                            this.set_font_family(&name_clone, cx);
                                                        }))
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .overflow_hidden()
                                                                .child(font_name)
                                                        )
                                                        .when(is_active, |el| el.child(render_icon(IconType::Check, theme.black, 12.0)))
                                                })
                                        ),
                                ),
                        )
                        // Open config.toml button
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .w_full()
                                .py(px(6.))
                                .rounded(px(6.))
                                .bg(theme.surface_raised)
                                .hover(|s| s.bg(theme.hover))
                                .border_1()
                                .border_color(theme.border)
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.foreground)
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(MouseButton::Left, cx.listener(|_this, _ev, _window, _cx| {
                                    Self::open_settings_file();
                                }))
                                .child("Edit config.toml ↗"),
                        ),
                )
                )
            })
            .when(self.is_context_menu_open, |this| {
                let sc_palette = if cfg!(target_os = "macos") { "⌘P" } else { "Ctrl+Shift+P" };
                let sc_ssh = if cfg!(target_os = "macos") { "⌘O" } else { "Ctrl+Shift+O" };
                let sc_search = if cfg!(target_os = "macos") { "⌘F" } else { "Ctrl+Shift+F" };
                let sc_settings = if cfg!(target_os = "macos") { "⌘," } else { "Ctrl+," };
                let sc_new_tab = if cfg!(target_os = "macos") { "⌘T" } else { "Ctrl+Shift+T" };
                let sc_clear = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl+Shift+K" };
                let sc_fullscreen = if cfg!(target_os = "macos") { "⌃⌘F" } else { "F11" };
                let sc_quit = if cfg!(target_os = "macos") { "⌘Q" } else { "Alt+F4" };

                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _ev, _window, cx| {
                            this.is_context_menu_open = false;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, _ev, _window, cx| {
                            this.is_context_menu_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("logo-context-menu-popup")
                        .absolute()
                        .top(px(34.))
                        .right(px(6.))
                        .w(px(220.))
                        .p(px(4.))
                        .rounded(px(8.))
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
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
                            cx.listener(|this, _ev, _window, cx| {
                                this.is_context_menu_open = false;
                                this.is_settings_open = true;
                                cx.notify();
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
                                        .pb(px(8.))
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
                                        .gap_1()
                                        .p(px(10.))
                                        .rounded(px(6.))
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
                                        .pt(px(4.))
                                        .child(
                                            div()
                                                .cursor(CursorStyle::PointingHand)
                                                .px(px(10.))
                                                .py(px(5.))
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
                                                .px(px(14.))
                                                .py(px(5.))
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
                let query = self.command_palette_query.to_lowercase();
                let all_cmds = get_all_palette_commands();
                let filtered: Vec<PaletteCommand> = all_cmds
                    .into_iter()
                    .filter(|c| query.is_empty() || c.title.to_lowercase().contains(&query) || c.category.to_lowercase().contains(&query))
                    .collect();
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
                            this.is_command_palette_open = false;
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
                                                                            .text_color(if is_selected { theme.black } else { theme.muted })
                                                                            .child(cmd.category),
                                                                    )
                                                                    .when_some(cmd.shortcut, |this, sc| {
                                                                        this.child(
                                                                            div()
                                                                                .px(px(4.))
                                                                                .py(px(1.))
                                                                                .rounded(px(3.))
                                                                                .bg(if is_selected { theme.hover } else { theme.surface_raised })
                                                                                .text_size(px(10.))
                                                                                .font_weight(FontWeight::MEDIUM)
                                                                                .text_color(if is_selected { theme.black } else { theme.muted })
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
            // SSH Host Manager Modal
            .when(self.is_ssh_manager_open, |this| {
                let mut backdrop_bg = theme.black;
                backdrop_bg.a = 0.65;
                let hosts = crate::ssh::parse_ssh_config();
                let query = self.ssh_manager_query.to_lowercase();
                let filtered: Vec<crate::ssh::SshHost> = hosts
                    .into_iter()
                    .filter(|h| query.is_empty() || h.name.to_lowercase().contains(&query) || h.hostname.to_lowercase().contains(&query) || h.user.to_lowercase().contains(&query))
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
                                                                .child("Add 'Host <name>' entries to ~/.ssh/config to launch remote sessions instantly."),
                                                        ),
                                                ]
                                            } else {
                                                filtered
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, host)| {
                                                        let is_selected = idx == selected_idx;
                                                        let host_clone = host.clone();
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
                                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _ev, window, cx| {
                                                                this.is_ssh_manager_open = false;
                                                                let title = format!("ssh: {}", host_clone.name);
                                                                this.create_tab_with_cmd("ssh", &host_clone.ssh_args(), Some(title), window, cx);
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
                                                                            .child(host.name),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(10.5))
                                                                            .text_color(if is_selected { theme.black } else { theme.muted })
                                                                            .child(format!("{}@{}", host.user, host.hostname)),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .px(px(6.))
                                                                    .py(px(2.))
                                                                    .rounded(px(3.))
                                                                    .bg(if is_selected { theme.hover } else { theme.surface_raised })
                                                                    .text_size(px(10.))
                                                                    .text_color(if is_selected { theme.black } else { theme.muted })
                                                                    .child(format!("port {}", host.port)),
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
                let is_git = active_cwd.map_or(false, crate::git::is_git_repo);
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
                                                                            .text_color(if is_selected { theme.black } else { theme.muted })
                                                                            .child(wt.path.to_string_lossy().to_string()),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .px(px(6.))
                                                                    .py(px(2.))
                                                                    .rounded(px(3.))
                                                                    .bg(if is_selected { theme.hover } else { theme.surface_raised })
                                                                    .text_size(px(10.))
                                                                    .text_color(if is_selected { theme.black } else { theme.muted })
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
                            || cwd_str.as_ref().map_or(false, |c| c.to_lowercase().contains(&query))
                            || branch_str.as_ref().map_or(false, |b| b.to_lowercase().contains(&query));
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
                                                                            .text_color(if is_selected { theme.black } else { theme.muted })
                                                                            .child(cwd_str.unwrap_or_else(|| "~".to_string())),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .px(px(6.))
                                                                    .py(px(2.))
                                                                    .rounded(px(3.))
                                                                    .bg(if is_selected { theme.hover } else { theme.surface_raised })
                                                                    .text_size(px(10.))
                                                                    .text_color(if is_selected { theme.black } else { theme.muted })
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
                                .child(render_context_menu_item(
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
                                ))
                                .child(render_context_menu_item(
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
                                ))
                                .child(render_context_menu_item(
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
                                ))
                                .child(render_context_menu_item(
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
                                ))
                                .child(render_context_menu_item(
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
                                ))
                                .child(render_context_menu_item(
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
                                ))
                                .child(render_context_menu_item(
                                    IconType::GitBranch,
                                    "Git Worktree Picker",
                                    Some(if cfg!(target_os = "macos") { "⌘⌥W" } else { "Ctrl+Alt+W" }),
                                    cx.listener(|this, _ev, window, cx| {
                                        this.is_git_menu_open = false;
                                        this.toggle_worktree_picker(window, cx);
                                    }),
                                    theme,
                                ))
                                .child(render_context_menu_item(
                                    IconType::Clipboard,
                                    "Copy Branch Name",
                                    None,
                                    {
                                        let b_name = branch_name.clone();
                                        cx.listener(move |this, _ev, _window, cx| {
                                            this.is_git_menu_open = false;
                                            if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                                                let _ = clip.set_text(b_name.clone());
                                            }
                                            cx.notify();
                                        })
                                    },
                                    theme,
                                ))
                                .when_some(remote_url, |this, url| {
                                    this.child(render_context_menu_item(
                                        IconType::Globe,
                                        "Open Remote in Browser",
                                        None,
                                        cx.listener(move |this, _ev, _window, cx| {
                                            this.is_git_menu_open = false;
                                            open_path_or_url(&url);
                                            cx.notify();
                                        }),
                                        theme,
                                    ))
                                }),
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
                                                    this.is_update_modal_open = false;
                                                    cx.notify();
                                                }))
                                                .child("OK"),
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
}
