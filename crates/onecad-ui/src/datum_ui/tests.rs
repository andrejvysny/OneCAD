use super::*;
use onecad_app::api;
use onecad_core::{
    edit::{EditCommand, SketchEditOp},
    ids::{EntityId, SketchId},
    math::Vec2,
    sketch::SketchEntity,
};
use onecad_interaction::DocumentContext;

use crate::sketch_services::test_support as support;

const SERVICE_TEST: &str = "datum_ui::tests::datum_authority_history_and_fresh_reopen";

#[test]
fn signed_offset_requires_finite_mm_and_cancel_only_discards_local_draft() {
    for text in ["", "NaN", "inf", "-inf", "1e999", "missing", "10 cm"] {
        assert!(definition(WorldPlane::XY, text).is_err(), "{text}");
    }
    for (plane, text, offset) in [
        (WorldPlane::XY, "12.5", 12.5),
        (WorldPlane::XZ, "-4", -4.0),
        (WorldPlane::YZ, "0", 0.0),
    ] {
        let datum = definition(plane, text).unwrap();
        assert_eq!(datum.base_plane_id, plane_name(plane));
        assert_eq!(datum.offset, offset);
        assert!(!datum.resolved_valid, "only the core may resolve the datum");
    }
    let mut entry = crate::sketch_entry::Entry::default();
    entry.open = true;
    entry.datum.open();
    entry.datum.offset = "27.125".into();
    entry.dismiss();
    assert!(!entry.open && !entry.datum.open);
    assert_eq!(entry.datum.offset, "10");
    assert!(entry.pending.is_none());
}

#[test]
fn unresolved_and_invalid_frames_cannot_start_datum_sketches() {
    let mut datum = datum_dto();
    assert!(sketch(&datum).is_ok());
    datum.resolved_valid = false;
    assert!(sketch(&datum).unwrap_err().contains("unresolved"));
    datum.resolved_valid = true;
    datum.plane.normal = [0.0; 3];
    assert!(sketch(&datum).unwrap_err().contains("invalid"));
}

fn datum_dto() -> onecad_app::dto::DatumDto {
    onecad_app::dto::DatumDto {
        id: DatumPlaneId::new().to_string(),
        name: "Offset".into(),
        kind: "OffsetFromPlane".into(),
        base_plane_id: "XY".into(),
        offset: 12.5,
        plane: SketchPlane {
            origin: Vec3::new_unchecked(0.0, 0.0, 12.5),
            ..SketchPlane::xy()
        }
        .into(),
        resolved_valid: true,
    }
}

#[test]
fn datum_authority_history_and_fresh_reopen() {
    support::required_worker();
    if let Some(phase) = std::env::var_os("ONECAD_NATIVE_PLATE_TEST_PHASE") {
        let root =
            std::path::PathBuf::from(std::env::var_os("ONECAD_NATIVE_PLATE_TEST_ROOT").unwrap());
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(service_phase(&root, phase == "datum-reopen"));
    } else {
        let root = tempfile::tempdir().unwrap();
        support::run_isolated_phase(root.path(), "datum-author", SERVICE_TEST);
        support::run_isolated_phase(root.path(), "datum-reopen", SERVICE_TEST);
    }
}

async fn service_phase(root: &std::path::Path, reopen: bool) {
    let harness = support::Harness::new(&root.join(if reopen {
        "datum-reopen-host"
    } else {
        "datum-author-host"
    }))
    .await;
    let path = root.join("offset-datum.onecad");
    if reopen {
        api::open_document(
            &harness.state,
            harness.host.clone(),
            path.to_string_lossy().into_owned(),
            None,
        )
        .await
        .unwrap();
        let projection = harness.projection().await;
        harness.settled(&projection).await;
        verify_reopened(&harness).await;
    } else {
        api::new_document(&harness.state, harness.host.clone())
            .await
            .unwrap();
        assert!(
            harness
                .state
                .live_worker()
                .unwrap()
                .wait_ready(std::time::Duration::from_secs(15))
                .await
        );
        numeric_cancel_preserves_document(&harness).await;
        refusal_preserves_document(&harness).await;
        for (plane, offset) in [
            (WorldPlane::XY, "12.5"),
            (WorldPlane::XZ, "-4"),
            (WorldPlane::YZ, "6"),
        ] {
            author_on_datum(&harness, plane, offset).await;
        }
        api::save_document(
            &harness.state,
            harness.host.clone(),
            Some(path.to_string_lossy().into_owned()),
            None,
        )
        .await
        .unwrap();
    }
    harness.close().await;
}

async fn numeric_cancel_preserves_document(harness: &support::Harness) {
    let before = harness.projection().await;
    let ctx = egui::Context::default();
    onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(false).unwrap(), false);
    ctx.enable_accesskit();
    let mut draft = Draft::default();
    form_frame(&ctx, &mut draft, Vec::new());
    let (_, _, output) = form_frame(&ctx, &mut draft, Vec::new());
    form_frame(
        &ctx,
        &mut draft,
        pointer_click(widget_center(&output, "onecad.datum.offset")),
    );
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    };
    form_frame(
        &ctx,
        &mut draft,
        vec![
            egui::Event::ModifiersChanged(command),
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: command,
            },
            egui::Event::Text("-27.125".into()),
        ],
    );
    assert_eq!(draft.offset, "-27.125");
    let (create, cancel, _) = form_frame(
        &ctx,
        &mut draft,
        pointer_click(widget_center(&output, "onecad.datum.cancel")),
    );
    assert!(!create && cancel);
    assert_eq!(harness.projection().await, before);
}

fn widget_center(output: &egui::FullOutput, id: &str) -> egui::Pos2 {
    let node = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .map(|(_, node)| node)
        .find(|node| node.author_id() == Some(id))
        .unwrap();
    let rect = node.bounds().unwrap();
    egui::pos2(
        ((rect.x0 + rect.x1) / 2.0) as f32,
        ((rect.y0 + rect.y1) / 2.0) as f32,
    )
}

fn pointer_click(position: egui::Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(position),
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn form_frame(
    ctx: &egui::Context,
    draft: &mut Draft,
    events: Vec<egui::Event>,
) -> (bool, bool, egui::FullOutput) {
    let mut result = (false, false);
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            ..Default::default()
        },
        |ui| result = offset_form(draft, ui, true),
    );
    output.textures_delta.clear();
    (result.0, result.1, output)
}

async fn context(harness: &support::Harness) -> DocumentContext {
    let scope = api::preview_context(&harness.state).await.unwrap();
    DocumentContext {
        document_id: scope.document_id,
        runtime_session: scope.runtime_session,
        revision: scope.revision.0,
        snapshot_id: scope.snapshot_id,
        scene: onecad_scene::SceneIdentity {
            session: 1,
            scene: 1,
        },
    }
}

async fn refusal_preserves_document(harness: &support::Harness) {
    let before = harness.projection().await;
    let current = context(harness).await;
    let mut stale = current.clone();
    stale.revision += 1;
    assert!(
        crate::sketch_services::create_datum(
            &harness.state,
            harness.host.clone(),
            &stale,
            definition(WorldPlane::XY, "5").unwrap()
        )
        .await
        .is_err()
    );
    let mut invalid = definition(WorldPlane::XY, "5").unwrap();
    invalid.base_plane_id = "missing".into();
    assert!(
        crate::sketch_services::create_datum(
            &harness.state,
            harness.host.clone(),
            &current,
            invalid
        )
        .await
        .is_err()
    );
    assert_eq!(harness.projection().await, before);
}

async fn author_on_datum(harness: &support::Harness, base: WorldPlane, offset: &str) {
    let before = harness.projection().await;
    let datum = definition(base, offset).unwrap();
    let id = datum.id.to_string();
    let receipt = crate::sketch_services::create_datum(
        &harness.state,
        harness.host.clone(),
        &context(harness).await,
        datum,
    )
    .await
    .unwrap();
    assert!(!receipt.regen_expected);
    assert_eq!(receipt.projection.undo_depth, before.undo_depth + 1);
    let stored = receipt.projection.datums[&id].clone();
    assert!(stored.resolved_valid);
    let undo = api::undo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(!undo.projection.datums.contains_key(&id));
    if undo.regen_expected {
        harness.settled(&undo.projection).await;
    }
    let redo = api::redo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    assert!(!redo.regen_expected);
    assert_eq!(redo.projection.datums[&id], stored);
    add_sketch(harness, &stored).await;
}

async fn add_sketch(harness: &support::Harness, datum: &onecad_app::dto::DatumDto) {
    let expected = sketch(datum).unwrap();
    let mut supplied = expected.clone();
    // The client frame is intentionally wrong: AddSketch must stamp the datum authority.
    supplied.plane = SketchPlane::xy();
    let id = supplied.id;
    api::apply_edit_command_with_completion(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddSketch { sketch: supplied },
    )
    .await
    .unwrap()
    .result
    .unwrap();
    assert_eq!(
        api::get_sketch_snapshot(&harness.state, id).await.unwrap(),
        expected
    );
    let undo = api::undo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    if undo.regen_expected {
        harness.settled(&undo.projection).await;
    }
    assert!(!undo.projection.sketches.contains_key(&id.to_string()));
    let redo = api::redo_with_outcome(&harness.state, harness.host.clone())
        .await
        .unwrap();
    if redo.regen_expected {
        harness.settled(&redo.projection).await;
    }
    assert_eq!(
        api::get_sketch_snapshot(&harness.state, id).await.unwrap(),
        expected
    );
    profile(harness, id).await;
}

async fn profile(harness: &support::Harness, id: SketchId) {
    api::enter_sketch_with_completion(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        Some(true),
    )
    .await
    .unwrap()
    .result
    .unwrap();
    let center = EntityId::new();
    let curve = EntityId::new();
    let receipt = api::author_sketch_primitive_controlled(
        &harness.state,
        harness.host.clone(),
        id.to_string(),
        vec![
            SketchEditOp::AddEntity {
                entity: SketchEntity::point(center, Vec2::new_unchecked(0.0, 0.0), false, false),
            },
            SketchEditOp::AddEntity {
                entity: SketchEntity::circle(curve, center, 5.0, false).unwrap(),
            },
        ],
        Default::default(),
    )
    .await
    .unwrap();
    assert!(receipt.result.unwrap().accepted);
    if receipt.regen_expected {
        harness.settled(&receipt.projection).await;
    }
    let receipt =
        api::finish_sketch_with_completion(&harness.state, harness.host.clone(), id.to_string())
            .await
            .unwrap();
    assert_eq!(receipt.result.unwrap().regions.len(), 1);
    if receipt.regen_expected {
        harness.settled(&receipt.projection).await;
    }
}

async fn verify_reopened(harness: &support::Harness) {
    let projection = harness.projection().await;
    assert_eq!(projection.datums.len(), 3);
    assert_eq!(projection.sketches.len(), 3);
    let mut attached = std::collections::BTreeSet::new();
    for saved in projection.sketches.values() {
        let actual = api::get_sketch_snapshot(&harness.state, saved.id.parse().unwrap())
            .await
            .unwrap();
        let SketchAttachment::Datum { datum } = actual.attachment else {
            panic!("reopened sketch lost its datum attachment")
        };
        assert!(attached.insert(datum));
        let expected = sketch(&projection.datums[&datum.to_string()]).unwrap();
        assert_eq!(actual.plane, expected.plane);
        assert_eq!(actual.attachment, expected.attachment);
        assert_eq!(actual.entities().len(), 2);
    }
}
