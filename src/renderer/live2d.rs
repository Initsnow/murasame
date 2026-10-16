use anyhow::{Result, anyhow};
use mocari::{
    ExpressionManager, ModelRuntime, MotionPlayer,
    assets::{DecodedTexture, load_model_runtime},
    core::Matrix44,
    expression::load_expression,
    json::{Expression3, ExpressionBlend, ExpressionParameter},
    moc3::Moc3DrawableMesh,
    motion::load_motion,
    render::wgpu::{
        WgpuClippingPlan, WgpuClippingResources, WgpuLive2dRenderer, WgpuMaskRenderTarget,
        WgpuMeshBuffers, WgpuTexture, WgpuTransform,
    },
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

// 2048x2048 high-definition mask texture prevents blurry clipping masks on eyes/clothes
const MASK_TEXTURE_SIZE: u32 = 2048;
const MODEL_VIEW_FILL: f32 = 1.85;
const SUPERSAMPLE_FACTOR: u32 = 2;

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

pub struct Live2dRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    renderer: WgpuLive2dRenderer,

    runtime: ModelRuntime,
    mesh_buffers: WgpuMeshBuffers,
    textures: Vec<WgpuTexture>,
    clipping_resources: WgpuClippingResources,
    mask_target: WgpuMaskRenderTarget,
    transform: WgpuTransform,
    model_bounds: ModelBounds,

    motion_player: Option<MotionPlayer>,
    fading_motion: Option<(MotionPlayer, f32, f32)>,
    idle_player: Option<MotionPlayer>,
    expression_manager: ExpressionManager,
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
    last_frame: Instant,

    // Natural animation timers
    breath_time: f32,
    blink_timer: f32,
    blink_progress: f32,
}

impl Live2dRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        model_path: impl AsRef<Path>,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self> {
        let color_format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = WgpuLive2dRenderer::new(&device, color_format);

        let loaded = load_model_runtime(model_path.as_ref())
            .map_err(|e| anyhow!("Failed to load model runtime: {e}"))?;
        let runtime = loaded.runtime().clone();
        let model_bounds = ModelBounds::from_drawables(runtime.meshes())
            .ok_or_else(|| anyhow!("Model has no drawable bounds"))?;

        let textures = Self::create_gpu_textures(&renderer, &device, &queue, loaded.textures())?;

        let empty_buffers = WgpuMeshBuffers::from_drawables(&device, &[])
            .ok_or_else(|| anyhow!("Failed to create initial empty mesh buffers"))?;
        let clipping_plan = WgpuClippingPlan::from_mesh_buffers(&empty_buffers);
        let clipping_resources = renderer
            .create_clipping_resources(&device, &clipping_plan)
            .map_err(|e| anyhow!("Failed to create clipping resources: {e}"))?;

        let mask_target = renderer
            .create_mask_render_target(&device, MASK_TEXTURE_SIZE)
            .map_err(|e| anyhow!("Failed to create mask render target: {e}"))?;

        let render_width = width.max(1) * SUPERSAMPLE_FACTOR;
        let render_height = height.max(1) * SUPERSAMPLE_FACTOR;

        let initial_matrix =
            fit_model_matrix_with_scale(model_bounds, render_width, render_height, scale);
        let transform = renderer.create_transform(&device, &initial_matrix);

        let output_texture = Self::create_target_texture(&device, render_width, render_height);
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mesh_buffers = WgpuMeshBuffers::from_drawables(&device, runtime.meshes())
            .ok_or_else(|| anyhow!("Failed to build mesh buffers for model"))?;

        // Attempt to load idle motion
        let model_dir = model_path.as_ref().parent();
        let idle_player = if let Some(dir) = model_dir {
            let idle_motion_file = dir.join("motion").join("motion01.motion3.json");
            if idle_motion_file.exists() {
                match load_motion(&idle_motion_file) {
                    Ok(motion) => {
                        tracing::info!(
                            "Loaded looping idle motion from {}",
                            idle_motion_file.display()
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
        } else {
            None
        };

        let mut instance = Self {
            device,
            queue,
            renderer,
            runtime,
            mesh_buffers,
            textures,
            clipping_resources,
            mask_target,
            transform,
            model_bounds,
            motion_player: None,
            fading_motion: None,
            idle_player,
            expression_manager: ExpressionManager::new(),
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
            last_frame: Instant::now(),
            breath_time: 0.0,
            blink_timer: 2.5,
            blink_progress: 0.0,
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
        self.render_width = width * SUPERSAMPLE_FACTOR;
        self.render_height = height * SUPERSAMPLE_FACTOR;

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
        self.expression_manager = ExpressionManager::new();
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
        Ok(())
    }

    pub fn is_playing_motion(&self) -> bool {
        self.motion_player.is_some()
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
            self.neutral_transition = None;
            self.current_expression = Some(expression.clone());
            self.expression_manager.play(expression);
            if self.expression_timeout > 0.0 {
                self.expression_timer = Some(self.expression_timeout);
            }
        }
        self.dirty = true;
        Ok(())
    }

    pub fn set_parameter(&mut self, id: &str, value: f32) {
        if let Some(index) = self.runtime.parameter_index(id) {
            self.runtime.set_parameter_override_by_index(index, value);
            self.dirty = true;
        }
    }

    pub fn set_mouth_open(&mut self, value: f32) {
        self.set_parameter("ParamMouthOpenY", value.clamp(0.0, 1.0));
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

        self.runtime
            .hit_test(model_x, model_y)
            .map(|area| area.name().to_string())
    }

    fn rebuild_clipping(&mut self) -> Result<()> {
        let mut plan = WgpuClippingPlan::from_mesh_buffers(&self.mesh_buffers);
        plan.prepare_single_texture_masks(&self.mesh_buffers)
            .map_err(|e| anyhow!("Clipping plan preparation failed: {e}"))?;
        self.clipping_resources = self
            .renderer
            .create_clipping_resources(&self.device, &plan)
            .map_err(|e| anyhow!("Failed to create clipping resources: {e}"))?;
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
        Ok(())
    }

    pub fn tick_and_render(&mut self) -> Result<slint::Image> {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        // Reset base states for frame evaluation
        self.runtime.reset_parameters();
        self.runtime.reset_part_opacities();

        // 1. Motion playback
        // 1.1 First apply fading outgoing motion with decaying weight (smooth crossfade)
        if let Some((fading, elapsed, fade_duration)) = self.fading_motion.as_mut() {
            *elapsed += delta;
            let weight = (1.0 - (*elapsed / *fade_duration)).clamp(0.0, 1.0);
            fading.set_weight(weight);
            fading.tick(delta);
            fading.apply(&mut self.runtime);
            if *elapsed >= *fade_duration || fading.is_finished() {
                self.fading_motion = None;
            }
        }

        // 1.2 Then apply primary motion or loop idle motion
        if let Some(player) = self.motion_player.as_mut() {
            player.tick(delta);
            player.apply(&mut self.runtime);
            if player.is_finished() {
                self.motion_player = None;
            }
        } else if let Some(idle) = self.idle_player.as_mut() {
            idle.tick(delta);
            idle.apply(&mut self.runtime);
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
                &mut self.runtime,
                alpha,
            );
            self.dirty = true;
            if transition.elapsed >= transition.duration {
                self.current_expression = Some(transition.neutral.clone());
                self.neutral_transition = None;
            }
        } else if let Some(current) = &self.current_expression {
            if self.expression_manager.is_empty() {
                apply_expression_direct(current, &mut self.runtime, 1.0);
            } else {
                self.expression_manager.tick(delta);
                self.expression_manager.apply(&mut self.runtime);
            }
        } else {
            self.expression_manager.tick(delta);
            self.expression_manager.apply(&mut self.runtime);
        }

        // 3. Natural Breathing oscillation (~3.2s cycle on ParamBreath)
        self.breath_time += delta;
        let breath_val = (self.breath_time * std::f32::consts::PI * 2.0 / 3.2).sin() * 0.5 + 0.5;
        self.set_parameter("ParamBreath", breath_val);

        // 4. Natural Eye Blink state machine
        self.blink_timer -= delta;
        if self.blink_timer <= 0.0 {
            self.blink_progress += delta;
            let eye_open = if self.blink_progress < 0.09 {
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
            self.set_parameter("ParamEyeLOpen", eye_open);
            self.set_parameter("ParamEyeROpen", eye_open);
        }

        // 5. Apply overrides, physics, and pose
        self.runtime.apply_parameter_overrides();
        self.runtime.apply_physics(delta);
        self.runtime.apply_pose(delta);

        if self.runtime.update_meshes().is_none() {
            return Err(anyhow!("Failed to update model meshes"));
        }

        // Update mesh buffers on GPU
        let update_res = self
            .mesh_buffers
            .update_drawables(&self.queue, self.runtime.meshes());
        match update_res {
            Ok(update) => {
                if update.bounds_changed() || update.visibility_changed() {
                    self.update_clipping()?;
                }
            }
            Err(_) => {
                let new_buffers =
                    WgpuMeshBuffers::from_drawables(&self.device, self.runtime.meshes())
                        .ok_or_else(|| anyhow!("Failed to re-create mesh buffers"))?;
                self.mesh_buffers = new_buffers;
                self.rebuild_clipping()?;
            }
        }

        // 6. GPU Render passes
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("murasame.render_pass.encoder"),
            });

        // 6.1 Mask pass
        {
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
        }

        // 6.2 Main pass onto target texture
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

        // Zero-copy conversion from supersampled wgpu::Texture into crisp slint::Image
        let slint_image = slint::Image::try_from(self.output_texture.clone())
            .map_err(|e| anyhow!("Failed to convert wgpu texture to Slint Image: {e}"))?;

        Ok(slint_image)
    }
}

fn blend_and_apply_expressions(
    outgoing: Option<&Expression3>,
    target: &Expression3,
    runtime: &mut ModelRuntime,
    alpha: f32,
) {
    let mut param_ids: Vec<&str> = Vec::new();
    if let Some(out) = outgoing {
        for p in out.parameters() {
            if !param_ids.contains(&p.id()) {
                param_ids.push(p.id());
            }
        }
    }
    for p in target.parameters() {
        if !param_ids.contains(&p.id()) {
            param_ids.push(p.id());
        }
    }

    for id in param_ids {
        let Some(index) = runtime.parameter_index(id) else {
            continue;
        };
        let current = runtime.parameter_value_by_index(index).unwrap_or(0.0);

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
        runtime.set_parameter_by_index(index, final_value);
    }
}

fn expression_blend_values(parameter: &ExpressionParameter, current: f32) -> (f32, f32, f32) {
    match parameter.blend() {
        ExpressionBlend::Add => (parameter.value(), 1.0, current),
        ExpressionBlend::Multiply => (0.0, parameter.value(), current),
        ExpressionBlend::Overwrite => (0.0, 1.0, parameter.value()),
    }
}

fn apply_expression_direct(expression: &Expression3, runtime: &mut ModelRuntime, weight: f32) {
    for parameter in expression.parameters() {
        let Some(index) = runtime.parameter_index(parameter.id()) else {
            continue;
        };
        let Some(current) = runtime.parameter_value_by_index(index) else {
            continue;
        };
        let (add, mul, over) = expression_blend_values(parameter, current);
        let blended_add = add * weight;
        let blended_mul = 1.0 + (mul - 1.0) * weight;
        let blended_over = current * (1.0 - weight) + over * weight;
        let target = (blended_over + blended_add) * blended_mul;
        runtime.set_parameter_by_index(index, target);
    }
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
        let loaded = load_model_runtime(Path::new("Murasame_live2d/Murasame.model3.json")).unwrap();
        let runtime = loaded.runtime().clone();
        let bounds = ModelBounds::from_drawables(runtime.meshes()).unwrap();

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
}
