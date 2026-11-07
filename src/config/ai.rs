use serde::{Deserialize, Serialize};
use super::{default_false, default_true};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProactiveConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_proactive_min_interval")]
    pub min_interval_minutes: u32,
    #[serde(default = "default_proactive_max_interval")]
    pub max_interval_minutes: u32,
    #[serde(default = "default_proactive_prompt")]
    pub prompt: String,
}

fn default_proactive_min_interval() -> u32 {
    15
}

fn default_proactive_max_interval() -> u32 {
    45
}

pub fn default_proactive_prompt() -> String {
    "Please say something that would be engaging and appropriate for the current context (current time: {time}, user has been idle for {idle_minutes} minutes). If you feel it is better not to interrupt the user right now, simply output nothing or [SILENCE].".to_string()
}

impl Default for ProactiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_interval_minutes: default_proactive_min_interval(),
            max_interval_minutes: default_proactive_max_interval(),
            prompt: default_proactive_prompt(),
        }
    }
}

fn default_ai_protocol() -> String {
    "openai".to_string()
}

fn default_hermes_url() -> String {
    "http://localhost:11434/v1".to_string()
}

fn default_hermes_key() -> String {
    String::new()
}

fn default_hermes_model() -> String {
    "murasame".to_string()
}

fn default_openai_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_openai_key() -> String {
    String::new()
}

fn default_openai_model() -> String {
    "deepseek-flash".to_string()
}

fn default_context_window() -> String {
    "300k".to_string()
}

fn default_max_context_tokens() -> usize {
    300_000
}

fn default_compression_strategy() -> String {
    "truncate".to_string()
}

pub fn parse_context_window_tokens(s: &str) -> usize {
    let lower = s.trim().to_lowercase();
    if let Some(num) = lower.strip_suffix('k') {
        if let Ok(val) = num.trim().parse::<f64>() {
            return (val * 1000.0) as usize;
        }
    }
    if let Some(num) = lower.strip_suffix('m') {
        if let Ok(val) = num.trim().parse::<f64>() {
            return (val * 1_000_000.0) as usize;
        }
    }
    if let Ok(val) = lower.parse::<usize>() {
        return val;
    }
    300_000
}

fn default_hermes_endpoint() -> String {
    "responses".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_ai_protocol")]
    pub protocol: String,

    #[serde(default = "default_hermes_url")]
    pub hermes_base_url: String,
    #[serde(default = "default_hermes_key")]
    pub hermes_api_key: String,
    #[serde(default = "default_hermes_model")]
    pub hermes_model: String,
    #[serde(default = "default_hermes_endpoint")]
    pub hermes_endpoint: String,

    #[serde(default = "default_openai_url")]
    pub openai_base_url: String,
    #[serde(default = "default_openai_key")]
    pub openai_api_key: String,
    #[serde(default = "default_openai_model")]
    pub openai_model: String,

    #[serde(default = "default_context_window")]
    pub context_window: String,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    #[serde(default = "default_compression_strategy")]
    pub compression_strategy: String,

    #[serde(default)]
    pub system_prompt_prefix: String,

    #[serde(default = "default_true")]
    pub thinking_enabled: bool,
    #[serde(default = "default_thinking_intensity")]
    pub thinking_intensity: String,

    #[serde(default = "default_true")]
    pub multimodal_vision: bool,

    // Backward compatibility
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

impl AiConfig {
    pub fn get_active_base_url(&self) -> &str {
        if self.protocol == "hermes" {
            &self.hermes_base_url
        } else {
            &self.openai_base_url
        }
    }

    pub fn get_active_api_key(&self) -> &str {
        if self.protocol == "hermes" {
            &self.hermes_api_key
        } else {
            &self.openai_api_key
        }
    }

    pub fn get_active_model(&self) -> &str {
        if self.protocol == "hermes" {
            &self.hermes_model
        } else {
            &self.openai_model
        }
    }

    pub fn is_vision_supported(&self) -> bool {
        self.multimodal_vision
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            protocol: default_ai_protocol(),
            hermes_base_url: default_hermes_url(),
            hermes_api_key: default_hermes_key(),
            hermes_model: default_hermes_model(),
            hermes_endpoint: default_hermes_endpoint(),
            openai_base_url: default_openai_url(),
            openai_api_key: default_openai_key(),
            openai_model: default_openai_model(),
            context_window: default_context_window(),
            max_context_tokens: default_max_context_tokens(),
            compression_strategy: default_compression_strategy(),
            system_prompt_prefix: String::new(),
            thinking_enabled: true,
            thinking_intensity: default_thinking_intensity(),
            multimodal_vision: true,
            base_url: None,
            api_key: None,
            model: None,
        }
    }
}

fn default_thinking_intensity() -> String {
    "medium".to_string()
}


pub fn normalize_api_base_url(url: &str) -> String {
    let clean = url.trim().trim_end_matches('/');
    if clean.is_empty() {
        return String::new();
    }
    if clean.ends_with("/v1") {
        clean.to_string()
    } else {
        format!("{clean}/v1")
    }
}
