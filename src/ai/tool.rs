use std::path::PathBuf;
use serde_json::Value;

use crate::ai::model::CancelToken;

#[derive(Debug, Clone)]
pub struct ToolCtx {
    pub cwd: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
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
