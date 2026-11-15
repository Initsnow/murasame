use std::collections::HashMap;
use std::fs;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use evdev::{AbsoluteAxisCode, Device, EventType, KeyCode, RelativeAxisCode};

/// State of the virtual mouse tracking relative to the pet window center.
#[derive(Debug, Clone, Copy)]
pub struct VirtualGazeState {
    /// Relative offset from pet window center (X)
    pub rel_x: f32,
    /// Relative offset from pet window center (Y)
    pub rel_y: f32,
    /// Whether the cursor is currently hovering directly inside the pet window
    pub is_hovering: bool,
    /// Whether the window itself is currently being dragged across the screen
    pub is_dragging_window: bool,
    /// Last time mouse motion was recorded or hovered
    pub last_activity: Instant,
}

impl Default for VirtualGazeState {
    fn default() -> Self {
        Self {
            rel_x: 0.0,
            rel_y: 0.0,
            is_hovering: false,
            is_dragging_window: false,
            last_activity: Instant::now(),
        }
    }
}

impl VirtualGazeState {
    /// Update tracking when the mouse enters or moves inside the pet window.
    /// Synchronizes the accumulator directly to true local coordinates (Anti-Drift Calibration).
    pub fn on_hover(&mut self, local_pos: (i32, i32), win_size: (i32, i32)) {
        self.is_hovering = true;
        let center_x = win_size.0 as f32 / 2.0;
        let center_y = win_size.1 as f32 / 2.0;
        self.rel_x = local_pos.0 as f32 - center_x;
        self.rel_y = local_pos.1 as f32 - center_y;
        self.last_activity = Instant::now();
    }

    /// Mark hover ended. The virtual cursor will continue smoothly from the exit position.
    pub fn on_leave(&mut self) {
        if self.is_hovering {
            self.is_hovering = false;
            self.last_activity = Instant::now();
        }
    }

    /// Mark window dragging started. Pauses relative accumulation to prevent window-drag drift.
    pub fn on_drag_start(&mut self) {
        self.is_dragging_window = true;
        self.is_hovering = false;
        self.rel_x = 0.0;
        self.rel_y = 0.0;
        self.last_activity = Instant::now();
    }

    /// Mark window dragging ended. Resets base position to window center so tracking resumes cleanly.
    pub fn on_drag_end(&mut self) {
        if self.is_dragging_window {
            self.is_dragging_window = false;
            self.is_hovering = false;
            self.rel_x = 0.0;
            self.rel_y = 0.0;
            self.last_activity = Instant::now();
        }
    }

    /// Accumulate relative motion (dx, dy) from evdev with anti-windup bounds clamping.
    pub fn on_relative_motion(&mut self, dx: f32, dy: f32, win_size: (i32, i32)) {
        if self.is_dragging_window {
            // While dragging the pet window across the screen, skip accumulating motion deltas
            // so the drag distance doesn't falsely pin the gaze to the corner.
            return;
        }

        // Live2D gaze range is span = win_size * 1.5.
        // We clamp accumulator to span * 1.5 (i.e. win_size * 2.25) to avoid windup lag when turning back.
        let max_span_x = (win_size.0 as f32).max(100.0) * 2.25;
        let max_span_y = (win_size.1 as f32).max(100.0) * 2.25;

        self.rel_x = (self.rel_x + dx).clamp(-max_span_x, max_span_x);
        self.rel_y = (self.rel_y + dy).clamp(-max_span_y, max_span_y);
        self.last_activity = Instant::now();
    }

    /// Smoothly decay accumulator towards center if mouse has been idle for > 2 seconds
    /// outside the window, preventing permanent offset when moved by window manager.
    pub fn decay_if_idle(&mut self) {
        if self.is_hovering || self.is_dragging_window {
            return;
        }
        if self.last_activity.elapsed() >= Duration::from_millis(2000) {
            self.rel_x *= 0.92;
            self.rel_y *= 0.92;
            if self.rel_x.abs() < 1.0 {
                self.rel_x = 0.0;
            }
            if self.rel_y.abs() < 1.0 {
                self.rel_y = 0.0;
            }
        }
    }

    /// Compute current virtual screen coordinates relative to the pet window (0, 0, width, height).
    pub fn to_cursor_pos(&mut self, win_size: (i32, i32)) -> (i32, i32) {
        self.decay_if_idle();
        let center_x = win_size.0 as f32 / 2.0;
        let center_y = win_size.1 as f32 / 2.0;
        let cur_x = (center_x + self.rel_x).round() as i32;
        let cur_y = (center_y + self.rel_y).round() as i32;
        (cur_x, cur_y)
    }
}

/// Global shared state for Linux evdev mouse tracking
static TRACKER_STATE: Mutex<Option<VirtualGazeState>> = Mutex::new(None);
static IS_EVDEV_ACTIVE: AtomicBool = AtomicBool::new(false);
static IS_DRAGGING_WINDOW: AtomicBool = AtomicBool::new(false);
static IS_LEFT_BUTTON_DOWN: AtomicBool = AtomicBool::new(false);
static TRACKER_INITIALIZED: OnceLock<()> = OnceLock::new();

/// Check whether the pet window is currently in an active dragging operation.
pub fn is_dragging_window() -> bool {
    IS_DRAGGING_WINDOW.load(Ordering::Acquire)
}

/// Check whether a pointer left button or touch contact is currently pressed down.
pub fn is_left_button_down() -> bool {
    IS_LEFT_BUTTON_DOWN.load(Ordering::Acquire)
}

/// Information maintained per registered input device
struct InputDeviceInfo {
    path: PathBuf,
    device: Device,
    is_touchpad: bool,
    last_abs_x: Option<i32>,
    last_abs_y: Option<i32>,
}

/// Check if a Linux evdev device is a mouse, trackball, or touchpad pointer device.
/// Returns `Some(is_touchpad)` if supported, `None` if not a pointer device.
pub fn classify_pointer_device(device: &Device) -> Option<bool> {
    // 1. Touchpads (emit BTN_TOUCH or BTN_TOOL_FINGER, and absolute X/Y axes)
    let has_touch_key = device.supported_keys().map_or(false, |keys| {
        keys.contains(KeyCode::BTN_TOUCH) || keys.contains(KeyCode::BTN_TOOL_FINGER)
    });
    let has_abs_axes = device.supported_absolute_axes().map_or(false, |axes| {
        axes.contains(AbsoluteAxisCode::ABS_X)
            || axes.contains(AbsoluteAxisCode::ABS_MT_POSITION_X)
    });
    if has_touch_key && has_abs_axes {
        return Some(true); // Touchpad device
    }

    // 2. Relative mice and trackballs (emit REL_X and REL_Y, and mouse buttons)
    let has_rel_axes = device.supported_relative_axes().map_or(false, |axes| {
        axes.contains(RelativeAxisCode::REL_X) && axes.contains(RelativeAxisCode::REL_Y)
    });
    let has_mouse_keys = device.supported_keys().map_or(false, |keys| {
        keys.contains(KeyCode::BTN_LEFT)
            || keys.contains(KeyCode::BTN_RIGHT)
            || keys.contains(KeyCode::BTN_MIDDLE)
    });
    if has_rel_axes && has_mouse_keys {
        return Some(false); // Mouse device
    }

    // Fallback: devices that have BTN_TOUCH alone
    if has_touch_key {
        return Some(true);
    }

    // Fallback: devices that have REL_X & REL_Y alone
    if has_rel_axes {
        return Some(false);
    }

    None
}

/// Helper for backwards compatibility with tests and callers
pub fn is_mouse_device(device: &Device) -> bool {
    classify_pointer_device(device).is_some()
}

/// Returns true if evdev global mouse listener is currently active and receiving events.
pub fn is_evdev_active() -> bool {
    IS_EVDEV_ACTIVE.load(Ordering::Relaxed)
}

/// Query current virtual cursor position from evdev tracker if active.
pub fn get_evdev_cursor_pos(win_size: (i32, i32)) -> Option<(i32, i32)> {
    if !is_evdev_active() {
        return None;
    }
    // Auto-recovery watchdog: if dragging flag is set but button is physically released, end drag immediately
    if is_dragging_window() && !IS_LEFT_BUTTON_DOWN.load(Ordering::Acquire) {
        notify_window_drag_ended();
    }
    if let Ok(mut guard) = TRACKER_STATE.lock() {
        guard.as_mut().map(|s| s.to_cursor_pos(win_size))
    } else {
        None
    }
}

/// Inform the tracker of window hover status from Slint.
pub fn notify_window_hover(hover: Option<(i32, i32)>, win_size: (i32, i32)) {
    if let Ok(mut guard) = TRACKER_STATE.lock() {
        let state = guard.get_or_insert_with(VirtualGazeState::default);
        if let Some(pos) = hover {
            state.on_hover(pos, win_size);
        } else {
            state.on_leave();
        }
    }
}

/// Notify that the user has started dragging the window across the screen.
/// Returns true if accepted, false if rejected as a phantom event.
pub fn notify_window_drag_started() -> bool {
    let button_down = IS_LEFT_BUTTON_DOWN.load(Ordering::Acquire);
    let evdev_active = is_evdev_active();
    if evdev_active && !button_down {
        tracing::debug!("Rejecting phantom window drag start: left button is not physically held");
        return false;
    }
    IS_DRAGGING_WINDOW.store(true, Ordering::Release);
    if let Ok(mut guard) = TRACKER_STATE.lock() {
        let state = guard.get_or_insert_with(VirtualGazeState::default);
        state.on_drag_start();
    }
    true
}

static RESET_TOUCH_FLAG: AtomicBool = AtomicBool::new(false);

/// Query and clear the touch reset request flag.
pub fn take_reset_touch_flag() -> bool {
    RESET_TOUCH_FLAG.swap(false, Ordering::AcqRel)
}

/// Request a clean reset of Slint's TouchArea to purge any stuck grabs.
pub fn trigger_touch_reset() {
    RESET_TOUCH_FLAG.store(true, Ordering::Release);
}

/// Notify that the window drag has ended.
pub fn notify_window_drag_ended() {
    let was_dragging = IS_DRAGGING_WINDOW.swap(false, Ordering::AcqRel);
    trigger_touch_reset();
    if !was_dragging {
        return;
    }
    if let Ok(mut guard) = TRACKER_STATE.lock() {
        let state = guard.get_or_insert_with(VirtualGazeState::default);
        state.on_drag_end();
    }
}

/// Record relative motion from evdev and update state.
pub fn record_relative_motion(dx: f32, dy: f32, win_size: (i32, i32)) {
    if let Ok(mut guard) = TRACKER_STATE.lock() {
        let state = guard.get_or_insert_with(VirtualGazeState::default);
        state.on_relative_motion(dx, dy, win_size);
    }
}

/// Initialize the Linux background mouse listener thread.
pub fn init_evdev_mouse_tracker<F>(get_win_size: F)
where
    F: Fn() -> (i32, i32) + Send + Sync + 'static,
{
    TRACKER_INITIALIZED.get_or_init(|| {
        if let Ok(mut guard) = TRACKER_STATE.lock() {
            *guard = Some(VirtualGazeState::default());
        }
        let win_size_fn = Arc::new(get_win_size);
        thread::Builder::new()
            .name("murasame-evdev-mouse".to_string())
            .spawn(move || {
                run_evdev_listener_loop(win_size_fn);
            })
            .expect("Failed to spawn murasame-evdev-mouse thread");
    });
}

/// Core event loop managing epoll, inotify, and evdev device streaming.
fn run_evdev_listener_loop(win_size_fn: Arc<dyn Fn() -> (i32, i32) + Send + Sync>) {
    let epoll_fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if epoll_fd < 0 {
        tracing::warn!("Failed to create epoll instance for evdev mouse tracking");
        return;
    }

    // Inotify watcher for /dev/input hotplugging
    let inotify_fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if inotify_fd >= 0 {
        let c_path = std::ffi::CString::new("/dev/input").unwrap();
        let inotify_watch_desc = unsafe {
            libc::inotify_add_watch(
                inotify_fd,
                c_path.as_ptr(),
                libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB,
            )
        };

        if inotify_watch_desc >= 0 {
            let mut ev = libc::epoll_event {
                events: libc::EPOLLIN as u32,
                u64: 0, // 0 indicates inotify
            };
            unsafe {
                libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, inotify_fd, &mut ev);
            }
        } else {
            tracing::debug!("Could not add inotify watch on /dev/input; hotplugging will be disabled");
        }
    }

    let mut devices: HashMap<i32, InputDeviceInfo> = HashMap::new();
    let mut next_token: u64 = 1;
    let mut token_to_fd: HashMap<u64, i32> = HashMap::new();
    let mut fd_to_token: HashMap<i32, u64> = HashMap::new();
    let mut had_permission_denied = false;

    // Initial scan of /dev/input/event*
    scan_and_add_devices(
        epoll_fd,
        &mut devices,
        &mut next_token,
        &mut token_to_fd,
        &mut fd_to_token,
        &mut had_permission_denied,
    );

    if !devices.is_empty() {
        IS_EVDEV_ACTIVE.store(true, Ordering::Relaxed);
        tracing::info!(
            "Wayland/Linux 全屏视线跟随已激活，成功监听 {} 个指针设备 (鼠标/触摸板)",
            devices.len()
        );
    } else if had_permission_denied {
        tracing::info!(
            "Wayland 全屏视线跟随：无 /dev/input 读取权限，已自动降级为窗口内追踪模式。\n\
             若需开启真正的全屏幕视线跟随，可配置 udev 规则赋予鼠标访问权限（无需加入 input 组）：\n\
             • NixOS:\n\
                 services.udev.extraRules = ''\n\
                   KERNEL==\"event*\", SUBSYSTEM==\"input\", ENV{{ID_INPUT_MOUSE}}==\"1\", MODE=\"0666\"\n\
                   KERNEL==\"event*\", SUBSYSTEM==\"input\", ENV{{ID_INPUT_TOUCHPAD}}==\"1\", MODE=\"0666\"\n\
                 '';\n\
             • 其他发行版 (/etc/udev/rules.d/99-murasame-input.rules):\n\
                 KERNEL==\"event*\", SUBSYSTEM==\"input\", ENV{{ID_INPUT_MOUSE}}==\"1\", MODE=\"0666\"\n\
                 KERNEL==\"event*\", SUBSYSTEM==\"input\", ENV{{ID_INPUT_TOUCHPAD}}==\"1\", MODE=\"0666\""
        );
    } else {
        tracing::debug!("No mouse devices discovered in /dev/input");
    }

    let mut events = [libc::epoll_event { events: 0, u64: 0 }; 32];
    let inotify_buf_size = std::mem::size_of::<libc::inotify_event>() + 256;
    let mut inotify_buf = vec![0u8; inotify_buf_size * 16];

    loop {
        let nfds = unsafe {
            libc::epoll_wait(
                epoll_fd,
                events.as_mut_ptr(),
                events.len() as i32,
                5000, // 5s timeout to re-check device health / hotplug
            )
        };

        if nfds < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            tracing::warn!("epoll_wait error in evdev tracker: {err}");
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        if nfds == 0 {
            continue;
        }

        for i in 0..nfds as usize {
            let token = events[i].u64;

            if token == 0 && inotify_fd >= 0 {
                // Inotify event: handle hotplugging
                let bytes_read = unsafe {
                    libc::read(
                        inotify_fd,
                        inotify_buf.as_mut_ptr() as *mut libc::c_void,
                        inotify_buf.len(),
                    )
                };
                if bytes_read > 0 {
                    scan_and_add_devices(
                        epoll_fd,
                        &mut devices,
                        &mut next_token,
                        &mut token_to_fd,
                        &mut fd_to_token,
                        &mut had_permission_denied,
                    );
                    IS_EVDEV_ACTIVE.store(!devices.is_empty(), Ordering::Relaxed);
                }
                continue;
            }

            // Pointer device event
            if let Some(&raw_fd) = token_to_fd.get(&token) {
                let mut is_disconnected = false;
                if let Some(dev_info) = devices.get_mut(&raw_fd) {
                    let mut sum_dx = 0.0f32;
                    let mut sum_dy = 0.0f32;

                    match dev_info.device.fetch_events() {
                        Ok(event_iter) => {
                            for ev in event_iter {
                                match ev.event_type() {
                                    EventType::RELATIVE => {
                                        if ev.code() == RelativeAxisCode::REL_X.0 {
                                            sum_dx += ev.value() as f32;
                                        } else if ev.code() == RelativeAxisCode::REL_Y.0 {
                                            sum_dy += ev.value() as f32;
                                        }
                                    }
                                    EventType::ABSOLUTE => {
                                        if dev_info.is_touchpad {
                                            let code = ev.code();
                                            let val = ev.value();
                                            if code == AbsoluteAxisCode::ABS_X.0
                                                || code == AbsoluteAxisCode::ABS_MT_POSITION_X.0
                                            {
                                                if let Some(last_x) = dev_info.last_abs_x {
                                                    let delta = (val - last_x) as f32;
                                                    // Scale raw touchpad counts (~31 units/mm) to screen pixels (~5 px/mm)
                                                    sum_dx += delta * 0.25;
                                                }
                                                dev_info.last_abs_x = Some(val);
                                            } else if code == AbsoluteAxisCode::ABS_Y.0
                                                || code == AbsoluteAxisCode::ABS_MT_POSITION_Y.0
                                            {
                                                if let Some(last_y) = dev_info.last_abs_y {
                                                    let delta = (val - last_y) as f32;
                                                    sum_dy += delta * 0.25;
                                                }
                                                dev_info.last_abs_y = Some(val);
                                            }
                                        }
                                    }
                                    EventType::KEY => {
                                        let code = ev.code();
                                        let val = ev.value();
                                        if code == KeyCode::BTN_TOUCH.0 {
                                            let is_down = val != 0;
                                            IS_LEFT_BUTTON_DOWN.store(is_down, Ordering::Release);
                                            if !is_down {
                                                // Finger lifted from touchpad
                                                dev_info.last_abs_x = None;
                                                dev_info.last_abs_y = None;
                                                if is_dragging_window() {
                                                    notify_window_drag_ended();
                                                }
                                            }
                                        } else if code == KeyCode::BTN_LEFT.0 {
                                            let is_down = val != 0;
                                            IS_LEFT_BUTTON_DOWN.store(is_down, Ordering::Release);
                                            if !is_down && is_dragging_window() {
                                                // Mouse left button released
                                                notify_window_drag_ended();
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Err(e) if e.raw_os_error() == Some(libc::ENODEV) => {
                            is_disconnected = true;
                        }
                        Err(_) => {}
                    }

                    if sum_dx != 0.0 || sum_dy != 0.0 {
                        let win_size = win_size_fn();
                        record_relative_motion(sum_dx, sum_dy, win_size);
                    }
                }

                if is_disconnected {
                    tracing::info!("Pointer device disconnected: fd={raw_fd}");
                    unsafe {
                        libc::epoll_ctl(
                            epoll_fd,
                            libc::EPOLL_CTL_DEL,
                            raw_fd,
                            std::ptr::null_mut(),
                        );
                    }
                    devices.remove(&raw_fd);
                    token_to_fd.remove(&token);
                    fd_to_token.remove(&raw_fd);
                    IS_EVDEV_ACTIVE.store(!devices.is_empty(), Ordering::Relaxed);
                }
            }
        }
    }
}

/// Scan `/dev/input/` and register any newly available mouse or touchpad devices into epoll.
fn scan_and_add_devices(
    epoll_fd: i32,
    devices: &mut HashMap<i32, InputDeviceInfo>,
    next_token: &mut u64,
    token_to_fd: &mut HashMap<u64, i32>,
    fd_to_token: &mut HashMap<i32, u64>,
    had_permission_denied: &mut bool,
) {
    let input_dir = Path::new("/dev/input");
    let entries = match fs::read_dir(input_dir) {
        Ok(e) => e,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                *had_permission_denied = true;
            }
            return;
        }
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        if !name_str.starts_with("event") {
            continue;
        }

        let path = entry.path();
        // Check if already registered
        if devices.values().any(|d| d.path == path) {
            continue;
        }

        match Device::open(&path) {
            Ok(device) => {
                if let Some(is_touchpad) = classify_pointer_device(&device) {
                    let fd = device.as_raw_fd();
                    unsafe {
                        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
                        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }

                    let token = *next_token;
                    *next_token += 1;

                    let mut epoll_ev = libc::epoll_event {
                        events: libc::EPOLLIN as u32,
                        u64: token,
                    };

                    let res = unsafe {
                        libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut epoll_ev)
                    };

                    if res == 0 {
                        let dev_name = device.name().unwrap_or("Unknown").to_string();
                        if let Ok(keys) = device.get_key_state() {
                            if keys.contains(KeyCode::BTN_LEFT) || keys.contains(KeyCode::BTN_TOUCH) {
                                IS_LEFT_BUTTON_DOWN.store(true, Ordering::Release);
                            }
                        }
                        token_to_fd.insert(token, fd);
                        fd_to_token.insert(fd, token);
                        devices.insert(
                            fd,
                            InputDeviceInfo {
                                path: path.clone(),
                                device,
                                is_touchpad,
                                last_abs_x: None,
                                last_abs_y: None,
                            },
                        );
                        tracing::info!(
                            "成功注册{}设备: {} ({:?}, fd={})",
                            if is_touchpad { "触摸板" } else { "鼠标" },
                            name_str,
                            dev_name,
                            fd
                        );
                    }
                }
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    *had_permission_denied = true;
                    tracing::warn!(
                        "无权读取输入设备 {}: Permission denied (需配置 udev 规则赋予访问权限)",
                        path.display()
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_gaze_anti_windup_clamping() {
        let mut state = VirtualGazeState::default();
        let win_size = (400, 700);
        let max_span_x = 400.0 * 2.25; // 900.0
        let max_span_y = 700.0 * 2.25; // 1575.0

        // Move 5000 pixels to the right
        state.on_relative_motion(5000.0, 0.0, win_size);
        assert_eq!(state.rel_x, max_span_x);

        // Immediately move 10 pixels to the left -> starts moving left immediately without lag
        state.on_relative_motion(-10.0, 0.0, win_size);
        assert_eq!(state.rel_x, max_span_x - 10.0);

        // Move 5000 pixels up (negative y)
        state.on_relative_motion(0.0, -5000.0, win_size);
        assert_eq!(state.rel_y, -max_span_y);

        state.on_relative_motion(0.0, 20.0, win_size);
        assert_eq!(state.rel_y, -max_span_y + 20.0);
    }

    #[test]
    fn test_virtual_gaze_on_hover_calibration() {
        let mut state = VirtualGazeState::default();
        let win_size = (380, 750);

        // Accumulate some offset
        state.on_relative_motion(400.0, 300.0, win_size);
        assert_eq!(state.rel_x, 400.0);
        assert_eq!(state.rel_y, 300.0);

        // Mouse enters window at (190, 375) - exact center of window
        state.on_hover((190, 375), win_size);
        assert!(state.is_hovering);
        assert_eq!(state.rel_x, 0.0);
        assert_eq!(state.rel_y, 0.0);
        assert_eq!(state.to_cursor_pos(win_size), (190, 375));

        // Subsequent hover in window updates exact local coordinates
        state.on_hover((240, 325), win_size);
        assert_eq!(state.rel_x, 50.0);
        assert_eq!(state.rel_y, -50.0);

        // Leave window
        state.on_leave();
        assert!(!state.is_hovering);

        // After leaving, relative motion accumulates seamlessly from exit point
        state.on_relative_motion(50.0, -50.0, win_size);
        assert_eq!(state.rel_x, 100.0);
        assert_eq!(state.rel_y, -100.0);
        assert_eq!(state.to_cursor_pos(win_size), (290, 275));
    }

    #[test]
    fn test_virtual_gaze_window_drag_behavior() {
        let mut state = VirtualGazeState::default();
        let win_size = (380, 750);

        // Start dragging the window
        state.on_drag_start();
        assert!(state.is_dragging_window);
        assert_eq!(state.rel_x, 0.0);
        assert_eq!(state.rel_y, 0.0);

        // While moving window, motion accumulation is skipped to prevent window-drag drift
        state.on_relative_motion(500.0, 300.0, win_size);
        assert_eq!(state.rel_x, 0.0);
        assert_eq!(state.rel_y, 0.0);

        // Drag ends at new screen location: coordinates stay at center
        state.on_drag_end();
        assert!(!state.is_dragging_window);
        assert_eq!(state.rel_x, 0.0);
        assert_eq!(state.rel_y, 0.0);

        // Now mouse movement tracks cleanly from the new window center
        state.on_relative_motion(120.0, -80.0, win_size);
        assert_eq!(state.rel_x, 120.0);
        assert_eq!(state.rel_y, -80.0);
    }

    #[test]
    fn test_drag_lifecycle_gate() {
        assert!(!is_dragging_window());

        // Random click release outside of drag: should no-op
        notify_window_drag_ended();
        assert!(!is_dragging_window());

        // Actual drag started when evdev inactive: accepted
        assert!(notify_window_drag_started());
        assert!(is_dragging_window());

        // Left button released: drag ends
        notify_window_drag_ended();
        assert!(!is_dragging_window());

        // Subsequent release: no-op again
        notify_window_drag_ended();
        assert!(!is_dragging_window());

        // Test phantom rejection when evdev is active:
        IS_EVDEV_ACTIVE.store(true, Ordering::SeqCst);
        IS_LEFT_BUTTON_DOWN.store(false, Ordering::SeqCst);
        assert!(!notify_window_drag_started());
        assert!(!is_dragging_window());

        // Button physically held down: drag accepted
        IS_LEFT_BUTTON_DOWN.store(true, Ordering::SeqCst);
        assert!(notify_window_drag_started());
        assert!(is_dragging_window());

        // Watchdog recovery: if button released without notify_window_drag_ended,
        // get_evdev_cursor_pos automatically ends the drag
        IS_LEFT_BUTTON_DOWN.store(false, Ordering::SeqCst);
        let _ = get_evdev_cursor_pos((380, 750));
        assert!(!is_dragging_window());

        IS_EVDEV_ACTIVE.store(false, Ordering::SeqCst);
    }

    #[test]
    fn test_idle_decay() {
        let mut state = VirtualGazeState::default();
        let win_size = (380, 750);

        state.on_relative_motion(200.0, 150.0, win_size);
        assert_eq!(state.rel_x, 200.0);
        assert_eq!(state.rel_y, 150.0);

        // Simulate idle for > 2 seconds
        state.last_activity = Instant::now() - Duration::from_secs(3);
        state.decay_if_idle();

        assert!(state.rel_x < 200.0);
        assert!(state.rel_y < 150.0);
    }
}
