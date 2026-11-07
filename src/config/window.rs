use serde::{Deserialize, Serialize};

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
