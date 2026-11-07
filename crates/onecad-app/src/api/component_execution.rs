//! One transient, bounded auxiliary worker for definition-local replay.
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use onecad_core::{
    document::component::ComponentDefId,
    regen::{GeometryEngine, HistoryPrefixHash, OpenSessionRequest, SessionMode},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    dto::AssemblyProjectionDto,
    error::ApiError,
    state::AppState,
    worker::{ElementQuery, WorkerManager, WorkerRole},
};

/// Applies one assembly edit to the identified live runtime.
///
/// # Errors
/// Rejects stale sessions and invalid edits without changing the document.
pub async fn apply_assembly_edit(
    state: &AppState,
    expected_session: &str,
    edit: onecad_core::edit::AssemblyEdit,
) -> Result<AssemblyProjectionDto, ApiError> {
    let (projection, changed) = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .filter(|runtime| runtime.matches_runtime_session(expected_session))
            .ok_or_else(|| {
                ApiError::NoDocument("assembly edit belongs to a stale runtime".into())
            })?;
        let before = runtime.persisted_generation();
        runtime.apply_assembly_edit(edit)?;
        (
            runtime.assembly_projection()?,
            runtime.persisted_generation() != before,
        )
    };
    if changed {
        state.note_mutation();
    }
    Ok(projection)
}

/// Reads the current assembly projection for the identified runtime.
pub async fn get_assembly_projection(
    state: &AppState,
    expected_session: &str,
) -> Result<AssemblyProjectionDto, ApiError> {
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .filter(|runtime| runtime.matches_runtime_session(expected_session))
        .ok_or_else(|| {
            ApiError::NoDocument("assembly projection belongs to a stale runtime".into())
        })?;
    runtime.assembly_projection()
}

/// Returns checked definition-local MESH1 bytes for an exact occurrence path.
///
/// # Errors
/// Rejects stale runtimes, paths, body identities, or mesh generations.
pub async fn component_mesh_for_path(
    state: &AppState,
    expected_session: &str,
    path: &onecad_core::document::component::OccurrencePath,
    body: onecad_core::ids::BodyId,
    generation: u64,
) -> Result<Arc<Vec<u8>>, ApiError> {
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .filter(|runtime| runtime.matches_runtime_session(expected_session))
        .ok_or_else(|| ApiError::NoDocument("component mesh belongs to a stale runtime".into()))?;
    runtime
        .component_mesh_for_path(path, body, generation)
        .ok_or_else(|| ApiError::StalePreview("component path or mesh generation is stale".into()))
}

/// Replays a definition to query a persisted element identity in its local frame.
/// Snapshot-local topology keys are deliberately omitted from the returned data.
///
/// # Errors
/// Rejects stale runtime, occurrence, mesh generation, or unavailable replay.
pub async fn component_element_info(
    state: &AppState,
    expected_session: &str,
    path: &onecad_core::document::component::OccurrencePath,
    body: onecad_core::ids::BodyId,
    generation: u64,
    element_id: &str,
) -> Result<Option<crate::dto::ElementInfoDto>, ApiError> {
    if !element_id.starts_with("el_") {
        return Err(ApiError::InvalidCommand(
            "component query requires a persistent element id".into(),
        ));
    }
    let input = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .filter(|runtime| runtime.matches_runtime_session(expected_session))
            .ok_or_else(|| {
                ApiError::NoDocument("component query belongs to a stale runtime".into())
            })?;
        let id = runtime
            .component_definition_for_path(path)
            .ok_or_else(|| ApiError::StalePreview("component path is stale".into()))?;
        if runtime
            .component_mesh_for_path(path, body, generation)
            .is_none()
        {
            return Err(ApiError::StalePreview(
                "component mesh generation is stale".into(),
            ));
        }
        runtime.prepare_component_replay(id, expected_session)?
    };
    let lease = open_worker(state, &input).await?;
    let output = input
        .drive(lease.worker.clone(), lease.epoch, false)
        .await?;
    let snapshot = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("component query".into()))?
            .validate_component_query(
                &output,
                lease.worker.current_epoch(),
                path,
                body,
                generation,
            )?
    };
    let query = lease
        .worker
        .query_element(snapshot, body, element_id)
        .await
        .map_err(|error| ApiError::Worker(format!("component element query: {error}")));
    let checked = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("component query".into()))?
            .validate_component_query(
                &output,
                lease.worker.current_epoch(),
                path,
                body,
                generation,
            )
    };
    let result = checked.and(query).and_then(|value| {
        value
            .map(|mut info| {
                if info.element_id != element_id
                    || crate::worker::wire::parse_body_id(&info.body_id).ok() != Some(body)
                {
                    return Err(ApiError::Worker(
                        "component element query returned mismatched identity".into(),
                    ));
                }
                info.topo_key.clear();
                Ok(info)
            })
            .transpose()
    });
    lease.drain().await;
    result
}

const READY_TIMEOUT: Duration = Duration::from_secs(8);
const REAP_TIMEOUT: Duration = Duration::from_secs(30);

fn execution_slot() -> Arc<Semaphore> {
    static SLOT: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Semaphore::new(1))).clone()
}

struct WorkerLease {
    worker: WorkerManager,
    permit: Option<OwnedSemaphorePermit>,
    epoch: onecad_core::ids::WorkerEpoch,
    slot: Arc<std::sync::Mutex<Option<(String, WorkerManager)>>>,
}

impl WorkerLease {
    async fn drain(mut self) {
        self.worker.retire();
        if self.worker.wait_torn_down(REAP_TIMEOUT).await {
            self.permit.take();
        }
    }
}

impl Drop for WorkerLease {
    fn drop(&mut self) {
        self.worker.retire();
        let mut slot = self.slot.lock().unwrap_or_else(|error| error.into_inner());
        if slot
            .as_ref()
            .is_some_and(|(_, current)| current.identity_token() == self.worker.identity_token())
        {
            slot.take();
        }
        drop(slot);
        let Some(permit) = self.permit.take() else {
            return;
        };
        let worker = self.worker.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                loop {
                    if worker.wait_torn_down(REAP_TIMEOUT).await {
                        drop(permit);
                        break;
                    }
                    tracing::warn!("component worker still retiring; retaining admission permit");
                }
            });
        } else {
            std::mem::forget(permit);
        }
    }
}

/// Replays one embedded definition on a checked transient worker and publishes
/// its local body metadata and meshes only if the root runtime remains current.
///
/// # Errors
/// Refuses stale sessions, unavailable worker capacity, failed replay, or resource limits.
pub async fn regenerate_component(
    state: &AppState,
    expected_session: &str,
    id: ComponentDefId,
) -> Result<(u64, AssemblyProjectionDto), ApiError> {
    let input = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("component replay".into()))?
            .prepare_component_replay(id, expected_session)?
    };
    let lease = open_worker(state, &input).await?;
    let mut result = input
        .clone()
        .drive(lease.worker.clone(), lease.epoch, true)
        .await?;
    let lease = if result.minted {
        lease.drain().await;
        let mut verification = input;
        verification.definition = result.definition.clone();
        let lease = open_worker(state, &verification).await?;
        let replay = verification
            .drive(lease.worker.clone(), lease.epoch, false)
            .await?;
        result.validate_rehydration(&replay)?;
        result.epoch = lease.epoch;
        lease
    } else {
        lease
    };
    #[cfg(test)]
    let hook = state.component_before_commit.lock().unwrap().take();
    #[cfg(test)]
    if let Some((started, release)) = hook {
        let _ = started.send(());
        let _ = release.await;
    }
    let result_minted = result.minted;
    let outcome = {
        let mut guard = state.runtime.lock().await;
        (|| {
            let runtime = guard
                .as_mut()
                .ok_or_else(|| ApiError::NoDocument("component replay".into()))?;
            let generation =
                runtime.commit_component_replay(result, lease.worker.current_epoch())?;
            Ok((generation, runtime.assembly_projection()?))
        })()
    };
    let minted = result_minted;
    lease.drain().await;
    if outcome.is_ok() && minted {
        state.note_mutation();
    }
    outcome
}

async fn open_worker(
    state: &AppState,
    input: &crate::document_runtime::component_execution::ComponentReplayInput,
) -> Result<WorkerLease, ApiError> {
    let permit = execution_slot()
        .try_acquire_owned()
        .map_err(|_| ApiError::InvalidCommand("component worker capacity is busy".into()))?;
    let worker = {
        let guard = state.runtime.lock().await;
        state.ensure_not_closing()?;
        let runtime = guard
            .as_ref()
            .filter(|runtime| {
                runtime.matches_runtime_session(&input.runtime_session)
                    && runtime.revision() == input.revision
            })
            .ok_or_else(|| {
                ApiError::StalePreview("component runtime changed before worker admission".into())
            })?;
        let mut slot = state
            .component_worker
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if slot.is_some() {
            return Err(ApiError::InvalidCommand(
                "component worker capacity is busy".into(),
            ));
        }
        let worker = state
            .services()
            .backend()
            .spawn(WorkerRole::Rebake)
            .ok_or_else(|| ApiError::Worker("component worker unavailable".into()))?;
        *slot = Some((runtime.runtime_session(), worker.clone()));
        worker
    };
    let mut lease = WorkerLease {
        worker,
        permit: Some(permit),
        epoch: onecad_core::ids::WorkerEpoch(0),
        slot: state.component_worker.clone(),
    };
    if !lease.worker.wait_ready(READY_TIMEOUT).await {
        return Err(ApiError::Worker(
            "component worker did not become ready".into(),
        ));
    }
    let epoch = lease.worker.current_epoch();
    lease
        .worker
        .open_session(OpenSessionRequest {
            document_id: input.namespace,
            document_revision: input.revision,
            worker_epoch: epoch,
            mode: SessionMode::Determinism,
        })
        .await
        .map_err(|error| ApiError::Worker(format!("component OpenSession: {error}")))?;
    let head = lease
        .worker
        .get_worker_head()
        .await
        .map_err(|error| ApiError::Worker(format!("component GetWorkerHead: {error}")))?;
    if head.worker_epoch != epoch
        || head.document_revision != input.revision
        || head.snapshot_id.0 != 0
        || head.history_prefix_hash != HistoryPrefixHash::empty()
        || head.has_scratch
        || head.has_restored_base
    {
        return Err(ApiError::Worker(
            "component worker returned a nonempty or stale session".into(),
        ));
    }
    lease.epoch = epoch;
    Ok(lease)
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::{
        document::{
            component::{ComponentDef, Occurrence, OccurrenceId, RigidTransform},
            Document,
        },
        ids::DocumentId,
        io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
    };
    use uuid::Uuid;

    #[tokio::test]
    #[ignore = "requires production worker; run with ONECAD_REQUIRE_WORKER=1 and ONECAD_WORKER_PATH"]
    async fn real_worker_replays_shared_definition_and_fresh_reopen() {
        let area = tempfile::tempdir().unwrap();
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/qa_plate_hole.onecad");
        let loaded = ContainerReader::open(&source).unwrap();
        let source = loaded.document();
        let id = ComponentDefId(Uuid::from_u128(0xD0));
        let mut def = ComponentDef::new(id, "Plate");
        def.variables = source.variables.clone();
        def.sketches = source.sketches.clone();
        def.sketch_visibility = source.sketch_visibility.clone();
        def.datum_planes = source.datum_planes.clone();
        def.timeline = source.timeline.clone();
        let mut doc = Document::new(DocumentId(Uuid::from_u128(0xD1)));
        doc.definitions.insert(id, def);
        for n in 0..2 {
            doc.assembly
                .occurrences
                .insert(Occurrence {
                    id: OccurrenceId(Uuid::from_u128(0xD2 + n)),
                    def: id,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: false,
                    name: format!("Plate {n}"),
                })
                .unwrap();
        }
        let path = area.path().join("assembly.onecad");
        let meta = SaveMeta {
            app_version: "test".into(),
            occt_fingerprint: None,
            created: String::new(),
            modified: String::new(),
        };
        ContainerWriter::save(&path, &doc, &ContainerCaches::none(), &meta).unwrap();
        let (state, host, _, _) =
            super::super::recovery_identity_tests::pending_copy(area.path(), false).await;
        let state = Arc::new(state);
        let opened = super::super::open_assembly_document(
            &state,
            host.clone(),
            path.to_string_lossy().into_owned(),
            None,
        )
        .await
        .unwrap();
        let session = opened.runtime_session;
        let (generation, projection) = regenerate_component(&state, &session, id).await.unwrap();
        assert!(state.runtime.lock().await.as_ref().unwrap().is_dirty());
        assert_eq!(projection.occurrences.len(), 2);
        let part = projection
            .definitions
            .iter()
            .find(|part| part.id == id.to_string())
            .unwrap();
        assert_eq!(part.mesh_generation, Some(generation));
        assert!(!part.bodies.is_empty());
        let first_body = part.bodies[0].id.clone();
        let body = first_body.parse::<onecad_core::ids::BodyId>().unwrap();
        let first_path = onecad_core::document::component::OccurrencePath(vec![OccurrenceId(
            Uuid::from_u128(0xD2),
        )]);
        let second_path = onecad_core::document::component::OccurrencePath(vec![OccurrenceId(
            Uuid::from_u128(0xD3),
        )]);
        let first_mesh = component_mesh_for_path(&state, &session, &first_path, body, generation)
            .await
            .unwrap();
        let second_mesh = component_mesh_for_path(&state, &session, &second_path, body, generation)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first_mesh, &second_mesh));
        let face_id = onecad_protocol::mesh::MeshView::decode(
            &first_mesh,
            onecad_protocol::mesh::MeshLimits::default(),
        )
        .unwrap()
        .face_id(0)
        .unwrap()
        .to_string();
        assert!(face_id.starts_with("el_"), "{face_id}");
        let info =
            component_element_info(&state, &session, &first_path, body, generation, &face_id)
                .await
                .unwrap()
                .expect("rehydrated face");
        assert_eq!(info.element_id, face_id);
        assert!(component_element_info(
            &state,
            &session,
            &first_path,
            body,
            generation,
            "el_missing"
        )
        .await
        .unwrap()
        .is_none());
        let invalid_path = onecad_core::document::component::OccurrencePath(vec![
            OccurrenceId(Uuid::from_u128(0xD2)),
            OccurrenceId(Uuid::from_u128(0xD3)),
        ]);
        assert!(
            component_mesh_for_path(&state, &session, &invalid_path, body, generation)
                .await
                .is_err()
        );
        let (started, ready) = tokio::sync::oneshot::channel();
        let (_release, resume) = tokio::sync::oneshot::channel();
        *state.component_before_commit.lock().unwrap() = Some((started, resume));
        let cancelled_state = state.clone();
        let cancelled_session = session.clone();
        let cancelled = tokio::spawn(async move {
            regenerate_component(&cancelled_state, &cancelled_session, id).await
        });
        ready.await.unwrap();
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Ok(permit) = execution_slot().try_acquire_owned() {
                    drop(permit);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled replay must retire its worker and release capacity");
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        *state.component_before_commit.lock().unwrap() = Some((started, resume));
        let replay_state = state.clone();
        let replay_session = session.clone();
        let replay =
            tokio::spawn(
                async move { regenerate_component(&replay_state, &replay_session, id).await },
            );
        ready.await.unwrap();
        assert!(matches!(regenerate_component(&state, &session, id).await,
            Err(ApiError::InvalidCommand(message)) if message.contains("capacity")));
        apply_assembly_edit(
            &state,
            &session,
            onecad_core::edit::AssemblyEdit::SetVisibility {
                occurrence: OccurrenceId(Uuid::from_u128(0xD3)),
                visible: false,
            },
        )
        .await
        .unwrap();
        release.send(()).unwrap();
        assert!(replay.await.unwrap().is_err());
        let bytes = {
            let mut guard = state.runtime.lock().await;
            let rt = guard.as_mut().unwrap();
            let body = first_body.parse::<onecad_core::ids::BodyId>().unwrap();
            rt.component_mesh(id, body, generation).unwrap()
        };
        assert!(!bytes.is_empty());
        let record = source
            .timeline
            .records()
            .iter()
            .rev()
            .find(|record| {
                matches!(
                    record.op,
                    onecad_core::document::record::Operation::Known(
                        onecad_core::document::record::KnownOperation::Extrude(_)
                    )
                )
            })
            .unwrap();
        let mut changed_op = record.op.clone();
        let onecad_core::document::record::Operation::Known(
            onecad_core::document::record::KnownOperation::Extrude(ref mut extrude),
        ) = changed_op
        else {
            unreachable!()
        };
        extrude.distance = onecad_core::document::variables::Scalar::new(9.0);
        let projection = apply_assembly_edit(
            &state,
            &session,
            onecad_core::edit::AssemblyEdit::EditDefinition {
                id,
                command: Box::new(onecad_core::edit::EditCommand::UpdateOperationParams {
                    record: record.record_id,
                    op: changed_op,
                }),
            },
        )
        .await
        .unwrap();
        assert!(projection
            .definitions
            .iter()
            .find(|def| def.id == id.to_string())
            .unwrap()
            .mesh_generation
            .is_none());
        assert!(
            component_mesh_for_path(&state, &session, &first_path, body, generation)
                .await
                .is_err()
        );
        let (edited_generation, _) = regenerate_component(&state, &session, id).await.unwrap();
        let edited_face = component_element_info(
            &state,
            &session,
            &first_path,
            body,
            edited_generation,
            &face_id,
        )
        .await
        .unwrap()
        .expect("edited face identity");
        assert_eq!(edited_face.element_id, face_id);
        let edited =
            component_mesh_for_path(&state, &session, &first_path, body, edited_generation)
                .await
                .unwrap();
        assert_ne!(edited.as_slice(), bytes.as_slice());
        let edited_mesh = onecad_protocol::mesh::MeshView::decode(
            &edited,
            onecad_protocol::mesh::MeshLimits::default(),
        )
        .unwrap();
        assert_eq!(edited_mesh.face_id(0), Some(face_id.as_str()));
        assert!(Arc::ptr_eq(
            &edited,
            &component_mesh_for_path(&state, &session, &second_path, body, edited_generation)
                .await
                .unwrap()
        ));
        super::super::undo(&state, host.clone()).await.unwrap();
        assert!(
            component_mesh_for_path(&state, &session, &first_path, body, edited_generation)
                .await
                .is_err()
        );
        super::super::redo(&state, host.clone()).await.unwrap();
        regenerate_component(&state, &session, id).await.unwrap();
        let independent = ComponentDefId(Uuid::from_u128(0xD4));
        apply_assembly_edit(
            &state,
            &session,
            onecad_core::edit::AssemblyEdit::MakeIndependent {
                occurrence: OccurrenceId(Uuid::from_u128(0xD3)),
                new_definition: independent,
                name: "Independent".into(),
            },
        )
        .await
        .unwrap();
        let (independent_generation, _) = regenerate_component(&state, &session, independent)
            .await
            .unwrap();
        let independent_mesh =
            component_mesh_for_path(&state, &session, &second_path, body, independent_generation)
                .await
                .unwrap();
        let mut changed_again = record.op.clone();
        let onecad_core::document::record::Operation::Known(
            onecad_core::document::record::KnownOperation::Extrude(ref mut extrude),
        ) = changed_again
        else {
            unreachable!()
        };
        extrude.distance = onecad_core::document::variables::Scalar::new(10.0);
        apply_assembly_edit(
            &state,
            &session,
            onecad_core::edit::AssemblyEdit::EditDefinition {
                id,
                command: Box::new(onecad_core::edit::EditCommand::UpdateOperationParams {
                    record: record.record_id,
                    op: changed_again,
                }),
            },
        )
        .await
        .unwrap();
        let (new_generation, _) = regenerate_component(&state, &session, id).await.unwrap();
        let changed_mesh =
            component_mesh_for_path(&state, &session, &first_path, body, new_generation)
                .await
                .unwrap();
        let preserved_mesh =
            component_mesh_for_path(&state, &session, &second_path, body, independent_generation)
                .await
                .unwrap();
        assert!(Arc::ptr_eq(&independent_mesh, &preserved_mesh));
        assert_ne!(changed_mesh.as_slice(), independent_mesh.as_slice());
        assert!(
            super::super::save_document(&state, host.clone(), None, None)
                .await
                .unwrap()
                .clean
        );
        super::super::close_document(&state, host.clone())
            .await
            .unwrap();
        let reopened = super::super::open_assembly_document(
            &state,
            host.clone(),
            path.to_string_lossy().into_owned(),
            None,
        )
        .await
        .unwrap();
        let next_session = reopened.runtime_session;
        let (reopened_generation, projection) = regenerate_component(&state, &next_session, id)
            .await
            .unwrap();
        let reopened_face = component_element_info(
            &state,
            &next_session,
            &first_path,
            body,
            reopened_generation,
            &face_id,
        )
        .await
        .unwrap()
        .expect("reopened face identity");
        assert_eq!(reopened_face.element_id, face_id);
        let part = projection
            .definitions
            .iter()
            .find(|part| part.id == id.to_string())
            .unwrap();
        assert!(part.bodies.iter().any(|body| body.id == first_body));
        let (_, projection) = regenerate_component(&state, &next_session, independent)
            .await
            .unwrap();
        assert!(projection
            .definitions
            .iter()
            .any(|part| part.id == independent.to_string()
                && part.bodies.iter().any(|body| body.id == first_body)));
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        *state.component_before_commit.lock().unwrap() = Some((started, resume));
        let replay_state = state.clone();
        let replay_session = next_session.clone();
        let replay =
            tokio::spawn(
                async move { regenerate_component(&replay_state, &replay_session, id).await },
            );
        ready.await.unwrap();
        super::super::close_document(&state, host.clone())
            .await
            .unwrap();
        assert!(state.runtime.lock().await.is_none());
        assert!(state.component_worker.lock().unwrap().is_none());
        release.send(()).unwrap();
        assert!(replay.await.unwrap().is_err());
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let Ok(permit) = execution_slot().try_acquire_owned() {
                    drop(permit);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("closed component worker must release capacity");
    }

    #[tokio::test]
    #[ignore = "requires production worker; run with ONECAD_REQUIRE_WORKER=1 and ONECAD_WORKER_PATH"]
    async fn real_worker_ten_definitions_twenty_occurrences_reopen_and_replay() {
        let area = tempfile::tempdir().unwrap();
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/qa_plate_hole.onecad");
        let source = ContainerReader::open(&fixture).unwrap();
        let source = source.document();
        let mut doc = Document::new(DocumentId(Uuid::from_u128(0xE00)));
        for index in 0..10u128 {
            let id = ComponentDefId(Uuid::from_u128(0xE10 + index));
            let mut def = ComponentDef::new(id, format!("Plate {index}"));
            def.variables = source.variables.clone();
            def.sketches = source.sketches.clone();
            def.sketch_visibility = source.sketch_visibility.clone();
            def.datum_planes = source.datum_planes.clone();
            def.timeline = source.timeline.clone();
            doc.definitions.insert(id, def);
            for copy in 0..2u128 {
                doc.assembly
                    .occurrences
                    .insert(Occurrence {
                        id: OccurrenceId(Uuid::from_u128(0xE80 + index * 2 + copy)),
                        def: id,
                        parent: None,
                        local: RigidTransform::IDENTITY,
                        grounded: false,
                        name: format!("Plate {index}.{copy}"),
                    })
                    .unwrap();
            }
        }
        let path = area.path().join("ten-definitions.onecad");
        let meta = SaveMeta {
            app_version: "test".into(),
            occt_fingerprint: None,
            created: String::new(),
            modified: String::new(),
        };
        ContainerWriter::save(&path, &doc, &ContainerCaches::none(), &meta).unwrap();
        let (state, host, _, _) =
            super::super::recovery_identity_tests::pending_copy(area.path(), false).await;
        let opened = super::super::open_assembly_document(
            &state,
            host.clone(),
            path.to_string_lossy().into_owned(),
            None,
        )
        .await
        .unwrap();
        let session = opened.runtime_session;
        for index in 0..10u128 {
            let id = ComponentDefId(Uuid::from_u128(0xE10 + index));
            let (_, projection) = regenerate_component(&state, &session, id).await.unwrap();
            assert!(projection
                .definitions
                .iter()
                .any(|def| def.id == id.to_string()
                    && def.mesh_generation.is_some()
                    && !def.bodies.is_empty()));
        }
        let projection = get_assembly_projection(&state, &session).await.unwrap();
        assert_eq!(projection.definitions.len(), 11);
        assert_eq!(projection.occurrences.len(), 20);
        assert!(
            super::super::save_document(&state, host.clone(), None, None)
                .await
                .unwrap()
                .clean
        );
        super::super::close_document(&state, host.clone())
            .await
            .unwrap();
        let reopened = super::super::open_assembly_document(
            &state,
            host,
            path.to_string_lossy().into_owned(),
            None,
        )
        .await
        .unwrap();
        for index in 0..10u128 {
            let id = ComponentDefId(Uuid::from_u128(0xE10 + index));
            let (_, projection) = regenerate_component(&state, &reopened.runtime_session, id)
                .await
                .unwrap();
            assert!(projection
                .definitions
                .iter()
                .any(|def| def.id == id.to_string() && def.mesh_generation.is_some()));
        }
    }
}
