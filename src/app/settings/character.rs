use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use slint::ComponentHandle;
use crate::{
    character::CharacterManager,
    config::AppConfig,
    model::ModelCatalog,
};
use crate::app::{
    ChatWindow, MainWindow, MurasameTray, SettingsWindow,
    helpers::{build_character_items, load_image_or_empty},
};
use super::populate::populate_character_settings;

pub fn setup(
    settings_window: &SettingsWindow,
    window: &MainWindow,
    tray: &MurasameTray,
    chat_window: &ChatWindow,
    config: &Arc<Mutex<AppConfig>>,
    char_manager: &Arc<Mutex<CharacterManager>>,
    _model_catalog: &Arc<Mutex<ModelCatalog>>,
    on_switch_character: &Arc<dyn Fn(&str) + Send + Sync + 'static>,
) {
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
}
