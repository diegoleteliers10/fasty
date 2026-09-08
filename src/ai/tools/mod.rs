pub mod edit_file;
pub mod read_file;
pub mod run_command;
pub mod search;

pub use edit_file::EditFileTool;
pub use read_file::ReadFileTool;
pub use run_command::RunCommandTool;
pub use search::SearchTool;

use std::sync::Arc;
use crate::ai::tool::Tool;

pub fn default_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(RunCommandTool::new()),
        Arc::new(ReadFileTool::new()),
        Arc::new(EditFileTool::new()),
        Arc::new(SearchTool::new()),
    ]
}
