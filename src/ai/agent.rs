use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;

use crate::ai::model::{
    CancelToken, CompletionEvent, CompletionRequest, LanguageModel, Message, StopReason, ToolCall,
};
use crate::ai::permissions::{PermissionChecker, PermissionDecision};
use crate::ai::tool::{Tool, ToolCtx};

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolStart {
        id: String,
        name: String,
        args: String,
    },
    ToolEnd {
        id: String,
        name: String,
        output: String,
        is_error: bool,
        diff: Option<crate::ai::diff::FileDiff>,
    },
    TurnEnd,
    Usage {
        input: u64,
        output: u64,
    },
    Error(String),
}

pub trait PermissionHandler: Send + Sync {
    fn ask_permission(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        input_summary: &str,
    ) -> PermissionDecision;
}

pub struct CliPermissionHandler {
    checker: Arc<PermissionChecker>,
}

impl CliPermissionHandler {
    pub fn new(checker: Arc<PermissionChecker>) -> Self {
        Self { checker }
    }
}

impl PermissionHandler for CliPermissionHandler {
    fn ask_permission(
        &self,
        _tool_call_id: &str,
        tool_name: &str,
        input_summary: &str,
    ) -> PermissionDecision {
        print!(
            "\n[fastty-ai] Tool '{}' requested permission to run:\n{}\nAllow execution? [y/N/always]: ",
            tool_name, input_summary
        );
        let _ = io::stdout().flush();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return PermissionDecision::Deny;
        }

        let trimmed = input.trim().to_lowercase();
        if trimmed == "y" || trimmed == "yes" {
            PermissionDecision::Allow
        } else if trimmed == "always" {
            let key = format!("{}:{}", tool_name, input_summary);
            self.checker.allow_always(&key);
            PermissionDecision::Allow
        } else {
            PermissionDecision::Deny
        }
    }
}

/// Max output tokens per model request. Large `edit_file` tool calls carry
/// whole code blocks as JSON, so this must be high enough that the tool-call
/// JSON is never truncated mid-argument. Truncated JSON is the main cause of
/// "the agent keeps loading but never applies the edit".
const MAX_OUTPUT_TOKENS: u32 = 8192;

/// How many times the exact same tool call may fail consecutively before the
/// turn is aborted. Prevents endless read -> failed edit -> retry loops that
/// spin the tool spinner forever without producing an edit.
const MAX_IDENTICAL_TOOL_FAILURES: usize = 3;

pub struct Agent {
    model: Arc<dyn LanguageModel>,
    model_name: String,
    tools: Vec<Arc<dyn Tool>>,
    permissions: Arc<PermissionChecker>,
    permission_handler: Arc<dyn PermissionHandler>,
    messages: Vec<Message>,
    ctx: ToolCtx,
    cancel: CancelToken,
    last_failed_call: Option<(String, String)>,
    identical_failures: usize,
}

impl Agent {
    pub fn new(
        model: Arc<dyn LanguageModel>,
        model_name: String,
        tools: Vec<Arc<dyn Tool>>,
        permissions: Arc<PermissionChecker>,
        permission_handler: Arc<dyn PermissionHandler>,
        ctx: ToolCtx,
    ) -> Self {
        Self {
            model,
            model_name,
            tools,
            permissions,
            permission_handler,
            messages: Vec::new(),
            ctx,
            cancel: CancelToken::new(),
            last_failed_call: None,
            identical_failures: 0,
        }
    }

    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn run_turn<F>(&mut self, on_event: F) -> anyhow::Result<()>
    where
        F: Fn(AgentEvent) + Send + Sync + 'static,
    {
        let tool_defs: Vec<_> = self
            .tools
            .iter()
            .map(|t| crate::ai::model::ToolDefinition {
                name: t.name().to_string(),
                description: t.description(),
                parameters: t.schema(),
            })
            .collect();

        loop {
            if self.cancel.is_cancelled() {
                on_event(AgentEvent::Error("Turn cancelled by user".to_string()));
                return Ok(());
            }

            let req = CompletionRequest {
                model: self.model_name.clone(),
                messages: self.messages.clone(),
                tools: tool_defs.clone(),
                temperature: Some(0.2),
                max_tokens: Some(MAX_OUTPUT_TOKENS),
            };

            let (tx, rx) = async_channel::unbounded();
            let _stream_cancel = self.model.stream(req, tx);

            let mut assistant_text = String::new();
            let mut partial_tool_calls: HashMap<usize, (Option<String>, Option<String>, String)> =
                HashMap::new();
            let mut stop_reason = StopReason::EndTurn;

            while let Ok(event) = rx.recv_blocking() {
                if self.cancel.is_cancelled() {
                    stop_reason = StopReason::Cancelled;
                    break;
                }

                match event {
                    CompletionEvent::Text(delta) => {
                        assistant_text.push_str(&delta);
                        on_event(AgentEvent::TextDelta(delta));
                    }
                    CompletionEvent::Thinking(delta) => {
                        on_event(AgentEvent::ThinkingDelta(delta));
                    }
                    CompletionEvent::ToolUse {
                        index,
                        id,
                        name,
                        args_delta,
                    } => {
                        let entry = partial_tool_calls.entry(index).or_insert((None, None, String::new()));
                        if let Some(i) = id {
                            entry.0 = Some(i);
                        }
                        if let Some(n) = name {
                            entry.1 = Some(n);
                        }
                        entry.2.push_str(&args_delta);
                    }
                    CompletionEvent::Usage { input, output } => {
                        on_event(AgentEvent::Usage { input, output });
                    }
                    CompletionEvent::Stop(reason) => {
                        stop_reason = reason;
                        break;
                    }
                }
            }

            if stop_reason == StopReason::Cancelled || self.cancel.is_cancelled() {
                on_event(AgentEvent::Error("Turn cancelled by user".to_string()));
                return Ok(());
            }

            if let StopReason::Error(err) = stop_reason {
                on_event(AgentEvent::Error(err.clone()));
                return Err(anyhow::anyhow!("Model stream error: {}", err));
            }

            // Convert partial tool calls to completed ToolCall list
            let mut completed_tool_calls: Vec<ToolCall> = Vec::new();
            let mut sorted_indices: Vec<_> = partial_tool_calls.keys().copied().collect();
            sorted_indices.sort_unstable();

            for idx in sorted_indices {
                if let Some((id, name, args)) = partial_tool_calls.remove(&idx) {
                    let tool_id = id.unwrap_or_else(|| format!("call_{}", idx));
                    let tool_name = name.unwrap_or_default();
                    if !tool_name.is_empty() {
                        completed_tool_calls.push(ToolCall {
                            id: tool_id,
                            name: tool_name,
                            arguments: args,
                        });
                    }
                }
            }

            // Record assistant message
            if !completed_tool_calls.is_empty() {
                self.messages.push(Message::assistant_tool_calls(
                    assistant_text,
                    completed_tool_calls.clone(),
                ));
            } else {
                self.messages.push(Message::assistant(assistant_text));
                on_event(AgentEvent::TurnEnd);
                break;
            }

            // Execute requested tools
            for tc in completed_tool_calls {
                if self.cancel.is_cancelled() {
                    break;
                }

                on_event(AgentEvent::ToolStart {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    args: tc.arguments.clone(),
                });

                let tool = self.tools.iter().find(|t| t.name() == tc.name);
                let Some(tool) = tool else {
                    let err_msg = format!("Tool '{}' not found", tc.name);
                    on_event(AgentEvent::ToolEnd {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        output: err_msg.clone(),
                        is_error: true,
                        diff: None,
                    });
                    self.messages.push(Message::tool_result(tc.id, err_msg, true));
                    continue;
                };

                // Check permissions
                let initial_decision = self.permissions.check_permission(&tc.name, &tc.arguments);
                let final_decision = match initial_decision {
                    PermissionDecision::Allow => PermissionDecision::Allow,
                    PermissionDecision::Deny => PermissionDecision::Deny,
                    PermissionDecision::Confirm => {
                        self.permission_handler
                            .ask_permission(&tc.id, &tc.name, &tc.arguments)
                    }
                };

                if final_decision == PermissionDecision::Deny {
                    let deny_msg = "Tool execution denied by user/policy.".to_string();
                    on_event(AgentEvent::ToolEnd {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        output: deny_msg.clone(),
                        is_error: true,
                        diff: None,
                    });
                    self.messages.push(Message::tool_result(tc.id, deny_msg, true));
                    continue;
                }

                // Parse arguments
                let input_val: serde_json::Value = match serde_json::from_str(&tc.arguments) {
                    Ok(v) => v,
                    Err(e) => {
                        let err_msg = if stop_reason == StopReason::MaxTokens {
                            format!(
                                "JSON parse error in tool arguments: {}. The tool call was cut off because the response hit the token limit. The edit is too large for one call: split it into several smaller edits with minimal old_string/new_string pairs.",
                                e
                            )
                        } else {
                            format!(
                                "JSON parse error in tool arguments: {}. Retry with valid JSON.",
                                e
                            )
                        };
                        on_event(AgentEvent::ToolEnd {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            output: err_msg.clone(),
                            is_error: true,
                            diff: None,
                        });
                        self.messages.push(Message::tool_result(tc.id, err_msg, true));
                        continue;
                    }
                };

                // Run tool
                let out = match tool.run(input_val, &self.ctx, &self.cancel) {
                    Ok(o) => o,
                    Err(e) => e,
                };

                if out.is_error {
                    // Circuit breaker: stop when the identical call keeps failing,
                    // otherwise the model retries the same edit forever.
                    let same_as_last = self
                        .last_failed_call
                        .as_ref()
                        .is_some_and(|(n, a)| n == &tc.name && a == &tc.arguments);
                    if same_as_last {
                        self.identical_failures += 1;
                    } else {
                        self.last_failed_call = Some((tc.name.clone(), tc.arguments.clone()));
                        self.identical_failures = 1;
                    }

                    if self.identical_failures >= MAX_IDENTICAL_TOOL_FAILURES {
                        let msg = format!(
                            "The same '{}' call failed {} times in a row. Stopping this turn to avoid an endless retry loop. Last error: {}",
                            tc.name, self.identical_failures, out.content
                        );
                        on_event(AgentEvent::ToolEnd {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            output: msg.clone(),
                            is_error: true,
                            diff: None,
                        });
                        self.messages.push(Message::tool_result(tc.id, msg, true));
                        on_event(AgentEvent::TurnEnd);
                        return Ok(());
                    }
                } else {
                    self.last_failed_call = None;
                    self.identical_failures = 0;
                }

                on_event(AgentEvent::ToolEnd {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    output: out.content.clone(),
                    is_error: out.is_error,
                    diff: out.diff,
                });

                self.messages.push(Message::tool_result(
                    tc.id,
                    out.content,
                    out.is_error,
                ));
            }
        }

        Ok(())
    }
}
