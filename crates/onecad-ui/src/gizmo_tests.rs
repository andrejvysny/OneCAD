use super::*;
use geometry::{Geometry, State, along, on_plane, project};
use onecad_scene::{Camera, DVec3, Projection, SceneIdentity, Viewport};

fn view(projection: Projection, scale: f64) -> ViewSnapshot {
    ViewSnapshot::new(
        SceneIdentity {
            session: 5,
            scene: 9,
        },
        2,
        Camera {
            offset: DVec3::new(7.0, -11.0, 9.0),
            projection,
            ..Default::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: scale,
        },
    )
    .unwrap()
}
fn rect() -> Rect {
    Rect::from_min_size(egui::pos2(30.0, 40.0), egui::vec2(800.0, 600.0))
}
fn triad() -> Triad {
    Triad {
        center: DVec3::ZERO,
        translate: true,
        rotate: true,
    }
}
fn screen(view: &ViewSnapshot, point: DVec3) -> Pos2 {
    project(view, rect(), point).unwrap()
}
fn translation(drag: Drag) -> DVec3 {
    match drag.change {
        Change::Translate(value) => value,
        _ => panic!("expected translation"),
    }
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn for_views(mut test: impl FnMut(ViewSnapshot)) {
    for projection in [Projection::Perspective, Projection::Orthographic] {
        for scale in [1.0, 2.0] {
            test(view(projection, scale));
        }
    }
}
#[test]
fn axis_and_plane_constraints_match_world_delta() {
    for_views(|v| {
        for i in 0..3 {
            let dir = geometry::DIRS[i];
            let start = screen(&v, dir);
            let mut state = State::begin(&v, rect(), triad(), Handle::Axis(i), start).unwrap();
            let drag = state
                .drag(screen(&v, dir * 2.0), 1.0, true, false, false)
                .unwrap();
            assert!((translation(drag) - dir).length() < 1e-5);
            let (a, b) = geometry::ring_basis(i).unwrap();
            let point = a * 0.3 + b * 0.4;
            let mut state =
                State::begin(&v, rect(), triad(), Handle::Plane(i), screen(&v, point)).unwrap();
            let drag = state
                .drag(
                    screen(&v, point + a * 0.5 + b * 0.8),
                    0.1,
                    true,
                    false,
                    false,
                )
                .unwrap();
            assert!((translation(drag) - (a * 0.5 + b * 0.8)).length() < 1e-5);
        }
    });
}
#[test]
fn ball_drag_uses_frozen_view_plane() {
    for_views(|v| {
        let start = screen(&v, DVec3::ZERO);
        let delta = v.basis().right * 1.3 + v.basis().up * 0.7;
        let mut state = State::begin(&v, rect(), triad(), Handle::Ball, start).unwrap();
        let drag = state
            .drag(screen(&v, delta), 0.0, true, false, false)
            .unwrap();
        assert!((translation(drag) - delta).length() < 1e-5);
    });
}
#[test]
fn rings_unwrap_across_pi_and_snap_to_five_degrees() {
    for_views(|v| {
        for axis in 0..3 {
            let (a, b) = geometry::ring_basis(axis).unwrap();
            let point = |degrees: f64| {
                screen(
                    &v,
                    a * degrees.to_radians().cos() + b * degrees.to_radians().sin(),
                )
            };
            let mut state =
                State::begin(&v, rect(), triad(), Handle::Ring(axis), point(170.0)).unwrap();
            for (degrees, expected) in [(179.0, 9.0), (-179.0, 11.0), (-160.0, 30.0)] {
                let drag = state.drag(point(degrees), 1.0, true, false, false).unwrap();
                let Change::Rotate { angle, .. } = drag.change else {
                    panic!("rotation");
                };
                // Pointer coordinates are f32 logical pixels, including the projected input.
                assert!((angle.to_degrees() - expected).abs() < 0.001);
            }
            let drag = state.drag(point(-158.0), 1.0, false, false, false).unwrap();
            let Change::Rotate { angle, .. } = drag.change else {
                panic!("rotation");
            };
            close(angle.to_degrees(), 30.0);
        }
    });
}
#[test]
fn distance_snap_and_free_mode_preserve_precision() {
    let v = view(Projection::Orthographic, 1.0);
    let start = screen(&v, DVec3::X);
    let mut state = State::begin(&v, rect(), triad(), Handle::Axis(0), start).unwrap();
    close(
        translation(
            state
                .drag(screen(&v, DVec3::X * 1.24), 0.5, false, false, false)
                .unwrap(),
        )
        .x,
        0.0,
    );
    close(
        translation(
            state
                .drag(screen(&v, DVec3::X * 1.24), 0.5, true, false, false)
                .unwrap(),
        )
        .x,
        0.24,
    );
}
#[test]
fn geometry_has_full_handle_set_and_constant_logical_size() {
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let a = Geometry::new(&view(projection, 1.0), rect(), triad()).unwrap();
        let b = Geometry::new(&view(projection, 2.0), rect(), triad()).unwrap();
        assert_eq!((a.arrows.len(), a.planes.len(), a.rings.len()), (3, 3, 3));
        for (first, second) in a.arrows.iter().zip(b.arrows.iter()) {
            assert_eq!(first, second);
        }
        assert_eq!(a.hit(a.ball), Some(Handle::Ball));
        assert!(!Gizmo::contains_pointer(
            &view(projection, 1.0),
            rect(),
            triad(),
            egui::pos2(-1.0, -1.0)
        ));
    }
}
#[test]
fn parallel_axis_and_edge_on_plane_refuse_drag() {
    let original = view(Projection::Orthographic, 1.0);
    let c = Camera {
        offset: -DVec3::Y * 10.0,
        ..original.camera()
    };
    let v = ViewSnapshot::new(original.identity().scene, 2, c, original.viewport()).unwrap();
    let center = screen(&v, DVec3::ZERO);
    assert!(along(&v, rect(), center, DVec3::ZERO, DVec3::Y).is_none());
    assert!(on_plane(&v, rect(), center, DVec3::ZERO, DVec3::X).is_none());
    assert!(State::begin(&v, rect(), triad(), Handle::Axis(1), center).is_none());
    assert!(State::begin(&v, rect(), triad(), Handle::Ring(99), center).is_none());
}
fn press(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
#[test]
fn raw_press_move_release_has_no_drag_threshold() {
    let v = view(Projection::Orthographic, 1.0);
    let geometry = Geometry::new(&v, rect(), triad()).unwrap();
    let start = geometry.ball;
    let end = start + egui::vec2(1.0, 0.0);
    let mut gizmo = Gizmo::default();
    let response = gizmo.process(
        &[press(start, true), egui::Event::PointerMoved(end)],
        &Frame {
            view: &v,
            rect: rect(),
            triad: triad(),
            step: 0.0,
            free: true,
            geometry: &geometry,
        },
    );
    let drag = response.drag.unwrap();
    assert!(drag.started && !drag.done && response.owns_pointer && gizmo.active());
    assert!(translation(drag).length() > 0.0);
    let response = gizmo.process(
        &[press(end, false)],
        &Frame {
            view: &v,
            rect: rect(),
            triad: triad(),
            step: 0.0,
            free: true,
            geometry: &geometry,
        },
    );
    assert!(response.drag.unwrap().done && response.owns_pointer && !gizmo.active());
}
#[test]
fn same_frame_release_preserves_started_and_cancel_emits_no_done() {
    let v = view(Projection::Orthographic, 1.0);
    let geometry = Geometry::new(&v, rect(), triad()).unwrap();
    let start = geometry.ball;
    let mut gizmo = Gizmo::default();
    let response = gizmo.process(
        &[press(start, true), press(start, false)],
        &Frame {
            view: &v,
            rect: rect(),
            triad: triad(),
            step: 0.0,
            free: true,
            geometry: &geometry,
        },
    );
    let drag = response.drag.unwrap();
    assert!(drag.started && drag.done && !gizmo.active());
    gizmo.process(
        &[press(start, true)],
        &Frame {
            view: &v,
            rect: rect(),
            triad: triad(),
            step: 0.0,
            free: true,
            geometry: &geometry,
        },
    );
    gizmo.cancel();
    assert!(!gizmo.active());
    let response = gizmo.process(
        &[press(start, false)],
        &Frame {
            view: &v,
            rect: rect(),
            triad: triad(),
            step: 0.0,
            free: true,
            geometry: &geometry,
        },
    );
    assert!(response.drag.is_none());
}
#[test]
fn capture_changes_and_malformed_data_refuse() {
    let v = view(Projection::Perspective, 1.0);
    let mut state =
        State::begin(&v, rect(), triad(), Handle::Ball, screen(&v, DVec3::ZERO)).unwrap();
    let moved = ViewSnapshot::new(
        v.identity().scene,
        2,
        Camera {
            target: DVec3::X,
            ..v.camera()
        },
        v.viewport(),
    )
    .unwrap();
    assert!(!state.matches(&moved, rect(), triad()));
    assert!(!state.matches(
        &v,
        rect(),
        Triad {
            center: DVec3::X,
            ..triad()
        }
    ));
    assert!(
        state
            .drag(screen(&v, DVec3::X), f64::NAN, false, false, false)
            .is_none()
    );
}

#[test]
fn malformed_or_clipped_geometry_refuses() {
    let v = view(Projection::Perspective, 1.0);
    assert!(
        Geometry::new(
            &v,
            rect(),
            Triad {
                center: DVec3::splat(f64::NAN),
                ..triad()
            }
        )
        .is_none()
    );
    assert!(Geometry::new(&v, Rect::NOTHING, triad()).is_none());
    assert!(
        Geometry::new(
            &v,
            rect(),
            Triad {
                center: v.eye() + v.basis().back,
                ..triad()
            }
        )
        .is_none()
    );
}

#[test]
fn capture_nonce_changes_preserve_drag_but_real_capture_changes_cancel() {
    let v = view(Projection::Perspective, 1.0);
    let state = State::begin(&v, rect(), triad(), Handle::Ball, screen(&v, DVec3::ZERO)).unwrap();
    let next = ViewSnapshot::new(v.identity().scene, 99, v.camera(), v.viewport()).unwrap();
    assert!(state.matches(&next, rect(), triad()));
    let changed_scene = ViewSnapshot::new(
        SceneIdentity {
            scene: 10,
            ..v.identity().scene
        },
        99,
        v.camera(),
        v.viewport(),
    )
    .unwrap();
    let changed_viewport = ViewSnapshot::new(
        v.identity().scene,
        99,
        v.camera(),
        Viewport {
            scale_factor: 2.0,
            ..v.viewport()
        },
    )
    .unwrap();
    assert!(!state.matches(&changed_scene, rect(), triad()));
    assert!(!state.matches(&changed_viewport, rect(), triad()));
    let mut gizmo = Gizmo { state: Some(state) };
    assert!(gizmo.geometry(&next, rect(), triad(), 0.0).is_some());
    assert!(gizmo.active());
    assert!(
        gizmo
            .geometry(&changed_scene, rect(), triad(), 0.0)
            .is_none()
    );
    assert!(!gizmo.active());
}

#[test]
fn disabled_or_popup_ui_cancels_active_drag_and_refuses_new_press() {
    for popup in [false, true] {
        for initially_active in [false, true] {
            let ctx = egui::Context::default();
            let v = view(Projection::Orthographic, 1.0);
            let pos = screen(&v, DVec3::ZERO);
            let mut gizmo = Gizmo::default();
            if initially_active {
                gizmo.state = State::begin(&v, rect(), triad(), Handle::Ball, pos);
            }
            let theme = crate::theme::load(false).unwrap();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect()),
                    focused: true,
                    events: vec![egui::Event::PointerMoved(pos), press(pos, true)],
                    ..Default::default()
                },
                |ui| {
                    if popup {
                        egui::Popup::open_id(ui.ctx(), egui::Id::unique("gizmo_modal_test"));
                    }
                    egui::CentralPanel::default().show(ui, |ui| {
                        if !popup {
                            ui.disable();
                        }
                        let response = gizmo.show(ui, &v, rect(), triad(), 0.0, &theme);
                        assert!(response.drag.is_none());
                        assert!(!response.owns_pointer);
                        assert!(!gizmo.active());
                    });
                },
            );
            output.textures_delta.clear();
        }
    }
}

fn chrome_frame(
    ctx: &egui::Context,
    gizmo: &mut Gizmo,
    events: Vec<egui::Event>,
    focused: bool,
) -> GizmoResponse {
    let v = view(Projection::Orthographic, 1.0);
    let center = screen(&v, DVec3::ZERO);
    let theme = crate::theme::load(false).unwrap();
    let mut response = GizmoResponse::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect()),
            focused,
            events,
            ..Default::default()
        },
        |ui| {
            egui::Area::new(egui::Id::unique("gizmo covering panel"))
                .fixed_pos(center - egui::vec2(40.0, 40.0))
                .show(ui.ctx(), |ui| {
                    ui.allocate_exact_size(egui::vec2(200.0, 120.0), egui::Sense::hover());
                });
            egui::CentralPanel::default().show(ui, |ui| {
                response = gizmo.show(ui, &v, rect(), triad(), 0.0, &theme);
            });
        },
    );
    output.textures_delta.clear();
    response
}

#[test]
fn covering_chrome_refuses_gizmo_press_but_keeps_captured_release() {
    let ctx = egui::Context::default();
    let v = view(Projection::Orthographic, 1.0);
    let start = screen(&v, DVec3::ZERO);
    let mut gizmo = Gizmo::default();
    chrome_frame(&ctx, &mut gizmo, Vec::new(), true);
    let response = chrome_frame(
        &ctx,
        &mut gizmo,
        vec![egui::Event::PointerMoved(start), press(start, true)],
        true,
    );
    assert!(response.drag.is_none() && !response.owns_pointer && !gizmo.active());
    gizmo.state = State::begin(&v, rect(), triad(), Handle::Ball, start);
    let end = start + egui::vec2(20.0, 0.0);
    let response = chrome_frame(
        &ctx,
        &mut gizmo,
        vec![egui::Event::PointerMoved(end), press(end, false)],
        true,
    );
    assert!(response.drag.unwrap().done && response.owns_pointer && !gizmo.active());
}

#[test]
fn escape_and_focus_loss_end_gizmo_without_dispatching_further_delta() {
    for focused in [false, true] {
        let ctx = egui::Context::default();
        let v = view(Projection::Orthographic, 1.0);
        let start = screen(&v, DVec3::ZERO);
        let mut gizmo = Gizmo {
            state: State::begin(&v, rect(), triad(), Handle::Ball, start),
        };
        let response = chrome_frame(
            &ctx,
            &mut gizmo,
            vec![
                egui::Event::PointerMoved(start + egui::vec2(20.0, 0.0)),
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ],
            focused,
        );
        assert!(response.drag.is_none() && !gizmo.active());
        assert!(
            response.cancelled,
            "the caller restores the authored placement"
        );
    }
}

#[test]
fn pointer_loss_reports_cancel_and_an_earlier_release_survives_a_new_press() {
    let v = view(Projection::Orthographic, 1.0);
    let geometry = Geometry::new(&v, rect(), triad()).unwrap();
    let frame = Frame {
        view: &v,
        rect: rect(),
        triad: triad(),
        step: 0.0,
        free: true,
        geometry: &geometry,
    };
    let start = geometry.ball;
    let end = start + egui::vec2(12.0, 0.0);
    let mut gizmo = Gizmo::default();
    let response = gizmo.process(
        &[
            press(start, true),
            egui::Event::PointerMoved(end),
            egui::Event::PointerGone,
        ],
        &frame,
    );
    assert!(response.cancelled && response.drag.is_none() && !gizmo.active());
    let idle = gizmo.process(&[egui::Event::PointerGone], &frame);
    assert!(!idle.cancelled, "no capture, nothing to restore");
    let response = gizmo.process(
        &[
            press(start, true),
            egui::Event::PointerMoved(end),
            press(end, false),
            press(start, true),
        ],
        &frame,
    );
    let finished = response.finished.unwrap();
    assert!(finished.done && translation(finished).length() > 0.0);
    assert!(response.drag.unwrap().started && gizmo.active());
    gizmo.cancel();
    // A loss after the release in the same batch keeps the release, at its position.
    let released = end + egui::vec2(30.0, 0.0);
    let response = gizmo.process(
        &[
            press(start, true),
            egui::Event::PointerMoved(end),
            press(released, false),
            egui::Event::PointerGone,
        ],
        &frame,
    );
    let drag = response.drag.unwrap();
    assert!(drag.done && !response.cancelled && !gizmo.active());
    let moved_only = Gizmo::default()
        .process(
            &[press(start, true), egui::Event::PointerMoved(end)],
            &frame,
        )
        .drag
        .unwrap();
    assert!(translation(drag).length() > translation(moved_only).length());
}
