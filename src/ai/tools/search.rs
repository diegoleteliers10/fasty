use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::ai::model::CancelToken;
use crate::ai::tool::{Tool, ToolCtx, ToolOutput};

const DEFAULT_MAX_RESULTS: usize = 50;
const MAX_SEARCH_DEPTH: usize = 8;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchInput {
    /// Relative or absolute path to search within (default: cwd)
    pub path: Option<String>,
    /// Content query/substring to grep inside files
    pub query: Option<String>,
    /// Filename pattern or extension filter (e.g. "*.rs" or "config")
    pub pattern: Option<String>,
    /// Maximum results to return (default: 50, max: 100)
    pub max_results: Option<usize>,
}

pub struct SearchTool;

impl SearchTool {
    pub fn new() -> Self {
        Self
    }
}

fn matches_pattern(file_name: &str, pattern: &str) -> bool {
    if pattern.starts_with('*') && pattern.ends_with('*') && pattern.len() > 2 {
        let sub = &pattern[1..pattern.len() - 1];
        file_name.contains(sub)
    } else if let Some(suffix) = pattern.strip_prefix('*') {
        file_name.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        file_name.starts_with(prefix)
    } else {
        file_name.contains(pattern)
    }
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".hg" | ".svn" | "dist" | "build" | ".next"
    )
}

fn walk_dir(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    files: &mut Vec<PathBuf>,
    cancel: &CancelToken,
) {
    if depth > max_depth || cancel.is_cancelled() {
        return;
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if cancel.is_cancelled() {
                return;
            }
            let path = entry.path();
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            if path.is_dir() {
                if !should_skip_dir(&name_str) {
                    walk_dir(&path, depth + 1, max_depth, files, cancel);
                }
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
}

impl Tool for SearchTool {
    fn name(&self) -> &'static str {
        "search"
    }

    fn description(&self) -> String {
        "Search codebase: grep text across files, find files matching name pattern, or list directory entries.".to_string()
    }

    fn schema(&self) -> Value {
        let schema = schemars::schema_for!(SearchInput);
        serde_json::to_value(&schema).unwrap_or_default()
    }

    fn run(
        &self,
        input: Value,
        ctx: &ToolCtx,
        cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolOutput> {
        let parsed: SearchInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return Err(ToolOutput::error(format!("Invalid arguments: {}", e))),
        };

        let base_path = match parsed.path {
            Some(ref p) if p.starts_with("~/") => dirs::home_dir()
                .map(|h| h.join(&p[2..]))
                .unwrap_or_else(|| PathBuf::from(p)),
            Some(ref p) => {
                let pb = PathBuf::from(p);
                if pb.is_absolute() {
                    pb
                } else {
                    ctx.cwd.join(pb)
                }
            }
            None => ctx.cwd.clone(),
        };

        let max_res = parsed.max_results.unwrap_or(DEFAULT_MAX_RESULTS).min(100);

        if !base_path.exists() {
            return Err(ToolOutput::error(format!(
                "Search path '{}' does not exist",
                base_path.display()
            )));
        }

        // Case 1: Just list directory entries if no query and no pattern and path is dir
        if parsed.query.is_none() && parsed.pattern.is_none() && base_path.is_dir() {
            let mut entries_out = Vec::new();
            if let Ok(entries) = fs::read_dir(&base_path) {
                for e in entries.flatten() {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    let suffix = if p.is_dir() { "/" } else { "" };
                    entries_out.push(format!("{}{}", name, suffix));
                    if entries_out.len() >= max_res {
                        break;
                    }
                }
            }
            entries_out.sort();
            return Ok(ToolOutput::success(format!(
                "Directory contents of '{}':\n{}",
                base_path.display(),
                entries_out.join("\n")
            )));
        }

        // Case 2: Walk directory to find files
        let mut all_files = Vec::new();
        if base_path.is_file() {
            all_files.push(base_path.clone());
        } else {
            walk_dir(&base_path, 0, MAX_SEARCH_DEPTH, &mut all_files, cancel);
        }

        // Filter by pattern if specified
        if let Some(ref pattern) = parsed.pattern {
            all_files.retain(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|name| matches_pattern(name, pattern))
                    .unwrap_or(false)
            });
        }

        // Case 3: If no query, return matched file list
        if parsed.query.is_none() {
            let display_files: Vec<String> = all_files
                .iter()
                .take(max_res)
                .map(|p| {
                    p.strip_prefix(&ctx.cwd)
                        .unwrap_or(p)
                        .display()
                        .to_string()
                })
                .collect();
            let total = all_files.len();
            return Ok(ToolOutput::success(format!(
                "Found {} matching files (showing {}):\n{}",
                total,
                display_files.len(),
                display_files.join("\n")
            )));
        }

        // Case 4: Grep query in files
        let query = parsed.query.as_ref().unwrap();
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for file_path in all_files {
            if cancel.is_cancelled() || results.len() >= max_res {
                break;
            }

            let file = match File::open(&file_path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let reader = BufReader::new(file);
            for (idx, line_res) in reader.lines().enumerate() {
                if cancel.is_cancelled() || results.len() >= max_res {
                    break;
                }
                let line = match line_res {
                    Ok(l) => l,
                    Err(_) => break, // Likely binary file, skip remainder
                };

                if line.to_lowercase().contains(&query_lower) {
                    let rel_path = file_path.strip_prefix(&ctx.cwd).unwrap_or(&file_path);
                    results.push(format!("{}:{}: {}", rel_path.display(), idx + 1, line.trim()));
                }
            }
        }

        if results.is_empty() {
            Ok(ToolOutput::success(format!("No matches found for '{}'.", query)))
        } else {
            let count = results.len();
            Ok(ToolOutput::success(format!(
                "Found {} matches:\n{}",
                count,
                results.join("\n")
            )))
        }
    }
}
