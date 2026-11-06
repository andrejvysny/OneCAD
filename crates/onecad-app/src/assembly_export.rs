//! Bounded flattened STEP export of visible assembly occurrences.
use onecad_core::{
    document::{
        component::{ComponentDefId, RigidTransform},
        record::TransformRotation,
        variables::Scalar,
    },
    ids::{BodyId, DocumentRevision, SnapshotId, WorkerEpoch},
    math::Vec3,
    regen::HistoryPrefixHash,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

use crate::{
    document_runtime::{component_execution::ComponentReplayInput, DocumentRuntime},
    error::ApiError,
    state::AppState,
};
#[cfg(test)]
mod barrier_tests;
mod worker;

#[cfg(test)]
type PublishHook = (
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);
#[cfg(test)]
static PUBLISH_HOOK: std::sync::Mutex<Option<PublishHook>> = std::sync::Mutex::new(None);
#[cfg(test)]
static COMMITTED_HOOK: std::sync::Mutex<Option<PublishHook>> = std::sync::Mutex::new(None);

#[cfg(test)]
async fn pause_before_publish(session: &str) {
    pause_hook(&PUBLISH_HOOK, session).await;
}

#[cfg(test)]
async fn pause_hook(slot: &std::sync::Mutex<Option<PublishHook>>, session: &str) {
    let hook = {
        let mut guard = slot.lock().unwrap();
        if guard.as_ref().is_some_and(|h| h.0 == session) {
            guard.take()
        } else {
            None
        }
    };
    if let Some((_, started, release)) = hook {
        let _ = started.send(());
        let _ = release.await;
    }
}

const MAX_OCCURRENCES: usize = 20;
const MAX_BODIES: usize = 256;

struct Placement {
    definition: ComponentDefId,
    pose: RigidTransform,
}

struct ExportInput {
    session: String,
    generation: u64,
    revision: DocumentRevision,
    epoch: WorkerEpoch,
    snapshot: SnapshotId,
    base: HistoryPrefixHash,
    root_bodies: Vec<BodyId>,
    root_triangles: usize,
    definitions: BTreeMap<ComponentDefId, ComponentReplayInput>,
    visible: BTreeMap<ComponentDefId, BTreeSet<BodyId>>,
    all_bodies: BTreeMap<ComponentDefId, BTreeSet<BodyId>>,
    placements: Vec<Placement>,
}

/// Exports flattened, posed millimetre solids; does not preserve assembly structure.
pub(crate) async fn export(state: &AppState, target: PathBuf) -> Result<(), ApiError> {
    let context = crate::read::preview_context(state).await?;
    let mut input = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("assembly export".into()))?;
        let input = capture(runtime)?;
        if context.runtime_session != input.session
            || context.revision != input.revision
            || context.worker_epoch != input.epoch
            || context.snapshot_id != input.snapshot
            || context.expected_base_hash != input.base
        {
            return Err(ApiError::StalePreview(
                "assembly export head changed".into(),
            ));
        }
        input
    };
    for body in &input.root_bodies {
        let mesh = crate::api::get_mesh_shared_bounded(
            state,
            body.to_string(),
            "fine".into(),
            None,
            Some(input.session.clone()),
            32 << 20,
        )
        .await?
        .ok_or_else(|| ApiError::InvalidCommand("assembly STEP root mesh unavailable".into()))?;
        let view = onecad_protocol::mesh::MeshView::decode(mesh.as_slice(), Default::default())
            .map_err(|e| ApiError::InvalidCommand(format!("assembly STEP root mesh: {e}")))?;
        input.root_triangles = input
            .root_triangles
            .checked_add(view.header().triangle_count as usize)
            .filter(|n| *n <= 500_000)
            .ok_or_else(|| {
                ApiError::InvalidCommand("assembly STEP triangle limit exceeded".into())
            })?;
    }
    let namespace = input
        .definitions
        .values()
        .next()
        .map_or_else(onecad_core::ids::DocumentId::new, |input| input.namespace);
    let lease = crate::api::component_execution::open_worker_session(
        state,
        &input.session,
        input.revision,
        namespace,
    )
    .await?;
    let runtime = state.runtime.clone();
    let exporter = state.exporter();
    let budget = state.archive_budget();
    let canceled = Arc::new(AtomicU8::new(0));
    let mut cancel_on_drop = CancelOnDrop(Some(canceled.clone()));
    let result = tokio::spawn(async move {
        worker::run(
            input, target, runtime, exporter, budget, lease, namespace, canceled,
        )
        .await
    })
    .await
    .map_err(|error| ApiError::Internal(format!("assembly export task stopped: {error}")))?;
    cancel_on_drop.0 = None;
    result
}

struct CancelOnDrop(Option<Arc<AtomicU8>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(flag) = self.0.take() {
            let _ = flag.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
        }
    }
}

fn verify(runtime: &DocumentRuntime, input: &ExportInput) -> Result<(), ApiError> {
    if !runtime.matches_runtime_session(&input.session)
        || runtime.persisted_generation() != input.generation
        || runtime.revision() != input.revision
        || runtime.fencing().get().1 != input.epoch
        || runtime.head_snapshot_id().unwrap_or(SnapshotId(0)) != input.snapshot
        || runtime.export_history_hash() != input.base
    {
        return Err(ApiError::StalePreview(
            "assembly export source changed".into(),
        ));
    }
    Ok(())
}

fn target_parent(path: &Path) -> Result<&Path, ApiError> {
    if path.file_name().is_none() {
        return Err(ApiError::InvalidCommand(
            "assembly STEP target needs a filename".into(),
        ));
    }
    Ok(path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(".")))
}

fn capture(runtime: &DocumentRuntime) -> Result<ExportInput, ApiError> {
    let projection = runtime.assembly_projection_bounded(2 << 20)?;
    let session = runtime.runtime_session();
    let snapshot = runtime.head_snapshot_id().unwrap_or(SnapshotId(0));
    let mut definitions = BTreeMap::new();
    let mut visible = BTreeMap::new();
    let mut all_bodies = BTreeMap::new();
    let mut placements = Vec::new();
    let mut count = 0usize;
    for occurrence in projection.occurrences.iter().filter(|o| o.visible) {
        if placements.len() == MAX_OCCURRENCES || occurrence.path.len() != 1 {
            return Err(ApiError::InvalidCommand(
                "assembly STEP occurrence limit or nesting exceeded".into(),
            ));
        }
        if !occurrence.local.is_rigid() {
            return Err(ApiError::InvalidCommand(
                "assembly STEP placement is not rigid".into(),
            ));
        }
        let id = ComponentDefId(
            uuid::Uuid::parse_str(&occurrence.definition)
                .map_err(|e| ApiError::InvalidCommand(e.to_string()))?,
        );
        if let std::collections::btree_map::Entry::Vacant(entry) = definitions.entry(id) {
            entry.insert(runtime.prepare_component_replay(id, &session)?);
            let definition = projection
                .definitions
                .iter()
                .find(|d| d.id == occurrence.definition)
                .ok_or_else(|| {
                    ApiError::InvalidCommand("assembly STEP definition missing".into())
                })?;
            let bodies = definition
                .bodies
                .iter()
                .filter(|b| b.visible)
                .map(|b| b.id.parse())
                .collect::<Result<BTreeSet<BodyId>, _>>()
                .map_err(|e| ApiError::InvalidCommand(format!("assembly body id: {e}")))?;
            visible.insert(id, bodies);
            let all = definition
                .bodies
                .iter()
                .map(|b| b.id.parse())
                .collect::<Result<BTreeSet<BodyId>, _>>()
                .map_err(|e| ApiError::InvalidCommand(format!("assembly body id: {e}")))?;
            all_bodies.insert(id, all);
        }
        count = count
            .checked_add(visible[&id].len())
            .ok_or_else(|| ApiError::InvalidCommand("assembly STEP body count overflow".into()))?;
        require_visible_bodies(&occurrence.id, &visible[&id])?;
        placements.push(Placement {
            definition: id,
            pose: occurrence.local,
        });
    }
    let root = projection
        .definitions
        .iter()
        .find(|d| d.id == projection.root_definition)
        .ok_or_else(|| ApiError::InvalidCommand("assembly STEP root missing".into()))?;
    let root_bodies: Vec<BodyId> = root
        .bodies
        .iter()
        .filter(|b| b.visible)
        .map(|b| b.id.parse())
        .collect::<Result<_, _>>()
        .map_err(|e| ApiError::InvalidCommand(format!("assembly root body id: {e}")))?;
    if !root_bodies.is_empty() && snapshot == SnapshotId(0) {
        return Err(ApiError::InvalidCommand(
            "assembly STEP root geometry is not published".into(),
        ));
    }
    if count + root_bodies.len() > MAX_BODIES {
        return Err(ApiError::InvalidCommand(
            "assembly STEP body limit exceeded".into(),
        ));
    }
    Ok(ExportInput {
        session,
        generation: runtime.persisted_generation(),
        revision: runtime.revision(),
        epoch: runtime.fencing().get().1,
        snapshot,
        base: runtime.export_history_hash(),
        root_bodies,
        root_triangles: 0,
        definitions,
        visible,
        all_bodies,
        placements,
    })
}

fn require_visible_bodies(occurrence: &str, bodies: &BTreeSet<BodyId>) -> Result<(), ApiError> {
    if bodies.is_empty() {
        return Err(ApiError::InvalidCommand(format!(
            "assembly STEP visible occurrence {occurrence} has no visible bodies"
        )));
    }
    Ok(())
}

fn placement_rotation(pose: RigidTransform) -> Result<TransformRotation, &'static str> {
    if !pose.is_rigid() {
        return Err("assembly export requires a finite rigid placement");
    }
    let [mut w, mut x, mut y, mut z] = pose.rotation.components();
    if w < 0.0 {
        w = -w;
        x = -x;
        y = -y;
        z = -z;
    }
    let norm = x.hypot(y).hypot(z);
    let (axis, angle) = if norm < 1e-15 {
        ([0.0, 0.0, 1.0], 0.0)
    } else {
        (
            [x / norm, y / norm, z / norm],
            (2.0 * norm.atan2(w)).to_degrees(),
        )
    };
    Ok(TransformRotation {
        center: Vec3::new_unchecked(0.0, 0.0, 0.0),
        axis: Vec3::new_unchecked(axis[0], axis[1], axis[2]),
        angle_deg: Scalar::new(angle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::document::component::{Quat, Vec3d};

    #[test]
    fn mixed_populated_and_empty_occurrences_refuse_empty_by_identity() {
        let populated = BTreeSet::from([BodyId::new()]);
        assert!(require_visible_bodies("populated", &populated).is_ok());
        let error = require_visible_bodies("empty-instance", &BTreeSet::new()).unwrap_err();
        assert!(error.to_string().contains("empty-instance"));
    }

    #[test]
    fn quaternion_half_turn_keeps_antiparallel_axis_and_translation() {
        let rotation =
            Quat::from_axis_angle(Vec3d::new(-1.0, 0.0, 0.0), std::f64::consts::PI).unwrap();
        let pose = RigidTransform::new(rotation, Vec3d::new(5.0, -2.0, 3.0));
        let result = placement_rotation(pose).unwrap();
        assert!((result.angle_deg.value - 180.0).abs() < 1e-10);
        assert!((result.axis.x + 1.0).abs() < 1e-12);
        assert_eq!(result.center, Vec3::new_unchecked(0.0, 0.0, 0.0));
        assert_eq!(pose.translation, Vec3d::new(5.0, -2.0, 3.0));
    }

    #[test]
    fn quaternion_sign_and_identity_have_same_rotation() {
        let positive = RigidTransform::new(Quat::try_new(0.0, 0.0, 1.0, 0.0).unwrap(), Vec3d::ZERO);
        let negative =
            RigidTransform::new(Quat::try_new(0.0, 0.0, -1.0, 0.0).unwrap(), Vec3d::ZERO);
        let a = placement_rotation(positive).unwrap();
        let b = placement_rotation(negative).unwrap();
        assert!((a.angle_deg.value - 180.0).abs() < 1e-10);
        assert!((b.angle_deg.value - 180.0).abs() < 1e-10);
        assert_eq!(
            placement_rotation(RigidTransform::IDENTITY)
                .unwrap()
                .angle_deg
                .value,
            0.0
        );
        assert!(placement_rotation(RigidTransform::new(
            Quat::IDENTITY,
            Vec3d::new(f64::NAN, 0.0, 0.0)
        ))
        .is_err());
    }
}
