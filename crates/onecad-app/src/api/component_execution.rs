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
mod joint_origin;
mod joint_revalidation;
pub use joint_origin::{
    component_joint_origin, get_joint_bindings, JointOriginRequest, JointOriginResolution,
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
    apply_assembly_edit_inner(state, expected_session, None, edit, None).await
}

/// Applies an edit only to the exact displayed persisted state.
pub async fn apply_assembly_edit_at_generation(
    state: &AppState,
    expected_session: &str,
    generation: u64,
    edit: onecad_core::edit::AssemblyEdit,
) -> Result<AssemblyProjectionDto, ApiError> {
    apply_assembly_edit_inner(state, expected_session, Some(generation), edit, None).await
}

/// Enforces host scene capacity atomically with the exact displayed edit.
pub async fn apply_assembly_edit_for_display(
    state: &AppState,
    expected_session: &str,
    generation: u64,
    edit: onecad_core::edit::AssemblyEdit,
    maximum_visible_bodies: usize,
) -> Result<AssemblyProjectionDto, ApiError> {
    apply_assembly_edit_inner(
        state,
        expected_session,
        Some(generation),
        edit,
        Some(maximum_visible_bodies),
    )
    .await
}

async fn apply_assembly_edit_inner(
    state: &AppState,
    expected_session: &str,
    expected_generation: Option<u64>,
    edit: onecad_core::edit::AssemblyEdit,
    maximum_visible_bodies: Option<usize>,
) -> Result<AssemblyProjectionDto, ApiError> {
    let (projection, changed) = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .filter(|runtime| runtime.matches_runtime_session(expected_session))
            .ok_or_else(|| {
                ApiError::NoDocument("assembly edit belongs to a stale runtime".into())
            })?;
        if expected_generation
            .is_some_and(|generation| runtime.persisted_generation() != generation)
        {
            return Err(ApiError::StalePreview(
                "assembly edit display generation is stale".into(),
            ));
        }
        if let Some(maximum) = maximum_visible_bodies {
            runtime.check_assembly_body_budget(&edit, maximum)?;
        }
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

/// Reads assembly metadata only after a caller-provided allocation bound passes.
pub async fn get_assembly_projection_bounded(
    state: &AppState,
    expected_session: &str,
    maximum_bytes: usize,
) -> Result<(AssemblyProjectionDto, usize), ApiError> {
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .filter(|rt| rt.matches_runtime_session(expected_session))
        .ok_or_else(|| {
            ApiError::NoDocument("assembly projection belongs to a stale runtime".into())
        })?;
    let charge = runtime.assembly_projection_charge(maximum_bytes)?;
    Ok((runtime.assembly_projection_bounded(maximum_bytes)?, charge))
}

/// Compact fence for a projection captured from a specific runtime.
pub async fn assembly_generation(
    state: &AppState,
    expected_session: &str,
) -> Result<u64, ApiError> {
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .filter(|rt| rt.matches_runtime_session(expected_session))
        .ok_or_else(|| {
            ApiError::NoDocument("assembly projection belongs to a stale runtime".into())
        })?;
    Ok(runtime.persisted_generation())
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
                false,
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
                false,
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

pub(crate) struct WorkerLease {
    pub(crate) worker: WorkerManager,
    permit: Option<OwnedSemaphorePermit>,
    pub(crate) epoch: onecad_core::ids::WorkerEpoch,
    slot: Arc<std::sync::Mutex<Option<(String, WorkerManager)>>>,
}

impl WorkerLease {
    async fn drain_on_error<T>(self, result: Result<T, ApiError>) -> Result<(Self, T), ApiError> {
        match result {
            Ok(value) => Ok((self, value)),
            Err(error) => {
                self.drain().await;
                Err(error)
            }
        }
    }

    pub(crate) async fn drain(mut self) {
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
    regenerate_component_inner(state, expected_session, id, None, None).await
}

/// Starts replay only from the exact displayed persisted generation.
pub async fn regenerate_component_at_generation(
    state: &AppState,
    expected_session: &str,
    id: ComponentDefId,
    generation: u64,
) -> Result<(u64, AssemblyProjectionDto), ApiError> {
    regenerate_component_inner(state, expected_session, id, Some(generation), None).await
}

/// Replays a definition and bounds projected metadata before and after commit.
pub async fn regenerate_component_bounded(
    state: &AppState,
    expected_session: &str,
    id: ComponentDefId,
    maximum_bytes: usize,
) -> Result<(u64, AssemblyProjectionDto), ApiError> {
    regenerate_component_inner(state, expected_session, id, None, Some(maximum_bytes)).await
}

async fn regenerate_component_inner(
    state: &AppState,
    expected_session: &str,
    id: ComponentDefId,
    expected_generation: Option<u64>,
    maximum_bytes: Option<usize>,
) -> Result<(u64, AssemblyProjectionDto), ApiError> {
    let input = {
        let guard = state.runtime.lock().await;
        if let Some(maximum) = maximum_bytes {
            guard
                .as_ref()
                .ok_or_else(|| ApiError::NoDocument("component replay".into()))?
                .assembly_projection_bounded(maximum)?;
        }
        if expected_generation.is_some_and(|generation| {
            guard
                .as_ref()
                .is_none_or(|runtime| runtime.persisted_generation() != generation)
        }) {
            return Err(ApiError::StalePreview(
                "component replay display generation is stale".into(),
            ));
        }

        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("component replay".into()))?
            .prepare_component_replay(id, expected_session)?
    };
    let lease = open_worker(state, &input).await?;
    let replay = input
        .clone()
        .drive(lease.worker.clone(), lease.epoch, true)
        .await;
    let (lease, mut result) = lease.drain_on_error(replay).await?;
    let lease = if result.minted {
        let signature = result.verification_signature();
        let (lease, signature) = lease.drain_on_error(signature).await?;
        // Retain only bounded signatures while the independent worker replays.
        result.meshes.clear();
        lease.drain().await;
        let mut verification = input;
        verification.definition = result.definition.clone();
        verification.translation_prefix = None;
        let lease = open_worker(state, &verification).await?;
        let replay = verification
            .drive(lease.worker.clone(), lease.epoch, false)
            .await;
        let (lease, replay) = lease.drain_on_error(replay).await?;
        let checked = replay.verification_signature().and_then(|actual| {
            if signature == actual {
                Ok(())
            } else {
                Err(ApiError::InvalidCommand(
                    "component rehydration geometry or identity mismatch".into(),
                ))
            }
        });
        let (lease, ()) = lease.drain_on_error(checked).await?;
        result.meshes = replay.meshes;
        result.budget = replay.budget;
        result.snapshot = replay.snapshot;
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
    let mut persisted_changed = false;
    let outcome: Result<(u64, AssemblyProjectionDto), ApiError> = {
        let mut guard = state.runtime.lock().await;
        (|| {
            let runtime = guard
                .as_mut()
                .ok_or_else(|| ApiError::NoDocument("component replay".into()))?;
            let before = runtime.persisted_generation();
            let generation =
                runtime.commit_component_replay(result, lease.worker.current_epoch())?;
            persisted_changed = runtime.persisted_generation() != before;
            let projection = match maximum_bytes {
                Some(maximum) => runtime.assembly_projection_bounded(maximum)?,
                None => runtime.assembly_projection()?,
            };
            Ok((generation, projection))
        })()
    };
    lease.drain().await;
    if persisted_changed {
        state.note_mutation();
    }
    let (generation, projection) = outcome?;
    #[cfg(test)]
    let hook = state.component_after_commit.lock().unwrap().take();
    #[cfg(test)]
    if let Some((started, release)) = hook {
        let _ = started.send(());
        let _ = release.await;
    }
    let validated = Box::pin(joint_revalidation::revalidate(
        state,
        expected_session,
        id,
        generation,
        &projection,
    ))
    .await?;
    let mut guard = state.runtime.lock().await;
    state.ensure_not_closing()?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("component validation".into()))?;
    runtime.finish_component_validation(
        expected_session,
        id,
        generation,
        validated.generation,
        &validated.head,
    )?;
    let projection = match maximum_bytes {
        Some(maximum) => runtime.assembly_projection_bounded(maximum)?,
        None => runtime.assembly_projection()?,
    };
    Ok((generation, projection))
}

pub(crate) async fn open_worker(
    state: &AppState,
    input: &crate::document_runtime::component_execution::ComponentReplayInput,
) -> Result<WorkerLease, ApiError> {
    open_worker_session(
        state,
        &input.runtime_session,
        input.revision,
        input.namespace,
    )
    .await
}

pub(crate) async fn open_worker_session(
    state: &AppState,
    session: &str,
    revision: onecad_core::ids::DocumentRevision,
    namespace: onecad_core::ids::DocumentId,
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
                runtime.matches_runtime_session(session) && runtime.revision() == revision
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
            document_id: namespace,
            document_revision: revision,
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
        || head.document_revision != revision
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
mod tests;
