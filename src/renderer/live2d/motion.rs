use mocari::{
    MotionPlayer,
    json::{apply_motion_fade, motion_fade_in_weight, motion_fade_out_weight},
};
use crate::cubism::CubismModel;

/// Applies looping idle motion with smooth crossfade back to the starting frame (time = 0.0),
/// eliminating any twitching/glitching at loop wraparounds.
pub fn apply_looping_idle_motion(
    idle: &mut MotionPlayer,
    model: &mut CubismModel,
    delta: f32,
) {
    idle.tick(delta);
    let time = idle.time();
    let motion = idle.motion();
    let duration = motion.meta().duration();
    if duration <= 0.0 {
        return;
    }

    let fade_duration = (duration * 0.1).clamp(0.2, 0.5);
    let is_in_fade = time >= duration - fade_duration;
    let fade_weight = if is_in_fade {
        let t = ((time - (duration - fade_duration)) / fade_duration).clamp(0.0, 1.0);
        // Smoothstep: 3*t^2 - 2*t^3
        t * t * (3.0 - 2.0 * t)
    } else {
        0.0
    };

    for curve in motion.curves() {
        let Some(mut sampled) = curve.sample(time) else {
            continue;
        };

        if is_in_fade {
            if let Some(start_val) = curve.sample(0.0) {
                sampled = sampled * (1.0 - fade_weight) + start_val * fade_weight;
            }
        }

        match curve.target() {
            "Parameter" => {
                let Some(index) = model.parameter_index(curve.id()) else {
                    continue;
                };
                let min = model.parameter_minimum_values()[index];
                let max = model.parameter_maximum_values()[index];
                model.parameter_values_mut()[index] = sampled.clamp(min, max);
            }
            "PartOpacity" => {
                let Some(index) = model.part_index(curve.id()) else {
                    continue;
                };
                model.part_opacities_mut()[index] = sampled.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }
}

pub fn apply_motion_to_cubism(
    player: &MotionPlayer,
    model: &mut CubismModel,
    default_fade_in: f32,
    default_fade_out: f32,
) {
    let motion = player.motion();
    let duration = motion.meta().duration();
    let end_time = if player.is_looping() { -1.0 } else { duration };
    let time = player.time();
    let motion_fade_in = motion_fade_in_weight(time, 0.0, default_fade_in);
    let motion_fade_out = motion_fade_out_weight(time, end_time, default_fade_out);

    for curve in motion.curves() {
        let Some(sampled) = curve.sample(time) else {
            continue;
        };

        let curve_fade_in = curve.fade_in_time().filter(|s| *s >= 0.0);
        let curve_fade_out = curve.fade_out_time().filter(|s| *s >= 0.0);

        let fin = match curve_fade_in {
            Some(0.0) => 1.0,
            Some(s) => motion_fade_in_weight(time, 0.0, s),
            None => motion_fade_in,
        };
        let fout = match curve_fade_out {
            Some(0.0) => 1.0,
            Some(_) if end_time < 0.0 => 1.0,
            Some(s) => motion_fade_out_weight(time, end_time, s),
            None => motion_fade_out,
        };

        let curve_weight = (player.weight() * fin * fout).clamp(0.0, 1.0);
        if curve_weight <= 0.0 {
            continue;
        }

        match curve.target() {
            "Parameter" => {
                let Some(index) = model.parameter_index(curve.id()) else {
                    continue;
                };
                let min = model.parameter_minimum_values()[index];
                let max = model.parameter_maximum_values()[index];
                let current = model.parameter_values()[index];
                let value = apply_motion_fade(current, sampled, curve_weight);
                model.parameter_values_mut()[index] = value.clamp(min, max);
            }
            "PartOpacity" => {
                let Some(index) = model.part_index(curve.id()) else {
                    continue;
                };
                let current = model.part_opacities()[index];
                let value = apply_motion_fade(current, sampled, curve_weight);
                model.part_opacities_mut()[index] = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }
}

