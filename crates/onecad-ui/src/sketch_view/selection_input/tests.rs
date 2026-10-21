use super::*;
use crate::sketch_view::selection::tests::{add_point, at, session};
use onecad_core::sketch::CurvePosition;

fn frame(
    ctx: &egui::Context,
    session: &SketchSession,
    rect: Rect,
    view: &ViewSnapshot,
    mut events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> Option<SketchAction> {
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    let raw = egui::RawInput {
        screen_rect: Some(rect.expand(100.0)),
        events,
        ..Default::default()
    };
    let mut action = None;
    let mut output = ctx.run_ui(raw, |ui| {
        let response = ui.interact(
            rect,
            egui::Id::unique("sketch-test-canvas"),
            egui::Sense::click_and_drag(),
        );
        action = selection_input(ui, rect, &response, view, session).unwrap();
    });
    output.textures_delta.clear();
    action
}

fn button(position: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    }
}

#[test]
fn pointer_replay_selects_point_roles_and_shift_or_meta_toggles() {
    let (mut session, rect, view) = session();
    let id = add_point(&mut session, 0.0, 0.0);
    let ctx = egui::Context::default();
    frame(&ctx, &session, rect, &view, vec![], egui::Modifiers::NONE);
    let position = at(&session, rect, &view, 0.0, 0.0);
    for modifiers in [
        egui::Modifiers::NONE,
        egui::Modifiers::SHIFT,
        egui::Modifiers::MAC_CMD,
    ] {
        frame(
            &ctx,
            &session,
            rect,
            &view,
            vec![
                egui::Event::PointerMoved(position),
                button(position, true, modifiers),
            ],
            modifiers,
        );
        let action = frame(
            &ctx,
            &session,
            rect,
            &view,
            vec![button(position, false, modifiers)],
            modifiers,
        )
        .unwrap();
        let SketchAction::Select {
            targets,
            mode,
            document,
            capture,
            ..
        } = action
        else {
            panic!("selection action")
        };
        assert_eq!(
            targets,
            vec![SketchTarget::Point {
                entity: id,
                role: CurvePosition::Arbitrary
            }]
        );
        assert_eq!(
            mode,
            if modifiers == egui::Modifiers::NONE {
                SelectionMode::Replace
            } else {
                SelectionMode::Toggle
            }
        );
        assert_eq!(document, session.document);
        assert_eq!(capture, view.identity());
    }
}

#[test]
fn pointer_drag_uses_original_capture_and_escape_clears_cache() {
    let (mut session, rect, view) = session();
    add_point(&mut session, 0.0, 0.0);
    let ctx = egui::Context::default();
    frame(&ctx, &session, rect, &view, vec![], egui::Modifiers::NONE);
    let start = rect.center() - egui::vec2(30.0, 30.0);
    let end = rect.center() + egui::vec2(30.0, 30.0);
    frame(
        &ctx,
        &session,
        rect,
        &view,
        vec![
            egui::Event::PointerMoved(start),
            button(start, true, egui::Modifiers::NONE),
        ],
        egui::Modifiers::NONE,
    );
    let action = frame(
        &ctx,
        &session,
        rect,
        &view,
        vec![egui::Event::PointerMoved(end)],
        egui::Modifiers::NONE,
    )
    .unwrap();
    assert!(matches!(
        action,
        SketchAction::SelectionDrag { active: true, .. }
    ));
    session.selection_dragging = true;
    let action = frame(
        &ctx,
        &session,
        rect,
        &view,
        vec![
            button(end, false, egui::Modifiers::NONE),
            egui::Event::PointerMoved(start),
        ],
        egui::Modifiers::NONE,
    )
    .unwrap();
    assert!(
        matches!(action, SketchAction::Select { targets, mode: SelectionMode::Replace, .. } if targets.len() == 1)
    );
    assert!(
        ctx.data(|data| data.get_temp::<Marquee>(cache_id()))
            .is_none()
    );
    session.selection_dragging = false;
    ctx.data_mut(|data| {
        data.insert_temp(
            cache_id(),
            Marquee {
                key: session.key.clone(),
                document: session.document.clone(),
                capture: view.identity(),
                start,
                mode: SelectionMode::Replace,
            },
        )
    });
    assert!(frame(&ctx, &session, rect, &view, vec![], egui::Modifiers::NONE).is_none());
    assert!(
        ctx.data(|data| data.get_temp::<Marquee>(cache_id()))
            .is_none()
    );
}

#[test]
fn click_uses_press_modifiers_and_position_despite_batched_later_input() {
    let (mut session, rect, view) = session();
    let id = add_point(&mut session, 0.0, 0.0);
    let ctx = egui::Context::default();
    frame(&ctx, &session, rect, &view, vec![], egui::Modifiers::NONE);
    let position = at(&session, rect, &view, 0.0, 0.0);
    let action = frame(
        &ctx,
        &session,
        rect,
        &view,
        vec![
            egui::Event::PointerMoved(position),
            button(position, true, egui::Modifiers::SHIFT),
            button(position, false, egui::Modifiers::NONE),
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::PointerMoved(position + egui::vec2(100.0, 0.0)),
        ],
        egui::Modifiers::NONE,
    )
    .unwrap();
    assert!(
        matches!(action, SketchAction::Select { mode: SelectionMode::Toggle, targets, .. }
        if targets == vec![SketchTarget::Point { entity: id, role: CurvePosition::Arbitrary }])
    );
}

#[test]
fn marquee_keeps_press_mode_after_shift_release_before_drag() {
    let (mut session, rect, view) = session();
    add_point(&mut session, 0.0, 0.0);
    let ctx = egui::Context::default();
    frame(&ctx, &session, rect, &view, vec![], egui::Modifiers::NONE);
    let start = rect.center() - egui::vec2(30.0, 30.0);
    let end = rect.center() + egui::vec2(30.0, 30.0);
    frame(
        &ctx,
        &session,
        rect,
        &view,
        vec![
            egui::Event::PointerMoved(start),
            button(start, true, egui::Modifiers::SHIFT),
        ],
        egui::Modifiers::SHIFT,
    );
    assert!(matches!(
        frame(
            &ctx,
            &session,
            rect,
            &view,
            vec![egui::Event::PointerMoved(end)],
            egui::Modifiers::NONE
        ),
        Some(SketchAction::SelectionDrag { active: true, .. })
    ));
    session.selection_dragging = true;
    assert!(matches!(
        frame(
            &ctx,
            &session,
            rect,
            &view,
            vec![button(end, false, egui::Modifiers::NONE)],
            egui::Modifiers::NONE
        ),
        Some(SketchAction::Select {
            mode: SelectionMode::Toggle,
            ..
        })
    ));
}

#[test]
fn multiple_primary_cycles_refuse_instead_of_selecting_last_input() {
    let (session, rect, view) = session();
    let ctx = egui::Context::default();
    let position = rect.center();
    let mut result = Ok(None);
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![
                button(position, true, egui::Modifiers::NONE),
                button(position, false, egui::Modifiers::NONE),
                button(position, true, egui::Modifiers::SHIFT),
                button(position, false, egui::Modifiers::SHIFT),
            ],
            ..Default::default()
        },
        |ui| {
            result = primary_press(ui, rect, &view, &session);
        },
    );
    output.textures_delta.clear();
    assert!(
        matches!(result, Err(message) if message == "Multiple pointer cycles in one frame; select again")
    );
    assert!(
        ctx.data(|data| data.get_temp::<Marquee>(press_id()))
            .is_none()
    );
}

#[test]
fn shift_up_after_same_batch_click_does_not_replace_existing_selection() {
    let (mut session, rect, view) = session();
    let first = add_point(&mut session, -20.0, 0.0);
    let second = add_point(&mut session, 20.0, 0.0);
    session.selection = vec![SketchTarget::Point {
        entity: first,
        role: CurvePosition::Arbitrary,
    }];
    let ctx = egui::Context::default();
    frame(&ctx, &session, rect, &view, vec![], egui::Modifiers::NONE);
    let position = at(&session, rect, &view, 20.0, 0.0);
    let action = frame(
        &ctx,
        &session,
        rect,
        &view,
        vec![
            egui::Event::PointerMoved(position),
            button(position, true, egui::Modifiers::SHIFT),
            button(position, false, egui::Modifiers::SHIFT),
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        ],
        egui::Modifiers::SHIFT,
    )
    .unwrap();
    let expected = SketchTarget::Point {
        entity: second,
        role: CurvePosition::Arbitrary,
    };
    assert!(
        matches!(action, SketchAction::Select { mode: SelectionMode::Toggle, targets, .. } if targets == vec![expected])
    );
    assert_eq!(
        session.selection,
        vec![SketchTarget::Point {
            entity: first,
            role: CurvePosition::Arbitrary
        }]
    );
}
