use super::{api, document, mesh, mesh_compare, replay, Harness};

pub(super) async fn translation_undo_redo(h: &Harness, session: &str, previous: &[u8]) {
    let translated = mesh(&h.state, session, 2, 2).await;
    api::undo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[2]).await;
    mesh_compare::equal(
        "native translation undo",
        previous,
        mesh(&h.state, session, 2, 2).await.as_slice(),
    );
    api::redo(&h.state, h.host.clone()).await.unwrap();
    replay(&h.state, session, &[2]).await;
    mesh_compare::equal(
        "native translation redo",
        translated.as_slice(),
        mesh(&h.state, session, 2, 2).await.as_slice(),
    );
    mesh_compare::equal(
        "independent after translation redo",
        previous,
        mesh(&h.state, session, 3, 3).await.as_slice(),
    );
    let p = api::get_assembly_projection(&h.state, session)
        .await
        .unwrap();
    assert!(p
        .definitions
        .iter()
        .find(|d| d.id == document::definition(2).to_string())
        .unwrap()
        .mesh_generation
        .is_some());
}
