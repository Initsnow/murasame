pub mod common;
pub use common::*;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
pub mod linux;
#[cfg(not(windows))]
pub use linux::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn test_linux_platform_cursor_and_window_rect() {
        update_pet_window_size(400, 700);
        assert_eq!(get_pet_window_rect(), Some((0, 0, 400, 700)));

        update_mouse_position(Some((150, 250)));
        assert_eq!(get_cursor_pos(), Some((150, 250)));

        update_mouse_position(None);
        assert_eq!(get_cursor_pos(), None);
    }
}
