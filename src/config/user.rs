use serde::{Deserialize, Serialize};
use super::default_true;

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
