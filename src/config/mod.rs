use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub mod agent;
pub mod ai;
pub mod audio;
pub mod cue;
pub mod model;
pub mod serde_helpers;
pub mod ui;
pub mod user;
pub mod window;

pub use agent::*;
pub use ai::*;
pub use audio::*;
pub use cue::*;
pub use model::*;
pub use serde_helpers::*;
pub use ui::*;
pub use user::*;
pub use window::*;

pub fn default_true() -> bool {
    true
}

pub fn default_false() -> bool {
    false
}

fn default_active_character() -> String {
    "murasame".to_string()
}

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
    use std::collections::HashMap;

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
    fn test_proactive_config_deserialization_and_defaults() {
        let toml_empty = "";
        let interaction: InteractionConfig =
            toml::from_str(toml_empty).expect("deserialize empty interaction");
        assert!(!interaction.proactive.enabled);
        assert_eq!(interaction.proactive.min_interval_minutes, 15);
        assert_eq!(interaction.proactive.max_interval_minutes, 45);
        assert!(interaction.proactive.prompt.contains("engaging"));

        let toml_custom = r#"
            [proactive]
            enabled = true
            min_interval_minutes = 20
            max_interval_minutes = 60
            prompt = "自定义搭话提示词"
        "#;
        let interaction_custom: InteractionConfig =
            toml::from_str(toml_custom).expect("deserialize custom interaction");
        assert!(interaction_custom.proactive.enabled);
        assert_eq!(interaction_custom.proactive.min_interval_minutes, 20);
        assert_eq!(interaction_custom.proactive.max_interval_minutes, 60);
        assert_eq!(interaction_custom.proactive.prompt, "自定义搭话提示词");
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
                take_screenshot: false,
                screenshot_save_to_disk: false,
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

    #[test]
    fn test_vision_model_config() {
        let mut ai_cfg = AiConfig::default();
        assert!(ai_cfg.is_vision_supported());

        ai_cfg.multimodal_vision = false;
        assert!(!ai_cfg.is_vision_supported());

        ai_cfg.multimodal_vision = true;
        assert!(ai_cfg.is_vision_supported());
    }
}

