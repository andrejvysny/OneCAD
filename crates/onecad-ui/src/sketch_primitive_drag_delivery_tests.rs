//! Confirmed Pencil samples reach the production sketch service, without physical-input qualification.
use super::point_center_delivery_tests::support::*;
use onecad_core::sketch::SketchEntity;
use onecad_interaction::sketch::{DraftPrimitive, SketchAction, SketchTool};

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn pencil_rectangle_drag_commits_on_release_once_and_undo_redo() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Box::pin(Host::new(root.path())).await;
    host.control("Rectangle");
    crate::viewport_widgets::set_pointer_kind(
        &host.context,
        crate::viewport_widgets::PointerKind::Pencil,
    );
    host.redraw();
    let start = host.project(-20.0, -10.0);
    let end = host.project(20.0, 10.0);
    let before = host.sketch().clone();
    let depth = host
        .app
        .doc
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .as_ref()
        .unwrap()
        .undo_depth;
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    host.ui(vec![egui::Event::PointerMoved(end)]);
    assert_eq!(
        host.sketch(),
        &before,
        "a drag must not mutate before release"
    );
    assert!(
        matches!(crate::sketch_creation::primitive_drag::preview(&host.context),
        Some(DraftPrimitive::Rectangle { first, opposite })
        if (first.x + 20.0).abs() < 1e-4 && (first.y + 10.0).abs() < 1e-4
            && (opposite.x - 20.0).abs() < 1e-4 && (opposite.y - 10.0).abs() < 1e-4)
    );
    host.ui(vec![button(end, false)]);
    assert_eq!(
        host.app.doc.sketch.session.as_ref().unwrap().phase,
        onecad_interaction::sketch::SketchPhase::Submitting,
        "raw rectangle drag must submit on its confirmed release"
    );
    host.count(8).await;
    assert_eq!(
        host.app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .history
            .as_ref()
            .unwrap()
            .undo_depth,
        depth + 1
    );
    host.history(false, 0).await;
    host.history(true, 8).await;
    host.stop().await;
}

fn depth(host: &Host) -> usize {
    host.app
        .doc
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .as_ref()
        .unwrap()
        .undo_depth
}

fn pencil(host: &Host) {
    crate::viewport_widgets::set_pointer_kind(
        &host.context,
        crate::viewport_widgets::PointerKind::Pencil,
    );
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn pencil_exact_tools_same_frame_release_ignores_later_hover_and_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Box::pin(Host::new(root.path())).await;
    pencil(&host);
    let mut count = 0;
    for (tool, added) in [
        (SketchTool::Circle, 2),
        (SketchTool::CenterRectangle, 10),
        (SketchTool::Line, 3),
    ] {
        host.app.sketch_dispatch(SketchAction::SetTool(tool));
        host.redraw();
        let from = host.project(-20.0, -10.0);
        let end = host.project(10.0, 10.0);
        let hover = host.project(30.0, 20.0);
        let before = host.sketch().clone();
        let undo = depth(&host);
        host.ui(vec![
            egui::Event::PointerMoved(from),
            button(from, true),
            egui::Event::PointerMoved(end),
            button(end, false),
            egui::Event::PointerMoved(hover),
            egui::Event::PointerGone,
        ]);
        count += added;
        host.count(count).await;
        assert_eq!(depth(&host), undo + 1);
        let created: Vec<_> = host
            .sketch()
            .entities()
            .iter()
            .filter(|entity| !before.contains_entity(entity.id()))
            .collect();
        assert!(
            created
                .iter()
                .any(|entity| matches!(entity, SketchEntity::Point { at, .. }
            if (at.x-10.0).abs()<1e-4 && (at.y-10.0).abs()<1e-4))
                || tool == SketchTool::Circle
        );
        if tool == SketchTool::Circle {
            assert!(created.iter().any(|entity| matches!(entity, SketchEntity::Circle { radius, .. } if (*radius-30.0_f64.hypot(20.0)).abs()<1e-4)));
        }
        let accepted = host.sketch().clone();
        host.ui(vec![button(hover, false), egui::Event::PointerMoved(from)]);
        assert_eq!(
            host.sketch(),
            &accepted,
            "post-release samples must not submit a second primitive"
        );
        host.history(false, count - added).await;
        assert_eq!(host.sketch(), &before);
        host.history(true, count).await;
        assert_eq!(host.sketch(), &accepted);
    }
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn pencil_primitive_cancellation_preserves_existing_tap_draft_and_authority() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Box::pin(Host::new(root.path())).await;
    for cancellation in 0..6 {
        pencil(&host);
        host.control("Rectangle");
        host.xy(-30.0, -20.0);
        let before = host.sketch().clone();
        let draft = format!("{:?}", host.app.doc.sketch.session.as_ref().unwrap().draft);
        let undo = depth(&host);
        let from = host.project(-20.0, -10.0);
        let end = host.project(10.0, 10.0);
        host.ui(vec![egui::Event::PointerMoved(from), button(from, true)]);
        host.ui(vec![egui::Event::PointerMoved(end)]);
        assert!(crate::sketch_creation::primitive_drag::preview(&host.context).is_some());
        assert_eq!(
            format!("{:?}", host.app.doc.sketch.session.as_ref().unwrap().draft),
            draft
        );
        match cancellation {
            0 => host.ui(vec![egui::Event::PointerGone]),
            1 => host.ui_focused(vec![], false),
            2 => {
                host.app.doc.camera.target.x += 1.0;
                host.redraw();
            }
            3 => crate::viewport_widgets::set_pointer_kind(
                &host.context,
                crate::viewport_widgets::PointerKind::Precise,
            ),
            4 => host.ui(vec![egui::Event::PointerMoved(
                host.rect("Circle").center(),
            )]),
            _ => host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]),
        }
        host.ui(vec![button(end, false)]);
        assert_eq!(host.sketch(), &before, "cancel case {cancellation}");
        assert_eq!(
            format!("{:?}", host.app.doc.sketch.session.as_ref().unwrap().draft),
            draft,
            "cancel case {cancellation}"
        );
        assert_eq!(depth(&host), undo);
    }
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn pencil_threshold_taps_and_desktop_tap_creation_keep_existing_semantics() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Box::pin(Host::new(root.path())).await;
    host.control("Rectangle");
    pencil(&host);
    let from = host.project(-20.0, -10.0);
    let nearby = from + egui::vec2(1.0, 1.0);
    host.ui(vec![
        egui::Event::PointerMoved(from),
        button(from, true),
        egui::Event::PointerMoved(nearby),
        button(nearby, false),
    ]);
    assert!(host.sketch().entities().is_empty());
    assert!(
        host.app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .draft
            .is_some()
    );
    host.xy(10.0, 10.0);
    host.count(8).await;
    crate::viewport_widgets::set_pointer_kind(
        &host.context,
        crate::viewport_widgets::PointerKind::Precise,
    );
    host.control("Circle");
    host.xy(25.0, 0.0);
    host.xy(30.0, 0.0);
    host.count(10).await;
    assert!(host.sketch().entities().iter().any(
        |entity| matches!(entity,SketchEntity::Circle {radius,..} if (*radius-5.0).abs()<1e-4)
    ));
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn pencil_same_frame_cancel_and_conflicting_presses_never_finish_a_pending_tap() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Box::pin(Host::new(root.path())).await;
    pencil(&host);
    for cancellation in 0..3 {
        host.control("Rectangle");
        host.xy(-30.0, -20.0);
        let before = host.sketch().clone();
        let draft = format!("{:?}", host.app.doc.sketch.session.as_ref().unwrap().draft);
        let undo = depth(&host);
        let from = host.project(-20.0, -10.0);
        let end = host.project(10.0, 10.0);
        let mut events = vec![
            egui::Event::PointerMoved(from),
            button(from, true),
            egui::Event::PointerMoved(end),
        ];
        events.push(match cancellation {
            0 => egui::Event::PointerGone,
            1 => button(end, true),
            _ => key(egui::Key::Escape, egui::Modifiers::NONE),
        });
        events.push(button(end, false));
        if cancellation == 1 {
            events.push(button(end, false));
        }
        host.ui(events);
        assert_eq!(
            host.sketch(),
            &before,
            "same-frame cancellation {cancellation}"
        );
        assert_eq!(
            format!("{:?}", host.app.doc.sketch.session.as_ref().unwrap().draft),
            draft
        );
        assert_eq!(depth(&host), undo);
    }
    host.stop().await;
}
