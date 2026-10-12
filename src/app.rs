use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use anyhow::{Context, Result};
use slint::ComponentHandle;
use tokio::sync::mpsc;

use crate::{
    ai::{LlmClient, StreamCueParser, StreamEvent, generate_system_prompt},
    audio::{AudioPlayer, LipSyncFilter, TtsEngine, VoiceChanger},
    config::AppConfig,
    model::{HitAreaHandler, ModelCatalog},
    renderer::Live2dRenderer,
};

slint::include_modules!();

pub struct Application {
    _audio_stream: rodio::OutputStream,
    config: Arc<Mutex<AppConfig>>,
    model_catalog: Arc<ModelCatalog>,
    hit_area_handler: Arc<HitAreaHandler>,
    tokio_rt: Arc<tokio::runtime::Runtime>,

    audio_player: Arc<Mutex<AudioPlayer>>,
    tts_engine: Arc<TtsEngine>,
    voice_changer: Arc<Mutex<Option<VoiceChanger>>>,
    lip_sync_filter: Arc<Mutex<LipSyncFilter>>,

    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    is_speaking: Arc<AtomicBool>,
}

impl Application {
    pub fn new() -> Result<Self> {
        let config = AppConfig::load_or_create("config.toml")
            .context("Failed to load or create config.toml")?;

        let model_catalog = ModelCatalog::load(
            &config.model.path,
            &config.cues,
            &config.interactions,
        )
        .context("Failed to load Live2D model catalog")?;

        let hit_area_handler =
            HitAreaHandler::new(&model_catalog.hit_areas, &config.interactions);

        let tokio_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("Failed to create tokio runtime")?;

        let (audio_stream, stream_handle) = rodio::OutputStream::try_default()
            .context("Failed to open audio output stream")?;

        let audio_player = Arc::new(Mutex::new(AudioPlayer::new(stream_handle)));

        let tts_engine = Arc::new(TtsEngine::new().context("Failed to initialize TtsEngine")?);

        let voice_changer = if config.audio.beatrice_enabled {
            match VoiceChanger::new(
                &config.audio.beatrice_model_path,
                config.audio.speaker_id,
                config.audio.pitch_shift,
                config.audio.formant_shift,
            ) {
                Ok(vc) => Some(vc),
                Err(e) => {
                    tracing::warn!("Beatrice initialization warning: {e}. Falling back to clean TTS.");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            _audio_stream: audio_stream,
            config: Arc::new(Mutex::new(config)),
            model_catalog: Arc::new(model_catalog),
            hit_area_handler: Arc::new(hit_area_handler),
            tokio_rt: Arc::new(tokio_rt),
            audio_player,
            tts_engine,
            voice_changer: Arc::new(Mutex::new(voice_changer)),
            lip_sync_filter: Arc::new(Mutex::new(LipSyncFilter::default())),
            live2d_renderer: Arc::new(Mutex::new(None)),
            is_speaking: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn run(self) -> Result<()> {
        // 1. Initialize Slint WGPU 30 backend
        slint::BackendSelector::new()
            .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::default())
            .select()
            .context("Failed to select Slint WGPU 30 backend")?;

        let window = MainWindow::new().context("Failed to instantiate Slint MainWindow")?;

        let cfg = self.config.lock().unwrap().clone();
        window.set_cfg_api_url(cfg.ai.base_url.into());
        window.set_cfg_api_key(cfg.ai.api_key.into());
        window.set_cfg_model(cfg.ai.model.into());
        window.set_cfg_voice(cfg.audio.tts_voice.into());
        window.set_cfg_beatrice_enabled(cfg.audio.beatrice_enabled);

        // 2. Setup WGPU device hook for Live2D renderer initialization
        let live2d_ref = Arc::clone(&self.live2d_renderer);
        let model_path = cfg.model.path.clone();
        let win_width = cfg.window.width;
        let win_height = cfg.window.height;
        let win_scale = cfg.model.scale;

        window.window().set_rendering_notifier(move |state, api| {
            if let (
                slint::RenderingState::RenderingSetup,
                slint::GraphicsAPI::WGPU30 { device, queue, .. },
            ) = (state, api)
            {
                let mut renderer_guard = live2d_ref.lock().unwrap();
                if renderer_guard.is_none() {
                    match Live2dRenderer::new(
                        Arc::new(device.clone()),
                        Arc::new(queue.clone()),
                        &model_path,
                        win_width,
                        win_height,
                        win_scale,
                    ) {
                        Ok(renderer) => {
                            tracing::info!("Live2D renderer initialized with WGPU 30!");
                            *renderer_guard = Some(renderer);
                        }
                        Err(e) => {
                            tracing::error!("Failed to initialize Live2D renderer: {e}");
                        }
                    }
                }
            }
        })?;

        // 3. Register callbacks
        self.setup_callbacks(&window);

        // 4. Start 60 FPS animation & render loop timer
        self.setup_render_timer(&window);

        // 5. Run Slint event loop
        window.run().context("Error during Slint UI execution")?;
        Ok(())
    }

    fn setup_callbacks(&self, window: &MainWindow) {
        let live2d_ref = Arc::clone(&self.live2d_renderer);
        let catalog = Arc::clone(&self.model_catalog);
        let hit_areas = Arc::clone(&self.hit_area_handler);
        let audio_player = Arc::clone(&self.audio_player);
        let window_weak = window.as_weak();

        // 1. Pet clicked (tap interaction)
        window.on_pet_clicked(move |x, y| {
            let mut guard = live2d_ref.lock().unwrap();
            let Some(renderer) = guard.as_mut() else { return; };

            let cursor_x = x as f64;
            let cursor_y = y as f64;

            if let Some(area_name) = renderer.hit_test(cursor_x, cursor_y) {
                tracing::info!("Hit area tapped: {area_name}");
                if let Some(motion_group) = hit_areas.get_motion_group_for_area(&area_name) {
                    if let Some(motions) = catalog.motion_groups.get(motion_group) {
                        if !motions.is_empty() {
                            let motion_path = &motions[rand::random::<usize>() % motions.len()];
                            let _ = renderer.play_motion(motion_path);

                            // Check if motion has sound or text
                            let motion_filename = motion_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or_default();

                            let mut sound_to_play = None;
                            let mut text_to_show = None;

                            // Scan model3.json entries for this motion
                            if let Ok(content) = std::fs::read_to_string(&catalog.model_file) {
                                if let Ok(json) = serde_json::from_str::<crate::model::loader::Model3Json>(&content) {
                                    if let Some(entries) = json.file_references.motions.get(motion_group) {
                                        for entry in entries {
                                            if entry.file.ends_with(motion_filename) {
                                                if let Some(sound) = &entry.sound {
                                                    let sound_path = catalog.model_dir.join(sound);
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

                            if let (Some(win), Some(txt)) = (window_weak.upgrade(), text_to_show) {
                                win.set_subtitle_text(txt.into());
                                win.set_subtitle_visible(true);
                                win.set_subtitle_opacity(1.0);
                            }

                            if let Some(sound_path) = sound_to_play {
                                if let Ok(mut player) = audio_player.lock() {
                                    let _ = player.play_wav_file(sound_path, 1.0);
                                }
                            }
                        }
                    }
                }
            }
        });

        // 2. Prompt submitted (AI conversation)
        let rt = Arc::clone(&self.tokio_rt);
        let config = Arc::clone(&self.config);
        let catalog_cue = Arc::clone(&self.model_catalog);
        let tts = Arc::clone(&self.tts_engine);
        let vc = Arc::clone(&self.voice_changer);
        let player = Arc::clone(&self.audio_player);
        let live2d_ai = Arc::clone(&self.live2d_renderer);
        let is_speaking = Arc::clone(&self.is_speaking);
        let window_weak_ai = window.as_weak();

        window.on_prompt_submitted(move |prompt| {
            let prompt_text = prompt.to_string();
            let cfg = config.lock().unwrap().clone();
            let catalog_cue = Arc::clone(&catalog_cue);
            let tts = Arc::clone(&tts);
            let vc = Arc::clone(&vc);
            let player = Arc::clone(&player);
            let live2d_ai = Arc::clone(&live2d_ai);
            let is_speaking = Arc::clone(&is_speaking);
            let win_weak = window_weak_ai.clone();

            if let Some(win) = win_weak.upgrade() {
                win.set_is_loading(true);
                win.set_subtitle_text("...".into());
                win.set_subtitle_visible(true);
                win.set_subtitle_opacity(1.0);
            }

            rt.spawn(async move {
                let system_prompt =
                    generate_system_prompt(&cfg.ai.system_prompt_prefix, &catalog_cue.cues);
                let mut llm_client = LlmClient::new(&cfg.ai, system_prompt);

                let (token_tx, mut token_rx) = mpsc::unbounded_channel::<String>();

                // Spawn LLM streaming request
                let prompt_clone = prompt_text.clone();
                tokio::spawn(async move {
                    if let Err(e) = llm_client.ask_stream(&prompt_clone, token_tx).await {
                        tracing::error!("LLM streaming request error: {e}");
                    }
                });

                let mut parser = StreamCueParser::new(catalog_cue.cues.clone());
                let mut accumulated_subtitle = String::new();

                while let Some(token) = token_rx.recv().await {
                    let events = parser.process_token(&token);
                    for event in events {
                        match event {
                            StreamEvent::Cue(cue_name) => {
                                tracing::info!("Streaming Cue intercepted: [{cue_name}]");
                                if let Ok(mut guard) = live2d_ai.lock() {
                                    if let Some(renderer) = guard.as_mut() {
                                        if let Some(expr_path) = catalog_cue.cues.get_expression(&cue_name) {
                                            let _ = renderer.play_expression(expr_path);
                                        } else if let Some(motion_path) = catalog_cue.cues.get_motion(&cue_name) {
                                            let _ = renderer.play_motion(motion_path);
                                        }
                                    }
                                }
                            }
                            StreamEvent::Text(txt) => {
                                accumulated_subtitle.push_str(&txt);
                                let current_text = accumulated_subtitle.clone();
                                let win_weak_text = win_weak.clone();
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(win) = win_weak_text.upgrade() {
                                        win.set_subtitle_text(current_text.into());
                                        win.set_subtitle_visible(true);
                                        win.set_subtitle_opacity(1.0);
                                    }
                                });
                            }
                            StreamEvent::Sentence(sentence) => {
                                // Synthesize sentence via TTS & Beatrice
                                let tts_clone = Arc::clone(&tts);
                                let vc_clone = Arc::clone(&vc);
                                let player_clone = Arc::clone(&player);
                                let voice = cfg.audio.tts_voice.clone();
                                let is_speaking_clone = Arc::clone(&is_speaking);
                                let beatrice_enabled = cfg.audio.beatrice_enabled;
                                let volume = cfg.audio.volume;

                                tokio::spawn(async move {
                                    is_speaking_clone.store(true, Ordering::Relaxed);
                                    match tts_clone.synthesize_to_pcm(&sentence, &voice).await {
                                        Ok(pcm_24k) => {
                                            let final_samples = if beatrice_enabled {
                                                if let Ok(mut vc_guard) = vc_clone.lock() {
                                                    if let Some(engine) = vc_guard.as_mut() {
                                                        match engine.convert(&pcm_24k) {
                                                            Ok(converted) => converted,
                                                            Err(e) => {
                                                                tracing::error!("VC conversion failed: {e}");
                                                                pcm_24k
                                                            }
                                                        }
                                                    } else {
                                                        pcm_24k
                                                    }
                                                } else {
                                                    pcm_24k
                                                }
                                            } else {
                                                pcm_24k
                                            };

                                            let sample_rate = if beatrice_enabled { 48000 } else { 24000 };
                                            if let Ok(mut audio_p) = player_clone.lock() {
                                                let _ = audio_p.play_samples(final_samples, sample_rate, volume);
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("TTS synthesis failed for sentence: {e}");
                                        }
                                    }
                                    is_speaking_clone.store(false, Ordering::Relaxed);
                                });
                            }
                        }
                    }
                }

                // Finish remaining parser buffers
                let remaining = parser.finish();
                for event in remaining {
                    if let StreamEvent::Sentence(sentence) = event {
                        let tts_clone = Arc::clone(&tts);
                        let vc_clone = Arc::clone(&vc);
                        let player_clone = Arc::clone(&player);
                        let voice = cfg.audio.tts_voice.clone();
                        let beatrice_enabled = cfg.audio.beatrice_enabled;
                        let volume = cfg.audio.volume;

                        tokio::spawn(async move {
                            if let Ok(pcm_24k) = tts_clone.synthesize_to_pcm(&sentence, &voice).await {
                                let final_samples = if beatrice_enabled {
                                    if let Ok(mut vc_guard) = vc_clone.lock() {
                                        if let Some(engine) = vc_guard.as_mut() {
                                            engine.convert(&pcm_24k).unwrap_or(pcm_24k)
                                        } else {
                                            pcm_24k
                                        }
                                    } else {
                                        pcm_24k
                                    }
                                } else {
                                    pcm_24k
                                };
                                let sample_rate = if beatrice_enabled { 48000 } else { 24000 };
                                if let Ok(mut audio_p) = player_clone.lock() {
                                    let _ = audio_p.play_samples(final_samples, sample_rate, volume);
                                }
                            }
                        });
                    }
                }

                let win_weak_done = win_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = win_weak_done.upgrade() {
                        win.set_is_loading(false);
                    }
                });
            });
        });

        // 3. Settings saved
        let config_save = Arc::clone(&self.config);
        let win_weak_cfg = window.as_weak();
        window.on_settings_saved(move || {
            if let Some(win) = win_weak_cfg.upgrade() {
                let mut cfg = config_save.lock().unwrap();
                cfg.ai.base_url = win.get_cfg_api_url().to_string();
                cfg.ai.api_key = win.get_cfg_api_key().to_string();
                cfg.ai.model = win.get_cfg_model().to_string();
                cfg.audio.tts_voice = win.get_cfg_voice().to_string();
                cfg.audio.beatrice_enabled = win.get_cfg_beatrice_enabled();

                if let Ok(serialized) = toml::to_string_pretty(&*cfg) {
                    let _ = std::fs::write("config.toml", serialized);
                    tracing::info!("Config saved to config.toml!");
                }
            }
        });

        // 4. Exit app
        window.on_exit_app(move || {
            slint::quit_event_loop().unwrap();
        });
    }

    fn setup_render_timer(&self, window: &MainWindow) {
        let live2d_ref = Arc::clone(&self.live2d_renderer);
        let audio_player = Arc::clone(&self.audio_player);
        let lip_sync = Arc::clone(&self.lip_sync_filter);
        let window_weak = window.as_weak();

        let mut subtitle_last_active = Instant::now();
        let timer = slint::Timer::default();

        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                let Some(win) = window_weak.upgrade() else { return; };

                // 1. Lip sync calculation
                let raw_rms = if let Ok(player) = audio_player.lock() {
                    player.current_rms()
                } else {
                    0.0
                };

                let mouth_open = if let Ok(mut filter) = lip_sync.lock() {
                    filter.update(raw_rms)
                } else {
                    0.0
                };

                // 2. Advance and render Live2D
                if let Ok(mut guard) = live2d_ref.lock() {
                    if let Some(renderer) = guard.as_mut() {
                        renderer.set_mouth_open(mouth_open);

                        match renderer.tick_and_render() {
                            Ok(image) => {
                                win.set_pet_texture(image);
                            }
                            Err(e) => {
                                tracing::error!("Render frame error: {e}");
                            }
                        }
                    }
                }

                // 3. Subtitle lifetime & fadeout
                let is_playing_audio = if let Ok(player) = audio_player.lock() {
                    player.is_playing()
                } else {
                    false
                };

                if is_playing_audio || win.get_is_loading() {
                    subtitle_last_active = Instant::now();
                    win.set_subtitle_opacity(1.0);
                } else {
                    let elapsed = subtitle_last_active.elapsed();
                    if elapsed > Duration::from_secs(4) {
                        let fade_duration = Duration::from_millis(600);
                        let fade_elapsed = elapsed - Duration::from_secs(4);
                        if fade_elapsed < fade_duration {
                            let progress =
                                fade_elapsed.as_secs_f32() / fade_duration.as_secs_f32();
                            win.set_subtitle_opacity((1.0 - progress).max(0.0));
                        } else {
                            win.set_subtitle_opacity(0.0);
                            win.set_subtitle_visible(false);
                        }
                    }
                }
            },
        );

        // Keep timer alive on window
        std::mem::forget(timer);
    }
}
