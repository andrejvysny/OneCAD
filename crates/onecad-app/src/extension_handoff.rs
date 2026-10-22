//! Host-owned export and desktop application handoff for approved extension requests.
//!
//! Guests supply neither filesystem destinations nor process arguments. The host
//! captures the document target and supplies its own retained export directory,
//! application descriptor and executable settings after capability approval.

mod application;
pub use application::*;

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use onecad_core::ids::{BodyId, SnapshotId};

use crate::{error::ApiError, read::PreviewContext, state::AppState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportTarget {
    pub document_id: String,
    pub runtime_session: String,
    pub revision: u64,
    pub snapshot_id: u64,
    pub worker_epoch: u64,
    pub expected_base_hash: String,
}

impl From<&PreviewContext> for ExportTarget {
    fn from(context: &PreviewContext) -> Self {
        Self {
            document_id: context.document_id.clone(),
            runtime_session: context.runtime_session.clone(),
            revision: context.revision.0,
            snapshot_id: context.snapshot_id.0,
            worker_epoch: context.worker_epoch.0,
            expected_base_hash: context.expected_base_hash.as_str().into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportScope {
    Document,
    Selection(Vec<BodyId>),
}

/// A completed export. It is deliberately retained after an application fails to
/// launch, so the user can open the same file manually or retry the handoff.
#[derive(Debug, Clone)]
pub struct ExportArtifact {
    path: PathBuf,
    context: PreviewContext,
    pub target: ExportTarget,
    pub body_count: usize,
}

impl ExportArtifact {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Exports captured bodies with the same fine-LOD, appearance-aware pipeline as
/// `api::export_3mf_file`. No document mutations or locks span worker I/O.
pub async fn export_3mf(
    state: &AppState,
    target: &ExportTarget,
    scope: &ExportScope,
    suggested_name: &str,
    directory: &Path,
) -> Result<ExportArtifact, ApiError> {
    let (bodies, meshes, mut attributes, pending, query) = {
        let guard = state.runtime.lock().await;
        let rt = guard.as_ref().ok_or_else(no_document)?;
        validate_target(rt, target)?;
        let bodies = select_bodies(rt.head_body_ids(), scope)?;
        let (attributes, pending) = crate::export::pending_step_attributes(rt);
        (
            bodies,
            rt.meshes_arc(),
            attributes,
            pending,
            state.element_query(),
        )
    };
    let context = checked_context(state, target).await?;
    let raw_colors = crate::export_threemf::raw_face_colors_by_body(&pending);
    let snapshot = Some(SnapshotId(target.snapshot_id));
    crate::export::resolve_face_colors(&mut attributes, pending, snapshot, query.as_ref()).await;
    let models = crate::export_threemf::build_bodies(
        &bodies,
        meshes.as_ref(),
        snapshot,
        &attributes,
        &raw_colors,
    )
    .await?;
    let bytes = tokio::task::spawn_blocking(move || onecad_core::io::threemf::write_3mf(&models))
        .await
        .map_err(|e| ApiError::Io(format!("3MF encoding task failed: {e}")))??;
    if checked_context(state, target).await? != context {
        return Err(stale_target());
    }
    let directory = directory.to_path_buf();
    let name = safe_name(suggested_name);
    let path = tokio::task::spawn_blocking(move || retain_export(&directory, &name, &bytes))
        .await
        .map_err(|e| ApiError::Io(format!("3MF write task failed: {e}")))??;
    let artifact = ExportArtifact {
        path,
        context,
        target: target.clone(),
        body_count: bodies.len(),
    };
    validate_artifact_current(state, &artifact).await?;
    Ok(artifact)
}

fn validate_target(
    rt: &crate::document_runtime::DocumentRuntime,
    target: &ExportTarget,
) -> Result<(), ApiError> {
    if rt.document_id() != target.document_id
        || !rt.matches_runtime_session(&target.runtime_session)
        || rt.revision().0 != target.revision
        || rt.head_snapshot_id().unwrap_or(SnapshotId(0)).0 != target.snapshot_id
        || rt.fencing().get().1 .0 != target.worker_epoch
    {
        return Err(stale_target());
    }
    if rt.export_history_hash().as_str() != target.expected_base_hash {
        return Err(ApiError::InvalidCommand(
            "document geometry has pending regeneration; retry after regeneration completes".into(),
        ));
    }
    Ok(())
}

async fn checked_context(
    state: &AppState,
    target: &ExportTarget,
) -> Result<PreviewContext, ApiError> {
    let context = crate::read::preview_context(state).await?;
    if context.document_id != target.document_id
        || context.runtime_session != target.runtime_session
        || context.revision.0 != target.revision
        || context.snapshot_id.0 != target.snapshot_id
        || target.snapshot_id == 0
        || context.worker_epoch.0 != target.worker_epoch
        || context.expected_base_hash.as_str() != target.expected_base_hash
    {
        return Err(stale_target());
    }
    let guard = state.runtime.lock().await;
    let runtime = guard.as_ref().ok_or_else(no_document)?;
    validate_target(runtime, target)?;
    Ok(context)
}

/// Rechecks the captured head after a retained-file write or before a retry.
pub async fn validate_artifact_current(
    state: &AppState,
    artifact: &ExportArtifact,
) -> Result<(), ApiError> {
    let guard = state.runtime.lock().await;
    validate_artifact_runtime(guard.as_ref().ok_or_else(no_document)?, artifact)
}

fn validate_artifact_runtime(
    runtime: &crate::document_runtime::DocumentRuntime,
    artifact: &ExportArtifact,
) -> Result<(), ApiError> {
    if artifact.target != ExportTarget::from(&artifact.context) {
        return Err(stale_target());
    }
    validate_target(runtime, &artifact.target)?;
    if runtime.fencing().get().1 != artifact.context.worker_epoch
        || runtime.export_history_hash() != artifact.context.expected_base_hash
    {
        return Err(stale_target());
    }
    Ok(())
}

/// Admits the handoff under the document lock, then launches without another
/// await. The caller's document service lane serializes authoring with admission;
/// the lock is released before process handling or LaunchServices acceptance.
/// A stale artifact remains available on disk.
pub async fn launch_current_artifact(
    state: &AppState,
    descriptor: &ExternalApplicationDescriptor,
    settings: &ExternalApplicationSettings,
    artifact: &ExportArtifact,
    launcher: &dyn ApplicationLauncher,
) -> LaunchOutcome {
    let result = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(no_document)
            .and_then(|runtime| validate_artifact_runtime(runtime, artifact))
    };
    if let Err(error) = result {
        return LaunchOutcome::Failed(error.to_string());
    }
    launch_artifact(descriptor, settings, artifact, launcher)
}

fn select_bodies(head: Vec<BodyId>, scope: &ExportScope) -> Result<Vec<BodyId>, ApiError> {
    let selected = match scope {
        ExportScope::Document => head,
        ExportScope::Selection(selected) => {
            if selected.iter().any(|body| !head.contains(body)) {
                return Err(ApiError::InvalidCommand(
                    "export selection contains a stale body".into(),
                ));
            }
            // Preserve document ordering and deduplicate selections.
            head.into_iter()
                .filter(|body| selected.contains(body))
                .collect()
        }
    };
    if selected.is_empty() {
        return Err(ApiError::InvalidCommand(
            "there are no bodies to export".into(),
        ));
    }
    Ok(selected)
}

fn safe_name(name: &str) -> String {
    let stem: String = name
        .trim_end_matches(".3mf")
        .chars()
        .take(80)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if stem.trim().is_empty() {
        "OneCAD export".into()
    } else {
        stem
    }
}

fn retain_export(directory: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, ApiError> {
    if !directory.is_absolute() {
        return Err(ApiError::InvalidCommand(
            "export directory must be absolute".into(),
        ));
    }
    std::fs::create_dir_all(directory).map_err(|e| ApiError::Io(e.to_string()))?;
    let path = directory.join(format!("{name}-{}.3mf", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| ApiError::Io(e.to_string()))?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = result {
        // Only our own partial output is eligible for cleanup.
        let _ = std::fs::remove_file(&path);
        return Err(ApiError::Io(format!("3MF export failed: {error}")));
    }
    Ok(path)
}

fn no_document() -> ApiError {
    ApiError::NoDocument("extension export".into())
}
fn stale_target() -> ApiError {
    ApiError::InvalidCommand("extension export belongs to a stale document snapshot".into())
}

#[cfg(test)]
mod tests;
