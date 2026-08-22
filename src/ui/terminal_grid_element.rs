use gpui::{
    fill, point, px, size, App, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Style, TextAlign, TextRun, Window,
};
use alacritty_terminal::vte::ansi::CursorShape;

use crate::ui::root_view::{Selection, StyledSpan};

#[derive(Clone, Debug)]
pub struct VisibleImage {
    pub row: i64,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
    pub z_index: i32,
    pub image: std::sync::Arc<gpui::RenderImage>,
}

pub struct TerminalGridElement {
    pub lines: Vec<Vec<StyledSpan>>,
    pub cell_w: f32,
    pub line_h: f32,
    pub row_width: f32,
    pub cursor_info: Option<(i32, usize, CursorShape)>,
    pub selection_range: Option<Selection>,
    pub search_highlights: Vec<Vec<(usize, usize, bool)>>,
    pub visible_images: Vec<VisibleImage>,
    pub theme: crate::ui::theme::Theme,
    pub cursor_color: gpui::Hsla,
    pub font_family: gpui::SharedString,
    pub font_size: f32,
    pub display_offset: usize,
}

impl IntoElement for TerminalGridElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalGridElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: gpui::Size {
                width: px(self.row_width).into(),
                height: px(self.line_h * self.lines.len() as f32).into(),
            },
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let origin = bounds.origin;
        let emoji_font = gpui::font(self.font_family.clone());
        let normal_font = gpui::font(self.font_family.clone());

        // 1. Background Pass: Render negative z-index images beneath cell backgrounds
        for img in self.visible_images.iter().filter(|i| i.z_index < 0) {
            let img_x = origin.x + px(img.col as f32 * self.cell_w);
            let img_y = origin.y + px(img.row as f32 * self.line_h);
            let img_w = img.cols as f32 * self.cell_w;
            let img_h = img.rows as f32 * self.line_h;
            let img_bounds = Bounds::new(
                point(img_x, img_y),
                size(px(img_w), px(img_h)),
            );
            let mask = bounds.intersect(&img_bounds);
            if mask.size.width > px(0.0) && mask.size.height > px(0.0) {
                let _ = window.paint_image(img_bounds, mask, gpui::Corners::default(), img.image.clone(), 0, false);
            }
        }

        for (row_idx, spans) in self.lines.iter().enumerate() {
            let grid_line_idx = row_idx as i32 - self.display_offset as i32;
            let y = origin.y + px((row_idx as f32 * self.line_h).floor());

            // 1. Cell Backgrounds
            for span in spans {
                if let Some(bg) = span.bg {
                    let x_start = (span.start_col as f32 * self.cell_w).floor();
                    let x_end = (span.end_col as f32 * self.cell_w).floor();
                    let span_width = (x_end - x_start).max(0.0);
                    let bg_bounds = Bounds::new(
                        point(origin.x + px(x_start), y),
                        size(px(span_width), px(self.line_h)),
                    );
                    window.paint_quad(fill(bg_bounds, bg));
                }
            }

            // 2. Selection
            if let Some(sel) = self.selection_range {
                let (min_p, max_p) = if sel.start <= sel.end { (sel.start, sel.end) } else { (sel.end, sel.start) };
                let sel_col_range = if grid_line_idx < min_p.line.0 || grid_line_idx > max_p.line.0 {
                    None
                } else if min_p.line.0 == max_p.line.0 {
                    Some((min_p.column.0, max_p.column.0 + 1))
                } else if grid_line_idx == min_p.line.0 {
                    Some((min_p.column.0, 300))
                } else if grid_line_idx == max_p.line.0 {
                    Some((0, max_p.column.0 + 1))
                } else {
                    Some((0, 300))
                };

                if let Some((c_start, c_end)) = sel_col_range {
                    let x_start = (c_start as f32 * self.cell_w).floor();
                    let x_end = (c_end as f32 * self.cell_w).floor();
                    let quad_w = (x_end - x_start).max(0.0);
                    let sel_bounds = Bounds::new(
                        point(origin.x + px(x_start), y),
                        size(px(quad_w), px(self.line_h)),
                    );
                    window.paint_quad(fill(sel_bounds, self.theme.accent.opacity(0.35)));
                }
            }

            // 3. Search Highlights
            if let Some(search_row) = self.search_highlights.get(row_idx) {
                for &(m_col, m_len, is_active) in search_row {
                    let x_start = (m_col as f32 * self.cell_w).floor();
                    let x_end = ((m_col + m_len) as f32 * self.cell_w).floor();
                    let quad_w = (x_end - x_start).max(self.cell_w);

                    let bg_color = if is_active { self.theme.accent.opacity(0.70) } else { self.theme.yellow.opacity(0.40) };
                    let border_color = if is_active { self.theme.accent } else { self.theme.yellow };

                    let hl_bounds = Bounds::new(
                        point(origin.x + px(x_start), y),
                        size(px(quad_w), px(self.line_h)),
                    );
                    window.paint_quad(fill(hl_bounds, bg_color));

                    let border_bounds = Bounds::new(
                        point(origin.x + px(x_start), y + px(self.line_h - 2.0)),
                        size(px(quad_w), px(2.0)),
                    );
                    window.paint_quad(fill(border_bounds, border_color));
                }
            }

            // 4. Text and Geometric Glyphs
            let default_bg = self.theme.background;

            for span in spans {
                if !span.text.is_empty() {
                    let is_scaled_emoji = span.emoji_scale.is_some();
                    let font_size = if let Some(scale) = span.emoji_scale {
                        self.font_size * scale
                    } else {
                        self.font_size
                    };

                    let has_geometric = span.text.chars().any(is_geometric_codepoint);

                    if has_geometric {
                        let mut text_run_buf = String::new();
                        let mut text_run_start_col = span.start_col;

                        for (i, c) in span.text.chars().enumerate() {
                            let char_col = span.char_cols.get(i).copied().unwrap_or(span.start_col + i);
                            let c_start = (char_col as f32 * self.cell_w).floor();
                            let c_end = ((char_col + 1) as f32 * self.cell_w).floor();
                            let c_w = (c_end - c_start).max(1.0);
                            let cell_bounds = Bounds::new(
                                point(origin.x + px(c_start), y),
                                size(px(c_w), px(self.line_h)),
                            );

                            if paint_geometric_char(c, cell_bounds, span.fg, span.bg, default_bg, window) {
                                if !text_run_buf.is_empty() {
                                    paint_text_run(
                                        &text_run_buf,
                                        text_run_start_col,
                                        span.fg,
                                        span.is_underline,
                                        span.is_emoji,
                                        font_size,
                                        self.cell_w,
                                        self.line_h,
                                        origin,
                                        y,
                                        &emoji_font,
                                        &normal_font,
                                        is_scaled_emoji,
                                        window,
                                        cx,
                                    );
                                    text_run_buf.clear();
                                }
                            } else {
                                if text_run_buf.is_empty() {
                                    text_run_start_col = char_col;
                                }
                                text_run_buf.push(c);
                            }
                        }

                        if !text_run_buf.is_empty() {
                            paint_text_run(
                                &text_run_buf,
                                text_run_start_col,
                                span.fg,
                                span.is_underline,
                                span.is_emoji,
                                font_size,
                                self.cell_w,
                                self.line_h,
                                origin,
                                y,
                                &emoji_font,
                                &normal_font,
                                is_scaled_emoji,
                                window,
                                cx,
                            );
                        }
                    } else {
                        paint_text_run(
                            &span.text,
                            span.start_col,
                            span.fg,
                            span.is_underline,
                            span.is_emoji,
                            font_size,
                            self.cell_w,
                            self.line_h,
                            origin,
                            y,
                            &emoji_font,
                            &normal_font,
                            is_scaled_emoji,
                            window,
                            cx,
                        );
                    }
                }
            }
            // 5. Cursor (Pass 4: exact cell-aligned cursor rendering)
            if let Some((c_row, c_col, c_shape)) = self.cursor_info {
                if grid_line_idx == c_row {
                    let x_start = (c_col as f32 * self.cell_w).floor();
                    let x_end = ((c_col + 1) as f32 * self.cell_w).floor();
                    let quad_w = (x_end - x_start).max(self.cell_w);

                    match c_shape {
                        CursorShape::Underline => {
                            let b = Bounds::new(
                                point(origin.x + px(x_start), y + px(self.line_h - 2.0)),
                                size(px(quad_w), px(2.0)),
                            );
                            window.paint_quad(fill(b, self.cursor_color));
                        }
                        CursorShape::HollowBlock => {
                            let b = Bounds::new(
                                point(origin.x + px(x_start), y),
                                size(px(quad_w), px(self.line_h)),
                            );
                            let mut q = fill(b, gpui::Hsla::transparent_black());
                            q.border_color = self.cursor_color;
                            q.border_widths = gpui::Edges {
                                top: px(1.0),
                                right: px(1.0),
                                bottom: px(1.0),
                                left: px(1.0),
                            };
                            window.paint_quad(q);
                        }
                        CursorShape::Block => {
                            let b = Bounds::new(
                                point(origin.x + px(x_start), y),
                                size(px(quad_w), px(self.line_h)),
                            );
                            window.paint_quad(fill(b, self.cursor_color.opacity(0.80)));
                        }
                        CursorShape::Beam | _ => {
                            // Beam `|` (2px width)
                            let b = Bounds::new(
                                point(origin.x + px(x_start), y),
                                size(px(2.0), px(self.line_h)),
                            );
                            window.paint_quad(fill(b, self.cursor_color));
                        }
                    }
                }
            }
        }

        // 6. Positive & Inline Images (Pass 5: on top of backgrounds/selection)
        for img in self.visible_images.iter().filter(|i| i.z_index >= 0) {
            let img_x = origin.x + px(img.col as f32 * self.cell_w);
            let img_y = origin.y + px(img.row as f32 * self.line_h);
            let img_w = img.cols as f32 * self.cell_w;
            let img_h = img.rows as f32 * self.line_h;
            let img_bounds = Bounds::new(
                point(img_x, img_y),
                size(px(img_w), px(img_h)),
            );
            let mask = bounds.intersect(&img_bounds);
            if mask.size.width > px(0.0) && mask.size.height > px(0.0) {
                let _ = window.paint_image(img_bounds, mask, gpui::Corners::default(), img.image.clone(), 0, false);
            }
        }
    }
}

#[inline]
fn is_geometric_codepoint(ch: char) -> bool {
    let code = ch as u32;
    (0x2500..=0x259F).contains(&code) || (0x25A0..=0x25FF).contains(&code)
}

fn paint_text_run(
    text: &str,
    start_col: usize,
    fg: gpui::Hsla,
    is_underline: bool,
    is_emoji: bool,
    font_size: f32,
    cell_w: f32,
    line_h: f32,
    origin: gpui::Point<Pixels>,
    row_y: Pixels,
    emoji_font: &gpui::Font,
    normal_font: &gpui::Font,
    is_scaled_emoji: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let x_start = (start_col as f32 * cell_w).floor();
    let text_pos = point(origin.x + px(x_start), row_y);

    let run = TextRun {
        len: text.len(),
        font: if is_emoji { emoji_font.clone() } else { normal_font.clone() },
        color: fg,
        background_color: None,
        underline: if is_underline {
            Some(gpui::UnderlineStyle {
                color: Some(fg),
                thickness: px(1.0),
                wavy: false,
            })
        } else {
            None
        },
        strikethrough: None,
    };

    let force_width = if is_scaled_emoji || is_emoji {
        None
    } else {
        Some(px(cell_w))
    };

    let align = if is_scaled_emoji {
        TextAlign::Center
    } else {
        TextAlign::Left
    };

    let align_width = if is_scaled_emoji {
        let span_w = (text.chars().count() as f32 * cell_w).floor();
        Some(px(span_w))
    } else {
        None
    };

    let _ = window
        .text_system()
        .shape_line(
            text.to_string().into(),
            px(font_size),
            &[run],
            force_width,
        )
        .paint(
            text_pos,
            px(line_h),
            align,
            align_width,
            window,
            cx,
        );
}

fn paint_geometric_char(
    ch: char,
    bounds: Bounds<Pixels>,
    fg: gpui::Hsla,
    bg: Option<gpui::Hsla>,
    theme_bg: gpui::Hsla,
    window: &mut Window,
) -> bool {
    let code = ch as u32;
    let bg_c = bg.unwrap_or(theme_bg);
    let x = bounds.origin.x;
    let y = bounds.origin.y;
    let width = bounds.size.width;
    let line_h = bounds.size.height;
    let half_h = (line_h / 2.0).floor().max(px(1.0));
    let rem_h = (line_h - half_h).max(px(1.0));
    let half_w = (width / 2.0).floor().max(px(1.0));
    let rem_w = (width - half_w).max(px(1.0));

    // 1. Block Elements (0x2580..=0x259F)
    if (0x2580..=0x259F).contains(&code) {
        match code {
            0x2588 => { // Full block █
                window.paint_quad(fill(bounds, fg));
            }
            0x2580 => { // Upper half block ▀
                window.paint_quad(fill(Bounds::new(point(x, y), size(width, half_h)), fg));
                window.paint_quad(fill(Bounds::new(point(x, y + half_h), size(width, rem_h)), bg_c));
            }
            0x2584 => { // Lower half block ▄
                window.paint_quad(fill(Bounds::new(point(x, y), size(width, half_h)), bg_c));
                window.paint_quad(fill(Bounds::new(point(x, y + half_h), size(width, rem_h)), fg));
            }
            0x258C => { // Left half block ▌
                window.paint_quad(fill(Bounds::new(point(x, y), size(half_w, line_h)), fg));
                window.paint_quad(fill(Bounds::new(point(x + half_w, y), size(rem_w, line_h)), bg_c));
            }
            0x2590 => { // Right half block ▐
                window.paint_quad(fill(Bounds::new(point(x, y), size(half_w, line_h)), bg_c));
                window.paint_quad(fill(Bounds::new(point(x + half_w, y), size(rem_w, line_h)), fg));
            }
            0x2581..=0x2587 => { // Lower fraction blocks  ▂▃▄▅▆▇
                let frac = (code - 0x2580) as f32 / 8.0;
                let fill_h = (line_h * frac).floor().max(px(1.0));
                window.paint_quad(fill(bounds, bg_c));
                window.paint_quad(fill(Bounds::new(point(x, y + line_h - fill_h), size(width, fill_h)), fg));
            }
            0x2589..=0x258F => { // Left fraction blocks ▏▎▍▌▋▊▉
                let frac = (8 - (code - 0x2588)) as f32 / 8.0;
                let fill_w = (width * frac).floor().max(px(1.0));
                window.paint_quad(fill(bounds, bg_c));
                window.paint_quad(fill(Bounds::new(point(x, y), size(fill_w, line_h)), fg));
            }
            0x2591 => { // Light shade ░
                let mut blended = fg;
                blended.a = 0.25;
                window.paint_quad(fill(bounds, bg_c));
                window.paint_quad(fill(bounds, blended));
            }
            0x2592 => { // Medium shade ▒
                let mut blended = fg;
                blended.a = 0.50;
                window.paint_quad(fill(bounds, bg_c));
                window.paint_quad(fill(bounds, blended));
            }
            0x2593 => { // Dark shade ▓
                let mut blended = fg;
                blended.a = 0.75;
                window.paint_quad(fill(bounds, bg_c));
                window.paint_quad(fill(bounds, blended));
            }
            0x2594 => { // Upper 1/8th ▔
                let fill_h = (line_h * 0.125).floor().max(px(1.0));
                window.paint_quad(fill(bounds, bg_c));
                window.paint_quad(fill(Bounds::new(point(x, y), size(width, fill_h)), fg));
            }
            0x2595 => { // Right 1/8th ▕
                let fill_w = (width * 0.125).floor().max(px(1.0));
                window.paint_quad(fill(bounds, bg_c));
                window.paint_quad(fill(Bounds::new(point(x + width - fill_w, y), size(fill_w, line_h)), fg));
            }
            0x2596..=0x259F => { // Quadrants ▖▗▘▙▛▜▟▚▞
                let (tl, tr, bl, br) = match code {
                    0x2596 => (bg_c, bg_c, fg, bg_c),
                    0x2597 => (bg_c, bg_c, bg_c, fg),
                    0x2598 => (fg, bg_c, bg_c, bg_c),
                    0x2599 => (fg, bg_c, fg, fg),
                    0x259A => (fg, bg_c, bg_c, fg),
                    0x259B => (fg, fg, fg, bg_c),
                    0x259C => (fg, fg, bg_c, fg),
                    0x259D => (bg_c, fg, bg_c, bg_c),
                    0x259E => (bg_c, fg, fg, bg_c),
                    0x259F => (bg_c, fg, fg, fg),
                    _ => return false,
                };
                window.paint_quad(fill(Bounds::new(point(x, y), size(half_w, half_h)), tl));
                window.paint_quad(fill(Bounds::new(point(x + half_w, y), size(rem_w, half_h)), tr));
                window.paint_quad(fill(Bounds::new(point(x, y + half_h), size(half_w, rem_h)), bl));
                window.paint_quad(fill(Bounds::new(point(x + half_w, y + half_h), size(rem_w, rem_h)), br));
            }
            _ => return false,
        }
        return true;
    }

    // 2. Box Drawing Characters (0x2500..=0x257F)
    if let Some((left_style, right_style, top_style, bottom_style, kind)) = crate::ui::root_view::decode_box_drawing(ch) {
        let mid_x = (width / 2.0).floor();
        let mid_y = (line_h / 2.0).floor();
        let t_light = px(1.0);
        let t_heavy = px(2.0);
        let get_t = |s: u8| if s == 2 { t_heavy } else { t_light };

        let t_l = get_t(left_style);
        let t_r = get_t(right_style);
        let t_t = get_t(top_style);
        let t_b = get_t(bottom_style);

        // Always paint background first
        window.paint_quad(fill(bounds, bg_c));

        if kind == 1 {
            // Round corners (0x256D..=0x2570)
            let radius = (mid_x.min(mid_y) * 0.9).round();
            let mut corner_bounds = bounds;
            let mut rounded = gpui::Corners::default();
            let mut borders = gpui::Edges::default();

            if code == 0x256D { // ╭
                corner_bounds = Bounds::new(point(x + mid_x - t_l / 2.0, y + mid_y - t_t / 2.0), size(width - mid_x + t_l / 2.0, line_h - mid_y + t_t / 2.0));
                rounded.top_left = radius;
                borders.top = t_t;
                borders.left = t_l;
            } else if code == 0x256E { // ╮
                corner_bounds = Bounds::new(point(x, y + mid_y - t_t / 2.0), size(mid_x + t_r / 2.0, line_h - mid_y + t_t / 2.0));
                rounded.top_right = radius;
                borders.top = t_t;
                borders.right = t_r;
            } else if code == 0x2570 { // ╰
                corner_bounds = Bounds::new(point(x + mid_x - t_l / 2.0, y), size(width - mid_x + t_l / 2.0, mid_y + t_b / 2.0));
                rounded.bottom_left = radius;
                borders.bottom = t_b;
                borders.left = t_l;
            } else if code == 0x256F { // ╯
                corner_bounds = Bounds::new(point(x, y), size(mid_x + t_r / 2.0, mid_y + t_b / 2.0));
                rounded.bottom_right = radius;
                borders.bottom = t_b;
                borders.right = t_r;
            }

            let mut q = fill(corner_bounds, gpui::Hsla::transparent_black());
            q.border_color = fg;
            q.border_widths = borders;
            q.corner_radii = rounded;
            window.paint_quad(q);
            return true;
        }

        // Horizontal strokes
        if left_style > 0 && right_style > 0 && left_style == right_style && left_style != 3 {
            window.paint_quad(fill(
                Bounds::new(point(x, y + mid_y - t_l / 2.0), size(width, t_l)),
                fg,
            ));
        } else {
            if left_style > 0 {
                window.paint_quad(fill(
                    Bounds::new(point(x, y + mid_y - t_l / 2.0), size(mid_x, t_l)),
                    fg,
                ));
            }
            if right_style > 0 {
                window.paint_quad(fill(
                    Bounds::new(point(x + mid_x, y + mid_y - t_r / 2.0), size(width - mid_x, t_r)),
                    fg,
                ));
            }
        }

        // Vertical strokes
        if top_style > 0 && bottom_style > 0 && top_style == bottom_style && top_style != 3 {
            window.paint_quad(fill(
                Bounds::new(point(x + mid_x - t_t / 2.0, y), size(t_t, line_h)),
                fg,
            ));
        } else {
            if top_style > 0 {
                window.paint_quad(fill(
                    Bounds::new(point(x + mid_x - t_t / 2.0, y), size(t_t, mid_y)),
                    fg,
                ));
            }
            if bottom_style > 0 {
                window.paint_quad(fill(
                    Bounds::new(point(x + mid_x - t_b / 2.0, y + mid_y), size(t_b, line_h - mid_y)),
                    fg,
                ));
            }
        }

        return true;
    }

    false
}

