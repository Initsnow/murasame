use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering},
    },
    time::{Duration, Instant},
};

use super::MainWindow;
use crate::config::AppConfig;

pub static SUBTITLE_TARGET_HEIGHT: AtomicI32 = AtomicI32::new(100);
pub static SUBTITLE_Y_OFFSET: AtomicI32 = AtomicI32::new(75);
pub static SUBTITLE_FONT_SIZE: AtomicI32 = AtomicI32::new(14);
pub static CHAT_OPEN_STATE: AtomicBool = AtomicBool::new(false);
pub static CHAT_LIFT_STATE: AtomicI32 = AtomicI32::new(0);
pub static LAST_SUBTITLE_RECT: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);

#[cfg(windows)]
static ORIGINAL_SUBTITLE_WNDPROC: AtomicIsize = AtomicIsize::new(0);
#[cfg(windows)]
static SUBTITLE_SUBCLASSED_HWND: AtomicIsize = AtomicIsize::new(0);
#[cfg(windows)]
static CACHED_PET_HWND: AtomicIsize = AtomicIsize::new(0);
#[cfg(windows)]
static CACHED_SUB_HWND: AtomicIsize = AtomicIsize::new(0);

#[cfg(windows)]
pub fn get_pet_hwnd() -> isize {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindow};

    let cached = CACHED_PET_HWND.load(Ordering::Relaxed);
    if cached != 0 && unsafe { IsWindow(cached) } != 0 {
        return cached;
    }
    let title: Vec<u16> = OsStr::new("Murasame Desktop Pet\0").encode_wide().collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd != 0 {
        CACHED_PET_HWND.store(hwnd, Ordering::Relaxed);
    }
    hwnd
}

#[cfg(windows)]
pub fn get_subtitle_hwnd() -> isize {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindow};

    let cached = CACHED_SUB_HWND.load(Ordering::Relaxed);
    if cached != 0 && unsafe { IsWindow(cached) } != 0 {
        return cached;
    }
    let title: Vec<u16> = OsStr::new("Murasame Subtitle\0").encode_wide().collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd != 0 {
        CACHED_SUB_HWND.store(hwnd, Ordering::Relaxed);
    }
    hwnd
}

#[cfg(windows)]
pub fn set_window_size(width: u32, height: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos,
    };

    let hwnd = get_pet_hwnd();
    if hwnd != 0 {
        unsafe {
            SetWindowPos(
                hwnd,
                0,
                0,
                0,
                width as i32,
                height as i32,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(not(windows))]
pub fn set_window_size(_width: u32, _height: u32) {}

#[cfg(windows)]
pub fn set_mouse_passthrough(passthrough: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongW, SetWindowLongW, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };

    let hwnd = get_pet_hwnd();
    if hwnd != 0 {
        unsafe {
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            let new_style = if passthrough {
                ex_style | (WS_EX_TRANSPARENT as i32) | (WS_EX_LAYERED as i32)
            } else {
                ex_style & !(WS_EX_TRANSPARENT as i32)
            };
            SetWindowLongW(hwnd, GWL_EXSTYLE, new_style);
            tracing::info!("Mouse passthrough set to: {passthrough}");
        }
    } else {
        tracing::warn!("Could not find window 'Murasame Desktop Pet' for mouse passthrough");
    }
}

#[cfg(not(windows))]
pub fn set_mouse_passthrough(_passthrough: bool) {}

pub fn calculate_subtitle_height(text: &str, font_size_px: f32, max_inner_width: f32) -> i32 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 90;
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
    let line_height = (font_size_px * 1.55).max(28.0);
    let padding_y = 38.0;
    let needed = (total_lines as f32 * line_height + padding_y).ceil() as i32;
    needed.clamp(90, 360)
}

#[cfg(windows)]
pub fn set_clipboard_text(text: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    let wide: Vec<u16> = OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let bytes_len = wide.len() * std::mem::size_of::<u16>();

    unsafe {
        if OpenClipboard(0) == 0 {
            return false;
        }
        EmptyClipboard();
        let h_mem = GlobalAlloc(GMEM_MOVEABLE, bytes_len);
        if !h_mem.is_null() {
            let ptr = GlobalLock(h_mem) as *mut u16;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                GlobalUnlock(h_mem);
                SetClipboardData(13 /* CF_UNICODETEXT */, h_mem as isize);
            }
        }
        CloseClipboard();
    }

    true
}

#[cfg(not(windows))]
pub fn set_clipboard_text(_text: &str) -> bool {
    false
}

#[cfg(windows)]
unsafe extern "system" fn subtitle_wndproc(
    hwnd: isize,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{CallWindowProcW, DefWindowProcW};

    match msg {
        0x0021 /* WM_MOUSEACTIVATE */ => 4 /* MA_NOACTIVATEANDEAT */,
        0x0086 /* WM_NCACTIVATE */ => 1 /* TRUE: prevent DefWindowProc from painting caption */,
        0x0085 /* WM_NCPAINT */ => 0,
        0x0083 /* WM_NCCALCSIZE */ if wparam != 0 => 0,
        _ => {
            let orig = ORIGINAL_SUBTITLE_WNDPROC.load(Ordering::Relaxed);
            if orig != 0 {
                unsafe {
                    CallWindowProcW(std::mem::transmute(orig), hwnd, msg, wparam, lparam)
                }
            } else {
                unsafe {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
        }
    }
}

#[cfg(windows)]
pub fn set_subtitle_window_attributes() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GetWindowLongW, LWA_ALPHA, SW_HIDE, SW_SHOWNOACTIVATE,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetLayeredWindowAttributes,
        SetWindowLongPtrW, SetWindowLongW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };

    let pet_hwnd = get_pet_hwnd();
    let sub_hwnd = get_subtitle_hwnd();
    unsafe {
        if pet_hwnd != 0 && sub_hwnd != 0 {
            #[link(name = "dwmapi")]
            unsafe extern "system" {
                fn DwmSetWindowAttribute(
                    hwnd: isize,
                    dwAttribute: u32,
                    pvAttribute: *const std::ffi::c_void,
                    cbAttribute: u32,
                ) -> i32;
            }

            // 1. Force DWM to completely disable non-client rendering (prevent Aero caption/title bar)
            const DWMWA_NCRENDERING_POLICY: u32 = 2;
            const DWMNCRP_DISABLED: u32 = 1;
            DwmSetWindowAttribute(
                sub_hwnd as isize,
                DWMWA_NCRENDERING_POLICY,
                &DWMNCRP_DISABLED as *const _ as *const _,
                4,
            );

            // 2. Prevent Windows 11 rounded corners decoration & default system borders
            const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
            const DWMWCP_DONOTROUND: u32 = 1;
            DwmSetWindowAttribute(
                sub_hwnd as isize,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &DWMWCP_DONOTROUND as *const _ as *const _,
                4,
            );

            const DWMWA_BORDER_COLOR: u32 = 34;
            const DWMWA_COLOR_NONE: u32 = 0xFFFFFFFE;
            DwmSetWindowAttribute(
                sub_hwnd as isize,
                DWMWA_BORDER_COLOR,
                &DWMWA_COLOR_NONE as *const _ as *const _,
                4,
            );

            // 3. Exclude from DWM Peek and thumbnail previews on taskbar hover
            let disallow: u32 = 1;
            DwmSetWindowAttribute(
                sub_hwnd as isize,
                12, /* DWMWA_EXCLUDED_FROM_PEEK */
                &disallow as *const _ as *const _,
                4,
            );
            DwmSetWindowAttribute(
                sub_hwnd as isize,
                11, /* DWMWA_DISALLOW_PEEK */
                &disallow as *const _ as *const _,
                4,
            );

            // 4. Temporarily hide window so shell updates window management cache
            ShowWindow(sub_hwnd, SW_HIDE);

            // 5. Subclass window procedure: suppress non-client title bar painting and mouse activation
            const GWLP_WNDPROC: i32 = -4;
            let current_subclassed = SUBTITLE_SUBCLASSED_HWND.load(Ordering::SeqCst);
            let wndproc_ptr = subtitle_wndproc as *const () as isize;
            if current_subclassed != sub_hwnd {
                let prev = SetWindowLongPtrW(sub_hwnd, GWLP_WNDPROC, wndproc_ptr);
                if prev != 0 && prev != wndproc_ptr {
                    ORIGINAL_SUBTITLE_WNDPROC.store(prev, Ordering::SeqCst);
                    SUBTITLE_SUBCLASSED_HWND.store(sub_hwnd, Ordering::SeqCst);
                }
            }

            // 6. Completely strip all standard window chrome styles and ensure WS_POPUP
            const WS_CAPTION: u32 = 0x00C00000;
            const WS_BORDER: u32 = 0x00800000;
            const WS_DLGFRAME: u32 = 0x00400000;
            const WS_THICKFRAME: u32 = 0x00040000;
            const WS_SYSMENU: u32 = 0x00080000;
            const WS_MINIMIZEBOX: u32 = 0x00020000;
            const WS_MAXIMIZEBOX: u32 = 0x00010000;
            const WS_POPUP: u32 = 0x80000000;
            const WS_CLIPSIBLINGS: u32 = 0x04000000;
            const WS_CLIPCHILDREN: u32 = 0x02000000;
            const ALL_CHROME: u32 = WS_CAPTION
                | WS_BORDER
                | WS_DLGFRAME
                | WS_THICKFRAME
                | WS_SYSMENU
                | WS_MINIMIZEBOX
                | WS_MAXIMIZEBOX;

            let style = GetWindowLongW(sub_hwnd, GWL_STYLE) as u32;
            let new_style_main =
                ((style & !ALL_CHROME) | WS_POPUP | WS_CLIPSIBLINGS | WS_CLIPCHILDREN) as i32;
            SetWindowLongW(sub_hwnd, GWL_STYLE, new_style_main);

            // 7. Set extended styles: layered window + transparent for native OS mouse pass-through
            let ex_style = GetWindowLongW(sub_hwnd, GWL_EXSTYLE);
            let new_style = (ex_style & !(WS_EX_APPWINDOW as i32))
                | (WS_EX_LAYERED as i32)
                | (WS_EX_TRANSPARENT as i32)
                | (WS_EX_TOOLWINDOW as i32)
                | (WS_EX_NOACTIVATE as i32);
            SetWindowLongW(sub_hwnd, GWL_EXSTYLE, new_style);

            // Initialize layered window attributes once (opacity 255)
            SetLayeredWindowAttributes(sub_hwnd, 0, 255, LWA_ALPHA);

            // 8. Set owner window to pet_hwnd (Win32 automatically keeps owned window above owner)
            const GWLP_HWNDPARENT: i32 = -8;
            SetWindowLongPtrW(sub_hwnd, GWLP_HWNDPARENT, pet_hwnd as isize);

            // 9. Notify DWM and system of frame changes and bring to topmost band
            const HWND_TOPMOST: isize = -1;
            SetWindowPos(
                sub_hwnd,
                HWND_TOPMOST as _,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );

            // 10. Show window again without activating
            ShowWindow(sub_hwnd, SW_SHOWNOACTIVATE);
            tracing::info!(
                "SubtitleWindow configured as frameless owned tool window (non-client rendering disabled)!"
            );
            return true;
        }
    }
    false
}

#[cfg(windows)]
pub fn hide_subtitle_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    let sub_hwnd = get_subtitle_hwnd();
    if sub_hwnd != 0 {
        unsafe {
            ShowWindow(sub_hwnd, SW_HIDE);
        }
    }
}

#[cfg(windows)]
pub fn show_subtitle_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
    let sub_hwnd = get_subtitle_hwnd();
    if sub_hwnd != 0 {
        unsafe {
            ShowWindow(sub_hwnd, SW_SHOWNOACTIVATE);
        }
    }
}

#[cfg(not(windows))]
pub fn hide_subtitle_window() {}

#[cfg(not(windows))]
pub fn show_subtitle_window() {}

#[cfg(not(windows))]
pub fn set_subtitle_window_attributes() -> bool {
    true
}

#[cfg(windows)]
pub fn update_subtitle_window_position(percent_offset: i32) {
    update_subtitle_window_position_inner(percent_offset, false);
}

#[cfg(windows)]
pub fn force_update_subtitle_window_position(percent_offset: i32) {
    update_subtitle_window_position_inner(percent_offset, true);
}

#[cfg(not(windows))]
pub fn update_subtitle_window_position(_y_offset: i32) {}

#[cfg(not(windows))]
pub fn force_update_subtitle_window_position(_y_offset: i32) {}

#[cfg(windows)]
fn update_subtitle_window_position_inner(percent_offset: i32, force: bool) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        UpdateWindow,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SWP_NOACTIVATE, SetWindowPos,
    };

    let pet_hwnd = get_pet_hwnd();
    let sub_hwnd = get_subtitle_hwnd();

    unsafe {
        if pet_hwnd != 0 && sub_hwnd != 0 {
            let mut pet_rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(pet_hwnd, &mut pet_rect) != 0 {
                // 1. 获取窗口真实的 DPI 缩放因子
                #[link(name = "user32")]
                unsafe extern "system" {
                    fn GetDpiForWindow(hwnd: isize) -> u32;
                }
                let dpi = GetDpiForWindow(pet_hwnd as isize);
                let scale = if dpi > 0 { dpi as f64 / 96.0 } else { 1.0 };

                let pet_w = pet_rect.right - pet_rect.left;
                let pet_h = (pet_rect.bottom - pet_rect.top).max(1);

                // 2. 将 Slint 的逻辑尺寸换算为真实的物理像素
                let logical_sub_w = 500.0;
                let logical_sub_h = SUBTITLE_TARGET_HEIGHT
                    .load(Ordering::Relaxed)
                    .clamp(90, 360) as f64;
                let sub_w = (logical_sub_w * scale).round() as i32;
                let sub_h = (logical_sub_h * scale).round() as i32;

                // 3. 水平居中：以桌宠物理宽度为基准绝对居中
                let mut sub_x = pet_rect.left + (pet_w - sub_w) / 2;

                // 4. 屏幕工作区限制
                let mut minfo: MONITORINFO = std::mem::zeroed();
                minfo.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                let hmon = MonitorFromWindow(pet_hwnd, MONITOR_DEFAULTTONEAREST);
                let (min_y, max_y, min_x, max_x) =
                    if hmon != 0 && GetMonitorInfoW(hmon, &mut minfo) != 0 {
                        (
                            minfo.rcWork.top + 6,
                            minfo.rcWork.bottom - sub_h - 6,
                            minfo.rcWork.left + 6,
                            minfo.rcWork.right - sub_w - 6,
                        )
                    } else {
                        (0, 2000, 0, 3000)
                    };

                // 5. 垂直位置计算：基准为用户配置的 subtitle_y_offset
                let target_y = pet_rect.top
                    + ((pet_h as f64 * (percent_offset as f64 / 100.0)).round() as i32);
                let normal_sub_y = target_y - sub_h / 2;

                // 6. 聊天输入框重叠遮挡检测与避让
                let is_chat_open = CHAT_OPEN_STATE.load(Ordering::Relaxed);
                let chat_lift = CHAT_LIFT_STATE.load(Ordering::Relaxed) as f64;
                let sub_y = if is_chat_open {
                    let pet_logical_h = pet_h as f64 / scale;
                    let card_h = 46.0;
                    let card_margin = 18.0;
                    let card_top_logical =
                        (pet_logical_h - card_h - card_margin - chat_lift).max(10.0);
                    let chat_card_top_physical =
                        pet_rect.top + (card_top_logical * scale).round() as i32;
                    let gap = (8.0 * scale).round() as i32;

                    if normal_sub_y + sub_h > chat_card_top_physical {
                        (chat_card_top_physical - sub_h - gap).clamp(min_y, max_y)
                    } else {
                        normal_sub_y.clamp(min_y, max_y)
                    }
                } else {
                    normal_sub_y.clamp(min_y, max_y)
                };

                sub_x = sub_x.clamp(min_x, max_x);

                if !force {
                    if let Ok(mut last) = LAST_SUBTITLE_RECT.lock() {
                        if *last == Some((sub_x, sub_y, sub_w, sub_h)) {
                            return;
                        }
                        *last = Some((sub_x, sub_y, sub_w, sub_h));
                    }
                } else if let Ok(mut last) = LAST_SUBTITLE_RECT.lock() {
                    *last = Some((sub_x, sub_y, sub_w, sub_h));
                }

                const HWND_TOPMOST: isize = -1;
                SetWindowPos(
                    sub_hwnd,
                    HWND_TOPMOST as _,
                    sub_x,
                    sub_y,
                    sub_w,
                    sub_h,
                    SWP_NOACTIVATE,
                );

                InvalidateRect(sub_hwnd, std::ptr::null(), 1);
                UpdateWindow(sub_hwnd);
            }
        }
    }
}

#[cfg(windows)]
pub fn spawn_subtitle_follow_task(
    config: Arc<Mutex<AppConfig>>,
    win_weak: slint::Weak<MainWindow>,
) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongW, GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowPos,
    };
    const VK_LBUTTON: i32 = 0x01;

    std::thread::spawn(move || {
        let mut configured = false;
        let mut restored_initial_pos = false;
        let mut was_mouse_down = false;
        let mut last_saved_pos: Option<(i32, i32)> = None;
        let mut last_known_pet_pos: Option<(i32, i32)> = None;
        let mut last_lift: f32 = 0.0;

        loop {
            std::thread::sleep(Duration::from_millis(15));
            let pet_hwnd = get_pet_hwnd();
            let sub_hwnd = get_subtitle_hwnd();
            unsafe {

                if pet_hwnd != 0 && !restored_initial_pos {
                    restored_initial_pos = true;
                    let (saved_x, saved_y) = {
                        let cfg = config.lock().unwrap();
                        (cfg.window.x, cfg.window.y)
                    };
                    if let (Some(x), Some(y)) = (saved_x, saved_y) {
                        SetWindowPos(
                            pet_hwnd,
                            0,
                            x,
                            y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        last_saved_pos = Some((x, y));
                        last_known_pet_pos = Some((x, y));
                    }
                    let y_off = config.lock().unwrap().ui.subtitle_y_offset;
                    update_subtitle_window_position(y_off);
                }

                if sub_hwnd != 0 {
                    let style = GetWindowLongW(sub_hwnd, GWL_STYLE) as u32;
                    const ALL_CHROME: u32 = 0x00C00000
                        | 0x00800000
                        | 0x00400000
                        | 0x00040000
                        | 0x00080000
                        | 0x00020000
                        | 0x00010000;
                    if !configured || (style & ALL_CHROME) != 0 {
                        if set_subtitle_window_attributes() {
                            let y_off = config.lock().unwrap().ui.subtitle_y_offset;
                            update_subtitle_window_position(y_off);
                            configured = true;
                        }
                    }
                }

                if pet_hwnd != 0 {
                    let mut rect = RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    };
                    if GetWindowRect(pet_hwnd, &mut rect) != 0 {
                        if last_known_pet_pos != Some((rect.left, rect.top)) {
                            last_known_pet_pos = Some((rect.left, rect.top));
                            let y_off = config.lock().unwrap().ui.subtitle_y_offset;
                            update_subtitle_window_position(y_off);
                        }
                    }

                    let mouse_down = (GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000) != 0;
                    if mouse_down {
                        was_mouse_down = true;
                    } else if was_mouse_down {
                        was_mouse_down = false;
                        if GetWindowRect(pet_hwnd, &mut rect) != 0 {
                            if last_saved_pos != Some((rect.left, rect.top)) {
                                last_saved_pos = Some((rect.left, rect.top));
                                if let Ok(mut cfg) = config.lock() {
                                    cfg.window.x = Some(rect.left);
                                    cfg.window.y = Some(rect.top);
                                    let _ = cfg.save_to_disk(crate::paths::config_path());
                                }
                            }
                        }
                    }

                    let mut pet_rect = RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    };
                    if GetWindowRect(pet_hwnd, &mut pet_rect) != 0 {
                        let monitor = MonitorFromWindow(pet_hwnd, MONITOR_DEFAULTTONEAREST);
                        if monitor != 0 {
                            let mut minfo = MONITORINFO {
                                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                                rcMonitor: RECT {
                                    left: 0,
                                    top: 0,
                                    right: 0,
                                    bottom: 0,
                                },
                                rcWork: RECT {
                                    left: 0,
                                    top: 0,
                                    right: 0,
                                    bottom: 0,
                                },
                                dwFlags: 0,
                            };
                            if GetMonitorInfoW(monitor, &mut minfo) != 0 {
                                let overflow = (pet_rect.bottom - minfo.rcWork.bottom).max(0);
                                let lift = overflow as f32;
                                if (lift - last_lift).abs() > 2.0 {
                                    last_lift = lift;
                                    CHAT_LIFT_STATE.store(lift as i32, Ordering::Relaxed);
                                    let win_weak_inner = win_weak.clone();
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(win) = win_weak_inner.upgrade() {
                                            win.set_chat_lift_offset(lift);
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    });
}

#[cfg(not(windows))]
pub fn spawn_subtitle_follow_task(
    _config: Arc<Mutex<AppConfig>>,
    _win_weak: slint::Weak<MainWindow>,
) {
}

#[cfg(windows)]
struct DragState {
    hwnd: isize,
    anchor_cursor: (i32, i32),
    anchor_win: (i32, i32),
    last_win_pos: (i32, i32),
    is_active: bool,
}

#[cfg(windows)]
static DRAG_STATE: Mutex<Option<DragState>> = Mutex::new(None);

#[cfg(windows)]
pub fn drag_start_record() {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect};

    let pet_hwnd = get_pet_hwnd();
    unsafe {
        if pet_hwnd != 0 {
            let mut cursor = POINT { x: 0, y: 0 };
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetCursorPos(&mut cursor) != 0 && GetWindowRect(pet_hwnd, &mut rect) != 0 {
                SetCapture(pet_hwnd);
                if let Ok(mut state) = DRAG_STATE.lock() {
                    *state = Some(DragState {
                        hwnd: pet_hwnd,
                        anchor_cursor: (cursor.x, cursor.y),
                        anchor_win: (rect.left, rect.top),
                        last_win_pos: (rect.left, rect.top),
                        is_active: true,
                    });
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub fn drag_start_record() {}

#[cfg(windows)]
pub fn drag_move(percent_offset: i32) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };

    let mut to_move = None;
    if let Ok(mut state_guard) = DRAG_STATE.lock() {
        if let Some(state) = state_guard.as_mut() {
            if state.is_active {
                let mut cursor = POINT { x: 0, y: 0 };
                unsafe {
                    if GetCursorPos(&mut cursor) != 0 {
                        let dx = cursor.x - state.anchor_cursor.0;
                        let dy = cursor.y - state.anchor_cursor.1;
                        let new_x = state.anchor_win.0 + dx;
                        let new_y = state.anchor_win.1 + dy;
                        if (new_x, new_y) != state.last_win_pos {
                            state.last_win_pos = (new_x, new_y);
                            to_move = Some((state.hwnd, new_x, new_y));
                        }
                    }
                }
            }
        }
    }

    if let Some((hwnd, x, y)) = to_move {
        unsafe {
            SetWindowPos(
                hwnd,
                0,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        update_subtitle_window_position(percent_offset);
    }
}

#[cfg(not(windows))]
pub fn drag_move(_percent_offset: i32) {}

#[cfg(windows)]
pub fn drag_end(config: &Arc<Mutex<AppConfig>>) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let mut last_hwnd = 0;
    if let Ok(mut state_guard) = DRAG_STATE.lock() {
        if let Some(state) = state_guard.take() {
            last_hwnd = state.hwnd;
        }
    }

    unsafe {
        ReleaseCapture();
        if last_hwnd != 0 {
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(last_hwnd, &mut rect) != 0 {
                if let Ok(mut cfg) = config.lock() {
                    cfg.window.x = Some(rect.left);
                    cfg.window.y = Some(rect.top);
                    let _ = cfg.save_to_disk(crate::paths::config_path());
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub fn drag_end(_config: &Arc<Mutex<AppConfig>>) {}

#[cfg(windows)]
pub fn drag_cancel() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;

    if let Ok(mut state_guard) = DRAG_STATE.lock() {
        state_guard.take();
    }
    unsafe {
        ReleaseCapture();
    }
}

#[cfg(not(windows))]
pub fn drag_cancel() {}

// =========================================================================
// Fullscreen App Detection & Window Alpha / Avoidance Control
// =========================================================================

#[cfg(windows)]
pub fn is_foreground_fullscreen() -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic,
        IsWindow,
    };

    unsafe {
        let fg_hwnd = GetForegroundWindow();
        if fg_hwnd == 0 || IsWindow(fg_hwnd) == 0 {
            return false;
        }

        // 1. Minimized window is not fullscreen
        if IsIconic(fg_hwnd) != 0 {
            return false;
        }

        // 2. Ignore Murasame's own windows (Pet, Subtitle, Settings, Chat, etc.)
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(fg_hwnd, &mut pid);
        if pid == GetCurrentProcessId() {
            return false;
        }

        // 3. Ignore Windows Shell / Desktop / Taskbar windows
        let mut class_buf = [0u16; 256];
        let len = GetClassNameW(fg_hwnd, class_buf.as_mut_ptr(), 256);
        if len > 0 {
            let class_str = String::from_utf16_lossy(&class_buf[..len as usize]);
            if class_str == "Progman"
                || class_str == "WorkerW"
                || class_str == "Shell_TrayWnd"
                || class_str == "Shell_SecondaryTrayWnd"
            {
                return false;
            }
        }

        // 4. Check if foreground window covers its monitor
        let h_monitor = MonitorFromWindow(fg_hwnd, MONITOR_DEFAULTTONEAREST);
        if h_monitor == 0 {
            return false;
        }

        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(h_monitor, &mut mi) == 0 {
            return false;
        }

        let mut win_rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(fg_hwnd, &mut win_rect) == 0 {
            return false;
        }

        // Verify window covers or exceeds rcMonitor
        win_rect.left <= mi.rcMonitor.left
            && win_rect.top <= mi.rcMonitor.top
            && win_rect.right >= mi.rcMonitor.right
            && win_rect.bottom >= mi.rcMonitor.bottom
    }
}

#[cfg(not(windows))]
pub fn is_foreground_fullscreen() -> bool {
    false
}

#[cfg(windows)]
pub fn get_cursor_pos() -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        if GetCursorPos(&mut pt) != 0 {
            Some((pt.x, pt.y))
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
pub fn get_cursor_pos() -> Option<(i32, i32)> {
    None
}

#[cfg(windows)]
pub fn get_pet_window_rect() -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let hwnd = get_pet_hwnd();
    if hwnd != 0 {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe {
            if GetWindowRect(hwnd, &mut rect) != 0 {
                return Some((
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                ));
            }
        }
    }
    None
}

#[cfg(not(windows))]
pub fn get_pet_window_rect() -> Option<(i32, i32, i32, i32)> {
    None
}

#[cfg(windows)]
pub fn set_pet_window_alpha(alpha: f32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongW,
        WS_EX_LAYERED,
    };
    let hwnd = get_pet_hwnd();
    if hwnd != 0 {
        unsafe {
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            if (ex_style & WS_EX_LAYERED as i32) == 0 {
                SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED as i32);
            }
            let byte_val = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            SetLayeredWindowAttributes(hwnd, 0, byte_val, LWA_ALPHA);
        }
    }
}

#[cfg(not(windows))]
pub fn set_pet_window_alpha(_alpha: f32) {}

#[cfg(windows)]
pub fn set_subtitle_window_alpha(alpha: f32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        LWA_ALPHA, SetLayeredWindowAttributes,
    };
    let hwnd = get_subtitle_hwnd();
    if hwnd != 0 {
        unsafe {
            let byte_val = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            SetLayeredWindowAttributes(hwnd, 0, byte_val, LWA_ALPHA);
        }
    }
}

#[cfg(not(windows))]
pub fn set_subtitle_window_alpha(_alpha: f32) {}

#[derive(Debug)]
pub struct FullscreenFadeController {
    raw_fullscreen: bool,
    debounced_fullscreen: bool,
    state_change_time: Instant,
    current_alpha: f32,
    target_alpha: f32,
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
            was_faded_out: false,
        }
    }

    pub fn update(
        &mut self,
        is_fullscreen_raw: bool,
        auto_hide: bool,
        delta: f32,
        user_mouse_passthrough: bool,
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
                set_mouse_passthrough(user_mouse_passthrough);
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

/// Trims the current process working set, actively stripping unreferenced pages back to Windows memory pool.
#[cfg(windows)]
pub fn trim_working_set() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    unsafe {
        // (usize::MAX, usize::MAX) instructs the Windows memory manager to strip unused pages from the working set.
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

#[cfg(not(windows))]
pub fn trim_working_set() {}

/// Returns (working_set_bytes, private_bytes) for the current process.
#[cfg(windows)]
pub fn get_process_memory_info() -> (usize, usize) {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut pmc = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
            (pmc.WorkingSetSize, pmc.PagefileUsage)
        } else {
            (0, 0)
        }
    }
}

#[cfg(not(windows))]
pub fn get_process_memory_info() -> (usize, usize) {
    (0, 0)
}


