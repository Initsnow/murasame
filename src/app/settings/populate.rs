use std::{
    collections::HashMap,
    path::Path,
};

use crate::{
    character::CharacterConfig,
    config::AppConfig,
    model::ModelCatalog,
};

use crate::app::{
    CueItem, McpServerItem, SettingsWindow, SkillItem,
    helpers::{
        detect_and_set_beatrice_info, load_image_or_empty, parse_tts_hz,
        parse_tts_percentage,
    },
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
    sw.set_cfg_builtin_take_screenshot(char_cfg.agent.builtin_tools.take_screenshot);
    sw.set_cfg_builtin_screenshot_save_to_disk(char_cfg.agent.builtin_tools.screenshot_save_to_disk);
    sw.set_cfg_multimodal_vision(char_cfg.ai.multimodal_vision);
    sw.set_cfg_is_vision_model(char_cfg.ai.multimodal_vision);

    refresh_mcp_servers_ui(sw, &char_cfg.agent.mcp_servers);

    let app_root = crate::paths::app_root_dir();
    let skills_mgr = crate::ai::skills::SkillManager::load_all(
        app_root.join("skills"),
        Some(&char_cfg.id),
        app_root.join("characters"),
    );
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

    // Proactive speaking
    sw.set_cfg_proactive_enabled(char_cfg.interactions.proactive.enabled);
    sw.set_cfg_proactive_min_interval(char_cfg.interactions.proactive.min_interval_minutes as f32);
    sw.set_cfg_proactive_max_interval(char_cfg.interactions.proactive.max_interval_minutes as f32);
    sw.set_cfg_proactive_prompt(char_cfg.interactions.proactive.prompt.clone().into());

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

