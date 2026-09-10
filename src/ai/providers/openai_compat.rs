use std::io::BufReader;
use std::time::Duration;
use serde_json::json;

use crate::ai::model::{
    CancelToken, CompletionEvent, CompletionRequest, LanguageModel, Message, Role, StopReason,
};
use crate::ai::sse::read_sse_event;

pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: Option<String>,
    default_model: String,
    client: ureq::Agent,
}

impl OpenAiCompatProvider {
    pub fn new(base_url: String, api_key: Option<String>, default_model: String) -> Self {
        let client = ureq::builder()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(60))
            .build();
        Self {
            base_url,
            api_key,
            default_model,
            client,
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

fn format_messages(messages: &[Message], is_openai_native: bool) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| match m.role {
            Role::System => json!({
                "role": "system",
                "content": m.content.as_text_lossy().to_string(),
            }),
            Role::User => match &m.content {
                crate::ai::model::MessageContent::Text(s) => json!({
                    "role": "user",
                    "content": s,
                }),
                crate::ai::model::MessageContent::Parts(parts) => {
                    let blocks: Vec<serde_json::Value> = parts
                        .iter()
                        .map(|p| match p {
                            crate::ai::model::ContentPart::Text { text } => json!({
                                "type": "text",
                                "text": text,
                            }),
                            crate::ai::model::ContentPart::Image { media_type, data } => json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{};base64,{}", media_type, data),
                                    "detail": "auto",
                                }
                            }),
                            crate::ai::model::ContentPart::Document { media_type, data, name } => {
                                if is_openai_native {
                                    json!({
                                        "type": "file",
                                        "file": {
                                            "filename": name.clone().unwrap_or_else(|| "document.pdf".to_string()),
                                            "file_data": format!("data:{};base64,{}", media_type, data),
                                        }
                                    })
                                } else {
                                    json!({
                                        "type": "text",
                                        "text": format!("[Attached PDF document: {}]", name.as_deref().unwrap_or("document.pdf")),
                                    })
                                }
                            }
                        })
                        .collect();
                    json!({
                        "role": "user",
                        "content": blocks,
                    })
                }
            },
            Role::Assistant => {
                let text = m.content.as_text_lossy().to_string();
                if let Some(ref tool_calls) = m.tool_calls {
                    let tc_json: Vec<_> = tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments,
                                }
                            })
                        })
                        .collect();
                    json!({
                        "role": "assistant",
                        "content": if text.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(text) },
                        "tool_calls": tc_json,
                    })
                } else {
                    json!({
                        "role": "assistant",
                        "content": text,
                    })
                }
            }
            Role::Tool => json!({
                "role": "tool",
                "tool_call_id": m.tool_call_id.clone().unwrap_or_default(),
                "content": m.content.as_text_lossy().to_string(),
            }),
        })
        .collect()
}

impl LanguageModel for OpenAiCompatProvider {
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
        let client = self.client.clone();

        std::thread::Builder::new()
            .name("ai-openai-stream".into())
            .spawn(move || {
                let clean_base = base_url.trim_end_matches('/');
                let url = if clean_base.ends_with("/chat/completions") {
                    clean_base.to_string()
                } else if clean_base.ends_with("/v1") {
                    format!("{}/chat/completions", clean_base)
                } else {
                    format!("{}/v1/chat/completions", clean_base)
                };

                let raw_model = if req.model.is_empty() { "default" } else { &req.model };
                let clean_model = raw_model
                    .split('[')
                    .next()
                    .unwrap_or(raw_model)
                    .split('(')
                    .next()
                    .unwrap_or(raw_model)
                    .trim();

                let is_openai_native = base_url.contains("api.openai.com") || base_url.contains("openai.azure.com");
                let mut body = json!({
                    "model": clean_model,
                    "messages": format_messages(&req.messages, is_openai_native),
                    "stream": true,
                    "stream_options": { "include_usage": true },
                });

                if let Some(temp) = req.temperature {
                    body["temperature"] = json!(temp);
                }
                if let Some(max_t) = req.max_tokens {
                    body["max_tokens"] = json!(max_t);
                }

                if !req.tools.is_empty() {
                    let tools_json: Vec<_> = req
                        .tools
                        .iter()
                        .map(|t| {
                            json!({
                                "type": "function",
                                "function": {
                                    "name": t.name,
                                    "description": t.description,
                                    "parameters": clean_schema(&t.parameters),
                                }
                            })
                        })
                        .collect();
                    body["tools"] = json!(tools_json);
                }

                let build_req = |b: &serde_json::Value| {
                    let mut r = client.post(&url);
                    if let Some(ref key) = api_key {
                        if !key.is_empty() {
                            r = r.set("Authorization", &format!("Bearer {}", key));
                        }
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
                                        "HTTP {} error: {}", c2, e2
                                    ))));
                                    return;
                                }
                                Err(other) => {
                                    let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(format!(
                                        "HTTP retry error: {}", other
                                    ))));
                                    return;
                                }
                            }
                        } else {
                            let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(format!(
                                "HTTP {} error: {}", code, err_text
                            ))));
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(format!(
                            "HTTP request failed: {}",
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

                    let data = event.data.trim();
                    if data == "[DONE]" {
                        let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::EndTurn));
                        break;
                    }

                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(err) = parsed.get("error") {
                            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("API error");
                            let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Error(msg.to_string())));
                            return;
                        }

                        if let Some(usage) = parsed.get("usage") {
                            let input = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                            let output = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                            let _ = tx.send_blocking(CompletionEvent::Usage { input, output });
                        }

                        if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array()) {
                            for choice in choices {
                                if let Some(delta) = choice.get("delta") {
                                    if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                                        if !text.is_empty() {
                                            let _ = tx.send_blocking(CompletionEvent::Text(text.to_string()));
                                        }
                                    }
                                    if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str()) {
                                        if !reasoning.is_empty() {
                                            let _ = tx.send_blocking(CompletionEvent::Thinking(reasoning.to_string()));
                                        }
                                    }
                                    if let Some(tool_calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
                                        for tc in tool_calls {
                                            let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                            let id = tc.get("id").and_then(|s| s.as_str()).map(|s| s.to_string());
                                            let func = tc.get("function");
                                            let name = func.and_then(|f| f.get("name")).and_then(|n| n.as_str()).map(|s| s.to_string());
                                            let args = func.and_then(|f| f.get("arguments")).and_then(|a| a.as_str()).unwrap_or("");
                                            let _ = tx.send_blocking(CompletionEvent::ToolUse {
                                                index: idx,
                                                id,
                                                name,
                                                args_delta: args.to_string(),
                                            });
                                        }
                                    }
                                }

                                if let Some(finish) = choice.get("finish_reason").and_then(|f| f.as_str()) {
                                    match finish {
                                        "tool_calls" => {
                                            let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::ToolUse));
                                            return;
                                        }
                                        "length" => {
                                            let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::MaxTokens));
                                            return;
                                        }
                                        "stop" => {
                                            let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::EndTurn));
                                            return;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }

                if cancel_clone.is_cancelled() {
                    let _ = tx.send_blocking(CompletionEvent::Stop(StopReason::Cancelled));
                }
            })
            .expect("failed to spawn OpenAI streaming thread");

        cancel
    }
}

pub fn detect_installed_ollama_models(base_url: Option<&str>) -> Vec<String> {
    let mut detected = Vec::new();

    // 1. Try querying Ollama HTTP API (via ureq)
    let url = base_url.unwrap_or("http://localhost:11434");
    let clean_base = url
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .trim_end_matches('/');
    let tags_url = format!("{}/api/tags", clean_base);

    if let Ok(resp) = ureq::get(&tags_url)
        .timeout(Duration::from_millis(800))
        .call()
    {
        if let Ok(json) = resp.into_json::<serde_json::Value>() {
            if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                for item in models {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        if !detected.contains(&name.to_string()) {
                            detected.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    // 2. If HTTP didn't find any or failed, fallback to running `ollama list` CLI
    if detected.is_empty() {
        if let Ok(output) = std::process::Command::new("ollama")
            .arg("list")
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for (idx, line) in text.lines().enumerate() {
                    if idx == 0 {
                        continue; // Skip table header
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Some(first_col) = trimmed.split_whitespace().next() {
                        let model_name = first_col.to_string();
                        if !detected.contains(&model_name) {
                            detected.push(model_name);
                        }
                    }
                }
            }
        }
    }

    detected
}
