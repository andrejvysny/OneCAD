use super::*;

pub(super) fn write_fixture(path: &Path) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/reference-derivatives/robonine/analytic");
    let observations = observation();
    let mut document = Document::new(DocumentId(Uuid::from_u128(0xC100)));
    let mut imports = ImportBlobs::new();
    for i in 0..4 {
        let bytes = std::fs::read(base.join(FILES[i])).unwrap();
        assert_eq!(bytes.len(), SIZES[i]);
        assert_eq!(digest(&bytes), HASHES[i]);
        assert_eq!(observations["parts"][i]["output_sha256"], HASHES[i]);
        assert_eq!(observations["parts"][i]["before"]["solids"], 1);
        assert_eq!(observations["parts"][i]["before"]["faces"], FACES[i]);
        let mut part = ComponentDef::new(
            def(i),
            observations["parts"][i]["product_name"].as_str().unwrap(),
        );
        part.timeline.insert_at_cursor(OperationRecord::new(
            record(i),
            0,
            "Imported analytic source",
            Operation::Known(KnownOperation::ImportStep(params(i, 1.0))),
        ));
        document.definitions.insert(part.id, part);
        imports.insert(
            HASHES[i].into(),
            ImportBlob {
                codec: ImportSourceCodec::Xbf,
                bytes: Arc::new(OwnedSource::untracked(bytes)),
            },
        );
    }
    let turn = Quat::from_axis_angle(Vec3d::Z, std::f64::consts::PI).unwrap();
    for (i, definition) in MAP.iter().enumerate() {
        document
            .assembly
            .occurrences
            .insert(Occurrence {
                id: occ(i),
                def: def(*definition),
                parent: None,
                local: if i < 4 {
                    RigidTransform::IDENTITY
                } else {
                    RigidTransform::new(turn, Vec3d::ZERO)
                },
                grounded: i == 0,
                name: format!("source occurrence {}", i + 1),
            })
            .unwrap();
    }
    let meta = SaveMeta {
        app_version: "robonine-analytic-test".into(),
        occt_fingerprint: None,
        created: String::new(),
        modified: String::new(),
    };
    ContainerWriter::save_with_imports(path, &document, &ContainerCaches::none(), &imports, &meta)
        .unwrap();
}
