use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use super::{default_false, default_true};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SecurityLevel {
    #[serde(rename = "read_only")]
    ReadOnly,
    #[serde(rename = "ask_confirm")]
    AskConfirm,
    #[serde(rename = "auto_execute")]
    AutoExecute,
}

impl Default for SecurityLevel {
    fn default() -> Self {
        Self::AskConfirm
    }
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            transport: default_mcp_transport(),
            command: None,
            args: Vec::new(),
            env: HashMap::new(),
            url: None,
        }
    }
}

fn default_max_steps() -> usize {
    10
}

fn default_thinking_cue() -> String {
    "thinking".to_string()
}

fn default_tool_cue() -> String {
    "nod".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuiltinToolsConfig {
    #[serde(default = "default_true")]
    pub use_skill: bool,
    #[serde(default = "default_true")]
    pub get_current_time: bool,
    #[serde(default = "default_true")]
    pub read_file: bool,
    #[serde(default = "default_true")]
    pub execute_command: bool,
    #[serde(default = "default_true")]
    pub take_screenshot: bool,
    #[serde(default = "default_false")]
    pub screenshot_save_to_disk: bool,
}

impl Default for BuiltinToolsConfig {
    fn default() -> Self {
        Self {
            use_skill: true,
            get_current_time: true,
            read_file: true,
            execute_command: true,
            take_screenshot: true,
            screenshot_save_to_disk: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub skills_enabled: bool,
    #[serde(default)]
    pub security_level: SecurityLevel,
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
    #[serde(default = "default_true")]
    pub live2d_thinking_enabled: bool,
    #[serde(default = "default_thinking_cue")]
    pub thinking_cue: String,
    #[serde(default = "default_false")]
    pub live2d_tool_cue_enabled: bool,
    #[serde(default = "default_tool_cue")]
    pub tool_cue: String,
    #[serde(default = "default_true")]
    pub subtitle_tool_call_status: bool,
    #[serde(default = "default_false")]
    pub subtitle_thinking_status: bool,
    #[serde(default)]
    pub builtin_tools: BuiltinToolsConfig,
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            skills_enabled: true,
            security_level: SecurityLevel::default(),
            max_steps: default_max_steps(),
            live2d_thinking_enabled: true,
            thinking_cue: default_thinking_cue(),
            live2d_tool_cue_enabled: false,
            tool_cue: default_tool_cue(),
            subtitle_tool_call_status: true,
            subtitle_thinking_status: false,
            builtin_tools: BuiltinToolsConfig::default(),
            mcp_servers: HashMap::new(),
        }
    }
}

impl AgentConfig {
    pub fn merge_with(&self, character_override: &AgentConfig) -> Self {
        let mut merged = self.clone();
        merged.enabled = character_override.enabled;
        merged.skills_enabled = character_override.skills_enabled;
        merged.security_level = character_override.security_level.clone();
        merged.max_steps = character_override.max_steps;
        merged.live2d_thinking_enabled = character_override.live2d_thinking_enabled;
        merged.thinking_cue = character_override.thinking_cue.clone();
        merged.live2d_tool_cue_enabled = character_override.live2d_tool_cue_enabled;
        merged.tool_cue = character_override.tool_cue.clone();
        merged.subtitle_tool_call_status = character_override.subtitle_tool_call_status;
        merged.subtitle_thinking_status = character_override.subtitle_thinking_status;
        merged.builtin_tools = character_override.builtin_tools.clone();
        for (k, v) in &character_override.mcp_servers {
            merged.mcp_servers.insert(k.clone(), v.clone());
        }
        merged
    }
}
