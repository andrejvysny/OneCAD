use super::*;

fn accepted(nav: &mut Navigation, source: MouseWheelSource, phase: TouchPhase, time: f64) {
    nav.accepted_event(&wheel(source, phase, vec2(1.0, 2.0), false), time, true);
}

#[test]
fn accepted_brackets_end_and_cancel_without_waiting_for_idle() {
    for ending in [TouchPhase::End, TouchPhase::Cancel] {
        let mut nav = Navigation::default();
        accepted(&mut nav, MouseWheelSource::Trackpad, TouchPhase::Start, 1.0);
        assert!(nav.active(1.5), "held bracket remains active between moves");
        assert!(
            nav.repaint_after(1.5).is_none(),
            "held gestures need no idle timer"
        );
        nav.accepted_event(
            &wheel(MouseWheelSource::Trackpad, ending, Vec2::ZERO, false),
            1.5,
            false,
        );
        assert!(!nav.active(1.5), "release over chrome still ends activity");
    }
}

#[test]
fn rejected_chrome_events_never_start_navigation_and_momentum_is_a_new_bracket() {
    let mut nav = Navigation::default();
    for phase in [TouchPhase::Start, TouchPhase::Move, TouchPhase::End] {
        nav.accepted_event(
            &wheel(MouseWheelSource::Trackpad, phase, vec2(1.0, 1.0), false),
            1.0,
            false,
        );
        assert!(!nav.active(1.0));
    }
    accepted(&mut nav, MouseWheelSource::Trackpad, TouchPhase::Start, 2.0);
    accepted(&mut nav, MouseWheelSource::Trackpad, TouchPhase::End, 2.02);
    assert!(!nav.active(2.02));
    accepted(
        &mut nav,
        MouseWheelSource::Momentum,
        TouchPhase::Start,
        2.03,
    );
    assert!(nav.active(2.5));
    accepted(&mut nav, MouseWheelSource::Momentum, TouchPhase::End, 2.6);
    assert!(!nav.active(2.6));
}

#[test]
fn unbracketed_wheel_zoom_and_phase_less_trackpad_expire_after_the_last_motion() {
    for event in [
        wheel(
            MouseWheelSource::Wheel,
            TouchPhase::Move,
            vec2(0.0, 1.0),
            false,
        ),
        wheel(
            MouseWheelSource::Trackpad,
            TouchPhase::Move,
            vec2(0.0, 1.0),
            false,
        ),
        Event::Zoom(1.1),
    ] {
        let mut nav = Navigation::default();
        nav.accepted_event(&event, 1.0, true);
        assert!(nav.active(1.119));
        nav.accepted_event(&event, 1.1, true);
        assert!(
            nav.active(1.219),
            "momentum extends only the bounded idle window"
        );
        assert!(!nav.active(1.221));
        assert!(nav.repaint_after(1.1).is_some());
        assert!(
            nav.repaint_after(1.221).is_none(),
            "idle expiry must stop repainting"
        );
    }
}

#[test]
fn drag_focus_pointer_and_context_transitions_never_leave_navigation_active() {
    let mut nav = Navigation::default();
    nav.dragging(true);
    assert!(nav.active(10.0));
    nav.dragging(false);
    assert!(!nav.active(10.0));
    for event in [Event::WindowFocused(false), Event::PointerGone] {
        nav.dragging(true);
        nav.accepted_event(&event, 10.0, false);
        assert!(!nav.active(10.0));
    }
    let ctx = egui::Context::default();
    let scene = Some(onecad_scene::SceneIdentity {
        session: 1,
        scene: 2,
    });
    nav.enter_context(&ctx, 1, scene);
    nav.dragging(true);
    nav.enter_context(&ctx, 1, scene);
    assert!(nav.active(0.0));
    nav.enter_context(&ctx, 2, scene);
    assert!(!nav.active(0.0));
    nav.dragging(true);
    nav.enter_context(&ctx, 1, scene);
    assert!(!nav.active(0.0));
    nav.dragging(true);
    nav.enter_context(&egui::Context::default(), 1, scene);
    assert!(!nav.active(0.0));
    nav.dragging(true);
    nav.enter_context(&ctx, 1, None);
    assert!(!nav.active(0.0));
}

#[test]
fn returning_from_a_hidden_viewport_does_not_resurrect_a_held_gesture() {
    let ctx = egui::Context::default();
    let mut nav = Navigation::default();
    let scene = Some(onecad_scene::SceneIdentity {
        session: 1,
        scene: 1,
    });
    let mut output = ctx.run_ui(Default::default(), |ui| {
        nav.enter_context(ui.ctx(), 1, scene)
    });
    output.textures_delta.clear();
    nav.dragging(true);
    for _ in 0..2 {
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
    }
    let mut output = ctx.run_ui(Default::default(), |ui| {
        nav.enter_context(ui.ctx(), 1, scene)
    });
    output.textures_delta.clear();
    assert!(!nav.active(ctx.input(|input| input.time)));
}

#[test]
fn camera_generation_tracks_actual_changes_and_survives_pointer_loss() {
    let mut nav = Navigation::default();
    let mut camera = onecad_scene::Camera::default();
    assert!(nav.observe_camera(camera));
    assert_eq!(nav.generation(), 1);
    assert!(!nav.observe_camera(camera));
    send(&mut nav, &Event::PointerGone, 1.0);
    assert_eq!(nav.generation(), 1);
    camera.target.x += 1.0;
    assert!(nav.observe_camera(camera));
    assert_eq!(nav.generation(), 2);
}
