use super::*;
use onecad_core::{
    ids::{SketchId, SnapshotId},
    math::Vec2,
    sketch::{SketchAttachment, SketchEntity, WorldPlane},
};
use onecad_scene::{Camera, SceneIdentity, Viewport};

fn fixture() -> (SketchInteraction, SketchTag, EntityId) {
    let context = DocumentContext {
        document_id: "document".into(),
        runtime_session: "runtime".into(),
        revision: 7,
        snapshot_id: SnapshotId(9),
        scene: SceneIdentity {
            session: 1,
            scene: 1,
        },
    };
    let mut sketch = Sketch::new(
        SketchId::new(),
        "Profile sketch",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    let entity = EntityId::new();
    sketch
        .add_entity(SketchEntity::point(
            entity,
            Vec2::new(0.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    let mut interaction = SketchInteraction::default();
    interaction.dispatch(SketchAction::ContextChanged(Some(context.clone())));
    interaction.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch),
        context,
        create: false,
    });
    interaction.session.as_mut().unwrap().phase = SketchPhase::Active;
    let effects = interaction.dispatch(SketchAction::Finish);
    let onecad_interaction::sketch::SketchEffect::FinishSketch { tag, .. } = &effects[0] else {
        panic!("Finish effect")
    };
    (interaction, tag.clone(), entity)
}

fn diagnostic(entity: EntityId) -> Diagnostic {
    Diagnostic {
        severity: Severity::Warning,
        code: "SKETCH_ENTITY_DEGENERATE".into(),
        message: "Entity is below the detector tolerance".into(),
        stage: Some("profile".into()),
        reason_code: None,
        evidence: Some(serde_json::json!({ "entityId": entity.to_string() })),
    }
}

fn finished(tag: SketchTag, diagnostics: Vec<Diagnostic>) -> SketchAction {
    SketchAction::Finished {
        warning: None,
        tag,
        result: Box::new(onecad_app::dto::FinishSketchDto {
            region_identity_version: 3,
            regions: vec![],
            diagnostics,
        }),
    }
}

#[test]
fn finish_report_keeps_exact_source_and_worker_evidence() {
    let (mut interaction, tag, entity) = fixture();
    let action = finished(tag.clone(), vec![diagnostic(entity)]);
    let report = Report::observe(&interaction, &action).unwrap();
    assert_eq!(report.source, tag);
    assert_eq!(report.regions, Some(0));
    assert_eq!(
        report.rows[0].message,
        "Entity is below the detector tolerance"
    );
    assert_eq!(
        report.rows[0].targets,
        Some(vec![SketchTarget::Entity(entity)])
    );
    interaction.dispatch(action);
    assert!(report.current(&interaction).is_some());
    let mut document = interaction.context.clone().unwrap();
    document.revision += 1;
    interaction.dispatch(SketchAction::ContextChanged(Some(document)));
    assert!(report.current(&interaction).is_none());
}

#[test]
fn stale_document_visit_and_request_do_not_publish_or_revive_reports() {
    let (mut interaction, tag, entity) = fixture();
    let mut stale = tag.clone();
    stale.key.generation += 1;
    assert!(Report::observe(&interaction, &finished(stale, vec![diagnostic(entity)])).is_none());
    let mut stale = tag.clone();
    stale.sequence += 1;
    assert!(Report::observe(&interaction, &finished(stale, vec![diagnostic(entity)])).is_none());
    let report = Report::observe(&interaction, &finished(tag.clone(), vec![])).unwrap();
    let mut document = interaction.context.clone().unwrap();
    document.runtime_session = "new runtime".into();
    interaction.dispatch(SketchAction::ContextChanged(Some(document)));
    assert!(report.current(&interaction).is_none());
    assert!(Report::observe(&interaction, &finished(tag, vec![])).is_none());
}

#[test]
fn unknown_wire_entity_ids_never_turn_into_uuid_or_partial_selection() {
    let (interaction, _, entity) = fixture();
    let sketch = &interaction.session.as_ref().unwrap().authoritative;
    let mut diagnostic = diagnostic(entity);
    diagnostic.evidence =
        Some(serde_json::json!({ "entityIds": [entity.to_string(), "wire_7#seg0"] }));
    let row = Row::new(&diagnostic, sketch);
    assert!(row.targets.is_none());
    assert!(row.evidence.contains("wire_7#seg0"));
}

#[test]
fn selection_requires_active_exact_capture_document_and_reported_entity() {
    let (mut interaction, tag, entity) = fixture();
    let report = Report::observe(&interaction, &finished(tag, vec![diagnostic(entity)])).unwrap();
    let session = interaction.session.as_mut().unwrap();
    let view = ViewSnapshot::new(
        session.document.scene,
        1,
        Camera::default(),
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    session.accepted_capture = Some(view.identity());
    assert!(selection(&report, session, Some(view), &report.rows[0]).is_none());
    session.phase = SketchPhase::Active;
    assert!(
        matches!(selection(&report, session, Some(view), &report.rows[0]), Some(SketchAction::Select { targets, .. }) if targets == vec![SketchTarget::Entity(entity)])
    );
    session.document.snapshot_id = SnapshotId(10);
    assert!(selection(&report, session, Some(view), &report.rows[0]).is_none());
}

#[test]
fn plain_finish_failure_preserves_message_without_invented_diagnostics() {
    let (interaction, tag, _) = fixture();
    let message = "worker profile fill failed after authoring was retained";
    let report = Report::observe(
        &interaction,
        &SketchAction::Failed {
            tag,
            message: message.into(),
        },
    )
    .unwrap();
    assert_eq!(report.failure.as_deref(), Some(message));
    assert!(report.rows.is_empty());
    assert!(report.regions.is_none());
}

#[test]
fn report_rows_are_bounded_and_unicode_remains_valid() {
    let (interaction, tag, entity) = fixture();
    let mut value = diagnostic(entity);
    value.message = "候補".repeat(4096);
    let report = Report::observe(&interaction, &finished(tag, vec![value; 100])).unwrap();
    assert_eq!(report.rows.len(), MAX_ROWS);
    assert_eq!(report.total, 100);
    assert!(report.rows[0].message.len() <= 4099);
    assert!(report.rows[0].message.ends_with('…'));
}

#[test]
fn light_and_dark_editor_show_real_diagnostic_text_without_status_bar() {
    let (interaction, tag, entity) = fixture();
    let report = Report::observe(&interaction, &finished(tag, vec![diagnostic(entity)])).unwrap();
    for dark in [true, false] {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, crate::theme::load(dark).unwrap(), dark);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(
                contents(
                    ui,
                    &report,
                    interaction.session.as_ref().unwrap(),
                    None,
                    false
                )
                .is_none()
            );
        });
        output.textures_delta.clear();
        assert!(format!("{:?}", output.shapes).contains("SKETCH_ENTITY_DEGENERATE"));
    }
}

#[test]
fn malformed_evidence_never_silently_selects_only_known_subset() {
    let (interaction, _, entity) = fixture();
    let mut value = diagnostic(entity);
    for evidence in [
        serde_json::json!({ "entityIds": [entity.to_string(), 7] }),
        serde_json::json!({ "entityId": entity.to_string(), "entityIds": "invalid" }),
    ] {
        value.evidence = Some(evidence);
        assert!(
            Row::new(&value, &interaction.session.as_ref().unwrap().authoritative)
                .targets
                .is_none()
        );
    }
}
