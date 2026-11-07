//! Assemble independently authored native parts with the pinned imported box.
use onecad_assembly::math::{Quat, RigidTransform, Vec3d};
use onecad_assembly::occurrence::{ComponentDefId, Occurrence, OccurrenceId};
use onecad_core::document::{
    component::ComponentDef,
    record::{
        ImportSourceCodec, ImportStepParams, KnownOperation, Operation, OperationRecord,
        IMPORT_HEAL_POLICY_V1,
    },
    variables::Scalar,
    Document,
};
use onecad_core::ids::{BodyId, DocumentId, RecordId};
use onecad_core::io::{
    container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
    imports::{ImportBlob, ImportBlobs, OwnedSource},
};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use uuid::Uuid;

pub fn definition(index: u128) -> ComponentDefId {
    ComponentDefId(Uuid::from_u128(0xA200 + index))
}

pub fn occurrence(index: u128) -> OccurrenceId {
    OccurrenceId(Uuid::from_u128(0xA400 + index))
}

fn vector(value: &serde_json::Value) -> Vec3d {
    Vec3d::new(
        value[0].as_f64().unwrap(),
        value[1].as_f64().unwrap(),
        value[2].as_f64().unwrap(),
    )
}

fn native_part(path: &Path, index: u128, name: &str, expected_records: usize) -> ComponentDef {
    let saved = ContainerReader::open(path).unwrap();
    assert!(saved.import_blobs().is_empty());
    assert!(saved.library_archives().is_empty());
    let source = saved.document();
    assert_eq!(source.timeline.records().len(), expected_records);
    let body = match index {
        1 => BodyId(Uuid::from_u128(0x531103)),
        2 => BodyId(Uuid::from_u128(0x531003)),
        _ => panic!("unexpected native part index {index}"),
    };
    assert!(source.bodies.contains(body), "native body identity missing");
    assert!(source
        .timeline
        .records()
        .iter()
        .all(|record| !matches!(record.op, Operation::Known(KnownOperation::ImportStep(_)))));
    ComponentDef {
        id: definition(index),
        name: name.into(),
        variables: source.variables.clone(),
        sketches: source.sketches.clone(),
        sketch_visibility: source.sketch_visibility.clone(),
        datum_planes: source.datum_planes.clone(),
        timeline: source.timeline.clone(),
        bodies: source.bodies.clone(),
        elements: source.elements.clone(),
        repair: source.repair.clone(),
        named_selections: source.named_selections.clone(),
        extra: source.root_definition_extra.clone(),
    }
}

pub fn write(path: &Path, cover: &Path, lever: &Path) {
    let source = include_bytes!("../../../../fixtures/public-cad/prusa/MINI-fsenzor-box.stp");
    let oracle: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    let sha = format!("{:x}", Sha256::digest(source));
    assert_eq!(sha, oracle["parts"]["box"]["sha256"].as_str().unwrap());
    let mut document = Document::new(DocumentId(Uuid::from_u128(0xA100)));
    let box_def = box_part(&sha);
    document.definitions.insert(box_def.id, box_def);
    for part in [
        native_part(cover, 1, "Cover", 32),
        native_part(lever, 2, "Lever", 8),
    ] {
        document.definitions.insert(part.id, part);
    }
    place_parts(&mut document, &oracle);
    let mut imports = ImportBlobs::new();
    imports.insert(
        sha,
        ImportBlob {
            codec: ImportSourceCodec::Step,
            bytes: Arc::new(OwnedSource::untracked(source.to_vec())),
        },
    );
    let meta = SaveMeta {
        app_version: "prusa-mixed-test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save_with_imports(path, &document, &ContainerCaches::none(), &imports, &meta)
        .unwrap();
}

fn box_part(sha: &str) -> ComponentDef {
    let mut box_def = ComponentDef::new(definition(0), "Box");
    let params = ImportStepParams {
        source_sha256: sha.to_owned(),
        source_codec: ImportSourceCodec::Step,
        source_name: "MINI-fsenzor-box.stp".into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(1.0),
        brep_format: None,
        provenance_sha256: None,
        extra: Default::default(),
    };
    box_def.timeline.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::from_u128(0xA300)),
        0,
        "Imported box",
        Operation::Known(KnownOperation::ImportStep(params)),
    ));
    box_def
}

fn place_parts(document: &mut Document, oracle: &serde_json::Value) {
    let turn = Quat::from_axis_angle(Vec3d::X, std::f64::consts::FRAC_PI_2).unwrap();
    let box_axis = vector(&oracle["parts"]["box"]["candidate"]["originMm"]);
    let lever_axis = vector(&oracle["parts"]["lever"]["candidate"]["originMm"]);
    let lever_pose = RigidTransform::new(turn, box_axis - turn.rotate(lever_axis));
    let placements = [
        RigidTransform::IDENTITY,
        RigidTransform::new(turn, Vec3d::ZERO),
        lever_pose,
    ];
    for (i, def) in [0, 1, 2].into_iter().enumerate() {
        document
            .assembly
            .occurrences
            .insert(Occurrence {
                id: occurrence(i as u128),
                def: definition(def),
                parent: None,
                local: placements[i],
                grounded: i == 0,
                name: format!("Part {i}"),
            })
            .unwrap();
    }
}
