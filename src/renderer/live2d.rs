use std::{
    path::Path,
    sync::Arc,
    time::Instant,
};
use anyhow::{Result, anyhow};
use mocari::{
    ExpressionManager, ModelRuntime, MotionPlayer,
    assets::{DecodedTexture, load_model_runtime},
    core::Matrix44,
    expression::load_expression,
    moc3::Moc3DrawableMesh,
    motion::load_motion,
    render::wgpu::{
        WgpuClippingPlan, WgpuClippingResources, WgpuLive2dRenderer, WgpuMaskRenderTarget,
        WgpuMeshBuffers, WgpuTexture, WgpuTransform,
    },
};

const MASK_TEXTURE_SIZE: u32 = 256;
const MODEL_VIEW_FILL: f32 = 1.85;

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
    expression_manager: ExpressionManager,
    dirty: bool,

    output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,

    width: u32,
    height: u32,
    scale: f32,
    last_frame: Instant,
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

        let initial_matrix = fit_model_matrix_with_scale(model_bounds, width, height, scale);
        let transform = renderer.create_transform(&device, &initial_matrix);

        let output_texture = Self::create_target_texture(&device, width, height);
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mesh_buffers = WgpuMeshBuffers::from_drawables(&device, runtime.meshes())
            .ok_or_else(|| anyhow!("Failed to build mesh buffers for model"))?;

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
            expression_manager: ExpressionManager::new(),
            dirty: true,
            output_texture,
            output_view,
            width,
            height,
            scale,
            last_frame: Instant::now(),
        };

        instance.rebuild_clipping()?;
        Ok(instance)
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
        if (self.width == width && self.height == height) || width == 0 || height == 0 {
            return;
        }
        self.width = width;
        self.height = height;
        self.output_texture = Self::create_target_texture(&self.device, width, height);
        self.output_view = self.output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.update_transform_matrix();
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.update_transform_matrix();
    }

    fn update_transform_matrix(&mut self) {
        let matrix = fit_model_matrix_with_scale(self.model_bounds, self.width, self.height, self.scale);
        self.transform.update_matrix(&self.queue, &matrix);
        self.dirty = true;
    }

    pub fn play_motion(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let motion = load_motion(path.as_ref())
            .map_err(|e| anyhow!("Failed to load motion file {}: {e}", path.as_ref().display()))?;
        self.motion_player = Some(MotionPlayer::new_once(motion));
        self.dirty = true;
        Ok(())
    }

    pub fn play_expression(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let expression = load_expression(path.as_ref())
            .map_err(|e| anyhow!("Failed to load expression file {}: {e}", path.as_ref().display()))?;
        self.expression_manager.play(expression);
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
        let matrix = fit_model_matrix_with_scale(self.model_bounds, self.width, self.height, self.scale);
        let clip_x = (cursor_x as f32 / self.width as f32) * 2.0 - 1.0;
        let clip_y = 1.0 - (cursor_y as f32 / self.height as f32) * 2.0;
        let model_x = matrix.invert_transform_x(clip_x);
        let model_y = matrix.invert_transform_y(clip_y);

        self.runtime.hit_test(model_x, model_y).map(|area| area.name().to_string())
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
        let delta = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;

        let animating = self.motion_player.is_some()
            || self.expression_manager.active_expression_count() > 0
            || self.runtime.physics().is_some();

        if self.dirty || animating {
            self.runtime.reset_parameters();
            self.runtime.reset_part_opacities();

            if let Some(player) = self.motion_player.as_mut() {
                player.tick(delta);
                player.apply(&mut self.runtime);
                if player.is_finished() {
                    self.motion_player = None;
                }
            }

            self.expression_manager.tick(delta);
            self.expression_manager.apply(&mut self.runtime);
            self.runtime.apply_parameter_overrides();
            self.runtime.apply_physics(delta);
            self.runtime.apply_pose(delta);

            if self.runtime.update_meshes().is_none() {
                return Err(anyhow!("Failed to update model meshes"));
            }

            // Update mesh buffers on GPU
            let update_res = self.mesh_buffers.update_drawables(&self.queue, self.runtime.meshes());
            match update_res {
                Ok(update) => {
                    if update.bounds_changed() || update.visibility_changed() {
                        self.update_clipping()?;
                    }
                }
                Err(_) => {
                    let new_buffers = WgpuMeshBuffers::from_drawables(&self.device, self.runtime.meshes())
                        .ok_or_else(|| anyhow!("Failed to re-create mesh buffers"))?;
                    self.mesh_buffers = new_buffers;
                    self.rebuild_clipping()?;
                }
            }

            self.dirty = false;
        }

        // Render passes
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("murasame.render_pass.encoder"),
            });

        // 1. Mask pass
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

        // 2. Main pass onto target texture
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

        // Zero-copy conversion from wgpu::Texture into slint::Image
        let slint_image = slint::Image::try_from(self.output_texture.clone())
            .map_err(|e| anyhow!("Failed to convert wgpu texture to Slint Image: {e}"))?;

        Ok(slint_image)
    }
}
