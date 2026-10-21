use slint::{ComponentHandle, Model};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    audio::{AudioPlayer, TtsEngine, VoiceChanger},
    character::{CharacterConfig, CharacterManager},
    config::AppConfig,
    model::ModelCatalog,
    renderer::Live2dRenderer,
};

use super::{
    ChatWindow, CueItem, MainWindow, McpServerItem, MurasameTray, SettingsWindow, SkillItem,
    SubtitleWindow,
    helpers::{
        ALL_EDGE_VOICES, build_character_items, detect_and_set_beatrice_info, fetch_models_list,
        format_tts_hz, format_tts_percentage, load_image_or_empty, parse_hex_color, parse_tts_hz,
        parse_tts_percentage,
    },
    platform::update_subtitle_window_position,
};

pub fn build_mcp_server_items(
    servers: &HashMap<String, crate::config::McpServerConfig>,
) -> Vec<McpServerItem> {
    let mut list: Vec<McpServerItem> = Vec::new();
    let mut keys: Vec<_> = servers.keys().cloned().collect();
    keys.sort();
    for k in keys {
        if let Some(srv) = servers.get(&k) {
            let detail = if srv.transport == "sse" {
                srv.url.clone().unwrap_or_else(|| "未指定 URL".to_string())
            } else {
                let cmd = srv.command.clone().unwrap_or_default();
                let args_str = srv.args.join(" ");
                format!("{cmd} {args_str}").trim().to_string()
            };
            list.push(McpServerItem {
                name: k.into(),
                transport: srv.transport.clone().into(),
                detail: detail.into(),
                enabled: srv.enabled,
            });
        }
    }
    list
}

pub fn refresh_mcp_servers_ui(
    sw: &SettingsWindow,
    servers: &HashMap<String, crate::config::McpServerConfig>,
) {
    let list = build_mcp_server_items(servers);
    sw.set_mcp_servers(slint::ModelRc::new(slint::VecModel::from(list)));
}

pub fn refresh_skills_ui(
    sw: &SettingsWindow,
    skills_mgr: &crate::ai::skills::SkillManager,
) {
    let skill_items: Vec<SkillItem> = skills_mgr
        .list_skills()
        .into_iter()
        .map(|s| SkillItem {
            name: s.metadata.name.clone().into(),
            description: s.metadata.description.clone().into(),
            is_character_specific: s.is_character_specific,
        })
        .collect();
    sw.set_skills(slint::ModelRc::new(slint::VecModel::from(skill_items)));
}

pub fn populate_character_settings(
    sw: &SettingsWindow,
    char_cfg: &CharacterConfig,
    base_dir: &Path,
    is_active: bool,
) {
    sw.set_editing_char_id(char_cfg.id.clone().into());
    sw.set_editing_char_name(char_cfg.name.clone().into());
    sw.set_editing_char_avatar(load_image_or_empty(char_cfg.get_avatar_full_path(base_dir)));
    sw.set_editing_char_is_active(is_active);

    // Audio
    sw.set_cfg_tts_enabled(char_cfg.audio.tts_enabled);
    sw.set_cfg_voice(char_cfg.audio.tts_voice.clone().into());
    sw.set_cfg_tts_rate(parse_tts_percentage(&char_cfg.audio.tts_rate));
    sw.set_cfg_tts_pitch(parse_tts_hz(&char_cfg.audio.tts_pitch));
    sw.set_cfg_tts_volume(parse_tts_percentage(&char_cfg.audio.tts_volume));
    sw.set_cfg_beatrice_enabled(char_cfg.audio.beatrice_enabled);
    sw.set_cfg_beatrice_model_path(char_cfg.audio.beatrice_model_path.clone().into());
    sw.set_cfg_speaker_id(char_cfg.audio.speaker_id as i32);
    sw.set_cfg_pitch_shift(char_cfg.audio.pitch_shift as f32);
    sw.set_cfg_formant_shift(char_cfg.audio.formant_shift as f32);
    sw.set_cfg_volume(char_cfg.audio.volume);
    detect_and_set_beatrice_info(sw, &char_cfg.audio.beatrice_model_path);

    // AI
    sw.set_cfg_api_protocol(char_cfg.ai.protocol.clone().into());
    sw.set_cfg_hermes_url(char_cfg.ai.hermes_base_url.clone().into());
    sw.set_cfg_hermes_key(char_cfg.ai.hermes_api_key.clone().into());
    sw.set_cfg_hermes_model(char_cfg.ai.hermes_model.clone().into());
    sw.set_cfg_openai_url(char_cfg.ai.openai_base_url.clone().into());
    sw.set_cfg_openai_key(char_cfg.ai.openai_api_key.clone().into());
    sw.set_cfg_openai_model(char_cfg.ai.openai_model.clone().into());
    sw.set_cfg_context_window(char_cfg.ai.context_window.clone().into());
    sw.set_cfg_compression_strategy(char_cfg.ai.compression_strategy.clone().into());
    sw.set_cfg_system_prompt(char_cfg.ai.system_prompt_prefix.clone().into());
    sw.set_cfg_thinking_enabled(char_cfg.ai.thinking_enabled);
    sw.set_cfg_thinking_intensity(char_cfg.ai.thinking_intensity.clone().into());

    // Agent & Skills
    let sec_str = match char_cfg.agent.security_level {
        crate::config::SecurityLevel::ReadOnly => "read_only",
        crate::config::SecurityLevel::AskConfirm => "ask_confirm",
        crate::config::SecurityLevel::AutoExecute => "auto_execute",
    };
    sw.set_cfg_security_level(sec_str.into());
    sw.set_cfg_max_steps(char_cfg.agent.max_steps as i32);
    sw.set_cfg_subtitle_tool_call_status(char_cfg.agent.subtitle_tool_call_status);
    sw.set_cfg_subtitle_thinking_status(char_cfg.agent.subtitle_thinking_status);
    sw.set_cfg_live2d_thinking_enabled(char_cfg.agent.live2d_thinking_enabled);
    sw.set_cfg_thinking_cue(char_cfg.agent.thinking_cue.clone().into());
    sw.set_cfg_live2d_tool_cue_enabled(char_cfg.agent.live2d_tool_cue_enabled);
    sw.set_cfg_tool_cue(char_cfg.agent.tool_cue.clone().into());

    sw.set_cfg_builtin_use_skill(char_cfg.agent.builtin_tools.use_skill);
    sw.set_cfg_builtin_get_current_time(char_cfg.agent.builtin_tools.get_current_time);
    sw.set_cfg_builtin_read_file(char_cfg.agent.builtin_tools.read_file);
    sw.set_cfg_builtin_execute_command(char_cfg.agent.builtin_tools.execute_command);

    refresh_mcp_servers_ui(sw, &char_cfg.agent.mcp_servers);

    let skills_mgr = crate::ai::skills::SkillManager::load_all("skills", Some(&char_cfg.id), "characters");
    refresh_skills_ui(sw, &skills_mgr);
    sw.set_loaded_skills_summary(format!("本地技能库就绪：已加载 {} 项技能", skills_mgr.len()).into());
    sw.set_loaded_mcp_summary(format!("MCP 服务池：已配置 {} 个外部服务", char_cfg.agent.mcp_servers.len()).into());

    // Live2D & Model
    sw.set_cfg_model_path(char_cfg.model.path.clone().into());
    sw.set_cfg_neutral_expression(char_cfg.model.neutral_expression.clone().into());
    sw.set_cfg_expression_timeout(char_cfg.model.expression_timeout);
    sw.set_cfg_motion_fade_out(char_cfg.model.motion_fade_out);
    sw.set_cfg_idle_motion_enabled(char_cfg.model.idle_motion_enabled);
    sw.set_cfg_idle_motion_target(char_cfg.model.idle_motion_target.clone().into());
    sw.set_cfg_idle_motion_interval(char_cfg.model.idle_motion_interval);
    sw.set_cfg_fps(char_cfg.model.fps as i32);
    sw.set_cfg_idle_fps(char_cfg.model.idle_fps as i32);
    sw.set_cfg_supersample(char_cfg.model.supersample);

    // Gaze tracking
    sw.set_cfg_gaze_enabled(char_cfg.interactions.gaze.enabled);
    sw.set_cfg_gaze_idle_timeout(char_cfg.interactions.gaze.idle_timeout);
    sw.set_cfg_gaze_blend_with_motions(char_cfg.interactions.gaze.blend_with_motions);
    sw.set_cfg_gaze_pause_in_fullscreen(char_cfg.interactions.gaze.pause_in_fullscreen);

    let mut cue_items: Vec<CueItem> = char_cfg
        .cues
        .normalized_mappings()
        .into_iter()
        .map(|(k, v)| CueItem {
            name: k.into(),
            expression: v.expression.join(", ").into(),
            motion: v.motion.join(", ").into(),
            motion_group: v.motion_group.join(", ").into(),
        })
        .collect();
    cue_items.sort_by(|a, b| a.name.cmp(&b.name));
    sw.set_cue_list(slint::ModelRc::new(slint::VecModel::from(cue_items)));
}

pub fn populate_global_settings(sw: &SettingsWindow, cfg: &AppConfig) {
    sw.set_cfg_toolbar_enabled(cfg.ui.toolbar_enabled);
    sw.set_cfg_subtitle_enabled(cfg.ui.subtitle_enabled);
    sw.set_cfg_subtitle_bubble_enabled(cfg.ui.subtitle_bubble_enabled);
    sw.set_cfg_subtitle_font_size(cfg.ui.subtitle_font_size as i32);
    sw.set_cfg_subtitle_color(cfg.ui.subtitle_color.clone().into());
    sw.set_cfg_subtitle_y_offset(cfg.ui.subtitle_y_offset);

    sw.set_cfg_auto_hide_in_fullscreen(cfg.window.auto_hide_in_fullscreen);
    sw.set_cfg_always_on_top(cfg.window.always_on_top);

    sw.set_cfg_user_avatar_enabled(cfg.user.avatar_enabled);
    sw.set_cfg_user_avatar(load_image_or_empty(&cfg.user.avatar_path));
}

pub fn populate_catalog_lists(sw: &SettingsWindow, catalog: &ModelCatalog) {
    let mut exp_names: Vec<String> = catalog
        .available_expressions
        .keys()
        .filter(|k| !k.contains('/') && !k.contains('\\') && !k.ends_with(".json"))
        .cloned()
        .collect();
    exp_names.sort();
    exp_names.dedup();
    sw.set_available_expressions(slint::ModelRc::new(slint::VecModel::from(
        exp_names
            .into_iter()
            .map(slint::SharedString::from)
            .collect::<Vec<_>>(),
    )));

    let mut grp_names: Vec<String> = catalog
        .motion_groups
        .iter()
        .map(|(k, v)| format!("{} ({})", k, v.len()))
        .collect();
    grp_names.sort();
    sw.set_available_motion_groups(slint::ModelRc::new(slint::VecModel::from(
        grp_names
            .into_iter()
            .map(slint::SharedString::from)
            .collect::<Vec<_>>(),
    )));

    let mut mot_names: Vec<String> = catalog
        .available_motions
        .keys()
        .filter(|k| !k.contains('/') && !k.contains('\\'))
        .cloned()
        .collect();
    mot_names.sort();
    mot_names.dedup();
    sw.set_available_motions(slint::ModelRc::new(slint::VecModel::from(
        mot_names
            .into_iter()
            .map(slint::SharedString::from)
            .collect::<Vec<_>>(),
    )));
}

pub fn setup_settings_callbacks(
    window: &MainWindow,
    tray: &MurasameTray,
    settings_window: &SettingsWindow,
    subtitle_window: &SubtitleWindow,
    chat_window: &ChatWindow,
    config: Arc<Mutex<AppConfig>>,
    char_manager: Arc<Mutex<CharacterManager>>,
    model_catalog: Arc<Mutex<ModelCatalog>>,
    tokio_rt: Arc<tokio::runtime::Runtime>,
    tts_engine: Arc<TtsEngine>,
    voice_changer: Arc<Mutex<Option<VoiceChanger>>>,
    audio_player: Arc<Mutex<AudioPlayer>>,
    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    on_switch_character: Arc<dyn Fn(&str) + Send + Sync + 'static>,
) {
    let sw_weak_filter = settings_window.as_weak();
    settings_window.on_filter_voices_changed(move |query| {
        if let Some(sw) = sw_weak_filter.upgrade() {
            let q = query.to_lowercase();
            let filtered: Vec<slint::SharedString> = ALL_EDGE_VOICES
                .iter()
                .filter(|(id, name)| {
                    q.is_empty()
                        || id.to_lowercase().contains(&q)
                        || name.to_lowercase().contains(&q)
                })
                .map(|(id, _)| (*id).into())
                .collect();
            sw.set_voice_list(slint::ModelRc::new(slint::VecModel::from(filtered)));
        }
    });

    let sw_weak_sv = settings_window.as_weak();
    settings_window.on_select_voice(move |voice_str| {
        if let Some(sw) = sw_weak_sv.upgrade() {
            sw.set_cfg_voice(voice_str);
        }
    });

    let sw_weak_tts_prev = settings_window.as_weak();
    let tts_engine_prev = Arc::clone(&tts_engine);
    let audio_player_prev = Arc::clone(&audio_player);
    let vc_prev = Arc::clone(&voice_changer);
    let rt_tts_prev = Arc::clone(&tokio_rt);
    settings_window.on_preview_tts(move || {
        if let Some(sw) = sw_weak_tts_prev.upgrade() {
            let voice = sw.get_cfg_voice().to_string();
            let rate = format_tts_percentage(sw.get_cfg_tts_rate());
            let pitch = format_tts_hz(sw.get_cfg_tts_pitch());
            let volume_tts = format_tts_percentage(sw.get_cfg_tts_volume());
            let beatrice_enabled = sw.get_cfg_beatrice_enabled();
            let beatrice_path = sw.get_cfg_beatrice_model_path().to_string();
            let speaker_id = sw.get_cfg_speaker_id() as u32;
            let pitch_shift = sw.get_cfg_pitch_shift() as f64;
            let formant_shift = sw.get_cfg_formant_shift() as f64;
            let volume = sw.get_cfg_volume();

            sw.set_tts_preview_status("".into());
            sw.set_tts_preview_is_error(false);

            let tts = Arc::clone(&tts_engine_prev);
            let player = Arc::clone(&audio_player_prev);
            let vc = Arc::clone(&vc_prev);
            let sw_weak_inner = sw_weak_tts_prev.clone();

            rt_tts_prev.spawn(async move {
                let sample_text = "你好！今天也要元气满满哦！";
                match tts
                    .synthesize_to_pcm(sample_text, &voice, &rate, &pitch, &volume_tts)
                    .await
                {
                    Ok(pcm_24k) => {
                        let conversion_res: Result<Vec<f32>, String> = if beatrice_enabled {
                            match tokio::task::spawn_blocking(move || {
                                let mut vc_guard = vc.lock().map_err(|e| format!("{e}"))?;
                                if vc_guard.is_none() {
                                    let new_vc = VoiceChanger::new(
                                        &beatrice_path,
                                        speaker_id,
                                        pitch_shift,
                                        formant_shift,
                                    )
                                    .map_err(|e| format!("{e}"))?;
                                    *vc_guard = Some(new_vc);
                                } else if let Some(engine) = vc_guard.as_mut() {
                                    let _ = engine.set_speaker(speaker_id);
                                    engine.set_pitch_shift(pitch_shift);
                                    engine.set_formant_shift(formant_shift);
                                }

                                if let Some(engine) = vc_guard.as_mut() {
                                    engine.convert(&pcm_24k).map_err(|e| format!("{e}"))
                                } else {
                                    Err("Beatrice 变声引擎未加载".to_string())
                                }
                            })
                            .await
                            {
                                Ok(res) => res,
                                Err(e) => Err(format!("任务执行错误: {e}")),
                            }
                        } else {
                            Ok(pcm_24k)
                        };

                        match conversion_res {
                            Ok(final_samples) => {
                                if !final_samples.is_empty() {
                                    let sample_rate = if beatrice_enabled { 48000 } else { 24000 };
                                    if let Ok(mut p_guard) = player.lock() {
                                        let _ = p_guard.play_samples(
                                            final_samples,
                                            sample_rate,
                                            volume,
                                        );
                                    }
                                    let sw_status = sw_weak_inner.clone();
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(sw) = sw_status.upgrade() {
                                            sw.set_tts_preview_status("".into());
                                            sw.set_tts_preview_is_error(false);
                                        }
                                    });
                                }
                            }
                            Err(err_msg) => {
                                let sw_status = sw_weak_inner.clone();
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(sw) = sw_status.upgrade() {
                                        sw.set_tts_preview_status(
                                            format!("Beatrice 变声失败: {err_msg}").into(),
                                        );
                                        sw.set_tts_preview_is_error(true);
                                    }
                                });
                            }
                        }
                    }
                    Err(e) => {
                        let sw_status = sw_weak_inner.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(sw) = sw_status.upgrade() {
                                sw.set_tts_preview_status(format!("TTS 试听失败: {e}").into());
                                sw.set_tts_preview_is_error(true);
                            }
                        });
                    }
                }
            });
        }
    });

    let sw_weak_sm = settings_window.as_weak();
    settings_window.on_select_model(move |model_name| {
        if let Some(sw) = sw_weak_sm.upgrade() {
            if sw.get_cfg_api_protocol() == "hermes" {
                sw.set_cfg_hermes_model(model_name);
            } else {
                sw.set_cfg_openai_model(model_name);
            }
        }
    });

    let sw_weak_bbf = settings_window.as_weak();
    settings_window.on_browse_beatrice_folder(move || {
        let sw_weak = sw_weak_bbf.clone();
        std::thread::spawn(move || {
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                let path_str = folder.to_string_lossy().to_string();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(sw) = sw_weak.upgrade() {
                        sw.set_cfg_beatrice_model_path(path_str.clone().into());
                        detect_and_set_beatrice_info(&sw, &path_str);
                    }
                });
            }
        });
    });

    let sw_weak_spk = settings_window.as_weak();
    settings_window.on_select_beatrice_speaker(move |idx| {
        if let Some(sw) = sw_weak_spk.upgrade() {
            let speakers = sw.get_beatrice_speakers();
            if let Some(spk_str) = speakers.row_data(idx as usize) {
                if let Some(id_str) = spk_str.as_str().split(':').next() {
                    if let Ok(id) = id_str.trim().parse::<i32>() {
                        sw.set_cfg_speaker_id(id);
                        sw.set_current_speaker_name(spk_str);
                    }
                }
            }
        }
    });

    let sw_weak_bm = settings_window.as_weak();
    settings_window.on_reload_beatrice_model(move || {
        if let Some(sw) = sw_weak_bm.upgrade() {
            let path = sw.get_cfg_beatrice_model_path().to_string();
            detect_and_set_beatrice_info(&sw, &path);
        }
    });

    let all_models: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let sw_weak_fm = settings_window.as_weak();
    let rt_fm = Arc::clone(&tokio_rt);
    let all_models_fetch = Arc::clone(&all_models);
    settings_window.on_fetch_models_from_api(move |base_url, api_key| {
        let sw_weak = sw_weak_fm.clone();
        let url = base_url.to_string();
        let key = api_key.to_string();
        let all_models_store = Arc::clone(&all_models_fetch);
        if let Some(sw) = sw_weak.upgrade() {
            sw.set_is_fetching_models(true);
            sw.set_fetch_models_status("正在请求模型列表...".into());
        }
        rt_fm.spawn(async move {
            match fetch_models_list(&url, &key).await {
                Ok(models) => {
                    let count = models.len();
                    *all_models_store.lock().unwrap() = models.clone();
                    let slint_models: Vec<slint::SharedString> =
                        models.into_iter().map(Into::into).collect();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(sw) = sw_weak.upgrade() {
                            sw.set_is_fetching_models(false);
                            sw.set_model_presets(slint::ModelRc::new(slint::VecModel::from(
                                slint_models,
                            )));
                            sw.set_fetch_models_status(
                                format!("成功获取到 {count} 个可用模型！").into(),
                            );
                            sw.set_model_dropdown_open(true);
                        }
                    });
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(sw) = sw_weak.upgrade() {
                            sw.set_is_fetching_models(false);
                            sw.set_fetch_models_status(format!("获取失败: {err_msg}").into());
                        }
                    });
                }
            }
        });
    });

    let all_models_filter = Arc::clone(&all_models);
    let sw_weak_filter_model = settings_window.as_weak();
    settings_window.on_filter_models_changed(move |query| {
        if let Some(sw) = sw_weak_filter_model.upgrade() {
            let q = query.to_lowercase();
            let guard = all_models_filter.lock().unwrap();
            let filtered: Vec<slint::SharedString> = guard
                .iter()
                .filter(|m| q.is_empty() || m.to_lowercase().contains(&q))
                .map(|m| m.clone().into())
                .collect();
            sw.set_model_presets(slint::ModelRc::new(slint::VecModel::from(filtered)));
        }
    });

    let sw_weak_test_llm = settings_window.as_weak();
    let rt_test_llm = Arc::clone(&tokio_rt);
    settings_window.on_test_llm_connection(move |base_url, api_key, model| {
        let sw_weak = sw_weak_test_llm.clone();
        let url = base_url.to_string();
        let key = api_key.to_string();
        let model_name = model.to_string();
        if let Some(sw) = sw_weak.upgrade() {
            sw.set_is_testing_llm(true);
            sw.set_llm_test_status("正在测试连通性...".into());
            sw.set_llm_test_is_error(false);
        }
        rt_test_llm.spawn(async move {
            let start = Instant::now();
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default();

            let clean_url = crate::config::normalize_api_base_url(&url);
            let res = if !model_name.trim().is_empty() {
                let chat_url = format!("{clean_url}/chat/completions");
                let body = serde_json::json!({
                    "model": model_name.trim(),
                    "messages": [{"role": "user", "content": "hi"}],
                    "max_tokens": 1
                });
                client
                    .post(&chat_url)
                    .header("Authorization", format!("Bearer {}", key.trim()))
                    .json(&body)
                    .send()
                    .await
            } else {
                let models_url = format!("{clean_url}/models");
                client
                    .get(&models_url)
                    .header("Authorization", format!("Bearer {}", key.trim()))
                    .send()
                    .await
            };

            let elapsed = start.elapsed().as_millis();
            let (status_text, is_err, raw_err) = match res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        (
                            format!("连接成功 (耗时: {elapsed}ms)"),
                            false,
                            String::new(),
                        )
                    } else {
                        let err_body = resp.text().await.unwrap_or_default();
                        (
                            format!("连接失败 (HTTP {status})"),
                            true,
                            format!("HTTP {status} 响应内容:\n{err_body}"),
                        )
                    }
                }
                Err(e) => (
                    "连接失败: 请求超时或网络错误".to_string(),
                    true,
                    format!("网络错误详情:\n{e}"),
                ),
            };

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(sw) = sw_weak.upgrade() {
                    sw.set_is_testing_llm(false);
                    sw.set_llm_test_status(status_text.into());
                    sw.set_llm_test_is_error(is_err);
                    sw.set_llm_test_raw_error(raw_err.into());
                    sw.set_error_copy_btn_text("复制错误信息".into());
                }
            });
        });
    });

    let sw_weak_skills = settings_window.as_weak();
    settings_window.on_reload_skills_clicked(move || {
        if let Some(sw) = sw_weak_skills.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let mgr = crate::ai::skills::SkillManager::load_all("skills", Some(&cid), "characters");
            refresh_skills_ui(&sw, &mgr);
            sw.set_loaded_skills_summary(format!("重新扫描完成：已加载 {} 项技能", mgr.len()).into());
        }
    });

    let sw_weak_tmcp = settings_window.as_weak();
    let char_mgr_tmcp = Arc::clone(&char_manager);
    let config_tmcp = Arc::clone(&config);
    settings_window.on_toggle_mcp_server(move |name_slint, enabled| {
        let name = name_slint.to_string();
        if let Some(sw) = sw_weak_tmcp.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let mut cm = char_mgr_tmcp.lock().unwrap();
            if let Some(char_cfg) = cm.get_character_mut(&cid) {
                if let Some(srv) = char_cfg.agent.mcp_servers.get_mut(&name) {
                    srv.enabled = enabled;
                }
                refresh_mcp_servers_ui(&sw, &char_cfg.agent.mcp_servers);
            }
            let mut cfg = config_tmcp.lock().unwrap();
            if let Some(srv) = cfg.agent.mcp_servers.get_mut(&name) {
                srv.enabled = enabled;
            }
            let _ = cfg.save_to_disk(crate::paths::config_path());
        }
    });

    let sw_weak_dmcp = settings_window.as_weak();
    let char_mgr_dmcp = Arc::clone(&char_manager);
    let config_dmcp = Arc::clone(&config);
    settings_window.on_delete_mcp_server(move |name_slint| {
        let name = name_slint.to_string();
        if let Some(sw) = sw_weak_dmcp.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let mut cm = char_mgr_dmcp.lock().unwrap();
            if let Some(char_cfg) = cm.get_character_mut(&cid) {
                char_cfg.agent.mcp_servers.remove(&name);
                refresh_mcp_servers_ui(&sw, &char_cfg.agent.mcp_servers);
            }
            let mut cfg = config_dmcp.lock().unwrap();
            cfg.agent.mcp_servers.remove(&name);
            let _ = cfg.save_to_disk(crate::paths::config_path());
        }
    });

    let sw_weak_amcp = settings_window.as_weak();
    let char_mgr_amcp = Arc::clone(&char_manager);
    let config_amcp = Arc::clone(&config);
    settings_window.on_add_mcp_server(move |name_slint, transport_slint, cmd_slint, args_slint, url_slint| {
        let name = name_slint.to_string().trim().to_string();
        if name.is_empty() {
            return;
        }
        let transport = transport_slint.to_string().trim().to_string();
        let cmd_str = cmd_slint.to_string().trim().to_string();
        let args_str = args_slint.to_string();
        let url_str = url_slint.to_string().trim().to_string();

        let args_vec: Vec<String> = args_str
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        let srv_cfg = crate::config::McpServerConfig {
            enabled: true,
            transport: if transport == "sse" { "sse".to_string() } else { "stdio".to_string() },
            command: if cmd_str.is_empty() { None } else { Some(cmd_str) },
            args: args_vec,
            env: HashMap::new(),
            url: if url_str.is_empty() { None } else { Some(url_str) },
        };

        if let Some(sw) = sw_weak_amcp.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let mut cm = char_mgr_amcp.lock().unwrap();
            if let Some(char_cfg) = cm.get_character_mut(&cid) {
                char_cfg.agent.mcp_servers.insert(name.clone(), srv_cfg.clone());
                refresh_mcp_servers_ui(&sw, &char_cfg.agent.mcp_servers);
            }
            let mut cfg = config_amcp.lock().unwrap();
            cfg.agent.mcp_servers.insert(name, srv_cfg);
            let _ = cfg.save_to_disk(crate::paths::config_path());
        }
    });

    let sw_weak_mcp = settings_window.as_weak();
    let char_mgr_mcp = Arc::clone(&char_manager);
    settings_window.on_refresh_mcp_clicked(move || {
        if let Some(sw) = sw_weak_mcp.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let cm = char_mgr_mcp.lock().unwrap();
            let count = cm.get_character(&cid)
                .map(|c| c.agent.mcp_servers.len())
                .unwrap_or(0);
            sw.set_loaded_mcp_summary(format!("MCP 服务状态：已配置 {count} 个外部服务，连接就绪").into());
        }
    });

    let sw_weak_bmf = settings_window.as_weak();
    settings_window.on_browse_model_file(move || {
        let sw_weak = sw_weak_bmf.clone();
        std::thread::spawn(move || {
            if let Some(file) = rfd::FileDialog::new()
                .add_filter(
                    "Live2D Model (*.model3.json, *.json)",
                    &["model3.json", "json"],
                )
                .add_filter("All Files (*.*)", &["*"])
                .pick_file()
            {
                let path_str = file.to_string_lossy().to_string();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(sw) = sw_weak.upgrade() {
                        sw.set_cfg_model_path(path_str.into());
                    }
                });
            }
        });
    });

    let sw_weak_export = settings_window.as_weak();
    let char_mgr_export = Arc::clone(&char_manager);
    settings_window.on_export_live2d_config(move || {
        let sw_weak = sw_weak_export.clone();
        let char_mgr = Arc::clone(&char_mgr_export);
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON (*.json)", &["json"])
                .add_filter("All Files (*.*)", &["*"])
                .set_file_name("murasame_live2d_cues.json")
                .save_file()
            {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(sw) = sw_weak.upgrade() {
                        let editing_id = sw.get_editing_char_id().to_string();
                        let model_path = sw.get_cfg_model_path().to_string();
                        let cue_model = sw.get_cue_list();
                        let mut cues = HashMap::new();
                        for i in 0..cue_model.row_count() {
                            if let Some(item) = cue_model.row_data(i) {
                                if !item.name.is_empty() {
                                    cues.insert(
                                        item.name.to_string(),
                                        crate::config::CueMapping::from_strings(
                                            &item.expression,
                                            &item.motion,
                                            &item.motion_group,
                                        ),
                                    );
                                }
                            }
                        }
                        let hit_areas = char_mgr
                            .lock()
                            .unwrap()
                            .get_character(&editing_id)
                            .map(|c| c.interactions.hit_areas.clone())
                            .unwrap_or_default();
                        let gaze = char_mgr
                            .lock()
                            .unwrap()
                            .get_character(&editing_id)
                            .map(|c| c.interactions.gaze.clone());
                        let profile = crate::config::Live2dProfile {
                            model_path,
                            neutral_expression: sw.get_cfg_neutral_expression().to_string(),
                            expression_timeout: sw.get_cfg_expression_timeout(),
                            motion_fade_out: sw.get_cfg_motion_fade_out(),
                            fps: (sw.get_cfg_fps() as u32).clamp(15, 144),
                            idle_fps: (sw.get_cfg_idle_fps() as u32).clamp(15, 60),
                            supersample: sw.get_cfg_supersample().clamp(1.0, 4.0),
                            cues,
                            hit_areas,
                            gaze,
                        };
                        if let Ok(json) = serde_json::to_string_pretty(&profile) {
                            let _ = std::fs::write(path, json);
                            tracing::info!("Live2D profile exported successfully!");
                        }
                    }
                });
            }
        });
    });

    let sw_weak_import = settings_window.as_weak();
    let char_mgr_import = Arc::clone(&char_manager);
    settings_window.on_import_live2d_config(move || {
        let sw_weak = sw_weak_import.clone();
        let char_mgr = Arc::clone(&char_mgr_import);
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON (*.json)", &["json"])
                .add_filter("All Files (*.*)", &["*"])
                .pick_file()
            {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(profile) =
                        serde_json::from_str::<crate::config::Live2dProfile>(&content)
                    {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(sw) = sw_weak.upgrade() {
                                if !profile.model_path.is_empty() {
                                    sw.set_cfg_model_path(profile.model_path.clone().into());
                                }
                                if !profile.neutral_expression.is_empty() {
                                    sw.set_cfg_neutral_expression(
                                        profile.neutral_expression.clone().into(),
                                    );
                                }
                                if profile.expression_timeout > 0.0 {
                                    sw.set_cfg_expression_timeout(profile.expression_timeout);
                                }
                                if profile.motion_fade_out >= 0.0 {
                                    sw.set_cfg_motion_fade_out(profile.motion_fade_out);
                                }
                                if profile.fps > 0 {
                                    sw.set_cfg_fps(profile.fps.clamp(15, 144) as i32);
                                }
                                if profile.idle_fps > 0 {
                                    sw.set_cfg_idle_fps(profile.idle_fps.clamp(15, 60) as i32);
                                }
                                if profile.supersample >= 1.0 {
                                    sw.set_cfg_supersample(profile.supersample.clamp(1.0, 4.0));
                                }
                                let mut items = Vec::new();
                                for (k, v) in profile.cues {
                                    items.push(CueItem {
                                        name: k.into(),
                                        expression: v.expression.join(", ").into(),
                                        motion: v.motion.join(", ").into(),
                                        motion_group: v.motion_group.join(", ").into(),
                                    });
                                }
                                items.sort_by(|a, b| a.name.cmp(&b.name));
                                sw.set_cue_list(slint::ModelRc::new(slint::VecModel::from(items)));
                                let editing_id = sw.get_editing_char_id().to_string();
                                if let Some(char_cfg) =
                                    char_mgr.lock().unwrap().get_character_mut(&editing_id)
                                {
                                    char_cfg.interactions.hit_areas = profile.hit_areas;
                                    if let Some(g) = profile.gaze {
                                        char_cfg.interactions.gaze = g.clone();
                                        sw.set_cfg_gaze_enabled(g.enabled);
                                        sw.set_cfg_gaze_idle_timeout(g.idle_timeout);
                                        sw.set_cfg_gaze_blend_with_motions(g.blend_with_motions);
                                        sw.set_cfg_gaze_pause_in_fullscreen(g.pause_in_fullscreen);
                                    }
                                }
                                tracing::info!("Live2D profile imported successfully!");
                            }
                        });
                    }
                }
            }
        });
    });

    let sw_weak_save_cue = settings_window.as_weak();
    settings_window.on_save_cue_form(move || {
        if let Some(sw) = sw_weak_save_cue.upgrade() {
            let name = sw.get_edit_cue_name().to_string();
            if name.trim().is_empty() {
                return;
            }
            let exp = sw.get_edit_cue_expression().to_string();
            let mot = sw.get_edit_cue_motion().to_string();
            let mut grp = sw.get_edit_cue_motion_group().to_string();
            if let Some((clean_grp, _)) = grp.split_once(" (") {
                grp = clean_grp.to_string();
            }

            let model = sw.get_cue_list();
            let mut items: Vec<CueItem> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();
            if let Some(existing) = items.iter_mut().find(|it| it.name == name) {
                existing.expression = exp.into();
                existing.motion = mot.into();
                existing.motion_group = grp.into();
            } else {
                items.push(CueItem {
                    name: name.into(),
                    expression: exp.into(),
                    motion: mot.into(),
                    motion_group: grp.into(),
                });
            }
            items.sort_by(|a, b| a.name.cmp(&b.name));
            sw.set_cue_list(slint::ModelRc::new(slint::VecModel::from(items)));
        }
    });

    let sw_weak_clear_cue = settings_window.as_weak();
    settings_window.on_clear_cue_form(move || {
        if let Some(sw) = sw_weak_clear_cue.upgrade() {
            sw.set_edit_cue_name("".into());
            sw.set_edit_cue_expression("".into());
            sw.set_edit_cue_motion("".into());
            sw.set_edit_cue_motion_group("".into());
        }
    });

    let sw_weak_edit_cue = settings_window.as_weak();
    settings_window.on_edit_cue(move |idx| {
        if let Some(sw) = sw_weak_edit_cue.upgrade() {
            let model = sw.get_cue_list();
            if let Some(item) = model.row_data(idx as usize) {
                sw.set_edit_cue_name(item.name);
                sw.set_edit_cue_expression(item.expression);
                sw.set_edit_cue_motion(item.motion);
                sw.set_edit_cue_motion_group(item.motion_group);
            }
        }
    });

    let sw_weak_del = settings_window.as_weak();
    settings_window.on_remove_cue(move |idx| {
        if let Some(sw) = sw_weak_del.upgrade() {
            let model = sw.get_cue_list();
            let mut items: Vec<CueItem> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();
            if (idx as usize) < items.len() {
                items.remove(idx as usize);
                sw.set_cue_list(slint::ModelRc::new(slint::VecModel::from(items)));
            }
        }
    });

    let sw_weak_prev_cue = settings_window.as_weak();
    let live2d_prev_cue = Arc::clone(&live2d_renderer);
    let catalog_prev_cue = Arc::clone(&model_catalog);
    settings_window.on_preview_cue(move |idx| {
        if let Some(sw) = sw_weak_prev_cue.upgrade() {
            let model = sw.get_cue_list();
            if let Some(item) = model.row_data(idx as usize) {
                if let Ok(mut guard) = live2d_prev_cue.lock() {
                    if let Some(renderer) = guard.as_mut() {
                        use rand::seq::SliceRandom;
                        let mut rng = rand::thread_rng();
                        let cat = catalog_prev_cue.lock().unwrap();

                        // 1. Play expression (supports comma-separated list or catalog cue lookup)
                        let exp_str = item.expression.to_string();
                        let exp_parts: Vec<&str> = exp_str
                            .split(',')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .collect();
                        let chosen_exp = exp_parts.choose(&mut rng).copied();
                        if let Some(exp_name) = chosen_exp {
                            if let Some(p) = cat.available_expressions.get(exp_name) {
                                let _ = renderer.play_expression(p);
                            } else {
                                let p = cat.model_dir.join(exp_name);
                                if p.exists() {
                                    let _ = renderer.play_expression(p);
                                }
                            }
                        } else if let Some(action) = cat.cues.get(&item.name.to_string()) {
                            if let Some(p) = action.pick_expression() {
                                let _ = renderer.play_expression(p);
                            }
                        }

                        // 2. Play motion (supports comma-separated list or catalog cue lookup)
                        let mot_str = item.motion.to_string();
                        let mot_parts: Vec<&str> = mot_str
                            .split(',')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .collect();
                        let chosen_mot = mot_parts.choose(&mut rng).copied();
                        if let Some(mot_name) = chosen_mot {
                            if let Some(p) = cat.available_motions.get(mot_name) {
                                let _ = renderer.play_motion(p);
                            } else {
                                let p = cat.model_dir.join(mot_name);
                                if p.exists() {
                                    let _ = renderer.play_motion(p);
                                }
                            }
                        } else {
                            let grp_str = item.motion_group.to_string();
                            let grp_parts: Vec<&str> = grp_str
                                .split(',')
                                .map(|s| s.trim())
                                .filter(|s| !s.is_empty())
                                .collect();
                            let chosen_grp = grp_parts.choose(&mut rng).copied();
                            if let Some(grp_name) = chosen_grp {
                                let clean_grp = grp_name
                                    .split_once(" (")
                                    .map(|(g, _)| g)
                                    .unwrap_or(grp_name);
                                if let Some(p) = cat.get_random_motion_from_group(clean_grp) {
                                    let _ = renderer.play_motion(p);
                                }
                            } else if let Some(action) = cat.cues.get(&item.name.to_string()) {
                                let motion_to_play = action.pick_motion().cloned().or_else(|| {
                                    action.pick_motion_group().and_then(|grp| {
                                        cat.get_random_motion_from_group(grp)
                                    })
                                });
                                if let Some(motion_path) = motion_to_play {
                                    let _ = renderer.play_motion(motion_path);
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    let live2d_prev_exp = Arc::clone(&live2d_renderer);
    let catalog_prev_exp = Arc::clone(&model_catalog);
    settings_window.on_preview_expression_name(move |exp_name| {
        let s = exp_name.to_string();
        if !s.is_empty() {
            if let Ok(mut guard) = live2d_prev_exp.lock() {
                if let Some(renderer) = guard.as_mut() {
                    let cat = catalog_prev_exp.lock().unwrap();
                    if let Some(p) = cat.available_expressions.get(&s) {
                        let _ = renderer.play_expression(p);
                    } else {
                        let p = cat.model_dir.join(&s);
                        if p.exists() {
                            let _ = renderer.play_expression(p);
                        }
                    }
                }
            }
        }
    });

    let live2d_prev_mot = Arc::clone(&live2d_renderer);
    let catalog_prev_mot = Arc::clone(&model_catalog);
    settings_window.on_preview_motion_name(move |mot_name| {
        let s = mot_name.to_string();
        if !s.is_empty() {
            if let Ok(mut guard) = live2d_prev_mot.lock() {
                if let Some(renderer) = guard.as_mut() {
                    let cat = catalog_prev_mot.lock().unwrap();
                    if let Some(p) = cat.available_motions.get(&s) {
                        let _ = renderer.play_motion(p);
                    } else {
                        let p = cat.model_dir.join(&s);
                        if p.exists() {
                            let _ = renderer.play_motion(p);
                        }
                    }
                }
            }
        }
    });

    let live2d_prev_grp = Arc::clone(&live2d_renderer);
    let catalog_prev_grp = Arc::clone(&model_catalog);
    settings_window.on_preview_motion_group_name(move |grp_name| {
        let s = grp_name.to_string();
        if !s.is_empty() {
            let clean_grp = if let Some((g, _)) = s.split_once(" (") {
                g
            } else {
                &s
            };
            if let Ok(mut guard) = live2d_prev_grp.lock() {
                if let Some(renderer) = guard.as_mut() {
                    let cat = catalog_prev_grp.lock().unwrap();
                    if let Some(p) = cat.get_random_motion_from_group(clean_grp) {
                        let _ = renderer.play_motion(p);
                    }
                }
            }
        }
    });

    let sw_weak_cancel = settings_window.as_weak();
    settings_window.on_cancel_clicked(move || {
        if let Some(sw) = sw_weak_cancel.upgrade() {
            let _ = sw.hide();
        }
    });

    // Multi-character callbacks
    let sw_weak_sel = settings_window.as_weak();
    let char_mgr_sel = Arc::clone(&char_manager);
    settings_window.on_select_editing_character(move |id| {
        let id_str = id.to_string();
        if let Some(sw) = sw_weak_sel.upgrade() {
            let cm = char_mgr_sel.lock().unwrap();
            if let Some(char_cfg) = cm.get_character(&id_str) {
                let is_act = cm.active_id == id_str;
                let base_dir = cm.base_dir.clone();
                populate_character_settings(&sw, char_cfg, &base_dir, is_act);
            }
        }
    });

    let switch_fn_act = Arc::clone(&on_switch_character);
    settings_window.on_set_as_active_character(move |id| {
        switch_fn_act(&id.to_string());
    });

    let sw_weak_scm = settings_window.as_weak();
    let char_mgr_scm = Arc::clone(&char_manager);
    let win_weak_scm = window.as_weak();
    let tray_weak_scm = tray.as_weak();
    let chat_win_scm = chat_window.as_weak();
    settings_window.on_create_character(move |id, name, copy_current| {
        if let Some(sw) = sw_weak_scm.upgrade() {
            let id_str = if id.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                id.to_string()
            };
            let name_str = name.to_string();
            let mut cm = char_mgr_scm.lock().unwrap();
            let copy_source = if copy_current {
                Some(cm.active_id.clone())
            } else {
                None
            };

            match cm.create_character(&id_str, &name_str, copy_source.as_deref()) {
                Ok(new_char) => {
                    let base_dir = cm.base_dir.clone();
                    let items = build_character_items(&cm);
                    let items_model = slint::ModelRc::new(slint::VecModel::from(items));
                    sw.set_character_list(items_model.clone());
                    if let Some(w) = win_weak_scm.upgrade() {
                        w.set_character_list(items_model.clone());
                    }
                    if let Some(t) = tray_weak_scm.upgrade() {
                        t.set_character_list(items_model.clone());
                    }
                    if let Some(cw) = chat_win_scm.upgrade() {
                        cw.set_character_list(items_model.clone());
                    }

                    populate_character_settings(&sw, &new_char, &base_dir, false);
                }
                Err(e) => {
                    tracing::error!("Failed to create character '{id_str}': {e}");
                }
            }
        }
    });

    let sw_weak_rnc = settings_window.as_weak();
    let char_mgr_rnc = Arc::clone(&char_manager);
    let win_weak_rnc = window.as_weak();
    let tray_weak_rnc = tray.as_weak();
    let chat_win_rnc = chat_window.as_weak();
    settings_window.on_rename_character(move |id, new_name| {
        if let Some(sw) = sw_weak_rnc.upgrade() {
            let id_str = id.to_string();
            let name_str = new_name.to_string();
            let mut cm = char_mgr_rnc.lock().unwrap();
            if let Err(e) = cm.rename_character(&id_str, &name_str) {
                tracing::error!("Failed to rename character: {e}");
                return;
            }
            if sw.get_editing_char_id().to_string() == id_str {
                sw.set_editing_char_name(name_str.clone().into());
            }
            if cm.active_id == id_str {
                if let Some(cw) = chat_win_rnc.upgrade() {
                    cw.set_active_character_name(name_str.clone().into());
                }
            }
            let items = build_character_items(&cm);
            let items_model = slint::ModelRc::new(slint::VecModel::from(items));
            sw.set_character_list(items_model.clone());
            if let Some(w) = win_weak_rnc.upgrade() {
                w.set_character_list(items_model.clone());
            }
            if let Some(t) = tray_weak_rnc.upgrade() {
                t.set_character_list(items_model.clone());
            }
            if let Some(cw) = chat_win_rnc.upgrade() {
                cw.set_character_list(items_model);
            }
        }
    });

    let sw_weak_cdel = settings_window.as_weak();
    let char_mgr_cdel = Arc::clone(&char_manager);
    let win_weak_cdel = window.as_weak();
    let tray_weak_cdel = tray.as_weak();
    let chat_win_cdel = chat_window.as_weak();
    settings_window.on_delete_character(move |id| {
        if let Some(sw) = sw_weak_cdel.upgrade() {
            let id_str = id.to_string();
            let mut cm = char_mgr_cdel.lock().unwrap();
            match cm.delete_character(&id_str) {
                Ok(_) => {
                    let base_dir = cm.base_dir.clone();
                    let items = build_character_items(&cm);
                    let items_model = slint::ModelRc::new(slint::VecModel::from(items));
                    sw.set_character_list(items_model.clone());
                    if let Some(w) = win_weak_cdel.upgrade() {
                        w.set_character_list(items_model.clone());
                    }
                    if let Some(t) = tray_weak_cdel.upgrade() {
                        t.set_character_list(items_model.clone());
                    }
                    if let Some(cw) = chat_win_cdel.upgrade() {
                        cw.set_character_list(items_model.clone());
                    }

                    let active_id = cm.active_id.clone();
                    if let Some(active_char) = cm.get_character(&active_id) {
                        populate_character_settings(&sw, active_char, &base_dir, true);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to delete character '{id_str}': {e}");
                }
            }
        }
    });

    let sw_weak_bca = settings_window.as_weak();
    let char_mgr_bca = Arc::clone(&char_manager);
    let chat_win_bca = chat_window.as_weak();
    settings_window.on_browse_character_avatar(move || {
        let sw_weak = sw_weak_bca.clone();
        let char_mgr = Arc::clone(&char_mgr_bca);
        let chat_win = chat_win_bca.clone();
        if let Some(sw) = sw_weak.upgrade() {
            let editing_id = sw.get_editing_char_id().to_string();
            std::thread::spawn(move || {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
                {
                    let base_dir = char_mgr.lock().unwrap().base_dir.clone();
                    let char_dir = base_dir.join(&editing_id);
                    let _ = std::fs::create_dir_all(&char_dir);
                    let dest = char_dir.join("avatar.png");

                    let save_ok = if let Ok(reader) = image::ImageReader::open(&path) {
                        if let Ok(reader) = reader.with_guessed_format() {
                            if let Ok(dyn_img) = reader.decode() {
                                dyn_img.save(&dest).is_ok()
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if save_ok {
                        let mut cm = char_mgr.lock().unwrap();
                        let base_dir = cm.base_dir.clone();
                        if let Some(char_cfg) = cm.get_character_mut(&editing_id) {
                            char_cfg.avatar_path = "avatar.png".to_string();
                            let _ = char_cfg.save_to_disk(&base_dir);
                        }
                        let is_active = cm.active_id == editing_id;
                        drop(cm);

                        let dest_clone = dest.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let loaded = load_image_or_empty(&dest_clone);
                            if let Some(sw) = sw_weak.upgrade() {
                                sw.set_editing_char_avatar(loaded.clone());
                            }
                            if is_active {
                                if let Some(cw) = chat_win.upgrade() {
                                    cw.set_character_avatar(loaded);
                                }
                            }
                        });
                    }
                }
            });
        }
    });

    let sw_weak_bsca = settings_window.as_weak();
    let char_mgr_bsca = Arc::clone(&char_manager);
    let chat_win_bsca = chat_window.as_weak();
    let win_weak_bsca = window.as_weak();
    let tray_weak_bsca = tray.as_weak();
    settings_window.on_browse_specific_char_avatar(move |target_id| {
        let sw_weak = sw_weak_bsca.clone();
        let char_mgr = Arc::clone(&char_mgr_bsca);
        let chat_win = chat_win_bsca.clone();
        let win_weak = win_weak_bsca.clone();
        let tray_weak = tray_weak_bsca.clone();
        let target_id_str = target_id.to_string();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                .pick_file()
            {
                let base_dir = char_mgr.lock().unwrap().base_dir.clone();
                let char_dir = base_dir.join(&target_id_str);
                let _ = std::fs::create_dir_all(&char_dir);
                let dest = char_dir.join("avatar.png");

                let save_ok = if let Ok(reader) = image::ImageReader::open(&path) {
                    if let Ok(reader) = reader.with_guessed_format() {
                        if let Ok(dyn_img) = reader.decode() {
                            dyn_img.save(&dest).is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if save_ok {
                    let mut cm = char_mgr.lock().unwrap();
                    let base_dir = cm.base_dir.clone();
                    if let Some(char_cfg) = cm.get_character_mut(&target_id_str) {
                        char_cfg.avatar_path = "avatar.png".to_string();
                        let _ = char_cfg.save_to_disk(&base_dir);
                    }
                    let is_active = cm.active_id == target_id_str;
                    drop(cm);

                    let target_id_clone = target_id_str.clone();
                    let dest_clone = dest.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let loaded = load_image_or_empty(&dest_clone);
                        if let Some(sw) = sw_weak.upgrade() {
                            if sw.get_editing_char_id().to_string() == target_id_clone {
                                sw.set_editing_char_avatar(loaded.clone());
                            }
                            let cm = char_mgr.lock().unwrap();
                            let items = build_character_items(&cm);
                            let items_model = slint::ModelRc::new(slint::VecModel::from(items));
                            sw.set_character_list(items_model.clone());
                            if let Some(w) = win_weak.upgrade() {
                                w.set_character_list(items_model.clone());
                            }
                            if let Some(t) = tray_weak.upgrade() {
                                t.set_character_list(items_model.clone());
                            }
                            if let Some(cw) = chat_win.upgrade() {
                                cw.set_character_list(items_model);
                            }
                        }
                        if is_active {
                            if let Some(cw) = chat_win.upgrade() {
                                cw.set_character_avatar(loaded);
                            }
                        }
                    });
                }
            }
        });
    });

    let sw_weak_bua = settings_window.as_weak();
    let chat_win_bua = chat_window.as_weak();
    let config_bua = Arc::clone(&config);
    settings_window.on_browse_user_avatar(move || {
        let sw_weak = sw_weak_bua.clone();
        let chat_win = chat_win_bua.clone();
        let cfg_clone = Arc::clone(&config_bua);
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                .pick_file()
            {
                let assets_dir = std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join("assets");
                let _ = std::fs::create_dir_all(&assets_dir);
                let dest = assets_dir.join("user_avatar.png");
                let save_ok = if let Ok(reader) = image::ImageReader::open(&path) {
                    if let Ok(reader) = reader.with_guessed_format() {
                        if let Ok(dyn_img) = reader.decode() {
                            dyn_img.save(&dest).is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                if save_ok {
                    let dest_clone = dest.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let loaded = load_image_or_empty(&dest_clone);
                        if let Some(sw) = sw_weak.upgrade() {
                            sw.set_cfg_user_avatar(loaded.clone());
                        }
                        if let Some(cw) = chat_win.upgrade() {
                            cw.set_user_avatar(loaded);
                        }
                        let mut cfg = cfg_clone.lock().unwrap();
                        cfg.user.avatar_path = "assets/user_avatar.png".to_string();
                        let _ = cfg.save_to_disk(crate::paths::config_path());
                    });
                }
            }
        });
    });

    let sw_weak_rua = settings_window.as_weak();
    let chat_win_rua = chat_window.as_weak();
    let config_rua = Arc::clone(&config);
    settings_window.on_reset_user_avatar(move || {
        if let Some(sw) = sw_weak_rua.upgrade() {
            let default_source = Path::new("assets").join("default_user_avatar.png");
            let target_path = Path::new("assets").join("user_avatar.png");
            if default_source.exists() {
                let _ = std::fs::copy(&default_source, &target_path);
            }
            let loaded = load_image_or_empty(&target_path);
            sw.set_cfg_user_avatar(loaded.clone());
            if let Some(cw) = chat_win_rua.upgrade() {
                cw.set_user_avatar(loaded);
            }
            let mut cfg = config_rua.lock().unwrap();
            cfg.user.avatar_path = "assets/user_avatar.png".to_string();
            let _ = cfg.save_to_disk(crate::paths::config_path());
        }
    });

    // Save logic
    let config_save = Arc::clone(&config);
    let char_mgr_save = Arc::clone(&char_manager);
    let vc_save = Arc::clone(&voice_changer);
    let live2d_save = Arc::clone(&live2d_renderer);
    let catalog_save = Arc::clone(&model_catalog);
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

                let _ = cfg.save_to_disk(crate::paths::config_path());
            }

            let cfg_clone = config_save.lock().unwrap().clone();
            if let Some(win) = win_weak_save.upgrade() {
                win.set_toolbar_visible(cfg_clone.ui.toolbar_enabled);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpServerConfig;

    #[test]
    fn test_refresh_mcp_servers_ui() {
        let mut servers = HashMap::new();
        servers.insert(
            "filesystem".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "stdio".to_string(),
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@modelcontextprotocol/server-filesystem".to_string()],
                env: HashMap::new(),
                url: None,
            },
        );
        servers.insert(
            "remote_mcp".to_string(),
            McpServerConfig {
                enabled: false,
                transport: "sse".to_string(),
                command: None,
                args: Vec::new(),
                env: HashMap::new(),
                url: Some("http://localhost:8000/sse".to_string()),
            },
        );

        let items = build_mcp_server_items(&servers);
        assert_eq!(items.len(), 2);

        let first = &items[0];
        assert_eq!(first.name, "filesystem");
        assert_eq!(first.transport, "stdio");
        assert!(first.detail.contains("npx"));
        assert!(first.enabled);

        let second = &items[1];
        assert_eq!(second.name, "remote_mcp");
        assert_eq!(second.transport, "sse");
        assert_eq!(second.detail, "http://localhost:8000/sse");
        assert!(!second.enabled);
    }
}

