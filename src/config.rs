use std::{collections::HashMap, fs, path::Path};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub cues: CueConfig,
    #[serde(default)]
    pub interactions: InteractionConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub ai: AiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub always_on_top: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 800,
            height: 900,
            always_on_top: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub path: String,
    pub scale: f32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            path: "Murasame_live2d/Murasame.model3.json".to_string(),
            scale: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CueConfig {
    #[serde(default)]
    pub expressions: HashMap<String, String>,
    #[serde(default)]
    pub motions: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InteractionConfig {
    #[serde(default)]
    pub hit_areas: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub tts_voice: String,
    pub beatrice_enabled: bool,
    pub beatrice_model_path: String,
    pub speaker_id: u32,
    pub pitch_shift: f64,
    pub formant_shift: f64,
    pub volume: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            tts_voice: "zh-CN-XiaoyiNeural".to_string(),
            beatrice_enabled: true,
            beatrice_model_path: r"C:\Users\Initsnow\Documents\eris-beatrice-v1".to_string(),
            speaker_id: 0,
            pitch_shift: 0.0,
            formant_shift: 0.0,
            volume: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub system_prompt_prefix: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".to_string(),
            api_key: "ollama".to_string(),
            model: "hermes-3".to_string(),
            system_prompt_prefix: "你是丛雨，一位可爱活泼的剑灵少女。".to_string(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut expressions = HashMap::new();
        expressions.insert("smile".to_string(), "exp1.exp3".to_string());
        expressions.insert("angry".to_string(), "exp2.exp3".to_string());
        expressions.insert("sad".to_string(), "exp3.exp3".to_string());
        expressions.insert("dark".to_string(), "exp4.exp3".to_string());
        expressions.insert("panic".to_string(), "exp5.exp3".to_string());
        expressions.insert("shy".to_string(), "exp6.exp3".to_string());
        expressions.insert("smug".to_string(), "exp7.exp3".to_string());
        expressions.insert("neutral".to_string(), "neutral.exp3".to_string());

        let mut motions = HashMap::new();
        motions.insert("greet".to_string(), "motion02.motion3.json".to_string());
        motions.insert("talk".to_string(), "motion03.motion3.json".to_string());
        motions.insert("show".to_string(), "motion04.motion3.json".to_string());
        motions.insert("relieved".to_string(), "motion05.motion3.json".to_string());
        motions.insert("deny".to_string(), "motion08.motion3.json".to_string());
        motions.insert("pout".to_string(), "motion10.motion3.json".to_string());
        motions.insert("morning".to_string(), "motion11.motion3.json".to_string());

        let mut hit_areas = HashMap::new();
        hit_areas.insert("face".to_string(), "Tapface".to_string());
        hit_areas.insert("hair".to_string(), "Taphair".to_string());
        hit_areas.insert("xiongbu".to_string(), "Tapxiongbu".to_string());
        hit_areas.insert("qunzi".to_string(), "Tapqunzi".to_string());
        hit_areas.insert("leg".to_string(), "Tapleg".to_string());

        Self {
            window: WindowConfig::default(),
            model: ModelConfig::default(),
            cues: CueConfig {
                expressions,
                motions,
            },
            interactions: InteractionConfig { hit_areas },
            audio: AudioConfig::default(),
            ai: AiConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_create(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            let content = fs::read_to_string(path)?;
            let config: AppConfig = toml::from_str(&content)?;
            Ok(config)
        } else {
            let config = AppConfig::default();
            let content = toml::to_string_pretty(&config)?;
            fs::write(path, content)?;
            Ok(config)
        }
    }
}
