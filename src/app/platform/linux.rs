use std::sync::{Arc, Mutex};
use slint::ComponentHandle;
use crate::app::MainWindow;
use crate::config::AppConfig;

#[cfg(not(windows))]
pub fn set_window_size(_width: u32, _height: u32) {}

#[cfg(not(windows))]
static PET_WINDOW_WEAK: Mutex<Option<slint::Weak<MainWindow>>> = Mutex::new(None);
#[cfg(not(windows))]
static MOUSE_POSITION: Mutex<Option<(i32, i32)>> = Mutex::new(None);
#[cfg(not(windows))]
static PET_WINDOW_SIZE: Mutex<(i32, i32)> = Mutex::new((380, 750));

pub fn register_pet_window(weak: slint::Weak<MainWindow>) {
    if let Ok(mut guard) = PET_WINDOW_WEAK.lock() {
        *guard = Some(weak);
    }
}

#[cfg(not(windows))]
pub fn update_mouse_position(pos: Option<(i32, i32)>) {
    if let Ok(mut guard) = MOUSE_POSITION.lock() {
        *guard = pos;
    }
}

#[cfg(not(windows))]
pub fn update_pet_window_size(width: i32, height: i32) {
    if let Ok(mut guard) = PET_WINDOW_SIZE.lock() {
        *guard = (width, height);
    }
}

#[cfg(not(windows))]
pub fn set_mouse_passthrough(passthrough: bool) {
    if let Ok(guard) = PET_WINDOW_WEAK.lock() {
        if let Some(weak) = guard.as_ref() {
            if let Some(win) = weak.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                win.window().with_winit_window(|winit_win| {
                    if let Err(e) = winit_win.set_cursor_hittest(!passthrough) {
                        tracing::warn!("Failed to set cursor hittest (passthrough={passthrough}): {e}");
                    } else {
                        tracing::info!("Linux mouse passthrough set to: {passthrough}");
                    }
                });
            } else {
                tracing::warn!("Pet window weak reference could not be upgraded for mouse passthrough");
            }
        } else {
            tracing::warn!("Pet window not registered for mouse passthrough");
        }
    }
}

#[cfg(not(windows))]
pub fn set_clipboard_text(text: &str) -> bool {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(cb) => cb,
        Err(e) => {
            tracing::warn!("Failed to initialize clipboard: {e}");
            return false;
        }
    };
    if let Err(e) = clipboard.set_text(text) {
        tracing::warn!("Failed to set clipboard text: {e}");
        return false;
    }
    true
}

#[cfg(not(windows))]
pub fn hide_subtitle_window() {}

#[cfg(not(windows))]
pub fn show_subtitle_window() {}

#[cfg(not(windows))]
pub fn set_subtitle_window_attributes() -> bool {
    true
}

#[cfg(not(windows))]
pub fn update_subtitle_window_position(_y_offset: i32) {}

#[cfg(not(windows))]
pub fn force_update_subtitle_window_position(_y_offset: i32) {}

#[cfg(not(windows))]
pub fn spawn_subtitle_follow_task(
    _config: Arc<Mutex<AppConfig>>,
    _win_weak: slint::Weak<MainWindow>,
) {
}

#[cfg(not(windows))]
pub fn drag_start_record() {}

#[cfg(not(windows))]
pub fn drag_move(_percent_offset: i32) {}

#[cfg(not(windows))]
pub fn drag_end(_config: &Arc<Mutex<AppConfig>>) {}

#[cfg(not(windows))]
pub fn drag_cancel() {}

#[cfg(not(windows))]
pub fn get_chat_hwnd() -> isize {
    0
}

#[cfg(not(windows))]
pub fn subclass_chat_window() {}


#[cfg(not(windows))]
pub fn is_foreground_fullscreen() -> bool {
    false
}

#[cfg(not(windows))]
pub fn get_cursor_pos() -> Option<(i32, i32)> {
    if let Ok(guard) = MOUSE_POSITION.lock() {
        *guard
    } else {
        None
    }
}

#[cfg(not(windows))]
pub fn get_pet_window_rect() -> Option<(i32, i32, i32, i32)> {
    if let Ok(guard) = PET_WINDOW_SIZE.lock() {
        Some((0, 0, guard.0, guard.1))
    } else {
        None
    }
}

#[cfg(not(windows))]
pub fn set_pet_window_alpha(_alpha: f32) {}

#[cfg(not(windows))]
pub fn set_subtitle_window_alpha(_alpha: f32) {}

#[cfg(target_os = "linux")]
pub fn trim_working_set() {
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn trim_working_set() {}


#[cfg(target_os = "linux")]
pub fn get_process_memory_info() -> (usize, usize) {
    if let Ok(content) = std::fs::read_to_string("/proc/self/statm") {
        let mut parts = content.split_whitespace();
        if let (Some(size_str), Some(res_str)) = (parts.next(), parts.next()) {
            if let (Ok(size_pages), Ok(res_pages)) =
                (size_str.parse::<usize>(), res_str.parse::<usize>())
            {
                let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
                let page_size = if page_size > 0 {
                    page_size as usize
                } else {
                    4096
                };
                return (res_pages * page_size, size_pages * page_size);
            }
        }
    }
    (0, 0)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn get_process_memory_info() -> (usize, usize) {
    (0, 0)
}


#[cfg(target_os = "linux")]
pub fn capture_screen_image() -> anyhow::Result<image::RgbaImage> {
    use std::process::Command;

    // 1. Try grim (standard Wayland screen capture)
    if let Ok(output) = Command::new("grim").args(["-t", "png", "-"]).output() {
        if output.status.success() && !output.stdout.is_empty() {
            let img = image::load_from_memory(&output.stdout)?;
            return Ok(img.to_rgba8());
        }
    }

    // 2. Try spectacle (KDE / Wayland)
    let tmp_path = format!("/tmp/murasame_screen_{}.png", std::process::id());
    if let Ok(status) = Command::new("spectacle")
        .args(["-b", "-n", "-o", &tmp_path])
        .status()
    {
        if status.success() && std::path::Path::new(&tmp_path).exists() {
            let res = (|| -> anyhow::Result<image::RgbaImage> {
                let img = image::open(&tmp_path)?;
                Ok(img.to_rgba8())
            })();
            let _ = std::fs::remove_file(&tmp_path);
            if let Ok(img) = res {
                return Ok(img);
            }
        }
    }

    // 3. Try maim (X11)
    if let Ok(output) = Command::new("maim").args(["-u"]).output() {
        if output.status.success() && !output.stdout.is_empty() {
            let img = image::load_from_memory(&output.stdout)?;
            return Ok(img.to_rgba8());
        }
    }

    // 4. Try scrot (X11)
    if let Ok(status) = Command::new("scrot").args(["-o", &tmp_path]).status() {
        if status.success() && std::path::Path::new(&tmp_path).exists() {
            let res = (|| -> anyhow::Result<image::RgbaImage> {
                let img = image::open(&tmp_path)?;
                Ok(img.to_rgba8())
            })();
            let _ = std::fs::remove_file(&tmp_path);
            if let Ok(img) = res {
                return Ok(img);
            }
        }
    }

    Err(anyhow::anyhow!(
        "无法获取屏幕截图：请确保系统已安装并配置截图工具（Wayland 推荐 grim，X11 推荐 maim 或 scrot）"
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn capture_screen_image() -> anyhow::Result<image::RgbaImage> {
    Err(anyhow::anyhow!("当前操作系统平台暂未实现原生屏幕截图"))
}
