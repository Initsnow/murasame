use std::time::Instant;

use crate::config::GazeConfig;
use crate::cubism::CubismModel;

#[derive(Debug, Clone)]
pub struct GazeController {
    pub config: GazeConfig,
    /// Current smoothed horizontal gaze in [-1.0, 1.0]
    pub current_x: f32,
    /// Current smoothed vertical gaze in [-1.0, 1.0] (downward positive in screen space)
    pub current_y: f32,
    /// Target horizontal gaze in [-1.0, 1.0]
    pub target_x: f32,
    /// Target vertical gaze in [-1.0, 1.0]
    pub target_y: f32,

    /// Mouse position tracking for idle timeout return
    pub last_cursor: Option<(i32, i32)>,
    pub last_cursor_move_time: Instant,
    pub is_mouse_idle: bool,

    // Cached Live2D parameter indices
    pub param_eyeball_x: Option<usize>,
    pub param_eyeball_y: Option<usize>,
    pub param_angle_x: Option<usize>,
    pub param_angle_y: Option<usize>,
    pub param_angle_z: Option<usize>,
    pub param_body_angle_x: Option<usize>,
}

impl GazeController {
    pub fn new(config: GazeConfig, model: &CubismModel) -> Self {
        Self {
            config,
            current_x: 0.0,
            current_y: 0.0,
            target_x: 0.0,
            target_y: 0.0,
            last_cursor: None,
            last_cursor_move_time: Instant::now(),
            is_mouse_idle: false,
            param_eyeball_x: model.parameter_index("ParamEyeBallX"),
            param_eyeball_y: model.parameter_index("ParamEyeBallY"),
            param_angle_x: model.parameter_index("ParamAngleX"),
            param_angle_y: model.parameter_index("ParamAngleY"),
            param_angle_z: model.parameter_index("ParamAngleZ"),
            param_body_angle_x: model.parameter_index("ParamBodyAngleX"),
        }
    }

    /// Update target gaze based on cursor screen coordinates and window bounds.
    pub fn update_target(
        &mut self,
        cursor: Option<(i32, i32)>,
        win_rect: Option<(i32, i32, i32, i32)>,
        is_fullscreen: bool,
        is_playing_motion: bool,
    ) {
        if !self.config.enabled {
            self.target_x = 0.0;
            self.target_y = 0.0;
            return;
        }

        // Fullscreen avoidance pause
        if self.config.pause_in_fullscreen && is_fullscreen {
            self.target_x = 0.0;
            self.target_y = 0.0;
            return;
        }

        // Pause gaze if motion is playing and blend_with_motions is disabled
        if !self.config.blend_with_motions && is_playing_motion {
            self.target_x = 0.0;
            self.target_y = 0.0;
            return;
        }

        let now = Instant::now();
        if let Some(pos) = cursor {
            if let Some(last_pos) = self.last_cursor {
                if (pos.0 - last_pos.0).abs() > 1 || (pos.1 - last_pos.1).abs() > 1 {
                    self.last_cursor = Some(pos);
                    self.last_cursor_move_time = now;
                    self.is_mouse_idle = false;
                } else if now.duration_since(self.last_cursor_move_time).as_secs_f32()
                    >= self.config.idle_timeout
                {
                    self.is_mouse_idle = true;
                }
            } else {
                self.last_cursor = Some(pos);
                self.last_cursor_move_time = now;
                self.is_mouse_idle = false;
            }
        } else {
            self.is_mouse_idle = true;
        }

        if self.is_mouse_idle {
            self.target_x = 0.0;
            self.target_y = 0.0;
        } else if let (Some(cur), Some(win)) = (cursor, win_rect) {
            // Anchor point: window horizontal center, eye height at 35% from top
            let eye_x = win.0 as f32 + win.2 as f32 * 0.5;
            let eye_y = win.1 as f32 + win.3 as f32 * 0.35;
            let dx = cur.0 as f32 - eye_x;
            let dy = cur.1 as f32 - eye_y;

            // Hyperbolic tangent S-curve for natural across-screen bounding
            let radius_x = 450.0;
            let radius_y = 350.0;
            self.target_x = (dx / radius_x).tanh().clamp(-1.0, 1.0);
            self.target_y = (dy / radius_y).tanh().clamp(-1.0, 1.0);
        } else {
            self.target_x = 0.0;
            self.target_y = 0.0;
        }
    }

    /// Advance smooth exponential damping filter by delta seconds.
    pub fn tick(&mut self, delta: f32) {
        let delta = delta.clamp(0.0, 0.1);
        // Exponential damping: lambda = 6.0 settles ~97.3% in 0.6 seconds
        let lerp_factor = 1.0 - (-delta * 6.0).exp();
        self.current_x += (self.target_x - self.current_x) * lerp_factor;
        self.current_y += (self.target_y - self.current_y) * lerp_factor;

        // Snap near zero to eliminate micro-jitter when settled
        if self.target_x == 0.0 && self.current_x.abs() < 0.001 {
            self.current_x = 0.0;
        }
        if self.target_y == 0.0 && self.current_y.abs() < 0.001 {
            self.current_y = 0.0;
        }
    }

    /// Apply gaze parameter offsets to the Live2D model.
    pub fn apply(&self, model: &mut CubismModel) {
        if self.current_x.abs() < 1e-4 && self.current_y.abs() < 1e-4 {
            return;
        }

        let tx = self.current_x;
        let ty = self.current_y;

        // 1. ParamEyeBallX: [-1.0, 1.0]
        if let Some(idx) = self.param_eyeball_x {
            let min = model.parameter_minimum_values()[idx];
            let max = model.parameter_maximum_values()[idx];
            let val = model.parameter_values()[idx] + tx * 1.0;
            model.parameter_values_mut()[idx] = val.clamp(min, max);
        }

        // 2. ParamEyeBallY: [-1.0, 1.0], Live2D UP is positive
        if let Some(idx) = self.param_eyeball_y {
            let min = model.parameter_minimum_values()[idx];
            let max = model.parameter_maximum_values()[idx];
            let val = model.parameter_values()[idx] + (-ty * 1.0);
            model.parameter_values_mut()[idx] = val.clamp(min, max);
        }

        // 3. ParamAngleX: Yaw [-30.0, 30.0], target_x * 15.0
        if let Some(idx) = self.param_angle_x {
            let min = model.parameter_minimum_values()[idx];
            let max = model.parameter_maximum_values()[idx];
            let val = model.parameter_values()[idx] + tx * 15.0;
            model.parameter_values_mut()[idx] = val.clamp(min, max);
        }

        // 4. ParamAngleY: Pitch [-30.0, 30.0], -target_y * 12.0
        if let Some(idx) = self.param_angle_y {
            let min = model.parameter_minimum_values()[idx];
            let max = model.parameter_maximum_values()[idx];
            let val = model.parameter_values()[idx] + (-ty * 12.0);
            model.parameter_values_mut()[idx] = val.clamp(min, max);
        }

        // 5. ParamAngleZ: Roll [-30.0, 30.0], -target_x * 4.0
        if let Some(idx) = self.param_angle_z {
            let min = model.parameter_minimum_values()[idx];
            let max = model.parameter_maximum_values()[idx];
            let val = model.parameter_values()[idx] + (-tx * 4.0);
            model.parameter_values_mut()[idx] = val.clamp(min, max);
        }

        // 6. ParamBodyAngleX: Body [-10.0, 10.0], target_x * 3.0
        if let Some(idx) = self.param_body_angle_x {
            let min = model.parameter_minimum_values()[idx];
            let max = model.parameter_maximum_values()[idx];
            let val = model.parameter_values()[idx] + tx * 3.0;
            model.parameter_values_mut()[idx] = val.clamp(min, max);
        }
    }

    /// Returns true if gaze is actively tracking, moving, or transitioning.
    pub fn is_active(&self) -> bool {
        (!self.is_mouse_idle && self.last_cursor_move_time.elapsed().as_millis() < 800)
            || (self.current_x - self.target_x).abs() > 0.002
            || (self.current_y - self.target_y).abs() > 0.002
    }

    pub fn set_config(&mut self, config: GazeConfig) {
        self.config = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gaze_target_calculation_and_bounding() {
        let mut controller = GazeController {
            config: GazeConfig::default(),
            current_x: 0.0,
            current_y: 0.0,
            target_x: 0.0,
            target_y: 0.0,
            last_cursor: None,
            last_cursor_move_time: Instant::now(),
            is_mouse_idle: false,
            param_eyeball_x: None,
            param_eyeball_y: None,
            param_angle_x: None,
            param_angle_y: None,
            param_angle_z: None,
            param_body_angle_x: None,
        };

        let win = Some((100, 100, 400, 600));
        // Eye position: x = 100 + 200 = 300, y = 100 + 210 = 310

        // Cursor exactly at eye position -> target (0.0, 0.0)
        controller.update_target(Some((300, 310)), win, false, false);
        assert!((controller.target_x).abs() < 1e-4);
        assert!((controller.target_y).abs() < 1e-4);

        // Cursor to the right and below
        controller.update_target(Some((750, 660)), win, false, false);
        assert!(controller.target_x > 0.5);
        assert!(controller.target_y > 0.5);

        // Extreme distance (e.g. secondary monitor 5000px away) must be safely bounded in [-1.0, 1.0]
        controller.update_target(Some((5000, 5000)), win, false, false);
        assert!(controller.target_x <= 1.0 && controller.target_x > 0.99);
        assert!(controller.target_y <= 1.0 && controller.target_y > 0.99);
    }

    #[test]
    fn test_gaze_exponential_damping_convergence() {
        let mut controller = GazeController {
            config: GazeConfig::default(),
            current_x: 0.0,
            current_y: 0.0,
            target_x: 1.0,
            target_y: 0.5,
            last_cursor: None,
            last_cursor_move_time: Instant::now(),
            is_mouse_idle: false,
            param_eyeball_x: None,
            param_eyeball_y: None,
            param_angle_x: None,
            param_angle_y: None,
            param_angle_z: None,
            param_body_angle_x: None,
        };

        // Advance 0.1s
        controller.tick(0.1);
        assert!(controller.current_x > 0.3 && controller.current_x < 0.6);

        // Advance a total of 0.7s -> should settle to within 98% of target
        for _ in 0..6 {
            controller.tick(0.1);
        }
        assert!((controller.current_x - 1.0).abs() < 0.03);
        assert!((controller.current_y - 0.5).abs() < 0.02);
    }

    #[test]
    fn test_gaze_fullscreen_and_motion_blend_pause() {
        let mut controller = GazeController {
            config: GazeConfig {
                enabled: true,
                idle_timeout: 2.5,
                blend_with_motions: false,
                pause_in_fullscreen: true,
            },
            current_x: 0.5,
            current_y: 0.5,
            target_x: 0.5,
            target_y: 0.5,
            last_cursor: Some((800, 800)),
            last_cursor_move_time: Instant::now(),
            is_mouse_idle: false,
            param_eyeball_x: None,
            param_eyeball_y: None,
            param_angle_x: None,
            param_angle_y: None,
            param_angle_z: None,
            param_body_angle_x: None,
        };

        let win = Some((100, 100, 400, 600));

        // When fullscreen is active and pause_in_fullscreen is true -> targets 0
        controller.update_target(Some((800, 800)), win, true, false);
        assert_eq!(controller.target_x, 0.0);
        assert_eq!(controller.target_y, 0.0);

        // When motion is playing and blend_with_motions is false -> targets 0
        controller.update_target(Some((800, 800)), win, false, true);
        assert_eq!(controller.target_x, 0.0);
        assert_eq!(controller.target_y, 0.0);
    }

    #[test]
    fn test_gaze_is_active_lifecycle() {
        let mut controller = GazeController {
            config: GazeConfig::default(),
            current_x: 0.0,
            current_y: 0.0,
            target_x: 0.8,
            target_y: 0.4,
            last_cursor: Some((800, 800)),
            last_cursor_move_time: Instant::now(),
            is_mouse_idle: false,
            param_eyeball_x: None,
            param_eyeball_y: None,
            param_angle_x: None,
            param_angle_y: None,
            param_angle_z: None,
            param_body_angle_x: None,
        };

        // 1. When mouse recently moved (<800ms), is_active is true
        assert!(controller.is_active());

        // 2. When mouse hasn't moved for 1s, but eyes haven't reached target yet, is_active is true
        controller.last_cursor_move_time = Instant::now() - std::time::Duration::from_millis(1000);
        assert!(controller.is_active());

        // 3. When mouse hasn't moved for 1s and eyes have settled at target, is_active is false
        controller.current_x = 0.8;
        controller.current_y = 0.4;
        assert!(!controller.is_active());

        // 4. When idle timeout triggers and targets reset to 0, eyes need to return, so is_active is true again
        controller.target_x = 0.0;
        controller.target_y = 0.0;
        assert!(controller.is_active());

        // 5. When eyes finish returning to 0.0, is_active is false
        controller.current_x = 0.0;
        controller.current_y = 0.0;
        assert!(!controller.is_active());
    }
}
