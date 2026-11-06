use super::*;
use crate::{
    api::component_execution::WorkerLease,
    export::{GeometryExporter, StepExportAttributes},
    resource_admission::{ArchiveBudget, ArchiveUse},
    worker::{MeshProvider, SolverEngine, StepImport, WorkerManager},
};
use onecad_core::{
    document::{
        record::{
            ImportSourceCodec, ImportStepParams, KnownOperation, Operation, OperationRecord,
            TransformBodyParams, IMPORT_HEAL_POLICY_V1,
        },
        refs::Extra,
    },
    edit::EditCommand,
    ids::{DocumentId, RecordId},
    io::imports::OwnedSource,
    regen::{CancelToken, GeometryEngine, OpenSessionRequest, Outcome, RegenRequest, SessionMode},
};

const MAX_BODY_BYTES: u64 = 32 << 20;
const MAX_TOTAL_BYTES: u64 = 128 << 20;

struct Solid {
    sha: String,
    format: u32,
    bytes: Arc<OwnedSource>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    input: ExportInput,
    target: PathBuf,
    runtime: Arc<tokio::sync::Mutex<Option<DocumentRuntime>>>,
    root: Arc<dyn GeometryExporter>,
    budget: ArchiveBudget,
    lease: WorkerLease,
    mut namespace: DocumentId,
    canceled: Arc<AtomicU8>,
) -> Result<(), ApiError> {
    let result = run_inner(
        &input,
        &target,
        &runtime,
        root,
        &budget,
        &lease,
        &mut namespace,
        &canceled,
    )
    .await;
    lease.drain().await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_inner(
    input: &ExportInput,
    target: &Path,
    runtime: &Arc<tokio::sync::Mutex<Option<DocumentRuntime>>>,
    root: Arc<dyn GeometryExporter>,
    budget: &ArchiveBudget,
    lease: &WorkerLease,
    namespace: &mut DocumentId,
    canceled: &AtomicU8,
) -> Result<(), ApiError> {
    let temp = tempfile::tempdir().map_err(|e| ApiError::Io(e.to_string()))?;
    let mut total = 0;
    let mut root_solids = Vec::new();
    for body in &input.root_bodies {
        check_source(runtime, input, canceled).await?;
        root_solids.push(
            bake(
                root.as_ref(),
                *body,
                Some(input.snapshot),
                temp.path(),
                budget,
                &mut total,
            )
            .await?,
        );
    }
    let mut solids = BTreeMap::new();
    let mut definition_triangles = BTreeMap::new();
    for (id, replay) in &input.definitions {
        check_source(runtime, input, canceled).await?;
        if *namespace != replay.namespace {
            switch_session(
                &lease.worker,
                *namespace,
                replay.namespace,
                input.revision,
                lease.epoch,
            )
            .await?;
            *namespace = replay.namespace;
        }
        let output = replay
            .clone()
            .drive(lease.worker.clone(), lease.epoch, false)
            .await?;
        let actual: BTreeSet<_> = output.meshes.keys().copied().collect();
        if input.all_bodies[id] != actual {
            return Err(ApiError::InvalidCommand(
                "assembly STEP replay body set differs from accepted definition".into(),
            ));
        }
        let mut triangles = 0usize;
        for body in &input.visible[id] {
            let mesh = output
                .meshes
                .get(body)
                .ok_or_else(|| ApiError::InvalidCommand("assembly STEP mesh missing".into()))?;
            let view = onecad_protocol::mesh::MeshView::decode(mesh.as_slice(), Default::default())
                .map_err(|e| ApiError::InvalidCommand(format!("assembly STEP mesh: {e}")))?;
            triangles = triangles
                .checked_add(view.header().triangle_count as usize)
                .filter(|n| *n <= 500_000)
                .ok_or_else(|| {
                    ApiError::InvalidCommand("assembly STEP triangle limit exceeded".into())
                })?;
        }
        definition_triangles.insert(*id, triangles);
        let mut exported = Vec::new();
        for body in &input.visible[id] {
            exported.push(
                bake(
                    &lease.worker,
                    *body,
                    Some(output.snapshot),
                    temp.path(),
                    budget,
                    &mut total,
                )
                .await?,
            );
        }
        solids.insert(*id, exported);
    }
    let mut replay_bytes = root_solids
        .iter()
        .map(|s| s.bytes.len() as u64)
        .sum::<u64>();
    let mut triangles = input.root_triangles;
    for placement in &input.placements {
        triangles = triangles
            .checked_add(definition_triangles[&placement.definition])
            .filter(|n| *n <= 500_000)
            .ok_or_else(|| {
                ApiError::InvalidCommand("assembly STEP triangle limit exceeded".into())
            })?;
        for solid in &solids[&placement.definition] {
            replay_bytes = replay_bytes
                .checked_add(solid.bytes.len() as u64)
                .filter(|n| *n <= MAX_TOTAL_BYTES)
                .ok_or_else(|| {
                    ApiError::InvalidCommand(
                        "assembly STEP placed source byte limit exceeded".into(),
                    )
                })?;
        }
    }
    let worker = Arc::new(lease.worker.clone());
    reject_coincident(&root_solids, &solids, &input.placements)?;
    let mut scratch = DocumentRuntime::new_blank(
        worker.clone() as Arc<dyn GeometryEngine>,
        worker.clone() as Arc<dyn MeshProvider>,
        worker.clone() as Arc<dyn SolverEngine>,
    );
    let scratch_id = scratch.document_uuid();
    switch_session(
        &lease.worker,
        *namespace,
        scratch_id,
        scratch.revision(),
        lease.epoch,
    )
    .await?;
    *namespace = scratch_id;
    let mut expected = Vec::new();
    for solid in &root_solids {
        add_solid(&mut scratch, solid, RigidTransform::IDENTITY, &mut expected)?;
    }
    for placement in &input.placements {
        for solid in solids.get(&placement.definition).ok_or_else(|| {
            ApiError::InvalidCommand("assembly STEP definition unavailable".into())
        })? {
            add_solid(&mut scratch, solid, placement.pose, &mut expected)?;
        }
    }
    if expected.is_empty() || expected.len() > MAX_BODIES {
        return Err(ApiError::InvalidCommand(
            "assembly STEP has no visible solids or exceeds the body limit".into(),
        ));
    }
    let report = scratch
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    if !matches!(report.outcome, Outcome::Published(_))
        || !report.failed_steps.is_empty()
        || !report.needs_repair.is_empty()
    {
        return Err(ApiError::InvalidCommand(
            "assembly STEP scratch replay failed".into(),
        ));
    }
    let actual: BTreeSet<_> = scratch.head_body_ids().into_iter().collect();
    if actual != expected.iter().copied().collect() {
        return Err(ApiError::InvalidCommand(
            "assembly STEP scratch body multiplicity mismatch".into(),
        ));
    }
    let staged = tempfile::NamedTempFile::new_in(target_parent(target)?)
        .map_err(|e| ApiError::Io(e.to_string()))?;
    lease
        .worker
        .export_step(
            staged.path().to_string_lossy().as_ref(),
            &expected,
            "AP242DIS",
            &StepExportAttributes::default(),
            scratch.head_snapshot_id(),
        )
        .await?;
    let inspection = lease.worker.inspect_step(staged.path(), false).await?;
    if inspection.solid_count != expected.len() || inspection.source_unit != "MM" {
        return Err(ApiError::InvalidCommand(
            "assembly STEP round-trip count or units mismatch".into(),
        ));
    }
    staged
        .as_file()
        .sync_all()
        .map_err(|e| ApiError::Io(e.to_string()))?;
    #[cfg(test)]
    pause_before_publish(&input.session).await;
    let guard = runtime.lock().await;
    verify(
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("assembly export".into()))?,
        input,
    )?;
    if canceled
        .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(ApiError::Cancelled("assembly export canceled".into()));
    }
    #[cfg(test)]
    pause_hook(&COMMITTED_HOOK, &input.session).await;
    staged
        .persist(target)
        .map_err(|e| ApiError::Io(e.error.to_string()))?;
    std::fs::File::open(target_parent(target)?)
        .and_then(|f| f.sync_all())
        .map_err(|e| {
            ApiError::Io(format!(
                "STEP output was published, but directory durability could not be confirmed: {e}"
            ))
        })?;
    Ok(())
}

fn reject_coincident(
    root: &[Solid],
    solids: &BTreeMap<ComponentDefId, Vec<Solid>>,
    placements: &[Placement],
) -> Result<(), ApiError> {
    let mut seen: Vec<(&str, RigidTransform)> = root
        .iter()
        .map(|s| (s.sha.as_str(), RigidTransform::IDENTITY))
        .collect();
    for placement in placements {
        for solid in solids.get(&placement.definition).ok_or_else(|| {
            ApiError::InvalidCommand("assembly STEP definition unavailable".into())
        })? {
            if seen.iter().any(|(sha, pose)| {
                *sha == solid.sha && pose.approx_eq(&placement.pose, 1e-12, 1e-12)
            }) {
                return Err(ApiError::InvalidCommand(
                    "assembly STEP cannot reimport coincident identical solids deterministically"
                        .into(),
                ));
            }
            seen.push((&solid.sha, placement.pose));
        }
    }
    Ok(())
}

async fn check_source(
    runtime: &Arc<tokio::sync::Mutex<Option<DocumentRuntime>>>,
    input: &ExportInput,
    canceled: &AtomicU8,
) -> Result<(), ApiError> {
    if canceled.load(Ordering::Acquire) == 1 {
        return Err(ApiError::Cancelled("assembly export canceled".into()));
    }
    let guard = runtime.lock().await;
    verify(
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("assembly export".into()))?,
        input,
    )
}

async fn switch_session(
    worker: &WorkerManager,
    old: DocumentId,
    new: DocumentId,
    revision: DocumentRevision,
    epoch: WorkerEpoch,
) -> Result<(), ApiError> {
    worker
        .close_session(old, epoch)
        .await
        .map_err(|e| ApiError::Worker(e.to_string()))?;
    worker
        .open_session(OpenSessionRequest {
            document_id: new,
            document_revision: revision,
            worker_epoch: epoch,
            mode: SessionMode::Determinism,
        })
        .await
        .map_err(|e| ApiError::Worker(e.to_string()))?;
    let head = worker
        .get_worker_head()
        .await
        .map_err(|e| ApiError::Worker(e.to_string()))?;
    if head.worker_epoch != epoch
        || head.document_revision != revision
        || head.snapshot_id.0 != 0
        || head.history_prefix_hash != HistoryPrefixHash::empty()
        || head.has_scratch
        || head.has_restored_base
    {
        return Err(ApiError::Worker(
            "assembly STEP worker session was not empty".into(),
        ));
    }
    Ok(())
}

async fn bake(
    exporter: &dyn GeometryExporter,
    body: BodyId,
    snapshot: Option<SnapshotId>,
    dir: &Path,
    budget: &ArchiveBudget,
    total: &mut u64,
) -> Result<Solid, ApiError> {
    let path = dir.join(format!("{}.brep", uuid::Uuid::new_v4()));
    let result = exporter
        .export_geometry(
            path.to_string_lossy().as_ref(),
            &[body],
            "brep",
            false,
            snapshot,
        )
        .await?;
    if result.solid_count != 1 || result.codec != "brep" || result.bytes_written > MAX_BODY_BYTES {
        return Err(ApiError::InvalidCommand(
            "assembly STEP source is not one bounded solid".into(),
        ));
    }
    *total = total
        .checked_add(result.bytes_written)
        .filter(|n| *n <= MAX_TOTAL_BYTES)
        .ok_or_else(|| {
            ApiError::InvalidCommand("assembly STEP source byte limit exceeded".into())
        })?;
    let file = std::fs::File::open(&path).map_err(|e| ApiError::Io(e.to_string()))?;
    let size = file
        .metadata()
        .map_err(|e| ApiError::Io(e.to_string()))?
        .len();
    if size != result.bytes_written {
        return Err(ApiError::Io("assembly STEP source size changed".into()));
    }
    let guard = budget
        .reserve(size as usize, ArchiveUse::Scratch)
        .map_err(ApiError::Io)?;
    let mut file = file;
    let mut bytes = vec![0; size as usize].into_boxed_slice();
    std::io::Read::read_exact(&mut file, &mut bytes).map_err(|e| ApiError::Io(e.to_string()))?;
    if std::io::Read::read(&mut file, &mut [0; 1]).map_err(|e| ApiError::Io(e.to_string()))? != 0 {
        return Err(ApiError::Io("assembly STEP source grew".into()));
    }
    let bytes = bytes.into_vec();
    Ok(Solid {
        sha: crate::imports::sha256_hex(&bytes),
        format: result.format,
        bytes: Arc::new(OwnedSource::tracked(bytes, guard)),
    })
}

fn add_solid(
    runtime: &mut DocumentRuntime,
    solid: &Solid,
    pose: RigidTransform,
    ids: &mut Vec<BodyId>,
) -> Result<(), ApiError> {
    runtime.stage_component_blob_shared(
        &solid.sha,
        ImportSourceCodec::Brep,
        solid.bytes.clone(),
    )?;
    let id = RecordId::new();
    let body = BodyId(id.0);
    let import = Operation::Known(KnownOperation::ImportStep(ImportStepParams {
        source_sha256: solid.sha.clone(),
        source_codec: ImportSourceCodec::Brep,
        source_name: "assembly-export.brep".into(),
        heal_policy: IMPORT_HEAL_POLICY_V1.into(),
        unit_scale: Scalar::new(1.0),
        brep_format: Some(solid.format),
        provenance_sha256: None,
        extra: Extra::new(),
    }));
    runtime.apply(EditCommand::AddOperation {
        record: OperationRecord::new(id, 0, "Export source", import),
        at_cursor: false,
    })?;
    let rotation =
        placement_rotation(pose).map_err(|message| ApiError::InvalidCommand(message.into()))?;
    let transform = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
        targets: vec![body],
        translate: [
            Scalar::new(pose.translation.x),
            Scalar::new(pose.translation.y),
            Scalar::new(pose.translation.z),
        ],
        rotate: rotation,
        copy: false,
        extra: Extra::new(),
    }));
    runtime.apply(EditCommand::AddOperation {
        record: OperationRecord::new(RecordId::new(), 0, "Export placement", transform),
        at_cursor: false,
    })?;
    ids.push(body);
    Ok(())
}
