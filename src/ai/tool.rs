use std::path::PathBuf;
use serde_json::Value;

use crate::ai::diff::FileDiff;
use crate::ai::model::CancelToken;

#[derive(Debug, Clone)]
pub struct ToolCtx {
    pub cwd: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    /// Structured file diff for edit tools, rendered as a review card.
    pub diff: Option<FileDiff>,
}

impl ToolOutput {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            diff: None,
        }
    }

    pub fn success_with_diff(content: impl Into<String>, diff: FileDiff) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            diff: Some(diff),
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            diff: None,
        }
    }
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> String;
    fn schema(&self) -> Value;
    fn run(
        &self,
        input: Value,
        ctx: &ToolCtx,
        cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolOutput>;
}
