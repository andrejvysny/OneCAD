use super::*;

pub(super) fn pointer(host: &mut Host, x: f64, y: f64) {
    host.redraw();
    host.ui(vec![egui::Event::PointerMoved(host.project(x, y))]);
}

pub(super) fn undo_depth(host: &Host) -> usize {
    host.app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .as_ref()
        .unwrap()
        .undo_depth
}

pub(super) fn tangent_mode(host: &Host) -> bool {
    host.app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .is_some_and(|draft| draft.tangent_arc)
}

pub(super) fn latest_arc(sketch: &Sketch) -> EntityId {
    sketch
        .entities()
        .iter()
        .rev()
        .find(|entity| matches!(entity, SketchEntity::Arc { .. }))
        .unwrap()
        .id()
}

pub(super) fn endpoint(
    sketch: &Sketch,
    entity: EntityId,
    role: CurvePosition,
) -> onecad_core::math::Vec2 {
    SketchTarget::Point { entity, role }
        .point_coordinates(sketch)
        .unwrap()
}

pub(super) fn assert_radius(sketch: &Sketch, expected: f64, expression: bool) {
    assert!(
        sketch
            .constraints()
            .iter()
            .any(|constraint| matches!(constraint,
        Constraint::Radius { value, .. } if (value.value - expected).abs() < 1e-5
            && value.expr.is_some() == expression))
    );
    let SketchEntity::Arc { radius, .. } = sketch.get_entity(latest_arc(sketch)).unwrap() else {
        panic!("Arc expected")
    };
    near(*radius, expected);
}

pub(super) fn numeric_one(host: &mut Host, text: &str) {
    host.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    assert!(crate::sketch_numeric::editor_open(&host.context));
    host.ui(vec![]);
    assert!(
        host.widgets
            .iter()
            .any(|(label, _)| label == "Radius" || label == "Width"),
        "numeric controls: {:?}",
        host.widgets
            .iter()
            .map(|(label, _)| label)
            .collect::<Vec<_>>()
    );
    assert!(!host.widgets.iter().any(|(label, _)| label == "Length"));
    let before = host.sketch().clone();
    let depth = undo_depth(host);
    host.replace(text);
    assert_eq!(host.sketch(), &before);
    assert_eq!(undo_depth(host), depth);
    host.ui(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert!(!crate::sketch_numeric::editor_open(&host.context));
}

pub(super) fn refusal(host: &mut Host, text: &str) {
    let accepted = host.sketch().clone();
    let history = host.app.sketch.session.as_ref().unwrap().history.clone();
    host.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    assert!(crate::sketch_numeric::editor_open(&host.context));
    host.replace(text);
    host.ui(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert_eq!(host.sketch(), &accepted);
    assert_eq!(host.app.sketch.session.as_ref().unwrap().history, history);
    if !crate::sketch_numeric::editor_open(&host.context) {
        host.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    }
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(!crate::sketch_numeric::editor_open(&host.context));
    assert_eq!(host.sketch(), &accepted);
}

pub(super) fn assert_alignment(
    sketch: &Sketch,
    arc: EntityId,
    role: CurvePosition,
    vertical: bool,
) {
    assert!(
        sketch.constraints().iter().any(|constraint| {
            let reference = match constraint {
                Constraint::VerticalPoints {
                    point2,
                    point2_position,
                    ..
                } if vertical => Some((*point2, *point2_position)),
                Constraint::HorizontalPoints {
                    point2,
                    point2_position,
                    ..
                } if !vertical => Some((*point2, *point2_position)),
                _ => None,
            };
            reference == Some((arc, role))
        }),
        "Arc endpoint must own persistent H/V inference: {:?}",
        sketch.constraints()
    );
}

pub(super) fn assert_ignored_keys(host: &mut Host) {
    for modifiers in [
        egui::Modifiers::CTRL,
        egui::Modifiers::ALT,
        egui::Modifiers::COMMAND,
    ] {
        host.ui(vec![key(egui::Key::A, modifiers)]);
        assert!(!tangent_mode(host));
    }
    let mut repeated = key(egui::Key::A, egui::Modifiers::NONE);
    if let egui::Event::Key { repeat, .. } = &mut repeated {
        *repeat = true;
    }
    host.ui(vec![repeated]);
    assert!(!tangent_mode(host));
}

pub(super) fn assert_numeric_keyboard_ownership(host: &mut Host) {
    host.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    assert!(crate::sketch_numeric::editor_open(&host.context));
    host.ui(vec![
        key(egui::Key::A, egui::Modifiers::NONE),
        egui::Event::Text("a".into()),
    ]);
    assert!(!tangent_mode(host));
    host.ui(vec![
        egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "候補".into(),
            active_range_chars: None,
        }),
        key(egui::Key::A, egui::Modifiers::NONE),
    ]);
    assert!(!tangent_mode(host));
    assert!(crate::sketch_numeric::editor_open(&host.context));
    host.ui(vec![egui::Event::Ime(egui::ImeEvent::Commit("8".into()))]);
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(!crate::sketch_numeric::editor_open(&host.context));
}

pub(super) async fn arc_to_arc_and_stale_capture(host: &mut Host) {
    let draft = host
        .app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    let curve = draft.preceding.unwrap();
    let anchor = draft.anchor;
    let SketchEntity::Arc {
        start_angle,
        end_angle,
        ..
    } = host.sketch().get_entity(curve.entity).unwrap()
    else {
        panic!("Arc chain expected")
    };
    let (tx, ty) = if curve.role == CurvePosition::End {
        (-end_angle.sin(), end_angle.cos())
    } else {
        (start_angle.sin(), -start_angle.cos())
    };
    host.ui(vec![key(egui::Key::A, egui::Modifiers::NONE)]);
    pointer(host, anchor.x + 8.0 * (tx - ty), anchor.y + 8.0 * (ty + tx));
    numeric_one(host, "8");
    host.count(7).await;
    let next = latest_arc(host.sketch());
    assert!(
        host.sketch()
            .constraints()
            .iter()
            .any(|constraint| matches!(constraint,
        Constraint::TangentAtPoint { entity1, entity1_position, entity2, .. }
        if *entity1 == curve.entity && *entity1_position == curve.role && *entity2 == next))
    );
    let before = host.sketch().clone();
    let depth = undo_depth(host);
    host.redraw();
    let anchor = host
        .app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .anchor;
    let from = host.project(anchor.x, anchor.y);
    let to = from + egui::vec2(20.0, 20.0);
    host.ui(vec![egui::Event::PointerMoved(from), button(from, true)]);
    host.ui(vec![egui::Event::PointerMoved(to)]);
    host.app
        .fail_viewport("Injected Arc chain capture failure".into());
    host.ui(vec![button(to, false)]);
    assert_eq!(host.sketch(), &before);
    assert_eq!(undo_depth(host), depth);
}

pub(super) async fn batched_drag_release(root: &std::path::Path, single_frame: bool) {
    let mut host = Host::new(root).await;
    host.control("Line");
    host.xy(-20.0, -10.0);
    host.xy(0.0, -10.0);
    host.count(3).await;
    host.redraw();
    let draft = host
        .app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    let from = host.project(draft.anchor.x, draft.anchor.y);
    let to = host.project(draft.anchor.x + 10.0, draft.anchor.y + 10.0);
    let mut events = vec![egui::Event::PointerMoved(from), button(from, true)];
    if !single_frame {
        host.ui(events);
        events = vec![];
    }
    events.extend([egui::Event::PointerMoved(to), button(to, false)]);
    host.ui(events);
    host.count(5).await;
    assert_eq!(undo_depth(&host), 2);
    assert_eq!(
        host.sketch()
            .entities()
            .iter()
            .filter(|entity| matches!(entity, SketchEntity::Arc { .. }))
            .count(),
        1
    );
    host.ui(vec![button(to, false)]);
    assert_eq!(undo_depth(&host), 2);
    host.stop().await;
}

pub(super) async fn cancelled_drag_ownership(root: &std::path::Path) {
    let mut host = Host::new(root).await;
    host.control("Line");
    host.xy(-20.0, -10.0);
    host.xy(0.0, -10.0);
    host.count(3).await;
    let before = host.sketch().clone();
    let line = before
        .entities()
        .iter()
        .find(|entity| matches!(entity, SketchEntity::Line { .. }))
        .unwrap()
        .id();
    let end = endpoint(&before, line, CurvePosition::End);
    for cancellation in [0, 1, 2] {
        host.control("Line");
        host.app.ui.sketch_snap = true;
        host.xy(end.x, end.y);
        let from = host.project(end.x, end.y);
        let to = host.project(end.x + 10.0, end.y + 10.0);
        host.ui(vec![egui::Event::PointerMoved(from), button(from, true)]);
        host.ui(vec![egui::Event::PointerMoved(to)]);
        assert!(tangent_mode(&host));
        if cancellation == 0 {
            let widget = host.rect("Line").center();
            host.ui(vec![
                egui::Event::PointerMoved(widget),
                button(widget, false),
            ]);
        } else if cancellation == 1 {
            host.ui(vec![egui::Event::PointerGone]);
            host.ui(vec![button(to, false)]);
        } else {
            host.app.composing = true;
            host.ui(vec![button(to, false)]);
            host.app.composing = false;
        }
        assert_eq!(host.sketch(), &before);
        assert_eq!(undo_depth(&host), 1);
    }
    host.stop().await;
}

pub(super) fn assert_native_arc_endpoint_seeding(host: &mut Host) {
    let accepted = host.sketch().clone();
    let depth = undo_depth(host);
    let arc = latest_arc(&accepted);
    host.app.ui.sketch_snap = true;
    for role in [CurvePosition::Start, CurvePosition::End] {
        host.control("Line");
        let at = endpoint(&accepted, arc, role);
        host.xy(at.x, at.y);
        let session = host.app.sketch.session.as_ref().unwrap();
        let draft = session.draft.as_ref().unwrap();
        let curve = draft
            .preceding
            .expect("unique Arc endpoint must seed a chain");
        assert_eq!((curve.entity, curve.role), (arc, role));
        assert_eq!(draft.anchor_id, None);
        assert_eq!(
            draft.first_target,
            Some(SketchTarget::Point { entity: arc, role })
        );
        assert!(session.can_tangent_arc());
        host.ui(vec![key(egui::Key::A, egui::Modifiers::NONE)]);
        assert!(tangent_mode(host));
        host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert!(host.app.sketch.session.as_ref().unwrap().draft.is_none());
        assert_eq!(host.sketch(), &accepted);
        assert_eq!(undo_depth(host), depth);
    }
    host.app.ui.sketch_snap = false;
}
