pub mod agent;
pub mod config;
pub mod model;
pub mod permissions;
pub mod prompt;
pub mod providers;
pub mod sse;
pub mod tool;
pub mod tools;

pub use agent::{Agent, AgentEvent, CliPermissionHandler, PermissionHandler};
pub use config::{AiConfig, PermissionMode, ProviderConfig};
pub use model::{
    CancelToken, CompletionEvent, CompletionRequest, LanguageModel, Message, Role, StopReason,
    ToolCall, ToolDefinition,
};
pub use permissions::{PermissionChecker, PermissionDecision};
pub use prompt::system_prompt;
pub use providers::{create_model_from_config, detect_installed_ollama_models, test_provider_connection};
pub use tool::{Tool, ToolCtx, ToolOutput};
pub use tools::default_tools;
