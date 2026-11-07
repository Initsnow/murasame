use serde::{Deserialize, Serialize};
use super::{default_false, default_true};

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

