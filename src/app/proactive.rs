use chrono::{Datelike, Timelike};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use crate::{
    app::{
        chat::{
            execute_proactive_speaking, get_last_interaction_ts, record_interaction, IS_GENERATING,
        },
        chat_window::ChatManager,
        ChatWindow, MainWindow, SubtitleWindow,
    },
    audio::{AudioPlayer, TtsEngine, VoiceChanger},
    character::CharacterManager,
    config::AppConfig,
    db::Database,
    model::ModelCatalog,
    renderer::Live2dRenderer,
};

/// 返回星期几的中文描述
pub fn get_weekday_description(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Mon => "星期一",
        chrono::Weekday::Tue => "星期二",
        chrono::Weekday::Wed => "星期三",
        chrono::Weekday::Thu => "星期四",
        chrono::Weekday::Fri => "星期五",
        chrono::Weekday::Sat => "星期六",
        chrono::Weekday::Sun => "星期日",
    }
}

/// 返回星期几的英文描述
pub fn get_weekday_en(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
}

/// 判断大模型主动搭话生成的内容是否判定为“保持安静 / 不输出”
pub fn is_silent_proactive_response(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    // 剥离可能的外层包裹符号：方括号、尖括号、圆括号、引号、花括号、中文括号等
    let clean = trimmed
        .trim_matches(|c: char| {
            c == '['
                || c == ']'
                || c == '<'
                || c == '>'
                || c == '('
                || c == ')'
                || c == '（'
                || c == '）'
                || c == '{'
                || c == '}'
                || c == '"'
                || c == '\''
                || c == '`'
                || c == '【'
                || c == '】'
        })
        .trim();

    clean.is_empty()
        || clean.eq_ignore_ascii_case("silence")
        || clean.eq_ignore_ascii_case("silent")
        || clean.eq_ignore_ascii_case("empty")
        || clean.eq_ignore_ascii_case("none")
        || clean.eq_ignore_ascii_case("null")
        || clean.eq_ignore_ascii_case("no response")
        || clean.eq_ignore_ascii_case("no_response")
        || clean == "保持安静"
        || clean == "静音"
        || clean == "保持沉默"
        || clean == "沉默"
        || clean == "无"
        || clean == "无需回话"
        || clean == "不打扰"
}

/// 组装主动搭话的系统隐式提示词，注入时间与空闲状态
pub fn format_proactive_prompt(template: &str, elapsed_secs: i64) -> String {
    let now = chrono::Local::now();
    let time_str = now.format("%Y-%m-%d %H:%M").to_string();
    let date_str = now.format("%Y-%m-%d").to_string();
    let hour_str = now.hour().to_string();
    let weekday_str = get_weekday_description(now.weekday());
    let weekday_en = get_weekday_en(now.weekday());
    let idle_minutes = (elapsed_secs / 60).max(1);

    template
        .replace("{time}", &time_str)
        .replace("{date}", &date_str)
        .replace("{hour}", &hour_str)
        .replace("{weekday}", weekday_str)
        .replace("{weekday_en}", weekday_en)
        .replace("{idle_minutes}", &idle_minutes.to_string())
}

/// 在最小与最大分钟区间内生成随机等待秒数
pub fn pick_random_interval_secs(min_minutes: u32, max_minutes: u32) -> u64 {
    let min_m = min_minutes.max(1);
    let max_m = max_minutes.max(min_m);
    let min_secs = min_m as u64 * 60;
    let max_secs = max_m as u64 * 60;
    if min_secs >= max_secs {
        min_secs
    } else {
        rand::Rng::gen_range(&mut rand::thread_rng(), min_secs..=max_secs)
    }
}

/// 启动主动搭话后台轮询协程
pub fn spawn_proactive_loop(
    config: Arc<Mutex<AppConfig>>,
    char_manager: Arc<Mutex<CharacterManager>>,
    chat_manager: Arc<ChatManager>,
    db: Arc<Database>,
    model_catalog: Arc<Mutex<ModelCatalog>>,
    tokio_rt: Arc<tokio::runtime::Runtime>,
    tts_engine: Arc<TtsEngine>,
    voice_changer: Arc<Mutex<Option<VoiceChanger>>>,
    audio_player: Arc<Mutex<AudioPlayer>>,
    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    is_speaking: Arc<AtomicBool>,
    subtitle_window: slint::Weak<SubtitleWindow>,
    chat_window: Option<slint::Weak<ChatWindow>>,
    main_window: Option<slint::Weak<MainWindow>>,
) {
    // 记录启动时刻为最初交互时间
    record_interaction();

    let initial_interval = {
        let cm = char_manager.lock().unwrap();
        if let Some(c) = cm.get_active_character() {
            pick_random_interval_secs(
                c.interactions.proactive.min_interval_minutes,
                c.interactions.proactive.max_interval_minutes,
            )
        } else {
            15 * 60
        }
    };

    let rt = Arc::clone(&tokio_rt);
    tokio_rt.spawn(async move {
        let mut target_interval_secs = initial_interval;
        tracing::info!(
            "Proactive speaking scheduler started. Target interval: {}s ({} min)",
            target_interval_secs,
            target_interval_secs / 60
        );

        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let proactive_cfg = {
                let cm = char_manager.lock().unwrap();
                if let Some(c) = cm.get_active_character() {
                    c.interactions.proactive.clone()
                } else {
                    continue;
                }
            };

            if !proactive_cfg.enabled {
                continue;
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let last_ts = get_last_interaction_ts();
            let elapsed = now.saturating_sub(last_ts) as u64;

            if elapsed >= target_interval_secs {
                // 1. 严格互斥保护：检查是否正在说话或生成中
                if is_speaking.load(Ordering::Relaxed) || IS_GENERATING.load(Ordering::SeqCst) {
                    tracing::debug!(
                        "Proactive speaking postponed: character is speaking or generating"
                    );
                    continue;
                }

                // 2. 检查音频播放器是否正在出声
                let is_audio_playing =
                    audio_player.lock().map(|p| p.is_playing()).unwrap_or(false);
                if is_audio_playing {
                    tracing::debug!("Proactive speaking postponed: audio player is playing");
                    continue;
                }

                tracing::info!("Proactive speaking triggered after {}s idle", elapsed);

                // 立即重置交互时间并抽取下一次随机时间，防止下一个 5s 周期重复触发
                record_interaction();
                target_interval_secs = pick_random_interval_secs(
                    proactive_cfg.min_interval_minutes,
                    proactive_cfg.max_interval_minutes,
                );

                let formatted_prompt =
                    format_proactive_prompt(&proactive_cfg.prompt, elapsed as i64);
                let session_id = chat_manager.get_active_session_id();

                execute_proactive_speaking(
                    formatted_prompt,
                    session_id,
                    Arc::clone(&config),
                    Arc::clone(&char_manager),
                    Arc::clone(&db),
                    Arc::clone(&model_catalog),
                    Arc::clone(&rt),
                    Arc::clone(&tts_engine),
                    Arc::clone(&voice_changer),
                    Arc::clone(&audio_player),
                    Arc::clone(&live2d_renderer),
                    Arc::clone(&is_speaking),
                    subtitle_window.clone(),
                    chat_window.clone(),
                    main_window.clone(),
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pick_random_interval_range() {
        for _ in 0..50 {
            let secs = pick_random_interval_secs(10, 20);
            assert!(secs >= 600 && secs <= 1200);
        }

        // Test boundary when min > max
        let clamped = pick_random_interval_secs(30, 10);
        assert_eq!(clamped, 1800);
    }

    #[test]
    fn test_format_proactive_prompt_substitution() {
        let template = "时间: {time}, 日期: {date}, 小时: {hour}, 星期: {weekday}({weekday_en}), 静默: {idle_minutes}分钟";
        let formatted = format_proactive_prompt(template, 1800);
        assert!(!formatted.contains("{time}"));
        assert!(!formatted.contains("{date}"));
        assert!(!formatted.contains("{hour}"));
        assert!(!formatted.contains("{weekday}"));
        assert!(!formatted.contains("{weekday_en}"));
        assert!(formatted.contains("静默: 30分钟"));
    }

    #[test]
    fn test_format_proactive_prompt_english_template() {
        let template = "Please say something that would be engaging and appropriate for the current context (current time: {time}, user has been idle for {idle_minutes} minutes). If you feel it is better not to interrupt the user right now, simply output nothing or [SILENCE].";
        let formatted = format_proactive_prompt(template, 1800);
        assert!(!formatted.contains("{time}"));
        assert!(!formatted.contains("{idle_minutes}"));
        assert!(formatted.contains("idle for 30 minutes"));
    }

    #[test]
    fn test_is_silent_proactive_response() {
        // True cases (silence requested)
        assert!(is_silent_proactive_response(""));
        assert!(is_silent_proactive_response("   \n\t  "));
        assert!(is_silent_proactive_response("[SILENCE]"));
        assert!(is_silent_proactive_response("[silence]"));
        assert!(is_silent_proactive_response("<silent>"));
        assert!(is_silent_proactive_response("<silence>"));
        assert!(is_silent_proactive_response("(保持安静)"));
        assert!(is_silent_proactive_response("[保持沉默]"));
        assert!(is_silent_proactive_response("EMPTY"));
        assert!(is_silent_proactive_response("null"));
        assert!(is_silent_proactive_response("none"));
        assert!(is_silent_proactive_response("不打扰"));
        assert!(is_silent_proactive_response("【沉默】"));

        // False cases (actual natural language speech)
        assert!(!is_silent_proactive_response("今天天气真好呀"));
        assert!(!is_silent_proactive_response("你在忙什么呢？"));
        assert!(!is_silent_proactive_response("Hello there!"));
    }
}
