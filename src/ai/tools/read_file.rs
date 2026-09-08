use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::ai::model::CancelToken;
use crate::ai::tool::{Tool, ToolCtx, ToolOutput};

const MAX_LINES: usize = 1000;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReadFileInput {
    /// Relative or absolute path to the file to read
    pub path: String,
    /// 1-based start line number to begin reading (default: 1)
    pub start_line: Option<usize>,
    /// Number of lines to read (default: 500, max: 1000)
    pub line_count: Option<usize>,
}

pub struct ReadFileTool;

impl ReadFileTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> String {
        "Read contents of a file with 1-based line numbers and optional range slice.".to_string()
    }

    fn schema(&self) -> Value {
        let schema = schemars::schema_for!(ReadFileInput);
        serde_json::to_value(&schema).unwrap_or_default()
    }

    fn run(
        &self,
        input: Value,
        ctx: &ToolCtx,
        _cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolOutput> {
        let parsed: ReadFileInput = match serde_json::from_value(input) {
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

        if !target_path.exists() {
            return Err(ToolOutput::error(format!(
                "File '{}' does not exist",
                target_path.display()
            )));
        }

        if target_path.is_dir() {
            return Err(ToolOutput::error(format!(
                "Path '{}' is a directory, not a file",
                target_path.display()
            )));
        }

        let file = match File::open(&target_path) {
            Ok(f) => f,
            Err(e) => {
                return Err(ToolOutput::error(format!(
                    "Cannot open '{}': {}",
                    target_path.display(),
                    e
                )))
            }
        };

        let start_line = parsed.start_line.unwrap_or(1).max(1);
        let max_count = parsed.line_count.unwrap_or(500).min(MAX_LINES);

        let reader = BufReader::new(file);
        let mut lines_out = Vec::new();
        let mut current_line_num = 1;

        for line_res in reader.lines() {
            let line = match line_res {
                Ok(l) => l,
                Err(e) => {
                    return Err(ToolOutput::error(format!(
                        "Error reading file (possible binary): {}",
                        e
                    )))
                }
            };

            if current_line_num >= start_line && lines_out.len() < max_count {
                lines_out.push(format!("{:4} | {}", current_line_num, line));
            }

            current_line_num += 1;
            if lines_out.len() >= max_count {
                break;
            }
        }

        let total_scanned = current_line_num.saturating_sub(1);
        let header = format!(
            "--- {} (lines {}-{} of {}) ---\n",
            target_path.display(),
            start_line,
            start_line + lines_out.len().saturating_sub(1),
            total_scanned
        );

        Ok(ToolOutput::success(format!("{}{}", header, lines_out.join("\n"))))
    }
}
