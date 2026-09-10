use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Cancelled,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data: String,
    },
    Document {
        media_type: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

impl From<&String> for MessageContent {
    fn from(s: &String) -> Self {
        Self::Text(s.clone())
    }
}

impl From<Vec<ContentPart>> for MessageContent {
    fn from(parts: Vec<ContentPart>) -> Self {
        Self::Parts(parts)
    }
}

impl MessageContent {
    pub fn as_text_lossy(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Text(t) => std::borrow::Cow::Borrowed(t),
            Self::Parts(parts) => {
                let mut out = String::new();
                for p in parts {
                    if let ContentPart::Text { text } = p {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(text);
                    }
                }
                std::borrow::Cow::Owned(out)
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(t) => t.is_empty(),
            Self::Parts(parts) => parts.is_empty(),
        }
    }
}

impl Default for MessageContent {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

impl Message {
    pub fn system(content: impl Into<MessageContent>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            is_error: false,
        }
    }

    pub fn user(content: impl Into<MessageContent>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            is_error: false,
        }
    }

    pub fn user_parts(parts: Vec<ContentPart>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
            is_error: false,
        }
    }

    pub fn assistant(content: impl Into<MessageContent>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            is_error: false,
        }
    }

    pub fn assistant_tool_calls(content: impl Into<MessageContent>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            is_error: false,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<MessageContent>, is_error: bool) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            is_error,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum CompletionEvent {
    Text(String),
    Thinking(String),
    ToolUse {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        args_delta: String,
    },
    Usage {
        input: u64,
        output: u64,
    },
    Stop(StopReason),
}

pub trait LanguageModel: Send + Sync {
    fn stream(&self, req: CompletionRequest, tx: async_channel::Sender<CompletionEvent>) -> CancelToken;
    fn default_model(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_content_serialization() {
        let text_msg = Message::user("Hello world");
        let json = serde_json::to_string(&text_msg).unwrap();
        assert!(json.contains(r#""content":"Hello world""#));

        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content.as_text_lossy(), "Hello world");

        let parts_msg = Message::user_parts(vec![
            ContentPart::Text { text: "Look at this:".to_string() },
            ContentPart::Image { media_type: "image/png".to_string(), data: "abc123base64".to_string() },
            ContentPart::Document {
                media_type: "application/pdf".to_string(),
                data: "pdfbase64data".to_string(),
                name: Some("doc.pdf".to_string()),
            },
        ]);
        let json_parts = serde_json::to_string(&parts_msg).unwrap();
        assert!(json_parts.contains(r#"{"type":"text","text":"Look at this:"}"#));
        assert!(json_parts.contains(r#"{"type":"image","media_type":"image/png","data":"abc123base64"}"#));
        assert!(json_parts.contains(r#"{"type":"document","media_type":"application/pdf","data":"pdfbase64data","name":"doc.pdf"}"#));

        let deserialized_parts: Message = serde_json::from_str(&json_parts).unwrap();
        assert_eq!(deserialized_parts.content.as_text_lossy(), "Look at this:");
    }
}

