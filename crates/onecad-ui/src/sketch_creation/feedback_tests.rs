use super::*;
use onecad_core::{
    ids::{EntityId, SketchId, SnapshotId},
    sketch::{CurvePosition, Sketch, SketchEntity, WorldPlane},
};
use onecad_interaction::{
    DocumentContext,
    sketch::{PrimitiveIds, SketchAction, SketchInteraction, SketchTarget, SketchTool},
};
use onecad_scene::{Camera, Projection, SceneIdentity, Viewport};

fn setup(tool: SketchTool) -> (SketchInteraction, ViewSnapshot, Rect) {
    let scene = SceneIdentity {
        session: 1,
        scene: 1,
    };
    let document = DocumentContext {
        document_id: "feedback".into(),
        runtime_session: "runtime".into(),
        revision: 1,
        snapshot_id: SnapshotId(1),
        scene,
    };
    let mut state = SketchInteraction::default();
    state.dispatch(SketchAction::ContextChanged(Some(document.clone())));
    state.dispatch(SketchAction::Begin {
        sketch: Box::new(Sketch::on_world_plane(
            SketchId::new(),
            "Sketch",
            WorldPlane::XY,
        )),
        create: false,
        context: document,
    });
    let view = ViewSnapshot::new(
        scene,
        1,
        Camera {
            offset: DVec3::Z * 100.0,
            projection: Projection::Orthographic,
            ..Default::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    )
    .unwrap();
    let session = state.session.as_mut().unwrap();
    session.phase = SketchPhase::Active;
    session.tool = tool;
    session.accepted_capture = Some(view.identity());
    (
        state,
        view,
        Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)),
    )
}

fn click(state: &mut SketchInteraction, view: &ViewSnapshot, at: Vec2) -> Vec2 {
    let session = state.session.as_ref().unwrap();
    let screen = view.project(DVec3::new(at.x, at.y, 0.0)).unwrap().logical;
    let point = onecad_interaction::sketch::capture_sketch_pointer(session, view, screen).unwrap();
    let captured = point.point;
    state.dispatch(SketchAction::Click {
        point,
        ids: PrimitiveIds {
            entities: std::array::from_fn(|_| EntityId::new()),
            constraints: std::array::from_fn(|_| onecad_core::ids::ConstraintId::new()),
        },
    });
    captured
}

#[test]
fn hover_candidate_paints_without_an_anchor_or_document_mutation() {
    let (state, view, rect) = setup(SketchTool::Line);
    let session = state.session.as_ref().unwrap();
    let before = serde_json::to_value(&session.authoritative).unwrap();
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        candidate(
            ui.painter(),
            rect,
            &view,
            session,
            Vec2::new_unchecked(10.0, 20.0),
            &Theme::filled(egui::Color32::WHITE),
        );
    });
    assert_eq!(
        output
            .shapes
            .iter()
            .filter(|shape| matches!(shape.shape, egui::Shape::Circle(_)))
            .count(),
        2
    );
    output.textures_delta.clear();
    assert!(session.draft.is_none());
    assert_eq!(
        before,
        serde_json::to_value(&session.authoritative).unwrap()
    );
}

#[test]
fn first_click_and_staged_anchors_exist_without_pointer_motion() {
    for tool in [
        SketchTool::Line,
        SketchTool::Rectangle,
        SketchTool::CenterRectangle,
        SketchTool::Circle,
        SketchTool::Arc,
        SketchTool::ThreePointArc,
        SketchTool::Slot,
    ] {
        let (mut state, view, _) = setup(tool);
        let first = Vec2::new_unchecked(10.0, 20.0);
        let first = click(&mut state, &view, first);
        let session = state.session.as_ref().unwrap();
        assert_eq!(anchor_points(session.draft.as_ref().unwrap()), vec![first]);
        assert!(session.authoritative.entities().is_empty());
        if matches!(
            tool,
            SketchTool::Arc | SketchTool::ThreePointArc | SketchTool::Slot
        ) {
            let second = Vec2::new_unchecked(30.0, 20.0);
            let second = click(&mut state, &view, second);
            assert_eq!(
                anchor_points(state.session.as_ref().unwrap().draft.as_ref().unwrap()),
                vec![first, second]
            );
        }
        state.dispatch(SketchAction::Escape);
        assert!(state.session.as_ref().unwrap().draft.is_none());
    }
}

#[test]
fn graphical_guide_uses_the_resolved_winner_reference_and_rejects_stale_source() {
    let (mut state, view, rect) = setup(SketchTool::Line);
    let reference = EntityId::new();
    let anchor = Vec2::new_unchecked(10.0, 20.0);
    let session = state.session.as_mut().unwrap();
    session
        .authoritative
        .add_entity(SketchEntity::point(reference, anchor, false, false))
        .unwrap();
    let point = Vec2::new_unchecked(45.0, 20.0);
    let pointer = project(rect, &view, session.plane, point).unwrap();
    let accepted = crate::sketch_snapping::resolve_snap(&crate::sketch_snapping::SnapRequest {
        sketch: &session.authoritative,
        plane: session.plane,
        view: &view,
        rect,
        pointer,
        anchor: None,
        previous: None,
        generation: session.request.sequence,
        settings: crate::sketch_snapping::SnapSettings {
            polar_step_degrees: None,
            ..Default::default()
        },
    })
    .unwrap();
    assert!((accepted.point.x - point.x).abs() < 1e-5);
    assert_eq!(accepted.point.y, point.y);
    assert_eq!(
        accepted.intent,
        SnapIntent::Guide {
            reference: SketchTarget::Point {
                entity: reference,
                role: CurvePosition::Arbitrary
            },
            alignment: onecad_interaction::sketch::inference::PointAlignment::Horizontal
        }
    );
    assert_eq!(guide_sources(session, &accepted), vec![anchor]);
    let ctx = egui::Context::default();
    session.request.sequence += 1;
    let mut output = ctx.run_ui(Default::default(), |ui| {
        snap(
            ui.painter(),
            rect,
            &view,
            session,
            accepted,
            &Theme::filled(egui::Color32::WHITE),
        );
    });
    assert!(output.shapes.is_empty());
    output.textures_delta.clear();
}

#[test]
fn guide_only_polar_has_anchor_but_no_persistent_intent() {
    let (mut state, view, rect) = setup(SketchTool::Line);
    let anchor = Vec2::new_unchecked(10.0, 20.0);
    let anchor = click(&mut state, &view, anchor);
    let session = state.session.as_ref().unwrap();
    let raw = Vec2::new_unchecked(30.0, 31.0);
    let accepted = crate::sketch_snapping::resolve_snap(&crate::sketch_snapping::SnapRequest {
        sketch: &session.authoritative,
        plane: session.plane,
        view: &view,
        rect,
        pointer: project(rect, &view, session.plane, raw).unwrap(),
        anchor: Some((anchor, None)),
        previous: None,
        generation: session.request.sequence,
        settings: crate::sketch_snapping::SnapSettings::default(),
    })
    .unwrap();
    assert_eq!(accepted.kind, SnapKind::Polar);
    assert_eq!(accepted.intent, SnapIntent::GeometryOnly);
    assert_eq!(guide_sources(session, &accepted), vec![anchor]);
}

#[test]
fn combined_guide_uses_both_accepted_sources() {
    let (mut state, view, rect) = setup(SketchTool::Line);
    let session = state.session.as_mut().unwrap();
    let sources = [
        Vec2::new_unchecked(10.0, 20.0),
        Vec2::new_unchecked(40.0, -10.0),
    ];
    for at in sources {
        session
            .authoritative
            .add_entity(SketchEntity::point(EntityId::new(), at, false, false))
            .unwrap();
    }
    let point = Vec2::new_unchecked(40.0, 20.0);
    let accepted = crate::sketch_snapping::resolve_snap(&crate::sketch_snapping::SnapRequest {
        sketch: &session.authoritative,
        plane: session.plane,
        view: &view,
        rect,
        pointer: project(rect, &view, session.plane, point).unwrap(),
        anchor: None,
        previous: None,
        generation: session.request.sequence,
        settings: crate::sketch_snapping::SnapSettings {
            polar_step_degrees: None,
            ..Default::default()
        },
    })
    .unwrap();
    assert_eq!(accepted.kind, SnapKind::Guides);
    assert_eq!(accepted.point, point);
    assert_eq!(guide_sources(session, &accepted), sources);
}

#[test]
fn hysteresis_cache_does_not_cross_tool_draft_or_visit_ownership() {
    let (mut state, view, _) = setup(SketchTool::Line);
    let session = state.session.as_mut().unwrap();
    let cached = crate::sketch_creation::CachedSnap {
        snap: AcceptedSnap {
            source: crate::sketch_snapping::snap_source(
                &session.authoritative,
                &view,
                session.request.sequence,
            ),
            point: Vec2::new_unchecked(0.0, 0.0),
            kind: SnapKind::Origin,
            intent: SnapIntent::Origin,
        },
        key: session.key.clone(),
        tool: session.tool,
        draft_generation: session.draft_generation,
    };
    assert!(cached.current(session));
    session.tool = SketchTool::Rectangle;
    assert!(!cached.current(session));
    session.tool = SketchTool::Line;
    session.draft_generation += 1;
    assert!(!cached.current(session));
    session.draft_generation -= 1;
    session.key.generation += 1;
    assert!(!cached.current(session));
    let ctx = egui::Context::default();
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("sketch-winning-snap"), cached));
    crate::sketch_creation::clear_candidate(&ctx);
    assert!(
        ctx.data(
            |data| data.get_temp::<crate::sketch_creation::CachedSnap>(egui::Id::unique(
                "sketch-winning-snap"
            ))
        )
        .is_none()
    );
}
