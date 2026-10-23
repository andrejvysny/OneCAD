use super::{test_support::*, *};
use onecad_core::{
    document::{
        record::*,
        refs::{AxisRef, ElementKind, ElementRef, PrimaryRef},
    },
    ids::{ElementId, SketchId},
    math::Vec3,
};
use onecad_interaction::{Action, DragPhase, Effect, NumericParameter as N};
use onecad_scene::DVec3;

#[test]
fn checked_topology_anchors_map_every_scalar_and_missing_ids_fail_closed() {
    let scene = scene();
    let context = anchors::Context {
        scene: &scene,
        records: &[],
        profiles: &[],
        profile_anchors: &[],
    };
    for (record, expected) in records(&scene) {
        let mut state = begin(record);
        let tool = state.tool.as_ref().unwrap();
        let targets = target::for_tool(tool, &context, &ModelHandle::default());
        let actual: Vec<_> = targets
            .iter()
            .filter_map(|t| match t.kind {
                Kind::Scalar(key) => Some(key.parameter()),
                _ => None,
            })
            .collect();
        assert_eq!(actual, expected, "{}", tool.record.name);
        for target in targets {
            assert!(target.geometry.base().is_finite());
            if let Kind::Scalar(key) = target.kind {
                assert_eq!(Some(target.value), tool.value(key));
            }
        }
        state.dispatch(Action::Cancel);
    }
    let body = scene.bodies()[0].body_id();
    let reference = ElementRef {
        primary: Some(PrimaryRef {
            body,
            kind: ElementKind::Face,
            element: ElementId::new("missing-face"),
            extra: Default::default(),
        }),
        intent: None,
        anchor: None,
        extra: Default::default(),
    };
    assert!(anchors::face_ref(&context, &reference).is_none());
    let mut reference = reference;
    reference.primary.as_mut().unwrap().body = onecad_core::ids::BodyId::new();
    assert!(anchors::face_ref(&context, &reference).is_none());
}

#[test]
fn revolve_resolves_real_axis_and_profile_geometry() {
    let scene = scene();
    let body = &scene.bodies()[0];
    let mesh = body.mesh().unwrap();
    let i = (0..mesh.header().edge_count as usize)
        .find(|&i| {
            let range = mesh.edge_range(i).unwrap();
            mesh.edge_point(range.first as usize)
                != mesh.edge_point((range.first + range.count - 1) as usize)
        })
        .unwrap();
    let sketch = SketchId::new();
    let profiles = crate::profiles::decode_profiles(
        sketch,
        onecad_core::sketch::SketchPlane::xy(),
        onecad_app::dto::FinishSketchDto {
            region_identity_version: 3,
            diagnostics: vec![],
            regions: vec![onecad_app::dto::SketchRegionDto {
                region_id: "cell-v3:handle".into(),
                outer_loop: vec![],
                holes: vec![],
                preview_triangles: Some(onecad_app::dto::PreviewTrianglesDto {
                    positions: vec![10.0, 10.0, 20.0, 10.0, 10.0, 20.0],
                    indices: vec![0, 1, 2],
                    holes_subtracted: 0,
                }),
            }],
        },
    )
    .unwrap();
    let record = onecad_interaction::modeling_records::revolve_axis(
        profiles[0].reference.clone(),
        AxisRef::Element {
            body: body.body_id(),
            edge: ElementId::new(mesh.edge_id(i).unwrap()),
            edge_ref: None,
            extra: Default::default(),
        },
        RecordId::new(),
        0,
    );
    let state = begin(record);
    let context = anchors::Context {
        scene: &scene,
        records: &[],
        profiles: &profiles,
        profile_anchors: &[],
    };
    let targets = target::for_tool(
        state.tool.as_ref().unwrap(),
        &context,
        &ModelHandle::default(),
    );
    assert_eq!(targets.len(), 1);
    assert!(matches!(
        targets[0].kind,
        Kind::Scalar(ParameterKey::Scalar(N::RevolveAngle))
    ));
    assert!(targets[0].geometry.angle());
}

#[test]
fn centered_circular_pattern_retains_an_axis_ring() {
    let scene = scene();
    let id = scene.bodies()[0].body_id();
    let context = anchors::Context {
        scene: &scene,
        records: &[],
        profiles: &[],
        profile_anchors: &[],
    };
    let mut record = onecad_interaction::modeling_records::circular_pattern(id, RecordId::new(), 0);
    if let Operation::Known(KnownOperation::CircularPattern(p)) = &mut record.op {
        p.axis_origin = anchors::core(anchors::center(&context, id).unwrap());
    }
    let state = begin(record);
    let targets = target::for_tool(
        state.tool.as_ref().unwrap(),
        &context,
        &ModelHandle::default(),
    );
    assert_eq!(targets.len(), 1);
    assert!(targets[0].geometry.angle());
}

#[test]
fn accepted_drag_updates_actual_registered_fields_and_cancel_restores_expression() {
    let scene = scene();
    let context = anchors::Context {
        scene: &scene,
        records: &[],
        profiles: &[],
        profile_anchors: &[],
    };
    for (mut record, expected) in records(&scene) {
        for (_, _, scalar) in match &mut record.op {
            Operation::Known(op) => op.scalars_mut(),
            _ => unreachable!(),
        } {
            scalar.expr = Some(format!("{} / 2", scalar.value * 2.0));
        }
        for parameter in expected {
            let mut state = begin(record.clone());
            let original = state
                .tool
                .as_ref()
                .unwrap()
                .params
                .field(parameter.into())
                .unwrap()
                .text
                .clone();
            let targets = target::for_tool(
                state.tool.as_ref().unwrap(),
                &context,
                &ModelHandle::default(),
            );
            let target = targets
                .into_iter()
                .find(|t| matches!(t.kind, Kind::Scalar(key) if key.parameter() == parameter))
                .unwrap();
            let view = view();
            let (_, press) = target.geometry.points(&view, rect(), target.value).unwrap();
            let mut drag = Drag::start(target.clone(), press, &view, rect(), true).unwrap();
            let delta = if target.geometry.angle() {
                -20.0
            } else if parameter == N::GearModule {
                2.0
            } else {
                30.0
            };
            let (_, moved) = target
                .geometry
                .points(&view, rect(), target.value + delta)
                .unwrap();
            drag.track(moved, &view, rect());
            assert!(drag.changed(), "{parameter:?}");
            let text = drag.pending().unwrap();
            let effects = state.dispatch(Action::ParameterDrag(DragPhase::Begin));
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::ApplyOperation { .. }))
            );
            let effects = state.dispatch(Action::ParameterDrag(DragPhase::Update {
                key: parameter.into(),
                text: text.clone(),
            }));
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::ApplyOperation { .. }))
            );
            evaluate(&mut state, drag.value);
            let tool = state.tool.as_ref().unwrap();
            assert_eq!(tool.value(parameter), Some(drag.value));
            assert_eq!(tool.params.field(parameter.into()).unwrap().text, text);
            let actual = onecad_interaction::numeric_parameters(&tool.effective_op())
                .into_iter()
                .find(|d| d.parameter == parameter)
                .unwrap()
                .scalar
                .value;
            assert!((actual - drag.value).abs() < 1e-9);
            state.dispatch(Action::ParameterDrag(DragPhase::Cancel));
            assert_eq!(
                state
                    .tool
                    .as_ref()
                    .unwrap()
                    .params
                    .field(parameter.into())
                    .unwrap()
                    .text,
                original
            );
            assert!(!state.tool.as_ref().unwrap().dragging());
        }
    }
}

#[test]
fn press_no_motion_and_return_to_original_preserve_expression_and_precision() {
    let view = view();
    let rect = rect();
    for text in ["6 * 2", "12.3456"] {
        let value = if text == "6 * 2" { 12.0 } else { 12.3456 };
        let target = Target {
            caption: None,
            record: RecordId::new(),
            kind: Kind::Scalar(N::HoleDiameter.into()),
            geometry: Geometry::axis(DVec3::ZERO, DVec3::X, 0.5),
            value,
            original: text.into(),
        };
        let (_, press) = target.geometry.points(&view, rect, value).unwrap();
        let mut drag = Drag::start(target.clone(), press, &view, rect, true).unwrap();
        assert!(drag.pending().is_none());
        drag.track(press + egui::vec2(3.0, -3.0), &view, rect);
        assert!(drag.pending().is_none());
        let (_, moved) = target.geometry.points(&view, rect, value + 10.0).unwrap();
        drag.track(moved, &view, rect);
        assert!(drag.changed());
        drag.sent = drag.pending();
        drag.track(press, &view, rect);
        assert!(!drag.changed());
        assert_eq!(drag.pending().as_deref(), Some(text));
    }
}

#[test]
fn parallel_axis_and_edge_on_ring_never_claim_pointer() {
    let view = onecad_scene::ViewSnapshot::new(
        context().scene,
        1,
        onecad_scene::Camera {
            offset: DVec3::Z * 100.0,
            projection: onecad_scene::Projection::Orthographic,
            ..Default::default()
        },
        view().viewport(),
    )
    .unwrap();
    let axis = Geometry::axis(DVec3::ZERO, DVec3::Z, 1.0);
    let (_, tip) = axis.points(&view, rect(), 10.0).unwrap();
    assert!(!axis.claims(&view, rect(), 10.0, tip));
    let ring = Geometry::ring(DVec3::ZERO, DVec3::X, DVec3::Y * 10.0).unwrap();
    let (_, tip) = ring.points(&view, rect(), 0.0).unwrap();
    assert!(!ring.claims(&view, rect(), 0.0, tip));
}

#[test]
fn mirror_options_drag_restores_exact_record_on_cancel_and_keeps_release_draft() {
    let scene = scene();
    let id = scene.bodies()[0].body_id();
    for cancel in [true, false] {
        let record = onecad_interaction::modeling_records::mirror(id, RecordId::new(), 0);
        let original = record.op.clone();
        let mut state = begin(record);
        state.dispatch(Action::ParameterDrag(DragPhase::Begin));
        let tag = state.tool.as_ref().unwrap().tag.clone();
        let effects = state.dispatch(Action::ModelingOptionsChanged {
            tag,
            options: ModelingOptions::Mirror {
                point: Vec3::new_unchecked(3.0, 0.0, 0.0),
                normal: Vec3::new_unchecked(1.0, 0.0, 0.0),
                fuse: false,
            },
        });
        assert!(state.tool.as_ref().unwrap().dragging());
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::PreviewExistingEdit { .. }))
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::ApplyOperation { .. }))
        );
        state.dispatch(Action::ParameterDrag(if cancel {
            DragPhase::Cancel
        } else {
            DragPhase::Commit
        }));
        let tool = state.tool.as_ref().unwrap();
        if cancel {
            assert_eq!(tool.record.op, original);
        } else {
            assert_ne!(tool.record.op, original);
        }
        assert!(!tool.dragging());
    }
}

#[test]
fn axis_extension_reaches_visible_viewport_beyond_chrome_without_changing_measurement() {
    let view = view();
    let rect = rect();
    let geometry = Geometry::axis(DVec3::ZERO, DVec3::X, 1.0);
    let base = geometry.points(&view, rect, 0.0).unwrap().0;
    let cover = Rect::from_center_size(base, egui::vec2(200.0, 200.0));
    for value in [2.0, 50.0, 5000.0] {
        let shown = geometry.avoid(&view, rect, value, &[cover]);
        let (_, tip) = shown.points(&view, rect, value).unwrap();
        assert!(rect.shrink(13.0).contains(tip), "{value}: {tip:?}");
        assert!(
            !cover.contains(tip),
            "extension must pass beyond both original +/-40 px arms"
        );
        let at = shown.coordinate(&view, rect, tip).unwrap();
        let (_, next) = shown.points(&view, rect, value + 1.0).unwrap();
        let delta = shown.coordinate(&view, rect, next).unwrap() - at;
        assert!(
            (delta - 1.0).abs() < 1e-3,
            "display extension must retain true parameter mapping: {delta}"
        );
    }
}
