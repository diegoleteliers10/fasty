use std::sync::Arc;
use std::time::Duration;
use gpui::*;
use crate::ui::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum MarkdownBlock {
    Paragraph(Vec<InlineSpan>),
    Heading { level: usize, content: Vec<InlineSpan> },
    CodeBlock { language: Option<String>, code: String },
    MathBlock { math: String },
    List { ordered: bool, items: Vec<Vec<MarkdownBlock>> },
    Blockquote(Vec<MarkdownBlock>),
    HorizontalRule,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InlineSpan {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Math(String),
}

impl InlineSpan {
    pub fn text(&self) -> &str {
        match self {
            InlineSpan::Text(t) => t,
            InlineSpan::Bold(t) => t,
            InlineSpan::Italic(t) => t,
            InlineSpan::Code(t) => t,
            InlineSpan::Math(t) => t,
        }
    }
}

/// Parses the full text into a sequence of MarkdownBlocks.
pub fn parse_markdown(text: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Code Blocks
        if trimmed.starts_with("```") {
            let rest = &trimmed[3..];
            if let Some(end_idx) = rest.rfind("```") {
                let inner = &rest[..end_idx];
                let (lang, code) = if let Some(space_idx) = inner.find(' ') {
                    (Some(inner[..space_idx].to_string()), inner[space_idx + 1..].to_string())
                } else {
                    (None, inner.to_string())
                };
                blocks.push(MarkdownBlock::CodeBlock {
                    language: lang.filter(|l| !l.is_empty()),
                    code,
                });
                continue;
            }
            let language = rest.trim().to_string();
            let mut code = String::new();
            while let Some(code_line) = lines.next() {
                if code_line.trim().starts_with("```") {
                    break;
                }
                code.push_str(code_line);
                code.push('\n');
            }
            blocks.push(MarkdownBlock::CodeBlock {
                language: if language.is_empty() { None } else { Some(language) },
                code: code.trim_end().to_string(),
            });
            continue;
        }

        // Math Blocks: $$...$$ or \[...\]
        if trimmed.starts_with("$$") || trimmed.starts_with("\\[") {
            let (marker, end_marker) = if trimmed.starts_with("$$") { ("$$", "$$") } else { ("\\[", "\\]") };
            let after_start = &trimmed[marker.len()..];
            
            // Single-line math block
            if after_start.ends_with(end_marker) && after_start.len() >= end_marker.len() {
                let inner = &after_start[..after_start.len() - end_marker.len()];
                blocks.push(MarkdownBlock::MathBlock {
                    math: inner.trim().to_string(),
                });
                continue;
            }

            let mut math = String::new();
            if !after_start.trim().is_empty() {
                math.push_str(after_start);
                math.push('\n');
            }
            while let Some(math_line) = lines.next() {
                let mtrim = math_line.trim();
                if mtrim.starts_with(end_marker) || mtrim.ends_with(end_marker) {
                    let before_end = mtrim.trim_end_matches(end_marker);
                    if !before_end.trim().is_empty() {
                        math.push_str(before_end);
                        math.push('\n');
                    }
                    break;
                }
                math.push_str(math_line);
                math.push('\n');
            }
            blocks.push(MarkdownBlock::MathBlock {
                math: math.trim().to_string(),
            });
            continue;
        }

        // Horizontal Rules
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            blocks.push(MarkdownBlock::HorizontalRule);
            continue;
        }

        // Headings (# Heading)
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            if level > 0 && level <= 6 && trimmed[level..].starts_with(' ') {
                let content = trimmed[level..].trim();
                blocks.push(MarkdownBlock::Heading {
                    level,
                    content: parse_inline(content),
                });
                continue;
            }
        }

        // Blockquotes
        if trimmed.starts_with("> ") || trimmed == ">" {
            let mut content = if trimmed.len() > 2 { trimmed[2..].to_string() } else { String::new() };
            content.push('\n');
            while let Some(next_line) = lines.peek() {
                let ntrim = next_line.trim_start();
                if ntrim.starts_with('>') {
                    content.push_str(ntrim.trim_start_matches('>').trim_start());
                    content.push('\n');
                    lines.next();
                } else if next_line.trim().is_empty() {
                    break;
                } else {
                    content.push_str(next_line);
                    content.push('\n');
                    lines.next();
                }
            }
            blocks.push(MarkdownBlock::Blockquote(parse_markdown(&content)));
            continue;
        }

        // Unordered Lists
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            let mut items = Vec::new();
            let content = trimmed[2..].to_string();
            items.push(parse_markdown(&content));

            while let Some(next_line) = lines.peek() {
                let next_trim = next_line.trim_start();
                if next_trim.starts_with("- ") || next_trim.starts_with("* ") || next_trim.starts_with("+ ") {
                    items.push(parse_markdown(next_trim[2..].trim_start()));
                    lines.next();
                } else if next_line.trim().is_empty() {
                    break;
                } else {
                    if let Some(last) = items.last_mut() {
                        let mut text = String::new();
                        text.push_str(next_line);
                        last.extend(parse_markdown(&text));
                    }
                    lines.next();
                }
            }
            blocks.push(MarkdownBlock::List { ordered: false, items });
            continue;
        }

        // Ordered Lists
        if let Some(dot_idx) = trimmed.find(". ") {
            if dot_idx > 0 && trimmed[..dot_idx].chars().all(|c| c.is_ascii_digit()) {
                let mut items = Vec::new();
                items.push(parse_markdown(trimmed[dot_idx + 2..].trim_start()));

                while let Some(next_line) = lines.peek() {
                    let next_trim = next_line.trim_start();
                    if let Some(ndot) = next_trim.find(". ") {
                        if ndot > 0 && next_trim[..ndot].chars().all(|c| c.is_ascii_digit()) {
                            items.push(parse_markdown(next_trim[ndot + 2..].trim_start()));
                            lines.next();
                            continue;
                        }
                    }
                    if next_line.trim().is_empty() {
                        break;
                    }
                    if let Some(last) = items.last_mut() {
                        let mut text = String::new();
                        text.push_str(next_line);
                        last.extend(parse_markdown(&text));
                    }
                    lines.next();
                }
                blocks.push(MarkdownBlock::List { ordered: true, items });
                continue;
            }
        }

        // Paragraph
        let mut para_text = line.to_string();
        while let Some(next_line) = lines.peek() {
            let ntrim = next_line.trim();
            if ntrim.is_empty()
                || ntrim.starts_with("```")
                || ntrim == "---" || ntrim == "***" || ntrim == "___"
                || (ntrim.starts_with('#') && ntrim.chars().take_while(|&c| c == '#').count() <= 6 && ntrim.find(' ').map_or(false, |s| s <= 6))
                || ntrim.starts_with("> ")
                || ntrim.starts_with("$$")
                || ntrim.starts_with("\\[")
                || ntrim.starts_with("- ")
                || ntrim.starts_with("* ")
                || ntrim.starts_with("+ ")
                || (ntrim.find(". ").map_or(false, |idx| idx > 0 && ntrim[..idx].chars().all(|c| c.is_ascii_digit())))
            {
                break;
            }
            para_text.push(' ');
            para_text.push_str(ntrim);
            lines.next();
        }
        blocks.push(MarkdownBlock::Paragraph(parse_inline(&para_text)));
    }

    blocks
}

/// Parses a string into a series of InlineSpans.
pub fn parse_inline(text: &str) -> Vec<InlineSpan> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut byte_idx = 0;
    let len = text.len();

    while byte_idx < len {
        let remaining = &text[byte_idx..];

        // Inline Code: `code`
        if remaining.starts_with('`') {
            let rest = &remaining[1..];
            if let Some(rel) = rest.find('`') {
                if !current.is_empty() {
                    spans.push(InlineSpan::Text(std::mem::take(&mut current)));
                }
                spans.push(InlineSpan::Code(rest[..rel].to_string()));
                byte_idx += 1 + rel + 1;
                continue;
            }
        }

        // Inline Math: $math$
        if remaining.starts_with('$') {
            let rest = &remaining[1..];
            let first_ch = rest.chars().next();
            // Don't treat currency ($50), spaced ($ x), or block ($$) as inline math
            let is_not_math = first_ch.map_or(true, |c| c.is_ascii_digit() || c.is_whitespace() || c == '$');
            if !is_not_math {
                if let Some(rel) = rest.find('$') {
                    let inner = &rest[..rel];
                    if !inner.is_empty() && !inner.contains('\n') && !inner.ends_with(char::is_whitespace) {
                        if !current.is_empty() {
                            spans.push(InlineSpan::Text(std::mem::take(&mut current)));
                        }
                        spans.push(InlineSpan::Math(inner.to_string()));
                        byte_idx += 1 + rel + 1;
                        continue;
                    }
                }
            }
        }

        // Bold: **bold**
        if remaining.starts_with("**") {
            let rest = &remaining[2..];
            if let Some(rel) = rest.find("**") {
                if rel > 0 {
                    if !current.is_empty() {
                        spans.push(InlineSpan::Text(std::mem::take(&mut current)));
                    }
                    spans.push(InlineSpan::Bold(rest[..rel].to_string()));
                    byte_idx += 2 + rel + 2;
                    continue;
                }
            }
        }

        // Italic: *italic*
        if remaining.starts_with('*') && !remaining.starts_with("**") {
            let rest = &remaining[1..];
            let mut close_idx = None;
            for (idx, ch) in rest.char_indices() {
                if ch == '*' {
                    if !rest[idx..].starts_with("**") {
                        close_idx = Some(idx);
                        break;
                    }
                }
            }
            if let Some(rel) = close_idx {
                if rel > 0 {
                    if !current.is_empty() {
                        spans.push(InlineSpan::Text(std::mem::take(&mut current)));
                    }
                    spans.push(InlineSpan::Italic(rest[..rel].to_string()));
                    byte_idx += 1 + rel + 1;
                    continue;
                }
            }
        }

        // Underscore Bold: __bold__
        if remaining.starts_with("__") {
            let is_boundary = current.is_empty() || current.ends_with(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
            if is_boundary {
                let rest = &remaining[2..];
                if let Some(rel) = rest.find("__") {
                    if rel > 0 {
                        if !current.is_empty() {
                            spans.push(InlineSpan::Text(std::mem::take(&mut current)));
                        }
                        spans.push(InlineSpan::Bold(rest[..rel].to_string()));
                        byte_idx += 2 + rel + 2;
                        continue;
                    }
                }
            }
        }

        // Underscore Italic: _italic_
        if remaining.starts_with('_') && !remaining.starts_with("__") {
            let is_boundary = current.is_empty() || current.ends_with(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
            if is_boundary {
                let rest = &remaining[1..];
                if let Some(rel) = rest.find('_') {
                    let after = &rest[rel + 1..];
                    let after_boundary = after.chars().next().map_or(true, |c| c.is_whitespace() || c.is_ascii_punctuation());
                    if rel > 0 && after_boundary && !rest[..rel].contains('\n') {
                        if !current.is_empty() {
                            spans.push(InlineSpan::Text(std::mem::take(&mut current)));
                        }
                        spans.push(InlineSpan::Italic(rest[..rel].to_string()));
                        byte_idx += 1 + rel + 1;
                        continue;
                    }
                }
            }
        }

        // Plain char
        let ch = remaining.chars().next().unwrap();
        current.push(ch);
        byte_idx += ch.len_utf8();
    }

    if !current.is_empty() {
        spans.push(InlineSpan::Text(current));
    }

    spans
}

pub fn render_markdown(text: &str, theme: &Theme, is_streaming: bool) -> Div {
    render_markdown_selectable(text, theme, is_streaming, None, None, None, 0.0, None)
}

pub fn render_markdown_selectable(
    text: &str,
    theme: &Theme,
    is_streaming: bool,
    selection: Option<(usize, usize)>,
    on_select_char: Option<Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    window: Option<&Window>,
) -> Div {
    let blocks = parse_markdown(text);
    let mut container = div().flex().flex_col().gap_3().w_full().min_w(px(0.));

    if blocks.is_empty() {
        if is_streaming {
            container = container.child(render_cursor(theme));
        }
        return container;
    }

    let mut offset_tracker = 0usize;
    for (i, block) in blocks.iter().enumerate() {
        let is_last = i == blocks.len() - 1;
        let show_cursor_here = is_last && is_streaming;
        let block_div = render_block(
            block,
            theme,
            show_cursor_here,
            selection,
            on_select_char.as_ref(),
            on_drag_char.as_ref(),
            text_left,
            &mut offset_tracker,
            text,
            window,
        );
        container = container.child(block_div);
    }

    container
}

fn render_cursor(theme: &Theme) -> impl IntoElement {
    div()
        .w(px(2.))
        .h(px(14.))
        .rounded(px(1.))
        .bg(theme.accent)
        .flex_shrink_0()
        .ml(px(2.))
        .with_animation(
            "ai-stream-cursor-inline",
            Animation::new(Duration::from_millis(1100))
                .repeat()
                .with_easing(pulsating_between(0.25, 1.0))
                .with_max_fps(30.),
            |el, delta| el.opacity(delta),
        )
}

fn render_block(
    block: &MarkdownBlock,
    theme: &Theme,
    show_cursor: bool,
    selection: Option<(usize, usize)>,
    on_select_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    offset_tracker: &mut usize,
    source_text: &str,
    window: Option<&Window>,
) -> Div {
    match block {
        MarkdownBlock::Paragraph(spans) => {
            render_inline_spans(
                spans,
                theme,
                px(13.),
                FontWeight::NORMAL,
                show_cursor,
                selection,
                on_select_char,
                on_drag_char,
                text_left,
                offset_tracker,
                window,
            )
        }
        MarkdownBlock::Heading { level, content } => {
            let (size, weight) = match level {
                1 => (px(20.), FontWeight::BOLD),
                2 => (px(18.), FontWeight::BOLD),
                3 => (px(16.), FontWeight::SEMIBOLD),
                _ => (px(14.), FontWeight::SEMIBOLD),
            };
            render_inline_spans(
                content,
                theme,
                size,
                weight,
                show_cursor,
                selection,
                on_select_char,
                on_drag_char,
                text_left,
                offset_tracker,
                window,
            )
        }
        MarkdownBlock::CodeBlock { language, code } => {
            let code_start = *offset_tracker;
            *offset_tracker += code.chars().count();

            let mut container = div()
                .w_full()
                .min_w(px(0.))
                .overflow_hidden()
                .rounded_md()
                .bg(theme.surface_raised)
                .border_1()
                .border_color(theme.border)
                .my(px(4.))
                .flex()
                .flex_col();

            let code_copy = code.clone();
            container = container.child(
                div()
                    .w_full()
                    .px_3()
                    .py_1()
                    .bg(theme.surface)
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .child(language.clone().unwrap_or_else(|| "text".to_string())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .cursor(CursorStyle::PointingHand)
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .hover(move |s| s.text_color(theme.foreground))
                            .on_mouse_down(MouseButton::Left, move |_ev, _window, _cx| {
                                if let Some(mut clip) = crate::event_listener::clipboard_helper() {
                                    let _ = clip.set_text(code_copy.clone());
                                }
                            })
                            .child(crate::ui::icons::render_icon(icons::common::IconType::Copy, theme.muted, 11.0))
                            .child("Copy"),
                    ),
            );

            let code_lines: Vec<&str> = code.lines().collect();
            let mut line_char_accum = code_start;
            let mut code_body = div()
                .w_full()
                .min_w(px(0.))
                .p_3()
                .flex()
                .flex_col()
                .gap(px(1.))
                .overflow_hidden();

            for code_line in code_lines {
                let cl_len = code_line.chars().count();
                code_body = code_body.child(render_span_with_selection(
                    code_line,
                    theme,
                    px(12.),
                    FontWeight::NORMAL,
                    false,
                    false,
                    false,
                    line_char_accum,
                    selection,
                    on_select_char,
                    on_drag_char,
                    text_left,
                    window,
                ));
                line_char_accum += cl_len + 1;
            }

            if show_cursor {
                code_body = code_body.child(render_cursor(theme));
            }

            container.child(code_body)
        }
        MarkdownBlock::MathBlock { math } => {
            let math_start = *offset_tracker;
            *offset_tracker += math.chars().count();

            let mut container = div()
                .w_full()
                .min_w(px(0.))
                .overflow_hidden()
                .flex()
                .justify_center()
                .my(px(3.))
                .py_2()
                .child(render_span_with_selection(
                    math,
                    theme,
                    px(13.),
                    FontWeight::NORMAL,
                    false,
                    false,
                    true,
                    math_start,
                    selection,
                    on_select_char,
                    on_drag_char,
                    text_left,
                    window,
                ));
            if show_cursor {
                container = container.child(render_cursor(theme));
            }
            container
        }
        MarkdownBlock::Blockquote(blocks) => {
            let mut quote = div()
                .w_full()
                .min_w(px(0.))
                .overflow_hidden()
                .border_l_4()
                .border_color(theme.border)
                .pl_3()
                .py_1()
                .my(px(2.))
                .flex()
                .flex_col()
                .gap_2()
                .italic()
                .text_color(theme.muted);

            for (i, b) in blocks.iter().enumerate() {
                let is_last = i == blocks.len() - 1;
                quote = quote.child(render_block(
                    b,
                    theme,
                    show_cursor && is_last,
                    selection,
                    on_select_char,
                    on_drag_char,
                    text_left,
                    offset_tracker,
                    source_text,
                    window,
                ));
            }
            quote
        }
        MarkdownBlock::HorizontalRule => {
            let mut container = div().w_full().min_w(px(0.)).overflow_hidden().flex().flex_col();
            container = container.child(div().w_full().h_px().bg(theme.border).my_2());
            if show_cursor {
                container = container.child(render_cursor(theme));
            }
            container
        }
        MarkdownBlock::List { ordered, items } => {
            let mut list_div = div().w_full().min_w(px(0.)).overflow_hidden().flex().flex_col().gap_1();
            for (i, item_blocks) in items.iter().enumerate() {
                let mut item_container = div().w_full().min_w(px(0.)).flex().flex_row().gap_2();

                let marker = if *ordered {
                    format!("{}.", i + 1)
                } else {
                    "•".to_string()
                };

                item_container = item_container.child(
                    div()
                        .w(px(16.))
                        .flex_shrink_0()
                        .text_size(px(13.))
                        .text_color(theme.foreground)
                        .child(marker),
                );

                let mut content_col = div().flex_1().min_w(px(0.)).overflow_hidden().flex().flex_col().gap_1();
                for (j, b) in item_blocks.iter().enumerate() {
                    let is_last_item = i == items.len() - 1;
                    let is_last_block = j == item_blocks.len() - 1;
                    content_col = content_col.child(render_block(
                        b,
                        theme,
                        show_cursor && is_last_item && is_last_block,
                        selection,
                        on_select_char,
                        on_drag_char,
                        text_left,
                        offset_tracker,
                        source_text,
                        window,
                    ));
                }

                list_div = list_div.child(item_container.child(content_col));
            }
            list_div
        }
    }
}

fn render_inline_spans(
    spans: &[InlineSpan],
    theme: &Theme,
    size: gpui::Pixels,
    weight: FontWeight,
    show_cursor: bool,
    selection: Option<(usize, usize)>,
    on_select_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    offset_tracker: &mut usize,
    _window: Option<&Window>,
) -> Div {
    let mut full_text = String::new();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();

    let para_char_start = *offset_tracker;
    let mut local_char_accum = 0usize;

    for span in spans {
        let t = span.text();
        local_char_accum += t.chars().count();

        let byte_start = full_text.len();
        if matches!(span, InlineSpan::Math(_)) {
            full_text.push('$');
            full_text.push_str(t);
            full_text.push('$');
        } else {
            full_text.push_str(t);
        }
        let byte_end = full_text.len();

        match span {
            InlineSpan::Bold(_) => {
                highlights.push((byte_start..byte_end, HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                }));
            }
            InlineSpan::Italic(_) => {
                highlights.push((byte_start..byte_end, HighlightStyle {
                    font_style: Some(FontStyle::Italic),
                    ..Default::default()
                }));
            }
            InlineSpan::Code(_) => {
                highlights.push((byte_start..byte_end, HighlightStyle {
                    color: Some(theme.accent),
                    background_color: Some(theme.surface_raised),
                    ..Default::default()
                }));
            }
            InlineSpan::Math(_) => {
                highlights.push((byte_start..byte_end, HighlightStyle {
                    color: Some(theme.accent),
                    ..Default::default()
                }));
            }
            InlineSpan::Text(_) => {}
        }
    }

    *offset_tracker = para_char_start + local_char_accum;

    let para_char_len = full_text.chars().count();
    let para_char_end = para_char_start + para_char_len;

    // Selection highlight
    if let Some((s, e)) = selection {
        let s_clamped = s.max(para_char_start).min(para_char_end);
        let e_clamped = e.max(para_char_start).min(para_char_end);
        if s_clamped < e_clamped {
            let rel_s = s_clamped - para_char_start;
            let rel_e = e_clamped - para_char_start;
            let b_start = crate::ui::text_input::char_to_byte_idx(&full_text, rel_s);
            let b_end = crate::ui::text_input::char_to_byte_idx(&full_text, rel_e);
            highlights.push((b_start..b_end, HighlightStyle {
                background_color: Some(crate::ui::text_input::selection_highlight_color()),
                color: Some(theme.bright_white),
                ..Default::default()
            }));
        }
    }

    let font_size = size.to_f64() as f32;
    let on_select = on_select_char.cloned();
    let on_drag = on_drag_char.cloned();
    let full_text_down = full_text.clone();
    let full_text_move = full_text.clone();

    let mut row = div()
        .w_full()
        .min_w(px(0.))
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
            if let Some(ref cb) = on_select {
                let click_x = ev.position.x.to_f64() as f32;
                let rel_x = (click_x - text_left).max(0.0);
                let col = crate::ui::text_input::index_for_x(&full_text_down, rel_x, font_size, window);
                let target = para_char_start + col.min(para_char_len);
                cb(target, window, cx);
            }
        });

    if on_drag.is_some() {
        row = row.on_mouse_move(move |ev, window, cx| {
            if ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            if let Some(ref cb) = on_drag {
                let click_x = ev.position.x.to_f64() as f32;
                let rel_x = (click_x - text_left).max(0.0);
                let col = crate::ui::text_input::index_for_x(&full_text_move, rel_x, font_size, window);
                let target = para_char_start + col.min(para_char_len);
                cb(target, window, cx);
            }
        });
    }

    let styled = StyledText::new(full_text).with_highlights(highlights);
    let text_el = div()
        .w_full()
        .min_w(px(0.))
        .text_size(size)
        .font_weight(weight)
        .text_color(theme.foreground)
        .child(styled);

    if show_cursor {
        row = row.flex().flex_row().flex_wrap().items_center().child(text_el).child(render_cursor(theme));
    } else {
        row = row.child(text_el);
    }

    row
}

fn render_span_with_selection(
    text: &str,
    theme: &Theme,
    size: gpui::Pixels,
    weight: FontWeight,
    is_italic: bool,
    is_code: bool,
    is_math: bool,
    span_start: usize,
    selection: Option<(usize, usize)>,
    on_select_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    _window: Option<&Window>,
) -> Div {
    let span_len = text.chars().count();
    let span_end = span_start + span_len;

    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();

    let default_color = if is_code || is_math { theme.accent } else { theme.foreground };
    let display_text = if is_math { format!("${}$", text) } else { text.to_string() };

    if is_italic {
        highlights.push((0..display_text.len(), HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        }));
    }

    if let Some((s, e)) = selection {
        let s_clamped = s.max(span_start).min(span_end);
        let e_clamped = e.max(span_start).min(span_end);
        if s_clamped < e_clamped {
            let rel_s = s_clamped - span_start;
            let rel_e = e_clamped - span_start;
            let b_start = crate::ui::text_input::char_to_byte_idx(&display_text, rel_s);
            let b_end = crate::ui::text_input::char_to_byte_idx(&display_text, rel_e);
            highlights.push((b_start..b_end, HighlightStyle {
                background_color: Some(crate::ui::text_input::selection_highlight_color()),
                color: Some(theme.bright_white),
                ..Default::default()
            }));
        }
    }

    let on_select = on_select_char.cloned();
    let on_drag = on_drag_char.cloned();
    let font_size = size.to_f64() as f32;
    let text_owned = display_text.clone();
    let text_owned2 = display_text.clone();

    let mut row = div()
        .min_w(px(0.))
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
            if let Some(ref cb) = on_select {
                let click_x = ev.position.x.to_f64() as f32;
                let rel_x = (click_x - text_left).max(0.0);
                let col = crate::ui::text_input::index_for_x(&text_owned, rel_x, font_size, window);
                let target = span_start + col.min(span_len);
                cb(target, window, cx);
            }
        });

    if on_drag.is_some() {
        row = row.on_mouse_move(move |ev, window, cx| {
            if ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            if let Some(ref cb) = on_drag {
                let click_x = ev.position.x.to_f64() as f32;
                let rel_x = (click_x - text_left).max(0.0);
                let col = crate::ui::text_input::index_for_x(&text_owned2, rel_x, font_size, window);
                let target = span_start + col.min(span_len);
                cb(target, window, cx);
            }
        });
    }

    let styled = StyledText::new(display_text).with_highlights(highlights);
    row.child(
        div()
            .text_size(size)
            .font_weight(weight)
            .text_color(default_color)
            .child(styled),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[std::prelude::v1::test]
    fn test_heading_parsing() {
        let blocks = parse_markdown("# Heading 1\n## Heading 2\n####### Not Heading\n#NotHeading");
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0], MarkdownBlock::Heading {
            level: 1,
            content: vec![InlineSpan::Text("Heading 1".to_string())],
        });
        assert_eq!(blocks[1], MarkdownBlock::Heading {
            level: 2,
            content: vec![InlineSpan::Text("Heading 2".to_string())],
        });
        assert_eq!(blocks[2], MarkdownBlock::Paragraph(vec![
            InlineSpan::Text("####### Not Heading #NotHeading".to_string())
        ]));
    }

    #[std::prelude::v1::test]
    fn test_single_line_and_multiline_math() {
        let blocks = parse_markdown("$$x^2 + y^2 = z^2$$\n\nText after");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], MarkdownBlock::MathBlock {
            math: "x^2 + y^2 = z^2".to_string(),
        });
        assert_eq!(blocks[1], MarkdownBlock::Paragraph(vec![InlineSpan::Text("Text after".to_string())]));

        let bracket_blocks = parse_markdown("\\[a + b = c\\]\nParagraph");
        assert_eq!(bracket_blocks.len(), 2);
        assert_eq!(bracket_blocks[0], MarkdownBlock::MathBlock {
            math: "a + b = c".to_string(),
        });

        let multi = parse_markdown("$$\n\\int_0^1 f(x) dx\n$$\nTail");
        assert_eq!(multi.len(), 2);
        assert_eq!(multi[0], MarkdownBlock::MathBlock {
            math: "\\int_0^1 f(x) dx".to_string(),
        });
    }

    #[std::prelude::v1::test]
    fn test_ordered_list_dot_guard() {
        let blocks = parse_markdown(". Not an ordered list\n1. Real item\n2. Second item");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], MarkdownBlock::Paragraph(vec![InlineSpan::Text(". Not an ordered list".to_string())]));
        match &blocks[1] {
            MarkdownBlock::List { ordered, items } => {
                assert!(ordered);
                assert_eq!(items.len(), 2);
            }
            _ => panic!("Expected ordered list"),
        }
    }

    #[std::prelude::v1::test]
    fn test_inline_math_and_currency() {
        let spans = parse_inline("Cost is $50 and env is $PATH and math is $x + y$.");
        assert_eq!(spans, vec![
            InlineSpan::Text("Cost is $50 and env is $PATH and math is ".to_string()),
            InlineSpan::Math("x + y".to_string()),
            InlineSpan::Text(".".to_string()),
        ]);
    }

    #[std::prelude::v1::test]
    fn test_unclosed_inline_math() {
        let spans = parse_inline("Unclosed $math without end");
        assert_eq!(spans, vec![InlineSpan::Text("Unclosed $math without end".to_string())]);
    }
}

