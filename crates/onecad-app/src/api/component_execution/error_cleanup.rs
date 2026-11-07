use super::*;

async fn definition(
    state: &AppState,
    session: &str,
    id: ComponentDefId,
) -> crate::document_runtime::component_execution::ComponentReplayInput {
    state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .prepare_component_replay(id, session)
        .unwrap()
}

pub(super) async fn failed_replay_releases_capacity(
    state: &AppState,
    session: &str,
    id: ComponentDefId,
    host: crate::host::HostContext,
) {
    use onecad_core::{
        document::{
            record::{KnownOperation, Operation},
            variables::Scalar,
        },
        edit::{AssemblyEdit, EditCommand},
    };
    let record = definition(state, session, id)
        .await
        .definition
        .timeline
        .records()
        .iter()
        .find(|r| matches!(r.op, Operation::Known(KnownOperation::Extrude(_))))
        .unwrap()
        .clone();
    let mut op = record.op;
    let Operation::Known(KnownOperation::Extrude(ref mut params)) = op else {
        unreachable!()
    };
    params.distance = Scalar::new(0.0);
    apply_assembly_edit(
        state,
        session,
        AssemblyEdit::EditDefinition {
            id,
            command: Box::new(EditCommand::UpdateOperationParams {
                record: record.record_id,
                op,
            }),
        },
    )
    .await
    .unwrap();
    let before = serde_json::to_value(&definition(state, session, id).await.definition).unwrap();
    let error = regenerate_component(state, session, id).await.unwrap_err();
    assert!(
        matches!(&error,ApiError::InvalidCommand(message) if message.contains("valid geometry")),
        "{error:?}"
    );
    assert_eq!(
        serde_json::to_value(&definition(state, session, id).await.definition).unwrap(),
        before
    );
    assert!(state.component_worker.lock().unwrap().is_none());
    let permit = execution_slot()
        .try_acquire_owned()
        .expect("failed replay must finish teardown before returning");
    drop(permit);
    crate::api::undo(state, host).await.unwrap();
}

pub(super) async fn unpromoted_read_only_replay(
    state: &AppState,
    session: &str,
    id: ComponentDefId,
) {
    let before = get_assembly_projection(state, session).await.unwrap();
    let mut input = definition(state, session, id).await;
    assert!(input.definition.elements.is_empty());
    input.read_only = true;
    let lease = open_worker(state, &input).await.unwrap();
    let output = input
        .drive(lease.worker.clone(), lease.epoch, false)
        .await
        .unwrap();
    assert!(!output.minted);
    // Feature-input identities may be acquired during replay; untouched faces
    // must remain transient when blanket promotion is disabled.
    assert!(output.meshes.values().any(|bytes| {
        let mesh = onecad_protocol::mesh::MeshView::decode(bytes, Default::default()).unwrap();
        (0..mesh.header().face_count as usize)
            .any(|row| mesh.face_id(row).is_some_and(|id| !id.starts_with("el_")))
    }));
    lease.drain().await;
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(get_assembly_projection(state, session).await.unwrap()).unwrap()
    );
}
