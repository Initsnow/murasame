use anyhow::{Context, Result, anyhow};
use mocari::{
    MotionPlayer,
    assets::DecodedTexture,
    core::{Matrix44, PhysicsRuntime},
    expression::load_expression,
    json::{
        Expression3, ExpressionBlend, ExpressionParameter, Physics3,
        apply_motion_fade, motion_fade_in_weight, motion_fade_out_weight,
    },
    moc3::Moc3DrawableMesh,
    motion::load_motion,
    render::wgpu::{
        WgpuClippingPlan, WgpuClippingResources, WgpuLive2dRenderer, WgpuMaskRenderTarget,
        WgpuMeshBuffers, WgpuTexture, WgpuTransform,
    },
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use crate::cubism::{CubismMoc, CubismModel};
use crate::model::HitAreaEntry;

// 1024x1024 high-definition mask texture provides crisp eye/clothes clipping while saving 75% GPU fill rate and memory over 2048
const MASK_TEXTURE_SIZE: u32 = 1024;
const MODEL_VIEW_FILL: f32 = 1.85;

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

pub struct Live2dRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    renderer: WgpuLive2dRenderer,

    cubism_model: CubismModel,
    // Must keep cubism_moc alive so its underlying memory buffer remains valid for cubism_model
    #[allow(dead_code)]
    cubism_moc: CubismMoc,
    meshes: Vec<Moc3DrawableMesh>,
    physics: Option<PhysicsRuntime>,
    hit_areas: Vec<HitAreaEntry>,
    parameter_overrides: HashMap<usize, f32>,

    mesh_buffers: WgpuMeshBuffers,
    textures: Vec<WgpuTexture>,
    clipping_resources: WgpuClippingResources,
    mask_drawable_indices: Vec<usize>,
    mask_dirty: bool,
    mask_target: WgpuMaskRenderTarget,
    transform: WgpuTransform,
    model_bounds: ModelBounds,

    motion_player: Option<MotionPlayer>,
    fading_motion: Option<(MotionPlayer, f32, f32)>,
    idle_player: Option<MotionPlayer>,
    idle_has_breath: bool,
    neutral_expression_path: Option<PathBuf>,
    current_expression: Option<Expression3>,
    neutral_transition: Option<NeutralTransition>,
    expression_timeout: f32,
    expression_timer: Option<f32>,
    expression_fade_time: f32,
    easing_curve: ExpressionEasingCurve,
    motion_fade_out: f32,
    dirty: bool,

    output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,

    logical_width: u32,
    logical_height: u32,
    render_width: u32,
    render_height: u32,
    scale: f32,
    supersample: f32,
    last_frame: Instant,

    // Natural animation timers
    breath_time: f32,
    blink_timer: f32,
    blink_progress: f32,

    // Perceptual dirty tracking (方案 1)
    last_rendered_parameters: Vec<f32>,
    last_rendered_time: Instant,
    force_dirty: bool,
    dirty_threshold: f32,
}

impl Live2dRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        model_path: impl AsRef<Path>,
        width: u32,
        height: u32,
        scale: f32,
        supersample: f32,
    ) -> Result<Self> {
        let model_file = model_path.as_ref();
        let model_dir = model_file
            .parent()
            .ok_or_else(|| anyhow!("Model path has no parent directory: {}", model_file.display()))?;

        let model_json_content = fs::read_to_string(model_file)
            .with_context(|| format!("Failed to read model JSON from {}", model_file.display()))?;
        let model_json: crate::model::loader::Model3Json = serde_json::from_str(&model_json_content)
            .with_context(|| format!("Failed to parse model JSON from {}", model_file.display()))?;

        // 1. Load Cubism Moc and instantiate Model via Native Cubism Core SDK
        let moc_path = model_dir.join(&model_json.file_references.moc);
        let cubism_moc = CubismMoc::from_file(&moc_path)
            .with_context(|| format!("Failed to load moc3 from {}", moc_path.display()))?;
        let cubism_model = CubismModel::new(&cubism_moc)
            .with_context(|| format!("Failed to initialize CubismModel from {}", moc_path.display()))?;

        // 2. Build initial drawable meshes and model bounds
        let meshes = cubism_model.to_drawable_meshes();
        let model_bounds = ModelBounds::from_drawables(&meshes)
            .ok_or_else(|| anyhow!("Model has no drawable bounds"))?;

        // 3. Load textures
        let mut textures = Vec::with_capacity(model_json.file_references.textures.len());
        for tex_rel in &model_json.file_references.textures {
            let tex_path = model_dir.join(tex_rel);
            let img = image::open(&tex_path)
                .with_context(|| format!("Failed to open texture image: {}", tex_path.display()))?;
            let rgba = img.to_rgba8();
            textures.push(DecodedTexture::new(
                rgba.width(),
                rgba.height(),
                rgba.into_raw(),
            ));
        }

        let color_format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = WgpuLive2dRenderer::new(&device, color_format);
        let gpu_textures = Self::create_gpu_textures(&renderer, &device, &queue, &textures)?;

        let empty_buffers = WgpuMeshBuffers::from_drawables(&device, &[])
            .ok_or_else(|| anyhow!("Failed to create initial empty mesh buffers"))?;
        let clipping_plan = WgpuClippingPlan::from_mesh_buffers(&empty_buffers);
        let clipping_resources = renderer
            .create_clipping_resources(&device, &clipping_plan)
            .map_err(|e| anyhow!("Failed to create clipping resources: {e}"))?;

        let mask_target = renderer
            .create_mask_render_target(&device, MASK_TEXTURE_SIZE)
            .map_err(|e| anyhow!("Failed to create mask render target: {e}"))?;

        let supersample = supersample.clamp(1.0, 4.0);
        let render_width = ((width.max(1) as f32) * supersample).round() as u32;
        let render_height = ((height.max(1) as f32) * supersample).round() as u32;

        let initial_matrix =
            fit_model_matrix_with_scale(model_bounds, render_width, render_height, scale);
        let transform = renderer.create_transform(&device, &initial_matrix);

        let output_texture = Self::create_target_texture(&device, render_width, render_height);
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mesh_buffers = WgpuMeshBuffers::from_drawables(&device, &meshes)
            .ok_or_else(|| anyhow!("Failed to build mesh buffers for model"))?;

        // 4. Load physics if available
        let physics = if let Some(phys_rel) = &model_json.file_references.physics {
            let phys_path = model_dir.join(phys_rel);
            if phys_path.exists() {
                match fs::read_to_string(&phys_path) {
                    Ok(phys_str) => match Physics3::from_json_str(&phys_str) {
                        Ok(phys3) => {
                            tracing::info!("Loaded physics from {}", phys_path.display());
                            Some(PhysicsRuntime::new(&phys3, cubism_model.parameter_ids()))
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse physics {}: {e}", phys_path.display());
                            None
                        }
                    },
                    Err(e) => {
                        tracing::warn!("Failed to read physics {}: {e}", phys_path.display());
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // 5. Attempt to load idle motion
        let idle_player = {
            let idle_candidate = model_json
                .file_references
                .motions
                .get("Idle")
                .and_then(|entries| entries.first())
                .map(|e| model_dir.join(&e.file))
                .unwrap_or_else(|| model_dir.join("motion").join("motion01.motion3.json"));
            if idle_candidate.exists() {
                match load_motion(&idle_candidate) {
                    Ok(motion) => {
                        tracing::info!(
                            "Loaded looping idle motion from {}",
                            idle_candidate.display()
                        );
                        Some(MotionPlayer::with_looping(motion, true))
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load idle motion: {e}");
                        None
                    }
                }
            } else {
                None
            }
        };
        let idle_has_breath = idle_player.as_ref().map_or(false, |player| {
            player
                .motion()
                .curves()
                .iter()
                .any(|c| c.target() == "Parameter" && c.id() == "ParamBreath")
        });

        let mut instance = Self {
            device,
            queue,
            renderer,
            cubism_model,
            cubism_moc,
            meshes,
            physics,
            hit_areas: model_json.hit_areas,
            parameter_overrides: HashMap::new(),
            mesh_buffers,
            textures: gpu_textures,
            clipping_resources,
            mask_drawable_indices: Vec::new(),
            mask_dirty: true,
            mask_target,
            transform,
            model_bounds,
            motion_player: None,
            fading_motion: None,
            idle_player,
            idle_has_breath,
            neutral_expression_path: None,
            current_expression: None,
            neutral_transition: None,
            expression_timeout: 5.0,
            expression_timer: None,
            expression_fade_time: 1.2,
            easing_curve: ExpressionEasingCurve::Smootherstep,
            motion_fade_out: 0.25,
            dirty: true,
            output_texture,
            output_view,
            logical_width: width,
            logical_height: height,
            render_width,
            render_height,
            scale,
            supersample,
            last_frame: Instant::now(),
            breath_time: 0.0,
            blink_timer: 2.5,
            blink_progress: 0.0,
            last_rendered_parameters: Vec::new(),
            last_rendered_time: Instant::now(),
            force_dirty: true,
            dirty_threshold: 0.003,
        };

        instance.rebuild_clipping()?;
        Ok(instance)
    }

    pub fn reload_model(&mut self, model_path: impl AsRef<Path>, scale: f32) -> Result<()> {
        let new_renderer = Self::new(
            Arc::clone(&self.device),
            Arc::clone(&self.queue),
            model_path,
            self.logical_width,
            self.logical_height,
            scale,
            self.supersample,
        )?;
        *self = new_renderer;
        Ok(())
    }

    fn create_gpu_textures(
        renderer: &WgpuLive2dRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: &[DecodedTexture],
    ) -> Result<Vec<WgpuTexture>> {
        let mut list = Vec::new();
        for tex in textures {
            let gpu_tex = renderer
                .create_rgba8_texture(device, queue, tex.width(), tex.height(), tex.rgba())
                .map_err(|e| anyhow!("Failed to create GPU texture: {e}"))?;
            list.push(gpu_tex);
        }
        Ok(list)
    }

    fn create_target_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("murasame.live2d.render_target"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if (self.logical_width == width && self.logical_height == height)
            || width == 0
            || height == 0
        {
            return;
        }
        self.logical_width = width;
        self.logical_height = height;
        self.recreate_render_target();
    }

    pub fn set_supersample(&mut self, factor: f32) {
        let factor = factor.clamp(1.0, 4.0);
        if (self.supersample - factor).abs() < 1e-4 {
            return;
        }
        self.supersample = factor;
        self.recreate_render_target();
    }

    pub fn supersample(&self) -> f32 {
        self.supersample
    }

    fn recreate_render_target(&mut self) {
        self.render_width = ((self.logical_width.max(1) as f32) * self.supersample).round() as u32;
        self.render_height = ((self.logical_height.max(1) as f32) * self.supersample).round() as u32;

        self.output_texture =
            Self::create_target_texture(&self.device, self.render_width, self.render_height);
        self.output_view = self
            .output_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.update_transform_matrix();
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.clamp(0.2, 3.5);
        self.update_transform_matrix();
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    fn update_transform_matrix(&mut self) {
        let matrix = fit_model_matrix_with_scale(
            self.model_bounds,
            self.render_width,
            self.render_height,
            self.scale,
        );
        self.transform.update_matrix(&self.queue, &matrix);
        self.dirty = true;
        self.mask_dirty = true;
    }

    pub fn set_neutral_expression(&mut self, path: Option<PathBuf>) {
        if let Some(p) = &path {
            if self.current_expression.is_none() && self.neutral_transition.is_none() {
                if let Ok(exp) = load_expression(p) {
                    self.current_expression = Some(exp);
                    self.dirty = true;
                }
            }
        }
        self.neutral_expression_path = path;
    }

    pub fn set_expression_timeout(&mut self, timeout: f32) {
        self.expression_timeout = timeout.max(0.1);
    }

    pub fn set_expression_fade_time(&mut self, fade: f32) {
        self.expression_fade_time = fade.max(0.1);
    }

    pub fn set_easing_curve(&mut self, curve: ExpressionEasingCurve) {
        self.easing_curve = curve;
    }

    pub fn start_neutral_transition(&mut self, neutral_exp: Expression3) {
        self.expression_timer = None;
        self.neutral_transition = Some(NeutralTransition {
            outgoing: self.current_expression.take(),
            neutral: neutral_exp,
            elapsed: 0.0,
            duration: self.expression_fade_time,
        });
        self.dirty = true;
    }

    pub fn set_motion_fade_out(&mut self, fade: f32) {
        self.motion_fade_out = fade.max(0.0);
    }

    pub fn reset_to_neutral(&mut self) -> Result<()> {
        if let Some(path) = &self.neutral_expression_path {
            let expression = load_expression(path).map_err(|e| {
                anyhow!("Failed to load neutral expression {}: {e}", path.display())
            })?;
            self.start_neutral_transition(expression);
        }
        Ok(())
    }

    pub fn play_motion(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let motion = load_motion(path.as_ref()).map_err(|e| {
            anyhow!(
                "Failed to load motion file {}: {e}",
                path.as_ref().display()
            )
        })?;

        // If there is currently an active motion, fade it out smoothly to avoid twitching
        if let Some(mut current) = self.motion_player.take() {
            if !current.is_finished() && self.motion_fade_out > 0.0 {
                current.set_weight(1.0);
                self.fading_motion = Some((current, 0.0, self.motion_fade_out));
            }
        }

        self.motion_player = Some(MotionPlayer::new_once(motion));
        self.dirty = true;
        self.force_dirty = true;
        Ok(())
    }

    pub fn is_playing_motion(&self) -> bool {
        self.motion_player.is_some() || self.fading_motion.is_some()
    }

    pub fn is_active_animation(&self) -> bool {
        self.motion_player.is_some()
            || self.fading_motion.is_some()
            || self.neutral_transition.is_some()
    }

    pub fn mark_dirty(&mut self) {
        self.force_dirty = true;
        self.dirty = true;
    }

    pub fn reset_last_frame(&mut self) {
        self.last_frame = Instant::now();
        self.force_dirty = true;
    }

    pub fn play_expression(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let p = path.as_ref();
        let expression = load_expression(p)
            .map_err(|e| anyhow!("Failed to load expression file {}: {e}", p.display()))?;

        let is_neutral = self
            .neutral_expression_path
            .as_ref()
            .map(|n| n == p)
            .unwrap_or(false);
        if is_neutral {
            self.start_neutral_transition(expression);
            self.expression_timer = None;
        } else {
            self.neutral_transition = Some(NeutralTransition {
                outgoing: self.current_expression.take(),
                neutral: expression.clone(),
                elapsed: 0.0,
                duration: self.expression_fade_time,
            });
            if self.expression_timeout > 0.0 {
                self.expression_timer = Some(self.expression_timeout);
            }
        }
        self.dirty = true;
        self.force_dirty = true;
        Ok(())
    }

    pub fn set_parameter(&mut self, id: &str, value: f32) {
        if let Some(index) = self.cubism_model.parameter_index(id) {
            let min = self.cubism_model.parameter_minimum_values()[index];
            let max = self.cubism_model.parameter_maximum_values()[index];
            let clamped = value.clamp(min, max);
            let prev = self.parameter_overrides.get(&index).copied();
            if prev != Some(clamped) {
                self.parameter_overrides.insert(index, clamped);
                self.dirty = true;
                let range = (max - min).abs().max(1e-4);
                let delta = prev.map_or(1.0, |p| (clamped - p).abs() / range);
                if delta >= self.dirty_threshold {
                    self.force_dirty = true;
                }
            }
        }
    }

    pub fn set_mouth_open(&mut self, value: f32) {
        let val = value.clamp(0.0, 1.0);
        if let Some(index) = self.cubism_model.parameter_index("ParamMouthOpenY") {
            let prev = self.parameter_overrides.get(&index).copied().unwrap_or(0.0);
            if (val - prev).abs() > 0.003 {
                self.set_parameter("ParamMouthOpenY", val);
            } else if prev != 0.0 && val == 0.0 {
                self.set_parameter("ParamMouthOpenY", 0.0);
            }
        }
    }

    pub fn output_texture(&self) -> &wgpu::Texture {
        &self.output_texture
    }

    pub fn hit_test(&self, cursor_x: f64, cursor_y: f64) -> Option<String> {
        let matrix = fit_model_matrix_with_scale(
            self.model_bounds,
            self.logical_width,
            self.logical_height,
            self.scale,
        );
        let clip_x = (cursor_x as f32 / self.logical_width as f32) * 2.0 - 1.0;
        let clip_y = 1.0 - (cursor_y as f32 / self.logical_height as f32) * 2.0;
        let model_x = matrix.invert_transform_x(clip_x);
        let model_y = matrix.invert_transform_y(clip_y);

        for area in &self.hit_areas {
            if let Some(mesh_idx) = self.cubism_model.drawable_index(&area.id) {
                if let Some(mesh) = self.meshes.get(mesh_idx) {
                    if drawable_contains_point(mesh, model_x, model_y) {
                        return Some(area.name.clone());
                    }
                }
            }
        }
        None
    }

    fn rebuild_clipping(&mut self) -> Result<()> {
        let mut plan = WgpuClippingPlan::from_mesh_buffers(&self.mesh_buffers);
        plan.prepare_single_texture_masks(&self.mesh_buffers)
            .map_err(|e| anyhow!("Clipping plan preparation failed: {e}"))?;
        self.clipping_resources = self
            .renderer
            .create_clipping_resources(&self.device, &plan)
            .map_err(|e| anyhow!("Failed to create clipping resources: {e}"))?;
        self.mask_drawable_indices.clear();
        for ctx in self.clipping_resources.contexts() {
            for &idx in ctx.mask_drawable_indices() {
                if !self.mask_drawable_indices.contains(&idx) {
                    self.mask_drawable_indices.push(idx);
                }
            }
        }
        self.mask_dirty = true;
        Ok(())
    }

    fn update_clipping(&mut self) -> Result<()> {
        let mut plan = WgpuClippingPlan::from_mesh_buffers(&self.mesh_buffers);
        plan.prepare_single_texture_masks(&self.mesh_buffers)
            .map_err(|e| anyhow!("Clipping plan preparation failed: {e}"))?;
        if !self
            .renderer
            .update_clipping_resources(&self.queue, &mut self.clipping_resources, &plan)
            .map_err(|e| anyhow!("Failed to update clipping resources: {e}"))?
        {
            self.clipping_resources = self
                .renderer
                .create_clipping_resources(&self.device, &plan)
                .map_err(|e| anyhow!("Failed to recreate clipping resources: {e}"))?;
        }
        self.mask_drawable_indices.clear();
        for ctx in self.clipping_resources.contexts() {
            for &idx in ctx.mask_drawable_indices() {
                if !self.mask_drawable_indices.contains(&idx) {
                    self.mask_drawable_indices.push(idx);
                }
            }
        }
        self.mask_dirty = true;
        Ok(())
    }

    pub fn tick_and_render(&mut self) -> Result<Option<slint::Image>> {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        if !self.tick_delta(delta)? {
            return Ok(None);
        }

        // Zero-copy conversion from supersampled wgpu::Texture into crisp slint::Image
        let slint_image = slint::Image::try_from(self.output_texture.clone())
            .map_err(|e| anyhow!("Failed to convert wgpu texture to Slint Image: {e}"))?;

        Ok(Some(slint_image))
    }

    pub fn tick_delta(&mut self, delta: f32) -> Result<bool> {
        // Reset base states for frame evaluation
        self.cubism_model.reset_parameters();
        self.cubism_model.reset_part_opacities();

        // 1. Motion playback
        // 1.1 First apply looping idle motion with seamless loop crossfade as the persistent foundational base layer
        if let Some(idle) = self.idle_player.as_mut() {
            apply_looping_idle_motion(idle, &mut self.cubism_model, delta);
        }

        // 1.2 Apply outgoing fading motion with decaying weight (smooth crossfade on interruption)
        if let Some((fading, elapsed, fade_duration)) = self.fading_motion.as_mut() {
            *elapsed += delta;
            let weight = (1.0 - (*elapsed / *fade_duration)).clamp(0.0, 1.0);
            fading.set_weight(weight);
            fading.tick(delta);
            apply_motion_to_cubism(fading, &mut self.cubism_model, 0.0, 0.0);
            if *elapsed >= *fade_duration {
                self.fading_motion = None;
            }
        }

        // 1.3 Apply primary one-shot motion with smooth FadeIn and FadeOut transitions
        if let Some(player) = self.motion_player.as_mut() {
            player.tick(delta);
            let duration = player.motion().meta().duration();
            let max_fade = (duration * 0.5).max(0.0);
            let fade_in = self.motion_fade_out.min(max_fade);
            let fade_out = self.motion_fade_out.min(max_fade);
            apply_motion_to_cubism(player, &mut self.cubism_model, fade_in, fade_out);
            if player.is_finished() {
                self.motion_player = None;
            }
        }

        // 2. Expressions & Neutral auto-reset timeout with smooth animation curve
        if let Some(timer) = self.expression_timer.as_mut() {
            *timer -= delta;
            if *timer <= 0.0 {
                self.expression_timer = None;
                if let Some(neutral_path) = &self.neutral_expression_path {
                    if let Ok(exp) = load_expression(neutral_path) {
                        tracing::info!(
                            "Expression reset timeout elapsed: animated transition to neutral using {:?} curve (duration: {:.2}s)",
                            self.easing_curve,
                            self.expression_fade_time
                        );
                        self.start_neutral_transition(exp);
                    }
                }
            }
        }

        if let Some(transition) = self.neutral_transition.as_mut() {
            transition.elapsed += delta;
            let t = (transition.elapsed / transition.duration).clamp(0.0, 1.0);
            let alpha = self.easing_curve.evaluate(t);
            blend_and_apply_expressions(
                transition.outgoing.as_ref(),
                &transition.neutral,
                &mut self.cubism_model,
                alpha,
            );
            self.dirty = true;
            if transition.elapsed >= transition.duration {
                self.current_expression = Some(transition.neutral.clone());
                self.neutral_transition = None;
            }
        } else if let Some(current) = &self.current_expression {
            apply_expression_direct(current, &mut self.cubism_model, 1.0);
        }

        // 3. Natural Breathing oscillation (~3.2s cycle on ParamBreath)
        // Only run procedural breathing if NO active motion controls ParamBreath
        let motion_controls_breath = (self.idle_has_breath && self.idle_player.is_some())
            || self.motion_player.as_ref().map_or(false, |p| {
                p.motion()
                    .curves()
                    .iter()
                    .any(|c| c.target() == "Parameter" && c.id() == "ParamBreath")
            })
            || self.fading_motion.as_ref().map_or(false, |(p, _, _)| {
                p.motion()
                    .curves()
                    .iter()
                    .any(|c| c.target() == "Parameter" && c.id() == "ParamBreath")
            });

        if !motion_controls_breath {
            self.breath_time += delta;
            let breath_val = (self.breath_time * std::f32::consts::PI * 2.0 / 3.2).sin() * 0.5 + 0.5;
            if let Some(idx) = self.cubism_model.parameter_index("ParamBreath") {
                let min = self.cubism_model.parameter_minimum_values()[idx];
                let max = self.cubism_model.parameter_maximum_values()[idx];
                self.cubism_model.parameter_values_mut()[idx] = breath_val.clamp(min, max);
            }
        }

        // 4. Natural Eye Blink state machine (multiplicative scaling directly on eye openness)
        self.blink_timer -= delta;
        if self.blink_timer <= 0.0 {
            self.blink_progress += delta;
            let eye_scale = if self.blink_progress < 0.09 {
                // Closing eye: 1.0 -> 0.0
                1.0 - (self.blink_progress / 0.09)
            } else if self.blink_progress < 0.13 {
                // Fully closed
                0.0
            } else if self.blink_progress < 0.23 {
                // Opening eye: 0.0 -> 1.0
                (self.blink_progress - 0.13) / 0.10
            } else {
                // Blink completed, schedule next blink randomly in 2.5s .. 5.5s
                self.blink_timer = 2.5 + rand::random::<f32>() * 3.0;
                self.blink_progress = 0.0;
                1.0
            };
            if eye_scale < 1.0 {
                if let Some(idx) = self.cubism_model.parameter_index("ParamEyeLOpen") {
                    self.cubism_model.parameter_values_mut()[idx] *= eye_scale;
                }
                if let Some(idx) = self.cubism_model.parameter_index("ParamEyeROpen") {
                    self.cubism_model.parameter_values_mut()[idx] *= eye_scale;
                }
            }
        }

        // 5. Apply explicit external overrides (e.g. lip-sync mouth opening) and physics
        let values = self.cubism_model.parameter_values_mut();
        for (&index, &override_val) in &self.parameter_overrides {
            if let Some(slot) = values.get_mut(index) {
                *slot = override_val;
            }
        }

        if let Some(physics) = self.physics.as_mut() {
            self.cubism_model.evaluate_physics(physics, delta);
        }

        // 5.5 方案 1: 感知级脏区跳帧与微动累积判定 (Perceptual Dirty Thresholding)
        let is_animating = self.is_active_animation();
        let is_blinking = self.blink_timer <= 0.0;
        let is_heartbeat = self.last_rendered_time.elapsed() >= std::time::Duration::from_millis(1500);
        let must_render = self.force_dirty || is_animating || is_blinking || is_heartbeat || self.last_rendered_parameters.is_empty();

        let is_dirty = if must_render {
            true
        } else {
            let current_values = self.cubism_model.parameter_values();
            let mins = self.cubism_model.parameter_minimum_values();
            let maxs = self.cubism_model.parameter_maximum_values();

            let mut max_delta: f32 = 0.0;
            for (i, (&curr, &last)) in current_values.iter().zip(self.last_rendered_parameters.iter()).enumerate() {
                let range = (maxs.get(i).copied().unwrap_or(1.0) - mins.get(i).copied().unwrap_or(0.0)).abs().max(1e-4);
                let normalized_delta = (curr - last).abs() / range;
                if normalized_delta > max_delta {
                    max_delta = normalized_delta;
                }
            }

            max_delta >= self.dirty_threshold
        };

        if !is_dirty {
            // 参数变化量在人眼感知门限以下：完全跳过昂贵的 GPU RenderPass 与 Slint 重绘！
            return Ok(false);
        }

        self.last_rendered_parameters.clear();
        self.last_rendered_parameters.extend_from_slice(self.cubism_model.parameter_values());
        self.last_rendered_time = Instant::now();
        self.force_dirty = false;

        // 6. Update deformers and drawable meshes via Native Cubism SDK
        self.cubism_model.update();
        self.cubism_model.update_drawable_meshes(&mut self.meshes);

        // Check if any mask drawable deformed or changed visibility
        let dynamic_flags = self.cubism_model.drawable_dynamic_flags();
        let mask_changed = self.mask_drawable_indices.iter().any(|&idx| {
            let flag = dynamic_flags.get(idx).copied().unwrap_or(0);
            (flag & (crate::cubism::sys::CSM_VERTEX_POSITIONS_DID_CHANGE
                | crate::cubism::sys::CSM_VISIBILITY_DID_CHANGE
                | crate::cubism::sys::CSM_OPACITY_DID_CHANGE)) != 0
        });
        if mask_changed {
            self.mask_dirty = true;
        }

        // 方案 2: Native Cubism SDK 动态标志过滤：仅在存在动态变更时提交网格缓冲区更新
        let any_mesh_changed = dynamic_flags.iter().any(|&flag| (flag & 0x7E) != 0);
        if any_mesh_changed || self.mesh_buffers.drawables().is_empty() {
            let update_res = self
                .mesh_buffers
                .update_drawables(&self.queue, &self.meshes);
            match update_res {
                Ok(update) => {
                    if update.bounds_changed() || update.visibility_changed() {
                        self.update_clipping()?;
                        self.mask_dirty = true;
                    }
                }
                Err(_) => {
                    let new_buffers =
                        WgpuMeshBuffers::from_drawables(&self.device, &self.meshes)
                            .ok_or_else(|| anyhow!("Failed to re-create mesh buffers"))?;
                    self.mesh_buffers = new_buffers;
                    self.rebuild_clipping()?;
                    self.mask_dirty = true;
                }
            }
        }

        self.cubism_model.reset_dynamic_flags();

        // 7. GPU Render passes
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("murasame.render_pass.encoder"),
            });

        // 7.1 Mask pass: Only execute when clipping masks have actually changed
        if self.mask_dirty && !self.clipping_resources.contexts().is_empty() {
            let mut mask_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("murasame.mask_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.mask_target.view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            self.renderer
                .draw_masks_with_textures(
                    &mut mask_pass,
                    &self.mesh_buffers,
                    &self.clipping_resources,
                    &self.textures,
                )
                .map_err(|e| anyhow!("Failed to draw masks: {e}"))?;

            self.mask_dirty = false;
        }

        // 7.2 Main pass onto target texture with Snug Scissor Rect (方案 4)
        {
            let mut main_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("murasame.main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // 方案 4: 视口紧凑裁剪：将光栅化限定在模型实际包围盒安全外接矩形内，消灭边缘空透明像素算力浪费
            let (sc_x, sc_y, sc_w, sc_h) = calculate_snug_scissor_rect(
                self.model_bounds,
                self.render_width,
                self.render_height,
                self.scale,
            );
            if sc_w > 0 && sc_h > 0 {
                main_pass.set_scissor_rect(sc_x, sc_y, sc_w, sc_h);
            }

            self.renderer
                .draw_with_textures_clipping_and_transform(
                    &mut main_pass,
                    &self.mesh_buffers,
                    &self.textures,
                    &self.clipping_resources,
                    &self.mask_target,
                    &self.transform,
                )
                .map_err(|e| anyhow!("Failed to draw model: {e}"))?;
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        Ok(true)
    }

    pub fn capture_frame(&self) -> Result<image::RgbaImage> {
        let width = self.render_width;
        let height = self.render_height;
        let bytes_per_pixel = 4;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = ((unpadded_bytes_per_row + align - 1) / align) * align;
        let buffer_size = (padded_bytes_per_row * height) as wgpu::BufferAddress;

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live2d.capture_staging_buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live2d.capture_encoder"),
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = staging_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .map_err(|e| anyhow!("device poll failed: {e:?}"))?;
        receiver.recv()??;

        let mapped = buffer_slice.get_mapped_range()
            .map_err(|e| anyhow!("Failed to get mapped range: {e:?}"))?;
        let mut raw_pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + unpadded_bytes_per_row as usize;
            raw_pixels.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        staging_buffer.unmap();

        let img = image::RgbaImage::from_raw(width, height, raw_pixels)
            .ok_or_else(|| anyhow!("Failed to create RgbaImage from raw pixels"))?;
        Ok(img)
    }
}

/// Applies looping idle motion with smooth crossfade back to the starting frame (time = 0.0),
/// eliminating any twitching/glitching at loop wraparounds.
fn apply_looping_idle_motion(
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

fn apply_motion_to_cubism(
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

fn blend_and_apply_expressions(
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

fn apply_expression_direct(expression: &Expression3, model: &mut CubismModel, weight: f32) {
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

fn drawable_contains_point(mesh: &Moc3DrawableMesh, x: f32, y: f32) -> bool {
    let Some(first) = mesh.vertices().first() else {
        return false;
    };

    let [first_x, first_y] = first.position();
    let mut min_x = first_x;
    let mut min_y = first_y;
    let mut max_x = first_x;
    let mut max_y = first_y;

    for vertex in mesh.vertices().iter().skip(1) {
        let [vertex_x, vertex_y] = vertex.position();
        min_x = min_x.min(vertex_x);
        min_y = min_y.min(vertex_y);
        max_x = max_x.max(vertex_x);
        max_y = max_y.max(vertex_y);
    }

    (min_x..=max_x).contains(&x) && (min_y..=max_y).contains(&y)
}

#[cfg(test)]
mod tests {
    use super::*;

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
