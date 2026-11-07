//! Unqualified assembly input stays refused; no guessed persistent face identity.
use super::*;
use onecad_core::io::container::{ContainerCaches, ContainerWriter, SaveMeta};
use onecad_core::io::imports::ImportBlobs;

fn write(path: &Path) {
    let mut document = Document::new(DocumentId(Uuid::from_u128(0xD100)));
    let mut imports = ImportBlobs::new();
    fixture::insert_source(
        &mut document,
        &mut imports,
        7,
        "ExpansionCard_ThreadedInsert.stp",
        include_bytes!(
            "../../../../fixtures/public-cad/framework/ExpansionCard_ThreadedInsert.stp"
        ),
        "6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09",
        onecad_core::document::record::ImportSourceCodec::Step,
    );
    document
        .assembly
        .occurrences
        .insert(Occurrence {
            id: fixture::occ(14),
            def: fixture::def(7),
            parent: None,
            local: RigidTransform::IDENTITY,
            grounded: true,
            name: "Unqualified complete Framework assembly".into(),
        })
        .unwrap();
    ContainerWriter::save_with_imports(
        path,
        &document,
        &ContainerCaches::none(),
        &imports,
        &SaveMeta::default(),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn framework_assembly_topology_rebind_refuses_without_adoption() {
    host::required_worker();
    let area = tempfile::tempdir().unwrap();
    let path = area.path().join("framework.onecad");
    write(&path);
    let (h, session) = open(&path, &area.path().join("host")).await;
    let before = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    let part = before
        .definitions
        .iter()
        .find(|part| part.id == fixture::def(7).to_string())
        .unwrap();
    assert!(part.bodies.is_empty());
    assert!(part.mesh_generation.is_none());
    let error = api::regenerate_component(&h.state, &session, fixture::def(7))
        .await
        .unwrap_err();
    assert!(
        matches!(error, onecad_app::error::ApiError::UnresolvedTopology(ref message) if message == "component topology identities could not be rebound"),
        "{error:?}"
    );
    let after = api::get_assembly_projection(&h.state, &session)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(&after).unwrap()
    );
    h.close().await;
}
