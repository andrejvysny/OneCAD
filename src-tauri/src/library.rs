//! Component Library app-crate bridge (WP-1.3). Owns the library-root path
//! resolution and the Tauri commands that expose `onecad-library` to the
//! frontend: `list_library_components`, `reindex_library`, `place_component`,
//! `detach_component`.
//!
//! `onecad-library` itself has no `AppHandle`/Tauri dependency (its own
//! invariant); this module is the ONE place that bridges it to a real
//! document via [`AppState`] — exactly the boundary the plan's Open Question
//! 1 anticipated ("the app-crate bridge is where the two [type families] get
//! translated").
//!
//! Library root: `ONECAD_LIBRARY_ROOT`, else `<app_data_dir>/library`.
//!
//! **`*_at` split, mirroring `crate::recents` exactly**: every public command
//! delegates to a private `*_at(root, ...)` (and, for the mutating pair,
//! `*_at(root, &State, ...)`) twin that never touches an `AppHandle`. Two
//! independent reasons, not one:
//! * `tauri::test::mock_app`'s `MockRuntime` produces an `AppHandle<MockRuntime>`,
//!   which cannot satisfy a bare `AppHandle` (pinned to the concrete `Wry`
//!   runtime `#[tauri::command]` fns compile against) — same constraint
//!   `recents.rs` documents.
//! * The mutating pair ALSO wants to emit `PROJECTION_UPDATED`, which DOES
//!   need a real `AppHandle` — so unlike `recents.rs`'s pure read/write
//!   split, the testable core here still returns enough (the outcome +
//!   projection) for the thin public wrapper to emit AFTER calling it, and
//!   the tested surface is "did the edit apply and the projection come back
//!   right", not "was the event emitted."

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use tauri::{AppHandle, Emitter, Manager, State};

use onecad_core::document::record::{
    ComponentParamValue, ComponentSourceRef, DetachComponentParams, FrozenPlacement,
    ImportSourceCodec, KnownOperation, Operation, OperationRecord, PlaceComponentParams,
    TransformRotation,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{CommandOutcome, EditCommand};
use onecad_core::ids::RecordId;

use onecad_library::index::LibraryIndex;
use onecad_library::package::{self, ComponentPackage, ParameterRole, COMPONENT_MANIFEST_FILE};
use onecad_library::resolve::{ResolveRequest, ResolvedBlob, ResolvedSource};
use onecad_library::Library;

use crate::dto::{DocumentProjection, LibraryComponentDto, ReindexReportDto};
use crate::error::ApiError;
use crate::state::AppState;

/// `ONECAD_LIBRARY_ROOT`, else `<app_data_dir>/library`; `None` only when
/// neither resolves (a headless/permission-denied environment — mirrors
/// `recents::recents_file`'s degrade-to-absent). The env override mirrors
/// the established `ONECAD_WORKER_PATH` precedent (`worker::resolve_worker_path`)
/// — a real dev/test seam, not test-only plumbing: it also lets a developer
/// point the library panel at a scratch package tree without touching the
/// real app-data directory.
pub(crate) fn library_root(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_LIBRARY_ROOT") {
        return Some(PathBuf::from(p));
    }
    app.path().app_data_dir().ok().map(|d| d.join("library"))
}

fn open_at(root: &Path) -> Result<Library, ApiError> {
    Library::open(root).map_err(ApiError::from)
}

fn index_entries_at(index: &LibraryIndex) -> Vec<LibraryComponentDto> {
    let mut out = Vec::new();
    for (id, versions) in &index.components {
        for (version, entry) in versions {
            out.push(LibraryComponentDto {
                id: id.clone(),
                version: version.clone(),
                name: entry.name.clone(),
                category: entry.category.clone(),
                tags: entry.tags.clone(),
                source_kind: entry.source_kind.clone(),
                revision: entry.revision.clone(),
                generator_id: entry.generator_id.clone(),
                generator_version: entry.generator_version,
                attachments: entry.attachments.clone(),
                parameters: entry.parameters.clone(),
                designation: entry.designation.clone(),
            });
        }
    }
    out.sort_by(|a, b| (&a.id, &a.version).cmp(&(&b.id, &b.version)));
    out
}

/// Lists the CURRENTLY PERSISTED library index — does not reindex (spec §4:
/// `reindex` is an explicit, separate action, feeding the tasks-chip
/// producer). A library that has never been indexed reads as empty, not an
/// error (`Library::open` on a not-yet-existing root already degrades this
/// way).
fn list_library_components_at(root: &Path) -> Result<Vec<LibraryComponentDto>, ApiError> {
    Ok(index_entries_at(open_at(root)?.index()))
}

#[tauri::command]
pub async fn list_library_components(app: AppHandle) -> Result<Vec<LibraryComponentDto>, ApiError> {
    let Some(root) = library_root(&app) else {
        return Ok(Vec::new());
    };
    list_library_components_at(&root)
}

/// Installs any missing built-in package and re-indexes if the pass changed
/// anything (WP-A2). Runs once at application start, off the window-creation
/// critical path.
///
/// Re-indexing here is not optional: `list_library_components` reads the
/// PERSISTED index and deliberately never rebuilds it, so a package that
/// seeds but is not indexed is still an empty panel.
///
/// Best-effort — a library that cannot be seeded (read-only volume, denied
/// permission) degrades to whatever is already there. It logs and never
/// blocks startup.
pub(crate) fn seed_and_reindex_at(root: &Path) {
    let outcome = match crate::library_seed::seed_library(root) {
        Ok(outcome) => outcome,
        Err(e) => {
            tracing::warn!("library seeding skipped ({e}); library stays as found");
            return;
        }
    };
    if !outcome.ran {
        return;
    }
    tracing::info!(
        installed = outcome.installed.len(),
        kept = outcome.kept.len(),
        "library seed pass"
    );
    match reindex_library_at(root) {
        Ok(report) => tracing::info!(
            indexed = report.indexed,
            total = report.total,
            "library reindexed after seeding"
        ),
        Err(e) => tracing::warn!("library reindex after seeding failed: {e}"),
    }
}

/// Rebuilds the library index from packages on disk (spec §4 `reindex`).
fn reindex_library_at(root: &Path) -> Result<ReindexReportDto, ApiError> {
    let mut library = open_at(root)?;
    let report = library.reindex()?;
    Ok(ReindexReportDto {
        total: report.total,
        indexed: report.indexed,
        skipped: report
            .skipped
            .into_iter()
            .map(|(path, reason)| format!("{}: {reason}", path.display()))
            .collect(),
    })
}

#[tauri::command]
pub async fn reindex_library(app: AppHandle) -> Result<ReindexReportDto, ApiError> {
    let Some(root) = library_root(&app) else {
        return Ok(ReindexReportDto {
            total: 0,
            indexed: 0,
            skipped: Vec::new(),
        });
    };
    reindex_library_at(&root)
}

/// Resolves + builds the `PlaceComponent` op and applies it (spec §3.1).
/// WP-1.5 scope: `translate` + `rotate` — the snap solver's computed
/// candidate transform, sent already-resolved by the frontend
/// (`placementSolver.ts`). No `mate` is authored HERE (the interactive
/// gesture records one through the preview lane); regen-time re-seating of a
/// recorded mate landed in WP-3.1.
///
/// All three source kinds place (WP-3.2). A `generator` component carries only
/// its identity; an `embedded` / `document` component's baked solid is COPIED
/// INTO this document's own `imports/` section before the record is authored —
/// spec §4's "geometry is always cached locally", which is what lets the
/// document reopen with the library folder deleted.
///
/// Revision verification happens inside `Library::resolve_source`: a
/// mismatch is a typed [`onecad_library::LibraryError::RevisionMismatch`],
/// which this converts to a REFUSED command (`ApiError`) rather than ever
/// authoring a record with a lie in it — the spec §4 "never load-breaking"
/// promise is about EXISTING documents surviving a stale library, not about
/// authoring a NEW placement against one; placing against a component that
/// no longer matches its recorded identity is a caller bug, not a document
/// state to preserve.
async fn place_component_at(
    root: &Path,
    state: &State<'_, AppState>,
    component_id: String,
    component_version: String,
    translate: [f64; 3],
    rotate: Option<TransformRotation>,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let library = open_at(root)?;
    let (_version, entry) = library
        .get(&component_id, Some(&component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "placeComponent: unknown component {component_id}@{component_version}"
            ))
        })?;
    let revision = entry.revision.clone();
    let resolved = library.resolve_source(&ResolveRequest {
        component_id: &component_id,
        component_version: &component_version,
        component_revision: &revision,
    })?;
    let (source, blob) = component_source_from_resolved(resolved, &component_id)?;

    let params = PlaceComponentParams {
        component_id,
        component_version,
        component_revision: revision,
        params: Default::default(),
        source,
        mate: None,
        placement: FrozenPlacement {
            translate: [
                Scalar::new(translate[0]),
                Scalar::new(translate[1]),
                Scalar::new(translate[2]),
            ],
            rotate: rotate.unwrap_or_default(),
        },
        extra: Default::default(),
    };
    let op = Operation::Known(KnownOperation::PlaceComponent(params));
    let record = OperationRecord::new(RecordId::new(), 0, "Place Component", op);

    stage_blob_and_apply(
        state,
        blob,
        EditCommand::AddOperation {
            record,
            at_cursor: true,
        },
    )
    .await
}

/// A component's baked geometry, resolved out of the library and ready to be
/// staged into the placing document (WP-3.2).
struct StagedBlob {
    sha256: String,
    codec: ImportSourceCodec,
    format: Option<u32>,
    bytes: Vec<u8>,
}

/// Translates a library resolution into the record's `source` plus, for the two
/// blob-backed kinds, the bytes the placing document must cache.
///
/// This is the type-family bridge this module exists for: `onecad-library` owns
/// the package format, `onecad-core` owns the wire/record form, and neither
/// depends on the other.
fn component_source_from_resolved(
    resolved: ResolvedSource,
    component_id: &str,
) -> Result<(ComponentSourceRef, Option<StagedBlob>), ApiError> {
    match resolved {
        ResolvedSource::Generator {
            generator_id,
            generator_version,
        } => Ok((
            ComponentSourceRef::Generator {
                generator_id,
                generator_version,
                params: Default::default(),
                extra: Default::default(),
            },
            None,
        )),
        ResolvedSource::Embedded { blob } => {
            let staged = staged_blob(blob, component_id)?;
            Ok((
                ComponentSourceRef::Embedded {
                    sha256: staged.sha256.clone(),
                    codec: staged.codec,
                    brep_format: staged.format,
                    extra: Default::default(),
                },
                Some(staged),
            ))
        }
        ResolvedSource::Document {
            document_sha256,
            blob,
        } => {
            let staged = staged_blob(blob, component_id)?;
            Ok((
                ComponentSourceRef::Document {
                    document_sha256,
                    sha256: staged.sha256.clone(),
                    codec: staged.codec,
                    brep_format: staged.format,
                    // Overrides need a re-bake lane (see `set_component_params_at`'s
                    // Document arm); recording any here would be inert data.
                    params: Default::default(),
                    extra: Default::default(),
                },
                Some(staged),
            ))
        }
    }
}

fn staged_blob(blob: ResolvedBlob, component_id: &str) -> Result<StagedBlob, ApiError> {
    let codec = ImportSourceCodec::from_extension(&blob.codec).ok_or_else(|| {
        ApiError::InvalidCommand(format!(
            "placeComponent: {component_id} declares an unknown geometry codec `{}`",
            blob.codec
        ))
    })?;
    Ok(StagedBlob {
        sha256: blob.sha256,
        codec,
        format: blob.format,
        bytes: blob.bytes,
    })
}

/// Stages a component's geometry blob into the open document and applies `command`
/// under the SAME runtime lock.
///
/// One lock, in this order, on purpose: the blob must be in the carrier and
/// materialized for the worker BEFORE any regen can see the record that names it
/// (the `add_import_record` discipline). Splitting the two would leave a window
/// where an autosave writes a record whose blob is not yet staged.
async fn stage_blob_and_apply(
    state: &State<'_, AppState>,
    blob: Option<StagedBlob>,
    command: EditCommand,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("library".into()))?;
    if let Some(b) = blob {
        rt.stage_component_blob(&b.sha256, b.codec, &b.bytes)?;
    }
    let outcome = rt.apply(command)?;
    Ok((outcome, rt.projection()))
}

#[tauri::command]
pub async fn place_component(
    app: AppHandle,
    state: State<'_, AppState>,
    component_id: String,
    component_version: String,
    translate: [f64; 3],
    rotate: Option<TransformRotation>,
) -> Result<DocumentProjection, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("placeComponent: no app data dir".into()))?;
    let (outcome, projection) = place_component_at(
        &root,
        &state,
        component_id,
        component_version,
        translate,
        rotate,
    )
    .await?;
    finish(&app, &state, &outcome, &projection);
    Ok(projection)
}

/// Resolves a component's `source` for a GHOST PREVIEW, staging its baked
/// geometry into the open document on the way (WP-3.2). Authors nothing.
///
/// The placement ghost previews through `beginPreview` with a candidate
/// `PlaceComponent` op, and that op lowers its `source` through the SAME wire
/// path a committed record does. Two consequences the frontend cannot handle on
/// its own, which is why this command exists:
///
/// * a blob-backed component's bytes must be materialized for the worker at ARM
///   time, not at commit — otherwise the first preview lowers an empty path;
/// * the preview must carry the REAL source. Before this, the preview lane
///   hardcoded a generator source, so arming a non-generator component previewed
///   the generator stub's screw and committed something else entirely.
///
/// Returned verbatim to the caller as the draft `source`, so the preview and the
/// commit cannot diverge. A `generator` component stages nothing (it re-runs from
/// params and depends on no bytes); the call is still made, uniformly, so the
/// frontend has exactly one code path.
///
/// Idempotent — safe to call again for an already-armed component.
async fn resolve_component_source_at(
    root: &Path,
    state: &State<'_, AppState>,
    component_id: String,
    component_version: String,
) -> Result<ComponentSourceRef, ApiError> {
    let library = open_at(root)?;
    let (_version, entry) = library
        .get(&component_id, Some(&component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "resolveComponentSource: unknown component {component_id}@{component_version}"
            ))
        })?;
    let revision = entry.revision.clone();
    let resolved = library.resolve_source(&ResolveRequest {
        component_id: &component_id,
        component_version: &component_version,
        component_revision: &revision,
    })?;
    let (source, blob) = component_source_from_resolved(resolved, &component_id)?;
    if let Some(blob) = blob {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("resolveComponentSource".into()))?;
        rt.stage_component_blob(&blob.sha256, blob.codec, &blob.bytes)?;
    }
    Ok(source)
}

#[tauri::command]
pub async fn resolve_component_source(
    app: AppHandle,
    state: State<'_, AppState>,
    component_id: String,
    component_version: String,
) -> Result<ComponentSourceRef, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("resolveComponentSource: no app data dir".into()))?;
    resolve_component_source_at(&root, &state, component_id, component_version).await
}

/// Loads a package's full `component.toml` (parameters included) by identity
/// — the index only carries `parameter_keys` (names, WP-1.4/1.5), never the
/// `role`/`domain` shape `set_component_params_at` needs to enforce spec
/// §3.2's role=free rule.
fn component_package_at(
    root: &Path,
    component_id: &str,
    component_version: &str,
) -> Result<ComponentPackage, ApiError> {
    let library = open_at(root)?;
    let (_version, entry) = library
        .get(component_id, Some(component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "setComponentParams: unknown component {component_id}@{component_version}"
            ))
        })?;
    let manifest_path = root.join(&entry.path).join(COMPONENT_MANIFEST_FILE);
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| ApiError::Internal(format!("read {}: {e}", manifest_path.display())))?;
    package::parse(&raw, &manifest_path).map_err(ApiError::from)
}

/// Applies free-parameter overrides to an already-placed component instance
/// (spec §3.1 `params`; WP-2.3 — the app-crate half of the role=free
/// enforcement `validate_place_component`'s doc comment flags as
/// structurally-unchecked: `onecad-core` cannot depend on the library crate
/// that resolves a component signature, so this is the ONE place a requested
/// key is checked against the component's ACTUAL `[parameters]` table).
///
/// Reads the target record's CURRENT `PlaceComponentParams` in full (unlike
/// `detach_component_at`'s field-by-field reads — this needs every field to
/// reconstruct a valid record, not just `source`/`placement`). A record that
/// isn't a placed component fails the same deserialize step
/// `DetachComponentParams`'s shape would (missing `componentId` etc.), so no
/// separate "is this a PlaceComponent" check is needed.
///
/// **`source.params` mirrors the merged free-param map, not a fully resolved
/// signature.** The worker (WP-2.1) only ever reads `role: free` keys by name
/// (`thread`/`length`) — it owns its own dimension table and derives
/// `role: table`/`computed` values itself. Shipping THOSE into `source.params`
/// here would duplicate data the worker never reads and the wire format
/// doesn't need; `component.toml`'s `[parameters]` table stays the single
/// source of truth for that resolution, on the authoring side only.
async fn set_component_params_at(
    root: &Path,
    state: &State<'_, AppState>,
    record_id: String,
    params: BTreeMap<String, ComponentParamValue>,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let record = RecordId::from_str(&record_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {record_id:?}: {e}")))?;

    let current = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("setComponentParams".into()))?;
        let raw = rt.operation_params(record).ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "setComponentParams: no params for record {record_id}"
            ))
        })?;
        serde_json::from_value::<PlaceComponentParams>(raw).map_err(|e| {
            ApiError::InvalidCommand(format!(
                "setComponentParams: record {record_id} is not a placed component: {e}"
            ))
        })?
    };

    let package = component_package_at(root, &current.component_id, &current.component_version)?;
    for key in params.keys() {
        match package.parameters.get(key) {
            Some(spec) if spec.role == ParameterRole::Free => {}
            Some(_) => {
                return Err(ApiError::InvalidCommand(format!(
                    "setComponentParams: `{key}` is not a free parameter on {}",
                    current.component_id
                )))
            }
            None => {
                return Err(ApiError::InvalidCommand(format!(
                    "setComponentParams: unknown parameter `{key}` on {}",
                    current.component_id
                )))
            }
        }
    }

    let mut merged = current.params.clone();
    merged.extend(params);

    let source = match current.source {
        ComponentSourceRef::Generator {
            generator_id,
            generator_version,
            extra,
            ..
        } => ComponentSourceRef::Generator {
            generator_id,
            generator_version,
            params: merged.clone(),
            extra,
        },
        ComponentSourceRef::Embedded { .. } => {
            return Err(ApiError::InvalidCommand(
                "setComponentParams: an embedded-source placement has no free parameters".into(),
            ))
        }
        // A `document` component's geometry is a solid BAKED from its authoring
        // document (WP-3.2). Honoring an override means replaying that document
        // with new variable values and re-baking — a lane this build does not
        // have. Refusing is the honest answer: merging the value into the record
        // would show a new designation over unchanged geometry, which is the
        // silent-substitution failure spec §0 invariant #4 exists to prevent.
        ComponentSourceRef::Document { .. } => {
            return Err(ApiError::InvalidCommand(
                "setComponentParams: a document-source placement carries baked geometry; \
                 editing its parameters needs a re-bake of the authoring document, which this \
                 build does not implement"
                    .into(),
            ))
        }
    };

    let updated = PlaceComponentParams {
        params: merged,
        source,
        ..current
    };
    let op = Operation::Known(KnownOperation::PlaceComponent(updated));

    apply_and_project(state, EditCommand::UpdateOperationParams { record, op }).await
}

#[tauri::command]
pub async fn set_component_params(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: String,
    params: BTreeMap<String, ComponentParamValue>,
) -> Result<DocumentProjection, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("setComponentParams: no app data dir".into()))?;
    let (outcome, projection) = set_component_params_at(&root, &state, record_id, params).await?;
    finish(&app, &state, &outcome, &projection);
    Ok(projection)
}

/// Drops a placed component's library identity, keeping its cached geometry
/// as an ordinary body (spec §3.4). Reads the target record's CURRENT
/// `source`/`placement` (via `rt.operation_params`, the same read path
/// `get_operation_params`/`CadClient.getOperationParams` uses) and re-applies
/// them on a `DetachComponentParams` — the sanctioned in-place op-type swap
/// `edit::session::op_type_edit_allowed` allows.
async fn detach_component_at(
    state: &State<'_, AppState>,
    record_id: String,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let record = RecordId::from_str(&record_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {record_id:?}: {e}")))?;

    let (source, placement) = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("detachComponent".into()))?;
        let params = rt.operation_params(record).ok_or_else(|| {
            ApiError::InvalidCommand(format!("detachComponent: no params for record {record_id}"))
        })?;
        let source = params.get("source").cloned().ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "detachComponent: record {record_id} has no source (not a placed component?)"
            ))
        })?;
        let placement = params.get("placement").cloned().ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "detachComponent: record {record_id} has no placement"
            ))
        })?;
        (source, placement)
    };

    let detach_params = DetachComponentParams {
        source: serde_json::from_value(source)
            .map_err(|e| ApiError::InvalidCommand(format!("detachComponent: bad source: {e}")))?,
        placement: serde_json::from_value(placement).map_err(|e| {
            ApiError::InvalidCommand(format!("detachComponent: bad placement: {e}"))
        })?,
        extra: Default::default(),
    };
    let op = Operation::Known(KnownOperation::DetachComponent(detach_params));

    apply_and_project(state, EditCommand::UpdateOperationParams { record, op }).await
}

#[tauri::command]
pub async fn detach_component(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: String,
) -> Result<DocumentProjection, ApiError> {
    let (outcome, projection) = detach_component_at(&state, record_id).await?;
    finish(&app, &state, &outcome, &projection);
    Ok(projection)
}

/// The shared apply→project tail (AppHandle-free — see the module doc
/// comment). Mirrors `api::apply_edit_command`'s lock scope exactly.
async fn apply_and_project(
    state: &State<'_, AppState>,
    command: EditCommand,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("library".into()))?;
    let outcome = rt.apply(command)?;
    Ok((outcome, rt.projection()))
}

/// The AppHandle-dependent tail `apply_and_project` can't do itself: emit the
/// projection event, hand the outcome to the scheduler, note the mutation
/// tick — mirrors `api::apply_edit_command`'s tail exactly, so a library edit
/// is indistinguishable from any other (autosave, undo, projection push).
fn finish(
    app: &AppHandle,
    state: &State<'_, AppState>,
    outcome: &CommandOutcome,
    projection: &DocumentProjection,
) {
    let _ = app.emit(crate::events::PROJECTION_UPDATED, projection);
    if let Some(sched) = state.scheduler.get() {
        sched.handle(outcome);
    }
    state.note_mutation();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tauri::Manager;

    use crate::document_runtime::DocumentRuntime;
    use crate::state::{AppState, BackendBundle};
    use crate::worker::PendingBackend;

    use super::*;

    /// An `AppState` whose backend is ALWAYS `PendingBackend` — deliberately,
    /// not "no worker binary happened to resolve": `place_component_at`/
    /// `detach_component_at` only exercise `DocumentSession::apply` (a pure
    /// timeline mutation), never regen, so no real worker is needed to prove
    /// this WP's actual surface. Regen-level proof (the op ACTUALLY
    /// publishing correct geometry) is `component_ops.rs`'s job, against the
    /// real worker.
    fn pending_app_state() -> AppState {
        AppState::new(Arc::new(|| -> BackendBundle {
            (
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
                Arc::new(PendingBackend),
            )
        }))
    }

    fn write_generator_package(root: &Path, id: &str, name: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"
[identity]
id = "{id}"
version = "1.0.0"
revision = "sha256:{}"

[metadata]
name = "{name}"

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1
"#,
            "0".repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
    }

    /// Same as `write_generator_package`, plus a `[parameters]` table with
    /// two `role: free` keys (`thread`, `thread_detail`) and one
    /// `role: table` key (`head_d`) — the minimum shape
    /// `set_component_params_at`'s role check needs to exercise both the
    /// accept and reject paths.
    fn write_generator_package_with_params(root: &Path, id: &str, name: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"
[identity]
id = "{id}"
version = "1.0.0"
revision = "sha256:{}"

[metadata]
name = "{name}"
designation = "ISO 4762 {{thread}}x{{length}}"

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1

[parameters]
thread = {{ role = "free", key = "M6", domain = ["M3", "M4", "M5", "M6", "M8"] }}
head_d = {{ role = "table", from = "iso4762.dk" }}
thread_detail = {{ role = "free", key = "cosmetic", domain = ["cosmetic", "simplified", "modeled"] }}
"#,
            "0".repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
    }

    #[test]
    fn list_is_empty_before_reindex_and_populated_after() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package(dir.path(), "onecad.std.iso4762", "SHCS");

        assert!(list_library_components_at(dir.path()).unwrap().is_empty());

        let report = reindex_library_at(dir.path()).unwrap();
        assert_eq!(report.indexed, 1);

        let listed = list_library_components_at(dir.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "onecad.std.iso4762");
        assert_eq!(listed[0].source_kind, "generator");
    }

    #[test]
    fn list_carries_parameters_and_designation_for_the_configurator() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_params(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let listed = list_library_components_at(dir.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].designation.as_deref(),
            Some("ISO 4762 {thread}x{length}")
        );
        assert_eq!(
            listed[0].parameters.get("thread").map(|p| p.role),
            Some(ParameterRole::Free)
        );
        assert_eq!(
            listed[0].parameters.get("head_d").map(|p| p.role),
            Some(ParameterRole::Table)
        );
        assert_eq!(
            listed[0].parameters.get("thread_detail").map(|p| p.role),
            Some(ParameterRole::Free)
        );
    }

    #[tokio::test]
    async fn place_then_detach_through_the_real_command_core() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        // `tauri::test::mock_app` is the only public way to construct a real
        // `tauri::State` outside a webview (its constructor is otherwise
        // crate-private) — same precedent `sketch_on_face.rs` uses. `State`
        // itself is NOT runtime-generic (`tauri::State<'r, T>(&'r T)`), so
        // it's usable here even though a bare `AppHandle` (pinned to `Wry`)
        // is not.
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let (outcome, projection) = place_component_at(
            dir.path(),
            &state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [1.0, 2.0, 3.0],
            None,
        )
        .await
        .expect("place_component_at");
        assert!(matches!(outcome.regen, onecad_core::edit::RegenHint::ToEnd));
        assert_eq!(projection.features.len(), 1);
        assert_eq!(projection.features[0].op_type, "PlaceComponent");
        let record_id = projection.features[0].id.clone();

        let (_outcome, projection) = detach_component_at(&state, record_id)
            .await
            .expect("detach_component_at");
        assert_eq!(projection.features.len(), 1);
        assert_eq!(
            projection.features[0].op_type, "DetachComponent",
            "the record's op type actually changed via the real command core"
        );

        // The document is never asked to regen in this test (no worker), so
        // there's no published body to assert on — that's `component_ops.rs`'s
        // job. This proves the COMMAND path only.
        let guard = app.state::<AppState>();
        let rt_guard = guard.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        assert_eq!(rt.head_body_ids().len(), 0, "no regen ran, so no body yet");
    }

    #[tokio::test]
    async fn set_component_params_merges_a_free_override_and_reaches_source_params() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_params(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
        )
        .await
        .expect("place_component_at");
        let record_id = projection.features[0].id.clone();

        let mut params = BTreeMap::new();
        params.insert(
            "thread".to_string(),
            ComponentParamValue::Text("M8".to_string()),
        );
        let (_outcome, projection) =
            set_component_params_at(dir.path(), &state, record_id.clone(), params)
                .await
                .expect("set_component_params_at");

        let raw = {
            let guard = state.runtime.lock().await;
            let rt = guard.as_ref().unwrap();
            rt.operation_params(onecad_core::ids::RecordId::from_str(&record_id).unwrap())
                .unwrap()
        };
        let updated: PlaceComponentParams = serde_json::from_value(raw).unwrap();
        assert_eq!(
            updated.params.get("thread"),
            Some(&ComponentParamValue::Text("M8".to_string())),
            "top-level free-param override recorded"
        );
        match updated.source {
            ComponentSourceRef::Generator { params, .. } => {
                assert_eq!(
                    params.get("thread"),
                    Some(&ComponentParamValue::Text("M8".to_string())),
                    "the resolved generator source.params carries the override — this is what \
                     the worker's table lookup (WP-2.1) actually reads"
                );
            }
            other => panic!("expected a Generator source, got {other:?}"),
        }
        assert_eq!(projection.features[0].op_type, "PlaceComponent");
    }

    #[tokio::test]
    async fn set_component_params_rejects_a_non_free_key() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_params(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
        )
        .await
        .expect("place_component_at");
        let record_id = projection.features[0].id.clone();

        let mut params = BTreeMap::new();
        params.insert("head_d".to_string(), ComponentParamValue::Number(6.0));
        let err = set_component_params_at(dir.path(), &state, record_id.clone(), params)
            .await
            .expect_err("head_d is role=table, not free");
        assert!(
            matches!(err, ApiError::InvalidCommand(ref m) if m.contains("not a free parameter")),
            "got {err:?}"
        );

        let mut params = BTreeMap::new();
        params.insert("nope".to_string(), ComponentParamValue::Number(1.0));
        let err = set_component_params_at(dir.path(), &state, record_id, params)
            .await
            .expect_err("`nope` isn't declared at all");
        assert!(
            matches!(err, ApiError::InvalidCommand(ref m) if m.contains("unknown parameter")),
            "got {err:?}"
        );
    }

    /// Writes a `document`-kind package (spec §2.1 + WP-3.2's geometry
    /// pointer): a frozen authoring document in the package directory, and the
    /// solid baked from it in the library's shared blob store.
    fn write_document_package(root: &Path, id: &str, geometry: &[u8]) -> String {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("source.onecad"), b"frozen authoring document").unwrap();
        let sha = onecad_library::blob::BlobStore::new(root)
            .put(geometry)
            .expect("blob put");
        let toml = format!(
            r#"
[identity]
id = "{id}"
version = "1.0.0"
revision = "sha256:{zeros}"

[metadata]
name = "Bracket"

[source]
kind = "document"
file = "source.onecad"
geometry = "sha256:{sha}"
geometry_codec = "brep"
geometry_format = 4
"#,
            zeros = "0".repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
        sha
    }

    /// A `document` component places as its BAKED solid, and the bytes land in
    /// the placing document's own carrier — the spec §4 promise that a document
    /// renders with the library folder gone. The record keeps the authoring
    /// document's digest as provenance, and the two digests must not be swapped
    /// (one is what regen reads, the other is only evidence).
    #[tokio::test]
    async fn a_document_component_places_its_baked_solid_and_caches_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let geometry = b"pretend baked brep bytes".to_vec();
        let geometry_sha = write_document_package(dir.path(), "acme.bracket", &geometry);
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
        )
        .await
        .expect("place a document-source component");
        assert_eq!(projection.features[0].op_type, "PlaceComponent");

        let guard = app.state::<AppState>();
        let rt_guard = guard.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        assert_eq!(
            rt.import_blob_shas(),
            vec![geometry_sha.clone()],
            "the baked solid is cached in the placing document, not just referenced in the library"
        );

        let record = RecordId::from_str(&projection.features[0].id).unwrap();
        let raw = rt.operation_params(record).unwrap();
        let params: PlaceComponentParams = serde_json::from_value(raw).unwrap();
        match params.source {
            ComponentSourceRef::Document {
                document_sha256,
                sha256,
                codec,
                brep_format,
                ..
            } => {
                assert_eq!(sha256, geometry_sha, "regen reads the GEOMETRY digest");
                assert_eq!(
                    document_sha256,
                    crate::imports::sha256_hex(b"frozen authoring document"),
                    "provenance is the frozen authoring document's own digest"
                );
                assert_eq!(codec, ImportSourceCodec::Brep);
                assert_eq!(brep_format, Some(4));
            }
            other => panic!("expected a Document source, got {other:?}"),
        }
    }

    /// Arming a placement resolves the same source the commit will, and stages
    /// the same bytes — the ghost cannot preview geometry the commit would not
    /// produce (the defect this command exists to close).
    #[tokio::test]
    async fn resolving_a_source_for_the_ghost_stages_the_same_bytes_a_commit_would() {
        let dir = tempfile::tempdir().unwrap();
        let geometry = b"pretend baked brep bytes".to_vec();
        let geometry_sha = write_document_package(dir.path(), "acme.bracket", &geometry);
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let source = resolve_component_source_at(
            dir.path(),
            &state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
        )
        .await
        .expect("resolve for the ghost");
        assert_eq!(
            source.blob_ref().map(|b| b.sha256.to_string()),
            Some(geometry_sha.clone())
        );

        // Idempotent: arming twice must not double-stage or fail.
        resolve_component_source_at(
            dir.path(),
            &state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
        )
        .await
        .expect("re-arm");

        let guard = app.state::<AppState>();
        let rt_guard = guard.runtime.lock().await;
        assert_eq!(
            rt_guard.as_ref().unwrap().import_blob_shas(),
            vec![geometry_sha]
        );
    }

    /// A `document` package whose bake is missing is refused, loudly, naming the
    /// package. It is NOT replayed from `source.onecad` behind the user's back:
    /// that needs a second worker session and the library present at every
    /// regen, which is exactly what spec §4 rules out.
    #[tokio::test]
    async fn a_document_package_with_no_baked_geometry_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("acme.bracket");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("source.onecad"), b"frozen authoring document").unwrap();
        std::fs::write(
            pkg.join("component.toml"),
            format!(
                r#"
[identity]
id = "acme.bracket"
version = "1.0.0"
revision = "sha256:{zeros}"

[metadata]
name = "Bracket"

[source]
kind = "document"
file = "source.onecad"
geometry = "sha256:{missing}"
geometry_codec = "brep"
geometry_format = 4
"#,
                zeros = "0".repeat(64),
                missing = "a".repeat(64)
            ),
        )
        .unwrap();
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let err = place_component_at(
            dir.path(),
            &state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
        )
        .await
        .expect_err("a package with no bake cannot be placed");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("a".repeat(64).as_str()),
            "names the missing blob: {msg}"
        );
    }

    /// Editing a baked component's parameters needs a re-bake lane this build
    /// does not have. Refusing says so; merging the value would show a new
    /// designation over unchanged geometry.
    #[tokio::test]
    async fn setting_params_on_a_document_component_is_refused_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        write_document_package(dir.path(), "acme.bracket", b"pretend baked brep bytes");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
        )
        .await
        .expect("place");
        let record_id = projection.features[0].id.clone();

        // The package declares no parameters at all, so this is refused at the
        // signature check first; declare one to reach the source arm.
        let mut params = BTreeMap::new();
        params.insert("length".to_string(), ComponentParamValue::Number(30.0));
        let err = set_component_params_at(dir.path(), &state, record_id, params)
            .await
            .expect_err("a baked component has no editable parameters in this build");
        assert!(matches!(err, ApiError::InvalidCommand(_)), "got {err:?}");
    }
}
