use std::time::{Duration, Instant};
use crate::app::MainWindow;

#[cfg(windows)]
use super::windows::{set_mouse_passthrough, set_pet_window_alpha, set_subtitle_window_alpha};

pub fn calculate_subtitle_height(text: &str, font_size_px: f32, max_inner_width: f32) -> i32 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 72;
    }
    let mut total_lines = 0;
    for paragraph in trimmed.split('\n') {
        let mut line_len = 0.0;
        let mut p_lines = 1;
        for ch in paragraph.chars() {
            let char_width = if ch.is_ascii() {
                font_size_px * 0.58
            } else {
                font_size_px * 1.05
            };
            if line_len + char_width > max_inner_width && line_len > 0.0 {
                p_lines += 1;
                line_len = char_width;
            } else {
                line_len += char_width;
            }
        }
        total_lines += p_lines;
    }
    let line_height = (font_size_px * 1.5).max(26.0);
    let padding_y = 28.0;
    let needed = (total_lines as f32 * line_height + padding_y).ceil() as i32;
    needed.clamp(72, 360)
}


pub fn start_window_drag(win: &MainWindow) {
    #[cfg(not(windows))]
    {
        use slint::ComponentHandle;
        use slint::winit_030::WinitWindowAccessor;
        win.window().with_winit_window(|winit_win| {
            let _ = winit_win.drag_window();
        });
    }
    #[cfg(windows)]
    {
        let _ = win;
    }
}


pub struct FullscreenFadeController {
    raw_fullscreen: bool,
    debounced_fullscreen: bool,
    state_change_time: Instant,
    current_alpha: f32,
    target_alpha: f32,
    #[cfg(windows)]
    was_faded_out: bool,
}

impl FullscreenFadeController {
    pub fn new() -> Self {
        Self {
            raw_fullscreen: false,
            debounced_fullscreen: false,
            state_change_time: Instant::now(),
            current_alpha: 1.0,
            target_alpha: 1.0,
            #[cfg(windows)]
            was_faded_out: false,
        }
    }

    pub fn update(
        &mut self,
        is_fullscreen_raw: bool,
        auto_hide: bool,
        delta: f32,
        _user_mouse_passthrough: bool,
    ) -> f32 {
        // 0.5s debounce filter on raw fullscreen status
        if is_fullscreen_raw != self.raw_fullscreen {
            self.raw_fullscreen = is_fullscreen_raw;
            self.state_change_time = Instant::now();
        } else if is_fullscreen_raw != self.debounced_fullscreen {
            if self.state_change_time.elapsed() >= Duration::from_millis(500) {
                self.debounced_fullscreen = is_fullscreen_raw;
            }
        }

        if auto_hide && self.debounced_fullscreen {
            self.target_alpha = 0.0;
        } else {
            self.target_alpha = 1.0;
        }

        // Fade transition: ~0.35s duration
        let fade_speed = delta / 0.35;
        if self.current_alpha < self.target_alpha {
            self.current_alpha = (self.current_alpha + fade_speed).min(self.target_alpha);
        } else if self.current_alpha > self.target_alpha {
            self.current_alpha = (self.current_alpha - fade_speed).max(self.target_alpha);
        }

        // Apply alpha to Win32 window
        #[cfg(windows)]
        {
            set_pet_window_alpha(self.current_alpha);
            set_subtitle_window_alpha(self.current_alpha);

            if self.current_alpha <= 0.001 {
                if !self.was_faded_out {
                    self.was_faded_out = true;
                    set_mouse_passthrough(true);
                }
            } else if self.was_faded_out {
                self.was_faded_out = false;
                set_mouse_passthrough(_user_mouse_passthrough);
            }
        }

        self.current_alpha
    }

    pub fn is_animating(&self) -> bool {
        (self.current_alpha - self.target_alpha).abs() > 0.001
    }

    pub fn is_fully_hidden(&self) -> bool {
        self.current_alpha <= 0.001 && self.target_alpha == 0.0
    }

    pub fn is_fullscreen_active(&self) -> bool {
        self.debounced_fullscreen
    }

    pub fn current_alpha(&self) -> f32 {
        self.current_alpha
    }
}

