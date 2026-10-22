//! ABI guards that need no app (creating one needs a Metal device and the
//! simulator harness): NULL and struct-size checks, `last_error`, and that a
//! rejected create leaves the one-app slot free.
use super::*;
use std::mem::MaybeUninit;

fn last_error() -> String {
    // SAFETY: `onecad_ios_last_error` never returns NULL and the text stays
    // valid until this thread's next failing call.
    unsafe { CStr::from_ptr(onecad_ios_last_error(std::ptr::null())) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_rejected_create_reports_why_and_frees_the_app_slot() {
    let mut out = std::ptr::null_mut();
    // SAFETY: NULL `out_app` is rejected before anything is read.
    let status = unsafe { onecad_ios_app_create(std::ptr::null(), std::ptr::null_mut()) };
    assert_eq!(status, Status::InvalidArgument);
    let small = 4u32;
    for _ in 0..2 {
        // SAFETY: the "config" is a readable `u32` that declares itself too small,
        // so nothing past it is read.
        let status = unsafe { onecad_ios_app_create((&raw const small).cast(), &mut out) };
        assert_eq!(
            status,
            Status::InvalidArgument,
            "not ERR_STATE: the slot was freed"
        );
        assert!(last_error().contains("struct_size 4"), "{}", last_error());
    }
    assert!(out.is_null());
}

#[test]
fn calls_without_an_app_are_invalid_arguments() {
    let null = std::ptr::null_mut();
    let mut result = MaybeUninit::<ffi::FrameResult>::uninit();
    // SAFETY: a NULL app is rejected before any other argument is touched.
    unsafe {
        assert_eq!(onecad_ios_app_destroy(null), Status::InvalidArgument);
        assert_eq!(
            onecad_ios_frame(null, 0.0, result.as_mut_ptr()),
            Status::InvalidArgument
        );
        assert_eq!(
            onecad_ios_lifecycle(null, ffi::WILL_TERMINATE),
            Status::InvalidArgument
        );
    }
    assert_eq!(last_error(), "app is NULL");
}

#[test]
fn out_parameters_need_only_their_size_initialized() {
    let mut result = MaybeUninit::<ffi::FrameResult>::uninit();
    let ptr = result.as_mut_ptr();
    // SAFETY: `ptr` is valid for a whole `FrameResult`; only its first field is
    // written, and `check_size` reads nothing else.
    unsafe {
        (&raw mut (*ptr).struct_size).write(std::mem::size_of::<ffi::FrameResult>() as u32);
        assert!(check_size(ptr.cast_const()).is_ok());
        (&raw mut (*ptr).struct_size).write(8);
        assert!(check_size(ptr.cast_const()).is_err());
    }
}
