use super::replay::hash_edges;
use super::*;
use onecad_core::edit::{AssemblyEdit, EditCommand};

fn runtime() -> DocumentRuntime {
    let backend = Arc::new(crate::worker::PendingBackend);
    DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend)
}

#[test]
fn replay_refuses_before_clone_when_host_admission_denies() {
    struct Deny;
    impl crate::resource_admission::ArchiveAdmission for Deny {
        fn reserve(
            &self,
            _: usize,
            _: crate::resource_admission::ArchiveUse,
        ) -> Result<crate::resource_admission::Retention, String> {
            Err("replay budget denied".into())
        }
    }
    let mut runtime = runtime();
    let id = ComponentDefId(Uuid::new_v4());
    runtime
        .apply_assembly_edit(AssemblyEdit::CreateDefinition {
            id,
            name: "Part".into(),
        })
        .unwrap();
    runtime.set_archive_budget(crate::resource_admission::ArchiveBudget::new(Arc::new(
        Deny,
    )));
    let error = match runtime.prepare_component_replay(id, &runtime.runtime_session()) {
        Ok(_) => panic!("replay should be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("replay budget denied"));
}

fn output(input: ComponentReplayInput, epoch: WorkerEpoch) -> ComponentReplayOutput {
    ComponentReplayOutput {
        id: input.id,
        runtime_session: input.runtime_session,
        revision: input.revision,
        epoch,
        source_fingerprint: input.source_fingerprint,
        definition: input.definition,
        meshes: BTreeMap::new(),
        budget: MeshBudget::default(),
        snapshot: SnapshotId(1),
        minted: false,
        _retention: input._retention,
    }
}

#[test]
fn prepare_replay_refuses_open_transaction_without_mutation() {
    let mut runtime = runtime();
    let id = ComponentDefId(Uuid::from_u128(0x501));
    runtime
        .apply_assembly_edit(AssemblyEdit::CreateDefinition {
            id,
            name: "Part".into(),
        })
        .unwrap();
    runtime.dirty = false;
    runtime.session.begin_transaction("caller");
    let pending = ComponentDefId(Uuid::from_u128(0x505));
    runtime
        .session
        .apply(EditCommand::Assembly {
            edit: Box::new(AssemblyEdit::CreateDefinition {
                id: pending,
                name: "Pending".into(),
            }),
        })
        .unwrap();
    let before = serde_json::to_value(runtime.session.document()).unwrap();
    let state = (
        runtime.revision(),
        runtime.persisted_generation(),
        runtime.undo_depth(),
        runtime.redo_depth(),
    );
    let error = match runtime.prepare_component_replay(id, &runtime.runtime_session()) {
        Ok(_) => panic!("replay preparation must refuse an open transaction"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("blocked by an open transaction"));
    assert!(runtime.session.transaction_open());
    assert_eq!(
        serde_json::to_value(runtime.session.document()).unwrap(),
        before
    );
    assert_eq!(
        (
            runtime.revision(),
            runtime.persisted_generation(),
            runtime.undo_depth(),
            runtime.redo_depth()
        ),
        state
    );
    assert!(!runtime.is_dirty());
    assert!(runtime.component_cache.is_empty());
    runtime.session.cancel_transaction();
    assert!(!runtime
        .session
        .document()
        .definitions
        .contains_key(&pending));
}

#[test]
fn commit_replay_refuses_transaction_opened_after_prepare_without_mutation() {
    let mut runtime = runtime();
    let id = ComponentDefId(Uuid::from_u128(0x502));
    runtime
        .apply_assembly_edit(AssemblyEdit::CreateDefinition {
            id,
            name: "Part".into(),
        })
        .unwrap();
    let input = runtime
        .prepare_component_replay(id, &runtime.runtime_session())
        .unwrap();
    runtime
        .commit_component_replay(output(input, WorkerEpoch(2)), WorkerEpoch(2))
        .unwrap();
    let input = runtime
        .prepare_component_replay(id, &runtime.runtime_session())
        .unwrap();
    let mut replay = output(input, WorkerEpoch(2));
    replay
        .definition
        .bodies
        .register(onecad_core::document::body::BodyMeta::new(
            BodyId(Uuid::from_u128(0x503)),
            "Replayed",
            RecordId(Uuid::from_u128(0x504)),
        ));
    runtime.dirty = false;
    runtime.session.begin_transaction("caller");
    let before = serde_json::to_value(runtime.session.document()).unwrap();
    let state = (
        runtime.revision(),
        runtime.persisted_generation(),
        runtime.undo_depth(),
        runtime.redo_depth(),
        runtime.component_generation,
    );
    let cache = &runtime.component_cache[&id];
    let cached = (cache.generation, cache.fingerprint.clone());
    let error = runtime
        .commit_component_replay(replay, WorkerEpoch(2))
        .unwrap_err();
    assert!(error.to_string().contains("blocked by an open transaction"));
    assert!(runtime.session.transaction_open());
    assert_eq!(
        serde_json::to_value(runtime.session.document()).unwrap(),
        before
    );
    assert_eq!(
        (
            runtime.revision(),
            runtime.persisted_generation(),
            runtime.undo_depth(),
            runtime.redo_depth(),
            runtime.component_generation
        ),
        state
    );
    assert_eq!(
        (
            runtime.component_cache[&id].generation,
            runtime.component_cache[&id].fingerprint.clone()
        ),
        cached
    );
    assert!(!runtime.is_dirty());
    runtime.session.cancel_transaction();
}

#[test]
fn replay_signature_detects_edge_partition_changes_with_equal_points() {
    let original = include_bytes!("../../../../../fixtures/render/edge-policy/box.mesh1");
    let mesh = MeshView::decode(original, MeshLimits::default()).unwrap();
    let offset = mesh.header().section(7).unwrap().offset as usize;
    let mut changed = original.to_vec();
    changed[offset + 4..offset + 8].copy_from_slice(&1u32.to_le_bytes());
    changed[offset + 8..offset + 12].copy_from_slice(&1u32.to_le_bytes());
    changed[offset + 12..offset + 16].copy_from_slice(&3u32.to_le_bytes());
    let changed = MeshView::decode(&changed, MeshLimits::default()).unwrap();
    assert!(mesh
        .edge_positions()
        .unwrap()
        .iter()
        .eq(changed.edge_positions().unwrap().iter()));
    assert_eq!(mesh.header().edge_count, changed.header().edge_count);
    let mut before = Sha256::new();
    let mut after = Sha256::new();
    hash_edges(&mut before, &mesh).unwrap();
    hash_edges(&mut after, &changed).unwrap();
    assert_ne!(before.finalize(), after.finalize());
}

#[test]
fn replay_dirty_state_tracks_persisted_changes_without_new_ids() {
    let mut runtime = runtime();
    let id = ComponentDefId(Uuid::from_u128(71));
    runtime
        .apply_assembly_edit(AssemblyEdit::CreateDefinition {
            id,
            name: "Part".into(),
        })
        .unwrap();
    runtime.dirty = false;
    let before = runtime.persisted_generation();
    let input = runtime
        .prepare_component_replay(id, &runtime.runtime_session())
        .unwrap();
    let mut replay = output(input, WorkerEpoch(2));
    replay
        .definition
        .bodies
        .register(onecad_core::document::body::BodyMeta::new(
            BodyId(Uuid::from_u128(72)),
            "Replayed",
            RecordId(Uuid::from_u128(73)),
        ));
    assert!(!replay.minted);
    runtime
        .commit_component_replay(replay, WorkerEpoch(2))
        .unwrap();
    assert!(runtime.is_dirty());
    assert_eq!(runtime.persisted_generation(), before + 1);
    runtime.dirty = false;
    let input = runtime
        .prepare_component_replay(id, &runtime.runtime_session())
        .unwrap();
    runtime
        .commit_component_replay(output(input, WorkerEpoch(2)), WorkerEpoch(2))
        .unwrap();
    assert!(!runtime.is_dirty());
    assert_eq!(runtime.persisted_generation(), before + 1);
}

#[test]
fn stale_worker_epoch_and_same_uuid_replacement_refuse_replay() {
    let mut original = runtime();
    let id = ComponentDefId(Uuid::from_u128(7));
    original
        .apply_assembly_edit(AssemblyEdit::CreateDefinition {
            id,
            name: "Part".into(),
        })
        .unwrap();
    let input = original
        .prepare_component_replay(id, &original.runtime_session())
        .unwrap();
    assert!(original
        .commit_component_replay(output(input, WorkerEpoch(2)), WorkerEpoch(3))
        .is_err());
    assert!(original.component_cache.is_empty());
    let input = original
        .prepare_component_replay(id, &original.runtime_session())
        .unwrap();
    let backend = Arc::new(crate::worker::PendingBackend);
    let mut replacement = DocumentRuntime::from_document(
        original.session.document().clone(),
        "Replacement".into(),
        None,
        false,
        backend.clone(),
        backend.clone(),
        backend,
    );
    assert_eq!(replacement.document_uuid(), original.document_uuid());
    assert_ne!(replacement.runtime_session(), original.runtime_session());
    assert!(replacement
        .commit_component_replay(output(input, WorkerEpoch(2)), WorkerEpoch(2))
        .is_err());
    assert!(replacement.component_cache.is_empty());
}

#[test]
fn verified_cache_becomes_pending_when_root_worker_epoch_changes() {
    let mut runtime = runtime();
    let id = ComponentDefId(Uuid::from_u128(0x445));
    runtime
        .apply_assembly_edit(AssemblyEdit::CreateDefinition {
            id,
            name: "Part".into(),
        })
        .unwrap();
    let session = runtime.runtime_session();
    let input = runtime.prepare_component_replay(id, &session).unwrap();
    let generation = runtime
        .commit_component_replay(output(input, WorkerEpoch(2)), WorkerEpoch(2))
        .unwrap();
    assert!(runtime.component_generation(id).is_none());
    let head = runtime.joint_head();
    runtime
        .finish_component_validation(
            &session,
            id,
            generation,
            runtime.persisted_generation(),
            &head,
        )
        .unwrap();
    assert_eq!(runtime.component_generation(id), Some(generation));
    runtime.fencing.set_epoch(runtime.fencing.get().1 .0 + 1);
    assert!(runtime.component_generation(id).is_none());
}

#[test]
fn root_publication_evicts_component_caches_before_new_scene() {
    let mut runtime = runtime();
    let id = ComponentDefId(Uuid::from_u128(123));
    runtime.component_cache.insert(
        id,
        ComponentCache {
            fingerprint: String::new(),
            generation: 1,
            verified_head: None,
            meshes: BTreeMap::new(),
            budget: MeshBudget {
                triangles: MAX_TRIANGLES,
                ..MeshBudget::default()
            },
        },
    );
    let snapshot = Arc::new(ModelSnapshot {
        id: SnapshotId(1),
        generation: 1,
        step_index: None,
        bodies: Vec::new(),
        stopped_reason: onecad_core::regen::StoppedReason::Completed,
        step_states: Vec::new(),
        signatures: None,
        diagnostics: Vec::new(),
        diagnostics_by_step: BTreeMap::new(),
        repair_summary: Default::default(),
        sketch_placement_by_step: BTreeMap::new(),
    });
    runtime.commit_snapshot(runtime.clone_regen_session(), &snapshot, Lod::Coarse, &[]);
    assert!(runtime.component_cache.is_empty());
}

#[test]
fn aggregate_budget_refusal_keeps_existing_component_cache() {
    let mut runtime = runtime();
    let existing = ComponentDefId(Uuid::from_u128(1));
    let incoming = ComponentDefId(Uuid::from_u128(2));
    runtime.component_cache.insert(
        existing,
        ComponentCache {
            fingerprint: String::new(),
            generation: 1,
            verified_head: None,
            meshes: BTreeMap::new(),
            budget: MeshBudget {
                triangles: 1,
                segments: 1,
                bodies: 1,
                bytes: 1,
            },
        },
    );
    assert!(runtime
        .check_component_budget(
            incoming,
            MeshBudget {
                triangles: MAX_TRIANGLES,
                segments: 0,
                bodies: 1,
                bytes: 1,
            }
        )
        .is_err());
    for over in [
        MeshBudget {
            segments: MAX_SEGMENTS,
            ..MeshBudget::default()
        },
        MeshBudget {
            bodies: MAX_BODIES,
            ..MeshBudget::default()
        },
        MeshBudget {
            bytes: MAX_RESIDENT_BYTES,
            ..MeshBudget::default()
        },
    ] {
        assert!(runtime.check_component_budget(incoming, over).is_err());
    }
    assert!(runtime.component_cache.contains_key(&existing));
}
