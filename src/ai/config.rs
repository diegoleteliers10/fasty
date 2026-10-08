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
    #[serde(rename = "opencode")]
    Opencode {
        #[serde(default = "default_opencode_command")]
        command: String,
        #[serde(default)]
        models: Vec<String>,
        #[serde(default)]
        variants: HashMap<String, Vec<String>>,
    },
    #[serde(rename = "acp")]
    Acp {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        models: Vec<String>,
        #[serde(default)]
        variants: HashMap<String, Vec<String>>,
    },
}

impl ProviderConfig {
    pub fn models(&self) -> &[String] {
        match self {
            Self::OpenaiCompat { models, .. } => models.as_slice(),
            Self::Anthropic { models, .. } => models.as_slice(),
            Self::Opencode { models, .. } => models.as_slice(),
            Self::Acp { models, .. } => models.as_slice(),
        }
    }

    pub fn base_url(&self) -> &str {
        match self {
            Self::OpenaiCompat { base_url, .. } => base_url.as_str(),
            Self::Anthropic { base_url, .. } => base_url.as_str(),
            Self::Opencode { .. } => "",
            Self::Acp { .. } => "",
        }
    }

    pub fn set_base_url(&mut self, new_url: String) {
        match self {
            Self::OpenaiCompat { base_url, .. } => *base_url = new_url,
            Self::Anthropic { base_url, .. } => *base_url = new_url,
            Self::Opencode { .. } => {},
            Self::Acp { .. } => {},
        }
    }

    pub fn api_key_env(&self) -> Option<&str> {
        match self {
            Self::OpenaiCompat { api_key_env, .. } => api_key_env.as_deref(),
            Self::Anthropic { api_key_env, .. } => Some(api_key_env.as_str()),
            Self::Opencode { .. } => None,
            Self::Acp { .. } => None,
        }
    }

    pub fn api_key(&self) -> Option<&str> {
        match self {
            Self::OpenaiCompat { api_key, .. } => api_key.as_deref(),
            Self::Anthropic { api_key, .. } => api_key.as_deref(),
            Self::Opencode { .. } => None,
            Self::Acp { .. } => None,
        }
    }

    pub fn set_api_key(&mut self, new_key: Option<String>) {
        match self {
            Self::OpenaiCompat { api_key, .. } => *api_key = new_key,
            Self::Anthropic { api_key, .. } => *api_key = new_key,
            Self::Opencode { .. } => {},
            Self::Acp { .. } => {},
        }
    }

    pub fn models_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::OpenaiCompat { models, .. } => models,
            Self::Anthropic { models, .. } => models,
            Self::Opencode { models, .. } => models,
            Self::Acp { models, .. } => models,
        }
    }

    pub fn command(&self) -> Option<&str> {
        match self { Self::Opencode { command, .. } | Self::Acp { command, .. } => Some(command), _ => None }
    }

    pub fn set_command(&mut self, value: String) {
        match self { Self::Opencode { command, .. } | Self::Acp { command, .. } => *command = value, _ => {} }
    }

    pub fn acp_args(&self) -> Option<Vec<String>> {
        match self {
            Self::Opencode { .. } => Some(vec!["acp".to_string()]),
            Self::Acp { args, .. } => Some(args.clone()),
            _ => None,
        }
    }

    pub fn is_acp(&self) -> bool {
        matches!(self, Self::Opencode { .. } | Self::Acp { .. })
    }

    pub fn acp_identity(&self, provider_name: &str) -> Option<String> {
        let command = self.command()?;
        let args = self.acp_args()?;
        Some(format!("{provider_name}\u{1f}{command}\u{1f}{}", args.join("\u{1f}")))
    }

    pub fn remember_models(&mut self, discovered: Vec<String>) {
        if let Self::Opencode { models, .. } | Self::Acp { models, .. } = self {
            *models = discovered.into_iter().map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty()).collect();
        }
    }

    pub fn set_acp_catalog(
        &mut self,
        models: Vec<String>,
        variants: HashMap<String, Vec<String>>,
    ) {
        if let Self::Opencode { models: stored_models, variants: stored_variants, .. }
        | Self::Acp { models: stored_models, variants: stored_variants, .. } = self {
            *stored_models = models.into_iter().map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty()).collect();
            *stored_variants = variants;
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

fn default_opencode_command() -> String { "opencode".to_string() }

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
    /// Learned auto-allow: count manual approvals of the same command
    /// family and offer to auto-allow it after enough confirmations.
    #[serde(default = "default_learned_allow")]
    pub learned_allow: bool,
}

fn default_learned_allow() -> bool {
    true
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
            "opencode" => vec![],
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
    map.insert("opencode".to_string(), ProviderConfig::Opencode {
        command: default_opencode_command(), models: Vec::new(), variants: HashMap::new(),
    });
    map.insert("claude-acp".to_string(), ProviderConfig::Acp {
        command: "claude-agent-acp".to_string(), args: Vec::new(), models: Vec::new(), variants: HashMap::new(),
    });
    map.insert("antigravity-acp".to_string(), ProviderConfig::Acp {
        command: "agy_acp_server".to_string(), args: Vec::new(), models: Vec::new(), variants: HashMap::new(),
    });
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
            learned_allow: default_learned_allow(),
        }
    }
}
