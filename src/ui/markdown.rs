use std::sync::Arc;
use std::time::Duration;
use gpui::*;
use crate::ui::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum MarkdownBlock {
    /// Inline content with the char offset of each span inside the source text.
    Paragraph { spans: Vec<LocSpan>, src: usize },
    Heading {
        level: usize,
        content: Vec<LocSpan>,
        src: usize,
    },
    /// Code lines paired with the char offset of each line in the source text.
    CodeBlock {
        language: Option<String>,
        code_lines: Vec<(String, usize)>,
    },
    MathBlock { math: String, src: usize },
    List {
        ordered: bool,
        items: Vec<Vec<MarkdownBlock>>,
    },
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

/// An inline span plus the char offset of its content in the source markdown.
/// Selection indices live in source char space, so every rendered character
/// can be mapped back to the position a user selected or copied.
#[derive(Debug, Clone, PartialEq)]
pub struct LocSpan {
    pub span: InlineSpan,
    pub src: usize,
}

/// One raw source line plus the char offset of its start. All offsets are in
/// char (not byte) space, matching the selection model.
struct SrcLine {
    raw: String,
    start: usize,
}

impl SrcLine {
    /// (char offset of first non-whitespace char, trimmed content)
    fn trim_info(&self) -> (usize, String) {
        let lead = self.raw.chars().take_while(|c| c.is_whitespace()).count();
        let trimmed: String = self
            .raw
            .chars()
            .skip(lead)
            .collect::<String>()
            .trim_end()
            .to_string();
        (self.start + lead, trimmed)
    }
}

fn char_slice(s: &str, from_chars: usize) -> String {
    s.chars().skip(from_chars).collect()
}

fn leading_whitespace_chars(s: &str) -> usize {
    s.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

fn split_source_lines(text: &str) -> Vec<SrcLine> {
    let mut lines = Vec::new();
    let mut off = 0usize;
    for raw_line in text.lines() {
        let had_cr = raw_line.ends_with('\r');
        let clean = if had_cr {
            &raw_line[..raw_line.len() - 1]
        } else {
            raw_line
        };
        lines.push(SrcLine {
            raw: clean.to_string(),
            start: off,
        });
        off += clean.chars().count() + usize::from(had_cr) + 1;
    }
    lines
}

/// Parses the full text into a sequence of MarkdownBlocks annotated with
/// source char offsets.
pub fn parse_markdown(text: &str) -> Vec<MarkdownBlock> {
    let segments: Vec<(String, usize)> = split_source_lines(text)
        .into_iter()
        .map(|l| (l.raw, l.start))
        .collect();
    parse_segments(&segments)
}

fn parse_segments(segments: &[(String, usize)]) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut i = 0usize;

    while i < segments.len() {
        let (raw, start) = &segments[i];
        let line = SrcLine {
            raw: raw.clone(),
            start: *start,
        };
        let (trim_start, trimmed) = line.trim_info();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        let trimmed_chars: Vec<char> = trimmed.chars().collect();

        // Code Blocks
        if trimmed.starts_with("```") {
            let rest: String = char_slice(&trimmed, 3);
            if let Some(end_rel) = rest.rfind("```") {
                let inner = &rest[..end_rel];
                let inner_ws = leading_whitespace_chars(inner);
                let (lang, code, code_src) = if let Some(space_idx) = inner.find(' ') {
                    (
                        Some(inner[..space_idx].to_string()),
                        inner[space_idx + 1..].to_string(),
                        trim_start + 3 + inner_ws + space_idx + 1,
                    )
                } else {
                    (None, inner.to_string(), trim_start + 3 + inner_ws)
                };
                blocks.push(MarkdownBlock::CodeBlock {
                    language: lang.filter(|l| !l.is_empty()),
                    code_lines: if code.is_empty() {
                        Vec::new()
                    } else {
                        vec![(code, code_src)]
                    },
                });
                i += 1;
                continue;
            }
            let language = rest.trim().to_string();
            let mut code_lines: Vec<(String, usize)> = Vec::new();
            i += 1;
            while i < segments.len() {
                let (code_raw, code_start) = &segments[i];
                let code_trim = code_raw.trim();
                if code_trim.starts_with("```") {
                    i += 1;
                    break;
                }
                code_lines.push((code_raw.clone(), *code_start));
                i += 1;
            }
            blocks.push(MarkdownBlock::CodeBlock {
                language: if language.is_empty() { None } else { Some(language) },
                code_lines,
            });
            continue;
        }

        // Math Blocks: $$...$$ or \[...\]
        if trimmed.starts_with("$$") || trimmed.starts_with("\\[") {
            let (marker, end_marker) = if trimmed.starts_with("$$") { ("$$", "$$") } else { ("\\[", "\\]") };
            let marker_chars = marker.chars().count();
            let after_start: String = char_slice(&trimmed, marker_chars);

            // Single-line math block
            if after_start.ends_with(end_marker) && after_start.chars().count() >= end_marker.chars().count() {
                let inner_len = after_start.chars().count() - end_marker.chars().count();
                let inner: String = after_start.chars().take(inner_len).collect();
                let inner_ws = leading_whitespace_chars(&inner);
                blocks.push(MarkdownBlock::MathBlock {
                    math: inner.trim().to_string(),
                    src: trim_start + marker_chars + inner_ws,
                });
                i += 1;
                continue;
            }

            let mut math = String::new();
            let math_src = if !after_start.trim().is_empty() {
                trim_start + marker_chars + leading_whitespace_chars(&after_start)
            } else {
                // Content starts at the next non-empty line.
                segments
                    .get(i + 1)
                    .map(|(r, s)| {
                        SrcLine { raw: r.clone(), start: *s }.trim_info().0
                    })
                    .unwrap_or(trim_start)
            };
            if !after_start.trim().is_empty() {
                math.push_str(after_start.trim());
                math.push('\n');
            }
            i += 1;
            while i < segments.len() {
                let (math_line, _) = &segments[i];
                let mtrim = math_line.trim();
                if mtrim.starts_with(end_marker) || mtrim.ends_with(end_marker) {
                    let before_end = mtrim.trim_end_matches(end_marker);
                    if !before_end.trim().is_empty() {
                        math.push_str(before_end);
                        math.push('\n');
                    }
                    i += 1;
                    break;
                }
                math.push_str(mtrim);
                math.push('\n');
                i += 1;
            }
            blocks.push(MarkdownBlock::MathBlock {
                math: math.trim().to_string(),
                src: math_src,
            });
            continue;
        }

        // Horizontal Rules
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            blocks.push(MarkdownBlock::HorizontalRule);
            i += 1;
            continue;
        }

        // Headings (# Heading)
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            if level > 0 && level <= 6 && trimmed_chars.get(level) == Some(&' ') {
                let content_raw = char_slice(&trimmed, level);
                let content_ws = leading_whitespace_chars(&content_raw).max(1);
                let content = content_raw.trim().to_string();
                blocks.push(MarkdownBlock::Heading {
                    level,
                    content: parse_inline_located(&content, trim_start + level + content_ws),
                    src: trim_start,
                });
                i += 1;
                continue;
            }
        }

        // Blockquotes
        if trimmed.starts_with("> ") || trimmed == ">" {
            let mut quote_segments: Vec<(String, usize)> = Vec::new();
            let content_ws = leading_whitespace_chars(char_slice(&trimmed, 2).as_str());
            let content = if trimmed.chars().count() > 2 {
                char_slice(&trimmed, 2).trim().to_string()
            } else {
                String::new()
            };
            quote_segments.push((content, trim_start + 2 + content_ws));
            i += 1;
            while i < segments.len() {
                let (next_raw, next_start) = &segments[i];
                let next_line = SrcLine { raw: next_raw.clone(), start: *next_start };
                let (ns, ntrim) = next_line.trim_info();
                if ntrim.starts_with('>') {
                    let stripped = char_slice(&ntrim, 1);
                    let stripped_ws = leading_whitespace_chars(&stripped);
                    quote_segments.push((stripped.trim().to_string(), ns + 1 + stripped_ws));
                    i += 1;
                } else if ntrim.is_empty() {
                    break;
                } else {
                    quote_segments.push((ntrim, ns));
                    i += 1;
                }
            }
            blocks.push(MarkdownBlock::Blockquote(parse_segments(&quote_segments)));
            continue;
        }

        // Unordered Lists
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            let mut items: Vec<Vec<MarkdownBlock>> = Vec::new();
            let item_ws = leading_whitespace_chars(char_slice(&trimmed, 2).as_str());
            items.push(parse_segments(&[(char_slice(&trimmed, 2).trim().to_string(), trim_start + 2 + item_ws)]));
            i += 1;
            while i < segments.len() {
                let (next_raw, next_start) = &segments[i];
                let next_line = SrcLine { raw: next_raw.clone(), start: *next_start };
                let (ns, ntrim) = next_line.trim_info();
                if ntrim.starts_with("- ") || ntrim.starts_with("* ") || ntrim.starts_with("+ ") {
                    let nws = leading_whitespace_chars(char_slice(&ntrim, 2).as_str());
                    items.push(parse_segments(&[(char_slice(&ntrim, 2).trim().to_string(), ns + 2 + nws)]));
                    i += 1;
                } else if ntrim.is_empty() {
                    break;
                } else {
                    if let Some(last) = items.last_mut() {
                        last.extend(parse_segments(&[(ntrim, ns)]));
                    }
                    i += 1;
                }
            }
            blocks.push(MarkdownBlock::List { ordered: false, items });
            continue;
        }

        // Ordered Lists
        if let Some(dot_idx) = trimmed.find(". ") {
            if dot_idx > 0 && trimmed[..dot_idx].chars().all(|c| c.is_ascii_digit()) {
                let mut items: Vec<Vec<MarkdownBlock>> = Vec::new();
                let after = &trimmed[dot_idx + 2..];
                let aws = leading_whitespace_chars(after);
                items.push(parse_segments(&[(after.trim().to_string(), trim_start + dot_idx + 2 + aws)]));
                i += 1;
                while i < segments.len() {
                    let (next_raw, next_start) = &segments[i];
                    let next_line = SrcLine { raw: next_raw.clone(), start: *next_start };
                    let (ns, ntrim) = next_line.trim_info();
                    if let Some(ndot) = ntrim.find(". ") {
                        if ndot > 0 && ntrim[..ndot].chars().all(|c| c.is_ascii_digit()) {
                            let nafter = &ntrim[ndot + 2..];
                            let naws = leading_whitespace_chars(nafter);
                            items.push(parse_segments(&[(nafter.trim().to_string(), ns + ndot + 2 + naws)]));
                            i += 1;
                            continue;
                        }
                    }
                    if ntrim.is_empty() {
                        break;
                    }
                    if let Some(last) = items.last_mut() {
                        last.extend(parse_segments(&[(ntrim, ns)]));
                    }
                    i += 1;
                }
                blocks.push(MarkdownBlock::List { ordered: true, items });
                continue;
            }
        }

        // Paragraph: first raw line plus trimmed continuation lines, each span
        // keeping its own source offset.
        let mut spans = parse_inline_located(raw, *start);
        i += 1;
        while i < segments.len() {
            let (next_raw, next_start) = &segments[i];
            let next_line = SrcLine { raw: next_raw.clone(), start: *next_start };
            let (ns, ntrim) = next_line.trim_info();
            let is_heading = ntrim.starts_with('#')
                && {
                    let lvl = ntrim.chars().take_while(|&c| c == '#').count();
                    lvl > 0 && lvl <= 6 && ntrim.chars().nth(lvl) == Some(' ')
                };
            let is_ordered_item = ntrim
                .find(". ")
                .map_or(false, |idx| idx > 0 && ntrim[..idx].chars().all(|c| c.is_ascii_digit()));
            if ntrim.is_empty()
                || ntrim.starts_with("```")
                || ntrim == "---" || ntrim == "***" || ntrim == "___"
                || is_heading
                || ntrim.starts_with("> ")
                || ntrim.starts_with("$$")
                || ntrim.starts_with("\\[")
                || ntrim.starts_with("- ")
                || ntrim.starts_with("* ")
                || ntrim.starts_with("+ ")
                || is_ordered_item
            {
                break;
            }
            // The visual space between joined lines maps onto the newline.
            let join_src = ns.saturating_sub(1);
            spans.push(LocSpan {
                span: InlineSpan::Text(" ".to_string()),
                src: join_src,
            });
            spans.extend(parse_inline_located(&ntrim, ns));
            i += 1;
        }
        blocks.push(MarkdownBlock::Paragraph { spans, src: *start });
    }

    blocks
}

/// Parses a string into a series of InlineSpans (no source offsets).
pub fn parse_inline(text: &str) -> Vec<InlineSpan> {
    parse_inline_located(text, 0).into_iter().map(|ls| ls.span).collect()
}

fn starts_with_at(chars: &[char], pos: usize, pat: &str) -> bool {
    let pat_chars: Vec<char> = pat.chars().collect();
    if pos + pat_chars.len() > chars.len() {
        return false;
    }
    chars[pos..pos + pat_chars.len()] == pat_chars[..]
}

fn find_from(chars: &[char], from: usize, needle: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == needle)
}

/// Inline parser tracking the source char offset of every span's content.
pub fn parse_inline_located(text: &str, base: usize) -> Vec<LocSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_src = base;
    let mut i = 0usize;
    let len = chars.len();

    while i < len {
        let rest = &chars[i..];

        // Inline Code: `code`
        if rest[0] == '`' {
            if let Some(rel) = find_from(&chars, i + 1, '`') {
                if rel > i + 1 {
                    if !current.is_empty() {
                        spans.push(LocSpan {
                            span: InlineSpan::Text(std::mem::take(&mut current)),
                            src: current_src,
                        });
                    }
                    let inner: String = chars[i + 1..rel].iter().collect();
                    spans.push(LocSpan {
                        span: InlineSpan::Code(inner),
                        src: base + i + 1,
                    });
                    i = rel + 1;
                    continue;
                }
            }
        }

        // Inline Math: $math$
        if rest[0] == '$' {
            let next_ch = chars.get(i + 1).copied();
            let is_not_math = next_ch.map_or(true, |c| c.is_ascii_digit() || c.is_whitespace() || c == '$');
            if !is_not_math {
                if let Some(rel) = find_from(&chars, i + 1, '$') {
                    let inner: String = chars[i + 1..rel].iter().collect();
                    if rel > i + 1 && !inner.contains('\n') && !inner.ends_with(char::is_whitespace) {
                        if !current.is_empty() {
                            spans.push(LocSpan {
                                span: InlineSpan::Text(std::mem::take(&mut current)),
                                src: current_src,
                            });
                        }
                        spans.push(LocSpan {
                            span: InlineSpan::Math(inner),
                            src: base + i + 1,
                        });
                        i = rel + 1;
                        continue;
                    }
                }
            }
        }

        // Bold: **bold**
        if starts_with_at(&chars, i, "**") {
            let mut close = None;
            let mut j = i + 2;
            while j + 1 < len {
                if chars[j] == '*' && chars[j + 1] == '*' {
                    close = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(j) = close {
                if j > i + 2 {
                    if !current.is_empty() {
                        spans.push(LocSpan {
                            span: InlineSpan::Text(std::mem::take(&mut current)),
                            src: current_src,
                        });
                    }
                    let inner: String = chars[i + 2..j].iter().collect();
                    spans.push(LocSpan {
                        span: InlineSpan::Bold(inner),
                        src: base + i + 2,
                    });
                    i = j + 2;
                    continue;
                }
            }
        }

        // Italic: *italic*
        if rest[0] == '*' && !starts_with_at(&chars, i, "**") {
            let mut close = None;
            let mut j = i + 1;
            while j < len {
                if chars[j] == '*' && !starts_with_at(&chars, j, "**") {
                    close = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(j) = close {
                if j > i + 1 {
                    if !current.is_empty() {
                        spans.push(LocSpan {
                            span: InlineSpan::Text(std::mem::take(&mut current)),
                            src: current_src,
                        });
                    }
                    let inner: String = chars[i + 1..j].iter().collect();
                    spans.push(LocSpan {
                        span: InlineSpan::Italic(inner),
                        src: base + i + 1,
                    });
                    i = j + 1;
                    continue;
                }
            }
        }

        // Underscore Bold: __bold__
        if starts_with_at(&chars, i, "__") {
            let is_boundary = current.is_empty()
                || current.ends_with(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
            if is_boundary {
                let mut close = None;
                let mut j = i + 2;
                while j + 1 < len {
                    if chars[j] == '_' && chars[j + 1] == '_' {
                        close = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(j) = close {
                    if j > i + 2 {
                        if !current.is_empty() {
                            spans.push(LocSpan {
                                span: InlineSpan::Text(std::mem::take(&mut current)),
                                src: current_src,
                            });
                        }
                        let inner: String = chars[i + 2..j].iter().collect();
                        spans.push(LocSpan {
                            span: InlineSpan::Bold(inner),
                            src: base + i + 2,
                        });
                        i = j + 2;
                        continue;
                    }
                }
            }
        }

        // Underscore Italic: _italic_
        if rest[0] == '_' && !starts_with_at(&chars, i, "__") {
            let is_boundary = current.is_empty()
                || current.ends_with(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
            if is_boundary {
                if let Some(j) = find_from(&chars, i + 1, '_') {
                    let inner: String = chars[i + 1..j].iter().collect();
                    let after_ch = chars.get(j + 1).copied();
                    let after_boundary =
                        after_ch.map_or(true, |c| c.is_whitespace() || c.is_ascii_punctuation());
                    if j > i + 1 && after_boundary && !inner.contains('\n') {
                        if !current.is_empty() {
                            spans.push(LocSpan {
                                span: InlineSpan::Text(std::mem::take(&mut current)),
                                src: current_src,
                            });
                        }
                        spans.push(LocSpan {
                            span: InlineSpan::Italic(inner),
                            src: base + i + 1,
                        });
                        i = j + 1;
                        continue;
                    }
                }
            }
        }

        // Plain char
        if current.is_empty() {
            current_src = base + i;
        }
        current.push(rest[0]);
        i += 1;
    }

    if !current.is_empty() {
        spans.push(LocSpan {
            span: InlineSpan::Text(current),
            src: current_src,
        });
    }

    spans
}

pub fn render_markdown(text: &str, theme: &Theme, is_streaming: bool) -> Div {
    render_markdown_selectable(text, theme, is_streaming, None, None, None, 0.0, None, None)
}

#[allow(clippy::too_many_arguments)]
pub fn render_markdown_selectable(
    text: &str,
    theme: &Theme,
    is_streaming: bool,
    selection: Option<(usize, usize)>,
    on_select_char: Option<Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    window: Option<&Window>,
    registry: Option<(&RowRegistry, usize)>,
) -> Div {
    let blocks = parse_markdown(text);
    let mut container = div().flex().flex_col().gap_3().w_full().min_w(px(0.));

    if blocks.is_empty() {
        if is_streaming {
            container = container.child(render_cursor(theme));
        }
        return container;
    }

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
            window,
            registry,
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

/// A selectable text row registered during render so container-level mouse
/// handling can continue (or start) a selection even when the pointer travels
/// over gaps, padding or list markers between text rows.
pub struct SelectableRow {
    pub msg_idx: usize,
    pub text: String,
    pub pieces: Vec<(usize, bool, bool)>,
    pub map: Vec<usize>,
    pub fallback_src: usize,
    pub font_size: f32,
    pub line_height: f32,
    pub bounds: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

pub type RowRegistry = std::rc::Rc<std::cell::RefCell<Vec<SelectableRow>>>;

/// Source char target for a click at `pos`, given a registered row. Falls
/// back to the row's first/last source position when the point is outside.
pub fn char_target_in_row(row: &SelectableRow, pos: Point<Pixels>, window: &Window) -> usize {
    let Some(b) = row.bounds.get() else {
        return row.fallback_src;
    };
    let rel_x = (pos.x - b.origin.x).max(px(0.));
    let rel_y = (pos.y - b.origin.y).to_f64() as f32;
    let width = b.size.width;
    let runs = shape_runs(window.text_style().font(), &row.pieces);
    let local = char_index_at_point(
        &row.text,
        &runs,
        point(rel_x, px(rel_y)),
        width,
        row.font_size,
        row.line_height,
        window,
    );
    src_for_col(&row.map, row.fallback_src, local)
}

pub fn src_for_col(map: &[usize], fallback_src: usize, col: usize) -> usize {
    if map.is_empty() {
        fallback_src
    } else if col == 0 {
        map[0]
    } else if col >= map.len() {
        map[map.len() - 1] + 1
    } else {
        map[col]
    }
}

/// The index of the row of `msg_idx` whose vertical bounds contain `pos`,
/// the closest row of that message above it, or its last row when the point
/// is below everything. Used to start a selection from padding/gaps where no
/// text row is directly hit.
pub fn row_at_point_in_message(
    registry: &RowRegistry,
    msg_idx: usize,
    pos: Point<Pixels>,
) -> Option<usize> {
    let rows = registry.borrow();
    let mut best: Option<usize> = None;
    let mut last: Option<usize> = None;
    for (i, row) in rows.iter().enumerate() {
        if row.msg_idx != msg_idx {
            continue;
        }
        let Some(b) = row.bounds.get() else {
            continue;
        };
        last = Some(i);
        let top = b.origin.y;
        let bottom = top + b.size.height;
        if pos.y >= top && pos.y <= bottom {
            return Some(i);
        }
        if pos.y < top {
            match best {
                None => best = Some(i),
                Some(cur) => {
                    let cur_top = rows[cur].bounds.get().map(|cb| cb.origin.y).unwrap_or(top);
                    if top < cur_top {
                        best = Some(i);
                    }
                }
            }
        }
    }
    best.or(last)
}

/// The index of the row whose vertical bounds contain `pos`, the closest row
/// above it, or the last row when the point is below everything.
pub fn row_at_point(registry: &RowRegistry, pos: Point<Pixels>) -> Option<usize> {
    let rows = registry.borrow();
    let mut best: Option<usize> = None;
    for (i, row) in rows.iter().enumerate() {
        let Some(b) = row.bounds.get() else {
            continue;
        };
        let top = b.origin.y;
        let bottom = top + b.size.height;
        if pos.y >= top && pos.y <= bottom {
            return Some(i);
        }
        // Above the row: candidate for "nearest above" fallback.
        if pos.y < top {
            match best {
                None => best = Some(i),
                Some(cur) => {
                    let cur_top = rows[cur].bounds.get().map(|cb| cb.origin.y).unwrap_or(top);
                    if top < cur_top {
                        best = Some(i);
                    }
                }
            }
        }
    }
    // If the point is below every row, use the last row.
    if best.is_none() {
        best = rows
            .iter()
            .rposition(|r| r.bounds.get().is_some());
    }
    best
}

fn resolve_rel(
    cell: &std::cell::Cell<Option<Bounds<Pixels>>>,
    pos: Point<Pixels>,
    fallback_left: f32,
) -> (f32, f32, f32) {
    if let Some(b) = cell.get() {
        let x = (pos.x - b.origin.x).max(px(0.)).to_f64() as f32;
        let y = (pos.y - b.origin.y).to_f64() as f32;
        (x, y, b.size.width.to_f64() as f32)
    } else {
        ((pos.x.to_f64() as f32 - fallback_left).max(0.0), 0.0, 0.0)
    }
}

/// Char index in `text` for a click at `rel` (relative to the painted text
/// origin), using GPUI's own shaped layout (`shape_text` with the same wrap
/// width the text is painted with). This is the same hit-testing machinery
/// Zed uses: the Y picks the wrapped line, X picks the closest char boundary.
/// A per-span run list keeps shaping metrics identical to the painted text
/// (bold/italic spans measure differently).
pub fn char_index_at_point(
    text: &str,
    runs: &[TextRun],
    rel: Point<Pixels>,
    wrap_width: Pixels,
    font_size: f32,
    line_height: f32,
    window: &Window,
) -> usize {
    if text.is_empty() {
        return 0;
    }
    let shaped = window.text_system().shape_text(
        SharedString::from(text.to_string()),
        px(font_size),
        runs,
        if wrap_width > px(0.) { Some(wrap_width) } else { None },
        None,
    );
    let Ok(lines) = shaped else {
        return 0;
    };
    let Some(line) = lines.first() else {
        return 0;
    };
    match line.closest_index_for_position(rel, px(line_height)) {
        Ok(byte_ix) | Err(byte_ix) => {
            crate::ui::text_input::byte_to_char_idx(text, byte_ix.min(text.len()))
        }
    }
}

/// Builds TextRuns covering the full text with one run per styled piece, so
/// the hit-test layout measures bold/italic pieces exactly like the painted
/// text. `pieces` are (byte len, bold, italic) in paint order.
fn shape_runs(base_font: Font, pieces: &[(usize, bool, bool)]) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::with_capacity(pieces.len());
    for &(len, bold, italic) in pieces {
        if len == 0 {
            continue;
        }
        let mut font = base_font.clone();
        if bold {
            font.weight = FontWeight::BOLD;
        }
        if italic {
            font.style = FontStyle::Italic;
        }
        runs.push(TextRun {
            len,
            font,
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    runs
}

fn selection_style(theme: &Theme) -> HighlightStyle {
    HighlightStyle {
        background_color: Some(crate::ui::text_input::selection_highlight_color()),
        color: Some(theme.bright_white),
        ..Default::default()
    }
}

/// Merges the selection style into the per-span highlights so the selection
/// visuals win over span styling (inline code background, math color, ...).
/// Selection entries are also added for the plain-text gaps between spans.
fn apply_selection_highlights(
    highlights: Vec<(std::ops::Range<usize>, HighlightStyle)>,
    sel: std::ops::Range<usize>,
    sel_style: HighlightStyle,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let mut out: Vec<(std::ops::Range<usize>, HighlightStyle)> =
        Vec::with_capacity(highlights.len() + 4);
    let mut covered: Vec<std::ops::Range<usize>> = Vec::new();

    for (range, mut style) in highlights {
        if range.end <= sel.start || range.start >= sel.end {
            out.push((range, style));
            continue;
        }
        let pre = range.start..range.end.min(sel.start);
        let mid = range.start.max(sel.start)..range.end.min(sel.end);
        let post = range.start.max(sel.end)..range.end;
        if !pre.is_empty() {
            out.push((pre, style.clone()));
        }
        if !mid.is_empty() {
            style.background_color = sel_style.background_color;
            style.color = sel_style.color;
            out.push((mid.clone(), style));
            covered.push(mid);
        }
        if !post.is_empty() {
            out.push((post, style));
        }
    }

    let mut cursor = sel.start;
    for c in &covered {
        if cursor < c.start {
            out.push((cursor..c.start, sel_style.clone()));
        }
        cursor = cursor.max(c.end);
    }
    if cursor < sel.end {
        out.push((cursor..sel.end, sel_style));
    }

    // GPUI's StyledText folds highlights strictly in entry order: each entry
    // becomes one text run tiling the bytes. Entries must be sorted by start
    // (and non-overlapping) or styles land on the wrong words, which is why
    // some words used to "skip" the selection in mixed content.
    out.retain(|(range, _)| !range.is_empty());
    out.sort_by_key(|(range, _)| range.start);
    out
}

/// Maps a source-char selection range onto rendered char positions using the
/// per-character source map. Returns None when the selection misses this run.
fn src_range_to_rendered(map: &[usize], s: usize, e: usize) -> Option<(usize, usize)> {
    let start = map.iter().position(|&c| c >= s)?;
    let end = map.iter().position(|&c| c >= e).unwrap_or(map.len());
    if start < end {
        Some((start, end))
    } else {
        None
    }
}

fn render_block(
    block: &MarkdownBlock,
    theme: &Theme,
    show_cursor: bool,
    selection: Option<(usize, usize)>,
    on_select_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    window: Option<&Window>,
    registry: Option<(&RowRegistry, usize)>,
) -> Div {
    match block {
        MarkdownBlock::Paragraph { spans, src } => render_inline_spans(
            spans,
            *src,
            theme,
            px(13.),
            FontWeight::NORMAL,
            show_cursor,
            selection,
            on_select_char,
            on_drag_char,
            text_left,
            registry,
        ),
        MarkdownBlock::Heading { level, content, src } => {
            let (size, weight) = match level {
                1 => (px(20.), FontWeight::BOLD),
                2 => (px(18.), FontWeight::BOLD),
                3 => (px(16.), FontWeight::SEMIBOLD),
                _ => (px(14.), FontWeight::SEMIBOLD),
            };
            render_inline_spans(
                content,
                *src,
                theme,
                size,
                weight,
                show_cursor,
                selection,
                on_select_char,
                on_drag_char,
                text_left,
                registry,
            )
        }
        MarkdownBlock::CodeBlock {
            language,
            code_lines,
        } => {
            let code_copy: String = code_lines
                .iter()
                .map(|(l, _)| l.as_str())
                .collect::<Vec<_>>()
                .join("\n");

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

            let mut code_body = div()
                .w_full()
                .min_w(px(0.))
                .p_3()
                .flex()
                .flex_col()
                .gap(px(1.))
                .overflow_hidden();

            for (code_line, line_src) in code_lines {
                code_body = code_body.child(render_source_span(
                    code_line,
                    *line_src,
                    theme,
                    px(12.),
                    FontWeight::NORMAL,
                    false,
                    false,
                    false,
                    selection,
                    on_select_char,
                    on_drag_char,
                    text_left,
                    registry,
                ));
            }

            if show_cursor {
                code_body = code_body.child(render_cursor(theme));
            }

            container.child(code_body)
        }
        MarkdownBlock::MathBlock { math, src } => {
            let mut container = div()
                .w_full()
                .min_w(px(0.))
                .overflow_hidden()
                .flex()
                .justify_center()
                .my(px(3.))
                .py_2()
                .child(render_source_span(
                    math,
                    *src,
                    theme,
                    px(13.),
                    FontWeight::NORMAL,
                    false,
                    true,
                    true,
                    selection,
                    on_select_char,
                    on_drag_char,
                    text_left,
                    registry,
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
                    window,
                    registry,
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
                        window,
                        registry,
                    ));
                }

                list_div = list_div.child(item_container.child(content_col));
            }
            list_div
        }
    }
}

fn render_inline_spans(
    spans: &[LocSpan],
    fallback_src: usize,
    theme: &Theme,
    size: gpui::Pixels,
    weight: FontWeight,
    show_cursor: bool,
    selection: Option<(usize, usize)>,
    on_select_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    registry: Option<(&RowRegistry, usize)>,
) -> Div {
    let mut full_text = String::new();
    // rendered char position -> source char position
    let mut map: Vec<usize> = Vec::new();
    // (byte len, bold, italic) per styled piece, for hit-test shaping runs
    let mut pieces: Vec<(usize, bool, bool)> = Vec::new();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();

    for ls in spans {
        let t = ls.span.text();

        let byte_start = full_text.len();
        match &ls.span {
            InlineSpan::Text(_) | InlineSpan::Bold(_) | InlineSpan::Italic(_) | InlineSpan::Code(_) => {
                for k in 0..t.chars().count() {
                    map.push(ls.src + k);
                }
                full_text.push_str(t);
            }
            InlineSpan::Math(_) => {
                map.push(ls.src.saturating_sub(1));
                for k in 0..t.chars().count() {
                    map.push(ls.src + k);
                }
                map.push(ls.src + t.chars().count());
                full_text.push('$');
                full_text.push_str(t);
                full_text.push('$');
            }
        }
        let byte_end = full_text.len();

        let (bold, italic) = match &ls.span {
            InlineSpan::Bold(_) => (true, false),
            InlineSpan::Italic(_) => (false, true),
            _ => (false, false),
        };
        pieces.push((byte_end - byte_start, bold, italic));

        match &ls.span {
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

    // Selection highlight in source char space, merged over span styling so
    // inline code and math are covered by the selection too.
    if let Some((s, e)) = selection {
        let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
        if let Some((r_start, r_end)) = src_range_to_rendered(&map, lo, hi) {
            let b_start = crate::ui::text_input::char_to_byte_idx(&full_text, r_start);
            let b_end = crate::ui::text_input::char_to_byte_idx(&full_text, r_end);
            if b_start < b_end {
                highlights =
                    apply_selection_highlights(highlights, b_start..b_end, selection_style(theme));
            }
        }
    }

    let font_size = size.to_f64() as f32;
    let line_height_px = font_size * 1.35;
    let on_select = on_select_char.cloned();
    let on_drag = on_drag_char.cloned();
    let full_text_down = full_text.clone();
    let full_text_move = full_text.clone();
    let pieces_down = pieces.clone();
    let pieces_move = pieces.clone();
    let map_down = map.clone();
    let map_move = map.clone();
    let bounds_cell: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>> =
        std::rc::Rc::new(std::cell::Cell::new(None));
    let bounds_down = bounds_cell.clone();
    let bounds_move = bounds_cell.clone();

    let map_to_src = move |map: &[usize], col: usize| -> usize {
        if map.is_empty() {
            fallback_src
        } else if col == 0 {
            map[0]
        } else if col >= map.len() {
            map[map.len() - 1] + 1
        } else {
            map[col]
        }
    };

    // Register the row so drags that cross gaps, padding or list markers keep
    // updating the selection via the container-level handler.
    if let Some((reg, msg_idx)) = registry {
        reg.borrow_mut().push(SelectableRow {
            msg_idx,
            text: full_text.clone(),
            pieces: pieces.clone(),
            map: map.clone(),
            fallback_src,
            font_size,
            line_height: line_height_px,
            bounds: bounds_cell.clone(),
        });
    }

    let mut text_el = div()
        .relative()
        .w_full()
        .min_w(px(0.))
        .text_size(size)
        .font_weight(weight)
        .line_height(px(line_height_px))
        .text_color(theme.foreground);

    if let Some(cb) = on_select {
        text_el = text_el.on_mouse_down(MouseButton::Left, move |ev, window, cx| {
            let (rel_x, rel_y, width) = resolve_rel(&bounds_down, ev.position, text_left);
            let runs = shape_runs(window.text_style().font(), &pieces_down);
            let local = char_index_at_point(
                &full_text_down,
                &runs,
                point(px(rel_x), px(rel_y)),
                px(width),
                font_size,
                line_height_px,
                window,
            );
            cb(map_to_src(&map_down, local), window, cx);
        });
    }

    if let Some(cb) = on_drag {
        text_el = text_el.on_mouse_move(move |ev, window, cx| {
            if ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            let (rel_x, rel_y, width) = resolve_rel(&bounds_move, ev.position, text_left);
            let runs = shape_runs(window.text_style().font(), &pieces_move);
            let local = char_index_at_point(
                &full_text_move,
                &runs,
                point(px(rel_x), px(rel_y)),
                px(width),
                font_size,
                line_height_px,
                window,
            );
            cb(map_to_src(&map_move, local), window, cx);
        });
    }

    let styled = StyledText::new(full_text).with_highlights(highlights);
    text_el = text_el
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, _, _| {
                    bounds_cell.set(Some(bounds));
                },
            )
            .absolute()
            .size_full(),
        )
        .child(styled);

    let row = div().w_full().min_w(px(0.)).cursor(CursorStyle::IBeam);

    if show_cursor {
        row.flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .child(text_el)
            .child(render_cursor(theme))
    } else {
        row.child(text_el)
    }
}

/// Renders a run whose rendered characters map 1:1 onto source chars
/// (code-block lines). `is_math` adds the `$` decorations around the display
/// text and maps them onto the adjacent source chars.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn render_source_span(
    text: &str,
    span_src: usize,
    theme: &Theme,
    size: gpui::Pixels,
    weight: FontWeight,
    is_italic: bool,
    is_code: bool,
    is_math: bool,
    selection: Option<(usize, usize)>,
    on_select_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_drag_char: Option<&Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    text_left: f32,
    registry: Option<(&RowRegistry, usize)>,
) -> Div {
    let span_len = text.chars().count();

    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();

    let default_color = if is_code { theme.accent } else { theme.foreground };
    // Math blocks show their $ decorations, mapped onto the adjacent source chars.
    let (decor_left, display_text, decor_right) = if is_math {
        (1, format!("${}$", text), 1)
    } else {
        (0, text.to_string(), 0)
    };
    let display_char_len = decor_left + span_len + decor_right;

    if is_italic {
        highlights.push((0..display_text.len(), HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        }));
    }

    if let Some((s, e)) = selection {
        let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
        let s_clamped = lo.max(span_src).min(span_src + span_len);
        let e_clamped = hi.max(span_src).min(span_src + span_len);
        if s_clamped < e_clamped {
            let rel_s = (s_clamped - span_src + decor_left).min(display_char_len);
            let rel_e = (e_clamped - span_src + decor_left).min(display_char_len);
            let b_start = crate::ui::text_input::char_to_byte_idx(&display_text, rel_s);
            let b_end = crate::ui::text_input::char_to_byte_idx(&display_text, rel_e);
            highlights = apply_selection_highlights(
                std::mem::take(&mut highlights),
                b_start..b_end,
                selection_style(theme),
            );
        }
    }

    let on_select = on_select_char.cloned();
    let on_drag = on_drag_char.cloned();
    let font_size = size.to_f64() as f32;
    let line_height_px = font_size * 1.35;
    let text_owned = display_text.clone();
    let text_owned2 = display_text.clone();
    let pieces: Vec<(usize, bool, bool)> = vec![(display_text.len(), false, is_italic)];
    let pieces2 = pieces.clone();
    let bounds_cell: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>> =
        std::rc::Rc::new(std::cell::Cell::new(None));
    let bounds_down = bounds_cell.clone();
    let bounds_move = bounds_cell.clone();

    // Identity source map (math decorations included) for container-level hits.
    if let Some((reg, msg_idx)) = registry {
        let mut map: Vec<usize> = Vec::with_capacity(display_char_len);
        for _ in 0..decor_left {
            map.push(span_src.saturating_sub(1));
        }
        for k in 0..span_len {
            map.push(span_src + k);
        }
        for _ in 0..decor_right {
            map.push(span_src + span_len);
        }
        reg.borrow_mut().push(SelectableRow {
            msg_idx,
            text: display_text.clone(),
            pieces: pieces.clone(),
            map,
            fallback_src: span_src,
            font_size,
            line_height: line_height_px,
            bounds: bounds_cell.clone(),
        });
    }

    let mut text_el = div()
        .relative()
        .min_w(px(0.))
        .text_size(size)
        .font_weight(weight)
        .line_height(px(line_height_px))
        .text_color(default_color);

    if let Some(cb) = on_select {
        let decor_l = decor_left;
        let span_l = span_len;
        let span_s = span_src;
        text_el = text_el.on_mouse_down(MouseButton::Left, move |ev, window, cx| {
            let (rel_x, rel_y, width) = resolve_rel(&bounds_down, ev.position, text_left);
            let runs = shape_runs(window.text_style().font(), &pieces);
            let col = char_index_at_point(
                &text_owned,
                &runs,
                point(px(rel_x), px(rel_y)),
                px(width),
                font_size,
                line_height_px,
                window,
            );
            let target = if col < decor_l {
                span_s.saturating_sub(1)
            } else if col >= decor_l + span_l {
                span_s + span_l
            } else {
                span_s + (col - decor_l).min(span_l)
            };
            cb(target, window, cx);
        });
    }

    if let Some(cb) = on_drag {
        let decor_l = decor_left;
        let span_l = span_len;
        let span_s = span_src;
        text_el = text_el.on_mouse_move(move |ev, window, cx| {
            if ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            let (rel_x, rel_y, width) = resolve_rel(&bounds_move, ev.position, text_left);
            let runs = shape_runs(window.text_style().font(), &pieces2);
            let col = char_index_at_point(
                &text_owned2,
                &runs,
                point(px(rel_x), px(rel_y)),
                px(width),
                font_size,
                line_height_px,
                window,
            );
            let target = if col < decor_l {
                span_s.saturating_sub(1)
            } else if col >= decor_l + span_l {
                span_s + span_l
            } else {
                span_s + (col - decor_l).min(span_l)
            };
            cb(target, window, cx);
        });
    }

    let styled = StyledText::new(display_text).with_highlights(highlights);
    text_el = text_el
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, _, _| {
                    bounds_cell.set(Some(bounds));
                },
            )
            .absolute()
            .size_full(),
        )
        .child(styled);

    div().w_full().min_w(px(0.)).cursor(CursorStyle::IBeam).child(text_el)
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
            content: vec![LocSpan { span: InlineSpan::Text("Heading 1".to_string()), src: 2 }],
            src: 0,
        });
        assert_eq!(blocks[1], MarkdownBlock::Heading {
            level: 2,
            content: vec![LocSpan { span: InlineSpan::Text("Heading 2".to_string()), src: 15 }],
            src: 12,
        });
        assert_eq!(blocks[2], MarkdownBlock::Paragraph {
            spans: vec![
                LocSpan {
                    span: InlineSpan::Text("####### Not Heading".to_string()),
                    src: 25,
                },
                LocSpan {
                    span: InlineSpan::Text(" ".to_string()),
                    src: 44,
                },
                LocSpan {
                    span: InlineSpan::Text("#NotHeading".to_string()),
                    src: 45,
                },
            ],
            src: 25,
        });
    }

    #[std::prelude::v1::test]
    fn test_single_line_and_multiline_math() {
        let blocks = parse_markdown("$$x^2 + y^2 = z^2$$\n\nText after");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], MarkdownBlock::MathBlock {
            math: "x^2 + y^2 = z^2".to_string(),
            src: 2,
        });
        assert_eq!(blocks[1], MarkdownBlock::Paragraph {
            spans: vec![LocSpan { span: InlineSpan::Text("Text after".to_string()), src: 21 }],
            src: 21,
        });

        let bracket_blocks = parse_markdown("\\[a + b = c\\]\nParagraph");
        assert_eq!(bracket_blocks.len(), 2);
        assert_eq!(bracket_blocks[0], MarkdownBlock::MathBlock {
            math: "a + b = c".to_string(),
            src: 2,
        });

        let multi = parse_markdown("$$\n\\int_0^1 f(x) dx\n$$\nTail");
        assert_eq!(multi.len(), 2);
        assert_eq!(multi[0], MarkdownBlock::MathBlock {
            math: "\\int_0^1 f(x) dx".to_string(),
            src: 3,
        });
    }

    #[std::prelude::v1::test]
    fn test_ordered_list_dot_guard() {
        let blocks = parse_markdown(". Not an ordered list\n1. Real item\n2. Second item");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], MarkdownBlock::Paragraph {
            spans: vec![LocSpan {
                span: InlineSpan::Text(". Not an ordered list".to_string()),
                src: 0,
            }],
            src: 0,
        });
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

    #[std::prelude::v1::test]
    fn test_bold_and_bullet_source_offsets() {
        // "**Hola** mundo" -> rendered "Hola mundo"; selection of source
        // chars 2..6 ("Hola") must highlight rendered chars 0..4.
        let blocks = parse_markdown("**Hola** mundo");
        let md = MarkdownBlock::Paragraph {
            spans: vec![
                LocSpan { span: InlineSpan::Bold("Hola".to_string()), src: 2 },
                LocSpan { span: InlineSpan::Text(" mundo".to_string()), src: 8 },
            ],
            src: 0,
        };
        assert_eq!(blocks[0], md);

        let listed = parse_markdown("- primer item\n- segundo item");
        match &listed[0] {
            MarkdownBlock::List { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2);
                assert_eq!(
                    items[0][0],
                    MarkdownBlock::Paragraph {
                        spans: vec![LocSpan {
                            span: InlineSpan::Text("primer item".to_string()),
                            src: 2,
                        }],
                        src: 2,
                    }
                );
                assert_eq!(
                    items[1][0],
                    MarkdownBlock::Paragraph {
                        spans: vec![LocSpan {
                            span: InlineSpan::Text("segundo item".to_string()),
                            src: 16,
                        }],
                        src: 16,
                    }
                );
            }
            _ => panic!("Expected list"),
        }
    }

    #[std::prelude::v1::test]
    fn test_code_block_line_offsets() {
        let blocks = parse_markdown("```rust\nfn a() {}\nlet b = 1;\n```");
        match &blocks[0] {
            MarkdownBlock::CodeBlock { language, code_lines } => {
                assert_eq!(language.as_deref(), Some("rust"));
                assert_eq!(code_lines.len(), 2);
                assert_eq!(code_lines[0].1, 8);  // "fn a() {}" starts after "```rust\n"
                assert_eq!(code_lines[1].1, 18); // "let b = 1;"
            }
            _ => panic!("Expected code block"),
        }
    }

    #[std::prelude::v1::test]
    fn test_src_range_mapping() {
        // Rendered "Hola mundo" from source "**Hola** mundo": chars 0..9 map to
        // src 2..6 then 8..14.
        let mut map = Vec::new();
        for k in 0..4 {
            map.push(2 + k);
        }
        for k in 0..6 {
            map.push(8 + k);
        }
        assert_eq!(src_range_to_rendered(&map, 2, 6), Some((0, 4)));
        assert_eq!(src_range_to_rendered(&map, 0, 6), Some((0, 4)));
        assert_eq!(src_range_to_rendered(&map, 8, 14), Some((4, 10)));
        assert_eq!(src_range_to_rendered(&map, 20, 30), None);
    }

    #[std::prelude::v1::test]
    fn test_mixed_content_span_maps_cover_rendered_text() {
        // A mixed paragraph: plain + bold + code + inline math. Every rendered
        // char must map back into its span's source range, so selecting any
        // range and copying the source slice reproduces exactly what is on
        // screen (markdown markers aside).
        let text = "Intro **bold** and `code` plus $x^2$ inline.";
        let blocks = parse_markdown(text);
        match &blocks[0] {
            MarkdownBlock::Paragraph { spans, .. } => {
                let mut covered = vec![false; text.chars().count()];
                for ls in spans {
                    let len = ls.span.text().chars().count();
                    for k in 0..len {
                        let pos = ls.src + k;
                        assert!(pos < covered.len(), "span maps outside source");
                        covered[pos] = true;
                    }
                }
                // Every source char that survives rendering (no ** ` $ markers)
                // is covered exactly once.
                for (idx, ch) in text.chars().enumerate() {
                    if matches!(ch, '*' | '`' | '$') {
                        assert!(!covered[idx], "marker char {ch} must not be mapped");
                    } else {
                        assert!(covered[idx], "source char '{ch}' at {idx} is unmapped");
                    }
                }
            }
            _ => panic!("expected paragraph"),
        }
    }

    #[std::prelude::v1::test]
    fn test_selection_highlights_are_sorted_and_tiling() {
        // Mixed spans: code (styled) + plain gap + bold (styled). A selection
        // spanning everything must produce sorted, non-overlapping ranges that
        // tile the full byte range — GPUI folds highlight entries strictly in
        // order, so unsorted entries style the wrong words.
        let text = "aa `bb` cc **dd** ee";
        let spans = parse_inline_located(text, 0);
        let mut full = String::new();
        let mut map: Vec<usize> = Vec::new();
        let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
        for ls in &spans {
            let t = ls.span.text();
            let b0 = full.len();
            for k in 0..t.chars().count() {
                map.push(ls.src + k);
            }
            full.push_str(t);
            let b1 = full.len();
            if !matches!(ls.span, InlineSpan::Text(_)) {
                highlights.push((b0..b1, HighlightStyle::default()));
            }
        }
        let total = text.chars().count();
        let (r0, r1) = src_range_to_rendered(&map, 0, total).expect("full range maps");
        let b0 = crate::ui::text_input::char_to_byte_idx(&full, r0);
        let b1 = crate::ui::text_input::char_to_byte_idx(&full, r1);
        let merged = apply_selection_highlights(highlights, b0..b1, HighlightStyle::default());

        let mut prev_end = 0;
        for (range, _) in &merged {
            assert!(range.start >= prev_end, "highlights must be sorted");
            assert!(range.start <= range.end, "no empty ranges");
            prev_end = range.end;
        }
        assert_eq!(prev_end, full.len(), "selection must tile the whole text");
    }

    #[std::prelude::v1::test]
    fn test_selection_overrides_span_highlights() {        let sel = HighlightStyle {
            background_color: Some(Hsla::default()),
            color: Some(Hsla::default()),
            ..Default::default()
        };
        // A code span (bytes 0..4) partially selected (bytes 2..6): the covered
        // part must adopt the selection style, the rest keeps the code style.
        let code_style = HighlightStyle {
            color: Some(Hsla::default()),
            background_color: Some(Hsla::default()),
            ..Default::default()
        };
        let highlights = vec![(0..4, code_style.clone())];
        let merged = apply_selection_highlights(highlights, 2..6, sel.clone());
        // (0..2 code style), (2..4 selection over code), (4..6 selection over
        // the plain-text tail beyond the span).
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].0, 0..2);
        assert_eq!(merged[0].1.background_color, code_style.background_color);
        assert_eq!(merged[1].0, 2..4);
        assert_eq!(merged[1].1.background_color, sel.background_color);
        assert_eq!(merged[2].0, 4..6);
        assert_eq!(merged[2].1.background_color, sel.background_color);

        // Selection covering plain text (no span highlights) still gets an entry.
        let merged = apply_selection_highlights(Vec::new(), 1..5, sel);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].0, 1..5);
    }
}
