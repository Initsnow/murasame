//! Raw FFI bindings for Live2D Cubism Core C API.
//!
//! Based on Live2D Cubism SDK for Native (CubismSdkForNative-5-r.5).

use std::ffi::{c_char, c_int, c_uint, c_void};

pub const CSM_ALIGNOF_MOC: usize = 64;
pub const CSM_ALIGNOF_MODEL: usize = 16;

pub const CSM_BLEND_ADDITIVE: u8 = 1 << 0;
pub const CSM_BLEND_MULTIPLICATIVE: u8 = 1 << 1;
pub const CSM_IS_DOUBLE_SIDED: u8 = 1 << 2;
pub const CSM_IS_INVERTED_MASK: u8 = 1 << 3;

pub const CSM_IS_VISIBLE: u8 = 1 << 0;
pub const CSM_VISIBILITY_DID_CHANGE: u8 = 1 << 1;
pub const CSM_OPACITY_DID_CHANGE: u8 = 1 << 2;
pub const CSM_DRAW_ORDER_DID_CHANGE: u8 = 1 << 3;
pub const CSM_RENDER_ORDER_DID_CHANGE: u8 = 1 << 4;
pub const CSM_VERTEX_POSITIONS_DID_CHANGE: u8 = 1 << 5;
pub const CSM_BLEND_COLOR_DID_CHANGE: u8 = 1 << 6;

pub type CsmVersion = c_uint;
pub type CsmMocVersion = c_uint;
pub type CsmFlags = u8;
pub type CsmParameterType = c_int;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct CsmVector2 {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct CsmVector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[repr(C)]
pub struct CsmMoc {
    _private: [u8; 0],
}

#[repr(C)]
pub struct CsmModel {
    _private: [u8; 0],
}

pub type CsmLogFunction = Option<unsafe extern "C" fn(message: *const c_char)>;

unsafe extern "C" {
    pub fn csmGetVersion() -> CsmVersion;
    pub fn csmGetLatestMocVersion() -> CsmMocVersion;
    pub fn csmGetMocVersion(address: *const c_void, size: c_uint) -> CsmMocVersion;
    pub fn csmHasMocConsistency(address: *mut c_void, size: c_uint) -> c_int;
    pub fn csmGetLogFunction() -> CsmLogFunction;
    pub fn csmSetLogFunction(handler: CsmLogFunction);

    pub fn csmReviveMocInPlace(address: *mut c_void, size: c_uint) -> *mut CsmMoc;
    pub fn csmGetSizeofModel(moc: *const CsmMoc) -> c_uint;
    pub fn csmInitializeModelInPlace(
        moc: *const CsmMoc,
        address: *mut c_void,
        size: c_uint,
    ) -> *mut CsmModel;
    pub fn csmUpdateModel(model: *mut CsmModel);
    pub fn csmGetRenderOrders(model: *const CsmModel) -> *const c_int;

    pub fn csmReadCanvasInfo(
        model: *const CsmModel,
        out_size_in_pixels: *mut CsmVector2,
        out_origin_in_pixels: *mut CsmVector2,
        out_pixels_per_unit: *mut f32,
    );

    pub fn csmGetParameterCount(model: *const CsmModel) -> c_int;
    pub fn csmGetParameterIds(model: *const CsmModel) -> *const *const c_char;
    pub fn csmGetParameterTypes(model: *const CsmModel) -> *const CsmParameterType;
    pub fn csmGetParameterMinimumValues(model: *const CsmModel) -> *const f32;
    pub fn csmGetParameterMaximumValues(model: *const CsmModel) -> *const f32;
    pub fn csmGetParameterDefaultValues(model: *const CsmModel) -> *const f32;
    pub fn csmGetParameterValues(model: *mut CsmModel) -> *mut f32;
    pub fn csmGetParameterRepeats(model: *const CsmModel) -> *const c_int;
    pub fn csmGetParameterKeyCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetParameterKeyValues(model: *const CsmModel) -> *const *const f32;

    pub fn csmGetPartCount(model: *const CsmModel) -> c_int;
    pub fn csmGetPartIds(model: *const CsmModel) -> *const *const c_char;
    pub fn csmGetPartOpacities(model: *mut CsmModel) -> *mut f32;
    pub fn csmGetPartParentPartIndices(model: *const CsmModel) -> *const c_int;
    pub fn csmGetPartOffscreenIndices(model: *const CsmModel) -> *const c_int;

    pub fn csmGetDrawableCount(model: *const CsmModel) -> c_int;
    pub fn csmGetDrawableIds(model: *const CsmModel) -> *const *const c_char;
    pub fn csmGetDrawableConstantFlags(model: *const CsmModel) -> *const CsmFlags;
    pub fn csmGetDrawableDynamicFlags(model: *const CsmModel) -> *const CsmFlags;
    pub fn csmGetDrawableTextureIndices(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableDrawOrders(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableOpacities(model: *const CsmModel) -> *const f32;
    pub fn csmGetDrawableMaskCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableMasks(model: *const CsmModel) -> *const *const c_int;
    pub fn csmGetDrawableVertexCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableVertexPositions(model: *const CsmModel) -> *const *const CsmVector2;
    pub fn csmGetDrawableVertexUvs(model: *const CsmModel) -> *const *const CsmVector2;
    pub fn csmGetDrawableIndexCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableIndices(model: *const CsmModel) -> *const *const u16;
    pub fn csmGetDrawableMultiplyColors(model: *const CsmModel) -> *const CsmVector4;
    pub fn csmGetDrawableScreenColors(model: *const CsmModel) -> *const CsmVector4;
    pub fn csmGetDrawableParentPartIndices(model: *const CsmModel) -> *const c_int;
    pub fn csmResetDrawableDynamicFlags(model: *mut CsmModel);
}
