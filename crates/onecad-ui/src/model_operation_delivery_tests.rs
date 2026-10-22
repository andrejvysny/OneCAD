//! Fillet and Move / Rotate through the real host route with the production
//! worker and Metal, addressed by the AccessKit automation ids the iPadOS bridge
//! exports. Synthetic pointer delivery is not physical acceptance.
use super::model_selection_delivery_tests::{boxed, click, labels, press, targets};
use super::point_center_delivery_tests::support::Host;
use super::*;
use crate::{automation as id, tool_registry::ToolId};
use onecad_core::document::record::{KnownOperation, Operation};
use onecad_interaction::{PreviewStatus, selection_set::SelectionItem};
use onecad_scene::PickKind;

/// One frame's named AccessKit nodes: the author ids the iPadOS bridge exports
/// as automation identifiers, with their bounds.
fn automation(host: &mut Host) -> Vec<(String, egui::Rect)> {
    let mut raw = egui::RawInput {
        screen_rect: Some(host.screen()),
        focused: true,
        ..Default::default()
    };
    crate::input::route(&mut host.app, &host.context, &mut raw);
    let mut output = host.context.run_ui(raw, |ui| {
        eframe::App::ui(&mut host.app, ui, &mut host.frame)
    });
    output.textures_delta.clear();
    let update = output
        .platform_output
        .accesskit_update
        .expect("AccessKit is enabled");
    update
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            let bounds = node.bounds()?;
            let min = egui::pos2(bounds.x0 as f32, bounds.y0 as f32);
            let max = egui::pos2(bounds.x1 as f32, bounds.y1 as f32);
            Some((
                node.author_id()?.to_owned(),
                egui::Rect::from_min_max(min, max),
            ))
        })
        .collect()
}

/// The control `id` names once it is laid out; fails listing what is named.
fn named(host: &mut Host, id: &str) -> egui::Rect {
    let mut seen = Vec::new();
    for _ in 0..20 {
        seen = automation(host);
        if let Some((_, rect)) = seen.iter().find(|(name, _)| name == id) {
            return *rect;
        }
    }
    let ids: Vec<_> = seen.iter().map(|(name, _)| name.as_str()).collect();
    panic!("{id} missing; named: {ids:?}")
}

/// Clicks a control by automation id, the way an XCUITest addresses it.
fn tap(host: &mut Host, id: &str) {
    let center = named(host, id).center();
    host.click(center);
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn multi_select_switch_and_pill_fillet_commit_through_the_card() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let edges = targets(&host, PickKind::Edge);
    assert!(edges.len() >= 2, "{edges:?}");
    named(&mut host, id::VIEWPORT);
    tap(&mut host, id::ADDITIVE_SELECT);
    assert!(host.app.ui.additive_select);
    click(&mut host, edges[0].0, egui::Modifiers::NONE);
    click(&mut host, edges[1].0, egui::Modifiers::NONE);
    let both = [edges[0].1.clone(), edges[1].1.clone()];
    assert_eq!(
        labels(&host),
        both,
        "plain picks add while the switch is on"
    );
    click(&mut host, edges[1].0, egui::Modifiers::NONE);
    assert_eq!(
        labels(&host),
        std::slice::from_ref(&edges[0].1),
        "a repeat pick removes"
    );
    tap(&mut host, id::ADDITIVE_SELECT);
    assert!(!host.app.ui.additive_select);
    let before = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .projection
        .features
        .len();
    named(&mut host, &id::tool(ToolId::Chamfer));
    tap(&mut host, &id::tool(ToolId::Fillet));
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| tool.preview == PreviewStatus::Ready)
    })
    .await;
    named(&mut host, id::DIMENSION_INPUT);
    named(&mut host, id::OP_CANCEL);
    tap(&mut host, id::OP_CONFIRM);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.is_none()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|display| display.projection.features.len() == before + 1)
    })
    .await;
    let display = host.app.doc.display.clone().unwrap();
    assert_eq!(
        display.projection.features.last().unwrap().op_type,
        "Fillet"
    );
    host.app.ui.inspector_open = true;
    host.app.ui.inspector_tab = crate::ui_state::InspectorTab::History;
    named(&mut host, &id::history(before));
    named(&mut host, &id::tab(0));
    host.stop().await;
}

/// Two clicks inside egui's double-click window: a touch double tap.
fn double_tap(host: &mut Host, pos: egui::Pos2) {
    click(host, pos, egui::Modifiers::NONE);
    host.ui(vec![press(pos, true, egui::Modifiers::NONE)]);
    host.ui(vec![press(pos, false, egui::Modifiers::NONE)]);
}

/// Bounds centre and extent of the one body in the published scene.
fn placement(host: &Host) -> ([f64; 3], [f64; 3]) {
    let display = host.app.doc.display.as_ref().unwrap();
    let bounds = display.scene.bodies()[0].bounds();
    let centre = ((bounds.min + bounds.max) * 0.5).to_array();
    (centre, (bounds.max - bounds.min).to_array())
}

/// Types `text` into the field an automation id names, replacing its value.
fn enter(host: &mut Host, field: &str, text: &str) {
    tap(host, field);
    host.replace(text);
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn double_tapped_body_moves_and_turns_by_typed_values() {
    use onecad_interaction::{NumericParameter, ParameterAxis};
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let faces = targets(&host, PickKind::Face);
    double_tap(&mut host, faces[0].0);
    assert!(
        matches!(host.app.doc.selection.items(), [SelectionItem::Body(_)]),
        "a double tap selects the whole body"
    );
    let (centre, extent) = placement(&host);
    let before = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .projection
        .features
        .len();
    tap(&mut host, &id::tool(ToolId::MoveRotate));
    named(&mut host, id::DIMENSION_INPUT);
    let y = id::param(NumericParameter::TransformTranslation(ParameterAxis::Y));
    enter(&mut host, &y, "5");
    enter(
        &mut host,
        &id::param(NumericParameter::TransformRotationAngle),
        "90",
    );
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            tool.preview == PreviewStatus::Ready
                && matches!(tool.effective_op(), Operation::Known(KnownOperation::TransformBody(p))
                    if p.translate[1].value == 5.0 && p.rotate.angle_deg.value == 90.0)
        })
    })
    .await;
    tap(&mut host, id::OP_CONFIRM);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.is_none()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|display| display.projection.features.len() == before + 1)
    })
    .await;
    host.redraw();
    let display = host.app.doc.display.clone().unwrap();
    assert_eq!(
        display.projection.features.last().unwrap().op_type,
        "TransformBody"
    );
    let (moved, turned) = placement(&host);
    let shift = [0.0, 5.0, 0.0];
    for axis in 0..3 {
        assert!(
            (moved[axis] - centre[axis] - shift[axis]).abs() < 1e-4,
            "{moved:?}"
        );
    }
    for (axis, swapped) in [(0, 1), (1, 0), (2, 2)] {
        assert!(
            (turned[axis] - extent[swapped]).abs() < 1e-4,
            "{turned:?} {extent:?}"
        );
    }
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn selection_change_while_staging_refuses_the_start() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let edges = targets(&host, PickKind::Edge);
    click(&mut host, edges[0].0, egui::Modifiers::NONE);
    tap(&mut host, &id::tool(ToolId::Chamfer));
    // `Host::ui` runs no logic tick, so the staging reply is still unread.
    assert!(crate::operation_start::pending(&host.app));
    crate::item_selection::retain(&mut host.app, |_| false);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !crate::operation_start::pending(app)
    })
    .await;
    assert!(host.app.doc.interaction.tool.is_none());
    assert!(
        host.app.doc.status.contains("selection changed"),
        "{}",
        host.app.doc.status
    );
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn sketch_chooser_planes_and_strip_actions_are_named() {
    use onecad_core::sketch::WorldPlane;
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    crate::item_selection::retain(&mut host.app, |_| false);
    tap(&mut host, &id::tool(ToolId::Sketch));
    named(&mut host, id::plane(WorldPlane::XZ));
    named(&mut host, id::plane(WorldPlane::YZ));
    tap(&mut host, id::plane(WorldPlane::XY));
    // Finish enables once the opened session is Active (`sketch_ui::can_exit`).
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == onecad_interaction::sketch::SketchPhase::Active)
    })
    .await;
    let plane = host.app.doc.sketch.session.as_ref().unwrap().plane;
    assert_eq!(plane, WorldPlane::XY.plane());
    named(&mut host, &id::tool(ToolId::Line));
    named(&mut host, id::DISCARD_SKETCH);
    tap(&mut host, id::FINISH_SKETCH);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_active()
    })
    .await;
    host.stop().await;
}
