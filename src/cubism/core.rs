//! High-level, memory-safe Rust wrapper around Live2D Cubism Core C API.

use std::{
    alloc::{Layout, alloc, dealloc},
    collections::HashMap,
    ffi::CStr,
    fs,
    path::Path,
    ptr::NonNull,
};

use anyhow::{Context, Result, anyhow, bail};
use mocari::moc3::{Moc3DrawableMesh, Moc3DrawableVertex};

use super::sys::*;

/// Memory buffer aligned to a specific boundary required by Cubism Core.
#[derive(Debug)]
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl AlignedBuffer {
    pub fn new(size: usize, align: usize) -> Result<Self> {
        let size = size.max(1);
        let layout = Layout::from_size_align(size, align)
            .map_err(|e| anyhow!("Invalid layout size={size}, align={align}: {e}"))?;
        let ptr = unsafe { alloc(layout) };
        let ptr = NonNull::new(ptr).ok_or_else(|| anyhow!("Failed to allocate aligned memory"))?;
        Ok(Self { ptr, layout })
    }

    pub fn from_slice(data: &[u8], align: usize) -> Result<Self> {
        let mut buf = Self::new(data.len(), align)?;
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), buf.as_mut_ptr(), data.len());
        }
        Ok(buf)
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    pub fn len(&self) -> usize {
        self.layout.size()
    }

    pub fn is_empty(&self) -> bool {
        self.layout.size() == 0
    }
}

unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

/// Canvas information queried from a model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasInfo {
    pub size_in_pixels: (f32, f32),
    pub origin_in_pixels: (f32, f32),
    pub pixels_per_unit: f32,
}

/// A parsed and revived Cubism Moc (.moc3).
pub struct CubismMoc {
    raw_moc: NonNull<CsmMoc>,
    buffer: AlignedBuffer,
    version: CsmMocVersion,
}

unsafe impl Send for CubismMoc {}
unsafe impl Sync for CubismMoc {}

impl CubismMoc {
    /// Loads and revives a `.moc3` file from disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let data = fs::read(path.as_ref())
            .with_context(|| format!("Failed to read moc3 file: {}", path.as_ref().display()))?;
        Self::from_bytes(&data)
    }

    /// Revives a `.moc3` file from memory bytes into 64-byte aligned memory.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            bail!("Moc data is empty");
        }

        let version = unsafe { csmGetMocVersion(data.as_ptr() as *const _, data.len() as u32) };
        if version == 0 {
            bail!("Invalid moc version or unsupported format");
        }

        let mut buffer = AlignedBuffer::from_slice(data, CSM_ALIGNOF_MOC)?;

        let has_consistency =
            unsafe { csmHasMocConsistency(buffer.as_mut_ptr() as *mut _, buffer.len() as u32) };
        if has_consistency == 0 {
            bail!("Moc consistency check failed");
        }

        let moc_ptr =
            unsafe { csmReviveMocInPlace(buffer.as_mut_ptr() as *mut _, buffer.len() as u32) };
        let raw_moc = NonNull::new(moc_ptr).ok_or_else(|| anyhow!("csmReviveMocInPlace returned null"))?;

        Ok(Self {
            raw_moc,
            buffer,
            version,
        })
    }

    pub fn version(&self) -> CsmMocVersion {
        self.version
    }

    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    pub fn as_raw(&self) -> *const CsmMoc {
        self.raw_moc.as_ptr()
    }
}

/// An instantiated, runnable Cubism Model driven by Cubism Core.
pub struct CubismModel {
    raw_model: NonNull<CsmModel>,
    model_buffer: AlignedBuffer,
    canvas_info: CanvasInfo,

    parameter_count: usize,
    parameter_id_strings: Vec<String>,
    parameter_indices: HashMap<String, usize>,

    part_count: usize,
    part_id_strings: Vec<String>,
    part_indices: HashMap<String, usize>,

    drawable_count: usize,
    drawable_id_strings: Vec<String>,
    drawable_indices: HashMap<String, usize>,
}

unsafe impl Send for CubismModel {}
unsafe impl Sync for CubismModel {}

impl CubismModel {
    /// Instantiates a new model instance from a revived moc.
    pub fn new(moc: &CubismMoc) -> Result<Self> {
        let size = unsafe { csmGetSizeofModel(moc.as_raw()) } as usize;
        if size == 0 {
            bail!("Failed to get model size from moc: csmGetSizeofModel returned 0");
        }

        let mut model_buffer = AlignedBuffer::new(size, CSM_ALIGNOF_MODEL)?;

        let model_ptr = unsafe {
            csmInitializeModelInPlace(
                moc.as_raw(),
                model_buffer.as_mut_ptr() as *mut _,
                size as u32,
            )
        };
        let raw_model = NonNull::new(model_ptr)
            .ok_or_else(|| anyhow!("csmInitializeModelInPlace returned null"))?;

        // Query canvas info
        let mut size_in_pixels = CsmVector2::default();
        let mut origin_in_pixels = CsmVector2::default();
        let mut pixels_per_unit = 0.0f32;
        unsafe {
            csmReadCanvasInfo(
                raw_model.as_ptr(),
                &mut size_in_pixels,
                &mut origin_in_pixels,
                &mut pixels_per_unit,
            );
        }
        let canvas_info = CanvasInfo {
            size_in_pixels: (size_in_pixels.x, size_in_pixels.y),
            origin_in_pixels: (origin_in_pixels.x, origin_in_pixels.y),
            pixels_per_unit,
        };

        // Cache parameter IDs
        let param_count_raw = unsafe { csmGetParameterCount(raw_model.as_ptr()) };
        let parameter_count = param_count_raw.max(0) as usize;
        let mut parameter_id_strings = Vec::with_capacity(parameter_count);
        let mut parameter_indices = HashMap::with_capacity(parameter_count);

        if parameter_count > 0 {
            let ids_ptr = unsafe { csmGetParameterIds(raw_model.as_ptr()) };
            if !ids_ptr.is_null() {
                for i in 0..parameter_count {
                    let id_ptr = unsafe { *ids_ptr.add(i) };
                    let id_str = if !id_ptr.is_null() {
                        unsafe { CStr::from_ptr(id_ptr).to_string_lossy().into_owned() }
                    } else {
                        format!("Param_{i}")
                    };
                    parameter_indices.insert(id_str.clone(), i);
                    parameter_id_strings.push(id_str);
                }
            }
        }

        // Cache part IDs
        let part_count_raw = unsafe { csmGetPartCount(raw_model.as_ptr()) };
        let part_count = part_count_raw.max(0) as usize;
        let mut part_id_strings = Vec::with_capacity(part_count);
        let mut part_indices = HashMap::with_capacity(part_count);

        if part_count > 0 {
            let ids_ptr = unsafe { csmGetPartIds(raw_model.as_ptr()) };
            if !ids_ptr.is_null() {
                for i in 0..part_count {
                    let id_ptr = unsafe { *ids_ptr.add(i) };
                    let id_str = if !id_ptr.is_null() {
                        unsafe { CStr::from_ptr(id_ptr).to_string_lossy().into_owned() }
                    } else {
                        format!("Part_{i}")
                    };
                    part_indices.insert(id_str.clone(), i);
                    part_id_strings.push(id_str);
                }
            }
        }

        // Cache drawable IDs
        let draw_count_raw = unsafe { csmGetDrawableCount(raw_model.as_ptr()) };
        let drawable_count = draw_count_raw.max(0) as usize;
        let mut drawable_id_strings = Vec::with_capacity(drawable_count);
        let mut drawable_indices = HashMap::with_capacity(drawable_count);

        if drawable_count > 0 {
            let ids_ptr = unsafe { csmGetDrawableIds(raw_model.as_ptr()) };
            if !ids_ptr.is_null() {
                for i in 0..drawable_count {
                    let id_ptr = unsafe { *ids_ptr.add(i) };
                    let id_str = if !id_ptr.is_null() {
                        unsafe { CStr::from_ptr(id_ptr).to_string_lossy().into_owned() }
                    } else {
                        format!("ArtMesh_{i}")
                    };
                    drawable_indices.insert(id_str.clone(), i);
                    drawable_id_strings.push(id_str);
                }
            }
        }

        let mut model = Self {
            raw_model,
            model_buffer,
            canvas_info,
            parameter_count,
            parameter_id_strings,
            parameter_indices,
            part_count,
            part_id_strings,
            part_indices,
            drawable_count,
            drawable_id_strings,
            drawable_indices,
        };

        // Initial update
        model.update();

        Ok(model)
    }

    /// Triggers model update (evaluates deformers, vertices, draw orders).
    #[inline]
    pub fn update(&mut self) {
        unsafe {
            csmUpdateModel(self.raw_model.as_ptr());
        }
    }

    pub fn canvas_info(&self) -> CanvasInfo {
        self.canvas_info
    }

    pub fn buffer_len(&self) -> usize {
        self.model_buffer.len()
    }

    pub fn parameter_count(&self) -> usize {
        self.parameter_count
    }

    pub fn parameter_ids(&self) -> &[String] {
        &self.parameter_id_strings
    }

    pub fn parameter_index(&self, id: &str) -> Option<usize> {
        self.parameter_indices.get(id).copied()
    }

    pub fn parameter_values(&self) -> &[f32] {
        unsafe {
            let ptr = csmGetParameterValues(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.parameter_count)
            }
        }
    }

    pub fn parameter_values_mut(&mut self) -> &mut [f32] {
        unsafe {
            let ptr = csmGetParameterValues(self.raw_model.as_ptr());
            if ptr.is_null() {
                &mut []
            } else {
                std::slice::from_raw_parts_mut(ptr, self.parameter_count)
            }
        }
    }

    pub fn parameter_minimum_values(&self) -> &[f32] {
        unsafe {
            let ptr = csmGetParameterMinimumValues(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.parameter_count)
            }
        }
    }

    pub fn parameter_maximum_values(&self) -> &[f32] {
        unsafe {
            let ptr = csmGetParameterMaximumValues(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.parameter_count)
            }
        }
    }

    pub fn parameter_default_values(&self) -> &[f32] {
        unsafe {
            let ptr = csmGetParameterDefaultValues(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.parameter_count)
            }
        }
    }

    pub fn set_parameter_value(&mut self, id: &str, value: f32) -> bool {
        if let Some(idx) = self.parameter_index(id) {
            let values = self.parameter_values_mut();
            if idx < values.len() {
                values[idx] = value;
                return true;
            }
        }
        false
    }

    pub fn part_count(&self) -> usize {
        self.part_count
    }

    pub fn part_ids(&self) -> &[String] {
        &self.part_id_strings
    }

    pub fn part_index(&self, id: &str) -> Option<usize> {
        self.part_indices.get(id).copied()
    }

    pub fn part_opacities(&self) -> &[f32] {
        unsafe {
            let ptr = csmGetPartOpacities(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.part_count)
            }
        }
    }

    pub fn part_opacities_mut(&mut self) -> &mut [f32] {
        unsafe {
            let ptr = csmGetPartOpacities(self.raw_model.as_ptr());
            if ptr.is_null() {
                &mut []
            } else {
                std::slice::from_raw_parts_mut(ptr, self.part_count)
            }
        }
    }

    pub fn drawable_count(&self) -> usize {
        self.drawable_count
    }

    pub fn drawable_ids(&self) -> &[String] {
        &self.drawable_id_strings
    }

    pub fn drawable_index(&self, id: &str) -> Option<usize> {
        self.drawable_indices.get(id).copied()
    }

    pub fn drawable_vertex_counts(&self) -> &[i32] {
        unsafe {
            let ptr = csmGetDrawableVertexCounts(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn drawable_vertex_positions(&self, index: usize) -> Option<&[CsmVector2]> {
        if index >= self.drawable_count {
            return None;
        }
        let count = self.drawable_vertex_counts().get(index).copied()? as usize;
        unsafe {
            let positions_ptr = csmGetDrawableVertexPositions(self.raw_model.as_ptr());
            if positions_ptr.is_null() {
                return None;
            }
            let ptr = *positions_ptr.add(index);
            if ptr.is_null() {
                None
            } else {
                Some(std::slice::from_raw_parts(ptr, count))
            }
        }
    }

    pub fn drawable_vertex_uvs(&self, index: usize) -> Option<&[CsmVector2]> {
        if index >= self.drawable_count {
            return None;
        }
        let count = self.drawable_vertex_counts().get(index).copied()? as usize;
        unsafe {
            let uvs_ptr = csmGetDrawableVertexUvs(self.raw_model.as_ptr());
            if uvs_ptr.is_null() {
                return None;
            }
            let ptr = *uvs_ptr.add(index);
            if ptr.is_null() {
                None
            } else {
                Some(std::slice::from_raw_parts(ptr, count))
            }
        }
    }

    pub fn drawable_index_counts(&self) -> &[i32] {
        unsafe {
            let ptr = csmGetDrawableIndexCounts(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn drawable_indices(&self, index: usize) -> Option<&[u16]> {
        if index >= self.drawable_count {
            return None;
        }
        let count = self.drawable_index_counts().get(index).copied()? as usize;
        unsafe {
            let indices_ptr = csmGetDrawableIndices(self.raw_model.as_ptr());
            if indices_ptr.is_null() {
                return None;
            }
            let ptr = *indices_ptr.add(index);
            if ptr.is_null() {
                None
            } else {
                Some(std::slice::from_raw_parts(ptr, count))
            }
        }
    }

    pub fn drawable_opacities(&self) -> &[f32] {
        unsafe {
            let ptr = csmGetDrawableOpacities(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn render_orders(&self) -> &[i32] {
        unsafe {
            let ptr = csmGetRenderOrders(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn drawable_constant_flags(&self) -> &[u8] {
        unsafe {
            let ptr = csmGetDrawableConstantFlags(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn drawable_dynamic_flags(&self) -> &[u8] {
        unsafe {
            let ptr = csmGetDrawableDynamicFlags(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn drawable_texture_indices(&self) -> &[i32] {
        unsafe {
            let ptr = csmGetDrawableTextureIndices(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn drawable_draw_orders(&self) -> &[i32] {
        unsafe {
            let ptr = csmGetDrawableDrawOrders(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn drawable_masks(&self, index: usize) -> &[i32] {
        if index >= self.drawable_count {
            return &[];
        }
        unsafe {
            let counts_ptr = csmGetDrawableMaskCounts(self.raw_model.as_ptr());
            let masks_ptr = csmGetDrawableMasks(self.raw_model.as_ptr());
            if counts_ptr.is_null() || masks_ptr.is_null() {
                return &[];
            }
            let count = *counts_ptr.add(index) as usize;
            let ptr = *masks_ptr.add(index);
            if ptr.is_null() || count == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, count)
            }
        }
    }

    pub fn drawable_multiply_colors(&self) -> &[CsmVector4] {
        unsafe {
            let ptr = csmGetDrawableMultiplyColors(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    pub fn drawable_screen_colors(&self) -> &[CsmVector4] {
        unsafe {
            let ptr = csmGetDrawableScreenColors(self.raw_model.as_ptr());
            if ptr.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(ptr, self.drawable_count)
            }
        }
    }

    /// Resets all parameter values to their default values declared by the moc.
    pub fn reset_parameters(&mut self) {
        unsafe {
            let defaults_ptr = csmGetParameterDefaultValues(self.raw_model.as_ptr());
            let values_ptr = csmGetParameterValues(self.raw_model.as_ptr());
            if !defaults_ptr.is_null() && !values_ptr.is_null() && self.parameter_count > 0 {
                std::ptr::copy_nonoverlapping(defaults_ptr, values_ptr, self.parameter_count);
            }
        }
    }

    /// Resets all part opacities to 1.0.
    pub fn reset_part_opacities(&mut self) {
        let opacities = self.part_opacities_mut();
        for op in opacities.iter_mut() {
            *op = 1.0;
        }
    }

    /// Evaluates physics simulation using separate parameter buffers directly from C API.
    pub fn evaluate_physics(&mut self, physics: &mut mocari::core::PhysicsRuntime, delta: f32) {
        let (values, mins, maxs, defs) = unsafe {
            let model_ptr = self.raw_model.as_ptr();
            let count = self.parameter_count;
            let v_ptr = csmGetParameterValues(model_ptr);
            let min_ptr = csmGetParameterMinimumValues(model_ptr);
            let max_ptr = csmGetParameterMaximumValues(model_ptr);
            let def_ptr = csmGetParameterDefaultValues(model_ptr);
            if v_ptr.is_null() || min_ptr.is_null() || max_ptr.is_null() || def_ptr.is_null() || count == 0 {
                return;
            }
            (
                std::slice::from_raw_parts_mut(v_ptr, count),
                std::slice::from_raw_parts(min_ptr, count),
                std::slice::from_raw_parts(max_ptr, count),
                std::slice::from_raw_parts(def_ptr, count),
            )
        };
        physics.evaluate(values, mins, maxs, defs, delta);
    }

    /// Builds a full set of `Moc3DrawableMesh` structures representing the current model state.
    pub fn to_drawable_meshes(&self) -> Vec<Moc3DrawableMesh> {
        let mut meshes = Vec::with_capacity(self.drawable_count);
        let constant_flags = self.drawable_constant_flags();
        let dynamic_flags = self.drawable_dynamic_flags();
        let opacities = self.drawable_opacities();
        let texture_indices = self.drawable_texture_indices();
        let draw_orders = self.drawable_draw_orders();
        let render_orders = self.render_orders();
        let mult_colors = self.drawable_multiply_colors();
        let screen_colors = self.drawable_screen_colors();

        for i in 0..self.drawable_count {
            let tex_idx = texture_indices.get(i).copied().unwrap_or(0);
            let flags = constant_flags.get(i).copied().unwrap_or(0);
            let is_visible = dynamic_flags
                .get(i)
                .map(|&f| (f & CSM_IS_VISIBLE) != 0)
                .unwrap_or(true);
            let op = if is_visible {
                opacities.get(i).copied().unwrap_or(1.0)
            } else {
                0.0
            };
            let draw_order = draw_orders.get(i).copied().unwrap_or(0) as f32;
            let render_order = render_orders.get(i).copied().unwrap_or(i as i32);

            let mult = mult_colors
                .get(i)
                .map(|c| [c.x, c.y, c.z])
                .unwrap_or([1.0, 1.0, 1.0]);
            let screen = screen_colors
                .get(i)
                .map(|c| [c.x, c.y, c.z])
                .unwrap_or([0.0, 0.0, 0.0]);

            let pos = self.drawable_vertex_positions(i).unwrap_or(&[]);
            let uvs = self.drawable_vertex_uvs(i).unwrap_or(&[]);
            let indices = self.drawable_indices(i).unwrap_or(&[]).to_vec();
            let masks = self.drawable_masks(i).to_vec();

            let vertices: Vec<Moc3DrawableVertex> = pos
                .iter()
                .zip(uvs.iter())
                .map(|(p, u)| Moc3DrawableVertex::new([p.x, p.y], [u.x, 1.0 - u.y]))
                .collect();

            let mut mesh = Moc3DrawableMesh::from_parts_with_render_order(
                tex_idx,
                flags,
                op,
                draw_order,
                render_order,
                vertices,
                indices,
                masks,
            );
            mesh.set_multiply_color(mult);
            mesh.set_screen_color(screen);
            meshes.push(mesh);
        }
        meshes
    }

    /// Updates existing `Moc3DrawableMesh` structures in-place from current model deformation.
    pub fn update_drawable_meshes(&self, meshes: &mut [Moc3DrawableMesh]) {
        let model_ptr = self.raw_model.as_ptr();
        let dynamic_flags = self.drawable_dynamic_flags();
        let opacities = self.drawable_opacities();
        let render_orders = self.render_orders();
        let draw_orders = self.drawable_draw_orders();
        let mult_colors = self.drawable_multiply_colors();
        let screen_colors = self.drawable_screen_colors();

        let (counts_ptr, positions_ptr, uvs_ptr) = unsafe {
            (
                csmGetDrawableVertexCounts(model_ptr),
                csmGetDrawableVertexPositions(model_ptr),
                csmGetDrawableVertexUvs(model_ptr),
            )
        };

        for (i, mesh) in meshes.iter_mut().enumerate().take(self.drawable_count) {
            let flag = dynamic_flags.get(i).copied().unwrap_or(CSM_IS_VISIBLE);

            // Fast-path: If no dynamic attributes changed for this mesh (bits 1..6 == 0), skip entirely!
            if (flag & 0x7E) == 0 {
                continue;
            }

            // 1. Visibility & Opacity
            let is_visible = (flag & CSM_IS_VISIBLE) != 0;
            let op = if is_visible {
                opacities.get(i).copied().unwrap_or(1.0)
            } else {
                0.0
            };
            mesh.set_opacity(op);

            // 2. Render Order
            if let Some(&ro) = render_orders.get(i) {
                mesh.set_render_order(ro);
            }

            // 3. Draw Order
            if let Some(&do_) = draw_orders.get(i) {
                mesh.set_draw_order(do_ as f32);
            }

            // 4. Multiply & Screen Colors
            if (flag & CSM_BLEND_COLOR_DID_CHANGE) != 0 {
                if let Some(m) = mult_colors.get(i) {
                    mesh.set_multiply_color([m.x, m.y, m.z]);
                }
                if let Some(s) = screen_colors.get(i) {
                    mesh.set_screen_color([s.x, s.y, s.z]);
                }
            }

            // 5. Deformed Vertex Positions: only re-evaluate if vertex positions actually deformed
            if (flag & CSM_VERTEX_POSITIONS_DID_CHANGE) != 0 {
                if !positions_ptr.is_null() && !counts_ptr.is_null() && !uvs_ptr.is_null() {
                    unsafe {
                        let count = *counts_ptr.add(i) as usize;
                        let pos_p = *positions_ptr.add(i);
                        let uv_p = *uvs_ptr.add(i);
                        if !pos_p.is_null() && !uv_p.is_null() && count > 0 {
                            let pos = std::slice::from_raw_parts(pos_p, count);
                            let uvs = std::slice::from_raw_parts(uv_p, count);
                            let verts = mesh.vertices_mut();
                            for (v, (p, u)) in verts.iter_mut().zip(pos.iter().zip(uvs.iter())) {
                                *v = Moc3DrawableVertex::new([p.x, p.y], [u.x, 1.0 - u.y]);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn reset_dynamic_flags(&mut self) {
        unsafe {
            csmResetDrawableDynamicFlags(self.raw_model.as_ptr());
        }
    }
}

/// Queries Cubism Core version string.
pub fn core_version() -> (u32, u32, u32) {
    let ver = unsafe { csmGetVersion() };
    let major = (ver >> 24) & 0xff;
    let minor = (ver >> 16) & 0xff;
    let patch = ver & 0xffff;
    (major, minor, patch)
}

/// Queries latest supported Moc version.
pub fn latest_moc_version() -> u32 {
    unsafe { csmGetLatestMocVersion() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_cubism_core_version() {
        let (major, minor, patch) = core_version();
        println!("Live2D Cubism Core Version: {major:02}.{minor:02}.{patch:04}");
        assert!(major >= 5, "Expected Cubism Core 5 or higher, got {major}");
        let latest = latest_moc_version();
        assert!(latest >= 5, "Expected Moc version 5 or higher, got {latest}");
    }

    #[test]
    fn test_cubism_core_load_murasame() {
        let moc_path = PathBuf::from("Murasame_live2d").join("Murasame.moc3");
        if !moc_path.exists() {
            eprintln!("Warning: Murasame.moc3 not found at {:?}", moc_path);
            return;
        }

        let moc = CubismMoc::from_file(&moc_path).expect("Failed to load and revive Murasame.moc3");
        assert!(moc.buffer_len() > 0);

        let mut model = CubismModel::new(&moc).expect("Failed to instantiate CubismModel");
        assert!(model.parameter_count() > 0, "Model should have parameters");
        assert!(model.drawable_count() > 0, "Model should have drawables");

        // Verify parameter lookup and modification
        if let Some(idx) = model.parameter_index("ParamAngleX") {
            let initial = model.parameter_values()[idx];
            model.parameter_values_mut()[idx] = 15.0;
            model.update();
            assert_eq!(model.parameter_values()[idx], 15.0);
            assert_ne!(initial, 15.0);
        }

        // Verify drawable mesh extraction
        let mut found_valid_mesh = false;
        for i in 0..model.drawable_count() {
            if let (Some(verts), Some(indices)) = (
                model.drawable_vertex_positions(i),
                model.drawable_indices(i),
            ) {
                if !verts.is_empty() && !indices.is_empty() {
                    found_valid_mesh = true;
                    break;
                }
            }
        }
        assert!(found_valid_mesh, "Should have found valid drawable mesh vertices and indices");
    }

    #[test]
    fn test_compare_cubism_and_mocari_vertices() {
        let moc_path = PathBuf::from("Murasame_live2d").join("Murasame.moc3");
        let model_json_path = PathBuf::from("Murasame_live2d").join("Murasame.model3.json");
        if !moc_path.exists() || !model_json_path.exists() {
            return;
        }

        let moc = CubismMoc::from_file(&moc_path).unwrap();
        let mut cubism_model = CubismModel::new(&moc).unwrap();
        cubism_model.update();

        let mocari_loaded = mocari::assets::load_model_runtime(&model_json_path).unwrap();
        let mocari_runtime = mocari_loaded.runtime();

        println!("Cubism drawables: {}, Mocari meshes: {}", cubism_model.drawable_count(), mocari_runtime.meshes().len());

        for i in 0..5.min(cubism_model.drawable_count()) {
            let cubism_verts = cubism_model.drawable_vertex_positions(i).unwrap();
            let mocari_mesh = &mocari_runtime.meshes()[i];
            let mocari_verts = mocari_mesh.vertices();
            println!("Mesh {i}:");
            if let (Some(cv), Some(mv)) = (cubism_verts.first(), mocari_verts.first()) {
                println!("  Cubism first vert: ({:.4}, {:.4})", cv.x, cv.y);
                println!("  Mocari first vert: ({:.4}, {:.4})", mv.position()[0], mv.position()[1]);
                assert!((cv.x - mv.position()[0]).abs() < 1e-4);
                assert!((cv.y - mv.position()[1]).abs() < 1e-4);
            }
            if let (Some(cuv), Some(mv)) = (cubism_model.drawable_vertex_uvs(i).and_then(|u| u.first()), mocari_verts.first()) {
                println!("  Cubism first uv: ({:.4}, {:.4})", cuv.x, cuv.y);
                println!("  Mocari first uv: ({:.4}, {:.4})", mv.uv()[0], mv.uv()[1]);
                assert!((cuv.x - mv.uv()[0]).abs() < 1e-4);
                assert!(((1.0 - cuv.y) - mv.uv()[1]).abs() < 1e-4);
            }
        }

        // Test to_drawable_meshes and update_drawable_meshes
        let mut meshes = cubism_model.to_drawable_meshes();
        assert_eq!(meshes.len(), cubism_model.drawable_count());
        assert!(!meshes[0].vertices().is_empty());
        assert!(!meshes[0].indices().is_empty());

        // Modify parameter and update
        if let Some(idx) = cubism_model.parameter_index("ParamAngleX") {
            cubism_model.parameter_values_mut()[idx] = 20.0;
            cubism_model.update();
            cubism_model.update_drawable_meshes(&mut meshes);
            assert_eq!(meshes.len(), cubism_model.drawable_count());
        }
    }
}
