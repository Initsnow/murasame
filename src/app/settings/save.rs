use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use slint::{ComponentHandle, Model};
use crate::{
    audio::VoiceChanger,
    character::CharacterManager,
    config::AppConfig,
    model::ModelCatalog,
    renderer::Live2dRenderer,
};
use crate::app::{
    ChatWindow, MainWindow, SettingsWindow, SubtitleWindow,
    helpers::{
        format_tts_hz, format_tts_percentage, parse_hex_color,
    },
    platform::update_subtitle_window_position,
};

pub fn setup(
    settings_window: &SettingsWindow,
    window: &MainWindow,
    subtitle_window: &SubtitleWindow,
    chat_window: &ChatWindow,
    config: &Arc<Mutex<AppConfig>>,
    char_manager: &Arc<Mutex<CharacterManager>>,
    model_catalog: &Arc<Mutex<ModelCatalog>>,
    voice_changer: &Arc<Mutex<Option<VoiceChanger>>>,
    live2d_renderer: &Arc<Mutex<Option<Live2dRenderer>>>,
) {
    let config_save = Arc::clone(config);
    let char_mgr_save = Arc::clone(char_manager);
    let vc_save = Arc::clone(voice_changer);
    let live2d_save = Arc::clone(live2d_renderer);
    let catalog_save = Arc::clone(model_catalog);
    let sw_weak_save = settings_window.as_weak();
    let win_weak_save = window.as_weak();
    let sub_weak_save = subtitle_window.as_weak();
    let chat_win_save = chat_window.as_weak();

    settings_window.on_save_clicked(move || {
        if let Some(sw) = sw_weak_save.upgrade() {
            let editing_id = sw.get_editing_char_id().to_string();

            // 1. Update character config
            let (is_active, active_id) = {
                let mut cm = char_mgr_save.lock().unwrap();
                let base_dir = cm.base_dir.clone();
                let active_id = cm.active_id.clone();
                let is_active = editing_id == active_id;

                if let Some(char_cfg) = cm.get_character_mut(&editing_id) {
                    // Audio
                    char_cfg.audio.tts_enabled = sw.get_cfg_tts_enabled();
                    char_cfg.audio.tts_voice = sw.get_cfg_voice().to_string();
                    char_cfg.audio.tts_rate = format_tts_percentage(sw.get_cfg_tts_rate());
                    char_cfg.audio.tts_pitch = format_tts_hz(sw.get_cfg_tts_pitch());
                    char_cfg.audio.tts_volume = format_tts_percentage(sw.get_cfg_tts_volume());
                    char_cfg.audio.beatrice_enabled = sw.get_cfg_beatrice_enabled();
                    char_cfg.audio.beatrice_model_path =
                        sw.get_cfg_beatrice_model_path().to_string();
                    char_cfg.audio.speaker_id = sw.get_cfg_speaker_id() as u32;
                    char_cfg.audio.pitch_shift = sw.get_cfg_pitch_shift() as f64;
                    char_cfg.audio.formant_shift = sw.get_cfg_formant_shift() as f64;
                    char_cfg.audio.volume = sw.get_cfg_volume();

                    // AI
                    char_cfg.ai.protocol = sw.get_cfg_api_protocol().to_string();
                    char_cfg.ai.hermes_base_url =
                        crate::config::normalize_api_base_url(&sw.get_cfg_hermes_url().to_string());
                    char_cfg.ai.hermes_api_key = sw.get_cfg_hermes_key().to_string();
                    char_cfg.ai.hermes_model = sw.get_cfg_hermes_model().to_string();
                    char_cfg.ai.openai_base_url =
                        crate::config::normalize_api_base_url(&sw.get_cfg_openai_url().to_string());
                    char_cfg.ai.openai_api_key = sw.get_cfg_openai_key().to_string();
                    char_cfg.ai.openai_model = sw.get_cfg_openai_model().to_string();

                    if char_cfg.ai.protocol == "hermes" {
                        char_cfg.ai.base_url = Some(char_cfg.ai.hermes_base_url.clone());
                        char_cfg.ai.api_key = Some(char_cfg.ai.hermes_api_key.clone());
                        char_cfg.ai.model = Some(char_cfg.ai.hermes_model.clone());
                    } else {
                        char_cfg.ai.base_url = Some(char_cfg.ai.openai_base_url.clone());
                        char_cfg.ai.api_key = Some(char_cfg.ai.openai_api_key.clone());
                        char_cfg.ai.model = Some(char_cfg.ai.openai_model.clone());
                    }

                    let ctx_win = sw.get_cfg_context_window().to_string();
                    char_cfg.ai.context_window = ctx_win.clone();
                    char_cfg.ai.max_context_tokens =
                        crate::config::parse_context_window_tokens(&ctx_win);
                    char_cfg.ai.compression_strategy =
                        sw.get_cfg_compression_strategy().to_string();
                    char_cfg.ai.system_prompt_prefix = sw.get_cfg_system_prompt().to_string();
                    char_cfg.ai.thinking_enabled = sw.get_cfg_thinking_enabled();
                    char_cfg.ai.thinking_intensity = sw.get_cfg_thinking_intensity().to_string();
                    char_cfg.ai.multimodal_vision = sw.get_cfg_multimodal_vision();

                    // Agent & Skills
                    char_cfg.agent.enabled = true;
                    char_cfg.agent.skills_enabled = true;
                    char_cfg.agent.security_level = match sw.get_cfg_security_level().as_str() {
                        "read_only" => crate::config::SecurityLevel::ReadOnly,
                        "auto_execute" => crate::config::SecurityLevel::AutoExecute,
                        _ => crate::config::SecurityLevel::AskConfirm,
                    };
                    char_cfg.agent.max_steps = (sw.get_cfg_max_steps() as usize).clamp(1, 30);
                    char_cfg.agent.subtitle_tool_call_status = sw.get_cfg_subtitle_tool_call_status();
                    char_cfg.agent.subtitle_thinking_status = sw.get_cfg_subtitle_thinking_status();
                    char_cfg.agent.live2d_thinking_enabled = sw.get_cfg_live2d_thinking_enabled();
                    char_cfg.agent.thinking_cue = sw.get_cfg_thinking_cue().to_string();
                    char_cfg.agent.live2d_tool_cue_enabled = sw.get_cfg_live2d_tool_cue_enabled();
                    char_cfg.agent.tool_cue = sw.get_cfg_tool_cue().to_string();

                    char_cfg.agent.builtin_tools.use_skill = sw.get_cfg_builtin_use_skill();
                    char_cfg.agent.builtin_tools.get_current_time = sw.get_cfg_builtin_get_current_time();
                    char_cfg.agent.builtin_tools.read_file = sw.get_cfg_builtin_read_file();
                    char_cfg.agent.builtin_tools.execute_command = sw.get_cfg_builtin_execute_command();
                    char_cfg.agent.builtin_tools.take_screenshot = sw.get_cfg_builtin_take_screenshot();
                    char_cfg.agent.builtin_tools.screenshot_save_to_disk = sw.get_cfg_builtin_screenshot_save_to_disk();
                    sw.set_cfg_is_vision_model(char_cfg.ai.is_vision_supported());

                    // Live2D
                    char_cfg.model.path = sw.get_cfg_model_path().to_string();
                    char_cfg.model.neutral_expression = sw.get_cfg_neutral_expression().to_string();
                    char_cfg.model.expression_timeout = sw.get_cfg_expression_timeout();
                    char_cfg.model.motion_fade_out = sw.get_cfg_motion_fade_out();
                    char_cfg.model.idle_motion_enabled = sw.get_cfg_idle_motion_enabled();
                    char_cfg.model.idle_motion_target = sw.get_cfg_idle_motion_target().to_string();
                    char_cfg.model.idle_motion_interval = sw.get_cfg_idle_motion_interval();
                    char_cfg.model.fps = (sw.get_cfg_fps() as u32).clamp(15, 144);
                    char_cfg.model.idle_fps = (sw.get_cfg_idle_fps() as u32).clamp(15, char_cfg.model.fps);
                    char_cfg.model.supersample = sw.get_cfg_supersample().clamp(1.0, 4.0);

                    // Gaze
                    char_cfg.interactions.gaze.enabled = sw.get_cfg_gaze_enabled();
                    char_cfg.interactions.gaze.idle_timeout = sw.get_cfg_gaze_idle_timeout();
                    char_cfg.interactions.gaze.blend_with_motions = sw.get_cfg_gaze_blend_with_motions();
                    char_cfg.interactions.gaze.pause_in_fullscreen = sw.get_cfg_gaze_pause_in_fullscreen();

                    // Proactive speaking
                    char_cfg.interactions.proactive.enabled = sw.get_cfg_proactive_enabled();
                    let min_inv = (sw.get_cfg_proactive_min_interval() as u32).max(1);
                    let max_inv = (sw.get_cfg_proactive_max_interval() as u32).max(min_inv);
                    char_cfg.interactions.proactive.min_interval_minutes = min_inv;
                    char_cfg.interactions.proactive.max_interval_minutes = max_inv;
                    char_cfg.interactions.proactive.prompt = sw.get_cfg_proactive_prompt().to_string();

                    let cue_model = sw.get_cue_list();
                    let mut mappings = HashMap::new();
                    for i in 0..cue_model.row_count() {
                        if let Some(item) = cue_model.row_data(i) {
                            let name = item.name.to_string();
                            if !name.is_empty() {
                                mappings.insert(
                                    name,
                                    crate::config::CueMapping::from_strings(
                                        &item.expression,
                                        &item.motion,
                                        &item.motion_group,
                                    ),
                                );
                            }
                        }
                    }
                    char_cfg.cues.mappings = mappings;

                    let _ = char_cfg.save_to_disk(&base_dir);
                }
                (is_active, active_id)
            };

            // 2. Update Global config (UI & User & Window)
            {
                let mut cfg = config_save.lock().unwrap();
                cfg.ui.toolbar_enabled = sw.get_cfg_toolbar_enabled();
                cfg.ui.subtitle_enabled = sw.get_cfg_subtitle_enabled();
                cfg.ui.subtitle_bubble_enabled = sw.get_cfg_subtitle_bubble_enabled();
                cfg.ui.subtitle_font_size = sw.get_cfg_subtitle_font_size() as u32;
                cfg.ui.subtitle_color = sw.get_cfg_subtitle_color().to_string();
                cfg.ui.subtitle_y_offset = sw.get_cfg_subtitle_y_offset();

                cfg.window.auto_hide_in_fullscreen = sw.get_cfg_auto_hide_in_fullscreen();
                cfg.window.always_on_top = sw.get_cfg_always_on_top();

                cfg.user.avatar_enabled = sw.get_cfg_user_avatar_enabled();

                cfg.agent.enabled = true;
                cfg.agent.skills_enabled = true;
                cfg.agent.security_level = match sw.get_cfg_security_level().as_str() {
                    "read_only" => crate::config::SecurityLevel::ReadOnly,
                    "auto_execute" => crate::config::SecurityLevel::AutoExecute,
                    _ => crate::config::SecurityLevel::AskConfirm,
                };
                cfg.agent.max_steps = (sw.get_cfg_max_steps() as usize).clamp(1, 30);
                cfg.agent.subtitle_tool_call_status = sw.get_cfg_subtitle_tool_call_status();
                cfg.agent.subtitle_thinking_status = sw.get_cfg_subtitle_thinking_status();
                cfg.agent.live2d_thinking_enabled = sw.get_cfg_live2d_thinking_enabled();
                cfg.agent.thinking_cue = sw.get_cfg_thinking_cue().to_string();
                cfg.agent.live2d_tool_cue_enabled = sw.get_cfg_live2d_tool_cue_enabled();
                cfg.agent.tool_cue = sw.get_cfg_tool_cue().to_string();

                cfg.agent.builtin_tools.use_skill = sw.get_cfg_builtin_use_skill();
                cfg.agent.builtin_tools.get_current_time = sw.get_cfg_builtin_get_current_time();
                cfg.agent.builtin_tools.read_file = sw.get_cfg_builtin_read_file();
                cfg.agent.builtin_tools.execute_command = sw.get_cfg_builtin_execute_command();
                cfg.agent.builtin_tools.take_screenshot = sw.get_cfg_builtin_take_screenshot();
                cfg.agent.builtin_tools.screenshot_save_to_disk = sw.get_cfg_builtin_screenshot_save_to_disk();

                let _ = cfg.save_to_disk(crate::paths::config_path());
            }

            let cfg_clone = config_save.lock().unwrap().clone();
            if let Some(win) = win_weak_save.upgrade() {
                win.set_toolbar_visible(cfg_clone.ui.toolbar_enabled);
                #[cfg(not(windows))]
                {
                    win.set_subtitle_enabled(cfg_clone.ui.subtitle_enabled);
                    win.set_bubble_enabled(cfg_clone.ui.subtitle_bubble_enabled);
                    win.set_subtitle_font_size(cfg_clone.ui.subtitle_font_size as f32);
                    win.set_subtitle_color(parse_hex_color(&cfg_clone.ui.subtitle_color));
                    win.set_subtitle_y_offset_percent(cfg_clone.ui.subtitle_y_offset);
                }
            }
            if let Some(sub) = sub_weak_save.upgrade() {
                sub.set_subtitle_enabled(cfg_clone.ui.subtitle_enabled);
                sub.set_bubble_enabled(cfg_clone.ui.subtitle_bubble_enabled);
                sub.set_subtitle_font_size(cfg_clone.ui.subtitle_font_size as f32);
                sub.set_subtitle_color(parse_hex_color(&cfg_clone.ui.subtitle_color));
                #[cfg(windows)]
                {
                    crate::app::platform::SUBTITLE_Y_OFFSET.store(cfg_clone.ui.subtitle_y_offset, std::sync::atomic::Ordering::Relaxed);
                    crate::app::platform::SUBTITLE_FONT_SIZE.store(cfg_clone.ui.subtitle_font_size as i32, std::sync::atomic::Ordering::Relaxed);
                }
                update_subtitle_window_position(cfg_clone.ui.subtitle_y_offset);
            }
            if let Some(chat_win) = chat_win_save.upgrade() {
                chat_win.set_user_avatar_enabled(cfg_clone.user.avatar_enabled);
            }

            // 3. If edited character is the active character, apply live updates to renderer and voice changer
            if is_active {
                let cm = char_mgr_save.lock().unwrap();
                if let Some(active_char) = cm.get_character(&active_id) {
                    crate::app::render_loop::update_render_fps(active_char.model.fps, active_char.model.idle_fps);

                    if let Ok(mut r_guard) = live2d_save.lock() {
                        if let Some(renderer) = r_guard.as_mut() {
                            let neutral_path = catalog_save
                                .lock()
                                .unwrap()
                                .available_expressions
                                .get(&active_char.model.neutral_expression)
                                .cloned();
                            renderer.set_neutral_expression(neutral_path);
                            renderer.set_expression_timeout(active_char.model.expression_timeout);
                            renderer
                                .set_expression_fade_time(active_char.model.expression_fade_time);
                            renderer.set_motion_fade_out(active_char.model.motion_fade_out);
                            renderer.set_supersample(active_char.model.supersample);
                            renderer.set_gaze_config(active_char.interactions.gaze.clone());
                        }
                    }

                    if active_char.audio.beatrice_enabled {
                        let beatrice_resolved = crate::paths::resolve_resource_path(
                            &active_char.audio.beatrice_model_path,
                        );
                        match VoiceChanger::new(
                            &beatrice_resolved,
                            active_char.audio.speaker_id,
                            active_char.audio.pitch_shift,
                            active_char.audio.formant_shift,
                        ) {
                            Ok(new_vc) => {
                                if let Ok(mut vc_guard) = vc_save.lock() {
                                    *vc_guard = Some(new_vc);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to reload Beatrice voice changer: {e}");
                            }
                        }
                    }
                }
            }

            let _ = sw.hide();
        }
    });
}
