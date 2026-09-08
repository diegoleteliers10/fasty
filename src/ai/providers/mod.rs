pub mod anthropic;
pub mod openai_compat;

pub use anthropic::AnthropicProvider;
pub use openai_compat::{OpenAiCompatProvider, detect_installed_ollama_models};

use std::sync::Arc;
use crate::ai::config::{AiConfig, ProviderConfig};
use crate::ai::model::LanguageModel;

pub fn create_model_from_config(
    config: &AiConfig,
    provider_name: Option<&str>,
    model_name: Option<&str>,
) -> anyhow::Result<(Arc<dyn LanguageModel>, String)> {
    let name = provider_name.unwrap_or(&config.default);
    let prov = config
        .providers
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("AI provider '{}' not found in configuration", name))?;

    let resolved_model = model_name
        .map(|m| m.to_string())
        .or_else(|| {
            if name == config.default {
                config.default_model.clone()
            } else {
                None
            }
        })
        .or_else(|| prov.models().first().cloned())
        .unwrap_or_else(|| "default".to_string());

    match prov {
        ProviderConfig::OpenaiCompat {
            base_url,
            api_key_env,
            api_key: direct_key,
            ..
        } => {
            let api_key = direct_key
                .clone()
                .filter(|k| !k.is_empty())
                .or_else(|| {
                    api_key_env
                        .as_ref()
                        .and_then(|env_var| std::env::var(env_var).ok())
                })
                .or_else(|| std::env::var("OPENAI_API_KEY").ok());
            let provider = OpenAiCompatProvider::new(base_url.clone(), api_key, resolved_model.clone());
            Ok((Arc::new(provider), resolved_model))
        }
        ProviderConfig::Anthropic {
            base_url,
            api_key_env,
            api_key: direct_key,
            ..
        } => {
            let api_key = direct_key
                .clone()
                .filter(|k| !k.is_empty())
                .or_else(|| std::env::var(api_key_env).ok())
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Anthropic API key is not set. Enter it in Settings or set {} environment variable.",
                        api_key_env
                    )
                })?;
            let provider = AnthropicProvider::new(base_url.clone(), api_key, resolved_model.clone());
            Ok((Arc::new(provider), resolved_model))
        }
    }
}

pub fn test_provider_connection(
    config: &AiConfig,
    provider_name: &str,
    model_name: &str,
) -> Result<std::time::Duration, String> {
    let start = std::time::Instant::now();
    let (model, resolved_model) = create_model_from_config(
        config,
        Some(provider_name),
        if model_name.trim().is_empty() {
            None
        } else {
            Some(model_name)
        },
    )
    .map_err(|e| format!("Config error: {e}"))?;

    let (tx, rx) = async_channel::bounded::<crate::ai::model::CompletionEvent>(16);
    let req = crate::ai::model::CompletionRequest {
        model: resolved_model,
        messages: vec![crate::ai::model::Message::user("Hello")],
        tools: Vec::new(),
        max_tokens: Some(4),
        temperature: Some(0.0),
    };

    let cancel = model.stream(req, tx);

    let (sync_tx, sync_rx) = std::sync::mpsc::channel();
    let cancel_clone = cancel.clone();
    std::thread::spawn(move || {
        while let Ok(ev) = rx.recv_blocking() {
            match ev {
                crate::ai::model::CompletionEvent::Text(_)
                | crate::ai::model::CompletionEvent::Thinking(_)
                | crate::ai::model::CompletionEvent::ToolUse { .. }
                | crate::ai::model::CompletionEvent::Usage { .. } => {
                    let _ = sync_tx.send(Ok(()));
                    break;
                }
                crate::ai::model::CompletionEvent::Stop(crate::ai::model::StopReason::Error(err)) => {
                    let _ = sync_tx.send(Err(err));
                    break;
                }
                crate::ai::model::CompletionEvent::Stop(crate::ai::model::StopReason::Cancelled) => {
                    let _ = sync_tx.send(Err("Request cancelled".to_string()));
                    break;
                }
                crate::ai::model::CompletionEvent::Stop(_) => {
                    let _ = sync_tx.send(Ok(()));
                    break;
                }
            }
        }
    });

    match sync_rx.recv_timeout(std::time::Duration::from_secs(8)) {
        Ok(Ok(())) => {
            cancel_clone.cancel();
            Ok(start.elapsed())
        }
        Ok(Err(e)) => {
            cancel_clone.cancel();
            Err(e)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            cancel_clone.cancel();
            Err("Timeout: no response after 8s".to_string())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            cancel_clone.cancel();
            Err("Connection closed or empty response".to_string())
        }
    }
}
