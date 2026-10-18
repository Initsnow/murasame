use super::{CharacterItem, SettingsWindow};
use anyhow::Result;
use std::path::Path;
use std::time::Duration;

pub fn load_image_or_empty(path: impl AsRef<Path>) -> slint::Image {
    let p = path.as_ref();
    if !p.exists() {
        return slint::Image::default();
    }
    // 1. Try decoding via image crate with content-based magic-byte detection
    if let Ok(reader) = image::ImageReader::open(p) {
        if let Ok(reader) = reader.with_guessed_format() {
            if let Ok(dyn_img) = reader.decode() {
                // Downscale large avatar images to max 256x256 to save memory (e.g. 2000x2000 -> 256x256 saves ~98.4% RAM/VRAM)
                let dyn_img = if dyn_img.width() > 256 || dyn_img.height() > 256 {
                    dyn_img.thumbnail(256, 256)
                } else {
                    dyn_img
                };
                let rgba = dyn_img.into_rgba8();
                let (w, h) = rgba.dimensions();
                let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                    rgba.as_raw(),
                    w,
                    h,
                );
                return slint::Image::from_rgba8(buffer);
            }
        }
    }
    // 2. Fallback to slint's built-in loader
    slint::Image::load_from_path(p).unwrap_or_default()
}

pub fn build_character_items(manager: &crate::character::CharacterManager) -> Vec<CharacterItem> {
    manager
        .list_summaries()
        .into_iter()
        .map(|s| CharacterItem {
            id: s.id.into(),
            name: s.name.into(),
            is_active: s.is_active,
        })
        .collect()
}

pub const ALL_EDGE_VOICES: &[(&str, &str)] = &[
    ("zh-CN-XiaoyiNeural", "普通话女声-软萌甜美"),
    ("zh-CN-XiaoxiaoNeural", "普通话女声-温暖活泼"),
    ("zh-CN-YunjianNeural", "普通话男声-沉稳磁性"),
    ("zh-CN-YunxiNeural", "普通话男声-少年清脆"),
    ("zh-CN-YunyangNeural", "普通话男声-播音专业"),
    ("zh-CN-YunfengNeural", "普通话男声-自然明亮"),
    ("zh-CN-XiaozheNeural", "普通话男声-沉静"),
    ("zh-CN-liaoning-XiaobeiNeural", "东北话女声-幽默风趣"),
    ("zh-CN-shaanxi-XiaoniNeural", "陕西话女声-质朴"),
    ("zh-CN-shandong-YunxiangNeural", "山东话男声"),
    ("zh-CN-sichuan-YunxiNeural", "四川话男声"),
    ("zh-HK-HiuMaanNeural", "粤语女声-晓曼"),
    ("zh-HK-WanLungNeural", "粤语男声-云龙"),
    ("zh-TW-HsiaoChenNeural", "台湾国语女声-晓臻"),
    ("zh-TW-YunJheNeural", "台湾国语男声-云哲"),
    ("ja-JP-NanamiNeural", "日文女声-七海/萌妹"),
    ("ja-JP-AoiNeural", "日文女声-葵/元气"),
    ("ja-JP-KeitaNeural", "日文男声-圭太/少年"),
    ("en-US-AnaNeural", "英文女声-少女"),
    ("en-US-JennyNeural", "英文女声-自然"),
    ("en-US-GuyNeural", "英文男声-青年"),
];

pub fn parse_hex_color(s: &str) -> slint::Color {
    let s = s.trim().trim_start_matches('#');
    if let Ok(val) = u32::from_str_radix(s, 16) {
        if s.len() == 6 {
            return slint::Color::from_argb_encoded(0xFF000000 | val);
        } else if s.len() == 8 {
            return slint::Color::from_argb_encoded(val);
        }
    }
    slint::Color::from_argb_encoded(0xFFFFFFFF)
}

pub fn parse_tts_percentage(s: &str) -> f32 {
    let clean = s.trim().trim_end_matches('%');
    clean.parse::<f32>().unwrap_or(0.0)
}

pub fn format_tts_percentage(val: f32) -> String {
    let rounded = val.round() as i32;
    if rounded >= 0 {
        format!("+{rounded}%")
    } else {
        format!("{rounded}%")
    }
}

pub fn parse_tts_hz(s: &str) -> f32 {
    let clean = s.trim().trim_end_matches("Hz").trim_end_matches("hz");
    clean.parse::<f32>().unwrap_or(0.0)
}

pub fn format_tts_hz(val: f32) -> String {
    let rounded = val.round() as i32;
    if rounded >= 0 {
        format!("+{rounded}Hz")
    } else {
        format!("{rounded}Hz")
    }
}

pub fn detect_and_set_beatrice_info(settings: &SettingsWindow, dir_path: &str) {
    let p = Path::new(dir_path);
    if !p.exists() {
        settings.set_beatrice_model_info("路径不存在".into());
        settings.set_beatrice_speakers(slint::ModelRc::new(slint::VecModel::default()));
        settings.set_current_speaker_name("".into());
        return;
    }
    if let Ok(entries) = std::fs::read_dir(p) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Ok(toml) = crate::audio::beatrice::BeatriceToml::load_from_tomlpath(&path) {
                    let voices = toml.get_voices_sorted();
                    let current_id = settings.get_cfg_speaker_id() as u32;
                    let mut speaker_strings = Vec::new();
                    let mut found_name = String::new();

                    for (id, v) in &voices {
                        let desc = format!("{id}: {} (基准音高 {:.1})", v.name, v.average_pitch);
                        speaker_strings.push(slint::SharedString::from(desc));
                        if *id == current_id {
                            found_name = format!("{id}: {}", v.name);
                        }
                    }

                    if found_name.is_empty() {
                        if let Some((first_id, first_v)) = voices.first() {
                            found_name = format!("{first_id}: {}", first_v.name);
                            settings.set_cfg_speaker_id(*first_id as i32);
                        }
                    }

                    settings.set_current_speaker_name(found_name.into());
                    settings.set_beatrice_speakers(slint::ModelRc::new(slint::VecModel::from(
                        speaker_strings,
                    )));

                    settings.set_beatrice_model_info(
                        format!(
                            "{}: {} | 可选发音人: {} 位",
                            toml.model.name,
                            toml.model.version,
                            voices.len()
                        )
                        .into(),
                    );
                    return;
                }
            }
        }
    }
    settings.set_beatrice_model_info("未检测到有效 Beatrice toml 配置文件".into());
    settings.set_beatrice_speakers(slint::ModelRc::new(slint::VecModel::default()));
    settings.set_current_speaker_name("".into());
}

pub async fn fetch_models_list(base_url: &str, api_key: &str) -> Result<Vec<String>> {
    let clean_url = crate::config::normalize_api_base_url(base_url);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;

    let models_endpoint = format!("{clean_url}/models");

    let mut req = client.get(&models_endpoint);
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key.trim());
    }

    if let Ok(resp) = req.send().await {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                let mut list = Vec::new();
                if let Some(arr) = json.get("data").and_then(|d| d.as_array()) {
                    for item in arr {
                        if let Some(id) = item.get("id").and_then(|s| s.as_str()) {
                            list.push(id.to_string());
                        }
                    }
                }
                if !list.is_empty() {
                    list.sort();
                    list.dedup();
                    return Ok(list);
                }
            }
        }
    }

    // Fallback: 本地 Ollama 原生端点 /api/tags
    let ollama_endpoint = if clean_url.ends_with("/v1") {
        let base = clean_url.trim_end_matches("/v1");
        format!("{base}/api/tags")
    } else {
        format!("{clean_url}/api/tags")
    };

    if let Ok(resp) = client.get(&ollama_endpoint).send().await {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                    let mut list = Vec::new();
                    for item in models {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            list.push(name.to_string());
                        }
                    }
                    if !list.is_empty() {
                        list.sort();
                        list.dedup();
                        return Ok(list);
                    }
                }
            }
        }
    }

    anyhow::bail!("未能从 API 获取模型列表，请确认 Base URL 与网络是否畅通")
}
