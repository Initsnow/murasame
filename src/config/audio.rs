use serde::{Deserialize, Serialize};
use super::default_true;

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
