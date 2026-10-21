use super::*;
use onecad_core::{math::Vec3, sketch::plane_from_point_normal};
use onecad_scene::{Camera, DVec3, Projection};

fn capture(number: u64) -> CaptureIdentity {
    onecad_scene::ViewSnapshot::new(
        onecad_scene::SceneIdentity {
            session: 1,
            scene: 2,
        },
        number,
        Camera::default(),
        onecad_scene::Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap()
    .identity()
}

fn pending_preview(capture: CaptureIdentity) -> (Entry, mpsc::Sender<Result<SketchPlane, String>>) {
    let (sender, receiver) = mpsc::channel();
    let candidate = Candidate::Face(onecad_core::ids::BodyId::new(), "face:test".into());
    (
        Entry {
            open: true,
            candidate: Some(candidate.clone()),
            capture: Some(capture),
            query: Some(Preview {
                candidate,
                capture,
                receiver: Arc::new(Mutex::new(receiver)),
            }),
            ..Default::default()
        },
        sender,
    )
}

#[test]
fn named_patch_picking_uses_frozen_plane_coordinates_and_refuses_off_patch() {
    for plane in [WorldPlane::XY, WorldPlane::XZ, WorldPlane::YZ] {
        let mut camera = Camera::default();
        align_camera(&mut camera, plane.plane());
        let view = onecad_scene::ViewSnapshot::new(
            onecad_scene::SceneIdentity {
                session: 1,
                scene: 1,
            },
            1,
            camera,
            onecad_scene::Viewport {
                width: 900.0,
                height: 560.0,
                scale_factor: 1.0,
            },
        )
        .unwrap();
        let extent = camera.offset.length() * 0.22;
        let (sx, sy) = picking::facing(&view, plane);
        for (factor, expected) in [(0.6, true), (0.01, false), (1.2, false)] {
            let point = plane
                .plane()
                .to_world(onecad_core::math::Vec2::new_unchecked(
                    sx * factor * extent,
                    sy * factor * extent,
                ));
            let projected = view.project(DVec3::new(point.x, point.y, point.z)).unwrap();
            assert_eq!(
                picking::patch_distance(&view, projected.logical, plane, extent).is_some(),
                expected
            );
        }
    }
}

#[test]
fn exact_face_preview_requires_current_candidate_and_capture() {
    let (mut entry, sender) = pending_preview(capture(1));
    sender.send(Ok(WorldPlane::XZ.plane())).unwrap();
    poll_preview(&mut entry, capture(2));
    assert!(entry.plane.is_none());
    let (mut entry, sender) = pending_preview(capture(1));
    entry.candidate = Some(Candidate::Named(WorldPlane::XY));
    sender.send(Ok(WorldPlane::XZ.plane())).unwrap();
    poll_preview(&mut entry, capture(1));
    assert!(entry.plane.is_none());
    let (mut entry, sender) = pending_preview(capture(1));
    sender.send(Ok(WorldPlane::XZ.plane())).unwrap();
    poll_preview(&mut entry, capture(1));
    assert_eq!(entry.plane, Some(WorldPlane::XZ.plane()));
}

#[test]
fn cancel_drops_preview_and_armed_click_without_admission() {
    let (mut entry, sender) = pending_preview(capture(1));
    entry.accept = entry
        .candidate
        .clone()
        .map(|candidate| (candidate, capture(1)));
    entry.dismiss();
    assert!(!entry.open);
    assert!(entry.query.is_none() && entry.accept.is_none() && entry.plane.is_none());
    assert!(sender.send(Ok(WorldPlane::XY.plane())).is_err());
    assert!(entry.pending.is_none());
}

#[test]
fn refused_face_preview_cancels_armed_click_without_xy_fallback() {
    let (mut entry, sender) = pending_preview(capture(1));
    entry.accept = entry
        .candidate
        .clone()
        .map(|candidate| (candidate, capture(1)));
    sender.send(Err("Face is not planar".into())).unwrap();
    poll_preview(&mut entry, capture(1));
    assert!(entry.plane.is_none() && entry.accept.is_none() && entry.pending.is_none());
    assert_eq!(entry.message.as_deref(), Some("Face is not planar"));
}

#[test]
fn camera_uses_persisted_world_and_exact_face_axes() {
    let tilted = plane_from_point_normal(
        Vec3::new(12.0, -5.0, 8.0).unwrap(),
        Vec3::new(0.6, 0.0, 0.8).unwrap(),
    );
    for plane in [
        WorldPlane::XY.plane(),
        WorldPlane::XZ.plane(),
        WorldPlane::YZ.plane(),
        tilted,
    ] {
        let mut camera = Camera::default();
        let distance = camera.offset.length();
        align_camera(&mut camera, plane);
        let basis = camera.basis().unwrap();
        let vector = |v: Vec3| DVec3::new(v.x, v.y, v.z);
        assert!((basis.right - vector(plane.x_axis)).length() < 1e-12);
        assert!((basis.up - vector(plane.y_axis)).length() < 1e-12);
        assert!((basis.back - vector(plane.normal)).length() < 1e-12);
        assert_eq!(camera.target, vector(plane.origin));
        assert!((camera.offset.length() - distance).abs() < 1e-12);
        assert_eq!(camera.projection, Projection::Orthographic);
    }
}

fn owned(lease: &Arc<()>) -> (Entry, SketchCandidateControl) {
    let control = SketchCandidateControl::default();
    let entry = Entry {
        open: true,
        pending: Some(Pending {
            control: control.clone(),
            lease: Arc::downgrade(lease),
            cancelled: None,
        }),
        ..Default::default()
    };
    (entry, control)
}

#[test]
fn entry_cancel_keeps_executing_service_owner() {
    let lease = Arc::new(());
    let (mut entry, control) = owned(&lease);
    entry.dismiss();
    entry.settle();
    assert!(entry.open);
    assert_eq!(entry.pending.as_ref().unwrap().cancelled, Some(true));
    assert!(!control.cancel());
    // A repeated cancel must not turn the won cancel into a reported admission.
    entry.dismiss();
    assert_eq!(entry.pending.as_ref().unwrap().cancelled, Some(true));
    drop(lease);
    entry.settle();
    assert!(!entry.open && entry.pending.is_none());
}

#[test]
fn ownership_outlives_idle_frames_until_the_delivery_is_applied() {
    let lease = Arc::new(());
    // The service moves a clone into its delivery before the request is consumed.
    let delivery = lease.clone();
    let (mut entry, control) = owned(&lease);
    drop(lease);
    for _ in 0..3 {
        entry.settle();
    }
    assert!(entry.open && entry.pending.is_some());
    drop(delivery);
    entry.settle();
    assert!(!entry.open && entry.pending.is_none());
    // Never cancelled: the request's own control remained untouched.
    assert!(control.cancel());
}

#[test]
fn refused_submit_releases_ownership_without_cancelling() {
    let lease = Arc::new(());
    let (mut entry, control) = owned(&lease);
    // A refused submit drops the request, and with it the only strong lease.
    drop(lease);
    entry.settle();
    assert!(!entry.open && entry.pending.is_none());
    assert!(control.cancel());
}

#[test]
fn settle_leaves_unowned_chooser_open() {
    let mut entry = Entry {
        open: true,
        ..Default::default()
    };
    entry.settle();
    assert!(entry.open);
}
