use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use slint::{ComponentHandle, Model};
use crate::{
    character::CharacterManager,
    config::AppConfig,
    model::ModelCatalog,
    renderer::Live2dRenderer,
};
use crate::app::{CueItem, SettingsWindow};

pub fn setup(
    settings_window: &SettingsWindow,
    _config: &Arc<Mutex<AppConfig>>,
    char_manager: &Arc<Mutex<CharacterManager>>,
    model_catalog: &Arc<Mutex<ModelCatalog>>,
    live2d_renderer: &Arc<Mutex<Option<Live2dRenderer>>>,
) {
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
}
