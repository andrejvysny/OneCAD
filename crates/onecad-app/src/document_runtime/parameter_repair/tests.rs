use super::*;
use onecad_core::document::record::DetachComponentParams;
use onecad_core::edit::AssemblyEdit;
use onecad_core::history::Timeline;

fn placed() -> PlaceComponentParams {
    serde_json::from_value(serde_json::json!({
        "componentId":"example.part", "componentVersion":"1.0.0",
        "componentRevision":format!("sha256:{}", "0".repeat(64)), "params":{},
        "source":{"kind":"generator", "generatorId":"iso4762", "generatorVersion":1, "params":{}},
        "placement":{"translate":[0,0,0]}
    }))
    .unwrap()
}

fn invalid_loaded() -> (DocumentRuntime, RecordId, PlaceComponentParams) {
    let params = placed();
    let record = OperationRecord::new(
        RecordId::new(),
        0,
        "Legacy",
        Operation::Known(KnownOperation::PlaceComponent(params.clone())),
    );
    let id = record.record_id;
    let mut document = Document::new(DocumentId::new());
    document.timeline = Timeline::from_records(vec![record]);
    let state = crate::state::AppState::default();
    let (engine, meshes, solver) = state.make_backend();
    let mut runtime = DocumentRuntime::from_document(
        document,
        "Legacy".into(),
        None,
        false,
        engine,
        meshes,
        solver,
    );
    runtime.repair_loaded_parameters().unwrap();
    assert_eq!(runtime.parameter_repair_gate(None), Some(0));
    (runtime, id, params)
}

#[tokio::test]
async fn copied_invalid_parameters_keep_repair_gate_in_new_definition() {
    let (mut runtime, _, _) = invalid_loaded();
    let definition = ComponentDefId::new();
    runtime
        .apply_assembly_edit(AssemblyEdit::CopyRoot {
            id: definition,
            name: "Copy".into(),
        })
        .unwrap();
    assert_eq!(runtime.parameter_repair_gate(Some(definition)), Some(0));
    assert!(runtime
        .prepare_component_replay(definition, &runtime.runtime_session())
        .is_err());
    let occurrence = onecad_core::document::component::OccurrenceId::new();
    runtime
        .apply_assembly_edit(AssemblyEdit::InsertOccurrence {
            occurrence: onecad_core::document::component::Occurrence {
                id: occurrence,
                def: definition,
                parent: None,
                local: onecad_core::document::component::RigidTransform::IDENTITY,
                grounded: false,
                name: "Copy".into(),
            },
        })
        .unwrap();
    let independent = ComponentDefId::new();
    runtime
        .apply_assembly_edit(AssemblyEdit::MakeIndependent {
            occurrence,
            new_definition: independent,
            name: "Independent".into(),
        })
        .unwrap();
    assert_eq!(runtime.parameter_repair_gate(Some(independent)), Some(0));
}

#[tokio::test]
async fn detached_invalid_source_keeps_repair_gate() {
    let (mut runtime, record, params) = invalid_loaded();
    runtime
        .apply(EditCommand::UpdateOperationParams {
            record,
            op: Operation::Known(KnownOperation::DetachComponent(DetachComponentParams {
                source: params.source,
                placement: params.placement,
                extra: Default::default(),
            })),
        })
        .unwrap();
    assert_eq!(runtime.parameter_repair_gate(None), Some(0));
}

#[tokio::test]
async fn changed_unverified_inputs_do_not_clear_repair_gate() {
    let (mut runtime, record, mut params) = invalid_loaded();
    if let ComponentSourceRef::Generator { params, .. } = &mut params.source {
        params.insert(
            "length".into(),
            onecad_core::document::record::ComponentParamValue::Number(50.0),
        );
    }
    runtime
        .apply(EditCommand::UpdateOperationParams {
            record,
            op: Operation::Known(KnownOperation::PlaceComponent(params)),
        })
        .unwrap();
    assert_eq!(runtime.parameter_repair_gate(None), Some(0));
}

#[tokio::test]
async fn embedded_generic_add_must_not_author_unauthenticated_components() {
    let state = crate::state::AppState::default();
    let (engine, meshes, solver) = state.make_backend();
    let mut runtime = DocumentRuntime::new_blank(engine, meshes, solver);
    let definition = ComponentDefId::new();
    runtime
        .apply_assembly_edit(AssemblyEdit::CreateDefinition {
            id: definition,
            name: "Part".into(),
        })
        .unwrap();
    let session = runtime.runtime_session();
    let before = runtime.persisted_generation();
    *state.runtime.lock().await = Some(runtime);
    let record = OperationRecord::new(
        RecordId::new(),
        0,
        "Forged",
        Operation::Known(KnownOperation::PlaceComponent(placed())),
    );
    let result = crate::api::apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::EditDefinition {
            id: definition,
            command: Box::new(EditCommand::AddOperation {
                record,
                at_cursor: true,
            }),
        },
    )
    .await;
    assert!(
        result.is_err(),
        "embedded component authoring must authenticate its package"
    );
    assert_eq!(
        state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .persisted_generation(),
        before
    );
}

#[tokio::test]
async fn embedded_generic_update_and_detach_refuse_before_history_changes() {
    for detach in [false, true] {
        let (mut runtime, record, mut params) = invalid_loaded();
        let definition = ComponentDefId::new();
        runtime
            .apply_assembly_edit(AssemblyEdit::CopyRoot {
                id: definition,
                name: "Copy".into(),
            })
            .unwrap();
        let session = runtime.runtime_session();
        let before = (runtime.persisted_generation(), runtime.undo_depth());
        let state = crate::state::AppState::default();
        *state.runtime.lock().await = Some(runtime);
        params.params.insert(
            "length".into(),
            onecad_core::document::record::ComponentParamValue::Number(50.0),
        );
        let operation = if detach {
            KnownOperation::DetachComponent(DetachComponentParams {
                source: params.source,
                placement: params.placement,
                extra: Default::default(),
            })
        } else {
            KnownOperation::PlaceComponent(params)
        };
        let edit = AssemblyEdit::EditDefinition {
            id: definition,
            command: Box::new(EditCommand::UpdateOperationParams {
                record,
                op: Operation::Known(operation),
            }),
        };
        assert!(crate::api::apply_assembly_edit(&state, &session, edit)
            .await
            .is_err());
        let runtime = state.runtime.lock().await;
        let runtime = runtime.as_ref().unwrap();
        assert_eq!(
            (runtime.persisted_generation(), runtime.undo_depth()),
            before
        );
    }
}
