/// Line-based diff used by the AI edit previews. Splitting around a common
/// prefix/suffix keeps the common case (a targeted replacement inside a file)
/// cheap and the result readable.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    /// 1-based line number in the ORIGINAL file (context/del rows).
    pub old_line: Option<usize>,
    /// 1-based line number in the UPDATED file (context/add rows).
    pub new_line: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
    /// True when the diff was truncated for display.
    pub truncated: bool,
}

const CONTEXT_LINES: usize = 2;
const MAX_ROWS: usize = 400;

/// Strips a baked-in `read_file`-style line-number prefix ("  12 | code")
/// for DISPLAY purposes. Only `|`/`│` separators are stripped — never `:`,
/// so timestamps like "12:30" are left untouched. Also strips "Línea N:"
/// style prefixes agents write into files. Returns the original slice when
/// there is no such prefix.
pub fn strip_baked_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits <= 6 {
        let rest = trimmed.get(digits..).map(str::trim_start).unwrap_or("");
        let sep = rest.chars().next();
        if matches!(sep, Some('|') | Some('│')) {
            let after = &rest[sep.map_or(0, |c| c.len_utf8())..];
            return after.strip_prefix(' ').unwrap_or(after);
        }
    }
    strip_linea_prefix(line).unwrap_or(line)
}

/// Strips a leading "Línea 12:" / "Linea 3 -" / "linea 4" style prefix for
/// DISPLAY purposes. Returns None when the line has no such prefix, or when
/// nothing would remain after it.
fn strip_linea_prefix(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    let head: String = chars.by_ref().take(5).collect::<String>().to_lowercase();
    if head != "linea" && head != "línea" {
        return None;
    }
    // Byte offset right after the 5-char word ("línea" is 6 bytes in UTF-8).
    let word_bytes = trimmed.len() - chars.as_str().len();
    let rest = trimmed.get(word_bytes..)?.trim_start();
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 6 {
        return None;
    }
    let rest = rest.get(digits..)?.trim_start();
    let rest = match rest.chars().next() {
        Some(c) if matches!(c, ':' | '-' | '–' | '.') => &rest[c.len_utf8()..],
        _ => rest,
    };
    let out = rest.strip_prefix(' ').unwrap_or(rest);
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// Diffs two file contents and returns a compact row list with context lines.
pub fn diff_contents(path: impl Into<String>, old: &str, new: &str) -> FileDiff {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let prefix = old_lines
        .iter()
        .zip(new_lines.iter())
        .take_while(|(a, b)| a == b)
        .count();
    // Compare from the ends so an insertion in the middle keeps the shared
    // tail aligned.
    let suffix = old_lines
        .iter()
        .rev()
        .zip(new_lines.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(old_lines.len() - prefix)
        .min(new_lines.len() - prefix);

    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];

    let mut lines: Vec<DiffLine> = Vec::new();
    let ctx_start = prefix.saturating_sub(CONTEXT_LINES);
    for (k, line) in old_lines[ctx_start..prefix].iter().enumerate() {
        let no = ctx_start + k + 1;
        lines.push(DiffLine {
            kind: DiffLineKind::Context,
            text: (*line).to_string(),
            old_line: Some(no),
            new_line: Some(no),
        });
    }
    for (k, line) in old_mid.iter().enumerate() {
        lines.push(DiffLine {
            kind: DiffLineKind::Del,
            text: (*line).to_string(),
            old_line: Some(prefix + k + 1),
            new_line: None,
        });
    }
    for (k, line) in new_mid.iter().enumerate() {
        lines.push(DiffLine {
            kind: DiffLineKind::Add,
            text: (*line).to_string(),
            old_line: None,
            new_line: Some(prefix + k + 1),
        });
    }
    let ctx_end = (new_lines.len() - suffix + CONTEXT_LINES).min(new_lines.len());
    for (k, line) in new_lines[new_lines.len() - suffix..ctx_end].iter().enumerate() {
        let no = new_lines.len() - suffix + k + 1;
        lines.push(DiffLine {
            kind: DiffLineKind::Context,
            text: (*line).to_string(),
            old_line: Some(no),
            new_line: Some(no),
        });
    }

    let truncated = lines.len() > MAX_ROWS;
    if truncated {
        lines.truncate(MAX_ROWS);
    }

    FileDiff {
        path: path.into(),
        added: new_mid.len(),
        removed: old_mid.len(),
        lines,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_reports_adds_and_dels() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nb\nX\nY\nd\ne\n";
        let diff = diff_contents("f.rs", old, new);
        assert_eq!(diff.added, 2);
        assert_eq!(diff.removed, 1);
        assert_eq!(diff.path, "f.rs");
        assert!(!diff.truncated);
        assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Context));
        assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Add));
        assert!(diff.lines.iter().any(|l| l.kind == DiffLineKind::Del));
    }

    #[test]
    fn identical_contents_yield_context_only() {
        let diff = diff_contents("f.rs", "a\nb\n", "a\nb\n");
        assert_eq!(diff.added, 0);
        assert_eq!(diff.removed, 0);
        assert!(diff.lines.iter().all(|l| l.kind == DiffLineKind::Context));
    }

    #[test]
    fn diff_lines_carry_real_line_numbers() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nb\nX\nY\nd\ne\n";
        let diff = diff_contents("f.rs", old, new);
        // Context rows share old/new numbers; del rows old only; add rows new only.
        let ctx: Vec<_> = diff
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Context)
            .collect();
        assert_eq!(ctx[0].old_line, Some(1));
        assert_eq!(ctx[0].new_line, Some(1));
        let del: Vec<_> = diff
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Del)
            .collect();
        assert_eq!(del.len(), 1);
        assert_eq!(del[0].old_line, Some(3));
        assert_eq!(del[0].new_line, None);
        let add: Vec<_> = diff
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Add)
            .collect();
        assert_eq!(add.len(), 2);
        assert_eq!(add[0].new_line, Some(3));
        assert_eq!(add[0].old_line, None);
        assert_eq!(add[1].new_line, Some(4));
    }

    #[test]
    fn baked_prefix_strip_keeps_timestamps() {
        assert_eq!(strip_baked_prefix("  12 | code"), "code");
        assert_eq!(strip_baked_prefix("3│x"), "x");
        assert_eq!(strip_baked_prefix("12:30 meeting"), "12:30 meeting");
        assert_eq!(strip_baked_prefix("2024: hello"), "2024: hello");
        assert_eq!(strip_baked_prefix("plain"), "plain");
        assert_eq!(strip_baked_prefix("12345678 | wide"), "12345678 | wide");
        assert_eq!(strip_baked_prefix("Línea 1: texto largo"), "texto largo");
        assert_eq!(strip_baked_prefix("Linea 12 - otro"), "otro");
        assert_eq!(strip_baked_prefix("linea 3 texto"), "texto");
        assert_eq!(strip_baked_prefix("LINEA 7. punto"), "punto");
        assert_eq!(strip_baked_prefix("linea de código"), "linea de código");
        assert_eq!(strip_baked_prefix("Línea 12"), "Línea 12");
    }
}
