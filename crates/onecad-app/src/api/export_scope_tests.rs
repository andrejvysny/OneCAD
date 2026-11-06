use super::*;
use onecad_core::{
    document::component::{ComponentDefId, Occurrence, OccurrenceId, RigidTransform},
    edit::AssemblyEdit,
};

#[tokio::test]
async fn root_only_export_refuses_to_omit_occurrences_and_preserves_existing_targets() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = recovery_identity_tests::pending_copy(area.path(), false).await;
    let session = get_projection(&state).await.unwrap().runtime_session;
    let definition = ComponentDefId::new();
    apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::CreateDefinition {
            id: definition,
            name: "Embedded part".into(),
        },
    )
    .await
    .unwrap();
    apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::InsertOccurrence {
            occurrence: Occurrence {
                id: OccurrenceId::new(),
                def: definition,
                parent: None,
                local: RigidTransform::IDENTITY,
                grounded: false,
                name: "Instance".into(),
            },
        },
    )
    .await
    .unwrap();
    let before = get_projection(&state).await.unwrap();
    for format in ["step", "stl", "obj", "3mf"] {
        let path = area.path().join(format!("protected.{format}"));
        std::fs::write(&path, b"existing target remains intact").unwrap();
        let path_string = Some(path.to_string_lossy().into_owned());
        let result = match format {
            "step" => export_step_file(&state, host.clone(), path_string).await,
            "stl" => export_stl_file(&state, host.clone(), path_string).await,
            "obj" => export_obj_file(&state, host.clone(), path_string).await,
            _ => export_3mf_file(&state, host.clone(), path_string).await,
        };
        if format == "step" {
            assert!(
                matches!(result, Err(ApiError::Worker(ref reason)) if reason.contains("worker not started")),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(ApiError::InvalidCommand(ref reason)) if reason.contains("omit occurrences")),
                "{format}: {result:?}"
            );
        }
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"existing target remains intact"
        );
    }
    let target = {
        let guard = state.runtime.lock().await;
        let runtime = guard.as_ref().unwrap();
        crate::extension_handoff::ExportTarget {
            document_id: runtime.document_id(),
            runtime_session: runtime.runtime_session(),
            revision: runtime.revision().0,
            snapshot_id: runtime.head_snapshot_id().unwrap_or(SnapshotId(0)).0,
            worker_epoch: runtime.fencing().get().1 .0,
            expected_base_hash: runtime.export_history_hash().as_str().into(),
        }
    };
    let extension_directory = area.path().join("extension-export");
    let result = crate::extension_handoff::export_3mf(
        &state,
        &target,
        &crate::extension_handoff::ExportScope::Document,
        "assembly",
        &extension_directory,
    )
    .await;
    assert!(
        matches!(result, Err(ApiError::InvalidCommand(ref reason)) if reason.contains("omit occurrences"))
    );
    assert!(!extension_directory.exists());
    assert_eq!(get_projection(&state).await.unwrap(), before);
}
