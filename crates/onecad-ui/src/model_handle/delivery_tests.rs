//! Production worker + Metal + real host input routing. Synthetic input is not physical qualification.
use super::{
    extrude_handle_delivery_tests::{confirm, extruding, undo_depth},
    point_center_delivery_tests::support::*,
    pump_until,
};
use crate::{app::App, model_handle};
use onecad_core::{
    document::record::{BooleanOp, KnownOperation, Operation, OperationRecord},
    ids::RecordId,
};
use onecad_interaction::{
    Action, NumericParameter as N, PreviewStatus, modeling_records, operand_binding,
};
use onecad_scene::{Camera, DVec3, Projection};

fn canvas(host: &Host) -> egui::Rect {
    crate::layout::chrome(host.screen(), &host.app.ui).viewport
}
fn tip(host: &Host, parameter: Option<N>, value: f64) -> egui::Pos2 {
    model_handle::testing_tip(&host.app, canvas(host), parameter, value)
        .expect("resolved model handle")
}
fn value(host: &Host, key: N) -> f64 {
    host.app
        .doc
        .interaction
        .tool
        .as_ref()
        .unwrap()
        .value(key)
        .unwrap()
}
fn text(host: &Host, key: N) -> &str {
    &host
        .app
        .doc
        .interaction
        .tool
        .as_ref()
        .unwrap()
        .params
        .field(key.into())
        .unwrap()
        .text
}
async fn begin(host: &mut Host, record: OperationRecord) {
    host.app.doc.gesture += 1;
    let context = host.app.doc.interaction.context.clone().unwrap();
    let action = operand_binding::begin_bound(record, host.app.doc.gesture, context).unwrap();
    host.app.dispatch(action);
    ready(host).await;
}
async fn ready(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|t| t.preview == PreviewStatus::Ready)
    })
    .await;
    host.redraw();
}
async fn plate(root: &std::path::Path) -> Host {
    let (mut host, _) = extruding(root, "12", 12.0).await;
    confirm(&mut host).await;
    host.app.doc.camera = Camera {
        offset: DVec3::new(100.0, -180.0, 130.0),
        projection: Projection::Perspective,
        ..host.app.doc.camera
    };
    host.redraw();
    host
}
fn hole_pick(app: &App) -> onecad_interaction::selection_set::SelectionItem {
    let body = &app.doc.display.as_ref().unwrap().scene.bodies()[0];
    let mesh = body.mesh().unwrap();
    let (face, point) = (0..mesh.header().face_count as usize)
        .find_map(|face| {
            let range = mesh.face_range(face)?;
            let [a, b, c] = mesh
                .triangle(range.first as usize)?
                .map(|i| DVec3::from_array(mesh.position(i as usize).unwrap().map(f64::from)));
            ((b - a).cross(c - a).try_normalize()?.dot(DVec3::Z) > 0.99)
                .then_some((face, (a + b + c) / 3.0))
        })
        .unwrap();
    let _ = point;
    onecad_interaction::selection_set::SelectionItem::Face(
        onecad_interaction::selection_set::TopologyRef {
            scene: app.doc.display.as_ref().unwrap().scene.identity(),
            body: body.body_id(),
            id: crate::model_pick::topology_id(mesh.face_id(face).unwrap()),
        },
    )
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn native_hole_scalar_click_drag_release_and_losses_preserve_draft_history_and_ownership() {
    let root = tempfile::tempdir().unwrap();
    let mut host = plate(root.path()).await;
    let pick = hole_pick(&host.app);
    crate::item_selection::apply(
        &mut host.app,
        onecad_interaction::selection_set::SelectionMode::Replace,
        vec![pick],
    );
    crate::operation_start::begin_hole(&mut host.app);
    ready(&mut host).await;
    let Operation::Known(KnownOperation::Hole(hole)) = host
        .app
        .doc
        .interaction
        .tool
        .as_ref()
        .unwrap()
        .effective_op()
    else {
        panic!()
    };
    let promoted = &hole.face.primary.as_ref().unwrap().element;
    let body = &host.app.doc.display.as_ref().unwrap().scene.bodies()[0];
    let mesh = body.mesh().unwrap();
    assert!(
        !(0..mesh.header().face_count as usize).any(|i| mesh.face_id(i) == Some(promoted.as_str())),
        "new staging must retain geometry even before promoted ids reach MESH1"
    );
    host.app.dispatch(Action::ParameterChanged {
        key: N::HoleDiameter.into(),
        text: "2 * 3".into(),
    });
    ready(&mut host).await;
    let undo = undo_depth(&host);
    let sequence = host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence;
    let start = tip(&host, Some(N::HoleDiameter), 6.0);
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        button(start, false),
    ]);
    assert_eq!(text(&host, N::HoleDiameter), "2 * 3");
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence,
        sequence
    );
    for loss in 0..4 {
        let start = tip(&host, Some(N::HoleDiameter), 6.0);
        let end = tip(&host, Some(N::HoleDiameter), 26.0);
        host.ui(vec![
            egui::Event::PointerMoved(start),
            button(start, true),
            egui::Event::PointerMoved(end),
        ]);
        assert!(host.app.doc.model_handle.active());
        assert_ne!(text(&host, N::HoleDiameter), "2 * 3");
        match loss {
            0 => host.ui(vec![egui::Event::PointerGone]),
            1 => host.ui_focused(vec![], false),
            2 => host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]),
            _ => {
                host.app.doc.camera.roll_radians += 0.1;
                host.ui(vec![]);
            }
        }
        assert!(!host.app.doc.model_handle.active());
        assert_eq!(text(&host, N::HoleDiameter), "2 * 3");
        host.ui(vec![button(end, false)]);
        ready(&mut host).await;
    }
    let start = tip(&host, Some(N::HoleDiameter), 6.0);
    let end = tip(&host, Some(N::HoleDiameter), 26.0);
    let camera = host.app.doc.camera;
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        egui::Event::PointerMoved(end),
        button(end, false),
    ]);
    ready(&mut host).await;
    assert!((value(&host, N::HoleDiameter) - 26.0).abs() < 0.2);
    assert_eq!(host.app.doc.camera, camera, "handle owns motion, no orbit");
    assert_eq!(undo_depth(&host), undo, "release authors no history");
    assert!(host.app.doc.interaction.tool.is_some());
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn native_pattern_angle_and_mirror_plane_drag_route_real_pointer_events() {
    let root = tempfile::tempdir().unwrap();
    let mut host = plate(root.path()).await;
    let id = host.app.doc.display.as_ref().unwrap().scene.bodies()[0].body_id();
    begin(
        &mut host,
        modeling_records::circular_pattern(id, RecordId::new(), 1),
    )
    .await;
    let undo = undo_depth(&host);
    let start = tip(&host, Some(N::CircularPatternAngle), 360.0);
    let end = tip(&host, Some(N::CircularPatternAngle), 300.0);
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        egui::Event::PointerMoved(end),
        button(end, false),
    ]);
    ready(&mut host).await;
    assert!((value(&host, N::CircularPatternAngle) - 300.0).abs() < 0.2);
    assert_eq!(undo_depth(&host), undo);
    host.app.dispatch(Action::Cancel);
    begin(&mut host, modeling_records::mirror(id, RecordId::new(), 1)).await;
    let original = host
        .app
        .doc
        .interaction
        .tool
        .as_ref()
        .unwrap()
        .record
        .clone();
    let start = tip(&host, None, 0.0);
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        button(start, false),
    ]);
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().record,
        original
    );
    let end = tip(&host, None, 8.0);
    host.ui(vec![button(start, true), egui::Event::PointerMoved(end)]);
    assert!(
        host.app.doc.model_handle.active(),
        "mirror press {start:?}, end {end:?}, screen {:?}, status {}",
        host.screen(),
        host.app.doc.status
    );
    assert_ne!(
        host.app.doc.interaction.tool.as_ref().unwrap().record,
        original
    );
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().record,
        original
    );
    host.ui(vec![button(end, false)]);
    ready(&mut host).await;
    let start = tip(&host, None, 0.0);
    let end = tip(&host, None, 8.0);
    host.ui(vec![
        button(start, true),
        egui::Event::PointerMoved(end),
        button(end, false),
    ]);
    ready(&mut host).await;
    let Operation::Known(KnownOperation::MirrorBody(p)) = host
        .app
        .doc
        .interaction
        .tool
        .as_ref()
        .unwrap()
        .effective_op()
    else {
        panic!()
    };
    assert!((p.plane_point.x - 8.0).abs() < 0.2);
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn native_boolean_viewport_controls_own_click_and_only_author_options() {
    let root = tempfile::tempdir().unwrap();
    let mut host = plate(root.path()).await;
    let id = host.app.doc.display.as_ref().unwrap().scene.bodies()[0].body_id();
    let mut mirror = modeling_records::mirror(id, RecordId::new(), 1);
    if let Operation::Known(KnownOperation::MirrorBody(p)) = &mut mirror.op {
        p.plane_point.x = -10.0;
    }
    begin(&mut host, mirror).await;
    confirm(&mut host).await;
    let ids: Vec<_> = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .scene
        .bodies()
        .iter()
        .map(|b| b.body_id())
        .collect();
    begin(
        &mut host,
        modeling_records::boolean(&ids, RecordId::new(), 2).unwrap(),
    )
    .await;
    let undo = undo_depth(&host);
    let before = host.app.doc.selection.items().to_vec();
    let subtract = model_handle::testing_boolean_button(&host.app, canvas(&host), 1).unwrap();
    let chrome = crate::layout::chrome(host.screen(), &host.app.ui).with_feature_card();
    let cube = crate::navigation_cube::rect(egui::Rect::from_min_max(
        egui::pos2(canvas(&host).left(), chrome.view_cluster.y),
        egui::pos2(chrome.view_cluster.x, canvas(&host).bottom()),
    ))
    .center();
    let sequence = host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence;
    host.ui(vec![
        egui::Event::PointerMoved(subtract),
        button(subtract, true),
    ]);
    host.ui(vec![egui::Event::PointerMoved(cube), button(cube, false)]);
    host.ui(vec![
        egui::Event::PointerMoved(subtract),
        button(subtract, false),
    ]);
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence,
        sequence,
        "release over cube clears button capture"
    );
    let union = model_handle::testing_boolean_button(&host.app, canvas(&host), 0).unwrap();
    host.click(union);
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence,
        sequence,
        "same option is a no-op"
    );
    for (index, expected) in [
        (1, BooleanOp::Cut),
        (2, BooleanOp::Intersect),
        (0, BooleanOp::Union),
    ] {
        let pos = model_handle::testing_boolean_button(&host.app, canvas(&host), index).unwrap();
        let sequence = host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence;
        host.click(pos);
        assert_eq!(
            host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence,
            sequence + 1,
            "one release authors one choice"
        );
        let Operation::Known(KnownOperation::Boolean(p)) = host
            .app
            .doc
            .interaction
            .tool
            .as_ref()
            .unwrap()
            .effective_op()
        else {
            panic!()
        };
        assert_eq!(p.operation, expected);
        assert_eq!(host.app.doc.selection.items(), before.as_slice());
        assert_eq!(undo_depth(&host), undo);
        host.redraw();
    }
    host.stop().await;
}

#[path = "history_regressions.rs"]
mod history_regressions;
