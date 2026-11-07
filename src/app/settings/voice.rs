use std::sync::{Arc, Mutex};
use slint::{ComponentHandle, Model};
use crate::{
    audio::{AudioPlayer, TtsEngine, VoiceChanger},
    config::AppConfig,
};
use crate::app::{
    SettingsWindow,
    helpers::{ALL_EDGE_VOICES, detect_and_set_beatrice_info, format_tts_hz, format_tts_percentage},
};

pub fn setup(
    settings_window: &SettingsWindow,
    _config: &Arc<Mutex<AppConfig>>,
    tts_engine: &Arc<TtsEngine>,
    voice_changer: &Arc<Mutex<Option<VoiceChanger>>>,
    audio_player: &Arc<Mutex<AudioPlayer>>,
    tokio_rt: &Arc<tokio::runtime::Runtime>,
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
}
