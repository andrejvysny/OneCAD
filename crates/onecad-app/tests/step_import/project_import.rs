use super::*;
use onecad_core::{
    document::refs::{ElementKind, ElementRef, PrimaryRef},
    ids::{ElementId, SnapshotId, TopoKey},
    sketch::SketchAttachment,
};

async fn source_with_hosted_sketch(
    bin: &std::path::Path,
    dir: &std::path::Path,
) -> (PathBuf, ElementId, ElementRef) {
    let step = dir.join("source.step");
    export_fixture(bin, &step, 26.0).await;
    let wm = spawn_worker(bin.to_path_buf()).await;
    let mut runtime = runtime_over(&wm);
    do_import(&wm, &mut runtime, &step, false).await;
    let report = regen_all(&mut runtime).await;
    published(&report, "source import");
    let body = runtime.head_body_ids()[0];
    let promoted = runtime
        .promote_selection(
            SnapshotId(report.snapshot_id),
            body,
            vec![(TopoKey::new("f:1"), None)],
        )
        .await
        .unwrap();
    let id = ElementId::new(&promoted[0].element_id);
    let path = dir.join("source.onecad");
    runtime.save(&path, save_meta()).unwrap();
    let loaded = ContainerReader::open(&path).unwrap();
    let entry = loaded.document().elements.get(&id).unwrap();
    assert!(entry.import_source_identity.is_some());
    let reference = ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: id.clone(),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: entry.descriptor.clone().map(|descriptor| {
            onecad_core::document::refs::IntentQuery {
                version: 1,
                kind: ElementKind::Face,
                descriptor,
                extra: Default::default(),
            }
        }),
        anchor: entry.anchor.clone(),
        import_source_identity: entry.import_source_identity.clone(),
        extra: Default::default(),
    };
    let sketch = Sketch::new(
        SketchId(Uuid::from_u128(0x9871)),
        "certified host",
        SketchAttachment::HostFace {
            face: reference.clone(),
            projected_boundary_version: 0,
        },
    );
    runtime
        .apply(EditCommand::AddSketch {
            sketch: sketch.clone(),
        })
        .unwrap();
    let mut record = sketch_record(0x9872, &sketch);
    let Operation::Known(KnownOperation::Sketch(params)) = &mut record.op else {
        unreachable!()
    };
    params.host_face = Some(reference.clone());
    runtime
        .apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .unwrap();
    published(&regen_all(&mut runtime).await, "hosted sketch source");
    runtime.save(&path, save_meta()).unwrap();
    wm.shutdown().await;
    (path, id, reference)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn certified_project_import_regenerates_undoes_and_cold_reopens() {
    let Some(bin) = real_worker() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (source, id, reference) = source_with_hosted_sketch(&bin, dir.path()).await;
    let path = dir.path().join("target.onecad");
    let wm = spawn_worker(bin.clone()).await;
    let mut runtime = runtime_over(&wm);
    runtime.import_project(&source).unwrap();
    published(&regen_all(&mut runtime).await, "imported hosted sketch");
    let volumes = head_volumes(&mut runtime).await;
    assert_eq!(volumes.len(), 2);
    assert!((volumes[0] - VOL_B).abs() < 1.0);
    assert!((volumes[1] - vol_a(26.0)).abs() < 1.0);
    runtime.undo().unwrap();
    regen_all(&mut runtime).await;
    assert!(runtime.head_body_ids().is_empty());
    runtime.redo().unwrap().unwrap();
    published(
        &regen_all(&mut runtime).await,
        "redo imported hosted sketch",
    );
    assert_eq!(head_volumes(&mut runtime).await, volumes);
    runtime.save(&path, save_meta()).unwrap();
    let loaded = ContainerReader::open(&path).unwrap();
    assert_eq!(
        loaded
            .document()
            .elements
            .get(&id)
            .unwrap()
            .import_source_identity,
        reference.import_source_identity
    );
    assert_eq!(
        loaded.document().sketches[&SketchId(Uuid::from_u128(0x9871))].host_face(),
        Some(&reference)
    );
    wm.shutdown().await;
    drop(runtime);
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(dir.path().join("source.step")).unwrap();
    let wm = spawn_worker(bin).await;
    let mut reopened = open_over(&wm, &path);
    published(
        &regen_all(&mut reopened).await,
        "cold imported hosted sketch",
    );
    assert_eq!(head_volumes(&mut reopened).await, volumes);
    wm.shutdown().await;
}
