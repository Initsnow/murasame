use slint::ComponentHandle;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{
    audio::{AudioPlayer, LipSyncFilter},
    character::CharacterManager,
    config::AppConfig,
    renderer::Live2dRenderer,
};

use super::{
    MainWindow, SubtitleWindow,
    platform::{
        CHAT_OPEN_STATE, LAST_SUBTITLE_RECT, SUBTITLE_TARGET_HEIGHT, calculate_subtitle_height,
        force_update_subtitle_window_position, update_subtitle_window_position,
    },
};

thread_local! {
    static TIMER_CONTROLLER: RefCell<Option<Box<dyn FnMut(u32)>>> = const { RefCell::new(None) };
}

pub fn update_render_fps(fps: u32) {
    let handled = TIMER_CONTROLLER.with(|cell| {
        if let Some(controller) = cell.borrow_mut().as_mut() {
            controller(fps);
            true
        } else {
            false
        }
    });

    if !handled {
        let _ = slint::invoke_from_event_loop(move || {
            TIMER_CONTROLLER.with(|cell| {
                if let Some(controller) = cell.borrow_mut().as_mut() {
                    controller(fps);
                }
            });
        });
    }
}

pub fn setup_render_timer(
    window: &MainWindow,
    subtitle_window: &SubtitleWindow,
    config: Arc<Mutex<AppConfig>>,
    character_manager: Arc<Mutex<CharacterManager>>,
    audio_player: Arc<Mutex<AudioPlayer>>,
    lip_sync_filter: Arc<Mutex<LipSyncFilter>>,
    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    is_speaking: Arc<AtomicBool>,
) {
    let live2d_ref = Arc::clone(&live2d_renderer);
    let player_ref = Arc::clone(&audio_player);
    let lip_filter = Arc::clone(&lip_sync_filter);
    let config_timer = Arc::clone(&config);
    let is_speaking_timer = Arc::clone(&is_speaking);
    let win_weak = window.as_weak();
    let sub_weak = subtitle_window.as_weak();

    let initial_fps = character_manager
        .lock()
        .map(|cm| {
            cm.get_active_character()
                .map(|c| c.model.fps)
                .unwrap_or(60)
        })
        .unwrap_or(60);

    let audio_state = player_ref.lock().map(|p| p.state_handle()).unwrap_or_else(|_| {
        audio_player.lock().unwrap().state_handle()
    });

    let mut subtitle_last_active = Instant::now();
    let mut last_subtitle_text = String::new();
    let mut cached_subtitle_height: i32 = 90;

    let timer = Rc::new(RefCell::new(slint::Timer::default()));
    let tick_core = Arc::new(Mutex::new(move || {
        let (is_playing_audio, raw_rms) = audio_state.audio_status();
        is_speaking_timer.store(is_playing_audio, Ordering::Relaxed);

        let mouth_open = if let Ok(mut filter) = lip_filter.lock() {
            filter.update(raw_rms)
        } else {
            0.0
        };

        let win_opt = win_weak.upgrade();
        let sub_opt = sub_weak.upgrade();

        if let Some(win) = &win_opt {
            if let Ok(mut guard) = live2d_ref.lock() {
                if let Some(renderer) = guard.as_mut() {
                    renderer.set_mouth_open(mouth_open);

                    match renderer.tick_and_render() {
                        Ok(image) => {
                            win.set_pet_texture(image);
                            win.window().request_redraw();
                        }
                        Err(e) => {
                            tracing::error!("Render frame error: {e}");
                        }
                    }
                }
            }
        }

        if let (Some(win), Some(sub)) = (win_opt, sub_opt) {
            let is_loading = win.get_is_loading();
            let chat_open = win.get_chat_open();
            let sub_text = sub.get_subtitle_text();
            let sub_str = sub_text.as_str();

            let text_changed = !sub_str.is_empty() && sub_str != last_subtitle_text;
            if text_changed {
                last_subtitle_text = sub_str.to_string();
                subtitle_last_active = Instant::now();
                sub.set_subtitle_opacity(1.0);
                sub.set_subtitle_visible(true);
                sub.window().request_redraw();

                #[cfg(windows)]
                {
                    let (y_off, font_size) = {
                        let cfg = config_timer.lock().unwrap();
                        (cfg.ui.subtitle_y_offset as i32, cfg.ui.subtitle_font_size)
                    };
                    cached_subtitle_height = calculate_subtitle_height(sub_str, font_size as f32, 460.0);
                    SUBTITLE_TARGET_HEIGHT.store(cached_subtitle_height, Ordering::Relaxed);
                    force_update_subtitle_window_position(y_off);
                }
            }

            #[cfg(windows)]
            {
                CHAT_OPEN_STATE.store(chat_open, Ordering::Relaxed);
                if !sub_str.is_empty() {
                    let cur_h = sub.get_window_height();
                    if (cur_h - cached_subtitle_height as f32).abs() > 0.5 {
                        sub.set_window_height(cached_subtitle_height as f32);
                    }
                    if (sub.get_window_width() - 500.0).abs() > 0.5 {
                        sub.set_window_width(500.0);
                    }
                }
            }

            if is_playing_audio || is_loading {
                subtitle_last_active = Instant::now();
                sub.set_subtitle_opacity(1.0);
                sub.set_subtitle_visible(true);
            } else if !sub_str.is_empty() {
                let elapsed = subtitle_last_active.elapsed();
                let wait_duration = Duration::from_millis(1500);
                let fade_duration = Duration::from_millis(400);
                if elapsed > wait_duration {
                    let fade_elapsed = elapsed - wait_duration;
                    if fade_elapsed < fade_duration {
                        let progress = fade_elapsed.as_secs_f32() / fade_duration.as_secs_f32();
                        sub.set_subtitle_opacity((1.0 - progress).max(0.0));
                        sub.window().request_redraw();
                    } else {
                        sub.set_subtitle_opacity(0.0);
                        sub.set_subtitle_visible(false);
                        sub.set_subtitle_text("".into());
                        last_subtitle_text.clear();
                        sub.window().request_redraw();
                        #[cfg(windows)]
                        if let Ok(mut last) = LAST_SUBTITLE_RECT.lock() {
                            *last = None;
                        }
                    }
                }
            }

            if sub.get_subtitle_visible() {
                let y_off = config_timer.lock().unwrap().ui.subtitle_y_offset as i32;
                update_subtitle_window_position(y_off);
                sub.window().request_redraw();
            }
        }
    }));

    let tick_slint = Arc::clone(&tick_core);
    let timer_clone = Rc::clone(&timer);

    let start_with_fps = move |fps: u32| {
        let clamped_fps = fps.clamp(15, 144);
        let interval = Duration::from_nanos(1_000_000_000 / clamped_fps as u64);
        let tick = Arc::clone(&tick_slint);
        timer_clone.borrow_mut().start(
            slint::TimerMode::Repeated,
            interval,
            move || {
                if let Ok(mut cb) = tick.lock() {
                    cb();
                }
            },
        );
        tracing::info!("Live2D render timer started at {clamped_fps} FPS (interval: {interval:?})");
    };

    start_with_fps(initial_fps);

    TIMER_CONTROLLER.with(|cell| {
        *cell.borrow_mut() = Some(Box::new(start_with_fps));
    });
}
