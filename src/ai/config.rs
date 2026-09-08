use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    #[default]
    ConfirmWrites,
    ConfirmAll,
    Yolo,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmWrites => write!(f, "confirm-writes"),
            Self::ConfirmAll => write!(f, "confirm-all"),
            Self::Yolo => write!(f, "yolo"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ProviderConfig {
    #[serde(rename = "openai-compat")]
    OpenaiCompat {
        base_url: String,
        #[serde(default)]
        api_key_env: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        models: Vec<String>,
    },
    #[serde(rename = "anthropic")]
    Anthropic {
        #[serde(default = "default_anthropic_base_url")]
        base_url: String,
        #[serde(default = "default_anthropic_api_key_env")]
        api_key_env: String,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default = "default_anthropic_models")]
        models: Vec<String>,
    },
}

impl ProviderConfig {
    pub fn models(&self) -> &[String] {
        match self {
            Self::OpenaiCompat { models, .. } => models.as_slice(),
            Self::Anthropic { models, .. } => models.as_slice(),
        }
    }

    pub fn base_url(&self) -> &str {
        match self {
            Self::OpenaiCompat { base_url, .. } => base_url.as_str(),
            Self::Anthropic { base_url, .. } => base_url.as_str(),
        }
    }

    pub fn set_base_url(&mut self, new_url: String) {
        match self {
            Self::OpenaiCompat { base_url, .. } => *base_url = new_url,
            Self::Anthropic { base_url, .. } => *base_url = new_url,
        }
    }

    pub fn api_key_env(&self) -> Option<&str> {
        match self {
            Self::OpenaiCompat { api_key_env, .. } => api_key_env.as_deref(),
            Self::Anthropic { api_key_env, .. } => Some(api_key_env.as_str()),
        }
    }

    pub fn api_key(&self) -> Option<&str> {
        match self {
            Self::OpenaiCompat { api_key, .. } => api_key.as_deref(),
            Self::Anthropic { api_key, .. } => api_key.as_deref(),
        }
    }

    pub fn set_api_key(&mut self, new_key: Option<String>) {
        match self {
            Self::OpenaiCompat { api_key, .. } => *api_key = new_key,
            Self::Anthropic { api_key, .. } => *api_key = new_key,
        }
    }

    pub fn models_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::OpenaiCompat { models, .. } => models,
            Self::Anthropic { models, .. } => models,
        }
    }

    pub fn remember_model(&mut self, model: String) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }
        let list = self.models_mut();
        list.retain(|m| m != trimmed);
        list.insert(0, trimmed.to_string());
    }
}

fn default_anthropic_base_url() -> String {
    "https://api.anthropic.com/v1".to_string()
}

fn default_anthropic_api_key_env() -> String {
    "ANTHROPIC_API_KEY".to_string()
}

fn default_anthropic_models() -> Vec<String> {
    vec![
        "claude-3-7-sonnet-latest".to_string(),
        "claude-3-5-sonnet-latest".to_string(),
        "claude-3-5-haiku-latest".to_string(),
    ]
}

fn default_default_provider() -> String {
    "ollama".to_string()
}

fn default_permission_mode() -> PermissionMode {
    PermissionMode::ConfirmWrites
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_default_provider")]
    pub default: String,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: PermissionMode,
    #[serde(default = "default_providers")]
    pub providers: HashMap<String, ProviderConfig>,
    /// Token budget for the model context window.
    /// Used to compute context usage % in the agent panel.
    #[serde(default = "default_context_window")]
    pub context_window: u32,
}

fn default_context_window() -> u32 {
    128_000
}

impl AiConfig {
    pub fn active_model(&self) -> String {
        if let Some(ref m) = self.default_model {
            if !m.is_empty() {
                return m.clone();
            }
        }
        if let Some(prov) = self.providers.get(&self.default) {
            if let Some(m) = prov.models().first() {
                return m.clone();
            }
        }
        "default".to_string()
    }

    pub fn default_preset_models(provider: &str) -> Vec<String> {
        match provider {
            "ollama" => vec![
                "llama3.2".to_string(),
                "qwen2.5-coder".to_string(),
                "mistral".to_string(),
                "deepseek-r1".to_string(),
            ],
            "anthropic" => vec![
                "claude-3-7-sonnet-latest".to_string(),
                "claude-3-5-sonnet-latest".to_string(),
                "claude-3-5-haiku-latest".to_string(),
                "claude-3-opus-latest".to_string(),
            ],
            "openai" => vec![
                "gpt-4o".to_string(),
                "gpt-4o-mini".to_string(),
                "o3-mini".to_string(),
                "o1".to_string(),
            ],
            "openrouter" => vec![
                "anthropic/claude-3.5-sonnet".to_string(),
                "deepseek/deepseek-r1".to_string(),
                "meta-llama/llama-3.3-70b-instruct".to_string(),
                "openai/gpt-4o".to_string(),
            ],
            "lm-studio" => vec![
                "local-model".to_string(),
                "qwen2.5-coder-7b-instruct".to_string(),
                "deepseek-r1-distill-qwen-7b".to_string(),
            ],
            _ => vec![],
        }
    }
}

fn default_providers() -> HashMap<String, ProviderConfig> {
    let mut map = HashMap::new();
    map.insert(
        "ollama".to_string(),
        ProviderConfig::OpenaiCompat {
            base_url: "http://localhost:11434/v1".to_string(),
            api_key_env: None,
            api_key: None,
            models: vec![
                "qwen2.5-coder:7b".to_string(),
                "llama3.2:3b".to_string(),
                "deepseek-r1:8b".to_string(),
            ],
        },
    );
    map.insert(
        "anthropic".to_string(),
        ProviderConfig::Anthropic {
            base_url: default_anthropic_base_url(),
            api_key_env: default_anthropic_api_key_env(),
            api_key: None,
            models: default_anthropic_models(),
        },
    );
    map.insert(
        "openai".to_string(),
        ProviderConfig::OpenaiCompat {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            api_key: None,
            models: vec![
                "gpt-4o".to_string(),
                "gpt-4o-mini".to_string(),
                "o3-mini".to_string(),
            ],
        },
    );
    map.insert(
        "openrouter".to_string(),
        ProviderConfig::OpenaiCompat {
            base_url: "https://openrouter.ai/api/v1".to_string(),
            api_key_env: Some("OPENROUTER_API_KEY".to_string()),
            api_key: None,
            models: vec![
                "anthropic/claude-3.7-sonnet".to_string(),
                "deepseek/deepseek-r1".to_string(),
                "meta-llama/llama-3.3-70b-instruct".to_string(),
            ],
        },
    );
    map.insert(
        "lm-studio".to_string(),
        ProviderConfig::OpenaiCompat {
            base_url: "http://localhost:1234/v1".to_string(),
            api_key_env: None,
            api_key: None,
            models: vec!["local-model".to_string()],
        },
    );
    map
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            default: default_default_provider(),
            default_model: None,
            permission_mode: default_permission_mode(),
            providers: default_providers(),
            context_window: default_context_window(),
        }
    }
}
