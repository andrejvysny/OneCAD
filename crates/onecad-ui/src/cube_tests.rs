use super::*;
use onecad_scene::{Camera, SceneIdentity, Viewport};

#[test]
fn accessibility_face_click_emits_one_camera_action_without_pointer_input() {
    let ctx = egui::Context::default();
    let faces = facets(view(DVec3::Z).basis(), pos2(200.0, 200.0), HALF);
    let direction = faces[0].direction;
    let mut id = egui::Id::NULL;
    let _ = frame(&ctx, Vec::new(), |ui| {
        id = ui.scope_id().with((
            "cube face",
            direction.x as i8,
            direction.y as i8,
            direction.z as i8,
        ));
        face_widgets(ui, &faces, None, &mut CubeResponse::default());
    });
    let request = egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Click,
        target_node: id.accesskit_id(),
        target_tree: egui::accesskit::TreeId::ROOT,
        data: None,
    };
    let mut output = CubeResponse::default();
    let _ = frame(
        &ctx,
        vec![egui::Event::AccessKitActionRequest(request)],
        |ui| face_widgets(ui, &faces, None, &mut output),
    );
    assert_eq!(output.actions, vec![CubeAction::SetDirection(direction)]);
}

fn view(direction: DVec3) -> ViewSnapshot {
    ViewSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 1,
        },
        1,
        Camera {
            offset: direction * 10.0,
            ..Camera::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    )
    .unwrap()
}

fn frame(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    mut paint: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            ..Default::default()
        },
        |ui| paint(ui),
    );
    output.textures_delta.clear();
    output
}

#[test]
fn cube_targets_are_exactly_six_faces_and_eight_corners() {
    let mut directions = Vec::new();
    for direction in [
        DVec3::X,
        -DVec3::X,
        DVec3::Y,
        -DVec3::Y,
        DVec3::Z,
        -DVec3::Z,
    ] {
        let faces = facets(view(direction).basis(), Pos2::ZERO, HALF);
        assert_eq!(faces.len(), 9);
        for face in faces {
            let count = [face.direction.x, face.direction.y, face.direction.z]
                .into_iter()
                .filter(|v| *v != 0.0)
                .count();
            assert!(count == 1 || count == 3);
            if !directions.contains(&face.direction) {
                directions.push(face.direction);
            }
        }
    }
    assert_eq!(directions.len(), 14);
}

#[test]
fn projected_facets_pick_centers_in_all_octants() {
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                let faces = facets(view(DVec3::new(x, y, z)).basis(), pos2(200.0, 200.0), HALF);
                for face in &faces {
                    let index = hit(&faces, face.centre).unwrap();
                    assert_eq!(faces[index].direction, face.direction);
                }
            }
        }
    }
    assert!(!contains(&[], Pos2::ZERO));
    assert!(!contains(&[Pos2::ZERO; 4], Pos2::ZERO));
}

#[test]
fn edge_strips_keep_face_target_and_silhouette() {
    let basis = view(-DVec3::Y).basis();
    let faces = facets(basis, pos2(100.0, 100.0), HALF);
    for direction in [DVec3::X, -DVec3::X, DVec3::Z, -DVec3::Z] {
        let p = pos2(100.0, 100.0) + project(basis, direction * 0.84, HALF);
        assert_eq!(faces[hit(&faces, p).unwrap()].direction, -DVec3::Y);
    }
    let bounds = Rect::from_points(
        &faces
            .iter()
            .flat_map(|f| f.points.iter().copied())
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        bounds,
        Rect::from_min_max(pos2(60.0, 60.0), pos2(140.0, 140.0))
    );
}

fn pointer(position: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

#[test]
fn raw_cube_drag_emits_small_motion_and_suppresses_release_click() {
    let ctx = egui::Context::default();
    let faces = facets(view(-DVec3::Y).basis(), pos2(200.0, 200.0), HALF);
    let start = pos2(200.0, 200.0);
    let mut result = (Vec::new(), false);
    let id = egui::Id::unique("test gesture");
    let _ = frame(
        &ctx,
        vec![egui::Event::PointerMoved(start), pointer(start, true)],
        |ui| {
            result = pointer_gesture(ui, id, &faces, true);
        },
    );
    assert!(result.1 && result.0.is_empty());
    let moved = start + vec2(1.0, 0.0);
    let _ = frame(&ctx, vec![egui::Event::PointerMoved(moved)], |ui| {
        result = pointer_gesture(ui, id, &faces, true);
    });
    assert_eq!(result.0, vec![CubeAction::Orbit(vec2(1.0, 0.0))]);
    let _ = frame(&ctx, vec![pointer(moved, false)], |ui| {
        result = pointer_gesture(ui, id, &faces, true);
    });
    assert!(result.1 && result.0.is_empty());
}

#[test]
fn stationary_cube_click_emits_view_and_disabled_gesture_cancels() {
    let ctx = egui::Context::default();
    let faces = facets(view(-DVec3::Y).basis(), pos2(200.0, 200.0), HALF);
    let position = pos2(200.0, 200.0);
    let id = egui::Id::unique("test gesture");
    let mut result = (Vec::new(), false);
    let _ = frame(
        &ctx,
        vec![egui::Event::PointerMoved(position), pointer(position, true)],
        |ui| {
            result = pointer_gesture(ui, id, &faces, true);
        },
    );
    let _ = frame(&ctx, vec![pointer(position, false)], |ui| {
        result = pointer_gesture(ui, id, &faces, true);
    });
    assert_eq!(result.0, vec![CubeAction::SetDirection(-DVec3::Y)]);
    let _ = frame(&ctx, vec![pointer(position, true)], |ui| {
        result = pointer_gesture(ui, id, &faces, true);
    });
    let _ = frame(&ctx, vec![], |ui| {
        result = pointer_gesture(ui, id, &faces, false);
    });
    assert!(result.0.is_empty());
    assert!(ctx.data(|d| d.get_temp::<Gesture>(id)).is_none());
}

#[test]
fn label_mesh_uvs_are_normalized_after_later_font_growth() {
    let ctx = egui::Context::default();
    let faces = facets(view(-DVec3::Y).basis(), pos2(200.0, 200.0), HALF);
    let palette = CubePalette::new(&crate::theme::load(true).unwrap(), true);
    let output = frame(&ctx, vec![], |ui| {
        painter::paint_cube(ui.painter(), &faces, None, palette, true, 1.0);
        // Populate the atlas after the cube has produced its pixel UVs.
        for size in [20.0, 32.0, 48.0, 64.0] {
            let _ = ui.painter().layout_no_wrap(
                "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".into(),
                FontId::proportional(size),
                palette.cube_text,
            );
        }
    });
    assert!(output.shapes.iter().any(
        |s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id == egui::TextureId::default())
    ));
    for shape in output.shapes {
        if let egui::Shape::Mesh(mesh) = shape.shape {
            assert_ne!(mesh.texture_id, painter::LABEL_PIXELS);
            for vertex in &mesh.vertices {
                assert!(vertex.uv.x <= 1.0 && vertex.uv.y <= 1.0);
            }
        }
    }
}

#[test]
fn quarter_roll_control_emits_camera_action_and_owns_click() {
    let ctx = egui::Context::default();
    let captured = view(-DVec3::Y);
    let before = captured.identity();
    let viewport = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let position = rect(viewport).center() + vec2(72.0, -42.0) * (SIZE / 164.0);
    let theme = crate::theme::load(true).unwrap();
    let mut result = CubeResponse::default();
    let _ = frame(&ctx, vec![egui::Event::PointerMoved(position)], |ui| {
        result = show(ui, viewport, &captured, &theme);
    });
    assert!(result.owns_pointer);
    assert!(result.actions.is_empty());
    let _ = frame(&ctx, vec![pointer(position, true)], |ui| {
        result = show(ui, viewport, &captured, &theme);
    });
    let _ = frame(&ctx, vec![pointer(position, false)], |ui| {
        result = show(ui, viewport, &captured, &theme);
    });
    assert!(result.owns_pointer);
    assert_eq!(
        result.actions,
        vec![CubeAction::Roll(std::f64::consts::FRAC_PI_2)]
    );
    assert_eq!(captured.identity(), before);
}

#[test]
fn face_hover_outline_has_no_coplanar_pick_seams() {
    let ctx = egui::Context::default();
    let faces = facets(view(-DVec3::Y).basis(), pos2(200.0, 200.0), HALF);
    let hovered = hit(&faces, pos2(200.0, 200.0)).unwrap();
    for dark in [false, true] {
        let palette = CubePalette::new(&crate::theme::load(dark).unwrap(), true);
        let output = frame(&ctx, vec![], |ui| {
            painter::paint_cube(ui.painter(), &faces, Some(hovered), palette, false, 1.0);
        });
        let interior =
            Rect::from_center_size(pos2(200.0, 200.0), vec2(2.0, 2.0) * (INSET * HALF) as f32);
        let mut outlines = 0;
        for shape in output.shapes {
            if let egui::Shape::LineSegment {
                points: [a, b],
                stroke,
            } = shape.shape
                && stroke.color == palette.accent
            {
                outlines += 1;
                assert!(!interior.contains(a.lerp(b, 0.5)));
            }
        }
        assert_eq!(outlines, 12);
    }
}
