use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_active_character")]
    pub active_character: String,
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub user: UserConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub agent: AgentConfig,
}

fn default_active_character() -> String {
    "murasame".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default = "default_true")]
    pub avatar_enabled: bool,
    #[serde(default = "default_user_avatar_path")]
    pub avatar_path: String,
}

fn default_user_avatar_path() -> String {
    "assets/user_avatar.png".to_string()
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            avatar_enabled: true,
            avatar_path: default_user_avatar_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_true")]
    pub toolbar_enabled: bool,
    #[serde(default = "default_true")]
    pub subtitle_enabled: bool,
    #[serde(default = "default_false")]
    pub subtitle_bubble_enabled: bool,
    #[serde(default = "default_subtitle_font_size")]
    pub subtitle_font_size: u32,
    #[serde(default = "default_subtitle_color")]
    pub subtitle_color: String,
    #[serde(default = "default_subtitle_y_offset")]
    pub subtitle_y_offset: i32,
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_subtitle_font_size() -> u32 {
    19
}

fn default_subtitle_color() -> String {
    "#ffffff".to_string()
}

fn default_subtitle_y_offset() -> i32 {
    75
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            toolbar_enabled: true,
            subtitle_enabled: true,
            subtitle_bubble_enabled: false,
            subtitle_font_size: 19,
            subtitle_color: "#ffffff".to_string(),
            subtitle_y_offset: 75,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub always_on_top: bool,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub auto_hide_in_fullscreen: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 380,
            height: 750,
            always_on_top: true,
            x: None,
            y: None,
            auto_hide_in_fullscreen: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub path: String,
    pub scale: f32,
    #[serde(default = "default_neutral_expression")]
    pub neutral_expression: String,
    #[serde(default = "default_expression_timeout")]
    pub expression_timeout: f32,
    #[serde(default = "default_expression_fade_time")]
    pub expression_fade_time: f32,
    #[serde(default = "default_motion_fade_out")]
    pub motion_fade_out: f32,
    #[serde(default = "default_idle_motion_enabled")]
    pub idle_motion_enabled: bool,
    #[serde(default = "default_idle_motion_target")]
    pub idle_motion_target: String,
    #[serde(default = "default_idle_motion_interval")]
    pub idle_motion_interval: f32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_idle_fps")]
    pub idle_fps: u32,
    #[serde(default = "default_supersample")]
    pub supersample: f32,
}

fn default_fps() -> u32 {
    60
}

fn default_idle_fps() -> u32 {
    30
}

fn default_supersample() -> f32 {
    2.0
}

fn default_neutral_expression() -> String {
    "neutral.exp3".to_string()
}

fn default_expression_timeout() -> f32 {
    5.0
}

fn default_expression_fade_time() -> f32 {
    1.2
}

fn default_motion_fade_out() -> f32 {
    0.25
}

fn default_idle_motion_enabled() -> bool {
    true
}

fn default_idle_motion_target() -> String {
    "Idle".to_string()
}

fn default_idle_motion_interval() -> f32 {
    30.0
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            path: "Murasame_live2d/Murasame.model3.json".to_string(),
            scale: 1.0,
            neutral_expression: default_neutral_expression(),
            expression_timeout: default_expression_timeout(),
            expression_fade_time: default_expression_fade_time(),
            motion_fade_out: default_motion_fade_out(),
            idle_motion_enabled: default_idle_motion_enabled(),
            idle_motion_target: default_idle_motion_target(),
            idle_motion_interval: default_idle_motion_interval(),
            fps: default_fps(),
            idle_fps: default_idle_fps(),
            supersample: default_supersample(),
        }
    }
}

pub fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct StringOrVecVisitor;

    impl<'de> serde::de::Visitor<'de> for StringOrVecVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or a list of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![trimmed.to_string()])
            }
        }

        fn visit_seq<S>(self, mut visitor: S) -> Result<Self::Value, S::Error>
        where
            S: serde::de::SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(elem) = visitor.next_element::<String>()? {
                let trimmed = elem.trim().to_string();
                if !trimmed.is_empty() && !vec.contains(&trimmed) {
                    vec.push(trimmed);
                }
            }
            Ok(vec)
        }
    }

    deserializer.deserialize_any(StringOrVecVisitor)
}

pub fn serialize_string_or_vec<S>(vec: &Vec<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if vec.is_empty() {
        serializer.serialize_str("")
    } else if vec.len() == 1 {
        serializer.serialize_str(&vec[0])
    } else {
        vec.serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrVecItem {
    Single(String),
    Multiple(Vec<String>),
}

pub fn deserialize_hashmap_string_or_vec<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, StringOrVecItem> = HashMap::deserialize(deserializer)?;
    let mut result = HashMap::new();
    for (k, v) in raw {
        match v {
            StringOrVecItem::Single(s) => {
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() {
                    result.insert(k, vec![trimmed]);
                }
            }
            StringOrVecItem::Multiple(list) => {
                let cleaned: Vec<String> = list
                    .into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !cleaned.is_empty() {
                    result.insert(k, cleaned);
                }
            }
        }
    }
    Ok(result)
}

pub fn serialize_hashmap_string_or_vec<S>(
    map: &HashMap<String, Vec<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut ser_map = serializer.serialize_map(Some(map.len()))?;
    for (k, v) in map {
        if v.len() == 1 {
            ser_map.serialize_entry(k, &v[0])?;
        } else {
            ser_map.serialize_entry(k, v)?;
        }
    }
    ser_map.end()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CueMapping {
    #[serde(
        default,
        alias = "expressions",
        serialize_with = "serialize_string_or_vec",
        deserialize_with = "deserialize_string_or_vec"
    )]
    pub expression: Vec<String>,
    #[serde(
        default,
        alias = "motions",
        serialize_with = "serialize_string_or_vec",
        deserialize_with = "deserialize_string_or_vec"
    )]
    pub motion: Vec<String>,
    #[serde(
        default,
        alias = "motion_groups",
        serialize_with = "serialize_string_or_vec",
        deserialize_with = "deserialize_string_or_vec"
    )]
    pub motion_group: Vec<String>,
}

impl CueMapping {
    pub fn from_single(
        exp: impl Into<String>,
        mot: impl Into<String>,
        grp: impl Into<String>,
    ) -> Self {
        let e = exp.into().trim().to_string();
        let m = mot.into().trim().to_string();
        let g = grp.into().trim().to_string();
        Self {
            expression: if e.is_empty() { vec![] } else { vec![e] },
            motion: if m.is_empty() { vec![] } else { vec![m] },
            motion_group: if g.is_empty() { vec![] } else { vec![g] },
        }
    }

    pub fn from_strings(exp: &str, mot: &str, grp: &str) -> Self {
        let split_items = |s: &str| -> Vec<String> {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        };
        Self {
            expression: split_items(exp),
            motion: split_items(mot),
            motion_group: split_items(grp),
        }
    }

    pub fn first_expression(&self) -> &str {
        self.expression.first().map(|s| s.as_str()).unwrap_or("")
    }

    pub fn first_motion(&self) -> &str {
        self.motion.first().map(|s| s.as_str()).unwrap_or("")
    }

    pub fn first_motion_group(&self) -> &str {
        self.motion_group.first().map(|s| s.as_str()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CueConfig {
    #[serde(default)]
    pub mappings: HashMap<String, CueMapping>,
    #[serde(
        default,
        serialize_with = "serialize_hashmap_string_or_vec",
        deserialize_with = "deserialize_hashmap_string_or_vec"
    )]
    pub expressions: HashMap<String, Vec<String>>,
    #[serde(
        default,
        serialize_with = "serialize_hashmap_string_or_vec",
        deserialize_with = "deserialize_hashmap_string_or_vec"
    )]
    pub motions: HashMap<String, Vec<String>>,
}

impl CueConfig {
    pub fn normalized_mappings(&self) -> HashMap<String, CueMapping> {
        let mut map = self.mappings.clone();
        for (k, v) in &self.expressions {
            let entry = map.entry(k.clone()).or_default();
            for exp in v {
                if !entry.expression.contains(exp) {
                    entry.expression.push(exp.clone());
                }
            }
        }
        for (k, v) in &self.motions {
            let entry = map.entry(k.clone()).or_default();
            for mot in v {
                if !entry.motion.contains(mot) {
                    entry.motion.push(mot.clone());
                }
            }
        }
        map
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Live2dProfile {
    pub model_path: String,
    #[serde(default)]
    pub neutral_expression: String,
    #[serde(default)]
    pub expression_timeout: f32,
    #[serde(default)]
    pub motion_fade_out: f32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_idle_fps")]
    pub idle_fps: u32,
    #[serde(default = "default_supersample")]
    pub supersample: f32,
    pub cues: HashMap<String, CueMapping>,
    #[serde(default)]
    pub hit_areas: HashMap<String, String>,
    #[serde(default)]
    pub gaze: Option<GazeConfig>,
}

fn default_gaze_idle_timeout() -> f32 {
    2.5
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GazeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_gaze_idle_timeout")]
    pub idle_timeout: f32,
    #[serde(default = "default_true")]
    pub blend_with_motions: bool,
    #[serde(default = "default_true")]
    pub pause_in_fullscreen: bool,
}

impl Default for GazeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_timeout: default_gaze_idle_timeout(),
            blend_with_motions: true,
            pause_in_fullscreen: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct InteractionConfig {
    #[serde(default)]
    pub hit_areas: HashMap<String, String>,
    #[serde(default)]
    pub gaze: GazeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_true")]
    pub tts_enabled: bool,
    pub tts_voice: String,
    #[serde(default = "default_tts_rate")]
    pub tts_rate: String,
    #[serde(default = "default_tts_pitch")]
    pub tts_pitch: String,
    #[serde(default = "default_tts_volume")]
    pub tts_volume: String,
    pub beatrice_enabled: bool,
    pub beatrice_model_path: String,
    pub speaker_id: u32,
    pub pitch_shift: f64,
    pub formant_shift: f64,
    pub volume: f32,
}

fn default_tts_rate() -> String {
    "+0%".to_string()
}

fn default_tts_pitch() -> String {
    "+0Hz".to_string()
}

fn default_tts_volume() -> String {
    "+0%".to_string()
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            tts_enabled: true,
            tts_voice: "zh-CN-XiaoyiNeural".to_string(),
            tts_rate: "+0%".to_string(),
            tts_pitch: "+0Hz".to_string(),
            tts_volume: "+0%".to_string(),
            beatrice_enabled: true,
            beatrice_model_path: "models/murasame-beatrice-v1".to_string(),
            speaker_id: 0,
            pitch_shift: 0.0,
            formant_shift: 0.0,
            volume: 1.0,
        }
    }
}

fn default_ai_protocol() -> String {
    "hermes".to_string()
}

fn default_hermes_url() -> String {
    "https://murasame.rarelotus.eu.org:12393/v1".to_string()
}

fn default_hermes_key() -> String {
    "c1a21f28cc43c5772b913c4b5296a07db21342aedd7225accfbe8d2ad0f5521f".to_string()
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
            base_url: None,
            api_key: None,
            model: None,
        }
    }
}

fn default_thinking_intensity() -> String {
    "medium".to_string()
}

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
}

impl Default for BuiltinToolsConfig {
    fn default() -> Self {
        Self {
            use_skill: true,
            get_current_time: true,
            read_file: true,
            execute_command: true,
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_character: default_active_character(),
            window: WindowConfig::default(),
            user: UserConfig::default(),
            ui: UiConfig::default(),
            agent: AgentConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_create(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            let content = fs::read_to_string(path)?;
            let mut config: AppConfig = toml::from_str(&content)?;
            if config.ui.subtitle_y_offset < -30 || config.ui.subtitle_y_offset == 0 {
                config.ui.subtitle_y_offset = 75;
            }
            Ok(config)
        } else {
            let config = AppConfig::default();
            let content = toml::to_string_pretty(&config)?;
            fs::write(path, content)?;
            Ok(config)
        }
    }

    pub fn save_to_disk(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_window_parsing() {
        assert_eq!(parse_context_window_tokens("128k"), 128_000);
        assert_eq!(parse_context_window_tokens("300k"), 300_000);
        assert_eq!(parse_context_window_tokens("600k"), 600_000);
        assert_eq!(parse_context_window_tokens("1m"), 1_000_000);
        assert_eq!(parse_context_window_tokens("2M"), 2_000_000);
        assert_eq!(parse_context_window_tokens("50000"), 50_000);
    }

    #[test]
    fn test_model_config_fps_deserialization_and_default() {
        let toml_without_fps = r#"
            path = "test/model.model3.json"
            scale = 1.0
        "#;
        let cfg: ModelConfig = toml::from_str(toml_without_fps).expect("deserialize without fps");
        assert_eq!(cfg.fps, 60, "ModelConfig should default to 60 FPS when omitted");
        assert_eq!(cfg.idle_fps, 30, "ModelConfig should default to 30 idle FPS when omitted");

        let toml_with_fps = r#"
            path = "test/model.model3.json"
            scale = 1.0
            fps = 120
            idle_fps = 45
        "#;
        let cfg: ModelConfig = toml::from_str(toml_with_fps).expect("deserialize with fps");
        assert_eq!(cfg.fps, 120, "ModelConfig should correctly parse configured fps");
        assert_eq!(cfg.idle_fps, 45, "ModelConfig should correctly parse configured idle_fps");
    }

    #[test]
    fn test_live2d_profile_fps_json_roundtrip() {
        let profile = Live2dProfile {
            model_path: "test/model.model3.json".to_string(),
            fps: 90,
            idle_fps: 40,
            ..Default::default()
        };
        let json = serde_json::to_string(&profile).expect("serialize Live2dProfile");
        let parsed: Live2dProfile = serde_json::from_str(&json).expect("deserialize Live2dProfile");
        assert_eq!(parsed.fps, 90, "Live2dProfile roundtrip should preserve fps");
        assert_eq!(parsed.idle_fps, 40, "Live2dProfile roundtrip should preserve idle_fps");
    }

    #[test]
    fn test_model_config_supersample_deserialization_and_default() {
        let toml_without_ss = r#"
            path = "test/model.model3.json"
            scale = 1.0
        "#;
        let cfg: ModelConfig = toml::from_str(toml_without_ss).expect("deserialize without supersample");
        assert_eq!(cfg.supersample, 2.0, "ModelConfig should default to 2.0 supersample when omitted");

        let toml_with_ss = r#"
            path = "test/model.model3.json"
            scale = 1.0
            supersample = 1.5
        "#;
        let cfg: ModelConfig = toml::from_str(toml_with_ss).expect("deserialize with supersample");
        assert_eq!(cfg.supersample, 1.5, "ModelConfig should correctly parse configured supersample");

        let profile = Live2dProfile {
            model_path: "test/model.model3.json".to_string(),
            supersample: 3.0,
            ..Default::default()
        };
        let json_str = serde_json::to_string(&profile).expect("serialize Live2dProfile");
        let parsed: Live2dProfile = serde_json::from_str(&json_str).expect("deserialize Live2dProfile");
        assert_eq!(parsed.supersample, 3.0, "Live2dProfile roundtrip should preserve supersample");
    }

    #[test]
    fn test_cue_config_list_and_merge() {
        let toml_str = r#"
            [mappings.tilt_head]
            expression = "exp1.exp3"
            motion = ["motion02.motion3.json", "motion06.motion3.json"]

            [mappings.idle]
            expression = ""
            motion = "motion01.motion3.json"

            [motions]
            tilt_head = "motion06.motion3.json"
            bow = "motion02.motion3.json"
        "#;
        let cfg: CueConfig = toml::from_str(toml_str).expect("deserialize CueConfig");
        let merged = cfg.normalized_mappings();

        let tilt = merged.get("tilt_head").expect("tilt_head mapping");
        assert_eq!(tilt.expression, vec!["exp1.exp3"]);
        assert_eq!(
            tilt.motion,
            vec!["motion02.motion3.json", "motion06.motion3.json"]
        );

        let idle = merged.get("idle").expect("idle mapping");
        assert_eq!(idle.motion, vec!["motion01.motion3.json"]);

        let bow = merged.get("bow").expect("bow mapping");
        assert_eq!(bow.motion, vec!["motion02.motion3.json"]);
    }

    #[test]
    fn test_gaze_config_deserialization_and_defaults() {
        let toml_empty = "";
        let interaction: InteractionConfig = toml::from_str(toml_empty).expect("deserialize empty interaction");
        assert!(interaction.gaze.enabled);
        assert_eq!(interaction.gaze.idle_timeout, 2.5);
        assert!(interaction.gaze.blend_with_motions);
        assert!(interaction.gaze.pause_in_fullscreen);

        let toml_custom = r#"
            [gaze]
            enabled = false
            idle_timeout = 4.0
            blend_with_motions = false
            pause_in_fullscreen = false
        "#;
        let interaction_custom: InteractionConfig = toml::from_str(toml_custom).expect("deserialize custom interaction");
        assert!(!interaction_custom.gaze.enabled);
        assert_eq!(interaction_custom.gaze.idle_timeout, 4.0);
        assert!(!interaction_custom.gaze.blend_with_motions);
        assert!(!interaction_custom.gaze.pause_in_fullscreen);
    }

    #[test]
    fn test_window_config_auto_hide_in_fullscreen() {
        let toml_default = r#"
            width = 400
            height = 800
            always_on_top = true
        "#;
        let win_cfg: WindowConfig = toml::from_str(toml_default).expect("deserialize window config default");
        assert!(!win_cfg.auto_hide_in_fullscreen);

        let toml_custom = r#"
            width = 400
            height = 800
            always_on_top = true
            auto_hide_in_fullscreen = true
        "#;
        let win_cfg_custom: WindowConfig = toml::from_str(toml_custom).expect("deserialize window config custom");
        assert!(win_cfg_custom.auto_hide_in_fullscreen);
    }

    #[test]
    fn test_agent_config_defaults_and_merge() {
        let toml_str = r#"
            enabled = true
            skills_enabled = true
            security_level = "auto_execute"
            max_steps = 15

            [mcp_servers.sqlite]
            enabled = true
            transport = "stdio"
            command = "uvx"
            args = ["mcp-server-sqlite", "--db-path", "test.db"]
        "#;
        let cfg: AgentConfig = toml::from_str(toml_str).expect("deserialize agent config");
        assert!(cfg.enabled);
        assert_eq!(cfg.security_level, SecurityLevel::AutoExecute);
        assert_eq!(cfg.max_steps, 15);
        assert_eq!(cfg.mcp_servers.len(), 1);
        assert_eq!(cfg.mcp_servers["sqlite"].command.as_deref(), Some("uvx"));

        let override_cfg = AgentConfig {
            enabled: false,
            skills_enabled: false,
            security_level: SecurityLevel::ReadOnly,
            max_steps: 5,
            live2d_thinking_enabled: false,
            thinking_cue: "awkward".to_string(),
            live2d_tool_cue_enabled: false,
            tool_cue: "gloomy".to_string(),
            subtitle_tool_call_status: false,
            subtitle_thinking_status: true,
            builtin_tools: BuiltinToolsConfig {
                use_skill: false,
                get_current_time: true,
                read_file: false,
                execute_command: false,
            },
            mcp_servers: HashMap::new(),
        };
        let merged = cfg.merge_with(&override_cfg);
        assert!(!merged.enabled);
        assert_eq!(merged.security_level, SecurityLevel::ReadOnly);
        assert_eq!(merged.max_steps, 5);
        assert!(!merged.live2d_thinking_enabled);
        assert_eq!(merged.thinking_cue, "awkward");
        assert!(!merged.live2d_tool_cue_enabled);
        assert_eq!(merged.tool_cue, "gloomy");
        assert!(!merged.builtin_tools.use_skill);
        // Inherited mcp_servers from base
        assert_eq!(merged.mcp_servers.len(), 1);
    }
}

