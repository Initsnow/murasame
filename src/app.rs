use anyhow::{Context, Result};
use slint::ComponentHandle;
use std::sync::{
    Arc, Mutex,
    atomic::AtomicBool,
};

use crate::{
    audio::{AudioPlayer, LipSyncFilter, TtsEngine, VoiceChanger},
    character::CharacterManager,
    config::AppConfig,
    model::{HitAreaHandler, ModelCatalog},
    renderer::Live2dRenderer,
};

pub mod callbacks;
pub mod chat;
pub mod chat_window;
pub mod helpers;
pub mod platform;
pub mod render_loop;
pub mod settings;

slint::include_modules!();

pub use helpers::*;
pub use platform::*;

pub struct Application {
    _audio_stream: rodio::OutputStream,
    config: Arc<Mutex<AppConfig>>,
    character_manager: Arc<Mutex<CharacterManager>>,
    model_catalog: Arc<Mutex<ModelCatalog>>,
    hit_area_handler: Arc<Mutex<HitAreaHandler>>,
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
        let config = AppConfig::load_or_create(crate::paths::config_path())
            .context("Failed to load or create config.toml")?;

        let characters_dir = crate::paths::app_root_dir().join("characters");
        let char_manager = CharacterManager::load_all(&characters_dir, &config.active_character)
            .context("Failed to load characters")?;

        let active_char = char_manager
            .get_active_character()
            .cloned()
            .context("No active character found")?;

        let resolved_model_path = crate::paths::resolve_resource_path(&active_char.model.path);
        let model_catalog = ModelCatalog::load(
            &resolved_model_path,
            &active_char.cues,
            &active_char.interactions,
        )
        .context("Failed to load Live2D model catalog")?;

        let hit_area_handler =
            HitAreaHandler::new(&model_catalog.hit_areas, &active_char.interactions);

        let tokio_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("Failed to create tokio runtime")?;
        let _guard = tokio_rt.enter();

        let (audio_stream, stream_handle) =
            rodio::OutputStream::try_default().context("Failed to open audio output stream")?;

        let audio_player = Arc::new(Mutex::new(AudioPlayer::new(stream_handle)));
        let tts_engine = Arc::new(TtsEngine::new().context("Failed to initialize TtsEngine")?);

        let voice_changer = if active_char.audio.beatrice_enabled {
            let beatrice_resolved =
                crate::paths::resolve_resource_path(&active_char.audio.beatrice_model_path);
            match VoiceChanger::new(
                &beatrice_resolved,
                active_char.audio.speaker_id,
                active_char.audio.pitch_shift,
                active_char.audio.formant_shift,
            ) {
                Ok(vc) => Some(vc),
                Err(e) => {
                    tracing::warn!(
                        "Beatrice initialization warning: {e}. Falling back to clean TTS."
                    );
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            _audio_stream: audio_stream,
            config: Arc::new(Mutex::new(config)),
            character_manager: Arc::new(Mutex::new(char_manager)),
            model_catalog: Arc::new(Mutex::new(model_catalog)),
            hit_area_handler: Arc::new(Mutex::new(hit_area_handler)),
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
        let tray = MurasameTray::new().context("Failed to instantiate Slint MurasameTray")?;
        let settings_window =
            SettingsWindow::new().context("Failed to instantiate Slint SettingsWindow")?;
        let subtitle_window =
            SubtitleWindow::new().context("Failed to instantiate Slint SubtitleWindow")?;
        let chat_window = ChatWindow::new().context("Failed to instantiate Slint ChatWindow")?;

        let cfg = self.config.lock().unwrap().clone();
        let active_char = self
            .character_manager
            .lock()
            .unwrap()
            .get_active_character()
            .cloned()
            .context("Active character not found")?;

        let db = Arc::new(crate::db::Database::open(crate::paths::database_path())?);
        let chat_manager = Arc::new(chat_window::ChatManager::new(
            Arc::clone(&db),
            &active_char.id,
            &active_char.ai.protocol,
        )?);

        // 1. Initialize MainWindow UI properties
        window.set_toolbar_visible(cfg.ui.toolbar_enabled);
        let _ = window.show();

        // 2. Initialize SubtitleWindow UI properties
        subtitle_window.set_subtitle_enabled(cfg.ui.subtitle_enabled);
        subtitle_window.set_bubble_enabled(cfg.ui.subtitle_bubble_enabled);
        subtitle_window.set_subtitle_font_size(cfg.ui.subtitle_font_size as f32);
        subtitle_window.set_subtitle_color(parse_hex_color(&cfg.ui.subtitle_color));
        subtitle_window.set_subtitle_visible(false);
        let _ = subtitle_window.show();
        set_subtitle_window_attributes();
        update_subtitle_window_position(cfg.ui.subtitle_y_offset);
        spawn_subtitle_follow_task(Arc::clone(&self.config), window.as_weak());

        // 3. Multi-character UI sync & populate
        let char_items = helpers::build_character_items(&self.character_manager.lock().unwrap());
        let items_model = slint::ModelRc::new(slint::VecModel::from(char_items));
        window.set_character_list(items_model.clone());
        tray.set_character_list(items_model.clone());
        chat_window.set_character_list(items_model.clone());
        settings_window.set_character_list(items_model);

        // Active character & user avatars in ChatWindow
        let base_dir = self.character_manager.lock().unwrap().base_dir.clone();
        let avatar_path = active_char.get_avatar_full_path(&base_dir);
        let char_avatar_img = helpers::load_image_or_empty(&avatar_path);
        chat_window.set_active_character_id(active_char.id.clone().into());
        chat_window.set_active_character_name(active_char.name.clone().into());
        chat_window.set_character_avatar(char_avatar_img);

        let user_avatar_img =
            helpers::load_image_or_empty(std::path::Path::new(&cfg.user.avatar_path));
        chat_window.set_user_avatar(user_avatar_img);
        chat_window.set_user_avatar_enabled(cfg.user.avatar_enabled);

        // Populate SettingsWindow
        settings::populate_global_settings(&settings_window, &cfg);
        settings::populate_character_settings(&settings_window, &active_char, &base_dir, true);
        settings::populate_catalog_lists(&settings_window, &self.model_catalog.lock().unwrap());

        // 4. Calculate dynamic fitted size
        let (base_width, base_height) = self
            .model_catalog
            .lock()
            .unwrap()
            .calculate_fitted_window_size(cfg.window.height as f32);

        let initial_scale = active_char.model.scale.clamp(0.4, 2.5);
        window.set_model_scale(initial_scale);

        let win_width = (base_width * initial_scale).round() as u32;
        let win_height = (base_height * initial_scale).round() as u32;
        window
            .window()
            .set_size(slint::LogicalSize::new(win_width as f32, win_height as f32));

        // 5. Setup WGPU device hook for Live2D renderer initialization
        let live2d_ref = Arc::clone(&self.live2d_renderer);
        let model_path = crate::paths::resolve_resource_path(&active_char.model.path)
            .to_string_lossy()
            .to_string();
        let catalog_hook = Arc::clone(&self.model_catalog);
        let neutral_exp_name = active_char.model.neutral_expression.clone();
        let exp_timeout = active_char.model.expression_timeout;
        let exp_fade = active_char.model.expression_fade_time;
        let mot_fade = active_char.model.motion_fade_out;
        let supersample = active_char.model.supersample;

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
                        1.0,
                        supersample,
                    ) {
                        Ok(mut renderer) => {
                            let neutral_path = catalog_hook
                                .lock()
                                .unwrap()
                                .available_expressions
                                .get(&neutral_exp_name)
                                .cloned();
                            renderer.set_neutral_expression(neutral_path);
                            renderer.set_expression_timeout(exp_timeout);
                            renderer.set_expression_fade_time(exp_fade);
                            renderer.set_motion_fade_out(mot_fade);
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

        // 6. Register callbacks
        callbacks::setup_callbacks(
            &window,
            &tray,
            &settings_window,
            &subtitle_window,
            &chat_window,
            Arc::clone(&chat_manager),
            Arc::clone(&self.config),
            Arc::clone(&self.character_manager),
            Arc::clone(&self.model_catalog),
            Arc::clone(&self.hit_area_handler),
            Arc::clone(&self.tokio_rt),
            Arc::clone(&self.tts_engine),
            Arc::clone(&self.voice_changer),
            Arc::clone(&self.audio_player),
            Arc::clone(&self.live2d_renderer),
            Arc::clone(&self.is_speaking),
        );

        // 7. Start animation & render loop timer with configured Live2D FPS
        render_loop::setup_render_timer(
            &window,
            &subtitle_window,
            Arc::clone(&self.config),
            Arc::clone(&self.character_manager),
            Arc::clone(&self.audio_player),
            Arc::clone(&self.lip_sync_filter),
            Arc::clone(&self.live2d_renderer),
            Arc::clone(&self.is_speaking),
        );

        if std::env::args().any(|arg| arg == "--chat") {
            let _ = chat_window.show();
        }

        // Check CLI flags for automated testing / direct opening
        if std::env::args().any(|arg| arg == "--settings") {
            if let Some(tab_pos) = std::env::args().position(|arg| arg == "--tab") {
                if let Some(val) = std::env::args().nth(tab_pos + 1) {
                    if let Ok(tab_idx) = val.parse::<i32>() {
                        settings_window.set_active_tab(tab_idx);
                    }
                }
            }
            let _ = settings_window.show();
        }

        // 8. Run Slint event loop
        window.run().context("Error during Slint UI execution")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_fitted_dimensions_calculation() {
        let test_bounds = crate::renderer::ModelBounds {
            min_x: -1.0,
            min_y: -2.0,
            max_x: 1.0,
            max_y: 2.0,
        };
        let (fitted_w, fitted_h) = test_bounds.calculate_fitted_dimensions(750.0);
        assert_eq!(fitted_h, 750.0);
        assert_eq!(fitted_w, 420.0);
    }



    #[test]
    fn test_calculate_subtitle_height() {
        let h_empty = calculate_subtitle_height("", 19.0, 460.0);
        assert_eq!(h_empty, 90);

        let h_short = calculate_subtitle_height("你好，我是丛雨！", 19.0, 460.0);
        assert_eq!(h_short, 90);

        let long_text = "现在已经能够正常地与各位进行日常交流与问候啦！\n……不过要是能有更多好吃的甜点就更好啦～\n无论发生什么事情，吾都会一直陪在汝身边的哦！";
        let h_long = calculate_subtitle_height(long_text, 19.0, 460.0);
        assert!(
            h_long >= 110 && h_long <= 220,
            "Expected multi-line height ~120-200px, got {h_long}"
        );

        let huge_text = "很多很多的文字，一直重复出现测试换行。".repeat(20);
        let h_huge = calculate_subtitle_height(&huge_text, 19.0, 460.0);
        assert_eq!(h_huge, 360);
    }
}
