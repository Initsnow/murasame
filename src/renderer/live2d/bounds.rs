use mocari::{
    core::Matrix44,
    moc3::Moc3DrawableMesh,
};

pub const MASK_TEXTURE_SIZE: u32 = 1024;
pub const MODEL_VIEW_FILL: f32 = 1.85;

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ModelBounds {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl ModelBounds {
    pub fn from_drawables(drawables: &[Moc3DrawableMesh]) -> Option<Self> {
        let mut bounds: Option<Self> = None;
        for vertex in drawables.iter().flat_map(Moc3DrawableMesh::vertices) {
            let [x, y] = vertex.position();
            bounds = Some(match bounds {
                Some(b) => Self {
                    min_x: b.min_x.min(x),
                    min_y: b.min_y.min(y),
                    max_x: b.max_x.max(x),
                    max_y: b.max_y.max(y),
                },
                None => Self {
                    min_x: x,
                    min_y: y,
                    max_x: x,
                    max_y: y,
                },
            });
        }
        bounds.filter(|b| b.width() > 0.0 && b.height() > 0.0)
    }

    pub fn width(self) -> f32 {
        self.max_x - self.min_x
    }

    pub fn height(self) -> f32 {
        self.max_y - self.min_y
    }

    pub fn center_x(self) -> f32 {
        (self.min_x + self.max_x) * 0.5
    }

    pub fn center_y(self) -> f32 {
        (self.min_y + self.max_y) * 0.5
    }

    /// 根据 Live2D 模型的实际 Mesh 包围盒动态计算贴合窗口基准宽高，留出适当动作余量
    pub fn calculate_fitted_dimensions(self, target_height: f32) -> (f32, f32) {
        let aspect = (self.width() / self.height().max(1e-4)).clamp(0.2, 3.0);
        // 预留约 12% 边缘余量，防止动作伸展超出画布，同时保持高贴合度
        let fitted_width = (target_height * aspect * 1.12).round().clamp(240.0, 1920.0);
        (fitted_width, target_height)
    }
}

pub fn fit_model_matrix_with_scale(
    bounds: ModelBounds,
    width: u32,
    height: u32,
    model_scale: f32,
) -> Matrix44 {
    let aspect = width as f32 / height.max(1) as f32;
    let view_fill = MODEL_VIEW_FILL * model_scale.clamp(0.1, 5.0);
    let fit_x = view_fill / (bounds.width() * aspect);
    let fit_y = view_fill / bounds.height();
    let scale_y = fit_x.min(fit_y);
    let scale_x = scale_y / aspect;

    let mut matrix = Matrix44::identity();
    matrix.scale(scale_x, scale_y);
    matrix.translate(-bounds.center_x() * scale_x, -bounds.center_y() * scale_y);
    matrix
}

pub fn calculate_snug_scissor_rect(
    bounds: ModelBounds,
    render_width: u32,
    render_height: u32,
    model_scale: f32,
) -> (u32, u32, u32, u32) {
    if render_width == 0 || render_height == 0 || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return (0, 0, render_width, render_height);
    }
    let aspect = render_width as f32 / render_height as f32;
    let view_fill = MODEL_VIEW_FILL * model_scale.clamp(0.1, 5.0);
    let fit_x = view_fill / (bounds.width() * aspect);
    let fit_y = view_fill / bounds.height();
    let scale_y = fit_x.min(fit_y);
    let scale_x = scale_y / aspect;

    // Expand bounding box by 12% safety margin to ensure physics swaying and gestures are never clipped
    let half_w = bounds.width() * 0.5 * 1.12;
    let half_h = bounds.height() * 0.5 * 1.12;

    let clip_x1 = -half_w * scale_x;
    let clip_x2 = half_w * scale_x;
    let clip_y1 = -half_h * scale_y;
    let clip_y2 = half_h * scale_y;

    // Convert clip coordinates [-1.0, 1.0] to pixel coordinates [0, width], [0, height]
    let px1 = ((clip_x1 + 1.0) * 0.5 * render_width as f32).floor().max(0.0) as u32;
    let px2 = ((clip_x2 + 1.0) * 0.5 * render_width as f32).ceil().min(render_width as f32) as u32;
    let py1 = ((1.0 - clip_y2) * 0.5 * render_height as f32).floor().max(0.0) as u32;
    let py2 = ((1.0 - clip_y1) * 0.5 * render_height as f32).ceil().min(render_height as f32) as u32;

    let sc_x = px1.min(render_width.saturating_sub(1));
    let sc_y = py1.min(render_height.saturating_sub(1));
    let sc_w = px2.saturating_sub(px1).max(1).min(render_width - sc_x);
    let sc_h = py2.saturating_sub(py1).max(1).min(render_height - sc_y);

    (sc_x, sc_y, sc_w, sc_h)
}

