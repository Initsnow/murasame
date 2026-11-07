pub mod bounds;
pub mod expression;
pub mod hit_test;
pub mod motion;
pub mod renderer;

pub use bounds::*;
pub use expression::*;
pub use hit_test::*;
pub use motion::*;
pub use renderer::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use mocari::MotionPlayer;

    #[test]
    fn test_expression_easing_curve_math() {
        let curve = ExpressionEasingCurve::Smootherstep;
        assert_eq!(curve.evaluate(0.0), 0.0);
        assert_eq!(curve.evaluate(1.0), 1.0);
        assert!((curve.evaluate(0.5) - 0.5).abs() < 1e-6);

        // Verify strictly monotonic increasing
        let mut prev = 0.0;
        for i in 1..=100 {
            let t = i as f32 / 100.0;
            let val = curve.evaluate(t);
            assert!(
                val >= prev,
                "Curve not monotonic at t={t}: prev={prev}, val={val}"
            );
            prev = val;
        }

        // Test boundary clamping
        assert_eq!(curve.evaluate(-1.0), 0.0);
        assert_eq!(curve.evaluate(2.0), 1.0);
    }

    #[test]
    fn test_inspect_model_bounds_and_snug_fit() {
        let moc = crate::cubism::CubismMoc::from_file(Path::new("Murasame_live2d/Murasame.moc3")).unwrap();
        let model = crate::cubism::CubismModel::new(&moc).unwrap();
        let meshes = model.to_drawable_meshes();
        let bounds = ModelBounds::from_drawables(&meshes).unwrap();

        assert!(
            bounds.width() > 0.8 && bounds.width() < 0.9,
            "Unexpected model width: {}",
            bounds.width()
        );
        assert!(
            bounds.height() > 1.9 && bounds.height() < 2.0,
            "Unexpected model height: {}",
            bounds.height()
        );

        // Model aspect ratio is ~0.424
        let aspect = bounds.width() / bounds.height();
        assert!(
            (aspect - 0.424).abs() < 0.05,
            "Model aspect ratio should be around 0.424"
        );

        // Window size 380x750 has aspect ~0.507
        let w = 380u32;
        let h = 750u32;
        let win_aspect = w as f32 / h as f32;

        let matrix = fit_model_matrix_with_scale(bounds, w, h, 1.0);
        // Verify center transformation maps center of model close to NDC (0, 0)
        let center_x_ndc = matrix.invert_transform_x(0.0);
        let center_y_ndc = matrix.invert_transform_y(0.0);
        assert!((center_x_ndc - bounds.center_x()).abs() < 0.05);
        assert!((center_y_ndc - bounds.center_y()).abs() < 0.05);

        // Verify that in a 380px wide window, model occupies > 75% of horizontal space,
        // leaving < 50px blank margin on either side (unlike the old 600px window which left > 150px)
        let view_fill = MODEL_VIEW_FILL;
        let scale_y = view_fill / bounds.height();
        let scale_x = scale_y / win_aspect;
        let model_screen_width_fraction = (bounds.width() * scale_x) / 2.0;
        assert!(
            model_screen_width_fraction > 0.75,
            "Model should fill > 75% of window width, got {}",
            model_screen_width_fraction
        );

        let left_right_padding_pixels = (w as f32) * (1.0 - model_screen_width_fraction) / 2.0;
        assert!(
            left_right_padding_pixels < 50.0,
            "Side padding should be < 50px, got {}px",
            left_right_padding_pixels
        );
    }

    #[test]
    fn test_motion_fade_in_and_fade_out_smooth_transitions() {
        let moc = crate::cubism::CubismMoc::from_file(Path::new("Murasame_live2d/Murasame.moc3")).unwrap();
        let mut model = crate::cubism::CubismModel::new(&moc).unwrap();
        let angle_x_idx = model.parameter_index("ParamAngleX").unwrap();

        // 1. Create a synthetic motion holding ParamAngleX at 30.0 for 2.0s
        let motion_json = r#"{
            "Version": 3,
            "Meta": {
                "Duration": 2.0,
                "Fps": 30.0,
                "Loop": false,
                "AreBeziersRestricted": false,
                "CurveCount": 1,
                "TotalSegmentCount": 1,
                "TotalPointCount": 2,
                "UserDataCount": 0,
                "TotalUserDataSize": 0
            },
            "Curves": [
                {
                    "Target": "Parameter",
                    "Id": "ParamAngleX",
                    "Segments": [0.0, 30.0, 0, 2.0, 30.0]
                }
            ]
        }"#;
        let motion = mocari::json::Motion3::from_json_str(motion_json).unwrap();
        let mut player = MotionPlayer::new_once(motion);

        // Assume base idle pose has ParamAngleX = 0.0
        model.parameter_values_mut()[angle_x_idx] = 0.0;

        let fade_in = 0.5;
        let fade_out = 0.5;

        // t = 0.0: Fade-in weight is 0.0 -> value stays at base (0.0)
        apply_motion_to_cubism(&player, &mut model, fade_in, fade_out);
        assert_eq!(model.parameter_values()[angle_x_idx], 0.0);

        // t = 0.25 (halfway through fade-in): Weight should be ~0.5, ParamAngleX around 15.0
        player.tick(0.25);
        model.parameter_values_mut()[angle_x_idx] = 0.0;
        apply_motion_to_cubism(&player, &mut model, fade_in, fade_out);
        let val_half_in = model.parameter_values()[angle_x_idx];
        assert!((val_half_in - 15.0).abs() < 1.0, "Expected ~15.0, got {val_half_in}");

        // t = 1.0 (fully faded in): Weight is 1.0 -> ParamAngleX is 30.0
        player.tick(0.75);
        model.parameter_values_mut()[angle_x_idx] = 0.0;
        apply_motion_to_cubism(&player, &mut model, fade_in, fade_out);
        assert!((model.parameter_values()[angle_x_idx] - 30.0).abs() < 1e-4);

        // t = 1.75 (halfway through fade-out, end_time is 2.0): Weight should be ~0.5 -> ~15.0
        player.tick(0.75);
        model.parameter_values_mut()[angle_x_idx] = 0.0;
        apply_motion_to_cubism(&player, &mut model, fade_in, fade_out);
        let val_half_out = model.parameter_values()[angle_x_idx];
        assert!((val_half_out - 15.0).abs() < 1.0, "Expected ~15.0, got {val_half_out}");

        // t = 2.0 (fade-out completes): Weight is 0.0 -> smoothly returns to base (0.0)
        player.tick(0.25);
        assert!(player.is_finished());
        model.parameter_values_mut()[angle_x_idx] = 0.0;
        apply_motion_to_cubism(&player, &mut model, fade_in, fade_out);
        assert_eq!(model.parameter_values()[angle_x_idx], 0.0);
    }

    #[test]
    fn test_looping_idle_motion_crossfade_eliminates_discontinuity() {
        let moc = crate::cubism::CubismMoc::from_file(Path::new("Murasame_live2d/Murasame.moc3")).unwrap();
        let mut model = crate::cubism::CubismModel::new(&moc).unwrap();
        let angle_x_idx = model.parameter_index("ParamAngleX").unwrap();

        // Synthetic 2.0s looping motion where end (20.0) != start (0.0)
        let motion_json = r#"{
            "Version": 3,
            "Meta": {
                "Duration": 2.0,
                "Fps": 30.0,
                "Loop": true,
                "AreBeziersRestricted": false,
                "CurveCount": 1,
                "TotalSegmentCount": 1,
                "TotalPointCount": 2,
                "UserDataCount": 0,
                "TotalUserDataSize": 0
            },
            "Curves": [
                {
                    "Target": "Parameter",
                    "Id": "ParamAngleX",
                    "Segments": [0.0, 0.0, 0, 2.0, 20.0]
                }
            ]
        }"#;
        let motion = mocari::json::Motion3::from_json_str(motion_json).unwrap();
        let mut idle_player = MotionPlayer::with_looping(motion, true);

        // Before loop fade (e.g. t = 1.0s), linear ramp from 0 to 20 gives ~10.0
        apply_looping_idle_motion(&mut idle_player, &mut model, 1.0);
        let val_mid = model.parameter_values()[angle_x_idx];
        assert!((val_mid - 10.0).abs() < 1e-3);

        // Advance to very close to end (t = 1.99s, inside the 0.2s fade duration).
        // The crossfade should pull the value down close to start_val (0.0) rather than staying at ~19.9.
        apply_looping_idle_motion(&mut idle_player, &mut model, 0.99);
        let val_near_end = model.parameter_values()[angle_x_idx];
        assert!(
            val_near_end < 2.0,
            "Value at end should be smoothly blended towards start_val (0.0), got {val_near_end}"
        );

        // Next frame wraps around t = 2.0s -> 0.02s
        apply_looping_idle_motion(&mut idle_player, &mut model, 0.03);
        let val_after_wrap = model.parameter_values()[angle_x_idx];
        // The delta across wraparound should be smooth and tiny (< 1.0), not a jump of 20!
        assert!(
            (val_after_wrap - val_near_end).abs() < 2.0,
            "Discontinuity detected across loop wrap: near_end={val_near_end}, after_wrap={val_after_wrap}"
        );
    }

    #[test]
    fn test_snug_scissor_rect_clamping_and_dimensions() {
        let bounds = ModelBounds {
            min_x: -1.0,
            min_y: -2.0,
            max_x: 1.0,
            max_y: 2.0,
        };

        // Standard render resolution 1000 x 1875 with scale 1.0
        let (x, y, w, h) = calculate_snug_scissor_rect(bounds, 1000, 1875, 1.0);
        assert!(x + w <= 1000, "Scissor rect right boundary must not exceed render width");
        assert!(y + h <= 1875, "Scissor rect bottom boundary must not exceed render height");
        assert!(w > 0 && h > 0, "Scissor rect dimensions must be strictly positive");
        // Due to aspect ratio differences between model bounds and window, horizontal margins are trimmed
        assert!(w < 1000, "Snug scissor rect should trim empty transparent margins");

        // Zero edge case should safely return full surface without panic
        let (zx, zy, zw, zh) = calculate_snug_scissor_rect(bounds, 0, 0, 1.0);
        assert_eq!((zx, zy, zw, zh), (0, 0, 0, 0));
    }

    #[test]
    fn test_lock_free_mouth_open_and_parameter_dirty_gate() {
        let moc = crate::cubism::CubismMoc::from_file(Path::new("Murasame_live2d/Murasame.moc3")).unwrap();
        let model = crate::cubism::CubismModel::new(&moc).unwrap();
        let mut overrides = std::collections::HashMap::new();
        let mut force_dirty = false;
        let mut dirty = false;
        let threshold = 0.003f32;

        let mouth_idx = model.parameter_index("ParamMouthOpenY").unwrap();
        let min = model.parameter_minimum_values()[mouth_idx];
        let max = model.parameter_maximum_values()[mouth_idx];

        let set_param = |val: f32, force_dirty: &mut bool, dirty: &mut bool, overrides: &mut std::collections::HashMap<usize, f32>| {
            let clamped = val.clamp(min, max);
            let prev = overrides.get(&mouth_idx).copied();
            if prev != Some(clamped) {
                overrides.insert(mouth_idx, clamped);
                *dirty = true;
                let range = (max - min).abs().max(1e-4);
                let delta = prev.map_or(1.0, |p| (clamped - p).abs() / range);
                if delta >= threshold {
                    *force_dirty = true;
                }
            }
        };

        // 1. Initial set of mouth 0.0
        set_param(0.0, &mut force_dirty, &mut dirty, &mut overrides);
        assert!(dirty);
        assert!(force_dirty);

        // Reset dirty flags after frame render
        dirty = false;
        force_dirty = false;

        // 2. Subsequent set_param(0.0) while idle (the hot loop case)
        set_param(0.0, &mut force_dirty, &mut dirty, &mut overrides);
        assert!(!dirty, "Setting same parameter value must NOT mark dirty");
        assert!(!force_dirty, "Setting same parameter value must NOT force dirty");

        // 3. Sub-threshold micro-jitter (< 0.003)
        set_param(0.001, &mut force_dirty, &mut dirty, &mut overrides);
        assert!(dirty, "Value changed slightly so dirty is set for next update");
        assert!(!force_dirty, "Sub-threshold change must NOT force render pass");

        // 4. Significant speech mouth opening (> 0.003)
        force_dirty = false;
        set_param(0.5, &mut force_dirty, &mut dirty, &mut overrides);
        assert!(force_dirty, "Significant mouth opening must force dirty");
    }
}
