use super::*;

#[test]
fn highlights_preserve_point_roles_and_do_not_turn_distance_lines_into_points() {
    let f = fixture();
    let endpoint = SketchTarget::Point {
        entity: f.arc.owner_id(),
        role: CurvePosition::End,
    };
    let constraint =
        operands::build(Relation::Distance, &f.sketch, &[endpoint, f.parallel], None).unwrap();
    assert_eq!(
        constraint_targets(&f.sketch, &constraint),
        vec![endpoint, f.parallel]
    );
    let constraint = operands::build(
        Relation::Symmetric,
        &f.sketch,
        &[f.point, f.point2, f.line],
        None,
    )
    .unwrap();
    assert_eq!(
        constraint_targets(&f.sketch, &constraint),
        vec![f.point, f.point2, f.line]
    );
}

#[test]
fn consecutive_authored_relations_allocate_distinct_nonnil_constraint_ids() {
    let f = fixture();
    let first = operands::create(Relation::Horizontal, &f.sketch, &[f.line], None).unwrap();
    let second = operands::create(Relation::Vertical, &f.sketch, &[f.rotated], None).unwrap();
    assert_ne!(first.id(), second.id());
    assert_ne!(first.id().0, uuid::Uuid::nil());
    assert_ne!(second.id().0, uuid::Uuid::nil());
}

#[test]
fn line_distance_uses_perpendicular_measure_for_rotated_parallel_lines() {
    let mut f = fixture();
    let a = point(&mut f.sketch, 0.0, 0.0);
    let b = point(&mut f.sketch, 10.0, 10.0);
    let c = point(&mut f.sketch, 0.0, 5.0);
    let d = point(&mut f.sketch, 10.0, 15.0);
    let first = add_line(&mut f.sketch, a, b);
    let second = add_line(&mut f.sketch, c, d);
    assert!(operands::build(Relation::Distance, &f.sketch, &[first, second], None).is_ok());
    assert!(
        (dimensions::initial(Relation::Distance, &f.sketch, &[first, second])
            - 5.0 / 2.0f64.sqrt())
        .abs()
            < 1e-12
    );
}

#[test]
fn circular_pair_matrix_includes_circle_circle_arc_arc_and_point_centers() {
    let mut f = fixture();
    let center = point(&mut f.sketch, 30.0, 20.0);
    let circle = EntityId::new();
    let arc = EntityId::new();
    f.sketch
        .add_entity(SketchEntity::circle(circle, center, 4.0, false).unwrap())
        .unwrap();
    f.sketch
        .add_entity(SketchEntity::arc(arc, center, 5.0, 0.2, 1.1, false).unwrap())
        .unwrap();
    for pair in [
        [f.circle, SketchTarget::Entity(circle)],
        [f.arc, SketchTarget::Entity(arc)],
        [f.circle, f.arc],
    ] {
        for relation in [Relation::Tangent, Relation::Concentric, Relation::Equal] {
            assert!(operands::build(relation, &f.sketch, &pair, None).is_ok());
            assert!(operands::build(relation, &f.sketch, &[pair[1], pair[0]], None).is_ok());
        }
    }
    let center = SketchTarget::Point {
        entity: circle,
        role: CurvePosition::Center,
    };
    let fixed = operands::build(Relation::Fixed, &f.sketch, &[center], None).unwrap();
    assert!(matches!(
        fixed,
        Constraint::Fixed {
            point_position: CurvePosition::Center,
            ..
        }
    ));
    let endpoint = SketchTarget::Point {
        entity: f.line.owner_id(),
        role: CurvePosition::Start,
    };
    assert_eq!(
        endpoint.constraint_point(&f.sketch),
        f.point.constraint_point(&f.sketch)
    );
}

fn session_for_editor(draft: &Draft) -> SketchSession {
    let mut f = fixture();
    f.sketch.id = draft.key.sketch_id;
    let mut interaction = onecad_interaction::sketch::SketchInteraction::default();
    interaction.dispatch(SketchAction::ContextChanged(Some(draft.document.clone())));
    interaction.dispatch(SketchAction::Begin {
        sketch: Box::new(f.sketch),
        create: false,
        context: draft.document.clone(),
    });
    let mut session = interaction.session.unwrap();
    session.key = draft.key.clone();
    session.phase = onecad_interaction::sketch::SketchPhase::Active;
    session
}

#[test]
fn pending_dimension_keeps_text_until_correlated_terminal_result() {
    let mut draft = draft();
    draft.text = "hole_diameter + 2".into();
    let mut session = session_for_editor(&draft);
    draft.pending = Some(session.request.clone());
    session.phase = onecad_interaction::sketch::SketchPhase::Submitting;
    assert!(matches!(
        editor::settle(&mut draft, &session),
        editor::Settlement::Waiting
    ));
    assert_eq!(draft.text, "hole_diameter + 2");
    session.phase = onecad_interaction::sketch::SketchPhase::Active;
    session.status =
        onecad_interaction::sketch::SketchStatus::Refused("Conflicting dimension".into());
    assert!(matches!(
        editor::settle(&mut draft, &session),
        editor::Settlement::Retry
    ));
    assert_eq!(draft.error.as_deref(), Some("Conflicting dimension"));
    assert_eq!(draft.text, "hole_diameter + 2");
    assert!(draft.pending.is_none());
}

#[test]
fn replaced_submission_cannot_restore_dimension_editor() {
    let mut draft = draft();
    let mut session = session_for_editor(&draft);
    draft.pending = Some(session.request.clone());
    session.request.key.generation += 1;
    assert!(matches!(
        editor::settle(&mut draft, &session),
        editor::Settlement::Complete
    ));
}

#[test]
fn dimension_chip_position_stays_fixed_and_clamps_on_resize() {
    let ctx = egui::Context::default();
    let mut draft = draft();
    let anchor = egui::pos2(700.0, 400.0);
    draft.position = Some(anchor);
    let mut previous = None;
    for width in [1280.0, 1280.0, 900.0] {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 560.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                editor(ui.ctx(), &mut draft, &[]);
            },
        );
        output.textures_delta.clear();
        let bounds = ctx
            .data(|data| data.get_temp::<egui::Rect>(editor_id().with("bounds")))
            .unwrap();
        assert!(screen.shrink(8.0).contains_rect(bounds));
        assert_eq!(draft.position, Some(anchor));
        if width == 1280.0 {
            if let Some(previous) = previous {
                assert_eq!(bounds.min, previous);
            }
            previous = Some(bounds.min);
        } else {
            assert!(bounds.right() <= 892.0);
        }
    }
}

#[test]
fn pending_dimension_enter_cannot_submit_twice_and_escape_dismisses() {
    let ctx = egui::Context::default();
    let mut draft = draft();
    let session = session_for_editor(&draft);
    draft.pending = Some(session.request.clone());
    for (key, expected) in [
        (egui::Key::Enter, (false, false)),
        (egui::Key::Escape, (false, true)),
    ] {
        let mut result = (true, false);
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                result = editor(ui.ctx(), &mut draft, &[]);
            },
        );
        output.textures_delta.clear();
        assert_eq!(result, expected);
    }
}

#[test]
fn opening_dimension_editor_reads_placement_before_storing_draft() {
    let ctx = egui::Context::default();
    let draft = draft();
    let session = session_for_editor(&draft);
    open_editor(&ctx, &session, draft.constraint, true);
    assert!(editor_open(&ctx));
}

#[test]
fn dimension_editor_stays_reachable_in_both_themes_at_supported_sizes_and_scale() {
    for dark in [false, true] {
        for (width, height, scale) in [
            (900.0, 560.0, 1.0),
            (1280.0, 820.0, 1.0),
            (1800.0, 1100.0, 2.0),
        ] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(dark).unwrap(), dark);
            let mut draft = draft();
            let viewport =
                egui::Rect::from_min_max(egui::pos2(40.0, 44.0), egui::pos2(width - 280.0, height));
            draft.viewport = Some(viewport);
            draft.position = Some(egui::pos2(width, height));
            draft.error = Some("Conflicting dimension; edit its expression or cancel".into());
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    ..Default::default()
                },
                |ui| {
                    editor(ui.ctx(), &mut draft, &[]);
                },
            );
            output.textures_delta.clear();
            let bounds = ctx
                .data(|data| data.get_temp::<egui::Rect>(editor_id().with("bounds")))
                .unwrap();
            assert!(bounds.width() <= crate::dimension_chip::WIDTH + 2.0);
            assert!(bounds.height() <= 104.0);
            assert!(
                viewport.contains_rect(bounds),
                "dark={dark} size={width}x{height} scale={scale}"
            );
        }
    }
}
