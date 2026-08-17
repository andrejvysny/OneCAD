//! Geometry-export seam — the app-layer trait the `export_*_file` commands drive.
//!
//! The worker's `ExportStep` / `ExportStl` / `ExportObj` verbs (SCHEMA §7.8) are
//! surfaced on [`WorkerManager`] as inherent `export_*` methods; this module wraps
//! them behind a small object-safe [`GeometryExporter`] trait so
//! [`AppState`](crate::state::AppState) can hold `Arc<dyn GeometryExporter>`
//! alongside the geometry backend (the SAME `WorkerManager` Arc;
//! [`PendingBackend`] when no worker resolved). Keeping the trait + impls here means
//! `worker/manager.rs` and `worker/mod.rs` stay untouched.

use std::collections::BTreeMap;

use async_trait::async_trait;

use onecad_core::ids::{BodyId, ElementId, SnapshotId};
use onecad_core::regen::EngineError;

use crate::document_runtime::DocumentRuntime;
use crate::worker::{BakedGeometry, ElementQuery, PendingBackend, WorkerManager};

/// The document-owned appearance a STEP export carries alongside the geometry
/// (SCHEMA §7.8 `ExportStep`, DI-5).
///
/// Every map is keyed by the **wire** body id (`body_<uuid>`), because that is what
/// the worker addresses its `BodyStore` with. Face colours are keyed by the body's
/// **current `TopoKey`**, resolved here rather than in the worker: a `TopoKey` is
/// snapshot-scoped ordinal evidence, and Rust is the identity authority, so the
/// persistent `ElementId` → `TopoKey` step belongs on this side of the wire and is
/// valid only for the snapshot this export runs against. An `ElementId` that does
/// not resolve is OMITTED — never approximated onto a plausible ordinal.
///
/// `BTreeMap` throughout so the emitted JSON is byte-stable for a given document
/// state (a HashMap would reorder the object between runs for no reason).
///
/// Empty is the pre-DI-5 request: the worker then writes geometry only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepExportAttributes {
    /// Wire body id → the tree's display name, written as the STEP product name.
    pub body_names: BTreeMap<String, String>,
    /// Wire body id → the authored whole-body colour (sRGB `[r,g,b,a]`), written on
    /// the body's own XCAF label.
    pub body_colors: BTreeMap<String, [u8; 4]>,
    /// Wire body id → (`TopoKey` → authored face colour). Overrides both the
    /// whole-body colour and any import-derived colour the worker already holds.
    pub face_colors: BTreeMap<String, BTreeMap<String, [u8; 4]>>,
}

/// One authored face colour still waiting for its `ElementId` → `TopoKey` step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingFaceColor {
    /// The body the coloured face belongs to.
    pub body: BodyId,
    /// That body's wire id — carried along so the resolution pass need not re-derive it.
    pub wire_body_id: String,
    /// The persistent id the colour is stored against in the document.
    pub element: ElementId,
    /// The authored sRGB colour.
    pub color: [u8; 4],
}

/// Snapshot the head's export attributes out of the runtime.
///
/// **Synchronous by design.** The caller holds the single-writer lock across this
/// call, so nothing here may await — the `ElementId` → `TopoKey` resolution that
/// needs the worker is deferred to [`resolve_face_colors`], which the caller runs
/// after dropping the lock (the R-WP11 rule).
#[must_use]
pub fn pending_step_attributes(
    rt: &DocumentRuntime,
) -> (StepExportAttributes, Vec<PendingFaceColor>) {
    let mut attributes = StepExportAttributes::default();
    let mut pending = Vec::new();
    for body in rt.head_body_ids() {
        let Some(meta) = rt.body_meta(body) else {
            continue;
        };
        let wire_body_id = crate::worker::wire::body_id_wire(body);
        if !meta.name.is_empty() {
            attributes
                .body_names
                .insert(wire_body_id.clone(), meta.name);
        }
        if let Some(color) = meta.color {
            attributes.body_colors.insert(wire_body_id.clone(), color);
        }
        for (element, color) in meta.face_colors {
            pending.push(PendingFaceColor {
                body,
                wire_body_id: wire_body_id.clone(),
                element,
                color,
            });
        }
    }
    (attributes, pending)
}

/// Fold `pending` into `attributes` by resolving each `ElementId` to the `TopoKey`
/// it names in `snapshot` (`QueryElement`, one round-trip each).
///
/// Runs OUTSIDE the runtime lock. An id that does not resolve — a face an edit
/// consumed, or a re-bind the DI-4 ladder honestly refused — is **omitted** and
/// logged; it is never mapped onto a plausible ordinal, because a colour on the
/// wrong face looks entirely successful (the H5-B failure mode). Returns how many
/// were omitted.
pub async fn resolve_face_colors(
    attributes: &mut StepExportAttributes,
    pending: Vec<PendingFaceColor>,
    snapshot: Option<SnapshotId>,
    query: &dyn ElementQuery,
) -> usize {
    if pending.is_empty() {
        return 0;
    }
    let Some(snapshot) = snapshot else {
        tracing::warn!(
            pending = pending.len(),
            "exportStep: no published snapshot to resolve authored face colours against — \
             they are omitted"
        );
        return pending.len();
    };
    let mut omitted = 0usize;
    for item in pending {
        let resolved = query
            .query_element(snapshot, item.body, item.element.as_str())
            .await;
        let topo_key = match resolved {
            Ok(Some(info)) if !info.topo_key.is_empty() => info.topo_key,
            Ok(_) => {
                omitted += 1;
                continue;
            }
            Err(err) => {
                // A transport failure is not a ladder answer, but the export call
                // right after this reports it properly if the worker is really gone.
                // Dropping the colour is the safe direction either way.
                tracing::warn!(
                    error = %err,
                    element = item.element.as_str(),
                    "exportStep: QueryElement failed for an authored face colour"
                );
                omitted += 1;
                continue;
            }
        };
        attributes
            .face_colors
            .entry(item.wire_body_id)
            .or_default()
            .insert(topo_key, item.color);
    }
    if omitted > 0 {
        tracing::warn!(
            omitted,
            "exportStep: authored face colours whose element does not resolve in the current \
             head are omitted from the STEP file rather than guessed"
        );
    }
    omitted
}

/// Exports bodies to a mesh/BREP file on disk. `path` is the target file, `bodies`
/// the body ids to write. Object-safe so the app can store it as a trait object.
///
/// One trait for all three formats (M5a widened the M4 STEP-only seam): a document
/// has exactly one worker, so a single `Arc<dyn GeometryExporter>` routes every
/// export to it.
#[async_trait]
pub trait GeometryExporter: Send + Sync {
    /// Writes `bodies` to `path` as STEP using the `schema` (e.g. `"AP242DIS"`),
    /// carrying `attributes` (names + colours) through XCAF.
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker or a worker-side export failure.
    async fn export_step(
        &self,
        path: &str,
        bodies: &[BodyId],
        schema: &str,
        attributes: &StepExportAttributes,
    ) -> Result<(), EngineError>;

    /// Writes `bodies` to `path` as STL (binary when `binary`, else ASCII), meshed
    /// at `lod` (SCHEMA §7.8).
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker or a worker-side export failure.
    async fn export_stl(
        &self,
        path: &str,
        bodies: &[BodyId],
        binary: bool,
        lod: &str,
    ) -> Result<(), EngineError>;

    /// Writes `bodies` to `path` as ASCII OBJ, meshed at `lod` (SCHEMA §7.8).
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker or a worker-side export failure.
    async fn export_obj(&self, path: &str, bodies: &[BodyId], lod: &str)
        -> Result<(), EngineError>;

    /// Bakes `bodies` into a §7.3 REPLAY codec (`"brep"` / `"xbf"`) at `path`
    /// (SCHEMA §7.8 `ExportGeometry`), reporting the codec + format version the
    /// worker actually wrote.
    ///
    /// Not a user-facing export: this is the lane a Component Library `embedded` /
    /// `document` source is baked through (spec §2.1), whose output is read back
    /// as a content-addressed blob rather than handed to the user.
    ///
    /// `union_solids` opts into the worker's multi-solid fuse (WP-F1.2, spec §9).
    ///
    /// # Errors
    /// [`EngineError`] on a disconnected worker or a worker-side failure.
    async fn export_geometry(
        &self,
        path: &str,
        bodies: &[BodyId],
        codec: &str,
        union_solids: bool,
    ) -> Result<BakedGeometry, EngineError>;
}

#[async_trait]
impl GeometryExporter for WorkerManager {
    async fn export_step(
        &self,
        path: &str,
        bodies: &[BodyId],
        schema: &str,
        attributes: &StepExportAttributes,
    ) -> Result<(), EngineError> {
        // Inherent `WorkerManager::export_step` (SCHEMA §7.8 passthrough) wins path
        // resolution over this trait method; it returns the bytes written, which the
        // app command does not surface.
        WorkerManager::export_step(self, path, bodies, schema, attributes)
            .await
            .map(|_bytes_written| ())
    }

    async fn export_stl(
        &self,
        path: &str,
        bodies: &[BodyId],
        binary: bool,
        lod: &str,
    ) -> Result<(), EngineError> {
        WorkerManager::export_stl(self, path, bodies, binary, lod)
            .await
            .map(|_written| ())
    }

    async fn export_obj(
        &self,
        path: &str,
        bodies: &[BodyId],
        lod: &str,
    ) -> Result<(), EngineError> {
        WorkerManager::export_obj(self, path, bodies, lod)
            .await
            .map(|_bytes_written| ())
    }

    async fn export_geometry(
        &self,
        path: &str,
        bodies: &[BodyId],
        codec: &str,
        union_solids: bool,
    ) -> Result<BakedGeometry, EngineError> {
        WorkerManager::export_geometry(self, path, bodies, codec, union_solids).await
    }
}

#[async_trait]
impl GeometryExporter for PendingBackend {
    async fn export_step(
        &self,
        _path: &str,
        _bodies: &[BodyId],
        _schema: &str,
        _attributes: &StepExportAttributes,
    ) -> Result<(), EngineError> {
        Err(not_ready("STEP"))
    }

    async fn export_stl(
        &self,
        _path: &str,
        _bodies: &[BodyId],
        _binary: bool,
        _lod: &str,
    ) -> Result<(), EngineError> {
        Err(not_ready("STL"))
    }

    async fn export_obj(
        &self,
        _path: &str,
        _bodies: &[BodyId],
        _lod: &str,
    ) -> Result<(), EngineError> {
        Err(not_ready("OBJ"))
    }

    async fn export_geometry(
        &self,
        _path: &str,
        _bodies: &[BodyId],
        _codec: &str,
        _union_solids: bool,
    ) -> Result<BakedGeometry, EngineError> {
        Err(not_ready("component geometry"))
    }
}

fn not_ready(kind: &str) -> EngineError {
    EngineError::Protocol {
        message: format!("worker not started; {kind} export unavailable"),
    }
}
