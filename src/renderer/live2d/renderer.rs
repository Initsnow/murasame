use anyhow::{Context, Result, anyhow};
use mocari::{
    MotionPlayer,
    core::PhysicsRuntime,
    expression::load_expression,
    json::{Expression3, Physics3},
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

use crate::config::GazeConfig;
use crate::cubism::{CubismMoc, CubismModel};
use crate::model::HitAreaEntry;
use crate::renderer::gaze::GazeController;

use super::bounds::*;
use super::expression::*;
use super::motion::*;
use super::hit_test::*;

pub const ACTIVE_DIRTY_THRESHOLD: f32 = 0.003;
pub const IDLE_DIRTY_THRESHOLD: f32 = 0.010;

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

    // Gaze controller
    gaze_controller: GazeController,
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
        let color_format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = WgpuLive2dRenderer::new(&device, color_format);
        let mut gpu_textures = Vec::with_capacity(model_json.file_references.textures.len());
        for tex_rel in &model_json.file_references.textures {
            let tex_path = model_dir.join(tex_rel);
            let img = image::open(&tex_path)
                .with_context(|| format!("Failed to open texture image: {}", tex_path.display()))?;
            let rgba = img.into_rgba8();
            let gpu_tex = renderer
                .create_rgba8_texture(&device, &queue, rgba.width(), rgba.height(), rgba.as_raw())
                .map_err(|e| anyhow!("Failed to create GPU texture: {e}"))?;
            gpu_textures.push(gpu_tex);
        }

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

        let gaze_controller = GazeController::new(GazeConfig::default(), &cubism_model);

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
            gaze_controller,
        };

        instance.rebuild_clipping()?;
        Ok(instance)
    }

    pub fn reload_model(&mut self, model_path: impl AsRef<Path>, scale: f32) -> Result<()> {
        let mut new_renderer = Self::new(
            Arc::clone(&self.device),
            Arc::clone(&self.queue),
            model_path,
            self.logical_width,
            self.logical_height,
            scale,
            self.supersample,
        )?;
        new_renderer.gaze_controller.config = self.gaze_controller.config.clone();
        *self = new_renderer;
        Ok(())
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

    pub fn set_gaze_config(&mut self, config: GazeConfig) {
        self.gaze_controller.set_config(config);
        self.force_dirty = true;
    }

    pub fn update_gaze_target(
        &mut self,
        cursor: Option<(i32, i32)>,
        win_rect: Option<(i32, i32, i32, i32)>,
        is_fullscreen: bool,
    ) {
        let is_playing_motion = self.is_playing_motion();
        self.gaze_controller
            .update_target(cursor, win_rect, is_fullscreen, is_playing_motion);
    }

    pub fn is_gaze_active(&self) -> bool {
        self.gaze_controller.is_active()
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

    pub fn set_dirty_threshold(&mut self, threshold: f32) {
        self.dirty_threshold = threshold.clamp(0.0001, 0.05);
    }

    pub fn dirty_threshold(&self) -> f32 {
        self.dirty_threshold
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
        self.tick_delta_and_render(delta)
    }

    pub fn tick_delta_and_render(&mut self, delta: f32) -> Result<Option<slint::Image>> {
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

        // 4.5. Gaze tracking smoothing & parameter injection
        self.gaze_controller.tick(delta);
        self.gaze_controller.apply(&mut self.cubism_model);

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
        let is_animating = self.is_active_animation() || self.gaze_controller.is_active();
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

