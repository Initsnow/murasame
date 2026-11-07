use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use super::ai::ProactiveConfig;
use super::cue::CueMapping;
use super::default_true;

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
    #[serde(default)]
    pub proactive: ProactiveConfig,
}
