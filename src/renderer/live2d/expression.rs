use mocari::json::{Expression3, ExpressionBlend, ExpressionParameter};
use crate::cubism::CubismModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExpressionEasingCurve {
    #[default]
    Smootherstep,
    EaseInOutCubic,
    EaseInOutSine,
}

impl ExpressionEasingCurve {
    pub fn evaluate(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Smootherstep => t * t * t * (t * (t * 6.0 - 15.0) + 10.0),
            Self::EaseInOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Self::EaseInOutSine => 0.5 - 0.5 * (t * std::f32::consts::PI).cos(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NeutralTransition {
    pub outgoing: Option<Expression3>,
    pub neutral: Expression3,
    pub elapsed: f32,
    pub duration: f32,
}


pub fn blend_and_apply_expressions(
    outgoing: Option<&Expression3>,
    target: &Expression3,
    model: &mut CubismModel,
    alpha: f32,
) {
    let mut param_ids: [&str; 64] = [""; 64];
    let mut count = 0;
    let mut fallback_vec: Option<Vec<&str>> = None;

    macro_rules! add_id {
        ($id:expr) => {
            let id: &str = $id;
            if let Some(vec) = fallback_vec.as_mut() {
                if !vec.contains(&id) {
                    vec.push(id);
                }
            } else if !param_ids[..count].contains(&id) {
                if count < 64 {
                    param_ids[count] = id;
                    count += 1;
                } else {
                    let mut v = param_ids[..count].to_vec();
                    v.push(id);
                    fallback_vec = Some(v);
                }
            }
        };
    }

    if let Some(out) = outgoing {
        for p in out.parameters() {
            add_id!(p.id());
        }
    }
    for p in target.parameters() {
        add_id!(p.id());
    }

    let ids: &[&str] = if let Some(v) = &fallback_vec {
        v.as_slice()
    } else {
        &param_ids[..count]
    };

    for &id in ids {
        let Some(index) = model.parameter_index(id) else {
            continue;
        };
        let current = model.parameter_values()[index];

        let (out_add, out_mul, out_over) = if let Some(out) = outgoing {
            if let Some(p) = out.parameters().iter().find(|param| param.id() == id) {
                expression_blend_values(p, current)
            } else {
                (0.0, 1.0, current)
            }
        } else {
            (0.0, 1.0, current)
        };

        let (tgt_add, tgt_mul, tgt_over) =
            if let Some(p) = target.parameters().iter().find(|param| param.id() == id) {
                expression_blend_values(p, current)
            } else {
                (0.0, 1.0, current)
            };

        let blended_add = out_add + (tgt_add - out_add) * alpha;
        let blended_mul = out_mul + (tgt_mul - out_mul) * alpha;
        let blended_over = out_over + (tgt_over - out_over) * alpha;

        let final_value = (blended_over + blended_add) * blended_mul;
        let min = model.parameter_minimum_values()[index];
        let max = model.parameter_maximum_values()[index];
        model.parameter_values_mut()[index] = final_value.clamp(min, max);
    }
}

fn expression_blend_values(parameter: &ExpressionParameter, current: f32) -> (f32, f32, f32) {
    match parameter.blend() {
        ExpressionBlend::Add => (parameter.value(), 1.0, current),
        ExpressionBlend::Multiply => (0.0, parameter.value(), current),
        ExpressionBlend::Overwrite => (0.0, 1.0, parameter.value()),
    }
}

pub fn apply_expression_direct(expression: &Expression3, model: &mut CubismModel, weight: f32) {
    for parameter in expression.parameters() {
        let Some(index) = model.parameter_index(parameter.id()) else {
            continue;
        };
        let current = model.parameter_values()[index];
        let (add, mul, over) = expression_blend_values(parameter, current);
        let blended_add = add * weight;
        let blended_mul = 1.0 + (mul - 1.0) * weight;
        let blended_over = current * (1.0 - weight) + over * weight;
        let target = (blended_over + blended_add) * blended_mul;
        let min = model.parameter_minimum_values()[index];
        let max = model.parameter_maximum_values()[index];
        model.parameter_values_mut()[index] = target.clamp(min, max);
    }
}
