//! Real containers, with process loss simulated by retiring hosts and marking
//! their durable checkpoints as dead-owner offers; this is not native evidence.
use super::*;
use onecad_app::autosave::{checkpoint_state_until, AutosaveAttempt};
use onecad_core::{
    document::variables::{Scalar, Variable},
    edit::EditCommand,
    ids::{DocumentId, VariableId},
    io::recovery::{marker_path, write_marker, SessionMarker},
};

fn states() -> Vec<AppState> {
    let services = Arc::new(ProcessServices::new());
    (0..3)
        .map(|_| AppState::new_with_services(Arc::new(pending_bundle), services.clone()))
        .collect()
}

async fn edit(state: &AppState, document: usize, phase: usize) {
    let mut guard = state.runtime.lock().await;
    let runtime = guard.as_mut().unwrap();
    runtime
        .apply(EditCommand::AddVariable {
            variable: Variable {
                id: VariableId::new(),
                name: format!("Depth{document}_{phase}"),
                value: Scalar::new((document * 10 + phase + 1) as f64),
                unit: Default::default(),
            },
        })
        .unwrap();
    assert!(runtime.undo().is_some());
    assert!(runtime.redo().unwrap().is_some());
}

async fn checkpoint(state: &AppState, root: &Path) {
    assert!(matches!(
        checkpoint_state_until(state, root, None, std::future::pending()).await,
        AutosaveAttempt::Written(_)
    ));
}

fn simulate_process_loss(states: Vec<AppState>, root: &Path, ids: &[DocumentId]) {
    drop(states);
    for id in ids {
        let mut marker: SessionMarker =
            serde_json::from_slice(&std::fs::read(marker_path(root, *id)).unwrap()).unwrap();
        marker.pid = i32::MAX as u32;
        write_marker(root, &marker).unwrap();
    }
}

async fn assert_contents(
    state: &AppState,
    id: DocumentId,
    path: &Path,
    index: usize,
    phases: usize,
) {
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().unwrap();
    assert_eq!(runtime.document_uuid(), id);
    assert_eq!(runtime.path(), Some(path));
    let variables = runtime.variables();
    assert_eq!(variables.len(), phases);
    for (phase, variable) in variables.iter().enumerate() {
        assert_eq!(variable.name, format!("Depth{index}_{phase}"));
        assert_eq!(variable.value.value, (index * 10 + phase + 1) as f64);
    }
    assert_eq!(
        state.services().reservations().path_owner(path),
        Some(state.host_id())
    );
}

#[tokio::test]
async fn three_dirty_documents_recover_independently_twice_then_save_and_cold_reopen() {
    let area = tempfile::tempdir().unwrap();
    let host = host(area.path());
    let data = host.paths.app_data.as_deref().unwrap();
    let paths: Vec<_> = (0..3)
        .map(|i| area.path().join(format!("part{i}.onecad")))
        .collect();
    let initial = states();
    let mut ids = Vec::new();
    let mut saved_before = Vec::new();
    for (index, state) in initial.iter().enumerate() {
        api::new_document(state, host.clone()).await.unwrap();
        edit(state, index, 0).await;
        api::save_document(state, host.clone(), Some(s(&paths[index])), None)
            .await
            .unwrap();
        saved_before.push(std::fs::read(&paths[index]).unwrap());
        edit(state, index, 1).await;
        checkpoint(state, data).await;
        ids.push(state.runtime.lock().await.as_ref().unwrap().document_uuid());
    }
    simulate_process_loss(initial, data, &ids);
    let restored = states();
    assert_eq!(
        api::check_recovery(&restored[0], host.clone())
            .await
            .unwrap()
            .len(),
        3
    );
    for index in [2, 0, 1] {
        let state = &restored[index];
        api::recover_document(state, host.clone(), ids[index].to_string(), true)
            .await
            .unwrap()
            .unwrap();
        assert_contents(state, ids[index], &paths[index], index, 2).await;
        assert!(api::get_projection(state).await.unwrap().dirty);
        assert_eq!(std::fs::read(&paths[index]).unwrap(), saved_before[index]);
        edit(state, index, 2).await;
        checkpoint(state, data).await;
    }
    assert_eq!(restored[0].services().reservations().open_documents(), 3);
    simulate_process_loss(restored, data, &ids);
    let recovered = states();
    assert_eq!(
        api::check_recovery(&recovered[0], host.clone())
            .await
            .unwrap()
            .len(),
        3
    );
    for index in [1, 2, 0] {
        let state = &recovered[index];
        api::recover_document(state, host.clone(), ids[index].to_string(), true)
            .await
            .unwrap()
            .unwrap();
        assert_contents(state, ids[index], &paths[index], index, 3).await;
        api::save_document(state, host.clone(), None, None)
            .await
            .unwrap();
        api::close_document(state, host.clone()).await.unwrap();
    }
    assert!(api::check_recovery(&recovered[0], host.clone())
        .await
        .unwrap()
        .is_empty());
    assert_eq!(recovered[0].services().reservations().open_documents(), 0);
    drop(recovered);
    let reopened = states();
    for (index, state) in reopened.iter().enumerate() {
        api::open_document(state, host.clone(), s(&paths[index]), None)
            .await
            .unwrap();
        assert_contents(state, ids[index], &paths[index], index, 3).await;
        assert!(!api::get_projection(state).await.unwrap().dirty);
    }
}
