use std::io::BufReader;
use std::time::Duration;
use serde_json::json;

use crate::ai::model::{
    CancelToken, CompletionEvent, CompletionRequest, LanguageModel, Message, Role, StopReason,
};
use crate::ai::sse::read_sse_event;

pub struct AnthropicProvider {
    base_url: String,
    api_key: String,
    default_model: String,
}

impl AnthropicProvider {
    pub fn new(base_url: String, api_key: String, default_model: String) -> Self {
        Self {
            base_url,
            api_key,
            default_model,
        }
    }
}

fn clean_schema(val: &serde_json::Value) -> serde_json::Value {
    match val {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                if k != "$schema" && k != "title" {
                    new_map.insert(k.clone(), clean_schema(v));
                }
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(clean_schema).collect())
        }
        other => other.clone(),
    }
}

fn convert_messages_and_system(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
    let mut system_prompts: Vec<String> = Vec::new();
    let mut out_messages: Vec<serde_json::Value> = Vec::new();

    let mut current_user_blocks: Vec<serde_json::Value> = Vec::new();

    for m in messages {
        match m.role {
            Role::System => {
                system_prompts.push(m.content.clone());
            }
            Role::User => {
                if !current_user_blocks.is_empty() {
                    out_messages.push(json!({
                        "role": "user",
                        "content": current_user_blocks.clone(),
                    }));
                    current_user_blocks.clear();
                }
                out_messages.push(json!({
                    "role": "user",
                    "content": m.content,
                }));
            }
            Role::Tool => {
                current_user_blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": m.content,
                    "is_error": m.is_error,
                }));
            }
            Role::Assistant => {
                if !current_user_blocks.is_empty() {
                    out_messages.push(json!({
                        "role": "user",
                        "content": current_user_blocks.clone(),
                    }));
                    current_user_blocks.clear();
                }

                if let Some(ref tool_calls) = m.tool_calls {
                    let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                    if !m.content.is_empty() {
                        content_blocks.push(json!({
                            "type": "text",
                            "text": m.content,
                        }));
                    }
                    for tc in tool_calls {
                        let parsed_input: serde_json::Value =
                            serde_json::from_str(&tc.arguments).unwrap_or_else(|_| json!({}));
                        content_blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": parsed_input,
                        }));
                    }
                    out_messages.push(json!({
                        "role": "assistant",
                        "content": content_blocks,
                    }));
                } else {
                    out_messages.push(json!({
                        "role": "assistant",
                        "content": m.content,
                    }));
                }
            }
        }
    }

    if !current_user_blocks.is_empty() {
        out_messages.push(json!({
            "role": "user",
            "content": current_user_blocks,
        }));
    }

    let system = if system_prompts.is_empty() {
        None
    } else {
        Some(system_prompts.join("\n\n"))
    };

    (system, out_messages)
}

impl LanguageModel for AnthropicProvider {
    fn default_model(&self) -> String {
        self.default_model.clone()
    }

    fn stream(
        &self,
        req: CompletionRequest,
        tx: async_channel::Sender<CompletionEvent>,
    ) -> CancelToken {
        let cancel = CancelToken::new();
        let cancel_clone = cancel.clone();
        let base_url = self.base_url.trim_end_matches('/').to_string();
        let api_key = self.api_key.clone();

        std::thread::Builder::new()
            .name("ai-anthropic-stream".into())
            .spawn(move || {
                let clean_base = base_url.trim_end_matches('/');
                let url = if clean_base.ends_with("/messages") {
                    clean_base.to_string()
                } else if clean_base.ends_with("/v1") {
                    format!("{}/messages", clean_base)
                } else {
                    format!("{}/v1/messages", clean_base)
                };
                let (system, messages) = convert_messages_and_system(&req.messages);

                let raw_model = if req.model.is_empty() { "claude-3-7-sonnet-latest" } else { &req.model };
                let clean_model = raw_model
                    .split('[')
                    .next()
                    .unwrap_or(raw_model)
                    .split('(')
                    .next()
                    .unwrap_or(raw_model)
                    .trim();

                let mut body = json!({
                    "model": clean_model,
                    "messages": messages,
                    "max_tokens": req.max_tokens.unwrap_or(4096),
                    "stream": true,
                });

                if let Some(sys) = system {
                    body["system"] = json!(sys);
                }
                if let Some(temp) = req.temperature {
                    body["temperature"] = json!(temp);
                }

                if !req.tools.is_empty() {
                    let tools_json: Vec<_> = req
                        .tools
                        .iter()
                        .map(|t| {
                            json!({
                                "name": t.name,
                                "description": t.description,
                                "input_schema": clean_schema(&t.parameters),
                            })
                        })
                        .collect();
                    body["tools"] = json!(tools_json);
                }

                let agent = ureq::builder()
                    .timeout_connect(Duration::from_secs(10))
                    .timeout_read(Duration::from_secs(60))
                    .build();

                let build_req = |b: &serde_json::Value| {
                    let mut r = agent.post(&url)
                        .set("Content-Type", "application/json")
                        .set("anthropic-version", "2023-06-01");
                    if !api_key.is_empty() {
                        r = r.set("x-api-key", &api_key)
                             .set("Authorization", &format!("Bearer {}", api_key));
                    }
                    r.send_json(b.clone())
                };

                let response = match build_req(&body) {
                    Ok(resp) => resp,
                    Err(ureq::Error::Status(code, resp)) => {
                        let err_text = resp.into_string().unwrap_or_default();
                        // If tools were provided and endpoint rejected with 400 Bad Request, retry without tools
                        if code == 400 && !req.tools.is_empty() {
                            let mut fallback_body = body;
                            fallback_body.as_object_mut().map(|m| m.remove("tools"));
                            match build_req(&fallback_body) {
                                Ok(resp) => resp,
                                Err(ureq::Error::Status(c2, r2)) => {
                                    let e2 = r2.into_string().unwrap_or_default();
                                    let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(format!(
                                        "Anthropic HTTP {} error: {}", c2, e2
                                    ))));
                                    return;
                                }
                                Err(other) => {
                                    let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(format!(
                                        "Anthropic HTTP retry error: {}", other
                                    ))));
                                    return;
                                }
                            }
                        } else {
                            let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(format!(
                                "Anthropic HTTP {} error: {}", code, err_text
                            ))));
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(format!(
                            "Anthropic HTTP request failed: {}",
                            e
                        ))));
                        return;
                    }
                };

                let content_type = response.header("content-type").unwrap_or("");
                if !content_type.contains("event-stream") && content_type.contains("json") {
                    let body_str = response.into_string().unwrap_or_default();
                    let msg = serde_json::from_str::<serde_json::Value>(&body_str)
                        .ok()
                        .and_then(|v| {
                            v.get("msg")
                                .or_else(|| v.get("message"))
                                .or_else(|| v.get("error").and_then(|e| e.get("message")))
                                .and_then(|m| m.as_str())
                                .map(|s| s.to_string())
                        })
                        .unwrap_or(body_str);
                    let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(format!(
                        "API returned error: {}", msg
                    ))));
                    return;
                }

                let mut reader = BufReader::new(response.into_reader());
                let mut input_tokens: u64 = 0;
                let mut output_tokens: u64 = 0;

                while !cancel_clone.is_cancelled() {
                    let event = match read_sse_event(&mut reader) {
                        Ok(Some(ev)) => ev,
                        Ok(None) => {
                            let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::EndTurn));
                            break;
                        }
                        Err(e) => {
                            if cancel_clone.is_cancelled() {
                                let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Cancelled));
                            } else {
                                let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(
                                    format!("Stream read error: {}", e),
                                )));
                            }
                            break;
                        }
                    };

                    let event_name = event.event_type.as_deref().unwrap_or("");
                    let data = event.data.trim();

                    if event_name == "message_stop" {
                        let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::EndTurn));
                        break;
                    }

                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        match event_name {
                            "message_start" => {
                                if let Some(msg) = parsed.get("message") {
                                    if let Some(usage) = msg.get("usage") {
                                        input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                        let _ = tx.send_blocking(CompletionEvent::Usage {
                                            input: input_tokens,
                                            output: output_tokens,
                                        });
                                    }
                                }
                            }
                            "content_block_start" => {
                                let idx = parsed.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                if let Some(block) = parsed.get("content_block") {
                                    let b_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if b_type == "tool_use" {
                                        let id = block.get("id").and_then(|s| s.as_str()).map(|s| s.to_string());
                                        let name = block.get("name").and_then(|s| s.as_str()).map(|s| s.to_string());
                                        let _ = tx.send_blocking(CompletionEvent::ToolUse {
                                            index: idx,
                                            id,
                                            name,
                                            args_delta: String::new(),
                                        });
                                    }
                                }
                            }
                            "content_block_delta" => {
                                let idx = parsed.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                if let Some(delta) = parsed.get("delta") {
                                    let d_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    match d_type {
                                        "text_delta" => {
                                            if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                                let _ = tx.send_blocking(CompletionEvent::Text(text.to_string()));
                                            }
                                        }
                                        "thinking_delta" => {
                                            if let Some(thinking) = delta.get("thinking").and_then(|t| t.as_str()) {
                                                let _ = tx.send_blocking(CompletionEvent::Thinking(thinking.to_string()));
                                            }
                                        }
                                        "input_json_delta" => {
                                            if let Some(partial) = delta.get("partial_json").and_then(|p| p.as_str()) {
                                                let _ = tx.send_blocking(CompletionEvent::ToolUse {
                                                    index: idx,
                                                    id: None,
                                                    name: None,
                                                    args_delta: partial.to_string(),
                                                });
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "message_delta" => {
                                if let Some(usage) = parsed.get("usage") {
                                    output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                    let _ = tx.send_blocking(CompletionEvent::Usage {
                                        input: input_tokens,
                                        output: output_tokens,
                                    });
                                }
                                if let Some(delta) = parsed.get("delta") {
                                    if let Some(stop_reason) = delta.get("stop_reason").and_then(|s| s.as_str()) {
                                        match stop_reason {
                                            "tool_use" => {
                                                let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::ToolUse));
                                                return;
                                            }
                                            "max_tokens" => {
                                                let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::MaxTokens));
                                                return;
                                            }
                                            "end_turn" | "stop_sequence" => {
                                                let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::EndTurn));
                                                return;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            "error" => {
                                let err_msg = parsed.get("error")
                                    .and_then(|e| e.get("message"))
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Anthropic API error");
                                let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(err_msg.to_string())));
                                return;
                            }
                            _ => {}
                        }
                    }
                }

                if cancel_clone.is_cancelled() {
                    let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Cancelled));
                }
            })
            .expect("failed to spawn Anthropic streaming thread");

        cancel
    }
}
