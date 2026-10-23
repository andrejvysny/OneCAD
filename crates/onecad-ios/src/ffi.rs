//! `#[repr(C)]` mirrors of `include/onecad_ios.h` (ABI version 2). Field order,
//! types and constants must match the header exactly; `tests` pins the sizes
//! clang reports for the header on arm64.
// A complete mirror: some constants are only produced or read by the host.
#![allow(dead_code)]
use std::ffi::{c_char, c_void};

pub const ABI_VERSION: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    InvalidArgument = 1,
    WrongThread = 2,
    State = 3,
    Panic = 4,
    Terminal = 5,
    Overflow = 6,
    Internal = 7,
    Unavailable = 8,
}

pub type WakeFn = extern "C" fn(ctx: *mut c_void);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Config {
    pub struct_size: u32,
    pub documents_dir: *const c_char,
    pub app_support_dir: *const c_char,
    pub caches_dir: *const c_char,
    pub tmp_dir: *const c_char,
    pub wake: Option<WakeFn>,
    pub wake_ctx: *mut c_void,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SurfaceDesc {
    pub struct_size: u32,
    pub ca_metal_layer: *mut c_void,
    pub width_px: u32,
    pub height_px: u32,
    pub scale: f32,
    pub safe_area: Insets,
}

pub const FRAME_REDRAW_REQUESTED: u32 = 1 << 0;
pub const FRAME_TEXT_INPUT_ACTIVE: u32 = 1 << 1;
pub const FRAME_A11Y_CHANGED: u32 = 1 << 2;
pub const FRAME_HOST_REQUESTS: u32 = 1 << 3;
pub const FRAME_KERNEL_TERMINAL: u32 = 1 << 4;
pub const FRAME_PRESENTED: u32 = 1 << 5;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameResult {
    pub struct_size: u32,
    pub flags: u32,
    pub repaint_after_s: f64,
    pub text_cursor_x: f32,
    pub text_cursor_y: f32,
    pub text_cursor_h: f32,
    pub text_input_kind: u32,
}

pub const CONTACT_FINGER: u32 = 0;
pub const CONTACT_PENCIL: u32 = 1;
pub const CONTACT_INDIRECT_POINTER: u32 = 2;

pub const PHASE_BEGAN: u32 = 0;
pub const PHASE_MOVED: u32 = 1;
pub const PHASE_STATIONARY: u32 = 2;
pub const PHASE_ENDED: u32 = 3;
pub const PHASE_CANCELLED: u32 = 4;
pub const PHASE_HOVER_MOVED: u32 = 5;
pub const PHASE_HOVER_EXITED: u32 = 6;

pub const SAMPLE_ACTUAL: u32 = 0;
pub const SAMPLE_COALESCED: u32 = 1;
pub const SAMPLE_PREDICTED: u32 = 2;
pub const SAMPLE_ESTIMATION_UPDATE: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TouchSample {
    pub contact_id: u64,
    pub kind: u32,
    pub phase: u32,
    pub provenance: u32,
    pub present: u32,
    pub estimated: u32,
    pub expects_update: u32,
    pub timestamp_s: f64,
    pub x_pt: f64,
    pub y_pt: f64,
    pub force: f64,
    pub max_force: f64,
    pub altitude_rad: f64,
    pub azimuth_rad: f64,
    pub roll_rad: f64,
    pub hover_z: f64,
    pub estimation_index: u64,
    pub tap_count: u32,
    pub reserved: u32,
}

pub const HAS_LOCATION: u32 = 1 << 6;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InputBatch {
    pub struct_size: u32,
    pub count: u32,
    pub samples: *const TouchSample,
    pub view_generation: u64,
}

pub const CANCEL_TEXT_FOCUS: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Navigation {
    pub struct_size: u32,
    pub phase: u32,
    pub anchor_x_pt: f64,
    pub anchor_y_pt: f64,
    pub pan_dx_pt: f64,
    pub pan_dy_pt: f64,
    pub scale_delta: f64,
    pub rotation_delta_rad: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PencilEvent {
    pub struct_size: u32,
    pub action: u32,
    pub preferred: u32,
    pub phase: u32,
    pub present: u32,
    pub x_pt: f64,
    pub y_pt: f64,
    pub hover_z: f64,
    pub altitude_rad: f64,
    pub azimuth_rad: f64,
    pub roll_rad: f64,
    pub timestamp_s: f64,
}

pub const CAP_PENCIL_CONNECTED: u32 = 1 << 0;
pub const CAP_HOVER: u32 = 1 << 1;
pub const CAP_BARREL_TAP: u32 = 1 << 3;
pub const CAP_SQUEEZE: u32 = 1 << 4;
pub const CAP_HAPTICS: u32 = 1 << 6;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Capabilities {
    pub struct_size: u32,
    pub flags: u32,
    pub prefers_pencil_only_drawing: u32,
    pub preferred_tap_action: u32,
    pub preferred_squeeze_action: u32,
}

pub const TEXT_GENERAL: u32 = 0;
pub const TEXT_NUMERIC_EXPRESSION: u32 = 1;

pub const MOD_SHIFT: u32 = 1 << 0;
pub const MOD_CONTROL: u32 = 1 << 1;
pub const MOD_OPTION: u32 = 1 << 2;
pub const MOD_COMMAND: u32 = 1 << 3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyEvent {
    pub struct_size: u32,
    pub hid_usage: u32,
    pub modifiers: u32,
    pub pressed: u32,
    pub repeat: u32,
}

pub const WILL_RESIGN_ACTIVE: u32 = 0;
pub const DID_ENTER_BACKGROUND: u32 = 1;
pub const WILL_ENTER_FOREGROUND: u32 = 2;
pub const DID_BECOME_ACTIVE: u32 = 3;
pub const MEMORY_WARNING: u32 = 4;
pub const WILL_TERMINATE: u32 = 5;
pub const TRAIT_DARK: u32 = 6;
pub const TRAIT_LIGHT: u32 = 7;

pub const ROLE_OTHER: u32 = 0;
pub const ROLE_BUTTON: u32 = 1;
pub const ROLE_TEXT_FIELD: u32 = 2;
pub const ROLE_STATIC_TEXT: u32 = 3;
pub const ROLE_TAB: u32 = 4;
pub const ROLE_CHECKBOX: u32 = 5;
pub const ROLE_IMAGE: u32 = 6;
pub const ROLE_CANVAS: u32 = 7;
pub const ROLE_MENU_ITEM: u32 = 8;
pub const ROLE_SLIDER: u32 = 9;
pub const ROLE_GROUP: u32 = 10;

pub const A11Y_DISABLED: u32 = 1 << 0;
pub const A11Y_SELECTED: u32 = 1 << 1;
pub const A11Y_FOCUSED: u32 = 1 << 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct A11yNode {
    pub id: u64,
    pub role: u32,
    pub flags: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub identifier: *const c_char,
    pub label: *const c_char,
    pub value: *const c_char,
}

pub const REQ_OPEN_DOCUMENT: u32 = 0;
pub const REQ_SAVE_DOCUMENT: u32 = 1;
pub const REQ_EXPORT_FILE: u32 = 2;
pub const REQ_HAPTIC: u32 = 3;
pub const REQ_COPY_TEXT: u32 = 4;
pub const REQ_OPEN_URL: u32 = 5;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostRequest {
    pub id: u64,
    pub kind: u32,
    pub multiple: u32,
    pub label: *const c_char,
    pub extensions: *const c_char,
    pub path: *const c_char,
    pub suggested_name: *const c_char,
    pub text: *const c_char,
}

/// Additive recovery snapshot; documents_json is borrowed until the next snapshot.
#[repr(C)]
pub struct RecoverySnapshot {
    pub struct_size: u32,
    pub status: u32,
    pub generation: u64,
    pub documents_json: *const c_char,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    /// Sizes and key offsets measured with clang on `onecad_ios.h` (arm64).
    #[test]
    fn layouts_match_the_c_header() {
        assert_eq!(size_of::<Status>(), 4);
        assert_eq!(
            (
                size_of::<RecoverySnapshot>(),
                offset_of!(RecoverySnapshot, generation)
            ),
            (24, 8)
        );
        assert_eq!(offset_of!(RecoverySnapshot, documents_json), 16);
        assert_eq!((size_of::<Config>(), offset_of!(Config, wake)), (64, 40));
        assert_eq!(size_of::<Insets>(), 16);
        assert_eq!(size_of::<SurfaceDesc>(), 48);
        assert_eq!(offset_of!(SurfaceDesc, safe_area), 28);
        assert_eq!(size_of::<FrameResult>(), 32);
        assert_eq!(offset_of!(FrameResult, repaint_after_s), 8);
        assert_eq!(offset_of!(FrameResult, text_input_kind), 28);
        assert_eq!(
            (size_of::<TouchSample>(), align_of::<TouchSample>()),
            (120, 8)
        );
        assert_eq!(offset_of!(TouchSample, timestamp_s), 32);
        assert_eq!(offset_of!(TouchSample, estimation_index), 104);
        assert_eq!(offset_of!(TouchSample, tap_count), 112);
        assert_eq!(size_of::<InputBatch>(), 24);
        assert_eq!(size_of::<Navigation>(), 56);
        assert_eq!(offset_of!(Navigation, anchor_x_pt), 8);
        assert_eq!(size_of::<PencilEvent>(), 80);
        assert_eq!(offset_of!(PencilEvent, x_pt), 24);
        assert_eq!(size_of::<Capabilities>(), 20);
        assert_eq!(size_of::<KeyEvent>(), 20);
        assert_eq!(size_of::<A11yNode>(), 56);
        assert_eq!(offset_of!(A11yNode, identifier), 32);
        assert_eq!(size_of::<HostRequest>(), 56);
        assert_eq!(offset_of!(HostRequest, label), 16);
    }
}
