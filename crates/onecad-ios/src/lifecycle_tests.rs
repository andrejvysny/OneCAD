//! Rendering policy across the surface lifecycle, without a GPU or kernel: what each
//! UIKit event does, which frames run, and that an idle editor asks for no frame.
use super::*;
use crate::frame::{View, admit, schedule};
use std::time::Duration;

fn view() -> View {
    View {
        size_px: [2064, 2752],
        scale: 2.0,
        safe_area: ffi::Insets::default(),
        keyboard_overlap: 0.0,
    }
}

#[test]
fn background_releases_targets_and_foreground_waits_for_the_next_frame() {
    let background = plan(Phase::Active, ffi::DID_ENTER_BACKGROUND).unwrap();
    assert_eq!(background.focus, Some(false));
    assert!(background.flush_autosave && background.release_targets);
    assert_eq!(background.phase, Some(Phase::Background));
    let foreground = plan(Phase::Background, ffi::WILL_ENTER_FOREGROUND).unwrap();
    assert_eq!(
        foreground,
        Plan {
            phase: Some(Phase::Inactive),
            ..Plan::default()
        },
        "targets are recreated by the first frame, not by the event"
    );
    let active = plan(Phase::Inactive, ffi::DID_BECOME_ACTIVE).unwrap();
    assert_eq!(active.focus, Some(true));
    assert!(!active.release_targets && !active.relieve_memory);
    let resign = plan(Phase::Active, ffi::WILL_RESIGN_ACTIVE).unwrap();
    assert!(!resign.release_targets, "inactive scenes keep drawing");
}

#[test]
fn memory_warnings_relieve_in_every_phase_and_nothing_else_does() {
    for phase in [
        Phase::Active,
        Phase::Inactive,
        Phase::Background,
        Phase::Terminated,
    ] {
        let warning = plan(phase, ffi::MEMORY_WARNING).unwrap();
        assert_eq!(
            warning,
            Plan {
                relieve_memory: true,
                // A release the GPU refused on entering the background is retried.
                release_targets: phase == Phase::Background,
                ..Plan::default()
            }
        );
    }
    for event in [
        ffi::WILL_RESIGN_ACTIVE,
        ffi::DID_ENTER_BACKGROUND,
        ffi::TRAIT_DARK,
    ] {
        let failure = plan(Phase::Terminated, event).unwrap_err();
        assert_eq!(failure.status, Status::State);
    }
    let unknown = plan(Phase::Active, 99).unwrap_err();
    assert_eq!(unknown.status, Status::InvalidArgument);
    let terminate = plan(Phase::Active, ffi::WILL_TERMINATE).unwrap();
    assert!(terminate.shutdown && !terminate.relieve_memory);
}

#[test]
fn frames_run_only_in_the_foreground_with_a_surface() {
    for phase in [Phase::Active, Phase::Inactive] {
        assert_eq!(
            admit(phase, Some(view()), 1.0).unwrap().size_px,
            [2064, 2752]
        );
        assert_eq!(admit(phase, None, 1.0).unwrap_err().status, Status::State);
    }
    for phase in [Phase::Background, Phase::Terminated] {
        assert_eq!(
            admit(phase, Some(view()), 1.0).unwrap_err().status,
            Status::State
        );
    }
    let nan = admit(Phase::Active, Some(view()), f64::NAN).unwrap_err();
    assert_eq!(nan.status, Status::InvalidArgument);
}

#[test]
fn a_zero_sized_view_is_refused_and_rotation_keeps_the_scale() {
    let desc = |width_px, height_px| ffi::SurfaceDesc {
        struct_size: std::mem::size_of::<ffi::SurfaceDesc>() as u32,
        ca_metal_layer: std::ptr::null_mut(),
        width_px,
        height_px,
        scale: 2.0,
        safe_area: ffi::Insets::default(),
    };
    for [w, h] in [[0, 2752], [2064, 0], [0, 0]] {
        let failure = crate::app::view(&desc(w, h), 0.0).unwrap_err();
        assert_eq!(failure.status, Status::InvalidArgument);
    }
    let portrait = crate::app::view(&desc(2064, 2752), 0.0).unwrap();
    let landscape = crate::app::view(&desc(2752, 2064), 0.0).unwrap();
    assert_eq!(portrait.size_px, [2064, 2752]);
    assert_eq!(landscape.size_px, [2752, 2064]);
    assert_eq!(landscape.scale, portrait.scale);
}

#[test]
fn an_idle_editor_asks_for_no_frame() {
    let idle = schedule(false, true, true, Duration::MAX);
    assert_eq!(idle, (-1.0, ffi::FRAME_PRESENTED));
    let blink = schedule(false, true, true, Duration::from_millis(500));
    assert_eq!(blink, (0.5, ffi::FRAME_PRESENTED));
    let busy = schedule(true, true, true, Duration::MAX);
    assert_eq!(
        busy,
        (0.0, ffi::FRAME_REDRAW_REQUESTED | ffi::FRAME_PRESENTED)
    );
    // A skipped drawable retries at once; without a surface nothing is retried.
    assert_eq!(
        schedule(false, false, true, Duration::MAX),
        (0.0, ffi::FRAME_REDRAW_REQUESTED)
    );
    assert_eq!(schedule(false, false, false, Duration::MAX), (-1.0, 0));
    assert_eq!(
        schedule(false, true, true, Duration::ZERO),
        (0.0, ffi::FRAME_REDRAW_REQUESTED | ffi::FRAME_PRESENTED)
    );
}
