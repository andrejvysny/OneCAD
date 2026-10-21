//! Synthetic native-host adapters with production worker and Metal; not physical input acceptance.
use super::*;
use crate::{
    bridge::Command,
    constraint_authoring::{self, Relation},
};
use onecad_core::{
    document::record::{KnownOperation, Operation},
    edit::HistoryDirection,
    ids::{ConstraintId, EntityId, RecordId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use onecad_interaction::{
    Action, CommitStatus, DraftStatus, FeatureOutcome, PreviewStatus, ProfileSelection,
    sketch::{
        CapturedSketchPoint, PrimitiveIds, SelectionMode, SketchAction, SketchPhase, SketchStatus,
        SketchTarget, SketchTool,
    },
};
use onecad_scene::{DVec3, ViewSnapshot};
#[path = "plate_delivery_tests/support.rs"]
mod support;
use support::*;
#[path = "plate_delivery_tests/assertions.rs"]
mod assertions;
use assertions::*;

#[path = "plate_delivery_tests/history_editor.rs"]
mod history_editor;

struct Plate {
    rectangle: PrimitiveIds,
    holes: [PrimitiveIds; 2],
    centerline: PrimitiveIds,
    width: ConstraintId,
    diameters: [ConstraintId; 2],
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic host adapters do not qualify physical input"]
async fn from_new_plate_constraints_edit_repair_history_finish_and_extrude() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Harness::new(area.path()).await;
    let plate = create_plate(&mut host).await;
    assert_plate(&host, &plate, 80.0);
    let identities: Vec<_> = host
        .sketch()
        .entities()
        .iter()
        .map(SketchEntity::id)
        .collect();
    let constraints: Vec<_> = host
        .sketch()
        .constraints()
        .iter()
        .map(Constraint::id)
        .collect();
    host.edit_width(plate.width, "90").await;
    assert_plate(&host, &plate, 90.0);
    host.sketch_history(HistoryDirection::Undo).await;
    assert_plate(&host, &plate, 80.0);
    host.sketch_history(HistoryDirection::Redo).await;
    assert_plate(&host, &plate, 90.0);
    remove_and_repair(&mut host, &plate).await;
    assert_eq!(
        host.sketch()
            .entities()
            .iter()
            .map(SketchEntity::id)
            .collect::<Vec<_>>(),
        identities
    );
    assert_eq!(
        host.sketch()
            .constraints()
            .iter()
            .map(Constraint::id)
            .collect::<Vec<_>>(),
        constraints
    );
    let sketch_id = host.sketch().id;
    host.finish().await;
    assert_profiles(&host, &plate);
    host.app.edit_sketch(sketch_id);
    host.wait(|app| {
        app.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    assert_plate(&host, &plate, 90.0);
    assert!(
        host.app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .history
            .as_ref()
            .is_some_and(|history| history.undo_depth == 0 && history.redo_depth == 0)
    );
    host.finish().await;
    assert_profiles(&host, &plate);
    extrude(&mut host).await;
    history_editor::check(&mut host).await;
    model_history(&mut host).await;
    assert_brep(&host, 90.0).await;
    assert_stale_brep_reads_refused(&host).await;
    downstream_width_edits(&mut host, &plate).await;
    assert!(host.app.document_dirty());
    assert!(host.app.doc.document_state.as_ref().unwrap().path.is_none());
    eprintln!(
        "AUTOMATED ONLY: From-New constrained plate, width edit, repair, history, Finish, re-entry, Extrude, downstream width regeneration. Physical input, Save dialog, clean Quit and fresh reopen remain unqualified."
    );
    host.app.stop_discard();
    host.wait(|app| app.may_close).await;
}

async fn create_plate(host: &mut Harness) -> Plate {
    let rectangle = host
        .primitive(SketchTool::Rectangle, [0.0, 0.0], [80.0, 50.0])
        .await;
    let first = host
        .primitive(SketchTool::Circle, [15.0, 25.0], [18.0, 25.0])
        .await;
    let second = host
        .primitive(SketchTool::Circle, [65.0, 25.0], [68.0, 25.0])
        .await;
    host.select(vec![]);
    crate::sketch_repair::toggle_construction(&mut host.app);
    assert!(host.app.doc.sketch.session.as_ref().unwrap().construction);
    let centerline = host
        .primitive(SketchTool::Line, [0.0, 25.0], [80.0, 25.0])
        .await;
    host.select(vec![]);
    crate::sketch_repair::toggle_construction(&mut host.app);
    let r = rectangle.entities;
    host.relation(Relation::Fixed, vec![point(r[0])], None)
        .await;
    let width = host
        .relation(
            Relation::HorizontalDistance,
            vec![point(r[0]), point(r[1])],
            Some("80"),
        )
        .await;
    host.relation(
        Relation::VerticalDistance,
        vec![point(r[0]), point(r[3])],
        Some("50"),
    )
    .await;
    let diameters = constrain_holes(host, &rectangle, [&first, &second]).await;
    host.relation(
        Relation::Midpoint,
        vec![point(centerline.entities[0]), entity(r[7])],
        None,
    )
    .await;
    host.relation(
        Relation::Midpoint,
        vec![point(centerline.entities[1]), entity(r[5])],
        None,
    )
    .await;
    Plate {
        rectangle,
        holes: [first, second],
        centerline,
        width,
        diameters,
    }
}

async fn constrain_holes(
    host: &mut Harness,
    rectangle: &PrimitiveIds,
    holes: [&PrimitiveIds; 2],
) -> [ConstraintId; 2] {
    let lower_left = point(rectangle.entities[0]);
    let lower_right = point(rectangle.entities[1]);
    let first = SketchTarget::Point {
        entity: holes[0].entities[1],
        role: CurvePosition::Center,
    };
    let second = SketchTarget::Point {
        entity: holes[1].entities[1],
        role: CurvePosition::Center,
    };
    host.relation(
        Relation::HorizontalDistance,
        vec![lower_left, first],
        Some("15"),
    )
    .await;
    host.relation(
        Relation::VerticalDistance,
        vec![lower_left, first],
        Some("25"),
    )
    .await;
    let first_diameter = host
        .relation(
            Relation::Diameter,
            vec![entity(holes[0].entities[1])],
            Some("6"),
        )
        .await;
    host.relation(
        Relation::HorizontalDistance,
        vec![lower_right, second],
        Some("-15"),
    )
    .await;
    host.relation(Relation::HorizontalPoints, vec![first, second], None)
        .await;
    let second_diameter = host
        .relation(
            Relation::Diameter,
            vec![entity(holes[1].entities[1])],
            Some("6"),
        )
        .await;
    [first_diameter, second_diameter]
}

async fn remove_and_repair(host: &mut Harness, plate: &Plate) {
    let id = plate.diameters[1];
    crate::sketch_repair::remove_constraint(&mut host.app, id);
    host.wait(|app| {
        app.doc.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active && !session.authoritative.contains_constraint(id)
        })
    })
    .await;
    assert_eq!(
        crate::sketch_feedback::summary(&host.app.doc.sketch.session.as_ref().unwrap().status)
            .assessment,
        crate::sketch_feedback::Assessment::UnderConstrained
    );
    host.sketch_history(HistoryDirection::Undo).await;
    assert_plate(host, plate, 90.0);
    assert!(
        host.app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .history
            .as_ref()
            .is_some_and(|history| history.redo_depth > 0)
    );
}

async fn extrude(host: &mut Harness) {
    let SketchStatus::Finished { result, .. } =
        &host.app.doc.sketch.session.as_ref().unwrap().status
    else {
        panic!("Finish result")
    };
    let material_region = &result
        .regions
        .iter()
        .find(|region| region.holes.len() == 2)
        .unwrap()
        .region_id;
    let display = host.app.doc.display.as_ref().unwrap();
    let profile = display
        .profiles
        .iter()
        .find(|profile| profile.reference.region.to_string() == *material_region)
        .unwrap();
    let context = host.app.doc.interaction.context.clone().unwrap();
    let view = ViewSnapshot::new(
        context.scene,
        host.app.view_number + 1,
        Default::default(),
        onecad_scene::Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let world = host
        .sketch()
        .effective_plane()
        .to_world(Vec2::new(45.0, 10.0).unwrap());
    let logical = view
        .project(DVec3::new(world.x, world.y, world.z))
        .unwrap()
        .logical;
    let (anchor, distance) = profile
        .pick(&view, logical)
        .expect("material profile picked through shared captured-view helper");
    assert!(distance.is_finite() && distance > 0.0);
    near(anchor[0], 45.0);
    near(anchor[1], 10.0);
    let mut reference = profile.reference.clone();
    reference.region_anchor = Some(anchor);
    let step_index = display.projection.applied_ops as u32;
    host.app
        .dispatch(Action::SelectProfile(Box::new(ProfileSelection {
            profile: reference.clone(),
            context,
            capture: view.identity(),
        })));
    let record = RecordId::new();
    host.app.dispatch(Action::BeginExtrude {
        record_id: record,
        gesture: 1,
        step_index,
    });
    host.app.dispatch(Action::DraftChanged("4".into()));
    host.wait(|app| {
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            matches!(&tool.draft_status, DraftStatus::Valid(value) if value.value == 4.0)
                && tool.preview == PreviewStatus::Ready
        })
    })
    .await;
    host.app.dispatch(Action::Confirm);
    host.wait(|app| {
        matches!(
            &app.doc.interaction.status,
            CommitStatus::Completed {
                outcome: FeatureOutcome::Published,
                ..
            }
        )
    })
    .await;
    let display = host.app.doc.display.as_ref().unwrap();
    assert_mesh_bounds(&host.app, 90.0);
    assert_eq!(display.scene.bodies().len(), 1);
    let operation = display
        .records
        .iter()
        .find(|operation| operation.record_id == record)
        .unwrap();
    assert!(
        matches!(&operation.op, Operation::Known(KnownOperation::Extrude(parameters)) if parameters.distance.value == 4.0 && parameters.profile.as_ref() == Some(&reference))
    );
}

async fn model_history(host: &mut Harness) {
    crate::input::submit_history(&mut host.app, Command::Undo);
    host.wait(|app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|display| display.scene.bodies().is_empty())
    })
    .await;
    crate::input::submit_history(&mut host.app, Command::Redo);
    host.wait(|app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|display| display.scene.bodies().len() == 1)
    })
    .await;
    assert_mesh_bounds(&host.app, 90.0);
}

async fn downstream_width_edits(host: &mut Harness, plate: &Plate) {
    let sketch_id = host.sketch().id;
    let display = host.app.doc.display.as_ref().unwrap();
    let body_id = display.scene.bodies()[0].body_id();
    let feature_ids: Vec<_> = display
        .projection
        .features
        .iter()
        .map(|feature| feature.id.clone())
        .collect();
    let extrusion = display
        .records
        .iter()
        .find(|record| matches!(record.op, Operation::Known(KnownOperation::Extrude(_))))
        .unwrap()
        .clone();
    for (text, width) in [("80", 80.0), ("90", 90.0)] {
        host.app.edit_sketch(sketch_id);
        host.wait(|app| {
            app.doc
                .sketch
                .session
                .as_ref()
                .is_some_and(|session| session.phase == SketchPhase::Active)
        })
        .await;
        host.edit_width(plate.width, text).await;
        assert_plate(host, plate, width);
        host.finish().await;
        assert_finished_profile(host, plate);
        assert_mesh_bounds(&host.app, width);
        assert_brep(host, width).await;
        let display = host.app.doc.display.as_ref().unwrap();
        assert_eq!(display.scene.bodies()[0].body_id(), body_id);
        assert_eq!(
            display
                .projection
                .features
                .iter()
                .map(|feature| feature.id.clone())
                .collect::<Vec<_>>(),
            feature_ids
        );
        let regenerated = display
            .records
            .iter()
            .find(|record| record.record_id == extrusion.record_id)
            .unwrap();
        assert_eq!(
            regenerated, &extrusion,
            "downstream extrusion identity and intended profile reference stay fixed"
        );
    }
}
