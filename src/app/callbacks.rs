use slint::ComponentHandle;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    audio::{AudioPlayer, TtsEngine, VoiceChanger},
    character::CharacterManager,
    config::AppConfig,
    model::{HitAreaHandler, ModelCatalog},
    renderer::Live2dRenderer,
};

use super::{
    ChatWindow, MainWindow, MurasameTray, SettingsWindow, SubtitleWindow,
    chat::{set_subtitle_window_text, setup_chat_callbacks},
    chat_window::ChatManager,
    platform::{
        CHAT_OPEN_STATE, LAST_SUBTITLE_RECT, drag_cancel, drag_end, drag_move, drag_start_record,
        force_update_subtitle_window_position, set_clipboard_text, set_mouse_passthrough,
        set_subtitle_window_attributes, set_window_size, update_subtitle_window_position,
    },
    settings::setup_settings_callbacks,
};

pub fn perform_character_switch(
    target_id: &str,
    char_mgr: &Arc<Mutex<CharacterManager>>,
    config: &Arc<Mutex<AppConfig>>,
    model_catalog: &Arc<Mutex<ModelCatalog>>,
    hit_area_handler: &Arc<Mutex<HitAreaHandler>>,
    audio_player: &Arc<Mutex<AudioPlayer>>,
    voice_changer: &Arc<Mutex<Option<VoiceChanger>>>,
    live2d_renderer: &Arc<Mutex<Option<Live2dRenderer>>>,
    is_speaking: &Arc<AtomicBool>,
    chat_manager: &Arc<ChatManager>,
    window_weak: slint::Weak<MainWindow>,
    tray_weak: slint::Weak<MurasameTray>,
    chat_win_weak: slint::Weak<ChatWindow>,
    settings_win_weak: slint::Weak<SettingsWindow>,
) {
    // 1. Audio cutoff
    if let Ok(mut player) = audio_player.lock() {
        player.stop();
    }
    is_speaking.store(false, Ordering::Relaxed);

    // 2. Character switch in CharacterManager
    let (new_char, char_items, base_dir) = {
        let mut mgr = char_mgr.lock().unwrap();
        if !mgr.switch_active(target_id) {
            tracing::warn!("Target character not found: {target_id}");
            return;
        }
        let char_cfg = mgr.get_active_character().unwrap().clone();
        let items = crate::app::helpers::build_character_items(&mgr);
        let base_dir = mgr.base_dir.clone();
        (char_cfg, items, base_dir)
    };


    // 3. Update global config and persist
    {
        let mut cfg = config.lock().unwrap();
        cfg.active_character = target_id.to_string();
        let _ = cfg.save_to_disk(crate::paths::config_path());
    }

    let resolved_model_path = crate::paths::resolve_resource_path(&new_char.model.path);

    // 4. Hot-reload Live2D Model & Render FPS
    crate::app::render_loop::update_render_fps(new_char.model.fps, new_char.model.idle_fps);
    if let Ok(mut r_guard) = live2d_renderer.lock() {
        if let Some(renderer) = r_guard.as_mut() {
            if let Err(e) =
                renderer.reload_model(&resolved_model_path, new_char.model.scale)
            {
                tracing::error!("Failed to hot-reload Live2D model for {target_id}: {e}");
            }
            renderer.set_motion_fade_out(new_char.model.motion_fade_out);
            renderer.set_gaze_config(new_char.interactions.gaze.clone());
        }
    }

    // 5. Reload ModelCatalog & HitAreaHandler
    if let Ok(new_cat) = ModelCatalog::load(
        &resolved_model_path,
        &new_char.cues,
        &new_char.interactions,
    ) {
        if let Ok(mut ha_guard) = hit_area_handler.lock() {
            *ha_guard = HitAreaHandler::new(&new_cat.hit_areas, &new_char.interactions);
        }
        if let Ok(mut cat_guard) = model_catalog.lock() {
            *cat_guard = new_cat;
        }
    }

    // 6. Reload Beatrice VoiceChanger
    if new_char.audio.beatrice_enabled {
        let beatrice_resolved =
            crate::paths::resolve_resource_path(&new_char.audio.beatrice_model_path);
        match VoiceChanger::new(
            &beatrice_resolved,
            new_char.audio.speaker_id,
            new_char.audio.pitch_shift,
            new_char.audio.formant_shift,
        ) {
            Ok(new_vc) => {
                if let Ok(mut vc_guard) = voice_changer.lock() {
                    *vc_guard = Some(new_vc);
                }
            }
            Err(e) => {
                tracing::warn!("Beatrice reload error for {target_id}: {e}");
                if let Ok(mut vc_guard) = voice_changer.lock() {
                    *vc_guard = None;
                }
            }
        }
    } else {
        if let Ok(mut vc_guard) = voice_changer.lock() {
            *vc_guard = None;
        }
    }

    // 7. Chat session switch & isolation
    let _ = chat_manager.switch_character(target_id, &new_char.ai.protocol);
    let active_session_id = chat_manager.get_active_session_id();

    // 8. Synchronize all UI elements
    let items_model = slint::ModelRc::new(slint::VecModel::from(char_items));
    let avatar_img =
        crate::app::helpers::load_image_or_empty(new_char.get_avatar_full_path(&base_dir));

    if let Some(win) = window_weak.upgrade() {
        win.set_character_list(items_model.clone());
        win.set_model_scale(new_char.model.scale.clamp(0.4, 2.5));
    }

    if let Some(tray) = tray_weak.upgrade() {
        tray.set_character_list(items_model.clone());
    }

    if let Some(chat_win) = chat_win_weak.upgrade() {
        chat_win.set_character_list(items_model.clone());
        chat_win.set_active_character_id(new_char.id.clone().into());
        chat_win.set_active_character_name(new_char.name.clone().into());
        chat_win.set_character_avatar(avatar_img.clone());
        crate::app::chat_window::refresh_sessions_ui(
            &chat_win,
            &chat_manager.db,
            target_id,
            &active_session_id,
        );
        let cues_opt = model_catalog.lock().ok().map(|c| c.cues.clone());
        crate::app::chat_window::refresh_messages_ui(
            &chat_win,
            &chat_manager.db,
            &active_session_id,
            cues_opt.as_ref(),
        );
    }

    if let Some(sw) = settings_win_weak.upgrade() {
        sw.set_character_list(items_model);
        let is_now_active = sw.get_editing_char_id() == new_char.id.as_str();
        sw.set_editing_char_is_active(is_now_active);
    }

    tracing::info!(
        "Successfully hot-swapped active character to '{}' ({})",
        new_char.name,
        target_id
    );
}

pub fn setup_callbacks(
    window: &MainWindow,
    tray: &MurasameTray,
    settings_window: &SettingsWindow,
    subtitle_window: &SubtitleWindow,
    chat_window: &ChatWindow,
    chat_manager: Arc<ChatManager>,
    config: Arc<Mutex<AppConfig>>,
    char_manager: Arc<Mutex<CharacterManager>>,
    model_catalog: Arc<Mutex<ModelCatalog>>,
    hit_area_handler: Arc<Mutex<HitAreaHandler>>,
    tokio_rt: Arc<tokio::runtime::Runtime>,
    tts_engine: Arc<TtsEngine>,
    voice_changer: Arc<Mutex<Option<VoiceChanger>>>,
    audio_player: Arc<Mutex<AudioPlayer>>,
    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    is_speaking: Arc<AtomicBool>,
    window_visible: Arc<AtomicBool>,
) {
    let live2d_ref = Arc::clone(&live2d_renderer);
    let catalog = Arc::clone(&model_catalog);
    let hit_areas = Arc::clone(&hit_area_handler);
    let audio_player_tap = Arc::clone(&audio_player);
    let sub_weak = subtitle_window.as_weak();
    let config_cb = Arc::clone(&config);

    window.on_copy_to_clipboard(move |text| {
        set_clipboard_text(&text);
    });
    settings_window.on_copy_to_clipboard(move |text| {
        set_clipboard_text(&text);
    });

    window.on_drag_start_record(move || {
        drag_start_record();
    });

    let config_drag_move = Arc::clone(&config);
    window.on_drag_move(move || {
        let y_off = config_drag_move.lock().unwrap().ui.subtitle_y_offset as i32;
        drag_move(y_off);
    });

    let config_drag_end = Arc::clone(&config);
    window.on_drag_end(move || {
        drag_end(&config_drag_end);
    });

    window.on_drag_cancel(move || {
        drag_cancel();
    });

    // 1. Pet clicked (tap interaction)
    window.on_pet_clicked(move |x, y| {
        let mut guard = live2d_ref.lock().unwrap();
        let Some(renderer) = guard.as_mut() else {
            return;
        };

        let cursor_x = x as f64;
        let cursor_y = y as f64;

        if let Some(area_name) = renderer.hit_test(cursor_x, cursor_y) {
            tracing::info!("Hit area tapped: {area_name}");
            let motion_group_opt = hit_areas
                .lock()
                .unwrap()
                .get_motion_group_for_area(&area_name)
                .cloned();
            if let Some(motion_group) = motion_group_opt {
                let cat = catalog.lock().unwrap();
                if let Some(motions) = cat.motion_groups.get(&motion_group) {
                    if !motions.is_empty() {
                        let motion_path = &motions[rand::random::<usize>() % motions.len()];
                        let _ = renderer.play_motion(motion_path);

                        let motion_filename = motion_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default();

                        let mut sound_to_play = None;
                        let mut text_to_show = None;

                        if let Ok(content) = std::fs::read_to_string(&cat.model_file) {
                            if let Ok(json) =
                                serde_json::from_str::<crate::model::loader::Model3Json>(&content)
                            {
                                if let Some(entries) =
                                    json.file_references.motions.get(&motion_group)
                                {
                                    for entry in entries {
                                        if entry.file.ends_with(motion_filename) {
                                            if let Some(sound) = &entry.sound {
                                                let sound_path = cat.model_dir.join(sound);
                                                if sound_path.exists() {
                                                    sound_to_play = Some(sound_path);
                                                }
                                            }
                                            if let Some(txt) = &entry.text {
                                                text_to_show = Some(txt.clone());
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if let (Some(sub), Some(txt)) = (sub_weak.upgrade(), text_to_show) {
                            set_subtitle_window_text(&sub, &txt);
                            sub.set_subtitle_visible(true);
                            sub.set_subtitle_opacity(1.0);
                            sub.window().request_redraw();
                            let y_off = config_cb.lock().unwrap().ui.subtitle_y_offset as i32;
                            force_update_subtitle_window_position(y_off);
                        }

                        if let Some(sound_path) = sound_to_play {
                            if let Ok(mut player) = audio_player_tap.lock() {
                                let _ = player.play_wav_file(sound_path, 1.0);
                            }
                        }
                    }
                }
            }
        }
    });

    // Character switch function generator
    let switch_fn: Arc<dyn Fn(&str) + Send + Sync + 'static> = {
        let cm = Arc::clone(&char_manager);
        let cfg = Arc::clone(&config);
        let cat = Arc::clone(&model_catalog);
        let ha = Arc::clone(&hit_area_handler);
        let ap = Arc::clone(&audio_player);
        let vc = Arc::clone(&voice_changer);
        let lr = Arc::clone(&live2d_renderer);
        let spk = Arc::clone(&is_speaking);
        let chm = Arc::clone(&chat_manager);
        let w_wk = window.as_weak();
        let t_wk = tray.as_weak();
        let cw_wk = chat_window.as_weak();
        let sw_wk = settings_window.as_weak();
        Arc::new(move |target_id: &str| {
            perform_character_switch(
                target_id,
                &cm,
                &cfg,
                &cat,
                &ha,
                &ap,
                &vc,
                &lr,
                &spk,
                &chm,
                w_wk.clone(),
                t_wk.clone(),
                cw_wk.clone(),
                sw_wk.clone(),
            );
        })
    };

    {
        let sw_fn = Arc::clone(&switch_fn);
        window.on_switch_character(move |id| {
            sw_fn(&id.to_string());
        });
    }

    {
        let sw_fn = Arc::clone(&switch_fn);
        tray.on_switch_character(move |id| {
            sw_fn(&id.to_string());
        });
    }

    // 1.1 ChatWindow callbacks
    crate::app::chat_window::setup_chat_window_callbacks(
        chat_window,
        Arc::clone(&chat_manager),
        Arc::clone(&config),
        Arc::clone(&char_manager),
        Arc::clone(&model_catalog),
        Arc::clone(&tokio_rt),
        Arc::clone(&tts_engine),
        Arc::clone(&voice_changer),
        Arc::clone(&audio_player),
        Arc::clone(&live2d_renderer),
        subtitle_window,
        Arc::clone(&is_speaking),
        Arc::clone(&switch_fn),
    );

    // 2. Chat / Prompt submitted
    setup_chat_callbacks(
        window,
        subtitle_window,
        chat_window,
        Arc::clone(&chat_manager),
        Arc::clone(&config),
        Arc::clone(&char_manager),
        Arc::clone(&model_catalog),
        Arc::clone(&tokio_rt),
        Arc::clone(&tts_engine),
        Arc::clone(&voice_changer),
        Arc::clone(&audio_player),
        Arc::clone(&live2d_renderer),
        Arc::clone(&is_speaking),
    );

    // 3. Settings Window callbacks
    setup_settings_callbacks(
        window,
        tray,
        settings_window,
        subtitle_window,
        chat_window,
        Arc::clone(&config),
        Arc::clone(&char_manager),
        Arc::clone(&model_catalog),
        Arc::clone(&tokio_rt),
        Arc::clone(&tts_engine),
        Arc::clone(&voice_changer),
        Arc::clone(&audio_player),
        Arc::clone(&live2d_renderer),
        Arc::clone(&switch_fn),
    );

    // 4. Open settings window
    let sw_weak_win = settings_window.as_weak();
    window.on_open_settings(move || {
        let sw_weak = sw_weak_win.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(sw) = sw_weak.upgrade() {
                let _ = sw.show();
            }
        });
    });

    // 4.1 Open chat window
    let cw_weak_win = chat_window.as_weak();
    window.on_open_chat_window(move || {
        let cw_weak = cw_weak_win.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(cw) = cw_weak.upgrade() {
                let _ = cw.show();
                cw.invoke_force_scroll_to_bottom();
            }
        });
    });

    let cw_weak_tray = chat_window.as_weak();
    tray.on_open_chat_window(move || {
        let cw_weak = cw_weak_tray.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(cw) = cw_weak.upgrade() {
                let _ = cw.show();
                cw.invoke_force_scroll_to_bottom();
            }
        });
    });

    // 5. Exit app
    window.on_exit_app(move || {
        let _ = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        });
    });

    // 6. Tray integration & window visibility
    let win_weak_tray = window.as_weak();
    let sub_weak_tray = subtitle_window.as_weak();
    let tray_weak = tray.as_weak();
    let win_vis_toggle = Arc::clone(&window_visible);
    let config_tray = Arc::clone(&config);
    tray.on_toggle_window(move || {
        let current = win_vis_toggle.load(Ordering::Relaxed);
        let next = !current;
        win_vis_toggle.store(next, Ordering::Relaxed);
        if let Some(t) = tray_weak.upgrade() {
            t.set_window_visible(next);
        }
        if let Some(win) = win_weak_tray.upgrade() {
            if next {
                let _ = win.show();
                crate::app::render_loop::wake_render_timer();
            } else {
                let _ = win.hide();
            }
        }
        if let Some(sub) = sub_weak_tray.upgrade() {
            if next {
                let _ = sub.show();
                set_subtitle_window_attributes();
                let y_off = config_tray.lock().unwrap().ui.subtitle_y_offset;
                update_subtitle_window_position(y_off);
            } else {
                let _ = sub.hide();
            }
        }
    });

    let sw_weak_tray = settings_window.as_weak();
    tray.on_open_settings(move || {
        let sw_weak = sw_weak_tray.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(sw) = sw_weak.upgrade() {
                let _ = sw.show();
            }
        });
    });

    tray.on_exit_app(move || {
        slint::quit_event_loop().unwrap();
    });

    // 7. Minimize to tray from UI
    let win_weak_min = window.as_weak();
    let sub_weak_min = subtitle_window.as_weak();
    let tray_weak_min = tray.as_weak();
    let win_vis_min = Arc::clone(&window_visible);
    window.on_minimize_to_tray(move || {
        win_vis_min.store(false, Ordering::Relaxed);
        if let Some(t) = tray_weak_min.upgrade() {
            t.set_window_visible(false);
        }
        let win_weak = win_weak_min.clone();
        let sub_weak = sub_weak_min.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = win_weak.upgrade() {
                let _ = win.hide();
            }
            if let Some(sub) = sub_weak.upgrade() {
                let _ = sub.hide();
            }
        });
    });

    // 8. Dynamic fitted dimensions calculation & reset
    let live2d_scale = Arc::clone(&live2d_renderer);
    let char_mgr_scale = Arc::clone(&char_manager);
    let catalog_scale = Arc::clone(&model_catalog);
    let win_weak_scale = window.as_weak();
    window.on_scale_changed(move |delta| {
        let current_scale = char_mgr_scale
            .lock()
            .unwrap()
            .get_active_character()
            .map(|c| c.model.scale)
            .unwrap_or(1.0);
        let new_scale = (current_scale + delta).clamp(0.4, 2.5);

        let (base_w, base_h) = catalog_scale
            .lock()
            .unwrap()
            .calculate_fitted_window_size(750.0);
        let new_w = (base_w * new_scale).round() as u32;
        let new_h = (base_h * new_scale).round() as u32;

        if let Some(win) = win_weak_scale.upgrade() {
            win.window()
                .set_size(slint::LogicalSize::new(new_w as f32, new_h as f32));
            win.set_model_scale(new_scale);
        }
        set_window_size(new_w, new_h);
        if let Ok(mut guard) = live2d_scale.lock() {
            if let Some(renderer) = guard.as_mut() {
                renderer.resize(new_w, new_h);
            }
        }
        if let Ok(mut mgr) = char_mgr_scale.lock() {
            if let Some(active) = mgr.get_active_character_mut() {
                active.model.scale = new_scale;
                let id = active.id.clone();
                let _ = mgr.save_character(&id);
            }
        }
    });

    let live2d_reset = Arc::clone(&live2d_renderer);
    let char_mgr_reset = Arc::clone(&char_manager);
    let catalog_reset = Arc::clone(&model_catalog);
    let win_weak_reset = window.as_weak();
    window.on_reset_scale(move || {
        let new_scale = 1.0;
        let (base_w, base_h) = catalog_reset
            .lock()
            .unwrap()
            .calculate_fitted_window_size(750.0);
        let new_w = (base_w * new_scale).round() as u32;
        let new_h = (base_h * new_scale).round() as u32;

        if let Some(win) = win_weak_reset.upgrade() {
            win.window()
                .set_size(slint::LogicalSize::new(new_w as f32, new_h as f32));
            win.set_model_scale(new_scale);
        }
        set_window_size(new_w, new_h);
        if let Ok(mut guard) = live2d_reset.lock() {
            if let Some(renderer) = guard.as_mut() {
                renderer.resize(new_w, new_h);
            }
        }
        if let Ok(mut mgr) = char_mgr_reset.lock() {
            if let Some(active) = mgr.get_active_character_mut() {
                active.model.scale = new_scale;
                let id = active.id.clone();
                let _ = mgr.save_character(&id);
            }
        }
    });

    // 9. Mouse passthrough toggles
    let mouse_pt = Arc::new(AtomicBool::new(false));
    let mouse_pt_win = Arc::clone(&mouse_pt);
    let mouse_pt_tray = Arc::clone(&mouse_pt);

    let win_weak_pt = window.as_weak();
    let win_weak_pt_tray = window.as_weak();
    let sub_weak_pt = subtitle_window.as_weak();
    let sub_weak_pt_tray = subtitle_window.as_weak();
    let tray_weak_pt = tray.as_weak();
    let tray_weak_pt_tray = tray.as_weak();
    let config_pt = Arc::clone(&config);

    window.on_toggle_mouse_passthrough(move || {
        let current = mouse_pt_win.load(Ordering::Relaxed);
        let next = !current;
        mouse_pt_win.store(next, Ordering::Relaxed);
        set_mouse_passthrough(next);
        if let Some(win) = win_weak_pt.upgrade() {
            win.set_mouse_passthrough(next);
        }
        if let Some(sub) = sub_weak_pt.upgrade() {
            if next {
                set_subtitle_window_text(&sub, "已开启鼠标穿透（可通过任务栏托盘图标右键关闭）");
                sub.set_subtitle_visible(true);
                sub.set_subtitle_opacity(1.0);
                sub.window().request_redraw();
                let y_off = config_pt.lock().unwrap().ui.subtitle_y_offset as i32;
                force_update_subtitle_window_position(y_off);
            }
        }
        if let Some(t) = tray_weak_pt.upgrade() {
            t.set_mouse_passthrough(next);
        }
    });

    let config_pt_tray = Arc::clone(&config);
    tray.on_toggle_mouse_passthrough(move || {
        let current = mouse_pt_tray.load(Ordering::Relaxed);
        let next = !current;
        mouse_pt_tray.store(next, Ordering::Relaxed);
        set_mouse_passthrough(next);
        if let Some(win) = win_weak_pt_tray.upgrade() {
            win.set_mouse_passthrough(next);
        }
        if let Some(sub) = sub_weak_pt_tray.upgrade() {
            if next {
                set_subtitle_window_text(&sub, "已开启鼠标穿透（可通过任务栏托盘图标右键关闭）");
                sub.set_subtitle_visible(true);
                sub.set_subtitle_opacity(1.0);
                sub.window().request_redraw();
                let y_off = config_pt_tray.lock().unwrap().ui.subtitle_y_offset as i32;
                force_update_subtitle_window_position(y_off);
            }
        }
        if let Some(t) = tray_weak_pt_tray.upgrade() {
            t.set_mouse_passthrough(next);
        }
    });

    let config_chat = Arc::clone(&config);
    window.on_chat_toggled(move |open| {
        #[cfg(windows)]
        {
            CHAT_OPEN_STATE.store(open, Ordering::Relaxed);
            if let Ok(mut last) = LAST_SUBTITLE_RECT.lock() {
                *last = None;
            }
        }
        let y_off = config_chat.lock().unwrap().ui.subtitle_y_offset as i32;
        update_subtitle_window_position(y_off);
    });
}
