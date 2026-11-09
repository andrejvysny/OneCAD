use super::*;
use onecad_core::io::container::SaveLimits;

#[tokio::test]
async fn rejected_authoritative_payload_preserves_save_save_as_and_autosave_state() {
    rejected_save(None, "recursion limit exceeded").await;
}

#[tokio::test]
async fn injected_save_limits_preserve_save_save_as_and_autosave_state() {
    for limits in [
        SaveLimits {
            document: 1,
            ..Default::default()
        },
        SaveLimits {
            manifest: 1,
            ..Default::default()
        },
        SaveLimits {
            operations: 1,
            ..Default::default()
        },
        SaveLimits {
            total: 1,
            ..Default::default()
        },
        SaveLimits {
            entries: 1,
            ..Default::default()
        },
    ] {
        rejected_save(Some(limits), "cap exceeded").await;
    }
}

async fn rejected_save(limits: Option<SaveLimits>, expected_error: &str) {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = pending_copy(area.path(), false).await;
    let old = area.path().join("admitted.onecad");
    save_document(
        &state,
        host.clone(),
        Some(old.to_string_lossy().into_owned()),
        None,
    )
    .await
    .unwrap();
    let previous = std::fs::read(&old).unwrap();
    let target = area.path().join("save-as.onecad");
    std::fs::copy(&old, &target).unwrap();
    let (id, generation) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        if let Some(limits) = limits {
            rt.set_test_save_limits(limits);
            let operation = serde_json::from_value(serde_json::json!({
                "opType": "FutureSaveFixture", "params": {}
            }))
            .unwrap();
            rt.apply(onecad_core::edit::EditCommand::AddOperation {
                record: onecad_core::document::record::OperationRecord::new(
                    onecad_core::ids::RecordId::new(),
                    0,
                    "Future",
                    operation,
                ),
                at_cursor: true,
            })
            .unwrap();
        } else {
            let mut payload = serde_json::Value::Null;
            for _ in 0..130 {
                payload = serde_json::json!([payload]);
            }
            rt.apply(onecad_core::edit::EditCommand::SetModuleState {
                module: onecad_core::document::modules::ModuleId::parse("test.deep").unwrap(),
                state: Some(onecad_core::document::modules::ModuleState::new(1, payload)),
            })
            .unwrap();
        }
        (rt.document_uuid(), rt.persisted_generation())
    };
    let recovery = onecad_core::io::recovery::autosave_path(area.path(), id);
    std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
    std::fs::write(&recovery, &previous).unwrap();
    let marker = onecad_core::io::recovery::SessionMarker {
        document_id: id,
        pid: 1,
        opened_path: Some(old.clone()),
        runtime_session: Some(
            state
                .runtime
                .lock()
                .await
                .as_ref()
                .unwrap()
                .runtime_session(),
        ),
        persisted_generation: Some(generation),
        last_autosave: String::new(),
        title: None,
    };
    onecad_core::io::recovery::write_marker(area.path(), &marker).unwrap();
    let marker_path = onecad_core::io::recovery::marker_path(area.path(), id);
    let marker_bytes = std::fs::read(&marker_path).unwrap();
    for path in [None, Some(target.to_string_lossy().into_owned())] {
        let error = save_document(&state, host.clone(), path, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected_error), "{error}");
        assert_eq!(std::fs::read(&old).unwrap(), previous);
        assert_eq!(std::fs::read(&target).unwrap(), previous);
    }
    let event = crate::autosave::autosave_current_reporting(
        &state.runtime,
        area.path(),
        &state.persistence,
    )
    .await
    .unwrap();
    assert!(event.error.as_deref().unwrap().contains(expected_error));
    assert_eq!(std::fs::read(recovery).unwrap(), previous);
    assert_eq!(std::fs::read(marker_path).unwrap(), marker_bytes);
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    assert_eq!(rt.path(), Some(old.as_path()));
    assert_eq!(rt.persisted_generation(), generation);
    assert!(rt.is_dirty());
    rt.undo().unwrap();
    assert_eq!(
        rt.unsaved_changes().count,
        0,
        "refusal must preserve saved-history identity"
    );
    ContainerReader::open(&old).unwrap();
}
