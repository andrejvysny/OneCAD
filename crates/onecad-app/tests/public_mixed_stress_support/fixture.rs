use super::*;
use onecad_core::document::record::{ImportSourceCodec, ImportStepParams, IMPORT_HEAL_POLICY_V1};
use onecad_core::io::container::{ContainerCaches, ContainerWriter, SaveMeta};
use onecad_core::io::imports::{ImportBlob, ImportBlobs, OwnedSource};

pub(super) fn def(i: usize) -> ComponentDefId {
    ComponentDefId(Uuid::from_u128(0xD200 + i as u128))
}
pub(super) fn occ(i: usize) -> OccurrenceId {
    OccurrenceId(Uuid::from_u128(0xD400 + i as u128))
}

fn native(path: &Path, index: usize, name: &str) -> ComponentDef {
    let saved = ContainerReader::open(path).unwrap();
    assert!(saved.import_blobs().is_empty());
    assert!(saved.library_archives().is_empty());
    let source = saved.document();
    if index == 5 {
        assert_eq!(source.timeline.records().len(), 32);
    }
    if index == 6 {
        assert_eq!(source.timeline.records().len(), 8);
    }
    assert!(!source.timeline.records().is_empty());
    assert!(source
        .timeline
        .records()
        .iter()
        .all(|record| !matches!(record.op, Operation::Known(KnownOperation::ImportStep(_)))));
    ComponentDef {
        id: def(index),
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

pub(super) fn insert_source(
    document: &mut Document,
    imports: &mut ImportBlobs,
    index: usize,
    name: &str,
    bytes: &[u8],
    expected: &str,
    codec: ImportSourceCodec,
) {
    let sha = format!("{:x}", Sha256::digest(bytes));
    assert_eq!(sha, expected, "source {name}");
    let params = ImportStepParams {
        source_sha256: sha.clone(),
        source_codec: codec,
        source_name: name.into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(1.0),
        brep_format: (codec == ImportSourceCodec::Xbf).then_some(12),
        provenance_sha256: None,
        extra: Default::default(),
    };
    let mut part = ComponentDef::new(def(index), name);
    part.timeline.insert_at_cursor(OperationRecord::new(
        RecordId(Uuid::from_u128(0xD300 + index as u128)),
        0,
        format!("Imported {name}"),
        Operation::Known(KnownOperation::ImportStep(params)),
    ));
    document.definitions.insert(part.id, part);
    imports.insert(
        sha,
        ImportBlob {
            codec,
            bytes: Arc::new(OwnedSource::untracked(bytes.to_vec())),
        },
    );
}

fn imported(document: &mut Document, imports: &mut ImportBlobs) {
    let robo: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-derivatives/robonine/analytic/observations.json"
    ))
    .unwrap();
    let leaves: [&[u8]; 4] = [
        include_bytes!("../../../../fixtures/reference-derivatives/robonine/analytic/part-010.xbf"),
        include_bytes!("../../../../fixtures/reference-derivatives/robonine/analytic/part-020.xbf"),
        include_bytes!("../../../../fixtures/reference-derivatives/robonine/analytic/part-030.xbf"),
        include_bytes!("../../../../fixtures/reference-derivatives/robonine/analytic/part-040.xbf"),
    ];
    let hashes = [
        "f643cfdf9ec480a23d14888bf896cad30e02bd05be079a0baf11f7fc88bd1992",
        "932cc0452078850d9059ed6ad0716dd3658d35b8117c5598c2f0699e2b14b21c",
        "d9880bc3f2d82b2e9cf49919603ada70af7e3f4c7d9d60caeb3f81a26fc28d8e",
        "2e780afa05c59f10fb3a8c4acf1cc0c2289d45312dedc0172bc7fa83ecbeffc0",
    ];
    for (i, bytes) in leaves.into_iter().enumerate() {
        let name = robo["parts"][i]["product_name"].as_str().unwrap();
        let hash = robo["parts"][i]["output_sha256"].as_str().unwrap();
        assert_eq!(hash, hashes[i]);
        insert_source(
            document,
            imports,
            i + 1,
            name,
            bytes,
            hash,
            ImportSourceCodec::Xbf,
        );
    }
    let prusa: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/prusa-sensor.json"
    ))
    .unwrap();
    assert_eq!(
        prusa["parts"]["box"]["sha256"],
        "c8b06ef5271166c367d27be13e9c8f93a7ae07707033e4d23bd1ed5997285cb6"
    );
    insert_source(
        document,
        imports,
        0,
        "MINI-fsenzor-box.stp",
        include_bytes!("../../../../fixtures/public-cad/prusa/MINI-fsenzor-box.stp"),
        prusa["parts"]["box"]["sha256"].as_str().unwrap(),
        ImportSourceCodec::Step,
    );
    let nist: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/reference-oracles/nist-ftc09.json"
    ))
    .unwrap();
    assert_eq!(
        nist["sourceSha256"],
        "d12866d3ab0fc260efd0f1a5c54af47950be513486f61f65513877045582afa8"
    );
    insert_source(
        document,
        imports,
        8,
        "nist_ftc_09_asme1_rd.stp",
        include_bytes!("../../../../fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp"),
        nist["sourceSha256"].as_str().unwrap(),
        ImportSourceCodec::Step,
    );
}

pub(super) fn write(path: &Path, cover: &Path, lever: &Path, plate: &Path, constrained: &Path) {
    let mut document = Document::new(DocumentId(Uuid::from_u128(0xD100)));
    let mut imports = ImportBlobs::new();
    imported(&mut document, &mut imports);
    for part in [
        native(cover, 5, "Native Prusa cover"),
        native(lever, 6, "Native Prusa lever"),
        native(plate, 9, "Native plate with hole"),
        native(constrained, 7, "Native constrained rectangular plate"),
    ] {
        document.definitions.insert(part.id, part);
    }
    assert_eq!(document.definitions.len(), 10);
    for i in 0..20 {
        document
            .assembly
            .occurrences
            .insert(Occurrence {
                id: occ(i),
                def: def(i / 2),
                parent: None,
                local: RigidTransform::new(
                    Quat::IDENTITY,
                    Vec3d::new((i % 5) as f64 * 1000.0, (i / 5) as f64 * 1000.0, 0.0),
                ),
                grounded: i == 0,
                name: format!("Authored source occurrence {i}"),
            })
            .unwrap();
    }
    ContainerWriter::save_with_imports(
        path,
        &document,
        &ContainerCaches::none(),
        &imports,
        &SaveMeta::default(),
    )
    .unwrap();
    retain_input(path);
}

fn retain_input(path: &Path) {
    use std::io::Write;
    let Some(root) = std::env::var_os("ONECAD_PUBLIC_ASSEMBLY_CAPTURE_DIR") else {
        return;
    };
    let directory = Path::new(&root).join("public-mixed-10x20-input");
    std::fs::create_dir_all(Path::new(&root)).unwrap();
    std::fs::create_dir(&directory).unwrap();
    let bytes = std::fs::read(path).unwrap();
    let artifact = directory.join("assembly.onecad");
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(artifact)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let manifest = serde_json::json!({
        "classification": "authored initial 10-definition/20-occurrence input; regeneration and native GUI qualification are separate",
        "artifactSha256": format!("{:x}", Sha256::digest(&bytes)),
        "sourceCommitFromEnvironment": std::env::var("ONECAD_SOURCE_COMMIT").ok(),
        "geometry": "six pinned public imports and four native histories; synthetic placements and plate dimensions",
    });
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

pub(super) fn retain_manifest() {
    use std::io::Write;
    let Some(root) = std::env::var_os("ONECAD_PUBLIC_ASSEMBLY_CAPTURE_DIR") else {
        return;
    };
    let file = Path::new(&root).join("public-mixed-10x20/sources.json");
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .unwrap();
    let value = serde_json::json!({
        "classification": "shared-service assembly stress; authored placements do not establish physical fit",
        "initialDefinitions": 10, "occurrences": 20,
        "sources": [
            {"definition": 0, "kind": "imported STEP", "source": "fixtures/public-cad/prusa/MINI-fsenzor-box.stp", "sha256": "c8b06ef5271166c367d27be13e9c8f93a7ae07707033e4d23bd1ed5997285cb6", "license": "GPL-3.0-only"},
            {"definitions": [1,2,3,4], "kind": "imported XBF analytic leaves", "source": "fixtures/reference-derivatives/robonine/analytic", "hashes": ["f643cfdf9ec480a23d14888bf896cad30e02bd05be079a0baf11f7fc88bd1992", "932cc0452078850d9059ed6ad0716dd3658d35b8117c5598c2f0699e2b14b21c", "d9880bc3f2d82b2e9cf49919603ada70af7e3f4c7d9d60caeb3f81a26fc28d8e", "2e780afa05c59f10fb3a8c4acf1cc0c2289d45312dedc0172bc7fa83ecbeffc0"], "license": "CERN-OHL-P-2.0"},
            {"definition": 5, "kind": "native 32-record source-derived history", "source": "fixtures/public-cad/prusa/MINI-fsenzor-cover.stp", "sha256": "e6391172f932b141483f7b8b1a0b560c4260ba1116e588873916ba0ecacb2bd5", "license": "GPL-3.0-only"},
            {"definition": 6, "kind": "native 8-record source-derived history, then authored edits", "source": "fixtures/public-cad/prusa/MINI-fsenzor-lever.stp", "sha256": "beee16c2e5c09d26777d4971df8c343d467f926121d1753c6b41bc0b5ff6bbc6", "license": "GPL-3.0-only"},
            {"definition": 7, "kind": "native fully constrained 80 x 50 x 6 mm rectangular plate", "source": "crates/onecad-app/tests/native_plate_support/profile.rs", "classification": "synthetic test dimensions, not public product requirements"},
            {"definition": 8, "kind": "imported STEP", "source": "fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp", "sha256": "d12866d3ab0fc260efd0f1a5c54af47950be513486f61f65513877045582afa8", "license": "see fixtures/public-cad/nist/README.txt", "comparison": "Upstream volume and area, preregistered 1e-6 relative trial; this is not an independently qualified 1e-6 accuracy bound. Source and transfer uncertainty and a second-reader crosscheck remain unresolved."},
            {"definition": 9, "kind": "native plate-with-hole history", "source": "crates/onecad-app/tests/fixtures/qa_plate_hole.onecad", "sha256": "a83d1bbac05941030fc97703eea69e9775b2c29326d8c6a91782383c1183d9ec"}
        ],
        "limits": "STEP/XBF sources are imported, not native histories. Native Prusa recipes are source-derived and their diagnostic tolerances are not proof of whole-shape equivalence. Cold reopen completes after artifact capture."
    });
    output
        .write_all(&serde_json::to_vec_pretty(&value).unwrap())
        .unwrap();
}

pub(super) fn verify_saved(path: &Path) {
    let saved = ContainerReader::open(path).unwrap();
    assert_eq!(saved.import_blobs().len(), 6);
    let bytes: [&[u8]; 6] = [
        include_bytes!("../../../../fixtures/public-cad/prusa/MINI-fsenzor-box.stp"),
        include_bytes!("../../../../fixtures/reference-derivatives/robonine/analytic/part-010.xbf"),
        include_bytes!("../../../../fixtures/reference-derivatives/robonine/analytic/part-020.xbf"),
        include_bytes!("../../../../fixtures/reference-derivatives/robonine/analytic/part-030.xbf"),
        include_bytes!("../../../../fixtures/reference-derivatives/robonine/analytic/part-040.xbf"),
        include_bytes!("../../../../fixtures/public-cad/nist/nist_ftc_09_asme1_rd.stp"),
    ];
    for (index, source) in [0, 1, 2, 3, 4, 8].into_iter().zip(bytes) {
        let sha = format!("{:x}", Sha256::digest(source));
        assert_eq!(saved.read_import_blob(&sha).unwrap(), source);
        let part = &saved.document().definitions[&def(index)];
        assert_eq!(part.timeline.records().len(), 1);
        assert!(
            matches!(&part.timeline.records()[0].op, Operation::Known(KnownOperation::ImportStep(p)) if p.source_sha256 == sha)
        );
    }
    for index in [5, 6, 7, 9, 10] {
        let part = &saved.document().definitions[&def(index)];
        assert!(part
            .timeline
            .records()
            .iter()
            .all(|r| !matches!(r.op, Operation::Known(KnownOperation::ImportStep(_)))));
    }
    assert_eq!(
        saved.document().definitions[&def(5)]
            .timeline
            .records()
            .len(),
        32
    );
    assert_eq!(
        saved.document().definitions[&def(6)]
            .timeline
            .records()
            .len(),
        9
    );
    assert_eq!(
        saved.document().definitions[&def(10)]
            .timeline
            .records()
            .len(),
        8
    );
}
