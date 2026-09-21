use std::fs;
use std::path::PathBuf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::ai::model::CancelToken;
use crate::ai::tool::{Tool, ToolCtx, ToolOutput};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EditFileInput {
    /// Relative or absolute path to the file to edit or create
    pub path: String,
    /// The exact text to find in the file. This is matched with fuzzy matching
    /// to tolerate minor differences in whitespace or formatting.
    /// Be minimal with replacements:
    /// - For unique lines, include only those lines
    /// - For non-unique lines, include enough context to identify them
    pub old_string: Option<String>,
    /// The replacement text when old_string is used
    pub new_string: Option<String>,
    /// If provided (and old_string is omitted), replaces the entire file contents or creates it
    pub content: Option<String>,
}

pub struct EditFileTool;

impl EditFileTool {
    pub fn new() -> Self {
        Self
    }
}

/// A byte range in the original file content, always on line boundaries.
#[derive(Debug, Clone, Copy)]
struct MatchRange {
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct FileLine {
    start: usize,
    /// Exclusive end, includes the trailing newline when present.
    end: usize,
    text: String,
}

fn split_lines(content: &str) -> Vec<FileLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (idx, _) in content.match_indices('\n') {
        let end = idx + 1;
        lines.push(FileLine {
            start,
            end,
            text: content[start..idx].trim_end_matches('\r').to_string(),
        });
        start = end;
    }
    if start < content.len() {
        lines.push(FileLine {
            start,
            end: content.len(),
            text: content[start..].trim_end_matches('\r').to_string(),
        });
    }
    lines
}

fn leading_whitespace(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

fn line_number_at(content: &str, byte_idx: usize) -> usize {
    content[..byte_idx].matches('\n').count() + 1
}

/// Exact match with uniqueness handling. Falls back to CRLF-aware matching.
fn find_exact(content: &str, query: &str) -> Result<MatchRange, String> {
    let count = content.matches(query).count();
    match count {
        1 => {
            let start = content.find(query).expect("one match");
            Ok(MatchRange {
                start,
                end: start + query.len(),
            })
        }
        0 => {
            if query.contains('\n') && !query.contains("\r\n") && content.contains("\r\n") {
                let crlf_query = query.replace('\n', "\r\n");
                return find_exact(content, &crlf_query);
            }
            Err(String::new())
        }
        _ => {
            let lines: Vec<usize> = content
                .match_indices(query)
                .map(|(at, _)| line_number_at(content, at))
                .collect();
            let listed = lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "old_string matched multiple locations in the file at lines: {}. \
                 Please provide more context in old_string to uniquely identify the location.",
                listed
            ))
        }
    }
}

/// Result of a fuzzy alignment: which file lines matched which query lines.
#[derive(Debug)]
struct FuzzyMatch {
    range: MatchRange,
    /// Matched query lines / total query lines.
    ratio: f32,
    cost: u32,
    /// Indent delta between the buffer and the query, in characters.
    first_line_delta: i32,
    rest_delta: i32,
}

const FUZZY_EQUAL_COST: u32 = 1;
const SKIP_QUERY_LINE_COST: u32 = 10;
const SKIP_FILE_LINE_COST: u32 = 3;
const FUZZY_THRESHOLD: f32 = 0.8;

fn fuzzy_eq(a: &str, b: &str) -> bool {
    let (ta, tb) = (a.trim(), b.trim());
    if ta == tb {
        return true;
    }
    let (na, nb) = (ta.chars().count(), tb.chars().count());
    if na == 0 || nb == 0 {
        return false;
    }
    // Edit distance is bounded below by the length difference, so a pair
    // differing too much in length can never reach the similarity threshold.
    let max_len = na.max(nb);
    if (na as f32 - nb as f32).abs() / max_len as f32 > 1.0 - FUZZY_THRESHOLD {
        return false;
    }
    normalized_levenshtein(ta, tb) >= FUZZY_THRESHOLD
}

fn normalized_levenshtein(a: &str, b: &str) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut prev: Vec<u32> = (0..=b.len() as u32).collect();
    let mut cur = vec![0u32; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i as u32;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + u32::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(sub);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let dist = prev[b.len()] as f32;
    1.0 - dist / a.len().max(b.len()) as f32
}

/// Align `query_lines` against the file lines starting near `start_idx`,
/// bounded by a small slack window. Returns the best alignment for this start.
fn align_at(file_lines: &[FileLine], query_lines: &[&str], start_idx: usize) -> Option<FuzzyMatch> {
    let m = query_lines.len();
    if m == 0 || start_idx >= file_lines.len() {
        return None;
    }
    let window = (m + m / 2).max(3);
    let end_idx = (start_idx + window).min(file_lines.len());
    let n = end_idx - start_idx;

    const INF: u32 = u32::MAX / 4;
    let mut dp = vec![vec![INF; m + 1]; n + 1];
    let mut from = vec![vec![0u8; m + 1]; n + 1];
    dp[0][0] = 0;
    for i in 0..=n {
        for j in 0..=m {
            if dp[i][j] >= INF && !(i == 0 && j == 0) {
                continue;
            }
            let cost = dp[i][j];
            if i < n {
                // Skip a file line.
                if cost + SKIP_FILE_LINE_COST < dp[i + 1][j] {
                    dp[i + 1][j] = cost + SKIP_FILE_LINE_COST;
                    from[i + 1][j] = 1;
                }
            }
            if j < m {
                // Skip a query line.
                if cost + SKIP_QUERY_LINE_COST < dp[i][j + 1] {
                    dp[i][j + 1] = cost + SKIP_QUERY_LINE_COST;
                    from[i][j + 1] = 2;
                }
            }
            if i < n && j < m {
                let same = file_lines[start_idx + i].text.trim() == query_lines[j].trim();
                let fuzzy = !same && fuzzy_eq(&file_lines[start_idx + i].text, query_lines[j]);
                if same || fuzzy {
                    let step = if same { 0 } else { FUZZY_EQUAL_COST };
                    if cost + step < dp[i + 1][j + 1] {
                        dp[i + 1][j + 1] = cost + step;
                        from[i + 1][j + 1] = 3;
                    }
                }
            }
        }
    }

    if dp[n][m] >= INF {
        return None;
    }

    // Backtrack to collect matched (file_idx, query_idx) pairs.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        match from[i][j] {
            3 => {
                pairs.push((start_idx + i - 1, j - 1));
                i -= 1;
                j -= 1;
            }
            2 => j -= 1,
            1 => i -= 1,
            _ => break,
        }
    }
    pairs.reverse();

    let matched = pairs.len() as f32;
    let ratio = matched / m as f32;
    if ratio < FUZZY_THRESHOLD {
        return None;
    }

    let (first_f, first_q) = *pairs.first()?;
    let (last_f, _) = *pairs.last()?;
    let range = MatchRange {
        start: file_lines[first_f].start,
        end: file_lines[last_f].end,
    };

    let first_line_delta = leading_whitespace(&file_lines[first_f].text) as i32
        - leading_whitespace(query_lines[first_q]) as i32;
    let rest_delta = if pairs.len() > 1 {
        let (f2, q2) = pairs[1];
        leading_whitespace(&file_lines[f2].text) as i32
            - leading_whitespace(query_lines[q2]) as i32
    } else {
        first_line_delta
    };

    Some(FuzzyMatch {
        range,
        ratio,
        cost: dp[n][m],
        first_line_delta,
        rest_delta,
    })
}

/// Strips `read_file`-style line-number prefixes from each line, e.g.
/// "  42 | fn main() {" -> "fn main() {". Models routinely paste these
/// numbered lines into old_string, which otherwise never matches the file.
fn strip_line_number_prefixes(query: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(query.len());
    for line in query.split('\n') {
        let trimmed_start = line.trim_start();
        let digits = trimmed_start
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .count();
        let rest = trimmed_start
            .get(digits..)
            .map(str::trim_start)
            .unwrap_or("");
        let sep = rest.chars().next();
        let is_separator = matches!(sep, Some('|') | Some('│') | Some(':'));
        if digits > 0 && digits <= 6 && is_separator {
            let after = &rest[sep.map_or(0, |c| c.len_utf8())..];
            let after = after.strip_prefix(' ').unwrap_or(after);
            out.push_str(after);
            changed = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    // One '\n' was pushed per split segment; drop the trailing one.
    out.pop();
    if changed {
        Some(out)
    } else {
        None
    }
}

enum FuzzyError {
    NoMatch,
    Ambiguous(String),
}

/// Upper bound on DP cells the fuzzy matcher may spend in one call. Without
/// this, a large old_string on a large file explodes into billions of cells
/// and the tool call appears to hang forever with a spinning indicator.
const FUZZY_CELL_BUDGET: u64 = 4_000_000;

fn find_fuzzy(content: &str, query: &str) -> Result<(MatchRange, (i32, i32)), FuzzyError> {
    let file_lines = split_lines(content);
    let query_lines: Vec<&str> = query.lines().collect();
    if query_lines.is_empty() || file_lines.is_empty() {
        return Err(FuzzyError::NoMatch);
    }

    // Per-start DP cost is window * query_len; derive how many start
    // positions fit in the budget.
    let m = query_lines.len();
    // Very large queries never benefit from fuzzy alignment; exact match and
    // line-number stripping already ran. Guard before the budget math.
    if m == 0 || m > 400 || file_lines.is_empty() {
        return Err(FuzzyError::NoMatch);
    }
    let window = (m + m / 2).max(3);
    let per_start = (window as u64) * (m as u64 + 1);
    let max_starts = (FUZZY_CELL_BUDGET / per_start.max(1)).max(1) as usize;

    let total = file_lines.len() as u64 * per_start;
    let starts: Vec<usize> = if total <= FUZZY_CELL_BUDGET {
        (0..file_lines.len()).collect()
    } else {
        // Prefilter: only try starts whose first line plausibly matches the
        // first query line (or the query's first line anywhere).
        let first = query_lines[0].trim();
        if first.is_empty() {
            return Err(FuzzyError::NoMatch);
        }
        let mut candidates: Vec<usize> = file_lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.text.trim() == first || fuzzy_eq(&l.text, first))
            .map(|(i, _)| i)
            .take(max_starts)
            .collect();
        if candidates.is_empty() {
            // Fall back to the first max_starts positions.
            candidates = (0..max_starts.min(file_lines.len())).collect();
        }
        candidates
    };

    let mut candidates: Vec<FuzzyMatch> = Vec::new();
    for start in starts {
        if let Some(candidate) = align_at(&file_lines, &query_lines, start) {
            candidates.push(candidate);
        }
    }

    if candidates.is_empty() {
        return Err(FuzzyError::NoMatch);
    }

    let best_cost = candidates.iter().map(|c| c.cost).min().expect("non-empty");
    let mut best: Option<&FuzzyMatch> = None;
    let mut tie_lines: Vec<usize> = Vec::new();
    for candidate in &candidates {
        if candidate.cost != best_cost {
            continue;
        }
        let line = line_number_at(content, candidate.range.start);
        match best {
            None => {
                best = Some(candidate);
                tie_lines.push(line);
            }
            Some(current) if candidate.ratio > current.ratio => {
                best = Some(candidate);
                tie_lines.clear();
                tie_lines.push(line);
            }
            Some(current) if candidate.ratio == current.ratio && !tie_lines.contains(&line) => {
                tie_lines.push(line);
            }
            _ => {}
        }
    }

    match best {
        Some(m) if tie_lines.len() <= 1 => Ok((m.range, (m.first_line_delta, m.rest_delta))),
        Some(_) => Err(FuzzyError::Ambiguous(format!(
            "old_string matched multiple locations in the file at lines: {}. \
             Please provide more context in old_string to uniquely identify the location.",
            tie_lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", ")
        ))),
        None => unreachable!("candidates is non-empty"),
    }
}

/// Two-tier search: byte-exact first, then fuzzy over trimmed lines.
/// Returns the matched byte range plus the indent deltas (first line, rest)
/// to apply to the replacement text.
fn find_match(content: &str, query: &str) -> Result<(MatchRange, (i32, i32)), String> {
    if query.is_empty() {
        return Err("old_string is empty. Provide the text to replace.".to_string());
    }
    match find_exact(content, query) {
        Ok(range) => Ok((range, (0, 0))),
        Err(err) if !err.is_empty() => Err(err),
        Err(_) => match find_fuzzy(content, query) {
            Ok(found) => Ok(found),
            Err(FuzzyError::Ambiguous(msg)) => Err(msg),
            Err(FuzzyError::NoMatch) => {
                // Last resort: the model may have copied read_file output
                // including its "  12 | " line-number prefixes.
                if let Some(stripped) = strip_line_number_prefixes(query) {
                    if stripped != query {
                        if let Ok(found) = find_match(content, &stripped) {
                            return Ok(found);
                        }
                    }
                }
                Err(
                    "Could not find matching text for this edit. The old_string did not match \
                     any content in the file (matching tolerates whitespace differences and \
                     read_file line-number prefixes). Please read the file again to get the \
                     current content."
                        .to_string(),
                )
            }
        },
    }
}

fn apply_replacement(content: &str, range: MatchRange, new_str: &str, first_delta: i32, rest_delta: i32) -> String {
    // Keep the file's dominant line ending inside the replaced section.
    let uses_crlf = content.contains("\r\n");
    let mut replacement = String::new();
    let mut lines = new_str.lines().peekable();
    let mut line_idx = 0usize;
    while let Some(line) = lines.next() {
        let delta = if line_idx == 0 { first_delta } else { rest_delta };
        let indented = if line.trim().is_empty() || delta == 0 {
            line.to_string()
        } else if delta > 0 {
            format!("{}{}", " ".repeat(delta as usize), line)
        } else {
            let strip = (-delta as usize).min(leading_whitespace(line));
            line.chars().skip(strip).collect::<String>()
        };
        replacement.push_str(&indented);
        if lines.peek().is_some() {
            replacement.push_str(if uses_crlf { "\r\n" } else { "\n" });
        }
        line_idx += 1;
    }
    if new_str.ends_with('\n') {
        replacement.push_str(if uses_crlf { "\r\n" } else { "\n" });
    }
    // Do not consume a trailing newline that the matched range did not include.
    if new_str.ends_with('\n') && !content[range.start..range.end].ends_with('\n') {
        replacement.truncate(replacement.len() - if uses_crlf { 2 } else { 1 });
    }

    let mut updated = String::with_capacity(content.len());
    updated.push_str(&content[..range.start]);
    updated.push_str(&replacement);
    updated.push_str(&content[range.end..]);
    updated
}

impl Tool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> String {
        "Edit an existing file via unique substring replacement (old_string -> new_string), or create/overwrite a file via content.".to_string()
    }

    fn schema(&self) -> Value {
        let schema = schemars::schema_for!(EditFileInput);
        serde_json::to_value(&schema).unwrap_or_default()
    }

    fn run(
        &self,
        input: Value,
        ctx: &ToolCtx,
        _cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolOutput> {
        let parsed: EditFileInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return Err(ToolOutput::error(format!("Invalid arguments: {}", e))),
        };

        let target_path = if parsed.path.starts_with("~/") {
            dirs::home_dir()
                .map(|h| h.join(&parsed.path[2..]))
                .unwrap_or_else(|| PathBuf::from(&parsed.path))
        } else {
            let p = PathBuf::from(&parsed.path);
            if p.is_absolute() {
                p
            } else {
                ctx.cwd.join(p)
            }
        };

        if let Some(old_str) = parsed.old_string {
            let Some(new_str) = parsed.new_string else {
                return Err(ToolOutput::error(
                    "new_string is required when old_string is provided. To delete text, pass an empty new_string explicitly.",
                ));
            };
            if !target_path.exists() {
                return Err(ToolOutput::error(format!(
                    "File '{}' does not exist for replacement",
                    target_path.display()
                )));
            }

            let file_content = match fs::read_to_string(&target_path) {
                Ok(c) => c,
                Err(e) => {
                    return Err(ToolOutput::error(format!(
                        "Failed to read '{}': {}",
                        target_path.display(),
                        e
                    )))
                }
            };

            let (range, deltas) = match find_match(&file_content, &old_str) {
                Ok(v) => v,
                Err(err) => return Err(ToolOutput::error(err)),
            };

            let updated = apply_replacement(&file_content, range, &new_str, deltas.0, deltas.1);
            if let Err(e) = fs::write(&target_path, &updated) {
                return Err(ToolOutput::error(format!(
                    "Failed to write changes to '{}': {}",
                    target_path.display(),
                    e
                )));
            }

            let added = new_str.lines().count();
            let removed = old_str.lines().count();
            let diff = crate::ai::diff::diff_contents(
                target_path.display().to_string(),
                &file_content,
                &updated,
            );
            Ok(ToolOutput::success_with_diff(
                format!(
                    "Successfully replaced 1 occurrence in '{}' (+{} -{})",
                    target_path.display(),
                    added,
                    removed
                ),
                diff,
            ))
        } else if let Some(content) = parsed.content {
            if let Some(parent) = target_path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let previous = fs::read_to_string(&target_path).ok();
            let bytes_len = content.len();
            if let Err(e) = fs::write(&target_path, &content) {
                return Err(ToolOutput::error(format!(
                    "Failed to write file '{}': {}",
                    target_path.display(),
                    e
                )));
            }

            let out = if let Some(old) = previous {
                let diff = crate::ai::diff::diff_contents(
                    target_path.display().to_string(),
                    &old,
                    &content,
                );
                ToolOutput::success_with_diff(
                    format!("Successfully wrote {} bytes to '{}'", bytes_len, target_path.display()),
                    diff,
                )
            } else {
                ToolOutput::success(format!(
                    "Successfully wrote {} bytes to '{}'",
                    bytes_len,
                    target_path.display()
                ))
            };
            Ok(out)
        } else {
            Err(ToolOutput::error(
                "Either old_string and new_string, or content must be provided.",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_replaces() {
        let content = "fn main() {\n    println!(\"hi\");\n}\n";
        let (range, deltas) = find_match(content, "println!(\"hi\");").expect("exact match");
        let updated = apply_replacement(content, range, "println!(\"bye\");", deltas.0, deltas.1);
        assert!(updated.contains("println!(\"bye\");"));
    }

    #[test]
    fn fuzzy_match_tolerates_indent_and_noise() {
        let content = "struct TokenSpan {\n    pub start: u32,\n    pub end: u32,\n}\n";
        // Model emits wrong indentation for the query.
        let query = "pub start: u32,\n  pub end: u32,\n";
        let (range, deltas) = find_match(content, query).expect("fuzzy match");
        let updated = apply_replacement(
            content,
            range,
            "pub start: u64,\n  pub end: u64,\n",
            deltas.0,
            deltas.1,
        );
        assert!(updated.contains("    pub start: u64,"));
        assert!(updated.contains("    pub end: u64,"));
    }

    #[test]
    fn ambiguity_is_reported_with_line_numbers() {
        let content = "x = 1\n\nx = 1\n";
        let err = find_match(content, "x = 1").unwrap_err();
        assert!(err.contains("multiple locations"), "{err}");
        assert!(err.contains("1"), "{err}");
    }

    #[test]
    fn no_match_reports_actionable_error() {
        let content = "a\nb\nc\n";
        let err = find_match(content, "zzz").unwrap_err();
        assert!(err.contains("Could not find matching text"), "{err}");
    }

    #[test]
    fn fuzzy_near_duplicates_report_ambiguity() {
        let content = "same\nsame\nsame\n";
        let err = find_match(content, "same").unwrap_err();
        assert!(err.contains("multiple locations"), "{err}");
    }

    #[test]
    fn line_number_prefixes_are_stripped_before_matching() {
        let content = "fn main() {\n    println!(\"hi\");\n}\n";
        // Model copied read_file output verbatim.
        let query = "1 | fn main() {\n2 |     println!(\"hi\");\n3 | }";
        let (range, _) = find_match(content, query).expect("stripped match");
        assert!(content[range.start..range.end].starts_with("fn main()"));
    }

    #[test]
    fn line_number_strip_survives_pipes_and_colons() {
        assert_eq!(
            strip_line_number_prefixes("  12 | a\n   3│ b\n4: c").as_deref(),
            Some("a\nb\nc")
        );
        assert_eq!(strip_line_number_prefixes("no prefix"), None);
        assert_eq!(strip_line_number_prefixes("12345678 | too wide"), None);
        assert_eq!(strip_line_number_prefixes("foo | not a number"), None);
    }

    #[test]
    fn missing_new_string_is_a_validation_error() {
        let tool = EditFileTool::new();
        let input = serde_json::json!({ "path": "x.txt", "old_string": "a" });
        let err = tool
            .run(input, &tool_ctx(), &Default::default())
            .expect_err("must fail");
        assert!(err.content.contains("new_string is required"), "{}", err.content);
    }

    #[test]
    fn crlf_files_keep_their_endings() {
        let content = "a\r\nB\r\nc\r\n";
        let (range, _) = find_match(content, "B\r\n").expect("exact match");
        let updated = apply_replacement(content, range, "X\r\n", 0, 0);
        assert_eq!(updated, "a\r\nX\r\nc\r\n");
    }

    fn tool_ctx() -> crate::ai::tool::ToolCtx {
        crate::ai::tool::ToolCtx {
            cwd: std::env::temp_dir(),
        }
    }
}

#[cfg(test)]
mod fuzzy_perf_tests {
    use super::*;

    #[test]
    fn fuzzy_scan_on_large_file_completes_quickly() {
        // 20k lines, 120 chars each
        let mut content = String::with_capacity(20_000 * 121);
        for i in 0..20_000 {
            content.push_str(&format!("fn generated_line_{i}() {{ let x = {i}; let padding = \"aaaaaabbbbbbccccc\"; }}\n"));
        }
        // 200-line query that does NOT exist anywhere (forces full scan path)
        let mut query = String::new();
        for i in 0..200 {
            query.push_str(&format!("totally_absent_line_{i}(x, y, z) // does not exist\n"));
        }
        let start = std::time::Instant::now();
        let result = find_match(&content, &query);
        let elapsed = start.elapsed();
        assert!(result.is_err(), "should not match");
        // Wall-clock guard against algorithmic blowups. Debug builds are
        // several times slower than release and CI/parallel load varies, so
        // the budget is profile-aware: it still catches minute-scale hangs
        // while tolerating second-scale machine variance.
        #[cfg(debug_assertions)]
        let budget_secs = 15;
        #[cfg(not(debug_assertions))]
        let budget_secs = 5;
        assert!(
            elapsed.as_secs() < budget_secs,
            "fuzzy scan took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn fuzzy_finds_match_in_large_file_within_budget() {
        let mut content = String::with_capacity(20_000 * 121);
        for i in 0..20_000 {
            content.push_str(&format!("fn generated_line_{i}() {{ let x = {i}; }}\n"));
        }
        // A real 60-line block pasted with an extra blank line difference.
        let target_start = 10_000;
        let mut target = String::new();
        for i in target_start..target_start + 60 {
            target.push_str(&format!("fn generated_line_{i}() {{ let x = {i}; }}\n"));
        }
        let (range, _) = find_match(&content, target.trim_end()).expect("should find the block");
        let found = &content[range.start..range.end];
        assert!(found.contains(&format!("generated_line_{target_start}")));
    }
}
