//! Pure placement decisions: snap selection, flip default, auto-size, mate construction.
use super::{
    autosize::{self, THREAD},
    mate,
    snap::{self, Candidate, Geometry, Sent, SnapIntent},
};
use crate::library_catalog::{
    draft::ParamDraft,
    testing::{component, iso_specs},
};
use onecad_app::dto::{ClassifyElementDto, ClassifyElementFrameDto};
use onecad_core::{
    document::record::{ComponentParamValue as Value, MateKind},
    ids::BodyId,
};
use onecad_library::package::{AttachmentFrame, AttachmentSpec};
use std::collections::{BTreeMap, BTreeSet};

fn attachments() -> BTreeMap<String, AttachmentSpec> {
    let spec = |on: &str, accepts: &[&str]| AttachmentSpec {
        on: on.into(),
        accepts: accepts.iter().map(|a| (*a).into()).collect(),
        frame: None,
    };
    BTreeMap::from([
        ("head_seat".into(), spec("face:head_underside", &["plane"])),
        (
            "shank_axis".into(),
            spec("cylinder:shank", &["cylinder", "hole", "circularEdge"]),
        ),
    ])
}

fn frame(
    axis: Option<[f64; 3]>,
    radius: Option<f64>,
    side: Option<&str>,
) -> ClassifyElementFrameDto {
    ClassifyElementFrameDto {
        origin: [10.0, 20.0, 5.0],
        normal: axis.is_none().then_some([0.0, 0.0, 1.0]),
        axis,
        radius,
        sidedness: side.map(Into::into),
    }
}

fn rim(radius: f64) -> ClassifyElementDto {
    ClassifyElementDto {
        kind: "edge".into(),
        surface_type: String::new(),
        curve_type: "circle".into(),
        frame: Some(frame(Some([0.0, 0.0, 1.0]), Some(radius), None)),
    }
}

fn bore(radius: f64, side: &str) -> ClassifyElementDto {
    ClassifyElementDto {
        kind: "face".into(),
        surface_type: "cylinder".into(),
        curve_type: String::new(),
        frame: Some(frame(Some([0.0, 0.0, 1.0]), Some(radius), Some(side))),
    }
}

fn plane() -> ClassifyElementDto {
    ClassifyElementDto {
        kind: "face".into(),
        surface_type: "plane".into(),
        curve_type: String::new(),
        frame: Some(frame(None, None, None)),
    }
}

const HIT: [f64; 3] = [1.0, 2.0, 3.0];

#[test]
fn a_rim_seats_the_shank_concentric_and_coincident_at_the_rim_centre() {
    let found = snap::candidates(&attachments(), &rim(3.0), HIT);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].attachment, "shank_axis");
    assert_eq!(found[0].kind, MateKind::ConcentricAndCoincident);
    assert_eq!(found[0].anchor, [10.0, 20.0, 5.0]);
    assert!(found[0].target_is_edge());
}

#[test]
fn a_cylinder_is_concentric_at_the_hit_and_a_plane_is_coincident_at_the_hit() {
    let found = snap::candidates(&attachments(), &bore(3.0, "hole"), HIT);
    assert_eq!(found[0].kind, MateKind::Concentric);
    assert_eq!(found[0].anchor, HIT);
    let found = snap::candidates(&attachments(), &plane(), HIT);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].attachment, "head_seat");
    assert_eq!(found[0].kind, MateKind::Coincident);
    assert_eq!(found[0].anchor, HIT);
}

#[test]
fn unseatable_geometry_and_unaccepted_kinds_fall_back_to_ground() {
    let cone = ClassifyElementDto {
        kind: "face".into(),
        surface_type: "cone".into(),
        curve_type: String::new(),
        frame: None,
    };
    assert!(snap::candidates(&attachments(), &cone, HIT).is_empty());
    let mut only_planes = attachments();
    only_planes.remove("shank_axis");
    assert!(snap::candidates(&only_planes, &rim(3.0), HIT).is_empty());
    assert!(snap::candidates(&BTreeMap::new(), &plane(), HIT).is_empty());
}

#[test]
fn ground_is_the_ray_on_z_zero_with_a_camera_facing_fallback() {
    let point = snap::ground_point([0.0, 0.0, 10.0], [0.0, 0.6, -0.8]).unwrap();
    assert!((point[2]).abs() < 1e-9 && (point[1] - 7.5).abs() < 1e-9);
    // Parallel to the ground: the plane through the origin facing the camera.
    let point = snap::ground_point([0.0, -5.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
    assert!(point.iter().all(|v| v.abs() < 1e-9), "{point:?}");
}

#[test]
fn the_head_defaults_toward_the_camera() {
    let candidate = |geometry, axis| Candidate {
        attachment: "a".into(),
        kind: MateKind::Concentric,
        geometry,
        anchor: HIT,
        axis,
        radius: Some(3.0),
        sidedness: None,
    };
    let up = Some([0.0, 0.0, 1.0]);
    let toward_camera = [0.0, 0.0, 1.0];
    assert!(!snap::default_flip(
        &candidate(Geometry::Rim, up),
        toward_camera
    ));
    assert!(snap::default_flip(
        &candidate(Geometry::Rim, up),
        [0.0, 0.0, -1.0]
    ));
    assert!(snap::default_flip(
        &candidate(Geometry::Cylinder, Some([0.0, 0.0, -1.0])),
        toward_camera
    ));
    // A plane seats on its outward normal as is.
    assert!(!snap::default_flip(
        &candidate(Geometry::Plane, None),
        [0.0, 0.0, -1.0]
    ));
}

#[test]
fn dispatch_only_when_the_intent_changes_or_a_following_anchor_moves_far() {
    let sent = |key, flipped, anchor: [f64; 3], follows| Sent {
        key,
        flipped,
        anchor,
        follows_pointer: follows,
    };
    let ground = snap::IntentKey::Ground;
    let base = sent(ground.clone(), false, [0.0; 3], true);
    assert!(snap::should_dispatch(None, &base));
    assert!(!snap::should_dispatch(
        Some(&base),
        &sent(ground.clone(), false, [0.2, 0.0, 0.0], true)
    ));
    assert!(snap::should_dispatch(
        Some(&base),
        &sent(ground.clone(), false, [0.3, 0.0, 0.0], true)
    ));
    assert!(snap::should_dispatch(
        Some(&base),
        &sent(ground.clone(), true, [0.0; 3], true)
    ));
    let mate_key = snap::IntentKey::Mate {
        body: BodyId::new(),
        topology: "t".into(),
        attachment: "shank_axis".into(),
    };
    let fixed = sent(mate_key.clone(), false, [0.0; 3], false);
    assert!(!snap::should_dispatch(
        Some(&fixed),
        &sent(mate_key, false, [9.0, 0.0, 0.0], false)
    ));
    assert!(snap::should_dispatch(
        Some(&fixed),
        &sent(snap::IntentKey::Ground, false, [0.0; 3], true)
    ));
}

fn thread_field() -> crate::library_catalog::draft::Field {
    let draft = ParamDraft::from_component(&component("iso", "1.0.0", iso_specs()));
    draft
        .fields()
        .iter()
        .find(|f| f.key == THREAD)
        .cloned()
        .unwrap()
}

fn candidate(radius: f64, side: Option<&str>, geometry: Geometry) -> Candidate {
    Candidate {
        attachment: "shank_axis".into(),
        kind: MateKind::Concentric,
        geometry,
        anchor: HIT,
        axis: Some([0.0, 0.0, 1.0]),
        radius: Some(radius),
        sidedness: side.map(Into::into),
    }
}

fn params() -> BTreeMap<String, Value> {
    ParamDraft::from_component(&component("iso", "1.0.0", iso_specs()))
        .resolve()
        .unwrap()
}

#[test]
fn the_largest_thread_that_fits_the_hole_is_chosen() {
    let field = thread_field();
    let domain: Vec<String> = field.domain.iter().map(|d| d.text()).collect();
    let pick = |r: f64| autosize::thread_for_radius(domain.iter().map(String::as_str), r);
    assert_eq!(pick(3.0), Some("M6"));
    assert_eq!(pick(3.3), Some("M6"));
    assert_eq!(pick(4.0), Some("M8"));
    assert_eq!(pick(2.5), Some("M5"));
    assert_eq!(pick(0.5), None);
}

#[test]
fn a_resize_sends_the_complete_parameter_map() {
    let field = thread_field();
    let before = params();
    let map = autosize::resized(
        Some(&field),
        &before,
        &BTreeSet::new(),
        &candidate(4.0, Some("hole"), Geometry::Cylinder),
    )
    .expect("M8 differs from the default M6");
    assert_eq!(map.get(THREAD), Some(&Value::Text("M8".into())));
    assert_eq!(
        map.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    assert_eq!(map.get("length"), before.get("length"));
    // Already that size: nothing to send.
    assert!(
        autosize::resized(
            Some(&field),
            &map,
            &BTreeSet::new(),
            &candidate(4.0, None, Geometry::Rim)
        )
        .is_none()
    );
}

#[test]
fn a_typed_thread_wins_and_a_pin_is_never_resized() {
    let field = thread_field();
    let manual = BTreeSet::from([THREAD.to_owned()]);
    let hole = candidate(4.0, Some("hole"), Geometry::Cylinder);
    assert!(autosize::resized(Some(&field), &params(), &manual, &hole).is_none());
    let pin = candidate(4.0, Some("pin"), Geometry::Cylinder);
    assert!(autosize::resized(Some(&field), &params(), &BTreeSet::new(), &pin).is_none());
    let face = candidate(4.0, None, Geometry::Plane);
    assert!(autosize::resized(Some(&field), &params(), &BTreeSet::new(), &face).is_none());
    assert!(autosize::resized(None, &params(), &BTreeSet::new(), &hole).is_none());
}

#[test]
fn the_mate_is_built_like_the_applications_with_the_frozen_attachment_frame() {
    let mut attachments = attachments();
    attachments.get_mut("shank_axis").unwrap().frame = Some(AttachmentFrame {
        origin: [0.0, 0.0, 1.5],
        z: [0.0, 0.0, 1.0],
        x: [1.0, 0.0, 0.0],
    });
    let body = BodyId::new();
    let found = snap::candidates(&attachments, &rim(3.0), HIT);
    let mate = mate::build(&attachments, &found[0], (body, "el_rim"), true);
    assert_eq!(mate.self_attachment, "shank_axis");
    assert_eq!(mate.kind, MateKind::ConcentricAndCoincident);
    assert!(mate.flipped);
    assert_eq!(mate.self_frame.unwrap().origin.z, 1.5);
    let primary = mate.target.primary.unwrap();
    assert_eq!((primary.body, primary.element.as_str()), (body, "el_rim"));
    assert_eq!(mate.target.anchor.unwrap().world_point.x, 10.0);
    assert!(mate.target_axis.is_none() && mate.target_sidedness.is_none());
    assert_eq!(
        snap::describe(&SnapIntent::Mate(found[0].clone()), false),
        "Concentric on rim \u{d8}6.0"
    );
}
