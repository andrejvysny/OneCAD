use super::*;
use onecad_core::io::container::{ContainerCaches, ContainerWriter, SaveMeta};
use std::collections::BTreeSet;

async fn fixture(area: &Path) -> std::path::PathBuf {
    let mut document = Document::new(DocumentId(Uuid::new_v4()));
    if let Some(path) = std::env::var_os("ONECAD_EVIDENCE_TRANSLATION_FIXTURE") {
        let source = ContainerReader::open(Path::new(&path)).unwrap();
        document.definitions.insert(
            fixture::def(6),
            source.document().definitions[&fixture::def(6)].clone(),
        );
    } else {
        let path = area.join("lever.onecad");
        lever::build(&path).await;
        let source = ContainerReader::open(&path).unwrap();
        let source = source.document();
        let mut definition = ComponentDef::new(fixture::def(6), "Native Prusa lever");
        definition.variables = source.variables.clone();
        definition.sketches = source.sketches.clone();
        definition.sketch_visibility = source.sketch_visibility.clone();
        definition.datum_planes = source.datum_planes.clone();
        definition.timeline = source.timeline.clone();
        definition.bodies = source.bodies.clone();
        definition.elements = source.elements.clone();
        document.definitions.insert(definition.id, definition);
    }
    document
        .assembly
        .occurrences
        .insert(Occurrence {
            id: fixture::occ(12),
            def: fixture::def(6),
            parent: None,
            local: RigidTransform::IDENTITY,
            grounded: false,
            name: "Lever".into(),
        })
        .unwrap();
    let path = area.join("minimal-lever.onecad");
    ContainerWriter::save(
        &path,
        &document,
        &ContainerCaches::none(),
        &SaveMeta::default(),
    )
    .unwrap();
    path
}

async fn translate(state: &AppState, session: &str, first: bool, distance: f64) {
    let projection = api::get_assembly_projection(state, session).await.unwrap();
    let part = projection
        .definitions
        .iter()
        .find(|part| part.id == fixture::def(6).to_string())
        .unwrap();
    let op = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets: part
            .bodies
            .iter()
            .map(|body| body.id.parse().unwrap())
            .collect(),
        translate: [Scalar::new(distance), Scalar::new(0.0), Scalar::new(0.0)],
        rotate: Default::default(),
        copy: false,
        extra: Default::default(),
    }));
    let record = RecordId(Uuid::from_u128(0xD6F0));
    let command = if first {
        EditCommand::AddOperation {
            record: OperationRecord::new(record, 0, "Measured lever translation", op),
            at_cursor: true,
        }
    } else {
        EditCommand::UpdateOperationParams { record, op }
    };
    edit(
        state,
        session,
        AssemblyEdit::EditDefinition {
            id: fixture::def(6),
            command: Box::new(command),
        },
    )
    .await;
    replay(state, session, &[6]).await;
}

async fn checked_ids(
    state: &AppState,
    session: &str,
    distance: f64,
) -> (BTreeSet<String>, Arc<Vec<u8>>) {
    let oracle: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    let blobs = meshes(state, session, 6, 12).await;
    assert_eq!(blobs.len(), 1);
    let mesh = MeshView::decode(&blobs[0], MeshLimits::default()).unwrap();
    for side in 0..2 {
        let bounds = if side == 0 {
            mesh.header().bbox_min
        } else {
            mesh.header().bbox_max
        };
        for (axis, actual) in bounds.into_iter().enumerate() {
            let expected = oracle["parts"]["lever"]["boundsMm"][side][axis]
                .as_f64()
                .unwrap()
                + if axis == 0 { distance } else { 0.0 };
            assert!((f64::from(actual) - expected).abs() <= 0.15);
        }
    }
    let ids: BTreeSet<_> = (0..mesh.header().face_count as usize)
        .map(|row| {
            mesh.face_id(row)
                .filter(|id| id.starts_with("el_"))
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(ids.len(), 28);
    (ids, blobs[0].clone())
}

fn relative_geometry(before: &[u8], after: &[u8], distance: f64) {
    let before = MeshView::decode(before, MeshLimits::default()).unwrap();
    let after = MeshView::decode(after, MeshLimits::default()).unwrap();
    assert_eq!(before.positions().len(), after.positions().len());
    for (coordinate, (prior, current)) in before
        .positions()
        .iter()
        .zip(after.positions().iter())
        .enumerate()
    {
        let expected = f64::from(prior) + if coordinate % 3 == 0 { distance } else { 0.0 };
        assert!(
            (f64::from(current) - expected).abs() <= 0.001,
            "coordinate {coordinate}: {current} vs {expected}"
        );
    }
    match (before.normals(), after.normals()) {
        (Some(before), Some(after)) => {
            let max = before
                .iter()
                .zip(after.iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            let first = before
                .iter()
                .zip(after.iter())
                .enumerate()
                .find(|(_, (a, b))| a != b);
            assert_eq!(before.len(), after.len());
            // Recomputed surfaces can perturb near-zero components below one f32 unit-scale rounding step.
            assert!(
                max <= f32::EPSILON,
                "translation changed normals at {distance} mm; max={max}, first={first:?}"
            );
        }
        (None, None) => {}
        _ => panic!("translation changed normal inventory"),
    }
    assert!(
        before.indices().iter().eq(after.indices().iter()),
        "translation changed triangle indices"
    );
    assert_eq!(before.header().face_count, after.header().face_count);
    for row in 0..before.header().face_count as usize {
        assert_eq!(before.face_id(row), after.face_id(row));
        assert_eq!(before.face_range(row), after.face_range(row));
    }
}

async fn checked_again(
    state: &AppState,
    session: &str,
    distance: f64,
    ids: &BTreeSet<String>,
    baseline: &[u8],
) {
    let (current_ids, current) = checked_ids(state, session, distance).await;
    assert_eq!(&current_ids, ids);
    relative_geometry(baseline, &current, distance);
}

#[tokio::test]
async fn same_length_translation_edits_keep_identity_through_zero_and_cold_reopen() {
    let _lane = WORKER_LANE.lock().await;
    let area = tempfile::tempdir().unwrap();
    let path = fixture(area.path()).await;
    let (h, session) = open(&path, &area.path().join("initial")).await;
    replay(&h.state, &session, &[6]).await;
    let (ids, baseline) = checked_ids(&h.state, &session, 0.0).await;
    translate(&h.state, &session, true, 1.0).await;
    checked_again(&h.state, &session, 1.0, &ids, &baseline).await;
    translate(&h.state, &session, false, 0.0).await;
    checked_again(&h.state, &session, 0.0, &ids, &baseline).await;
    translate(&h.state, &session, false, 2.0).await;
    checked_again(&h.state, &session, 2.0, &ids, &baseline).await;
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, &session, &[6]).await;
    checked_again(&h.state, &session, 0.0, &ids, &baseline).await;
    api::redo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, &session, &[6]).await;
    checked_again(&h.state, &session, 2.0, &ids, &baseline).await;
    api::save_document(&h.state, h.host.clone(), None, None)
        .await
        .unwrap();
    h.close().await;
    let (h, session) = open(&path, &area.path().join("fresh")).await;
    replay(&h.state, &session, &[6]).await;
    checked_again(&h.state, &session, 2.0, &ids, &baseline).await;
    h.close().await;
}
