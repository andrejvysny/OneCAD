//! Component-library application services shared by all hosts.
//!
//! Explicit library paths and application state preserve one implementation for
//! interactive placement, previews, templates, rebaking, and headless ingestion.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

pub(crate) mod commit_authorization;

use crate::host::{AppEvent, HostContext};

use onecad_core::document::record::{
    ComponentMate, ComponentParamValue, ComponentSourceRef, DetachComponentParams, FrozenPlacement,
    ImportSourceCodec, KnownOperation, MateFrame, Operation, OperationRecord, PlaceComponentParams,
    TransformRotation,
};
use onecad_core::document::refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{CommandOutcome, EditCommand};
use onecad_core::ids::{BodyId, ElementId, RecordId, TopoKey};
use onecad_core::math::Vec3;
use onecad_core::regen::{CancelToken, GeometryEngine, Outcome, RegenRequest};

use onecad_library::index::{IndexEntry, LibraryIndex};
#[cfg(test)]
use onecad_library::package::COMPONENT_MANIFEST_FILE;
use onecad_library::package::{self, ComponentPackage, ParameterRole, ParameterSpec};
use onecad_library::resolve::{ResolveRequest, ResolvedBlob, ResolvedSnapshot, ResolvedSource};
use onecad_library::search::{SearchQuery, SearchSort};
use onecad_library::Library;

use crate::document_runtime::DocumentRuntime;
#[cfg(test)]
#[path = "library_bootstrap_tests.rs"]
mod bootstrap_tests;
use crate::dto::{
    ComponentDetailDto, ComponentStatusDto, ComponentUpgradeDto, DocumentProjection,
    LibraryComponentDto, PlaceComponentMateInput, ProjectTemplateDto, ReindexReportDto,
    ReplaceComponentReportDto, ReplaceComponentResultDto, SearchQueryDto, SearchSortDto,
};
use crate::error::ApiError;
use crate::export::GeometryExporter;
use crate::state::AppState;
use crate::worker::{MeshProvider, SolverEngine, WorkerBackend, WorkerManager, WorkerRole};

/// Runs the synchronous index/disk work of a catalog wrapper off the async worker
/// thread; a panicking job is an error, not a hung caller.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|e| ApiError::Internal(format!("library task stopped: {e}")))?
}

/// `ONECAD_LIBRARY_ROOT`, else `<app_data_dir>/library`; `None` only when
/// neither resolves (a headless/permission-denied environment — mirrors
/// `recents::recents_file`'s degrade-to-absent). The env override mirrors
/// the established `ONECAD_WORKER_PATH` precedent (`worker::resolve_worker_path`)
/// — a real dev/test seam, not test-only plumbing: it also lets a developer
/// point the library panel at a scratch package tree without touching the
/// real app-data directory.
pub fn library_root(app: &HostContext) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_LIBRARY_ROOT") {
        return Some(PathBuf::from(p));
    }
    app.paths.app_data.clone().map(|d| d.join("library"))
}

pub(crate) fn open_at(root: &Path) -> Result<Library, ApiError> {
    Library::open(root).map_err(ApiError::from)
}

fn component_dto(root: &Path, id: &str, version: &str, entry: &IndexEntry) -> LibraryComponentDto {
    // Disk check, not the cached `has_preview`, so list/search/detail agree.
    let preview = Some(root.join(&entry.path).join("preview.png")).filter(|p| p.is_file());
    LibraryComponentDto {
        id: id.to_string(),
        version: version.to_string(),
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
        standard: entry.standard.clone(),
        unit: entry.unit.clone(),
        preview_path: preview.map(|p| p.to_string_lossy().into_owned()),
        configurations: entry.configurations.clone(),
        default_configuration: entry.default_configuration.clone(),
    }
}

fn index_entries_at(root: &Path, index: &LibraryIndex) -> Vec<LibraryComponentDto> {
    let mut out = Vec::new();
    for (id, versions) in &index.components {
        for (version, entry) in versions {
            out.push(component_dto(root, id, version, entry));
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
pub fn list_library_components_at(root: &Path) -> Result<Vec<LibraryComponentDto>, ApiError> {
    Ok(index_entries_at(root, open_at(root)?.index()))
}

pub async fn list_library_components(
    app: HostContext,
) -> Result<Vec<LibraryComponentDto>, ApiError> {
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
/// A failed seed preserves a readable existing index. Missing, malformed or
/// unsupported indices report an initialization error to catalog consumers;
/// window creation remains independent of this blocking disk work.
pub fn seed_and_reindex_at(root: &Path) -> Result<(), String> {
    // Multiple document hosts initialize the same library. The marker and index
    // form one bootstrap transaction, including template installation.
    static BOOTSTRAP: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _transaction = BOOTSTRAP.lock().unwrap_or_else(|error| error.into_inner());
    let index_path = root.join(onecad_library::index::INDEX_FILE);
    let prior = LibraryIndex::load(&index_path).map_err(|error| error.to_string())?;
    if prior.index_version != onecad_library::index::INDEX_VERSION {
        return Err(format!(
            "unsupported library index version {}",
            prior.index_version
        ));
    }
    let outcome = match crate::library_seed::seed_library(root) {
        Ok(outcome) => outcome,
        Err(e) => {
            tracing::warn!("library seeding skipped ({e}); library stays as found");
            return if index_path.is_file() {
                Ok(())
            } else {
                Err(e.to_string())
            };
        }
    };
    if !outcome.ran
        && index_path.is_file()
        && !crate::library_seed::seed_index_needs_rebuild(root, &prior)?
    {
        return Ok(());
    }
    tracing::info!(
        installed = outcome.installed.len(),
        kept = outcome.kept.len(),
        pruned = outcome.pruned.len(),
        adopted = outcome.adopted.len(),
        templates_installed = outcome.templates_installed.len(),
        templates_kept = outcome.templates_kept.len(),
        "library seed pass"
    );
    // A pass that DELETED from the user's library names what it took, at a
    // level a user-facing log actually shows. The counts above are the summary;
    // this is the receipt.
    if !outcome.pruned.is_empty() {
        tracing::info!(
            packages = ?outcome.pruned,
            "pruned retired built-in packages this app installed and no longer ships"
        );
    }
    if !outcome.adopted.is_empty() {
        tracing::info!(
            packages = ?outcome.adopted,
            "retired built-in packages left in place — modified since install, so they are the user's now"
        );
    }
    match reindex_library_at(root) {
        Ok(report) => {
            tracing::info!(
                indexed = report.indexed,
                total = report.total,
                "library reindexed after seeding"
            );
            Ok(())
        }
        Err(e) => {
            tracing::warn!("library reindex after seeding failed: {e}");
            Err(e.to_string())
        }
    }
}

/// Rebuilds the library index from packages on disk (spec §4 `reindex`).
pub fn reindex_library_at(root: &Path) -> Result<ReindexReportDto, ApiError> {
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

pub async fn reindex_library(app: HostContext) -> Result<ReindexReportDto, ApiError> {
    let Some(root) = library_root(&app) else {
        return Ok(ReindexReportDto {
            total: 0,
            indexed: 0,
            skipped: Vec::new(),
        });
    };
    blocking(move || reindex_library_at(&root)).await
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
/// Builds the recorded [`ComponentMate`] from the gesture's raw pick evidence
/// plus the promoted (or pick-carried) element id. Pure — promotion happens at
/// the call site, under the runtime lock.
pub fn component_mate_from_input(
    input: &PlaceComponentMateInput,
    body: BodyId,
    element: ElementId,
    self_frame: Option<MateFrame>,
) -> ComponentMate {
    ComponentMate {
        self_attachment: input.self_attachment.clone(),
        self_frame,
        target: ElementRef {
            import_source_identity: None,
            primary: Some(PrimaryRef {
                body,
                element,
                kind: if input.target_kind == "edge" {
                    ElementKind::Edge
                } else {
                    ElementKind::Face
                },
                extra: Default::default(),
            }),
            // No `intent`: the descriptor is worker-owned evidence; the ladder
            // re-derives it (same reasoning as `add_sketch_on_face`).
            intent: None,
            anchor: Some(AnchorIntent {
                world_point: Vec3::new_unchecked(
                    input.anchor_world_point[0],
                    input.anchor_world_point[1],
                    input.anchor_world_point[2],
                ),
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        },
        kind: input.kind,
        flipped: input.flipped,
        // WP-I: the frozen axis/sidedness are BACKFILLED by the §7.2
        // `mateResolved` writeback on this record's first regen — the pick
        // evidence this builder reads carries neither.
        target_axis: None,
        target_sidedness: None,
        extra: Default::default(),
    }
}

/// Resolves the gesture's mate evidence to a full [`ComponentMate`], promoting
/// the pick's `TopoKey` to a Rust-minted `ElementId` at the CURRENT head when
/// the pick carried none (§0 invariant 8 — the frontend never fabricates
/// identity). FAILS CLOSED: a mate that cannot be promoted refuses the whole
/// placement rather than silently degrading to a free-space record — the
/// user saw a snap, so a commit that forgot it would lose re-seating without
/// a trace (the same class of silent degradation spec §4 forbids).
async fn resolve_mate_input(
    state: &AppState,
    input: &PlaceComponentMateInput,
    self_frame: Option<MateFrame>,
    runtime_session: &str,
) -> Result<ComponentMate, ApiError> {
    if input.self_attachment.trim().is_empty() {
        return Err(ApiError::InvalidCommand(
            "placeComponent: mate.selfAttachment must not be empty".into(),
        ));
    }
    let body = crate::worker::wire::parse_body_id(&input.target_body_id)
        .map_err(|e| ApiError::InvalidCommand(format!("placeComponent: mate.targetBodyId: {e}")))?;
    if let Some(id) = input.target_element_id.as_deref().filter(|s| !s.is_empty()) {
        return Ok(component_mate_from_input(
            input,
            body,
            ElementId::new(id),
            self_frame,
        ));
    }
    let element = promote_mate_target_for_session(
        state,
        &input.target_body_id,
        &input.target_topo_key,
        input.anchor_world_point,
        Some(runtime_session),
    )
    .await?;
    Ok(component_mate_from_input(
        input,
        body,
        ElementId::new(&element),
        self_frame,
    ))
}

/// Promotes a pick's `TopoKey` to a Rust-minted element id at the CURRENT head
/// (`promote_selection`), the identity half of [`resolve_mate_input`]. Mints
/// PERSISTED identity (`regen.elements`) and dirties the document: a commit-time
/// caller only. A hover/draft caller must use [`stage_mate_target`].
///
/// # Errors
/// `InvalidCommand` when there is no head yet or the target does not resolve.
pub async fn promote_mate_target(
    state: &AppState,
    target_body_id: &str,
    target_topo_key: &str,
    anchor_world_point: [f64; 3],
) -> Result<String, ApiError> {
    promote_mate_target_for_session(
        state,
        target_body_id,
        target_topo_key,
        anchor_world_point,
        None,
    )
    .await
}

async fn promote_mate_target_for_session(
    state: &AppState,
    target_body_id: &str,
    target_topo_key: &str,
    anchor_world_point: [f64; 3],
    expected_session: Option<&str>,
) -> Result<String, ApiError> {
    let body = crate::worker::wire::parse_body_id(target_body_id)
        .map_err(|e| ApiError::InvalidCommand(format!("placeComponent: mate.targetBodyId: {e}")))?;
    let anchor = AnchorIntent {
        world_point: Vec3::new_unchecked(
            anchor_world_point[0],
            anchor_world_point[1],
            anchor_world_point[2],
        ),
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("placeComponent".into()))?;
    if let Some(expected) = expected_session {
        require_runtime_session(rt, expected)?;
    }
    let Some(head) = rt.head_snapshot_id() else {
        return Err(ApiError::InvalidCommand(
            "placeComponent: cannot record a mate before the first publish — re-pick".into(),
        ));
    };
    let promoted = rt
        .promote_selection(
            head,
            body,
            vec![(TopoKey::new(target_topo_key), Some(anchor))],
        )
        .await
        .map_err(|e| {
            ApiError::InvalidCommand(format!(
                "placeComponent: mate target could not be promoted ({e}) — re-pick"
            ))
        })?;
    let Some(first) = promoted.first() else {
        return Err(ApiError::InvalidCommand(format!(
            "placeComponent: mate target `{target_topo_key}` on body {target_body_id} did not \
             resolve at the head — re-pick"
        )));
    };
    Ok(first.element_id.clone())
}

/// Stages a draft mate target's element id WITHOUT persisting or dirtying
/// (the operand lanes' mechanism): the id lives in the runtime's staged map and
/// is adopted into `doc.elements` only when a committed record references it —
/// `apply_operation` of a `PlaceComponent` does; a Copy (`DetachComponent`, no
/// mate) or an abandoned draft leaves nothing behind.
///
/// `fence` is the pick's snapshot scope. A fence that is not the current head is
/// refused with [`ApiError::StalePreview`]; the head is never substituted.
///
/// # Errors
/// `StalePreview` for a stale fence; `InvalidCommand` for an unresolvable target.
pub async fn stage_mate_target(
    state: &AppState,
    target_body_id: &str,
    target_topo_key: &str,
    anchor_world_point: [f64; 3],
    fence: &crate::dto::GeometryReadFenceDto,
) -> Result<String, ApiError> {
    let body = crate::worker::wire::parse_body_id(target_body_id)
        .map_err(|e| ApiError::InvalidCommand(format!("placeComponent: mate.targetBodyId: {e}")))?;
    let anchor = AnchorIntent {
        world_point: Vec3::new_unchecked(
            anchor_world_point[0],
            anchor_world_point[1],
            anchor_world_point[2],
        ),
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("placeComponent".into()))?;
    let ticket = rt
        .stage_ticket_for_fence(
            &fence.document_id,
            &fence.runtime_session,
            onecad_core::ids::SnapshotId(fence.snapshot_id),
        )
        .map_err(stage_failure)?;
    let refs = rt
        .stage_anchored(
            &ticket,
            body,
            vec![(TopoKey::new(target_topo_key), Some(anchor))],
        )
        .await
        .map_err(stage_failure)?;
    refs.first()
        .and_then(|r| r.primary.as_ref())
        .map(|p| p.element.as_str().to_string())
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "placeComponent: mate target `{target_topo_key}` did not resolve — re-pick"
            ))
        })
}

fn stage_failure(error: onecad_core::regen::EngineError) -> ApiError {
    use onecad_core::regen::{EngineError, OpFailureCode};
    match error {
        EngineError::OpFailed {
            code: OpFailureCode::StalePreview,
            message,
            ..
        } => ApiError::StalePreview(message),
        other => ApiError::InvalidCommand(format!(
            "placeComponent: mate target could not be staged ({other}) — re-pick"
        )),
    }
}

/// The component-local frame `attachment` declares, converted to the record's
/// own type (spec §2.1 → §3.1; WP-F1.1). `None` when the package declares no
/// frame for it — including when it declares no such attachment at all.
///
/// Deliberately NOT a refusal for an unknown key: WP-H2 authors mates without
/// validating `selfAttachment` against the package, and turning that into a
/// hard error here would change an unrelated contract. An unknown key simply
/// seats at the model origin, exactly as it did before this landed.
///
/// The axes are already orthonormal — `package::parse` normalizes every frame
/// it reads, so nothing downstream re-derives a basis.
fn attachment_frame(package: &ComponentPackage, attachment: &str) -> Option<MateFrame> {
    let frame = package.attachments.get(attachment)?.frame?;
    Some(MateFrame {
        origin: Vec3::new_unchecked(frame.origin[0], frame.origin[1], frame.origin[2]),
        z: Vec3::new_unchecked(frame.z[0], frame.z[1], frame.z[2]),
        x: Vec3::new_unchecked(frame.x[0], frame.x[1], frame.x[2]),
    })
}

/// Everything a placement needs, as one value (the argument list of
/// [`place_component_at`], named so a native host can build it incrementally).
#[derive(Debug, Clone)]
pub struct PlaceComponentRequest {
    pub component_id: String,
    pub component_version: String,
    pub translate: [f64; 3],
    pub rotate: Option<TransformRotation>,
    pub params: BTreeMap<String, ComponentParamValue>,
    pub mate: Option<PlaceComponentMateInput>,
    /// A named package configuration (`[configurations.<name>]`) to place.
    /// Resolved to its params map and merged UNDER `params`, so an explicit
    /// override still wins. The package's `default_configuration` is NOT
    /// applied implicitly: the caller (a picker) chooses.
    pub configuration: Option<String>,
}

/// A placement resolved and authored but not yet applied: the `AddOperation`
/// command plus the geometry blob the open document must cache before any
/// regen can see the record that names it.
pub struct PreparedPlacement {
    pub command: EditCommand,
    pub blob: Option<StagedBlob>,
    pub runtime_session: String,
    pub(crate) archive: crate::library_archive::CapturedArchive,
}

/// The testable core of [`place_component`] — `pub` for the same reason
/// [`save_as_component_at`] is: a worker-backed integration test drives the real
/// application logic directly over explicit state.
// One argument per placement field preserves the existing application contract.
#[allow(clippy::too_many_arguments)]
pub async fn place_component_at(
    root: &Path,
    state: &AppState,
    component_id: String,
    component_version: String,
    translate: [f64; 3],
    rotate: Option<TransformRotation>,
    free_params: BTreeMap<String, ComponentParamValue>,
    mate: Option<PlaceComponentMateInput>,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let prepared = prepare_place_component_at(
        root,
        state,
        PlaceComponentRequest {
            component_id,
            component_version,
            translate,
            rotate,
            params: free_params,
            mate,
            configuration: None,
        },
    )
    .await?;
    stage_archive_and_apply(
        state,
        prepared.blob,
        prepared.archive,
        prepared.command,
        &prepared.runtime_session,
    )
    .await
}

/// Everything [`place_component_at`] does up to, but not including, staging
/// the blob and applying the command.
///
/// Not side-effect free when `mate` is `Some`: the mate's target selection is
/// promoted (`promote_selection`) here, minting identity in the open document.
/// So it is not meant to run per frame of a preview.
pub async fn prepare_place_component_at(
    root: &Path,
    state: &AppState,
    req: PlaceComponentRequest,
) -> Result<PreparedPlacement, ApiError> {
    let runtime_session = current_runtime_session(state, "placeComponent").await?;
    prepare_place_with_session(
        root,
        state,
        req,
        runtime_session,
        onecad_library::blob::MAX_BLOB_BYTES,
    )
    .await
}

async fn prepare_place_with_session(
    root: &Path,
    state: &AppState,
    req: PlaceComponentRequest,
    runtime_session: String,
    source_limit: u64,
) -> Result<PreparedPlacement, ApiError> {
    let PlaceComponentRequest {
        component_id,
        component_version,
        translate,
        rotate,
        params: free_params,
        mate,
        configuration,
    } = req;
    let source_retention = state
        .archive_budget()
        .reserve(
            MAX_DRAFT_PAYLOAD_PART as usize,
            crate::resource_admission::ArchiveUse::Scratch,
        )
        .map_err(ApiError::Io)?;
    let snapshot = component_snapshot_at_bounded(
        "placeComponent",
        root,
        &component_id,
        &component_version,
        None,
        source_limit.min(MAX_DRAFT_PAYLOAD_PART),
    )
    .map_err(|error| match error {
        onecad_library::LibraryError::BlobTooLarge { .. }
            if source_limit == MAX_CAPTURED_DRAFT_BYTES =>
        {
            ApiError::InvalidCommand("placement draft source exceeds the 32 MiB limit".into())
        }
        other => ApiError::from(other),
    })?;
    let archive = crate::library_archive::capture(
        &snapshot,
        MAX_CAPTURED_DRAFT_BYTES,
        &state.archive_budget(),
    )?;
    let package = &snapshot.package;
    let free_params = match &configuration {
        Some(name) => configured_params(package, &component_id, name, free_params)?,
        None => free_params,
    };
    // Role enforcement at the PLACE site, not just the edit site (WP-A3): the
    // interactive gesture now sends free params of its own (auto-size on a hole
    // rim), and a placement that writes a non-free key would author a record
    // `set_component_params` would then refuse to touch.
    if !free_params.is_empty() {
        validate_component_params("placeComponent", package, &component_id, &free_params)?;
    }
    // Refused BEFORE the mate is promoted: promotion mints identity, and a
    // refused placement must leave nothing behind.
    if let Some(input) = &mate {
        refuse_variant_mate(package, &input.self_attachment, &free_params)?;
    }
    // Promote FIRST (its own short lock): identity must exist before the record
    // that names it is authored; `stage_blob_and_apply` re-locks below.
    //
    // The attachment's own local frame is FROZEN into the mate here (WP-F1.1),
    // read from the package the placement is authored against — so the record
    // re-seats identically later even with the library deleted, and a package
    // revision that moves its attachment never silently moves this instance.
    let mate = match &mate {
        Some(input) => {
            let self_frame = attachment_frame(package, &input.self_attachment);
            Some(resolve_mate_input(state, input, self_frame, &runtime_session).await?)
        }
        None => None,
    };
    let revision = snapshot.package.identity.revision.clone();
    let declared = declared_numeric_params(&snapshot.package.parameters);
    let (source, blob) = component_source_from_resolved(
        snapshot.source,
        &component_id,
        &declared,
        Some(source_retention),
    )?;

    let mut params = PlaceComponentParams {
        component_id,
        component_version,
        component_revision: revision,
        params: free_params,
        source,
        mate,
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
    let (source, blob) = placed_source(
        state,
        &snapshot.package,
        snapshot.document.as_deref(),
        &params,
        blob,
    )
    .await?;
    params.source = source;
    let retained = archive
        .bytes
        .len()
        .saturating_add(blob.as_ref().map_or(0, |blob| blob.bytes.len()));
    if retained as u64 > MAX_CAPTURED_DRAFT_BYTES {
        return Err(ApiError::InvalidCommand(
            "placement draft source and archive exceed the 32 MiB limit".into(),
        ));
    }
    let op = Operation::Known(KnownOperation::PlaceComponent(params));
    let record = OperationRecord::new(RecordId::new(), 0, "Place Component", op);
    Ok(PreparedPlacement {
        command: EditCommand::AddOperation {
            record,
            at_cursor: true,
        },
        blob,
        runtime_session,
        archive,
    })
}

async fn current_runtime_session(state: &AppState, what: &str) -> Result<String, ApiError> {
    let guard = state.runtime.lock().await;
    guard
        .as_ref()
        .map(DocumentRuntime::runtime_session)
        .ok_or_else(|| ApiError::NoDocument(what.into()))
}

fn require_runtime_session(rt: &DocumentRuntime, expected: &str) -> Result<(), ApiError> {
    if rt.matches_runtime_session(expected) {
        Ok(())
    } else {
        Err(ApiError::StalePreview(
            "document session changed; retry the component operation".into(),
        ))
    }
}

/// Stages a prepared placement's blob into the open document without applying
/// anything — for a preview that needs the bytes materialized before commit.
/// Idempotent.
pub async fn stage_prepared_blob(
    state: &AppState,
    prepared: &PreparedPlacement,
) -> Result<(), ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("library".into()))?;
    require_runtime_session(rt, &prepared.runtime_session)?;
    if let Some(blob) = &prepared.blob {
        blob.stage_into(rt)?;
    }
    Ok(())
}

const MAX_CAPTURED_DRAFT_BYTES: u64 = 32 * 1024 * 1024;
// Three simultaneous payloads: captured closure, ZIP and optional re-bake output.
const MAX_DRAFT_PAYLOAD_PART: u64 = (MAX_CAPTURED_DRAFT_BYTES - 4096) / 3;

/// Arms a placement draft: authors the record the commit will use and stages its
/// blob, returning the record's `params` for the preview/commit pair to share.
///
/// Never promotes: a `mate` must already carry `target_element_id` (see
/// [`stage_mate_target`]) so arming has no hidden side effect on identity and
/// the preview cannot diverge from the commit.
pub async fn prepare_component_draft_at(
    root: &Path,
    state: &AppState,
    req: PlaceComponentRequest,
) -> Result<PlaceComponentParams, ApiError> {
    let generation = begin_component_draft_capture(state)?;
    prepare_component_draft_generation_at(root, state, req, generation).await
}

async fn prepare_component_draft_generation_at(
    root: &Path,
    state: &AppState,
    req: PlaceComponentRequest,
    generation: u64,
) -> Result<PlaceComponentParams, ApiError> {
    if let Some(mate) = &req.mate {
        if mate.target_element_id.as_deref().is_none_or(str::is_empty) {
            return Err(ApiError::InvalidCommand(
                "placeComponent: a draft mate needs targetElementId — promote the pick first"
                    .into(),
            ));
        }
    }
    let runtime_session = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("placeComponent".into()))?;
        let _slot = state.captured_library_draft.lock().unwrap();
        if component_draft_generation(state) != generation {
            return Err(ApiError::StalePreview(
                "component request was cancelled before admission".into(),
            ));
        }
        rt.runtime_session()
    };
    let pending =
        prepare_place_with_session(root, state, req, runtime_session, MAX_CAPTURED_DRAFT_BYTES);
    finish_draft_when_ready(state, pending, generation).await
}

async fn finish_draft_when_ready<F>(
    state: &AppState,
    pending: F,
    generation: u64,
) -> Result<PlaceComponentParams, ApiError>
where
    F: std::future::Future<Output = Result<PreparedPlacement, ApiError>>,
{
    let prepared = pending.await?;
    if prepared
        .blob
        .as_ref()
        .is_some_and(|b| b.bytes.len() as u64 > MAX_CAPTURED_DRAFT_BYTES)
    {
        return Err(ApiError::InvalidCommand(
            "placement draft geometry exceeds the 32 MiB source limit".into(),
        ));
    }
    finish_component_draft(state, prepared, generation).await
}

async fn finish_component_draft(
    state: &AppState,
    prepared: PreparedPlacement,
    generation: u64,
) -> Result<PlaceComponentParams, ApiError> {
    let EditCommand::AddOperation {
        record:
            OperationRecord {
                op: Operation::Known(KnownOperation::PlaceComponent(params)),
                ..
            },
        ..
    } = prepared.command
    else {
        return Err(ApiError::Internal(
            "prepare_component_draft: placement did not author a PlaceComponent".into(),
        ));
    };
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("library".into()))?;
    require_runtime_session(rt, &prepared.runtime_session)?;
    let mut slot = state.captured_library_draft.lock().unwrap();
    if state
        .captured_library_generation
        .load(std::sync::atomic::Ordering::Relaxed)
        != generation
    {
        return Err(ApiError::StalePreview(
            "component draft was superseded or cancelled".into(),
        ));
    }
    let workspace = if let Some(blob) = &prepared.blob {
        let mut workspace = crate::imports::ImportWorkspace::new(rt.document_uuid());
        workspace
            .materialize(&blob.sha256, blob.codec, &blob.bytes)
            .map_err(|error| {
                ApiError::Io(format!("cannot materialize component draft: {error}"))
            })?;
        Some(std::sync::Arc::new(workspace))
    } else {
        None
    };
    *slot = Some(CapturedDraft {
        workspace,
        archive: Some(prepared.archive),
        source: params.source.clone(),
        component_id: params.component_id.clone(),
        component_version: params.component_version.clone(),
        component_revision: params.component_revision.clone(),
        source_sha: params.source.blob_ref().map(|b| b.sha256.to_string()),
        runtime_session: prepared.runtime_session,
        blob: prepared.blob,
    });
    Ok(params)
}

/// [`prepare_component_draft_at`] over the app's library root.
pub async fn prepare_component_draft(
    app: &HostContext,
    state: &AppState,
    req: PlaceComponentRequest,
) -> Result<PlaceComponentParams, ApiError> {
    let root = library_root(app)
        .ok_or_else(|| ApiError::Internal("placeComponent: no app data dir".into()))?;
    prepare_component_draft_at(&root, state, req).await
}

pub async fn prepare_component_draft_in_generation(
    app: &HostContext,
    state: &AppState,
    req: PlaceComponentRequest,
    generation: u64,
) -> Result<PlaceComponentParams, ApiError> {
    let root = library_root(app)
        .ok_or_else(|| ApiError::Internal("placeComponent: no app data dir".into()))?;
    prepare_component_draft_generation_at(&root, state, req, generation).await
}

pub fn component_draft_generation(state: &AppState) -> u64 {
    state
        .captured_library_generation
        .load(std::sync::atomic::Ordering::Acquire)
}

pub fn begin_component_draft_capture(state: &AppState) -> Result<u64, ApiError> {
    let mut slot = state.captured_library_draft.lock().unwrap();
    let previous = state
        .captured_library_generation
        .try_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |value| value.checked_add(1).filter(|next| *next != u64::MAX),
        )
        .map_err(|_| ApiError::Internal("component request identity exhausted".into()))?;
    *slot = None;
    Ok(previous + 1)
}

/// Re-stage geometry from the document carrier or the single captured draft.
/// No package or blob paths are read after the draft was prepared.
pub async fn restage_component_blob_at(
    _root: &Path,
    state: &AppState,
    params: &PlaceComponentParams,
) -> Result<(), ApiError> {
    let want = params.source.blob_ref().map(|b| b.sha256.to_string());
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("restageComponentBlob".into()))?;
    let mut slot = state.captured_library_draft.lock().unwrap();
    let draft = slot.as_mut().filter(|draft| draft.matches(params));
    if let Some(draft) = draft.as_ref() {
        require_runtime_session(rt, &draft.runtime_session)?;
    }
    let Some(want) = want else {
        return Ok(());
    };
    if rt.restage_carried_blob(&want)? {
        if let Some(draft) = draft {
            draft.blob = None;
        }
        return Ok(());
    }
    let draft = draft.ok_or_else(|| {
        ApiError::InvalidCommand(format!(
            "placeComponent: geometry for {}@{} is not in the document or captured draft",
            params.component_id, params.component_version
        ))
    })?;
    let blob = draft.blob.as_ref().ok_or_else(|| {
        ApiError::InvalidCommand(format!(
            "placeComponent: geometry for {}@{} is not in the document or captured draft",
            params.component_id, params.component_version
        ))
    })?;
    blob.stage_into(rt)?;
    draft.blob = None;
    Ok(())
}

/// Release the host's single captured draft, for cancellation and shutdown.
pub fn clear_component_draft_capture(state: &AppState) {
    let mut slot = state.captured_library_draft.lock().unwrap();
    state
        .captured_library_generation
        .try_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |value| value.checked_add(1),
        )
        .ok();
    *slot = None;
}

/// Total bytes retained by the captured draft.
#[must_use]
pub fn captured_component_draft_bytes(state: &AppState) -> usize {
    state
        .captured_library_draft
        .lock()
        .unwrap()
        .as_ref()
        .map_or(0, |draft| {
            draft.blob.as_ref().map_or(0, |blob| blob.bytes.len())
                + draft
                    .archive
                    .as_ref()
                    .map_or(0, |archive| archive.bytes.len())
        })
}

/// Source-only bytes charged to the native draft lease; archive bytes have their own lease.
#[must_use]
pub fn captured_component_draft_source_bytes(state: &AppState) -> usize {
    state
        .captured_library_draft
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|draft| draft.blob.as_ref())
        .map_or(0, |blob| blob.bytes.len())
}

/// Session captured with the currently armed matching draft, when present.
#[must_use]
pub fn captured_component_draft_session(
    state: &AppState,
    params: &PlaceComponentParams,
) -> Option<String> {
    let guard = state.captured_library_draft.lock().unwrap();
    let draft = guard.as_ref();
    draft
        .filter(|draft| draft.matches(params))
        .map(|draft| draft.runtime_session.clone())
}

/// [`restage_component_blob_at`] over the app's library root.
pub async fn restage_component_blob(
    app: &HostContext,
    state: &AppState,
    params: &PlaceComponentParams,
) -> Result<(), ApiError> {
    let root = library_root(app)
        .ok_or_else(|| ApiError::Internal("placeComponent: no app data dir".into()))?;
    restage_component_blob_at(&root, state, params).await
}

/// The `DetachComponent` twin of a freshly authored `PlaceComponent` record:
/// same id, source and placement (bindings cleared, as `detach_component_at`
/// does), so adding it directly equals Place followed by Detach.
///
/// `api::apply_operation` refuses a batch that adds a record and then swaps its
/// op type (it accepts fresh `AddOperation`s or ONE `UpdateOperationParams`), so
/// "Copy" is a single `AddOperation` of this record: one undo step by
/// construction.
///
/// `resolved` is the worker's mate-resolved placement (`PreviewOp`
/// `matePlacement`): when `Some` it REPLACES the draft placement so the copy
/// lands at the exact snapped transform; `None` keeps the draft's (unmated copy).
///
/// # Errors
/// `InvalidCommand` when `record` is not a `PlaceComponent`.
pub fn detached_record(
    record: OperationRecord,
    resolved: Option<FrozenPlacement>,
) -> Result<OperationRecord, ApiError> {
    let Operation::Known(KnownOperation::PlaceComponent(placed)) = record.op else {
        return Err(ApiError::InvalidCommand(
            "copyPlacement: record is not a PlaceComponent".into(),
        ));
    };
    let mut placement = resolved.unwrap_or(placed.placement);
    placement.clear_bindings();
    let op = Operation::Known(KnownOperation::DetachComponent(DetachComponentParams {
        source: placed.source,
        placement,
        extra: Default::default(),
    }));
    Ok(OperationRecord::new(
        record.record_id,
        record.step_index,
        record.name,
        op,
    ))
}

/// The commands for a "Copy" placement: one `AddOperation` of the detached twin.
///
/// # Errors
/// See [`detached_record`].
pub fn copy_placement_commands(
    record: OperationRecord,
    resolved: Option<FrozenPlacement>,
) -> Result<Vec<EditCommand>, ApiError> {
    Ok(vec![EditCommand::AddOperation {
        record: detached_record(record, resolved)?,
        at_cursor: true,
    }])
}

/// Rejects any key that is not `role = "free"` in the component's own
/// signature (spec §3.1/§3.2), then checks each value against what the
/// package declares for it: same kind as the declared default, finite when a
/// number, a member of `domain`, and at least `min`. `snap` is a UI hint and is
/// not enforced. `onecad-core` cannot depend on the crate that resolves a
/// signature, so this is the only place the check can live; the place, edit
/// and replace sites all call it, and `what` names the caller in the message.
pub(crate) fn validate_component_params(
    what: &str,
    package: &ComponentPackage,
    component_id: &str,
    params: &BTreeMap<String, ComponentParamValue>,
) -> Result<(), ApiError> {
    for key in params.keys() {
        match package.parameters.get(key) {
            Some(spec) if spec.role == ParameterRole::Free => {}
            Some(_) => {
                return Err(ApiError::InvalidCommand(format!(
                    "{what}: `{key}` is not a free parameter on {component_id}"
                )))
            }
            None => {
                return Err(ApiError::InvalidCommand(format!(
                    "{what}: unknown parameter `{key}` on {component_id}"
                )))
            }
        }
    }
    for (key, value) in params {
        if let Some(spec) = package.parameters.get(key) {
            validate_param_value(what, component_id, key, spec, value)?;
        }
    }
    Ok(())
}

fn validate_param_value(
    what: &str,
    component_id: &str,
    key: &str,
    spec: &ParameterSpec,
    value: &ComponentParamValue,
) -> Result<(), ApiError> {
    let refuse = |why: String| {
        Err(ApiError::InvalidCommand(format!(
            "{what}: parameter `{key}` on {component_id} {why}"
        )))
    };
    if let Some(default) = &spec.value {
        if !param_kind_matches(default, value) {
            return refuse(format!(
                "expects the same kind as its default {default}, got {value:?}"
            ));
        }
    }
    if matches!(value, ComponentParamValue::Number(n) if !n.is_finite()) {
        return refuse("must be a finite number".into());
    }
    if let Some(domain) = &spec.domain {
        if !domain.iter().any(|d| param_equals(d, value)) {
            return refuse(format!(
                "is {value:?}, which is outside its declared domain"
            ));
        }
    }
    if let Some(min) = spec.min {
        match value {
            ComponentParamValue::Number(n) if *n >= min => {}
            ComponentParamValue::Number(n) => {
                return refuse(format!("is {n}, below its minimum {min}"))
            }
            _ => return refuse(format!("must be a number of at least {min}, got {value:?}")),
        }
    }
    Ok(())
}

fn param_kind_matches(declared: &toml::Value, value: &ComponentParamValue) -> bool {
    matches!(
        (declared, value),
        (
            toml::Value::Integer(_) | toml::Value::Float(_),
            ComponentParamValue::Number(_)
        ) | (toml::Value::String(_), ComponentParamValue::Text(_))
            | (toml::Value::Boolean(_), ComponentParamValue::Bool(_))
    )
}

fn param_equals(declared: &toml::Value, value: &ComponentParamValue) -> bool {
    match (declared, value) {
        (toml::Value::Integer(i), ComponentParamValue::Number(n)) => *i as f64 == *n,
        (toml::Value::Float(f), ComponentParamValue::Number(n)) => {
            (f - n).abs() <= 1e-9 * f.abs().max(1.0)
        }
        (toml::Value::String(s), ComponentParamValue::Text(t)) => s == t,
        (toml::Value::Boolean(b), ComponentParamValue::Bool(v)) => b == v,
        _ => false,
    }
}

/// Carries the caller's free params into the resolved source.
///
/// Only the two kinds that re-derive geometry from params consume them. A
/// `generator` source re-runs the generator on every regen, and a `profile`
/// source re-prisms its canonical face to `params.length` — which is exactly
/// what makes each one's size editable. A source carrying geometry BAKED at
/// authoring time has no such lane: recording params against it would show one
/// designation over unrelated geometry, the silent substitution spec §0
/// invariant 4 forbids. Refused loudly instead.
///
/// The two live kinds differ in how the params arrive. A `generator`'s are the
/// RESOLVED set and are replaced wholesale. A `profile`'s are MERGED over the
/// package defaults already seeded by
/// [`component_source_from_resolved`] — `length` is required, so dropping the
/// seed to honour a partial override would author an invalid record.
///
/// A `document` component's params are honoured by RE-BAKING (WP-F1.3), in
/// `placed_source` at placement and in `set_component_params_at` on an
/// instance; this function is not that lane and refuses baked kinds.
fn source_with_free_params(
    source: ComponentSourceRef,
    free_params: &BTreeMap<String, ComponentParamValue>,
    component_id: &str,
) -> Result<ComponentSourceRef, ApiError> {
    if free_params.is_empty() {
        return Ok(source);
    }
    source.with_free_params(free_params).map_err(|_| {
        ApiError::InvalidCommand(format!(
            "placeComponent: {component_id} carries baked geometry, which has no free \
             parameters to set"
        ))
    })
}

/// The record `source` (and the blob it needs staged) for a placement whose
/// free params are already merged into `params.params`.
///
/// A `document` source with params is RE-BAKED through the same lane
/// `setComponentParams` uses — the solid the library carries is the package's
/// default variant, so recording params over it would show a variant's
/// designation on the default's geometry (spec §0 invariant 4). The document
/// sha stays the package's frozen one; only the baked blob moves.
async fn placed_source(
    state: &AppState,
    package: &ComponentPackage,
    document: Option<&[u8]>,
    params: &PlaceComponentParams,
    blob: Option<StagedBlob>,
) -> Result<(ComponentSourceRef, Option<StagedBlob>), ApiError> {
    let merged = &params.params;
    match &params.source {
        ComponentSourceRef::Document {
            document_sha256,
            extra,
            ..
        } if !merged.is_empty() => {
            let staged = rebake_document_component(
                (state.services().backend(), &state.archive_budget()),
                package,
                document,
                params,
                document_sha256.as_str(),
                merged,
                MAX_DRAFT_PAYLOAD_PART,
            )
            .await?;
            Ok((
                ComponentSourceRef::Document {
                    document_sha256: document_sha256.clone(),
                    sha256: staged.sha256.clone(),
                    codec: staged.codec,
                    brep_format: staged.format,
                    params: merged.clone(),
                    extra: extra.clone(),
                },
                Some(staged),
            ))
        }
        source => Ok((
            source_with_free_params(source.clone(), merged, &params.component_id)?,
            blob,
        )),
    }
}

/// `toml` default -> record param value; `None` for a table/array/datetime.
pub(crate) fn toml_to_param(value: &toml::Value) -> Option<ComponentParamValue> {
    match value {
        toml::Value::Integer(i) => Some(ComponentParamValue::Number(*i as f64)),
        toml::Value::Float(f) => Some(ComponentParamValue::Number(*f)),
        toml::Value::String(s) => Some(ComponentParamValue::Text(s.clone())),
        toml::Value::Boolean(b) => Some(ComponentParamValue::Bool(*b)),
        _ => None,
    }
}

/// The package's declared defaults for its free parameters, whatever their kind.
pub(crate) fn free_defaults(
    specs: &BTreeMap<String, ParameterSpec>,
) -> BTreeMap<String, ComponentParamValue> {
    specs
        .iter()
        .filter(|(_, spec)| spec.role == ParameterRole::Free)
        .filter_map(|(key, spec)| Some((key.clone(), toml_to_param(spec.value.as_ref()?)?)))
        .collect()
}

/// Whether `params` holds a value other than the package's CURRENT default.
pub(crate) fn differs_from_defaults(
    package: &ComponentPackage,
    params: &BTreeMap<String, ComponentParamValue>,
) -> bool {
    let defaults = free_defaults(&package.parameters);
    params.iter().any(|(k, v)| defaults.get(k) != Some(v))
}

const VARIANT_MATE_REFUSAL: &str = "Mating a non-default variant of an authored component isn't \
     supported yet — place it unmated or use the default variant";

/// An authored attachment frame was captured on the DEFAULT bake. A re-baked
/// variant has different geometry, so seating it by that frame would silently
/// mis-seat. Refused for a `document` package when the chosen attachment
/// carries a frame and `params` is a non-default variant.
fn refuse_variant_mate(
    package: &ComponentPackage,
    attachment: &str,
    params: &BTreeMap<String, ComponentParamValue>,
) -> Result<(), ApiError> {
    let framed = package
        .attachments
        .get(attachment)
        .is_some_and(|a| a.frame.is_some());
    if matches!(package.source, package::SourceSpec::Document { .. })
        && framed
        && differs_from_defaults(package, params)
    {
        return Err(ApiError::InvalidCommand(format!(
            "placeComponent: {VARIANT_MATE_REFUSAL}"
        )));
    }
    Ok(())
}

/// A configuration's values as free params, with `explicit` layered on top.
///
/// # Errors
/// An unknown configuration (naming the ones the package has), or a stored
/// value that is not a number, string or boolean.
fn configured_params(
    package: &ComponentPackage,
    component_id: &str,
    configuration: &str,
    explicit: BTreeMap<String, ComponentParamValue>,
) -> Result<BTreeMap<String, ComponentParamValue>, ApiError> {
    let values = package.configurations.get(configuration).ok_or_else(|| {
        let known: Vec<&str> = package.configurations.keys().map(String::as_str).collect();
        ApiError::InvalidCommand(format!(
            "placeComponent: {component_id} has no configuration `{configuration}` \
             (available: {known:?})"
        ))
    })?;
    let mut merged = BTreeMap::new();
    for (key, value) in values {
        let value = match value {
            toml::Value::Integer(i) => ComponentParamValue::Number(*i as f64),
            toml::Value::Float(f) => ComponentParamValue::Number(*f),
            toml::Value::String(s) => ComponentParamValue::Text(s.clone()),
            toml::Value::Boolean(b) => ComponentParamValue::Bool(*b),
            other => {
                return Err(ApiError::InvalidCommand(format!(
                    "placeComponent: configuration `{configuration}` of {component_id} sets \
                     `{key}` to {other}, which is not a number, string or boolean"
                )))
            }
        };
        merged.insert(key.clone(), value);
    }
    merged.extend(explicit);
    Ok(merged)
}

/// A component's baked geometry, resolved out of the library and ready to be
/// staged into the placing document (WP-3.2).
#[derive(Debug)]
pub struct StagedBlob {
    sha256: String,
    codec: ImportSourceCodec,
    format: Option<u32>,
    bytes: std::sync::Arc<onecad_core::io::imports::OwnedSource>,
}

/// One host-owned draft payload; bounded by the import blob limit.
pub(crate) struct CapturedDraft {
    workspace: Option<std::sync::Arc<crate::imports::ImportWorkspace>>,
    archive: Option<crate::library_archive::CapturedArchive>,
    source: ComponentSourceRef,
    component_id: String,
    component_version: String,
    component_revision: String,
    source_sha: Option<String>,
    runtime_session: String,
    blob: Option<StagedBlob>,
}

impl CapturedDraft {
    fn matches(&self, params: &PlaceComponentParams) -> bool {
        self.component_id == params.component_id
            && self.component_version == params.component_version
            && self.component_revision == params.component_revision
            && commit_authorization::preview_source_matches(
                &self.source,
                &params.source,
                &params.params,
            )
            && self.source_sha.as_deref() == params.source.blob_ref().map(|b| b.sha256)
    }
}

/// Retains preview input files until the admitted worker operation terminates.
pub(crate) fn captured_component_draft_workspace(
    state: &AppState,
    operation: &Operation,
    session: &str,
) -> Option<std::sync::Arc<crate::imports::ImportWorkspace>> {
    let Operation::Known(KnownOperation::PlaceComponent(params)) = operation else {
        return None;
    };
    let guard = state.captured_library_draft.lock().unwrap();
    let draft = guard.as_ref()?;
    if draft.runtime_session != session || !draft.matches(params) {
        return None;
    }
    draft.workspace.clone()
}

pub(crate) fn component_commit_guard<'a>(
    state: &'a AppState,
    commands: &[EditCommand],
    session: &str,
    authorization: Option<&commit_authorization::CommitAuthorization>,
) -> Result<std::sync::MutexGuard<'a, Option<CapturedDraft>>, ApiError> {
    let guard = state.captured_library_draft.lock().unwrap();
    commit_authorization::check(state, commands, session, guard.as_ref(), authorization)?;
    Ok(guard)
}

impl CapturedDraft {
    pub(crate) fn archive_for_commit(
        &self,
        rt: &DocumentRuntime,
    ) -> Result<crate::library_archive::CapturedArchive, ApiError> {
        let archive = self
            .archive
            .as_ref()
            .ok_or_else(|| ApiError::StalePreview("component archive already committed".into()))?;
        crate::library_archive::select_archive(rt, archive)
    }

    pub(crate) fn stage_for_commit(
        &self,
        rt: &mut DocumentRuntime,
        archive: &crate::library_archive::CapturedArchive,
    ) -> Result<(), ApiError> {
        if let Some(blob) = &self.blob {
            blob.stage_into(rt)?;
        }
        rt.stage_library_archive(archive)
    }

    pub(crate) fn release_committed_bytes(&mut self) {
        self.blob = None;
        self.archive = None;
    }
}

impl StagedBlob {
    /// Stages the bytes into `rt`'s carrier; callers hold the runtime lock and
    /// do this BEFORE the edit that names the blob.
    pub(crate) fn stage_into(&self, rt: &mut DocumentRuntime) -> Result<(), ApiError> {
        rt.stage_component_blob_shared(&self.sha256, self.codec, self.bytes.clone())
    }
}

/// The numeric defaults a package DECLARES for its free parameters, in the
/// record's own param domain.
///
/// A `profile` source's `params` are a REGEN INPUT — unlike every other kind's
/// — so they must be COMPLETE at authoring: `params.length` is required and
/// there is no library lookup at regen to fall back on. The package's
/// `[parameters]` table is where that number lives (a 500 mm vendor stick
/// declares `length = 500`), and the caller's free params override it.
///
/// Numbers only: `ParameterSpec::number()` is the whole accessor this crate has,
/// because the package format's `toml::Value` is deliberately not a dependency
/// here.
fn declared_numeric_params(
    specs: &BTreeMap<String, ParameterSpec>,
) -> BTreeMap<String, ComponentParamValue> {
    specs
        .iter()
        .filter(|(_, spec)| spec.role == ParameterRole::Free)
        .filter_map(|(key, spec)| {
            spec.number()
                .map(|v| (key.clone(), ComponentParamValue::Number(v)))
        })
        .collect()
}

/// Translates a library resolution into the record's `source` plus, for the
/// blob-backed kinds, the bytes the placing document must cache.
///
/// This is the type-family bridge this module exists for: `onecad-library` owns
/// the package format, `onecad-core` owns the wire/record form, and neither
/// depends on the other.
///
/// `declared` is the package's own default parameter set
/// ([`declared_numeric_params`]); only a `profile` source reads it, because it
/// is the only kind whose `params` regen actually consumes.
fn component_source_from_resolved(
    resolved: ResolvedSource,
    component_id: &str,
    declared: &BTreeMap<String, ComponentParamValue>,
    retention: Option<crate::resource_admission::Retention>,
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
            let staged = staged_blob(blob, component_id, retention)?;
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
        // WP-C: the blob is a canonical planar FACE, not a solid, and the
        // placement carries the length the worker prisms it to. That number
        // starts at the package's declared default and is overridden by the
        // gesture's free params (`source_with_free_params`).
        ResolvedSource::Profile { blob } => {
            let staged = staged_blob(blob, component_id, retention)?;
            Ok((
                ComponentSourceRef::Profile {
                    sha256: staged.sha256.clone(),
                    codec: staged.codec,
                    brep_format: staged.format,
                    params: declared.clone(),
                    extra: Default::default(),
                },
                Some(staged),
            ))
        }
        ResolvedSource::Document {
            document_sha256,
            blob,
        } => {
            let staged = staged_blob(blob, component_id, retention)?;
            Ok((
                ComponentSourceRef::Document {
                    document_sha256,
                    sha256: staged.sha256.clone(),
                    codec: staged.codec,
                    brep_format: staged.format,
                    // A placement records the package's own defaults, i.e. the
                    // values its frozen document already builds — so an empty
                    // map IS the configuration. `set_component_params_at` fills
                    // this in when the first override re-bakes (WP-F1.3).
                    params: Default::default(),
                    extra: Default::default(),
                },
                Some(staged),
            ))
        }
    }
}

fn staged_blob(
    blob: ResolvedBlob,
    component_id: &str,
    retention: Option<crate::resource_admission::Retention>,
) -> Result<StagedBlob, ApiError> {
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
        bytes: std::sync::Arc::new(match retention {
            Some(guard) => onecad_core::io::imports::OwnedSource::tracked(blob.bytes, guard),
            None => onecad_core::io::imports::OwnedSource::untracked(blob.bytes),
        }),
    })
}

pub(crate) fn commit_component_archives(
    rt: &mut DocumentRuntime,
    blobs: Vec<StagedBlob>,
    archives: Vec<crate::library_archive::CapturedArchive>,
    mut commands: Vec<EditCommand>,
    label: &str,
) -> Result<CommandOutcome, ApiError> {
    let mut selected: Vec<crate::library_archive::CapturedArchive> = Vec::new();
    for incoming in &archives {
        let archive = if let Some(existing) = selected
            .iter()
            .find(|archive| archive.revision == incoming.revision)
        {
            if existing.reference.component_id != incoming.reference.component_id
                || existing.reference.component_version != incoming.reference.component_version
            {
                return Err(ApiError::InvalidCommand(
                    "component archive identity mismatch".into(),
                ));
            }
            existing.clone()
        } else {
            crate::library_archive::select_archive(rt, incoming)?
        };
        selected.push(archive);
    }
    let refs: Vec<_> = selected.iter().collect();
    let metadata = crate::library_archive::metadata_command(rt, &refs)?;
    let old_imports = rt.import_blob_shas();
    let old_archives = rt.library_archive_checkpoint();
    for blob in blobs {
        if let Err(error) = blob.stage_into(rt) {
            rt.discard_uncommitted_component_blobs(&old_imports);
            return Err(error);
        }
    }
    for archive in &selected {
        if let Err(error) = rt.stage_library_archive(archive) {
            rt.discard_uncommitted_component_blobs(&old_imports);
            rt.restore_library_archive_checkpoint(&old_archives);
            return Err(error);
        }
    }
    commands.push(metadata);
    match rt.apply_transaction(label, commands) {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            rt.discard_uncommitted_component_blobs(&old_imports);
            rt.restore_library_archive_checkpoint(&old_archives);
            Err(error.into())
        }
    }
}

async fn stage_archive_and_apply(
    state: &AppState,
    blob: Option<StagedBlob>,
    archive: crate::library_archive::CapturedArchive,
    command: EditCommand,
    expected_session: &str,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("library".into()))?;
    require_runtime_session(rt, expected_session)?;
    let outcome = commit_component_archives(
        rt,
        blob.into_iter().collect(),
        vec![archive],
        vec![command],
        "Component",
    )?;
    Ok((outcome, rt.projection()))
}

/// Stages a component's geometry blob into the open document and applies `command`
/// under the SAME runtime lock.
///
/// One lock, in this order, on purpose: the blob must be in the carrier and
/// materialized for the worker BEFORE any regen can see the record that names it
/// (the `add_import_record` discipline). Splitting the two would leave a window
/// where an autosave writes a record whose blob is not yet staged.
#[cfg(test)]
async fn stage_blob_and_apply(
    state: &AppState,
    blob: Option<StagedBlob>,
    command: EditCommand,
    expected_session: &str,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("library".into()))?;
    require_runtime_session(rt, expected_session)?;
    if let Some(b) = blob {
        b.stage_into(rt)?;
    }
    let outcome = rt.apply(command)?;
    Ok((outcome, rt.projection()))
}

/// [`stage_blob_and_apply`] that first re-reads `record` under the runtime lock
/// and refuses with [`ApiError::StalePreview`] unless it still equals `expected`.
#[cfg(test)]
async fn stage_blob_and_apply_unchanged(
    state: &AppState,
    blob: Option<StagedBlob>,
    command: EditCommand,
    (record, expected): (RecordId, &PlaceComponentParams),
    expected_session: &str,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("library".into()))?;
    require_runtime_session(rt, expected_session)?;
    let live = rt
        .operation_params(record)
        .and_then(|raw| serde_json::from_value::<PlaceComponentParams>(raw).ok());
    if live.as_ref() != Some(expected) {
        return Err(ApiError::StalePreview(format!(
            "record {record} changed while its component was being re-baked — retry"
        )));
    }
    if let Some(b) = blob {
        b.stage_into(rt)?;
    }
    let outcome = rt.apply(command)?;
    Ok((outcome, rt.projection()))
}

/// `params` carries the free-parameter overrides chosen during the gesture —
/// auto-size on a hole rim today (WP-A3), the configurator's own values later.
/// Absent ⇒ the generator's declared defaults, byte-identical to every prior
/// caller.
#[allow(clippy::too_many_arguments)]
pub async fn place_component(
    app: HostContext,
    state: &AppState,
    component_id: String,
    component_version: String,
    translate: [f64; 3],
    rotate: Option<TransformRotation>,
    params: Option<BTreeMap<String, ComponentParamValue>>,
    mate: Option<PlaceComponentMateInput>,
) -> Result<DocumentProjection, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("placeComponent: no app data dir".into()))?;
    let (outcome, projection) = place_component_at(
        &root,
        state,
        component_id,
        component_version,
        translate,
        rotate,
        params.unwrap_or_default(),
        mate,
    )
    .await?;
    finish(&app, state, &outcome, &projection);
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
    state: &AppState,
    component_id: String,
    component_version: String,
) -> Result<ComponentSourceRef, ApiError> {
    let runtime_session = current_runtime_session(state, "resolveComponentSource").await?;
    let admitted = admitted_component_snapshot_at(
        "resolveComponentSource",
        root,
        &component_id,
        &component_version,
        None,
        &state.archive_budget(),
    )?;
    let snapshot = admitted.snapshot;
    let declared = declared_numeric_params(&snapshot.package.parameters);
    let (source, blob) = component_source_from_resolved(
        snapshot.source,
        &component_id,
        &declared,
        Some(admitted.retention),
    )?;
    if let Some(blob) = blob {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("resolveComponentSource".into()))?;
        require_runtime_session(rt, &runtime_session)?;
        blob.stage_into(rt)?;
    }
    Ok(source)
}

pub async fn resolve_component_source(
    app: HostContext,
    state: &AppState,
    component_id: String,
    component_version: String,
) -> Result<ComponentSourceRef, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("resolveComponentSource: no app data dir".into()))?;
    resolve_component_source_at(&root, state, component_id, component_version).await
}

/// Loads a package's full `component.toml` (parameters included) by identity
/// — the index only carries `parameter_keys` (names, WP-1.4/1.5), never the
/// `role`/`domain` shape `set_component_params_at` needs to enforce spec
/// §3.2's role=free rule.
#[cfg(test)]
pub(crate) fn component_package_at(
    what: &str,
    root: &Path,
    component_id: &str,
    component_version: &str,
) -> Result<ComponentPackage, ApiError> {
    Ok(component_snapshot_at(what, root, component_id, component_version, None)?.package)
}

#[cfg(test)]
pub(crate) fn component_snapshot_at(
    what: &str,
    root: &Path,
    id: &str,
    version: &str,
    revision: Option<&str>,
) -> Result<ResolvedSnapshot, ApiError> {
    component_snapshot_at_bounded(
        what,
        root,
        id,
        version,
        revision,
        onecad_library::blob::MAX_BLOB_BYTES,
    )
    .map_err(ApiError::from)
}

pub(crate) struct AdmittedSnapshot {
    pub(crate) snapshot: ResolvedSnapshot,
    pub(crate) retention: crate::resource_admission::Retention,
}

pub(crate) fn admitted_component_snapshot_at(
    what: &str,
    root: &Path,
    id: &str,
    version: &str,
    revision: Option<&str>,
    budget: &crate::resource_admission::ArchiveBudget,
) -> Result<AdmittedSnapshot, ApiError> {
    let retention = budget
        .reserve(
            MAX_DRAFT_PAYLOAD_PART as usize,
            crate::resource_admission::ArchiveUse::Scratch,
        )
        .map_err(ApiError::Io)?;
    let snapshot =
        component_snapshot_at_bounded(what, root, id, version, revision, MAX_DRAFT_PAYLOAD_PART)
            .map_err(ApiError::from)?;
    Ok(AdmittedSnapshot {
        snapshot,
        retention,
    })
}

fn component_snapshot_at_bounded(
    _what: &str,
    root: &Path,
    id: &str,
    version: &str,
    revision: Option<&str>,
    limit: u64,
) -> onecad_library::LibraryResult<ResolvedSnapshot> {
    let library = Library::open(root)?;
    let (_, entry) = library
        .get(id, Some(version))
        .ok_or_else(|| onecad_library::LibraryError::NotFound(format!("{id}@{version}")))?;
    library.resolve_snapshot_bounded(
        &ResolveRequest {
            component_id: id,
            component_version: version,
            component_revision: revision.unwrap_or(&entry.revision),
        },
        limit,
    )
}

// ── The re-bake lane (WP-F1.3, spec §3.2 + §13.3 deviation 3) ───────────────

/// How long a re-bake waits for its own worker to finish the OCW1 handshake.
/// Generous: this is a cold process start plus OCCT's own init, on a machine
/// already running the document's worker.
const REBAKE_WORKER_READY_TIMEOUT: Duration = Duration::from_secs(20);

/// Owns the re-bake's OWN worker process and retires it on **every** exit path
/// — the `?` early returns included, which is the whole reason it is a guard
/// rather than a call at the end of the happy path. A leaked sidecar would
/// outlive the edit and hold an OCCT process for the rest of the session.
///
/// This manager is never installed into [`AppState`]'s `live`/`warm` slots, gets
/// no restart hook and no status forwarder: it is not the document's worker, and
/// the open document's session, fencing tokens and epoch never see it.
struct EphemeralWorker(WorkerManager);

impl Drop for EphemeralWorker {
    fn drop(&mut self) {
        self.0.retire();
    }
}

/// The frozen `source.onecad` bytes a re-bake replays, **revision-verified**.
///
/// A package that changed underneath this instance is refused with the same
/// typed [`onecad_library::LibraryError`] resolution already produces (absent /
/// `RevisionMismatch`). The refusal is the honest answer and costs nothing: the
/// instance's baked geometry is cached in the document's own `imports/` section,
/// so the document still renders exactly as before — only the EDIT fails.
fn frozen_source_document<'a>(
    document: Option<&'a [u8]>,
    current: &PlaceComponentParams,
    recorded_document_sha256: &str,
) -> Result<&'a [u8], ApiError> {
    let bytes = document.ok_or_else(|| {
        ApiError::InvalidCommand(format!(
            "setComponentParams: {} is not a document-source component",
            current.component_id
        ))
    })?;
    let actual = onecad_library::blob::sha256_hex(bytes);
    if actual != recorded_document_sha256 {
        return Err(ApiError::InvalidCommand(format!(
            "setComponentParams: {} carries a frozen document hashing to {actual}, but this \
             instance was placed against {recorded_document_sha256} — re-baking it would build \
             different geometry under the same identity",
            current.component_id
        )));
    }
    Ok(bytes)
}

/// `(variable name, value)` for every override, resolved through each
/// parameter's own [`package::ParameterSpec::key`].
///
/// A free parameter with no `key` is a package the re-bake cannot honor: nothing
/// says WHICH variable it drives. Refused loudly, naming the parameter — guessing
/// (first variable, same-name variable) is exactly the silent mis-bind spec §0
/// invariant 4 forbids.
fn variable_overrides(
    package: &ComponentPackage,
    component_id: &str,
    params: &BTreeMap<String, ComponentParamValue>,
) -> Result<Vec<(String, f64)>, ApiError> {
    let mut out = Vec::with_capacity(params.len());
    for (name, value) in params {
        // `validate_component_params` already proved the key exists and is `role = free`.
        let spec = package.parameters.get(name).ok_or_else(|| {
            ApiError::Internal(format!(
                "setComponentParams: parameter `{name}` vanished from {component_id}"
            ))
        })?;
        let variable = spec.key.as_ref().ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "setComponentParams: free parameter `{name}` on {component_id} names no source \
                 variable (`key`), so a re-bake cannot know what to set"
            ))
        })?;
        let ComponentParamValue::Number(n) = value else {
            return Err(ApiError::InvalidCommand(format!(
                "setComponentParams: parameter `{name}` on {component_id} drives variable \
                 `{variable}`, which is a number — got {value:?}"
            )));
        };
        out.push((variable.clone(), *n));
    }
    Ok(out)
}

/// Replays a `document` component's frozen source with `merged`'s variable
/// overrides and re-bakes its solid (spec §3.1's "replays the document with
/// overrides"; WP-F1.3). Returns the new blob, ready to stage into the OPEN
/// document.
///
/// **On its own worker, over its own scratch [`DocumentRuntime`].** The worker
/// is one session per process, so replaying a second document needs a second
/// process; running it on the open document's worker would trample the session
/// the user is editing. The scratch runtime is never published, never locked
/// into [`AppState`], and is dropped with the worker.
async fn rebake_document_component(
    resources: (&WorkerBackend, &crate::resource_admission::ArchiveBudget),
    package: &ComponentPackage,
    document: Option<&[u8]>,
    current: &PlaceComponentParams,
    recorded_document_sha256: &str,
    merged: &BTreeMap<String, ComponentParamValue>,
    geometry_limit: u64,
) -> Result<StagedBlob, ApiError> {
    let (backend, budget) = resources;
    let overrides = variable_overrides(package, &current.component_id, merged)?;
    let frozen = frozen_source_document(document, current, recorded_document_sha256)?;

    // Scratch alongside the authoring bake's (same process-scoped temp root, same
    // "worker hand-off material, not user data" status). Swept on every path.
    let scratch = std::env::temp_dir().join(format!(
        "onecad-rebake-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&scratch)
        .map_err(|e| ApiError::Internal(format!("setComponentParams: scratch dir: {e}")))?;
    let document_path = scratch.join("source.onecad");
    let geometry_path = scratch.join("geometry.xbf");
    let staged = match std::fs::write(&document_path, frozen) {
        Ok(()) => {
            replay_and_bake(
                budget,
                backend,
                &document_path,
                &geometry_path,
                &overrides,
                &current.component_id,
                geometry_limit,
            )
            .await
        }
        Err(e) => Err(ApiError::Io(format!(
            "setComponentParams: write frozen source to scratch: {e}"
        ))),
    };
    let _ = std::fs::remove_dir_all(&scratch);
    staged
}

/// The worker-facing half of [`rebake_document_component`], split out so the
/// scratch directory is swept whatever this returns.
async fn replay_and_bake(
    budget: &crate::resource_admission::ArchiveBudget,
    backend: &WorkerBackend,
    document_path: &Path,
    geometry_path: &Path,
    overrides: &[(String, f64)],
    component_id: &str,
    geometry_limit: u64,
) -> Result<StagedBlob, ApiError> {
    let worker = EphemeralWorker(backend.spawn(WorkerRole::Rebake).ok_or_else(|| {
        ApiError::Internal(
            "setComponentParams: no geometry worker binary resolved — a re-bake needs one".into(),
        )
    })?);
    if !worker.0.wait_ready(REBAKE_WORKER_READY_TIMEOUT).await {
        return Err(ApiError::Worker(format!(
            "setComponentParams: the re-bake worker for {component_id} never became ready"
        )));
    }

    let engine: Arc<dyn GeometryEngine> = Arc::new(worker.0.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(worker.0.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(worker.0.clone());
    let mut rt = DocumentRuntime::open_with_budget(
        document_path,
        engine,
        meshes,
        solver,
        false,
        budget.clone(),
    )
    .map_err(|e| {
        ApiError::Io(format!(
            "setComponentParams: reopen {component_id}'s frozen source document: {e}"
        ))
    })?;

    for (variable, value) in overrides {
        let id = rt
            .variables()
            .iter()
            .find(|v| &v.name == variable)
            .map(|v| v.id)
            .ok_or_else(|| {
                ApiError::InvalidCommand(format!(
                    "setComponentParams: {component_id} binds a parameter to variable \
                     `{variable}`, which its source document does not declare"
                ))
            })?;
        let value = Scalar::try_new(*value).ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "setComponentParams: variable `{variable}` must be finite"
            ))
        })?;
        rt.apply(EditCommand::SetVariable {
            variable: id,
            value,
        })?;
    }

    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    match &report.outcome {
        Outcome::Published(_) => {}
        other => {
            return Err(ApiError::OpFailed {
                message: format!(
                    "setComponentParams: re-baking {component_id} did not publish ({other:?})"
                ),
                diagnostics: Vec::new(),
            })
        }
    }
    if let Some(failed) = report.failed_steps.first() {
        return Err(ApiError::OpFailed {
            message: format!(
                "setComponentParams: re-baking {component_id} failed at step {}: {}",
                failed.record_id, failed.message
            ),
            diagnostics: failed.diagnostics.clone(),
        });
    }

    // EXACTLY one solid, and never a silent fuse: the author already answered
    // the multi-solid question at save time (`unionSolids`), and re-deciding it
    // here would change what the component IS behind their back. A document
    // whose new parameter value splits the body is a refusal, not a repair.
    let bodies = rt.head_body_ids();
    let [body] = bodies.as_slice() else {
        return Err(ApiError::InvalidCommand(format!(
            "setComponentParams: re-baking {component_id} produced {} bodies; a component is \
             exactly one solid (spec §9) — the value cannot be applied",
            bodies.len()
        )));
    };
    let baked = GeometryExporter::export_geometry(
        &worker.0,
        &geometry_path.to_string_lossy(),
        &[*body],
        "xbf",
        false,
        // WP-H fence (SCHEMA §7.8): the bake writes the head the regen above just
        // published. A head that moved under it would bake a component out of
        // geometry the caller never validated — refused, not silently written.
        crate::export::export_fence(&rt),
    )
    .await?;
    if baked.solid_count != 1 {
        return Err(ApiError::InvalidCommand(format!(
            "setComponentParams: re-baking {component_id} produced {} solids; a component is \
             exactly one solid (spec §9) — the value cannot be applied",
            baked.solid_count
        )));
    }

    use std::io::Read;
    let mut file = std::fs::File::open(geometry_path).map_err(|e| {
        ApiError::Io(format!(
            "setComponentParams: read re-baked geometry for {component_id}: {e}"
        ))
    })?;
    let size = file
        .metadata()
        .map_err(|e| ApiError::Io(e.to_string()))?
        .len();
    if size > geometry_limit || usize::try_from(size).is_err() {
        return Err(ApiError::InvalidCommand(
            "re-baked component geometry exceeds the draft source limit".into(),
        ));
    }
    let retention = budget
        .reserve(
            size as usize,
            crate::resource_admission::ArchiveUse::Scratch,
        )
        .map_err(ApiError::Io)?;
    let mut bytes = vec![0; size as usize].into_boxed_slice();
    file.read_exact(&mut bytes)
        .map_err(|e| ApiError::Io(e.to_string()))?;
    if file
        .read(&mut [0; 1])
        .map_err(|e| ApiError::Io(e.to_string()))?
        != 0
    {
        return Err(ApiError::InvalidCommand(
            "re-baked component geometry changed during read".into(),
        ));
    }
    let bytes = bytes.into_vec();
    let codec = ImportSourceCodec::from_extension(&baked.codec).ok_or_else(|| {
        ApiError::Internal(format!(
            "setComponentParams: the worker baked {component_id} in unknown codec `{}`",
            baked.codec
        ))
    })?;
    Ok(StagedBlob {
        sha256: crate::imports::sha256_hex(&bytes),
        codec,
        format: Some(baked.format),
        bytes: std::sync::Arc::new(onecad_core::io::imports::OwnedSource::tracked(
            bytes, retention,
        )),
    })
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
pub async fn set_component_params_at(
    root: &Path,
    state: &AppState,
    record_id: String,
    params: BTreeMap<String, ComponentParamValue>,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    set_component_params_removing_at(root, state, record_id, params, &[]).await
}

/// [`set_component_params_at`] that also drops `removed` override keys before
/// merging `params` — the only way an override leaves a record (reset to default).
pub(crate) async fn set_component_params_removing_at(
    root: &Path,
    state: &AppState,
    record_id: String,
    params: BTreeMap<String, ComponentParamValue>,
    removed: &[String],
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let record = RecordId::from_str(&record_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {record_id:?}: {e}")))?;

    let (current, runtime_session, archived) = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("setComponentParams".into()))?;
        let raw = rt.operation_params(record).ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "setComponentParams: no params for record {record_id}"
            ))
        })?;
        let current = serde_json::from_value::<PlaceComponentParams>(raw).map_err(|e| {
            ApiError::InvalidCommand(format!(
                "setComponentParams: record {record_id} is not a placed component: {e}"
            ))
        })?;
        let archived = rt.captured_library_archive(
            &current.component_id,
            &current.component_version,
            &current.component_revision,
        )?;
        (current, rt.runtime_session(), archived)
    };

    let (snapshot, _decode_retention) = if let Some((entry, bytes)) = archived {
        let (snapshot, guard) = crate::library_archive::decode(
            &entry,
            &current.component_revision,
            &bytes,
            &state.archive_budget(),
        )?;
        (snapshot, Some(guard))
    } else {
        let admitted = admitted_component_snapshot_at(
            "setComponentParams",
            root,
            &current.component_id,
            &current.component_version,
            Some(&current.component_revision),
            &state.archive_budget(),
        )?;
        (admitted.snapshot, Some(admitted.retention))
    };
    let archive = crate::library_archive::capture(
        &snapshot,
        MAX_CAPTURED_DRAFT_BYTES,
        &state.archive_budget(),
    )?;
    let package = &snapshot.package;
    validate_component_params(
        "setComponentParams",
        package,
        &current.component_id,
        &params,
    )?;

    let mut merged = current.params.clone();
    merged.retain(|key, _| !removed.contains(key));
    merged.extend(params);

    let (source, blob) = match &current.source {
        ComponentSourceRef::Generator {
            generator_id,
            generator_version,
            extra,
            ..
        } => (
            ComponentSourceRef::Generator {
                generator_id: generator_id.clone(),
                generator_version: *generator_version,
                params: merged.clone(),
                extra: extra.clone(),
            },
            None,
        ),
        ComponentSourceRef::Embedded { .. } => {
            return Err(ApiError::InvalidCommand(
                "setComponentParams: an embedded-source placement has no free parameters".into(),
            ))
        }
        // WP-C: a `profile` source's `params` ARE the regen input — the worker
        // re-prisms the SAME canonical face to the new `length` — so the edit is
        // a plain in-place merge with NO re-bake. There is no authoring document
        // to replay and the blob does not move, which is why this arm stages
        // nothing: the planner hash covers `source.params`, so the value change
        // alone is what moves the geometry.
        ComponentSourceRef::Profile {
            sha256,
            codec,
            brep_format,
            params: source_params,
            extra,
        } => {
            let mut source_params = source_params.clone();
            // A removed key falls back to the package default, never to absence:
            // a profile's `length` is a required regen input.
            source_params.retain(|key, _| !removed.contains(key));
            source_params.extend(
                declared_numeric_params(&package.parameters)
                    .into_iter()
                    .filter(|(key, _)| removed.contains(key)),
            );
            source_params.extend(merged.clone());
            (
                ComponentSourceRef::Profile {
                    sha256: sha256.clone(),
                    codec: *codec,
                    brep_format: *brep_format,
                    params: source_params,
                    extra: extra.clone(),
                },
                None,
            )
        }
        // A `document` component's geometry is a solid BAKED from its authoring
        // document (WP-3.2), so an override is honored by RE-BAKING: replay the
        // frozen source with the new variable values and swap in the solid it
        // produces (WP-F1.3). Merging the value into the record without that
        // would show a new designation over unchanged geometry — the silent
        // substitution spec §0 invariant #4 exists to prevent.
        ComponentSourceRef::Document {
            document_sha256,
            extra,
            ..
        } => {
            // The mate's frozen frame was captured on the default bake; a
            // variant has different geometry (see `refuse_variant_mate`).
            if current
                .mate
                .as_ref()
                .is_some_and(|m| m.self_frame.is_some())
                && differs_from_defaults(package, &merged)
            {
                return Err(ApiError::InvalidCommand(format!(
                    "setComponentParams: {VARIANT_MATE_REFUSAL}; re-place it unmated or reset \
                     to the defaults"
                )));
            }
            let staged = rebake_document_component(
                (state.services().backend(), &state.archive_budget()),
                &snapshot.package,
                snapshot.document.as_deref(),
                &current,
                document_sha256.as_str(),
                &merged,
                MAX_DRAFT_PAYLOAD_PART,
            )
            .await?;
            (
                ComponentSourceRef::Document {
                    document_sha256: document_sha256.clone(),
                    sha256: staged.sha256.clone(),
                    codec: staged.codec,
                    brep_format: staged.format,
                    params: merged.clone(),
                    extra: extra.clone(),
                },
                Some(staged),
            )
        }
    };

    let updated = PlaceComponentParams {
        params: merged,
        source,
        // The package did not change, so neither does the recorded revision: this
        // instance is still an instance of exactly the component it was placed as.
        ..current.clone()
    };
    let op = Operation::Known(KnownOperation::PlaceComponent(updated));

    // ONE undoable edit at the SAME `RecordId`, with the re-baked blob staged
    // under the same runtime lock — the `add_import_record` discipline: no regen
    // may see a record naming a blob that is not yet in the carrier. The record
    // is re-read under that lock: the re-bake awaited a worker with the lock
    // released, and writing a result computed from an older record would
    // silently undo whatever edited it meanwhile.
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("setComponentParams".into()))?;
    require_runtime_session(rt, &runtime_session)?;
    let live = rt
        .operation_params(record)
        .and_then(|raw| serde_json::from_value::<PlaceComponentParams>(raw).ok());
    if live.as_ref() != Some(&current) {
        return Err(ApiError::StalePreview(format!(
            "record {record} changed while its component was being re-baked — retry"
        )));
    }
    let outcome = commit_component_archives(
        rt,
        blob.into_iter().collect(),
        vec![archive],
        vec![EditCommand::UpdateOperationParams { record, op }],
        "Component parameters",
    )?;
    Ok((outcome, rt.projection()))
}

pub async fn set_component_params(
    app: HostContext,
    state: &AppState,
    record_id: String,
    params: BTreeMap<String, ComponentParamValue>,
) -> Result<DocumentProjection, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("setComponentParams: no app data dir".into()))?;
    let (outcome, projection) = set_component_params_at(&root, state, record_id, params).await?;
    finish(&app, state, &outcome, &projection);
    Ok(projection)
}

/// Meshes a library component so the UI can SHOW it (WP-B6) — the card
/// thumbnails and the detail preview in the library browser.
///
/// **Runs with or without an open document.** A preview is a `PlaceComponent`
/// candidate against a throwaway copy of the worker's session head (SCHEMA
/// §7.6 `PreviewOp`), and an empty head is a perfectly good thing to place
/// into — which matters because the most useful place to browse a catalog is
/// the START SCREEN, before any project exists. It therefore picks the open
/// document's worker when there is one and the PRE-WARMED worker otherwise,
/// rather than going through `AppState::preview()` (which is
/// `PendingBackend` until a document opens).
///
/// Nothing is committed, nothing is recorded, and the open document — if any —
/// is untouched: `PreviewOp` never mutates the head it copies.
pub async fn component_preview_mesh(
    app: HostContext,
    state: &AppState,
    component_id: String,
    component_version: String,
    params: Option<BTreeMap<String, ComponentParamValue>>,
) -> Result<crate::dto::PreviewResultDto, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("componentPreviewMesh: no app data dir".into()))?;
    let preview: std::sync::Arc<dyn crate::worker::PreviewEngine> = state
        .live_worker()
        .or_else(|| state.warm_worker())
        .map(|wm| std::sync::Arc::new(wm) as std::sync::Arc<dyn crate::worker::PreviewEngine>)
        .ok_or_else(|| {
            ApiError::Internal(
                "componentPreviewMesh: no geometry worker is running (a preview needs one)".into(),
            )
        })?;
    component_preview_mesh_with_budget(
        &root,
        preview,
        component_id,
        component_version,
        params,
        &state.archive_budget(),
    )
    .await
}

/// Explicit-engine preview lane, usable without an open document.
pub async fn component_preview_mesh_at(
    root: &Path,
    preview: std::sync::Arc<dyn crate::worker::PreviewEngine>,
    component_id: String,
    component_version: String,
    params: Option<BTreeMap<String, ComponentParamValue>>,
) -> Result<crate::dto::PreviewResultDto, ApiError> {
    component_preview_mesh_with_budget(
        root,
        preview,
        component_id,
        component_version,
        params,
        &Default::default(),
    )
    .await
}

async fn component_preview_mesh_with_budget(
    root: &Path,
    preview: std::sync::Arc<dyn crate::worker::PreviewEngine>,
    component_id: String,
    component_version: String,
    params: Option<BTreeMap<String, ComponentParamValue>>,
    budget: &crate::resource_admission::ArchiveBudget,
) -> Result<crate::dto::PreviewResultDto, ApiError> {
    let admitted = admitted_component_snapshot_at(
        "componentPreviewMesh",
        root,
        &component_id,
        &component_version,
        None,
        budget,
    )?;
    let snapshot = admitted.snapshot;
    let revision = snapshot.package.identity.revision.clone();
    let declared = declared_numeric_params(&snapshot.package.parameters);
    let (source, blob) = component_source_from_resolved(
        snapshot.source,
        &component_id,
        &declared,
        Some(admitted.retention),
    )?;
    // Free params select geometry only for the two kinds that re-derive it: a
    // GENERATOR and (WP-C) a PROFILE, whose `length` is the whole point of the
    // preview. A BAKED component previews the solid its package carries, so its
    // params are dropped here rather than refused: since WP-F1.3 a `document`
    // package legitimately declares free params, and the configurator asks for a
    // picture of the component with them — erroring on a thumbnail would be the
    // wrong answer to the right question. This is a catalog preview of the
    // PACKAGE; a placed instance's re-baked geometry lives in the document, not
    // here.
    let source = match source {
        ComponentSourceRef::Generator { .. } | ComponentSourceRef::Profile { .. } => {
            source_with_free_params(source, &params.unwrap_or_default(), &component_id)?
        }
        baked => baked,
    };

    // A blob-backed component's bytes must be readable by the worker before the
    // op is lowered (the wire injects `source.path` from the process-global
    // sha→path registry). With no document open there is no import workspace to
    // stage into, so the preview lane keeps its own.
    if let Some(staged) = blob {
        materialize_for_preview(&staged)?;
    }

    let op = Operation::Known(KnownOperation::PlaceComponent(PlaceComponentParams {
        component_id,
        component_version,
        component_revision: revision,
        params: Default::default(),
        source,
        mate: None,
        // At the origin, unrotated: the viewer frames whatever comes back, and a
        // preview has no target to seat against.
        placement: FrozenPlacement {
            translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
        },
        extra: Default::default(),
    }));

    crate::worker::PreviewEngine::preview_op(
        preview.as_ref(),
        op,
        // A fresh UUID, NOT a decorated string: the preview mints a body id of
        // `body_<opId>` and the adoption check requires that to parse as a
        // UUID, so a "preview_…" prefix is refused at the wire (found by
        // `every_seeded_component_meshes_for_the_library_ui`, not by reading).
        uuid::Uuid::new_v4().to_string(),
        None,
        None,
        onecad_core::regen::Lod::Medium,
    )
    .await
    .map_err(ApiError::from)
}

/// Writes a component blob where the worker can read it, for the preview lane.
///
/// Its own [`crate::imports::ImportWorkspace`], process-global and never torn
/// down: previews repeat constantly (every card, every parameter change) and
/// the store is content-addressed, so re-materializing the same digest is a
/// no-op after the first write.
fn materialize_for_preview(staged: &StagedBlob) -> Result<(), ApiError> {
    use std::sync::{Mutex, OnceLock};
    static WORKSPACE: OnceLock<Mutex<crate::imports::ImportWorkspace>> = OnceLock::new();
    let workspace = WORKSPACE.get_or_init(|| {
        Mutex::new(crate::imports::ImportWorkspace::new(
            onecad_core::ids::DocumentId::new(),
        ))
    });
    let mut guard = workspace.lock().map_err(|_| {
        ApiError::Internal("componentPreviewMesh: preview workspace poisoned".into())
    })?;
    guard
        .materialize(&staged.sha256, staged.codec, &staged.bytes)
        .map_err(|e| ApiError::Internal(format!("componentPreviewMesh: materialize blob: {e}")))?;
    Ok(())
}

// ── Templates (spec §8) ─────────────────────────────────────────────────────

/// A template's PNG thumbnail as a data URL, or `None`.
///
/// Inlined rather than handed over as a path: the webview has zero filesystem
/// capability by design, so a path would be unreadable to it. Bounded by the
/// same cap the document preview uses — a template with an absurd thumbnail
/// still lists, just without one.
fn template_preview_data_url(path: Option<&Path>) -> Option<String> {
    use base64::Engine as _;
    let bytes = std::fs::read(path?).ok()?;
    if bytes.len() > crate::api::MAX_PREVIEW_PNG_BYTES {
        return None;
    }
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn template_dto(entry: &onecad_library::template::TemplateEntry) -> ProjectTemplateDto {
    ProjectTemplateDto {
        id: entry.id.clone(),
        name: entry.name.clone(),
        description: entry.description.clone(),
        preview_data_url: template_preview_data_url(entry.preview_path.as_deref()),
    }
}

fn list_templates_at(root: &Path) -> Vec<ProjectTemplateDto> {
    onecad_library::template::list(root)
        .iter()
        .map(template_dto)
        .collect()
}

/// Every project template in the library (spec §8). An unreadable library root
/// is an empty list, not an error — the start screen must still render.
pub async fn list_templates(app: HostContext) -> Result<Vec<ProjectTemplateDto>, ApiError> {
    let Some(root) = library_root(&app) else {
        return Ok(Vec::new());
    };
    Ok(list_templates_at(&root))
}

/// "Save as Template" (spec §8) — freezes the OPEN document as a starter.
///
/// The document is frozen with the same lock discipline `save_as_component`
/// uses, and for the same reason. Nothing about the open document changes: it
/// keeps its path, its title and its dirty flag, because saving a template is
/// not saving your work.
pub async fn save_as_template(
    app: HostContext,
    state: &AppState,
    id: String,
    name: String,
    description: Option<String>,
    preview_png: Option<String>,
) -> Result<ProjectTemplateDto, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("saveAsTemplate: no app data dir".into()))?;
    let scratch = std::env::temp_dir().join(format!("onecad-template-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)
        .map_err(|e| ApiError::Internal(format!("saveAsTemplate: scratch dir: {e}")))?;
    let frozen = scratch.join("template.onecad");

    let payload = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("saveAsTemplate".into()))?;
        rt.build_save_payload(
            crate::api::save_meta(),
            // Meshes ON: a template is opened cold and its bodies should paint
            // before the from-0 regen finishes, exactly like any other document.
            crate::document_runtime::SaveCaches::explicit(),
        )
    };
    crate::document_runtime::DocumentRuntime::write_payload(&frozen, &payload)
        .map_err(|e| ApiError::Internal(format!("saveAsTemplate: freeze document: {e}")))?;
    let document = std::fs::read(&frozen)
        .map_err(|e| ApiError::Internal(format!("saveAsTemplate: read frozen document: {e}")))?;
    let _ = std::fs::remove_dir_all(&scratch);

    let entry = onecad_library::template::save(
        &root,
        onecad_library::template::NewTemplate {
            id,
            name,
            description,
            document,
            preview_png: crate::api::decode_preview_png(preview_png),
        },
    )?;
    Ok(template_dto(&entry))
}

/// Starts a NEW untitled document from a template (spec §8).
///
/// The template's container is opened like any other document and then
/// DETACHED from its file, so the copy is untitled and dirty and the first Save
/// prompts for a location. That detach is the whole of the template's
/// immutability: nothing marks the file read-only, it simply is never the save
/// target.
pub async fn new_from_template(
    app: HostContext,
    state: &AppState,
    id: String,
) -> Result<crate::dto::DocumentSnapshotDto, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("newFromTemplate: no app data dir".into()))?;
    let entry = onecad_library::template::get(&root, &id).ok_or_else(|| {
        ApiError::InvalidCommand(format!("newFromTemplate: unknown template {id}"))
    })?;

    state.ensure_document_capacity()?;
    let mut rt =
        crate::api::open_runtime_over_new_backend_blocking(state, entry.document_path.clone())
            .await?;
    rt.detach_from_file(&entry.name);
    // The copy keeps the template's identity, so a second untitled copy open in
    // another host is refused rather than sharing its autosave and recovery state.
    let claim = match state.claim_document(rt.document_uuid()) {
        Ok(claim) => claim,
        Err(error) => {
            state.rollback_backend();
            return Err(error);
        }
    };

    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        if let Err(error) = state.ensure_not_closing() {
            drop(guard);
            state.rollback_backend();
            return Err(error);
        }
        crate::api::retire_component_worker(state);
        clear_component_draft_capture(state);
        *guard = Some(rt);
        let rt = guard.as_ref().unwrap();
        rt.adopt_current_epoch();
        state.bind_restart_owner(rt);
        let result = (crate::api::snapshot_of(rt), rt.projection());
        state.commit_backend();
        state.adopt_claims(vec![claim]);
        result
    };
    let _ = app.emit(AppEvent::ProjectionUpdated(projection.clone()));
    // NOT recorded in recents: the user has not saved anything yet, and a
    // recents entry pointing INTO the library would reopen the template itself.
    crate::api::schedule_initial_regen(state).await;
    Ok(snapshot)
}

/// The identity + metadata half of "Save as Component" (spec §7), as the
/// authoring UI supplies it. Everything about GEOMETRY is decided by the
/// backend: which body was baked, in which codec, and where the blob landed.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewComponentSpec {
    /// Namespaced, e.g. `"acme.bracket"`. Validated by the library crate.
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub category: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub designation: Option<String>,
    /// Named mate points the author placed (spec §2.1's `[attachments]`) —
    /// `{ on, accepts }` per key. A component with none can never snap, so the
    /// UI is expected to require at least one; this layer does not, because a
    /// non-snapping component is still a legitimate thing to save.
    #[serde(default)]
    pub attachments: BTreeMap<String, AttachmentSpecInput>,
    /// `role = "free"` parameters the author declared (spec §2.1's
    /// `[parameters]`; WP-F1.3) — each one bound to a VARIABLE of the document
    /// being frozen. A placed instance edits these through
    /// `setComponentParams`, which re-bakes the frozen document with the new
    /// values. Empty ⇒ a component with no editable size, exactly as every
    /// pre-WP-F1.3 authored package.
    #[serde(default)]
    pub parameters: BTreeMap<String, FreeParameterInput>,
    /// Named variants: configuration name -> (free-param key -> value). Every
    /// key must be one of `parameters` and every value must pass the same
    /// checks a placement's params do; the refusal names the configuration.
    #[serde(default)]
    pub configurations: BTreeMap<String, BTreeMap<String, ComponentParamValue>>,
    /// The configuration a picker preselects; must be one of `configurations`.
    #[serde(default)]
    pub default_configuration: Option<String>,
    /// A STATIC save: the package carries no free parameters and no
    /// configurations — a fixed baked solid. Supplying either alongside this
    /// flag is refused rather than silently dropped.
    #[serde(default)]
    pub static_save: bool,
}

/// One declared free parameter (spec §2.1 `[parameters].<key>`, `role = "free"`).
///
/// Numeric only, because a document variable IS a number
/// ([`onecad_core::document::variables::Variable`]) — there is nothing else a
/// re-bake could set.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeParameterInput {
    /// The source document's variable name this parameter drives.
    pub key: String,
    /// Ignored on save: the declared default is the document variable's own
    /// value, read when the document is frozen (the saved DTO reports it).
    pub value: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentSpecInput {
    pub on: String,
    pub accepts: Vec<String>,
    /// The attachment's component-local seating frame (WP-F1.1). Optional —
    /// absent means the component seats at its own model origin, which is what
    /// the authoring UI produces until it grows a frame picker.
    #[serde(default)]
    pub frame: Option<package::AttachmentFrame>,
}

/// The MACHINE-READABLE marker in the multi-solid refusal's message (WP-F1.2).
///
/// `ApiError` carries a `kind` + a prose `message` and no error code, and the
/// authoring dialog has to tell "your body is several solids" (offer the fuse)
/// apart from every other `invalidCommand` (do not). Matching prose would break
/// the moment the sentence is reworded, so the sentence carries a token instead.
/// Frontend counterpart: `MULTI_SOLID_REFUSAL` in `SaveAsComponentDialog.tsx`.
pub const MULTI_SOLID_REFUSAL: &str = "MULTI_SOLID_BODY";

/// "Save as Component" (spec §7) — captures one body at head as a reusable
/// `document`-kind package.
///
/// **The bake is the whole design.** The package stores the frozen authoring
/// document AND a solid baked out of the live session (`ExportGeometry`, SCHEMA
/// §7.8); a placement copies the baked solid into the placing document. Nothing
/// ever replays a frozen document — spec §4 requires a placed component to
/// render with the library deleted, and the worker is one session per process,
/// so a nested replay would need a second worker. The frozen document is
/// provenance and the input a future re-bake lane would replay (WP-3.2).
///
/// **Spec §9's single-solid rule is enforced HERE, at save time**, where the
/// author can still do something about it — not at placement, where they would
/// discover it as a failure on someone else's machine. `ExportGeometry` reports
/// the solid count it actually wrote. `union_solids` is the author's opt-in
/// answer to that refusal (WP-F1.2): the BAKE fuses, so the open document keeps
/// its several bodies and only the component package holds the fused solid.
///
/// The document is frozen WITHOUT adopting a path or clearing the dirty flag:
/// `build_save_payload` + `write_payload` are used directly rather than
/// `DocumentRuntime::save`, which would tell the open document it had been
/// saved to a temp file inside the library.
pub async fn save_as_component(
    app: HostContext,
    state: &AppState,
    body_id: String,
    spec: NewComponentSpec,
    preview_png: Option<String>,
    union_solids: Option<bool>,
) -> Result<LibraryComponentDto, ApiError> {
    state
        .await_library_initialization()
        .await
        .map_err(ApiError::Internal)?;
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("saveAsComponent: no app data dir".into()))?;
    save_as_component_at(
        &root,
        &state.runtime,
        state.exporter(),
        body_id,
        spec,
        preview_png,
        union_solids.unwrap_or(false),
    )
    .await
}

/// A scratch directory owned by one operation and removed when dropped, so no
/// error path (or panic) can leak it and no two operations can share it.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn create(what: &str) -> Result<Self, ApiError> {
        let path = std::env::temp_dir().join(format!(
            "onecad-authoring-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path)
            .map_err(|e| ApiError::Internal(format!("{what}: scratch dir: {e}")))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The package's free parameters, each bound to a variable of the document
/// being frozen. The declared default is the DOCUMENT's value (read under the
/// freeze lock); a client-supplied value is ignored, since a default that
/// disagrees with the frozen document would break "an instance that overrides
/// nothing resolves to exactly the baked solid". A derived variable (one with
/// an expression) cannot be set by a re-bake, so it is refused.
fn document_bound_parameters(
    spec: &NewComponentSpec,
    variables: &BTreeMap<String, (f64, bool)>,
) -> Result<BTreeMap<String, ParameterSpec>, ApiError> {
    let mut parameters = BTreeMap::new();
    for (name, p) in &spec.parameters {
        let Some(&(value, derived)) = variables.get(&p.key) else {
            return Err(ApiError::InvalidCommand(format!(
                "saveAsComponent: parameter `{name}` binds to variable `{}`, which this document \
                 does not declare",
                p.key
            )));
        };
        if derived {
            return Err(ApiError::InvalidCommand(format!(
                "saveAsComponent: parameter `{name}` binds to `{}`, which is derived from an \
                 expression; expose its inputs instead",
                p.key
            )));
        }
        if !value.is_finite() {
            return Err(ApiError::InvalidCommand(format!(
                "saveAsComponent: variable `{}` has a non-finite value",
                p.key
            )));
        }
        parameters.insert(name.clone(), ParameterSpec::free_variable(&p.key, value));
    }
    Ok(parameters)
}

/// A static save (`static_save`) is a fixed baked solid: parameters or
/// configurations supplied with it are a contradiction, refused explicitly.
fn reject_static_save_extras(spec: &NewComponentSpec) -> Result<(), ApiError> {
    if !spec.static_save {
        return Ok(());
    }
    if !spec.parameters.is_empty() {
        return Err(ApiError::InvalidCommand(
            "saveAsComponent: a static save carries no free parameters, but parameters were \
             supplied — drop them, or save parametrically"
                .into(),
        ));
    }
    if !spec.configurations.is_empty() || spec.default_configuration.is_some() {
        return Err(ApiError::InvalidCommand(
            "saveAsComponent: a static save carries no configurations (they set free \
             parameters, which a static save does not have)"
                .into(),
        ));
    }
    Ok(())
}

/// The package's `[configurations]`, validated against the free parameters it
/// is about to declare: names, non-empty, declared keys, and per-value checks
/// (kind, finite, domain, minimum) — each refusal names the configuration.
fn authored_configurations(
    spec: &NewComponentSpec,
    parameters: &BTreeMap<String, ParameterSpec>,
) -> Result<package::Configurations, ApiError> {
    let refuse = |msg: String| Err(ApiError::InvalidCommand(format!("saveAsComponent: {msg}")));
    let mut out = package::Configurations::new();
    for (name, values) in &spec.configurations {
        if !package::is_valid_configuration_name(name) {
            return refuse(format!(
                "configuration `{name}` has an invalid name (1..={} characters of \
                 [A-Za-z0-9 _.-])",
                package::MAX_CONFIGURATION_NAME_LEN
            ));
        }
        if values.is_empty() {
            return refuse(format!("configuration `{name}` sets no parameters"));
        }
        let what = format!("saveAsComponent: configuration `{name}`");
        let mut table = BTreeMap::new();
        for (key, value) in values {
            let Some(param) = parameters.get(key) else {
                return refuse(format!(
                    "configuration `{name}` sets `{key}`, which is not a free parameter of \
                     this component"
                ));
            };
            validate_param_value(&what, &spec.id, key, param, value)?;
            table.insert(key.clone(), param_value_to_toml(value));
        }
        out.insert(name.clone(), table);
    }
    match &spec.default_configuration {
        Some(d) if !out.contains_key(d) => refuse(format!(
            "default configuration `{d}` is not one of the configurations"
        )),
        _ => Ok(out),
    }
}

fn param_value_to_toml(value: &ComponentParamValue) -> toml::Value {
    match value {
        ComponentParamValue::Number(n) => toml::Value::Float(*n),
        ComponentParamValue::Text(t) => toml::Value::String(t.clone()),
        ComponentParamValue::Bool(b) => toml::Value::Boolean(*b),
    }
}

/// An attachment derived from a pick on the source body (Create Component).
///
/// Classifies `topo_key` on `body_id` at head and builds the seating frame in
/// the SOURCE DOCUMENT's world frame — which is also the baked solid's local
/// frame, because the bake (`ExportGeometry`) applies no re-centring or
/// transform. Frame rules:
///
/// * planar face: `accepts = ["plane"]`, `z` = the face's outward normal
///   (orientation-aware, SCHEMA §7.5), `origin` = the face frame origin;
/// * cylindrical face: `accepts = ["cylinder","hole","circularEdge"]`, `origin`
///   = a point on the axis, `z` = the parametric axis (which has no intrinsic
///   sign). Sidedness, when measured, orients it: a `hole` keeps the axis as
///   given, a `pin` (boss/shaft) takes the reversed axis so its `z` opposes
///   the axis of the hole it will seat into; unmeasured keeps the axis as given;
/// * circular edge: same `accepts`, `origin` = circle centre, `z` = circle axis.
///
/// `body_id` is the bare uuid `saveAsComponent` takes or the wire `body_<uuid>`.
///
/// `x` is world X projected perpendicular to `z`, else world Y when `z` is
/// parallel to X — deterministic, and only fixes roll. `on` is
/// `face:<name>` | `cylinder:<name>` | `edge:<name>`.
///
/// The read is fenced to `fence` (the pick's document/session/snapshot): a head
/// that moved is refused as [`ApiError::StalePreview`], never read at the new head.
///
/// # Errors
/// A pick that no longer resolves, or is not a planar face, a cylinder or a
/// circular edge ("attachments need a planar face, a cylinder or a circular
/// edge").
pub async fn attachment_from_pick(
    state: &AppState,
    body_id: String,
    topo_key: String,
    name: String,
    fence: &crate::dto::GeometryReadFenceDto,
) -> Result<AttachmentSpecInput, ApiError> {
    let refusal = || {
        ApiError::InvalidCommand(
            "attachmentFromPick: attachments need a planar face, a cylinder or a circular edge"
                .into(),
        )
    };
    if name.trim().is_empty() {
        return Err(ApiError::InvalidCommand(
            "attachmentFromPick: an attachment needs a name".into(),
        ));
    }
    // `saveAsComponent` takes a bare uuid body id, `classifyElement` the wire
    // `body_<uuid>` form; accept either so one id serves both calls.
    let wire_id = if body_id.starts_with("body_") {
        body_id
    } else {
        format!("body_{body_id}")
    };
    let info = crate::api::classify_element(
        state,
        wire_id,
        String::new(),
        Some(topo_key),
        Some(fence.clone()),
    )
    .await?
    .ok_or_else(|| {
        ApiError::InvalidCommand("attachmentFromPick: the pick no longer resolves".into())
    })?;
    let frame = info.frame.as_ref().ok_or_else(refusal)?;
    let cylinder_accepts = || {
        ["cylinder", "hole", "circularEdge"]
            .map(String::from)
            .to_vec()
    };
    let (kind, accepts, z) = match (info.kind.as_str(), info.surface_type.as_str()) {
        ("face", "plane") => ("face", vec!["plane".to_string()], frame.normal),
        ("face", "cylinder") => {
            let axis = frame.axis.map(|a| match frame.sidedness.as_deref() {
                Some("pin") => [-a[0], -a[1], -a[2]],
                _ => a,
            });
            ("cylinder", cylinder_accepts(), axis)
        }
        ("edge", _) if info.curve_type == "circle" => ("edge", cylinder_accepts(), frame.axis),
        _ => return Err(refusal()),
    };
    let z = z.ok_or_else(refusal)?;
    let seat = package::AttachmentFrame {
        origin: frame.origin,
        z,
        x: [1.0, 0.0, 0.0],
    };
    let seat = seat_frame(seat).ok_or_else(refusal)?;
    Ok(AttachmentSpecInput {
        on: format!("{kind}:{}", name.trim()),
        accepts,
        frame: Some(seat),
    })
}

/// `frame` with `x` replaced by world X projected perpendicular to `z`, or by
/// world Y when `z` is (nearly) parallel to X. `None` for a degenerate `z`.
fn seat_frame(frame: package::AttachmentFrame) -> Option<package::AttachmentFrame> {
    let len = frame.z.iter().map(|c| c * c).sum::<f64>().sqrt();
    if !len.is_finite() || len < 1e-9 {
        return None;
    }
    let z = frame.z.map(|c| c / len);
    let px = [1.0 - z[0] * z[0], -z[1] * z[0], -z[2] * z[0]];
    let plen = px.iter().map(|c| c * c).sum::<f64>().sqrt();
    let x = if plen > 1e-6 {
        px.map(|c| c / plen)
    } else {
        [0.0, 1.0, 0.0]
    };
    package::AttachmentFrame { z, x, ..frame }.orthonormalized()
}

/// The `profile`-source ingest probe (SCHEMA §7.8 `ExtractPrismProfile`) —
/// "is this body a length-parametric extrusion, and if so give me its canonical
/// end-cap face".
///
/// The Rust half of WP-C's authoring flow: import a vendor STEP, point at the
/// 500 mm stick it produced, and get back the measurements plus a `brep` file
/// containing exactly ONE canonical planar face. The caller hands those bytes to
/// [`Library::save_embedded_component`](onecad_library::Library::save_embedded_component)
/// as a `profile` package; every later placement re-prisms that face to its own
/// `length`.
///
/// Fences on the CURRENT head, read under a short lock and released before the
/// worker round-trip (the R-WP11 rule — the single writer is never held across
/// worker IO). A head that moves under the call comes back `STALE_PREVIEW` from
/// the worker's own fence, which is the point: the answer is about to be frozen
/// into a package.
///
/// A body that is not a prism is a successful
/// [`Refused`](crate::dto::PrismProfileAnswerDto::Refused) answer carrying the
/// measured `volumeRatio`, not an error — a cross-drilled stick must be able to
/// say by how much it missed.
///
/// # Errors
/// [`ApiError::NoDocument`] with no document open, [`ApiError::InvalidCommand`]
/// for a malformed body id or one that is not at head, and the worker's own
/// errors (`stalePreview`, `opFailed` for an unresolved body) otherwise.
pub async fn extract_prism_profile_at(
    runtime: &tokio::sync::Mutex<Option<DocumentRuntime>>,
    extractor: Arc<dyn GeometryExporter>,
    body_id: &str,
    path: Option<&str>,
) -> Result<crate::dto::PrismProfileAnswerDto, ApiError> {
    let body = BodyId::from_str(body_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad bodyId {body_id:?}: {e}")))?;

    let snapshot = {
        let guard = runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("extractPrismProfile".into()))?;
        if !rt.head_body_ids().contains(&body) {
            return Err(ApiError::InvalidCommand(format!(
                "extractPrismProfile: {body_id} is not a body at head"
            )));
        }
        rt.head_snapshot_id().ok_or_else(|| {
            ApiError::InvalidCommand(
                "extractPrismProfile: no published snapshot to fence against yet".into(),
            )
        })?
    };

    extractor
        .extract_prism_profile(snapshot, body, path, None)
        .await
        .map_err(Into::into)
}

/// The testable core of [`save_as_component`].
///
/// Explicit runtime, exporter, and library path let every host and integration
/// test drive the same worker-backed component extraction.
pub async fn save_as_component_at(
    root: &Path,
    runtime: &tokio::sync::Mutex<Option<crate::document_runtime::DocumentRuntime>>,
    exporter: std::sync::Arc<dyn crate::export::GeometryExporter>,
    body_id: String,
    spec: NewComponentSpec,
    preview_png: Option<String>,
    union_solids: bool,
) -> Result<LibraryComponentDto, ApiError> {
    let body = onecad_core::ids::BodyId::from_str(&body_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad bodyId {body_id:?}: {e}")))?;

    // A scratch directory UNIQUE to this save (two saves of one body must not
    // share or sweep each other's), removed by the guard on every exit path.
    let scratch = ScratchDir::create("saveAsComponent")?;
    let document_path = scratch.path().join("source.onecad");
    let geometry_path = scratch.path().join("geometry.xbf");

    // Freeze the document under the runtime lock, write it outside — the same
    // split `save_document` uses, for the same reason (serialization must not
    // block an edit).
    // `fence` is read in the SAME lock acquisition that validated `body` against the
    // head, so the bake below names the snapshot this component was checked on
    // (WP-H, SCHEMA §7.8) — a head that moves before the bake is a refusal.
    let (payload, variables, fence) = {
        let mut guard = runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("saveAsComponent".into()))?;
        if !rt.head_body_ids().contains(&body) {
            return Err(ApiError::InvalidCommand(format!(
                "saveAsComponent: {body_id} is not a body at head"
            )));
        }
        // Name -> (value, derived-from-an-expression), read under THIS lock so the
        // declared defaults are the numbers the frozen document builds.
        let variables: BTreeMap<String, (f64, bool)> = rt
            .variables()
            .iter()
            .map(|v| (v.name.clone(), (v.value.value, v.value.expr.is_some())))
            .collect();
        let fence = crate::export::export_fence(rt);
        (
            rt.build_save_payload(
                crate::api::save_meta(),
                crate::document_runtime::SaveCaches::none(),
            ),
            variables,
            fence,
        )
    };

    // A declared parameter that names no variable of the document being frozen
    // could never be honored by the re-bake — refused HERE, where the author can
    // still fix it, rather than at the first instance edit on someone else's
    // machine (the same discipline as the single-solid rule below).
    reject_static_save_extras(&spec)?;
    let parameters = document_bound_parameters(&spec, &variables)?;
    let configurations = authored_configurations(&spec, &parameters)?;
    crate::document_runtime::DocumentRuntime::write_payload(&document_path, &payload)
        .map_err(|e| ApiError::Internal(format!("saveAsComponent: freeze document: {e}")))?;

    // Bake the solid out of the LIVE session. `xbf` is the attribute-preserving
    // form (face colors survive), and the worker echoes back the codec/format
    // it actually wrote rather than this layer pinning a version.
    let baked = exporter
        .export_geometry(
            &geometry_path.to_string_lossy(),
            &[body],
            "xbf",
            union_solids,
            fence,
        )
        .await?;
    if baked.solid_count != 1 {
        return Err(ApiError::InvalidCommand(format!(
            "saveAsComponent: {MULTI_SOLID_REFUSAL}: a component must resolve to exactly one \
             solid (spec §9); {body_id} baked {} — union them at bake, or split them into \
             separate components",
            baked.solid_count
        )));
    }

    let document = std::fs::read(&document_path)
        .map_err(|e| ApiError::Internal(format!("saveAsComponent: read frozen document: {e}")))?;
    let geometry = std::fs::read(&geometry_path)
        .map_err(|e| ApiError::Internal(format!("saveAsComponent: read baked geometry: {e}")))?;
    drop(scratch);

    // Frames are orthonormalized (and a degenerate one refused) HERE, at the
    // authoring door, so the manifest that lands on disk is already in the
    // form `package::parse` guarantees every reader.
    let mut attachments = BTreeMap::new();
    for (key, a) in spec.attachments {
        let frame = match a.frame {
            Some(f) => Some(f.orthonormalized().ok_or_else(|| {
                ApiError::InvalidCommand(format!(
                    "saveAsComponent: attachment `{key}` has a degenerate frame — `z` and `x` \
                     must be non-zero, finite, and not parallel"
                ))
            })?),
            None => None,
        };
        attachments.insert(
            key,
            package::AttachmentSpec {
                on: a.on,
                accepts: a.accepts,
                frame,
            },
        );
    }

    let package = ComponentPackage {
        identity: package::Identity {
            id: spec.id.clone(),
            version: spec.version.clone(),
            // Recomputed by `save_component` over what actually lands on disk.
            revision: format!("sha256:{}", "0".repeat(64)),
        },
        metadata: package::Metadata {
            name: spec.name,
            standard: None,
            designation: spec.designation,
            category: spec.category,
            tags: spec.tags,
            unit: "mm".to_string(),
            default_configuration: spec.default_configuration,
        },
        // Overwritten by `save_component` with the document source it produces.
        source: package::SourceSpec::Generator {
            generator: String::new(),
            generator_version: 0,
        },
        // Each declared parameter binds to a VARIABLE of the document being
        // frozen above (WP-F1.3); `set_component_params` replays that document
        // with the overridden values and re-bakes. The `value` recorded here is
        // the variable's authoring-time number, so an instance that overrides
        // nothing resolves to exactly the solid baked below.
        parameters,
        attachments,
        configurations,
    };

    let mut library = open_at(root)?;
    library.save_component(onecad_library::NewComponent {
        package,
        document,
        geometry,
        geometry_codec: baked.codec,
        geometry_format: Some(baked.format),
        preview_png: crate::api::decode_preview_png(preview_png),
    })?;

    let (version, entry) = library
        .get(&spec.id, Some(&spec.version))
        .ok_or_else(|| ApiError::Internal("saveAsComponent: saved but not indexed".into()))?;
    Ok(component_dto(root, &spec.id, version, entry))
}

/// A replace resolved but not yet applied: the edit, the blob it needs staged
/// first, and the report. Lets a batch (version upgrade of many instances)
/// compose several replaces into one transaction.
pub(crate) struct PreparedReplace {
    pub(crate) archive: crate::library_archive::CapturedArchive,
    pub(crate) command: EditCommand,
    pub(crate) blob: Option<StagedBlob>,
    pub(crate) report: ReplaceComponentReportDto,
    runtime_session: String,
}

/// Swaps a placed instance to a DIFFERENT component (spec §3.3
/// `ReplaceComponent`) — all M6 socket caps to flanged button heads, or one
/// instance to a newer version of the same package.
///
/// **In-place at the SAME `RecordId`**, via `UpdateOperationParams`, exactly
/// like `setComponentParams`: the record keeps its identity, its position in
/// the timeline, and every downstream reference to the body it mints (which is
/// `body_<opId>` — derived from the record, so it survives too). Deleting and
/// re-adding would break all three, which is why spec §3.3 calls this an
/// in-place edit and SCHEMA §7.3 gives it no `opType` of its own.
///
/// **The mate is carried by ATTACHMENT NAME, or not at all.** A recorded mate
/// names an attachment on the OLD component (`shank_axis`); the new component
/// is a different package with its own `[attachments]` table. If it declares
/// the same name the mate rides across unchanged — the target ElementRef, the
/// kind and the flip are all still valid, and regen re-seats it (WP-3.1). If it
/// does NOT, the mate is dropped and the instance keeps its frozen
/// `placement`: the component stays exactly where it is, and nothing re-seats
/// it against geometry it was never mated to. Silently binding the new
/// component's first attachment instead would be the mis-bind spec §0
/// invariant 4 exists to forbid. The report says which name was lost so the UI
/// can tell the user rather than leaving them to notice.
async fn replace_component_at(
    root: &Path,
    state: &AppState,
    record_id: String,
    component_id: String,
    component_version: String,
    free_params: BTreeMap<String, ComponentParamValue>,
) -> Result<(CommandOutcome, DocumentProjection, Option<String>), ApiError> {
    let prepared = prepare_replace_component_at(
        root,
        state,
        record_id,
        component_id,
        component_version,
        free_params,
    )
    .await?;
    let (outcome, projection) = stage_archive_and_apply(
        state,
        prepared.blob,
        prepared.archive,
        prepared.command,
        &prepared.runtime_session,
    )
    .await?;
    Ok((outcome, projection, prepared.report.dropped_mate_attachment))
}

/// Resolves a replace (everything [`replace_component_at`] documents) without
/// staging or applying it.
pub(crate) async fn prepare_replace_component_at(
    root: &Path,
    state: &AppState,
    record_id: String,
    component_id: String,
    component_version: String,
    free_params: BTreeMap<String, ComponentParamValue>,
) -> Result<PreparedReplace, ApiError> {
    let runtime_session = current_runtime_session(state, "replaceComponent").await?;
    let admitted = admitted_component_snapshot_at(
        "replaceComponent",
        root,
        &component_id,
        &component_version,
        None,
        &state.archive_budget(),
    )?;
    prepare_replace_component_from_snapshot(
        state,
        record_id,
        component_id,
        component_version,
        free_params,
        admitted,
        &runtime_session,
    )
    .await
}

pub(crate) async fn prepare_replace_component_from_snapshot(
    state: &AppState,
    record_id: String,
    component_id: String,
    component_version: String,
    free_params: BTreeMap<String, ComponentParamValue>,
    admitted: AdmittedSnapshot,
    expected_session: &str,
) -> Result<PreparedReplace, ApiError> {
    let AdmittedSnapshot {
        snapshot,
        retention,
    } = admitted;
    let record = RecordId::from_str(&record_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {record_id:?}: {e}")))?;

    let (current, runtime_session) = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("replaceComponent".into()))?;
        require_runtime_session(rt, expected_session)?;
        let raw = rt.operation_params(record).ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "replaceComponent: no params for record {record_id}"
            ))
        })?;
        let current = serde_json::from_value::<PlaceComponentParams>(raw).map_err(|e| {
            ApiError::InvalidCommand(format!(
                "replaceComponent: record {record_id} is not a placed component: {e}"
            ))
        })?;
        (current, rt.runtime_session())
    };

    let archive = crate::library_archive::capture(
        &snapshot,
        MAX_CAPTURED_DRAFT_BYTES,
        &state.archive_budget(),
    )?;
    let revision = snapshot.package.identity.revision.clone();
    let declared = declared_numeric_params(&snapshot.package.parameters);
    let resolved = snapshot.source;
    let package = snapshot.package;
    // Params are checked against the NEW signature, not the old one: a key the
    // old component called free may not exist here at all.
    validate_component_params("replaceComponent", &package, &component_id, &free_params)?;

    let (source, blob) =
        component_source_from_resolved(resolved, &component_id, &declared, Some(retention))?;
    let source = source_with_free_params(source, &free_params, &component_id)?;

    let (mate, dropped_attachment) = match current.mate {
        // A replace IS an authoring event, so the kept mate re-freezes the NEW
        // package's frame for that attachment (WP-F1.1). Carrying the old
        // component's frozen frame onto different geometry would seat the new
        // part by the old one's dimensions — a silent mis-seat.
        Some(mut mate) if package.attachments.contains_key(&mate.self_attachment) => {
            mate.self_frame = attachment_frame(&package, &mate.self_attachment);
            (Some(mate), None)
        }
        Some(mate) => {
            let lost = mate.self_attachment.clone();
            (None, Some(lost))
        }
        None => (None, None),
    };

    let updated = PlaceComponentParams {
        component_id,
        component_version,
        component_revision: revision,
        params: free_params,
        source,
        mate,
        placement: current.placement,
        extra: current.extra,
    };
    let op = Operation::Known(KnownOperation::PlaceComponent(updated));

    Ok(PreparedReplace {
        archive,
        command: EditCommand::UpdateOperationParams { record, op },
        blob,
        report: ReplaceComponentReportDto {
            dropped_mate_attachment: dropped_attachment,
        },
        runtime_session,
    })
}

/// Whether a NEWER version of this instance's component is indexed (spec §3.3's
/// "opt-in version upgrade").
///
/// Read-only and opt-in BY CONSTRUCTION: it reports, it never swaps. Automatic
/// upgrading is precisely the SolidWorks Toolbox failure spec §0 invariant 4
/// names — a document that opens differently because a shared library moved
/// underneath it. Acting on this answer is a separate, explicit
/// `replaceComponent` call the user makes.
fn component_upgrade_available_at(
    root: &Path,
    component_id: &str,
    component_version: &str,
) -> Result<Option<ComponentUpgradeDto>, ApiError> {
    let library = open_at(root)?;
    let Some((latest_version, entry)) = library.index().newer_than(component_id, component_version)
    else {
        return Ok(None);
    };
    Ok(Some(ComponentUpgradeDto {
        component_id: component_id.to_string(),
        current_version: component_version.to_string(),
        latest_version: latest_version.to_string(),
        latest_revision: entry.revision.clone(),
    }))
}

/// Swaps a placed instance to a different component in place (spec §3.3).
///
/// Returns `{ projection, report }` (W5 result truth), not the bare report. A
/// replace is an `UpdateOperationParams` that re-bakes geometry and schedules a
/// regen, so resolving is NOT the same as succeeding — and the frontend can only
/// correlate that regen against the post-apply projection revision, exactly as
/// `apply_edit_command` does. The report keeps its own shape and stays the only
/// place a dropped mate attachment is named.
pub async fn replace_component(
    app: HostContext,
    state: &AppState,
    record_id: String,
    component_id: String,
    component_version: String,
    params: Option<BTreeMap<String, ComponentParamValue>>,
) -> Result<ReplaceComponentResultDto, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("replaceComponent: no app data dir".into()))?;
    let (outcome, projection, dropped_mate_attachment) = replace_component_at(
        &root,
        state,
        record_id,
        component_id,
        component_version,
        params.unwrap_or_default(),
    )
    .await?;
    finish(&app, state, &outcome, &projection);
    Ok(ReplaceComponentResultDto {
        projection,
        report: ReplaceComponentReportDto {
            dropped_mate_attachment,
        },
    })
}

/// Reports whether a newer version of the component behind `record_id` is
/// indexed. Read-only; see [`component_upgrade_available_at`].
pub async fn component_upgrade_available(
    app: HostContext,
    state: &AppState,
    record_id: String,
) -> Result<Option<ComponentUpgradeDto>, ApiError> {
    let Some(root) = library_root(&app) else {
        return Ok(None);
    };
    let record = RecordId::from_str(&record_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {record_id:?}: {e}")))?;
    let placed = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("componentUpgradeAvailable".into()))?;
        let Some(raw) = rt.operation_params(record) else {
            return Ok(None);
        };
        // A record that is not a placed component simply has no upgrade — this
        // is a read the inspector fires for whatever is selected, so a
        // non-component selection must answer "none", never error.
        match serde_json::from_value::<PlaceComponentParams>(raw) {
            Ok(p) => p,
            Err(_) => return Ok(None),
        }
    };
    component_upgrade_available_at(&root, &placed.component_id, &placed.component_version)
}

/// Catalog search over the persisted index (does not reindex).
pub fn search_library_components_at(
    root: &Path,
    query: &SearchQueryDto,
) -> Result<Vec<LibraryComponentDto>, ApiError> {
    let library = open_at(root)?;
    let query = SearchQuery {
        text: query.text.clone(),
        category: query.category.clone(),
        source_kind: query.source_kind.clone(),
        latest_only: query.latest_only,
        sort: match query.sort {
            SearchSortDto::Name => SearchSort::Name,
            SearchSortDto::Id => SearchSort::Id,
        },
    };
    Ok(library
        .search(&query)
        .into_iter()
        .map(|hit| component_dto(root, hit.id, hit.version, hit.entry))
        .collect())
}

pub async fn search_library_components(
    app: HostContext,
    query: SearchQueryDto,
) -> Result<Vec<LibraryComponentDto>, ApiError> {
    let Some(root) = library_root(&app) else {
        return Ok(Vec::new());
    };
    blocking(move || search_library_components_at(&root, &query)).await
}

/// One component at `version`, plus every indexed version of its id.
pub fn component_detail_at(
    root: &Path,
    component_id: &str,
    component_version: &str,
) -> Result<ComponentDetailDto, ApiError> {
    let library = open_at(root)?;
    let (version, entry) = library
        .get(component_id, Some(component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "componentDetail: unknown component {component_id}@{component_version}"
            ))
        })?;
    let versions = library
        .index()
        .versions(component_id)
        .into_iter()
        .map(str::to_string)
        .collect();
    Ok(ComponentDetailDto {
        component: component_dto(root, component_id, version, entry),
        versions,
    })
}

pub async fn component_detail(
    app: HostContext,
    component_id: String,
    component_version: String,
) -> Result<ComponentDetailDto, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("componentDetail: no app data dir".into()))?;
    blocking(move || component_detail_at(&root, &component_id, &component_version)).await
}

/// Returns editor details from the document's verified immutable package archive.
/// `None` means this legacy document has no captured archive for the revision.
pub async fn component_detail_archived(
    state: &AppState,
    component_id: &str,
    component_version: &str,
    revision: &str,
) -> Result<Option<ComponentDetailDto>, ApiError> {
    let (archived, session) = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("componentDetail".into()))?;
        (
            runtime.captured_library_archive(component_id, component_version, revision)?,
            runtime.runtime_session(),
        )
    };
    let Some((entry, bytes)) = archived else {
        return Ok(None);
    };
    let (snapshot, _decode_retention) =
        crate::library_archive::decode(&entry, revision, &bytes, &state.archive_budget())?;
    let package = snapshot.package;
    let (source_kind, generator_id, generator_version) = match package.source {
        package::SourceSpec::Generator {
            generator,
            generator_version,
        } => ("generator", Some(generator), Some(generator_version)),
        package::SourceSpec::Embedded { .. } => ("embedded", None, None),
        package::SourceSpec::Profile { .. } => ("profile", None, None),
        package::SourceSpec::Document { .. } => ("document", None, None),
    };
    {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("componentDetail".into()))?;
        require_runtime_session(runtime, &session)?;
    }
    Ok(Some(ComponentDetailDto {
        versions: vec![package.identity.version.clone()],
        component: LibraryComponentDto {
            id: package.identity.id,
            version: package.identity.version,
            revision: package.identity.revision,
            name: package.metadata.name,
            category: package.metadata.category,
            tags: package.metadata.tags,
            source_kind: source_kind.into(),
            generator_id,
            generator_version,
            attachments: package.attachments,
            parameters: package.parameters,
            designation: package.metadata.designation,
            standard: package.metadata.standard,
            unit: Some(package.metadata.unit),
            preview_path: None,
            configurations: package.configurations,
            default_configuration: package.metadata.default_configuration,
        },
    }))
}

/// Returns editor details from the exact verified revision recorded by a placement.
pub fn component_detail_pinned_at(
    root: &Path,
    component_id: &str,
    component_version: &str,
    revision: &str,
) -> Result<ComponentDetailDto, ApiError> {
    let library = open_at(root)?;
    let (version, entry) = library
        .get(component_id, Some(component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "componentDetail: unknown component {component_id}@{component_version}"
            ))
        })?;
    let snapshot = library.resolve_snapshot(&ResolveRequest {
        component_id,
        component_version,
        component_revision: revision,
    })?;
    let pkg = snapshot.package;
    let mut component = component_dto(root, component_id, version, entry);
    component.name = pkg.metadata.name;
    component.category = pkg.metadata.category;
    component.tags = pkg.metadata.tags;
    component.designation = pkg.metadata.designation;
    component.standard = pkg.metadata.standard;
    component.unit = Some(pkg.metadata.unit);
    component.revision = pkg.identity.revision;
    component.attachments = pkg.attachments;
    component.parameters = pkg.parameters;
    component.configurations = pkg.configurations;
    component.default_configuration = pkg.metadata.default_configuration;
    let (kind, generator, generator_version) = match pkg.source {
        package::SourceSpec::Generator {
            generator,
            generator_version,
        } => ("generator", Some(generator), Some(generator_version)),
        package::SourceSpec::Embedded { .. } => ("embedded", None, None),
        package::SourceSpec::Profile { .. } => ("profile", None, None),
        package::SourceSpec::Document { .. } => ("document", None, None),
    };
    component.source_kind = kind.into();
    component.generator_id = generator;
    component.generator_version = generator_version;
    let versions = library
        .index()
        .versions(component_id)
        .into_iter()
        .map(str::to_string)
        .collect();
    Ok(ComponentDetailDto {
        component,
        versions,
    })
}

pub async fn component_detail_pinned(
    app: HostContext,
    component_id: String,
    component_version: String,
    revision: String,
) -> Result<ComponentDetailDto, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("componentDetail: no app data dir".into()))?;
    blocking(move || {
        component_detail_pinned_at(&root, &component_id, &component_version, &revision)
    })
    .await
}

/// How a placed instance relates to the library right now. Reads the library
/// only, so any host can call it per record. Missing id@version wins over an
/// unreadable manifest, which wins over a revision mismatch, which wins over
/// an upgrade offer. Opens the library per call; sweeps use
/// [`component_status_with`].
pub fn component_status_for(root: &Path, placed: &PlaceComponentParams) -> ComponentStatusDto {
    match Library::open(root) {
        Ok(library) => component_status_with(&library, placed),
        Err(e) => ComponentStatusDto::DefinitionUnreadable {
            reason: e.to_string(),
        },
    }
}

pub fn component_status_with(
    library: &Library,
    placed: &PlaceComponentParams,
) -> ComponentStatusDto {
    let current = match library.declared_revision(&placed.component_id, &placed.component_version) {
        Ok(revision) => revision,
        Err(onecad_library::LibraryError::NotFound(_)) => {
            return ComponentStatusDto::DefinitionMissing
        }
        Err(e) => {
            return ComponentStatusDto::DefinitionUnreadable {
                reason: e.to_string(),
            }
        }
    };
    if current != placed.component_revision {
        return ComponentStatusDto::RevisionMismatch {
            recorded: placed.component_revision.clone(),
            current,
        };
    }
    if let Err(error) = library.resolve_snapshot(&ResolveRequest {
        component_id: &placed.component_id,
        component_version: &placed.component_version,
        component_revision: &placed.component_revision,
    }) {
        return ComponentStatusDto::DefinitionUnreadable {
            reason: error.to_string(),
        };
    }
    match library
        .index()
        .newer_than(&placed.component_id, &placed.component_version)
    {
        Some((version, _)) => ComponentStatusDto::UpgradeAvailable {
            version: version.to_string(),
        },
        None => ComponentStatusDto::Linked,
    }
}

/// `(record id, status)` for every placed or detached component in the open
/// document. Read-only.
pub async fn component_statuses(
    app: HostContext,
    state: &AppState,
) -> Result<Vec<(String, ComponentStatusDto)>, ApiError> {
    component_statuses_at(library_root(&app).as_deref(), state).await
}

/// [`component_statuses`] over an explicit root; `None` (no library) reads
/// components with a carried source remain editable even when no catalog is present.
pub async fn component_statuses_at(
    root: Option<&Path>,
    state: &AppState,
) -> Result<Vec<(String, ComponentStatusDto)>, ApiError> {
    let (session, revision, records): (_, _, Vec<OperationRecord>) = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("componentStatuses".into()))?;
        let records = rt
            .projection()
            .features
            .iter()
            .filter(|f| matches!(f.op_type.as_str(), "PlaceComponent" | "DetachComponent"))
            .filter_map(|f| rt.operation_record(RecordId::from_str(&f.id).ok()?))
            .collect();
        (rt.runtime_session(), rt.revision(), records)
    };
    let library = root.and_then(|r| Library::open(r).ok());
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let status = match &record.op {
            Operation::Known(KnownOperation::PlaceComponent(placed)) => {
                let catalog = match &library {
                    Some(library) => component_status_with(library, placed),
                    None => ComponentStatusDto::DefinitionMissing,
                };
                match component_detail_archived(
                    state,
                    &placed.component_id,
                    &placed.component_version,
                    &placed.component_revision,
                )
                .await
                {
                    Ok(Some(_))
                        if !matches!(
                            catalog,
                            ComponentStatusDto::Linked
                                | ComponentStatusDto::UpgradeAvailable { .. }
                        ) =>
                    {
                        ComponentStatusDto::ArchivedSnapshot {
                            catalog: Box::new(catalog),
                        }
                    }
                    Ok(_) => catalog,
                    Err(error) => ComponentStatusDto::DefinitionUnreadable {
                        reason: error.to_string(),
                    },
                }
            }
            Operation::Known(KnownOperation::DetachComponent(_)) => ComponentStatusDto::Detached,
            _ => continue,
        };
        out.push((record.record_id.to_string(), status));
    }
    let guard = state.runtime.lock().await;
    if guard
        .as_ref()
        .is_none_or(|rt| !rt.matches_runtime_session(&session) || rt.revision() != revision)
    {
        return Err(ApiError::StalePreview(
            "component status belongs to an obsolete document".into(),
        ));
    }
    Ok(out)
}

/// Drops a placed component's library identity, keeping its cached geometry
/// as an ordinary body (spec §3.4). Reads the target record's CURRENT
/// `source`/`placement` (via `rt.operation_params`, the same read path
/// `get_operation_params`/`CadClient.getOperationParams` uses) and re-applies
/// them on a `DetachComponentParams` — the sanctioned in-place op-type swap
/// `edit::session::op_type_edit_allowed` allows.
async fn detach_component_at(
    state: &AppState,
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

    // Spec §3.4 — a detached component is INERT PROVENANCE: it remembers where
    // the instance came from and where it sat, and stops tracking anything. The
    // placement is lifted VERBATIM from the `PlaceComponent` record, so it can
    // arrive carrying a variable binding; `DetachComponentParams` exposes no
    // drivable scalars, so that binding would never be substituted again, would
    // survive a rename as a dangling reference, and would ride into the planner
    // hash and onto the OCW1 wire (both strips are registry-driven). Detach is
    // the moment the tracking ends, so the bindings end here too — the NUMBERS
    // are kept exactly as they last resolved.
    let mut placement: FrozenPlacement = serde_json::from_value(placement)
        .map_err(|e| ApiError::InvalidCommand(format!("detachComponent: bad placement: {e}")))?;
    placement.clear_bindings();
    let detach_params = DetachComponentParams {
        source: serde_json::from_value(source)
            .map_err(|e| ApiError::InvalidCommand(format!("detachComponent: bad source: {e}")))?,
        placement,
        extra: Default::default(),
    };
    let op = Operation::Known(KnownOperation::DetachComponent(detach_params));

    apply_and_project(state, EditCommand::UpdateOperationParams { record, op }).await
}

pub async fn detach_component(
    app: HostContext,
    state: &AppState,
    record_id: String,
) -> Result<DocumentProjection, ApiError> {
    let (outcome, projection) = detach_component_at(state, record_id).await?;
    finish(&app, state, &outcome, &projection);
    Ok(projection)
}

/// The shared apply→project tail (HostContext-free — see the module doc
/// comment). Mirrors `api::apply_edit_command`'s lock scope exactly.
async fn apply_and_project(
    state: &AppState,
    command: EditCommand,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("library".into()))?;
    let outcome = rt.apply(command)?;
    Ok((outcome, rt.projection()))
}

/// The HostContext-dependent tail `apply_and_project` can't do itself: emit the
/// projection event, hand the outcome to the scheduler, note the mutation
/// tick — mirrors `api::apply_edit_command`'s tail exactly, so a library edit
/// is indistinguishable from any other (autosave, undo, projection push).
pub(crate) fn finish(
    app: &HostContext,
    state: &AppState,
    outcome: &CommandOutcome,
    projection: &DocumentProjection,
) {
    let _ = app.emit(AppEvent::ProjectionUpdated(projection.clone()));
    if let Some(sched) = state.scheduler.get() {
        sched.handle(outcome);
    }
    state.note_mutation();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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

    fn stamp_package(dir: &Path) {
        let raw = std::fs::read_to_string(dir.join("component.toml")).unwrap();
        let pkg = package::parse(&raw, dir).unwrap();
        package::write_package(dir, &pkg).unwrap();
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
        stamp_package(&dir);
    }

    /// Same as `write_generator_package`, plus an `[attachments]` table where
    /// ONE attachment declares a `frame` and the other does not — the shape
    /// WP-F1.1's freeze needs to exercise both arms. `z` is deliberately
    /// un-normalized so the test can also see that the parse normalized it.
    fn write_generator_package_with_attachment_frame(root: &Path, id: &str, name: &str) {
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

[attachments]
head_seat = {{ on = "face:head_underside", accepts = ["plane"], frame = {{ origin = [0.0, 0.0, 10.0], z = [0.0, 0.0, 4.0], x = [1.0, 0.0, 0.0] }} }}
shank_axis = {{ on = "cylinder:shank", accepts = ["cylinder", "hole"] }}
"#,
            "0".repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
        stamp_package(&dir);
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
        stamp_package(&dir);
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

        let state = &app_state;

        let (outcome, projection) = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [1.0, 2.0, 3.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place_component_at");
        assert!(matches!(outcome.regen, onecad_core::edit::RegenHint::ToEnd));
        assert_eq!(projection.features.len(), 1);
        assert_eq!(projection.features[0].op_type, "PlaceComponent");
        let record_id = projection.features[0].id.clone();

        let (_outcome, projection) = detach_component_at(state, record_id)
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
        let guard = app_state;
        let rt_guard = guard.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        assert_eq!(rt.head_body_ids().len(), 0, "no regen ran, so no body yet");
    }

    /// F1 — spec §3.4 "inert provenance", end to end through the real command
    /// core: a placement bound to a document variable loses that binding at
    /// detach, keeping the number it last resolved to.
    ///
    /// Detach is the moment tracking ends. `DetachComponentParams` exposes no
    /// drivable scalars, so a surviving binding would never be substituted
    /// again, would ride into the planner hash and onto the OCW1 wire (both
    /// strips being registry-driven), and would outlive a `rename_variable` as a
    /// dangling reference. The params this asserts on are exactly what
    /// `get_operation_params` serves the inspector, so this also pins that a
    /// detached row shows no binding.
    #[tokio::test]
    async fn detach_clears_a_placement_variable_binding_and_keeps_its_number() {
        use onecad_core::document::variables::{Unit, Variable};
        use onecad_core::ids::VariableId;

        let dir = tempfile::tempdir().unwrap();
        write_generator_package(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        {
            let mut guard = state.runtime.lock().await;
            guard
                .as_mut()
                .unwrap()
                .apply(EditCommand::AddVariable {
                    variable: Variable {
                        id: VariableId::new(),
                        name: "gap".into(),
                        value: Scalar::new(12.0),
                        unit: Unit::Mm,
                    },
                })
                .expect("AddVariable gap");
        }

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place_component_at");
        let record_id = projection.features[0].id.clone();
        let record = RecordId::from_str(&record_id).unwrap();

        {
            let mut guard = state.runtime.lock().await;
            let rt = guard.as_mut().unwrap();
            let mut current: PlaceComponentParams =
                serde_json::from_value(rt.operation_params(record).unwrap()).unwrap();
            current.placement.translate[0] = Scalar::with_expr(24.0, "gap * 2");
            rt.apply(EditCommand::UpdateOperationParams {
                record,
                op: Operation::Known(KnownOperation::PlaceComponent(current)),
            })
            .expect("bind the placement to `gap`");
            // Sanity: the binding really is on the record before the detach.
            let raw = rt.operation_params(record).unwrap();
            assert_eq!(
                raw["placement"]["translate"][0]["expr"],
                serde_json::json!("gap * 2")
            );
        }

        let (_outcome, projection) = detach_component_at(state, record_id.clone())
            .await
            .expect("detach_component_at");
        assert_eq!(projection.features[0].op_type, "DetachComponent");

        let guard = state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let raw = rt.operation_params(record).unwrap();
        assert_eq!(
            raw["placement"]["translate"][0],
            serde_json::json!({ "value": 24.0 }),
            "the number survives; the binding does not"
        );
        assert!(
            !serde_json::to_string(&raw).unwrap().contains("expr"),
            "a detached placement carries no binding anywhere: {raw}"
        );
        // …and the row the inspector draws shows no unresolved-expression state.
        let feature = projection
            .features
            .into_iter()
            .find(|f| f.id == record_id)
            .expect("the detached row");
        assert!(feature.primary_expr.is_none());
        assert!(feature.diagnostics.is_empty(), "{:?}", feature.diagnostics);
    }

    /// WP-H2: a mate whose pick already carries a Rust-minted `ElementId`
    /// needs no promotion — the record must land with the full `ComponentMate`
    /// exactly as the gesture described it. (The promotion branch needs a real
    /// worker and is exercised by `component_ops.rs`.)
    #[tokio::test]
    async fn place_with_an_element_id_mate_records_the_full_mate() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let body_uuid = uuid::Uuid::from_u128(0xb0d1);
        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            Some(PlaceComponentMateInput {
                self_attachment: "shank_axis".to_string(),
                target_body_id: format!("body_{body_uuid}"),
                target_topo_key: "f:7".to_string(),
                target_element_id: Some("el_0000000000000f7a".to_string()),
                target_kind: "face".to_string(),
                kind: onecad_core::document::record::MateKind::Concentric,
                flipped: true,
                anchor_world_point: [1.0, 2.0, 3.0],
            }),
        )
        .await
        .expect("place with mate");
        let record_id = projection.features[0].id.clone();

        let guard = app_state;
        let rt_guard = guard.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        let params = rt
            .operation_params(RecordId(uuid::Uuid::parse_str(&record_id).unwrap()))
            .expect("record params");
        let mate = &params["mate"];
        assert_eq!(mate["selfAttachment"], "shank_axis");
        assert_eq!(mate["kind"], "concentric");
        assert_eq!(mate["flipped"], true);
        assert_eq!(
            mate["target"]["primary"]["elementId"],
            "el_0000000000000f7a"
        );
        assert_eq!(
            mate["target"]["anchor"]["worldPoint"],
            serde_json::json!([1.0, 2.0, 3.0])
        );
        // The package declares no `[attachments]` frame, so none is frozen —
        // the pre-WP-F1.1 shape, unchanged.
        assert!(mate.get("selfFrame").is_none(), "unframed mate: {mate}");
    }

    /// WP-F1.1: the matched attachment's own local frame is FROZEN into the
    /// authored mate. That is what lets the placement re-seat by its
    /// attachment point on every later regen — including with the library
    /// folder deleted, which is exactly why it is frozen and not looked up.
    #[tokio::test]
    async fn place_with_a_mate_freezes_the_packages_attachment_frame() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_attachment_frame(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let body_uuid = uuid::Uuid::from_u128(0xb0d2);
        let mate_input = |attachment: &str| PlaceComponentMateInput {
            self_attachment: attachment.to_string(),
            target_body_id: format!("body_{body_uuid}"),
            target_topo_key: "f:7".to_string(),
            target_element_id: Some("el_0000000000000f7a".to_string()),
            target_kind: "face".to_string(),
            kind: onecad_core::document::record::MateKind::Concentric,
            flipped: false,
            anchor_world_point: [1.0, 2.0, 3.0],
        };

        // Both placements FIRST — `place_component_at` takes the runtime lock
        // itself, so nothing may hold it across these calls.
        let mut record_ids = Vec::new();
        for attachment in ["head_seat", "shank_axis"] {
            let (_outcome, projection) = place_component_at(
                dir.path(),
                state,
                "onecad.std.iso4762".to_string(),
                "1.0.0".to_string(),
                [0.0, 0.0, 0.0],
                None,
                Default::default(),
                Some(mate_input(attachment)),
            )
            .await
            .unwrap_or_else(|e| panic!("place with a `{attachment}` mate: {e:?}"));
            record_ids.push(projection.features.last().unwrap().id.clone());
        }

        let guard = app_state;
        let rt_guard = guard.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        let params_of = |id: &String| {
            rt.operation_params(RecordId(uuid::Uuid::parse_str(id).unwrap()))
                .expect("record params")
        };

        let framed = params_of(&record_ids[0]);
        let frame = &framed["mate"]["selfFrame"];
        assert_eq!(frame["origin"], serde_json::json!([0.0, 0.0, 10.0]));
        // Normalized on the way in — the manifest wrote `z = [0, 0, 4]`.
        assert_eq!(frame["z"], serde_json::json!([0.0, 0.0, 1.0]));
        assert_eq!(frame["x"], serde_json::json!([1.0, 0.0, 0.0]));

        // An attachment that declares NO frame in the same package freezes
        // none — the lookup is per-attachment, not per-package.
        let unframed = params_of(&record_ids[1]);
        assert!(
            unframed["mate"].get("selfFrame").is_none(),
            "unframed attachment: {}",
            unframed["mate"]
        );
    }

    /// WP-H2 fail-closed: a mate that needs promotion (no `ElementId` on the
    /// pick) before the first publish must refuse the WHOLE placement — never
    /// author a silently-unmated record.
    #[tokio::test]
    async fn place_with_an_unpromotable_mate_refuses_the_placement() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let err = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            Some(PlaceComponentMateInput {
                self_attachment: "shank_axis".to_string(),
                target_body_id: format!("body_{}", uuid::Uuid::from_u128(0xb0d2)),
                target_topo_key: "f:7".to_string(),
                target_element_id: None,
                target_kind: "face".to_string(),
                kind: onecad_core::document::record::MateKind::Concentric,
                flipped: false,
                anchor_world_point: [0.0, 0.0, 0.0],
            }),
        )
        .await
        .expect_err("no head snapshot ⇒ the mate cannot promote ⇒ refuse");
        assert!(
            err.to_string().contains("re-pick"),
            "actionable refusal, got: {err}"
        );

        let guard = app_state;
        let rt_guard = guard.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        assert!(
            rt.projection().features.is_empty(),
            "fail closed: nothing was authored"
        );
    }

    /// WP-A3: the placement gesture sends free params of its own (auto-size on
    /// a hole rim), so they must land on BOTH the record and the generator
    /// source at PLACE time — not only through a follow-up edit. A commit that
    /// dropped them would place a differently-sized screw than the ghost showed.
    /// A generator package that declares ONE attachment, so the
    /// carry-the-mate-by-name path has something to match (or not) against.
    fn write_generator_package_with_attachment(root: &Path, id: &str, attachment: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"
[identity]
id = "{id}"
version = "1.0.0"
revision = "sha256:{zeros}"

[metadata]
name = "{id}"

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1

[attachments]
{attachment} = {{ on = "cylinder:shank", accepts = ["cylinder", "hole"] }}
"#,
            zeros = "0".repeat(64)
        );
        std::fs::write(dir.join(COMPONENT_MANIFEST_FILE), toml).unwrap();
        stamp_package(&dir);
    }

    /// Places `component_id`, then hand-authors a mate onto the resulting
    /// record. (`place_component_at` CAN author one since WP-H2, but its input
    /// shape wants pick evidence; writing the `ComponentMate` directly keeps
    /// these carry-rule tests independent of the promotion path.)
    async fn place_then_mate(
        root: &Path,
        state: &AppState,
        component_id: &str,
        attachment: &str,
    ) -> String {
        let (_outcome, projection) = place_component_at(
            root,
            state,
            component_id.to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place");
        let record_id = projection.features[0].id.clone();
        let record = RecordId::from_str(&record_id).unwrap();

        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let mut current: PlaceComponentParams =
            serde_json::from_value(rt.operation_params(record).unwrap()).unwrap();
        current.mate = Some(onecad_core::document::record::ComponentMate {
            self_attachment: attachment.to_string(),
            target: onecad_core::document::refs::ElementRef {
                import_source_identity: None,
                primary: Some(onecad_core::document::refs::PrimaryRef {
                    body: onecad_core::ids::BodyId::new(),
                    element: onecad_core::ids::ElementId::new("el_target"),
                    kind: onecad_core::document::refs::ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: None,
                anchor: None,
                extra: Default::default(),
            },
            kind: onecad_core::document::record::MateKind::Concentric,
            flipped: false,
            self_frame: None,
            target_axis: None,
            target_sidedness: None,
            extra: Default::default(),
        });
        rt.apply(EditCommand::UpdateOperationParams {
            record,
            op: Operation::Known(KnownOperation::PlaceComponent(current)),
        })
        .expect("author the mate");
        record_id
    }

    /// Spec §3.3: the swap happens IN PLACE. A new record would break the
    /// timeline position, every downstream ref, and the `body_<opId>` the
    /// instance mints — which is exactly why this is not a wire op.
    #[tokio::test]
    async fn replace_swaps_identity_at_the_same_record() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package(dir.path(), "onecad.std.iso4762", "SHCS");
        // The SECOND package is under an authored (`me.`) id on purpose: the
        // shipped catalog is one package, and a synthetic fixture wearing an
        // `onecad.std.*` id the app does not ship reads as a claim about the
        // seed catalog rather than what it is — some other component to swap to.
        write_generator_package(dir.path(), "me.custom.button-head", "Button head");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [1.0, 2.0, 3.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place");
        let record_id = projection.features[0].id.clone();

        let (_outcome, projection, dropped) = replace_component_at(
            dir.path(),
            state,
            record_id.clone(),
            "me.custom.button-head".to_string(),
            "1.0.0".to_string(),
            Default::default(),
        )
        .await
        .expect("replace");

        assert_eq!(projection.features.len(), 1, "no second record was added");
        assert_eq!(projection.features[0].id, record_id, "same RecordId");
        assert_eq!(projection.features[0].op_type, "PlaceComponent");
        assert!(dropped.is_none(), "no mate to lose");

        let raw = {
            let guard = state.runtime.lock().await;
            let rt = guard.as_ref().unwrap();
            rt.operation_params(RecordId::from_str(&record_id).unwrap())
                .unwrap()
        };
        let after: PlaceComponentParams = serde_json::from_value(raw).unwrap();
        assert_eq!(after.component_id, "me.custom.button-head");
        match after.source {
            ComponentSourceRef::Generator { generator_id, .. } => {
                assert_eq!(
                    generator_id, "iso4762",
                    "the NEW package's declared generator"
                );
            }
            other => panic!("expected a generator source, got {other:?}"),
        }
        // The frozen placement is the instance's position; a replace is not a move.
        assert_eq!(after.placement.translate[0], Scalar::new(1.0));
    }

    /// A mate survives a replace ONLY when the new component declares the same
    /// attachment name. Both halves are pinned: silently re-binding to some
    /// other attachment is the mis-bind spec §0 invariant 4 forbids, and
    /// silently dropping it without saying so leaves the user to discover that
    /// their screw stopped following the plate.
    #[tokio::test]
    async fn replace_carries_a_mate_by_attachment_name_and_reports_a_dropped_one() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_attachment(dir.path(), "acme.with.shank", "shank_axis");
        write_generator_package_with_attachment(dir.path(), "acme.also.shank", "shank_axis");
        write_generator_package_with_attachment(dir.path(), "acme.other.seat", "seat");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        // Carried: the new package declares `shank_axis` too.
        let record_id = place_then_mate(dir.path(), state, "acme.with.shank", "shank_axis").await;
        let (_o, _p, dropped) = replace_component_at(
            dir.path(),
            state,
            record_id.clone(),
            "acme.also.shank".to_string(),
            "1.0.0".to_string(),
            Default::default(),
        )
        .await
        .expect("replace onto a package with the same attachment");
        assert!(dropped.is_none(), "the mate rode across");
        let carried: PlaceComponentParams = {
            let guard = state.runtime.lock().await;
            let rt = guard.as_ref().unwrap();
            serde_json::from_value(
                rt.operation_params(RecordId::from_str(&record_id).unwrap())
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(
            carried.mate.as_ref().map(|m| m.self_attachment.as_str()),
            Some("shank_axis")
        );

        // Dropped: the next package declares `seat` only.
        let (_o, _p, dropped) = replace_component_at(
            dir.path(),
            state,
            record_id.clone(),
            "acme.other.seat".to_string(),
            "1.0.0".to_string(),
            Default::default(),
        )
        .await
        .expect("replace onto a package without the attachment");
        assert_eq!(
            dropped.as_deref(),
            Some("shank_axis"),
            "names what was lost"
        );
        let after: PlaceComponentParams = {
            let guard = state.runtime.lock().await;
            let rt = guard.as_ref().unwrap();
            serde_json::from_value(
                rt.operation_params(RecordId::from_str(&record_id).unwrap())
                    .unwrap(),
            )
            .unwrap()
        };
        assert!(
            after.mate.is_none(),
            "dropped, never re-bound to the new component's own attachment"
        );
    }

    /// The upgrade offer is a REPORT. It never swaps anything, and it never
    /// points backwards.
    #[test]
    fn upgrade_is_offered_only_when_a_strictly_newer_version_is_indexed() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package(dir.path(), "acme.part", "Part");
        reindex_library_at(dir.path()).unwrap();

        // Only 1.0.0 exists.
        assert!(
            component_upgrade_available_at(dir.path(), "acme.part", "1.0.0")
                .unwrap()
                .is_none()
        );
        // An instance recorded at a version NEWER than anything indexed is not
        // offered a downgrade.
        assert!(
            component_upgrade_available_at(dir.path(), "acme.part", "2.0.0")
                .unwrap()
                .is_none()
        );
        assert!(
            component_upgrade_available_at(dir.path(), "acme.missing", "1.0.0")
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn place_with_a_free_param_records_it_on_the_record_and_the_source() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_params(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let mut free = BTreeMap::new();
        free.insert(
            "thread".to_string(),
            ComponentParamValue::Text("M8".to_string()),
        );
        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            free,
            None,
        )
        .await
        .expect("place_component_at with a free param");

        let record_id = projection.features[0].id.clone();
        let raw = {
            let guard = state.runtime.lock().await;
            let rt = guard.as_ref().unwrap();
            rt.operation_params(onecad_core::ids::RecordId::from_str(&record_id).unwrap())
                .unwrap()
        };
        let placed: PlaceComponentParams = serde_json::from_value(raw).unwrap();
        assert_eq!(
            placed.params.get("thread"),
            Some(&ComponentParamValue::Text("M8".to_string()))
        );
        match placed.source {
            ComponentSourceRef::Generator { params, .. } => assert_eq!(
                params.get("thread"),
                Some(&ComponentParamValue::Text("M8".to_string())),
                "source.params is what the worker's table lookup reads"
            ),
            other => panic!("expected a Generator source, got {other:?}"),
        }
    }

    /// The role check runs at the place site too — otherwise a gesture could
    /// author a record whose own edit command would then refuse to touch it.
    #[tokio::test]
    async fn place_with_a_non_free_param_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_params(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let mut free = BTreeMap::new();
        // `head_d` is `role = "table"` in the fixture — derived, never set.
        free.insert("head_d".to_string(), ComponentParamValue::Number(9.0));
        let err = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            free,
            None,
        )
        .await
        .expect_err("a table-role key is not settable at placement either");
        let msg = format!("{err:?}");
        assert!(msg.contains("head_d"), "names the offending key: {msg}");
    }

    /// `set_component_params` re-emits the WHOLE `PlaceComponent` op through
    /// `UpdateOperationParams`, so it inherits the edit-scoped expression rule
    /// for free: a placement bound to a variable that has since been deleted
    /// keeps its broken binding (and its `EXPR_UNRESOLVED` diagnostic) without
    /// making the component's own parameters uneditable. Whole-op validation
    /// would have bricked this path.
    #[tokio::test]
    async fn set_component_params_still_works_after_a_placement_variable_is_deleted() {
        use onecad_core::document::variables::{Unit, Variable};
        use onecad_core::ids::VariableId;

        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_params(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        // A variable the placement will be bound to.
        let var_id = VariableId::new();
        {
            let mut guard = state.runtime.lock().await;
            guard
                .as_mut()
                .unwrap()
                .apply(EditCommand::AddVariable {
                    variable: Variable {
                        id: var_id,
                        name: "gap".into(),
                        value: Scalar::new(12.0),
                        unit: Unit::Mm,
                    },
                })
                .expect("AddVariable gap");
        }

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place_component_at");
        let record_id = projection.features[0].id.clone();
        let record = RecordId::from_str(&record_id).unwrap();

        // Bind the placement's X offset to `gap`, then delete `gap`.
        {
            let mut guard = state.runtime.lock().await;
            let rt = guard.as_mut().unwrap();
            let mut current: PlaceComponentParams =
                serde_json::from_value(rt.operation_params(record).unwrap()).unwrap();
            current.placement.translate[0] = Scalar::with_expr(12.0, "gap * 2");
            rt.apply(EditCommand::UpdateOperationParams {
                record,
                op: Operation::Known(KnownOperation::PlaceComponent(current)),
            })
            .expect("bind the placement to `gap`");
            rt.apply(EditCommand::RemoveVariable { variable: var_id })
                .expect("RemoveVariable gap");
        }

        // The component's OWN parameters still re-edit, broken binding and all.
        let mut params = BTreeMap::new();
        params.insert(
            "thread".to_string(),
            ComponentParamValue::Text("M8".to_string()),
        );
        set_component_params_at(dir.path(), state, record_id.clone(), params)
            .await
            .expect("a broken placement binding must not brick setComponentParams");

        let guard = state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let updated: PlaceComponentParams =
            serde_json::from_value(rt.operation_params(record).unwrap()).unwrap();
        assert_eq!(
            updated.placement.translate[0].expr.as_deref(),
            Some("gap * 2"),
            "the binding is preserved verbatim, not silently rewritten"
        );
        // …and it is still reported broken, per-step, on the projection.
        let feature = rt
            .projection()
            .features
            .into_iter()
            .find(|f| f.id == record_id)
            .expect("the placement row");
        assert!(
            feature
                .diagnostics
                .iter()
                .any(|d| d.code == "EXPR_UNRESOLVED"),
            "the broken binding keeps surfacing as a diagnostic: {:?}",
            feature.diagnostics
        );
    }

    #[tokio::test]
    async fn set_component_params_merges_a_free_override_and_reaches_source_params() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package_with_params(dir.path(), "onecad.std.iso4762", "SHCS");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
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
            set_component_params_at(dir.path(), state, record_id.clone(), params)
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

        let state = &app_state;

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "onecad.std.iso4762".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place_component_at");
        let record_id = projection.features[0].id.clone();

        let mut params = BTreeMap::new();
        params.insert("head_d".to_string(), ComponentParamValue::Number(6.0));
        let err = set_component_params_at(dir.path(), state, record_id.clone(), params)
            .await
            .expect_err("head_d is role=table, not free");
        assert!(
            matches!(err, ApiError::InvalidCommand(ref m) if m.contains("not a free parameter")),
            "got {err:?}"
        );

        let mut params = BTreeMap::new();
        params.insert("nope".to_string(), ComponentParamValue::Number(1.0));
        let err = set_component_params_at(dir.path(), state, record_id, params)
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
        write_document_package_with_free_param(root, id, geometry, "")
    }

    /// [`write_document_package`] plus a verbatim `[parameters]` row, so a test
    /// can author the exact shape the re-bake lane's refusals key on.
    fn write_document_package_with_free_param(
        root: &Path,
        id: &str,
        geometry: &[u8],
        parameters: &str,
    ) -> String {
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

[parameters]
{parameters}
"#,
            zeros = "0".repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
        stamp_package(&dir);
        sha
    }

    /// A `document` component places as its BAKED solid, and the bytes land in
    /// the placing document's own carrier — the spec §4 promise that a document
    /// renders with the library folder gone. The record keeps the authoring
    /// document's digest as provenance, and the two digests must not be swapped
    /// (one is what regen reads, the other is only evidence).
    #[tokio::test]
    async fn failed_restage_keeps_captured_bytes_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let geometry = b"isolated failed-restage retry geometry";
        write_document_package(dir.path(), "acme.bracket", geometry);
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let req = PlaceComponentRequest {
            component_id: "acme.bracket".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: None,
        };
        let draft = prepare_component_draft_at(dir.path(), &state, req)
            .await
            .unwrap();
        let workspace = state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .import_workspace_dir()
            .to_path_buf();
        // The workspace directory belongs to every live document in this process.
        // Obstruct only this test's unique digest, never the shared directory.
        let blob = draft.source.blob_ref().unwrap();
        let path = workspace.join(format!("{}.{}", blob.sha256, blob.codec.extension()));
        // A concurrent last-holder drop may remove an otherwise empty directory.
        let sentinel = tempfile::NamedTempFile::new_in(&workspace).unwrap();
        std::fs::write(sentinel.path(), b"unrelated live source").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path)
            .unwrap_or_else(|error| panic!("obstruct {}: {error}", path.display()));
        let retained_before = captured_component_draft_bytes(&state);
        assert!(retained_before > geometry.len());
        assert!(restage_component_blob_at(dir.path(), &state, &draft)
            .await
            .is_err());
        assert_eq!(captured_component_draft_bytes(&state), retained_before);
        assert_eq!(
            std::fs::read(sentinel.path()).unwrap(),
            b"unrelated live source"
        );
        std::fs::remove_dir(&path).unwrap();
        restage_component_blob_at(dir.path(), &state, &draft)
            .await
            .unwrap();
        assert_eq!(
            captured_component_draft_bytes(&state),
            retained_before - geometry.len()
        );
        assert!(captured_component_draft_session(&state, &draft).is_some());
        clear_component_draft_capture(&state);
    }

    #[tokio::test]
    async fn captured_draft_over_native_source_limit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let oversized = vec![7u8; 32 * 1024 * 1024 + 1];
        write_document_package(dir.path(), "acme.bracket", &oversized);
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let req = PlaceComponentRequest {
            component_id: "acme.bracket".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: None,
        };
        let error = prepare_component_draft_at(dir.path(), &state, req)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("32 MiB"), "{error}");
        assert_eq!(captured_component_draft_bytes(&state), 0);
        assert!(state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .import_blob_shas()
            .is_empty());
    }

    #[tokio::test]
    async fn captured_draft_reports_its_size_and_can_be_released() {
        let dir = tempfile::tempdir().unwrap();
        write_document_package(dir.path(), "acme.bracket", b"captured geometry");
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let req = PlaceComponentRequest {
            component_id: "acme.bracket".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: None,
        };
        prepare_component_draft_at(dir.path(), &state, req)
            .await
            .unwrap();
        assert!(captured_component_draft_bytes(&state) > b"captured geometry".len());
        clear_component_draft_capture(&state);
        assert_eq!(captured_component_draft_bytes(&state), 0);
    }

    #[tokio::test]
    async fn a_document_component_places_its_baked_solid_and_caches_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let geometry = b"pretend baked brep bytes".to_vec();
        let geometry_sha = write_document_package(dir.path(), "acme.bracket", &geometry);
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place a document-source component");
        assert_eq!(projection.features[0].op_type, "PlaceComponent");

        let guard = app_state;
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

        let state = &app_state;

        let source = resolve_component_source_at(
            dir.path(),
            state,
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
            state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
        )
        .await
        .expect("re-arm");

        let guard = app_state;
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
        stamp_package(&pkg);
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let err = place_component_at(
            dir.path(),
            state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
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

    /// A key the package does not declare is refused at the SIGNATURE check,
    /// before the re-bake lane spends a worker on it — a `document` component
    /// that declares no parameters has nothing editable, exactly like a
    /// generator that declares none.
    #[tokio::test]
    async fn setting_an_undeclared_param_on_a_document_component_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_document_package(dir.path(), "acme.bracket", b"pretend baked brep bytes");
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place");
        let record_id = projection.features[0].id.clone();

        // The package declares no parameters at all, so this is refused at the
        // signature check first; declare one to reach the source arm.
        let mut params = BTreeMap::new();
        params.insert("length".to_string(), ComponentParamValue::Number(30.0));
        let err = set_component_params_at(dir.path(), state, record_id, params)
            .await
            .expect_err("a key the package never declared is not editable");
        assert!(matches!(err, ApiError::InvalidCommand(_)), "got {err:?}");
    }

    /// The `length` a `profile` source currently carries.
    fn profile_length(source: &ComponentSourceRef) -> f64 {
        match source {
            ComponentSourceRef::Profile { params, .. } => match params.get("length") {
                Some(ComponentParamValue::Number(v)) => *v,
                other => panic!("profile source has no numeric length: {other:?}"),
            },
            other => panic!("expected a Profile source, got {other:?}"),
        }
    }

    fn write_profile_package(root: &Path, id: &str, face: &[u8]) -> String {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let sha = onecad_library::blob::BlobStore::new(root)
            .put(face)
            .expect("blob put");
        let toml = format!(
            r#"
[identity]
id = "{id}"
version = "1.0.0"
revision = "sha256:{zeros}"

[metadata]
name = "2020 extrusion"

[source]
kind = "profile"
blob = "sha256:{sha}"
codec = "brep"
format = 4

[parameters]
length = {{ role = "free", key = "length", value = 500.0, min = 1.0 }}
"#,
            zeros = "0".repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
        stamp_package(&dir);
        sha
    }

    async fn profile_state(dir: &Path) -> (AppState, String) {
        let sha = write_profile_package(dir, "acme.extrusion.2020", b"pretend canonical face");
        reindex_library_at(dir).unwrap();
        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        (app_state, sha)
    }

    #[tokio::test]
    async fn a_profile_placement_with_no_free_params_lands_the_packages_declared_length() {
        let dir = tempfile::tempdir().unwrap();
        let (app_state, face_sha) = profile_state(dir.path()).await;
        let state = &app_state;

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "acme.extrusion.2020".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place a profile component at its declared default");

        let rt_guard = state.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        assert_eq!(
            rt.import_blob_shas(),
            vec![face_sha.clone()],
            "the canonical FACE is cached in the placing document, like every other blob kind"
        );
        let record = RecordId::from_str(&projection.features[0].id).unwrap();
        let params: PlaceComponentParams =
            serde_json::from_value(rt.operation_params(record).unwrap()).unwrap();
        assert_eq!(
            profile_length(&params.source),
            500.0,
            "a placement that overrides nothing must still carry a complete length"
        );
        assert_eq!(params.source.blob_ref().unwrap().sha256, face_sha);
    }

    /// WP-C's two load-bearing app-crate arms, through the SAME timeline
    /// surface every other kind uses — no worker needed, because
    /// `place_component_at` / `set_component_params_at` are pure timeline
    /// mutations (regen-level proof is `component_ops.rs`'s job, against the
    /// real worker).
    ///
    /// 1. A gesture length MERGES over the package default rather than being
    ///    refused: the non-generator arm used to hard-error, so a 300 mm stick
    ///    could not be placed at all.
    /// 2. A re-length is an in-place param edit with NO re-bake — the blob
    ///    digest must not move, which is the whole difference from a `document`
    ///    source (whose edit replays an authoring document and swaps the blob).
    #[tokio::test]
    async fn a_profile_length_merges_at_placement_and_re_lengths_without_a_re_bake() {
        let dir = tempfile::tempdir().unwrap();
        let (app_state, face_sha) = profile_state(dir.path()).await;
        let state = &app_state;

        let mut at_120 = BTreeMap::new();
        at_120.insert("length".to_string(), ComponentParamValue::Number(120.0));
        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "acme.extrusion.2020".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            at_120,
            None,
        )
        .await
        .expect("a gesture length must PLACE, not hard-error");
        let record_id = projection.features[0].id.clone();
        let record = RecordId::from_str(&record_id).unwrap();

        {
            let rt_guard = state.runtime.lock().await;
            let rt = rt_guard.as_ref().unwrap();
            let params: PlaceComponentParams =
                serde_json::from_value(rt.operation_params(record).unwrap()).unwrap();
            assert_eq!(profile_length(&params.source), 120.0);
        }

        let mut at_300 = BTreeMap::new();
        at_300.insert("length".to_string(), ComponentParamValue::Number(300.0));
        set_component_params_at(dir.path(), state, record_id, at_300)
            .await
            .expect("re-length the placed instance");

        let rt_guard = state.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        let params: PlaceComponentParams =
            serde_json::from_value(rt.operation_params(record).unwrap()).unwrap();
        assert_eq!(profile_length(&params.source), 300.0);
        assert_eq!(
            params.params.get("length"),
            Some(&ComponentParamValue::Number(300.0)),
            "the instance's own override tracks the source"
        );
        assert_eq!(
            params.source.blob_ref().unwrap().sha256,
            face_sha,
            "a length edit re-prisms the SAME face: no re-bake, so the digest must not move"
        );
        assert_eq!(
            rt.import_blob_shas(),
            vec![face_sha],
            "and no second blob was staged"
        );
    }

    /// `length` is a REGEN INPUT, so an impossible one dies at the AUTHORING
    /// door (`PlaceComponentParams::validate` via the edit session), not at the
    /// next regen on someone else's machine.
    #[tokio::test]
    async fn a_profile_re_length_to_an_impossible_value_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (app_state, _sha) = profile_state(dir.path()).await;
        let state = &app_state;

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "acme.extrusion.2020".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place");
        let record_id = projection.features[0].id.clone();

        for bad in [-1.0, 0.0, 1e5 + 1.0] {
            let mut params = BTreeMap::new();
            params.insert("length".to_string(), ComponentParamValue::Number(bad));
            let err = set_component_params_at(dir.path(), state, record_id.clone(), params)
                .await
                .expect_err("an impossible length must be refused at the edit");
            let msg = format!("{err:?}");
            assert!(
                msg.contains("length"),
                "length {bad}: the refusal must name the parameter, got {msg}"
            );
        }
    }

    /// A `role = "free"` parameter with no `key` names no source variable, so
    /// the re-bake has nothing to set. Refused BY NAME and before any worker is
    /// spawned — guessing a variable (the first one, the same-named one) is the
    /// silent mis-bind spec §0 invariant 4 forbids.
    #[tokio::test]
    async fn a_free_param_naming_no_source_variable_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        write_document_package_with_free_param(
            dir.path(),
            "acme.bracket",
            b"pretend baked brep bytes",
            // `role = "free"` and nothing else: no `key`, so nothing to set.
            r#"depth = { role = "free", value = 10.0 }"#,
        );
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let (_outcome, projection) = place_component_at(
            dir.path(),
            state,
            "acme.bracket".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect("place");
        let record_id = projection.features[0].id.clone();

        let mut params = BTreeMap::new();
        params.insert("depth".to_string(), ComponentParamValue::Number(30.0));
        let err = set_component_params_at(dir.path(), state, record_id, params)
            .await
            .expect_err("a free param that names no variable cannot be re-baked");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("depth") && msg.contains("key"),
            "the refusal must name the parameter and what it is missing: {msg}"
        );
    }

    /// Same shape as [`write_profile_package`], minus the `[parameters]` table
    /// entirely — a package that never declares `length` at all.
    fn write_profile_package_without_length(root: &Path, id: &str, face: &[u8]) -> String {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let sha = onecad_library::blob::BlobStore::new(root)
            .put(face)
            .expect("blob put");
        let toml = format!(
            r#"
[identity]
id = "{id}"
version = "1.0.0"
revision = "sha256:{zeros}"

[metadata]
name = "2020 extrusion, no declared length"

[source]
kind = "profile"
blob = "sha256:{sha}"
codec = "brep"
format = 4
"#,
            zeros = "0".repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
        stamp_package(&dir);
        sha
    }

    /// `component_source_from_resolved` seeds a `profile` record's `params` from
    /// `declared_numeric_params(&entry.parameters)` — the package's own
    /// `[parameters]` table. A package that declares NO `length` at all seeds an
    /// EMPTY map, and `PlaceComponentParams::validate` (`validate_component_source`)
    /// catches that at `AddOperation`: `length` is a REGEN INPUT with no
    /// worker-side fallback, so an incomplete record must be refused BY NAME at
    /// authoring, not left to fail obscurely at the next regen.
    #[tokio::test]
    async fn a_profile_package_without_a_declared_length_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        write_profile_package_without_length(
            dir.path(),
            "acme.extrusion.nolength",
            b"pretend canonical face",
        );
        reindex_library_at(dir.path()).unwrap();

        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        let state = &app_state;

        let err = place_component_at(
            dir.path(),
            state,
            "acme.extrusion.nolength".to_string(),
            "1.0.0".to_string(),
            [0.0, 0.0, 0.0],
            None,
            Default::default(),
            None,
        )
        .await
        .expect_err("a profile package with no declared length cannot author a complete record");
        assert!(
            err.to_string().contains("length"),
            "the refusal must name the missing key, got: {err}"
        );

        let guard = app_state;
        let rt_guard = guard.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        assert!(
            rt.projection().features.is_empty(),
            "fail closed: nothing was authored"
        );
    }

    fn write_param_package(root: &Path, id: &str, version: &str, params: &str) {
        let dir = root.join(format!("{id}@{version}"));
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"
[identity]
id = "{id}"
version = "{version}"
revision = "sha256:{}"

[metadata]
name = "Part"
standard = "ACME 1"
category = ["parts"]

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1

[parameters]
{params}
"#,
            "0".repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
        stamp_package(&dir);
    }

    const VALIDATED_PARAMS: &str = r#"
thread = { role = "free", value = "M6", domain = ["M3", "M6", "M8"] }
length = { role = "free", value = 20, snap = "preferred", min = 2 }
tolerance = { role = "free", min = 0.5 }
modeled = { role = "free", value = false }
sizes = { role = "free", value = 10, domain = [10, 12.5] }
head_d = { role = "table", from = "iso4762.dk" }
"#;

    fn validated_package() -> ComponentPackage {
        let toml = format!(
            r#"
[identity]
id = "acme.part"
version = "1.0.0"
revision = "sha256:{}"
[metadata]
name = "Part"
[source]
kind = "generator"
generator = "g"
generator_version = 1
[parameters]
{VALIDATED_PARAMS}
"#,
            "0".repeat(64)
        );
        package::parse(&toml, Path::new("component.toml")).unwrap()
    }

    fn check(key: &str, value: ComponentParamValue) -> Result<(), ApiError> {
        let params = BTreeMap::from([(key.to_string(), value)]);
        validate_component_params("test", &validated_package(), "acme.part", &params)
    }

    fn refusal(key: &str, value: ComponentParamValue) -> String {
        check(key, value).expect_err("must be refused").to_string()
    }

    #[test]
    fn validate_accepts_values_that_meet_every_declared_constraint() {
        let params = BTreeMap::from([
            ("thread".to_string(), ComponentParamValue::Text("M8".into())),
            ("length".to_string(), ComponentParamValue::Number(2.0)),
            ("tolerance".to_string(), ComponentParamValue::Number(0.5)),
            ("modeled".to_string(), ComponentParamValue::Bool(true)),
            ("sizes".to_string(), ComponentParamValue::Number(12.5)),
        ]);
        validate_component_params("test", &validated_package(), "acme.part", &params).unwrap();
        // A float domain entry tolerates representation noise.
        check("sizes", ComponentParamValue::Number(12.5 + 1e-12)).unwrap();
        assert!(refusal("sizes", ComponentParamValue::Number(12.5001)).contains("domain"));
        // An integer domain entry equals the same number as f64.
        check("sizes", ComponentParamValue::Number(10.0)).unwrap();
        // `snap = "preferred"` is a UI hint: an off-grid length passes.
        check("length", ComponentParamValue::Number(23.7)).unwrap();
    }

    #[test]
    fn validate_keeps_the_role_checks() {
        assert!(refusal("head_d", ComponentParamValue::Number(1.0)).contains("not a free"));
        assert!(refusal("nope", ComponentParamValue::Number(1.0)).contains("unknown parameter"));
    }

    #[test]
    fn validate_refuses_a_kind_that_differs_from_the_declared_default() {
        assert!(refusal("length", ComponentParamValue::Text("20".into())).contains("same kind"));
        assert!(refusal("thread", ComponentParamValue::Number(6.0)).contains("same kind"));
        assert!(refusal("modeled", ComponentParamValue::Number(1.0)).contains("same kind"));
    }

    #[test]
    fn validate_refuses_non_finite_numbers() {
        assert!(refusal("length", ComponentParamValue::Number(f64::NAN)).contains("finite"));
        assert!(refusal("length", ComponentParamValue::Number(f64::INFINITY)).contains("finite"));
    }

    #[test]
    fn validate_refuses_a_value_outside_the_domain() {
        assert!(refusal("thread", ComponentParamValue::Text("M7".into())).contains("domain"));
        assert!(refusal("sizes", ComponentParamValue::Number(11.0)).contains("domain"));
    }

    #[test]
    fn validate_refuses_below_min_and_non_numbers_where_min_is_declared() {
        assert!(refusal("length", ComponentParamValue::Number(1.9)).contains("minimum"));
        // No declared default here, so only the `min` rule can catch a Text.
        assert!(refusal("tolerance", ComponentParamValue::Text("1".into())).contains("number"));
    }

    #[test]
    fn search_filters_and_sorts_through_the_app_dto() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.9.0", VALIDATED_PARAMS);
        write_param_package(dir.path(), "acme.part", "1.10.0", VALIDATED_PARAMS);
        reindex_library_at(dir.path()).unwrap();

        let all = search_library_components_at(
            dir.path(),
            &SearchQueryDto {
                text: Some(" acme ".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let versions: Vec<_> = all.iter().map(|c| c.version.as_str()).collect();
        assert_eq!(versions, ["1.9.0", "1.10.0"]);
        assert_eq!(all[0].standard.as_deref(), Some("ACME 1"));
        assert_eq!(all[0].unit.as_deref(), Some("mm"));
        assert!(all[0].preview_path.is_none());

        let latest = search_library_components_at(
            dir.path(),
            &SearchQueryDto {
                latest_only: true,
                category: Some("parts".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].version, "1.10.0");
    }

    #[test]
    fn pinned_detail_refuses_changed_manifest_without_reindex() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.0.0", VALIDATED_PARAMS);
        reindex_library_at(dir.path()).unwrap();
        let library = open_at(dir.path()).unwrap();
        let revision = library
            .get("acme.part", Some("1.0.0"))
            .unwrap()
            .1
            .revision
            .clone();
        assert!(component_detail_pinned_at(dir.path(), "acme.part", "1.0.0", &revision).is_ok());
        let path = dir.path().join("acme.part@1.0.0/component.toml");
        let raw = std::fs::read_to_string(&path)
            .unwrap()
            .replace("name = \"Part\"", "name = \"changed\"");
        std::fs::write(path, raw).unwrap();
        assert!(component_detail_pinned_at(dir.path(), "acme.part", "1.0.0", &revision).is_err());
    }

    #[test]
    fn detail_lists_versions_ascending_and_the_preview_path_only_when_present() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.10.0", VALIDATED_PARAMS);
        write_param_package(dir.path(), "acme.part", "1.9.0", VALIDATED_PARAMS);
        std::fs::write(dir.path().join("acme.part@1.9.0/preview.png"), b"png").unwrap();
        reindex_library_at(dir.path()).unwrap();

        let with = component_detail_at(dir.path(), "acme.part", "1.9.0").unwrap();
        assert_eq!(with.versions, ["1.9.0", "1.10.0"]);
        let path = with.component.preview_path.expect("preview exists");
        assert!(Path::new(&path).is_absolute() && path.ends_with("preview.png"));
        let without = component_detail_at(dir.path(), "acme.part", "1.10.0").unwrap();
        assert!(without.component.preview_path.is_none());
        assert!(component_detail_at(dir.path(), "acme.part", "9.9.9").is_err());
    }

    fn placed_params(version: &str, revision: &str) -> PlaceComponentParams {
        serde_json::from_value(serde_json::json!({
            "componentId": "acme.part",
            "componentVersion": version,
            "componentRevision": revision,
            "params": {},
            "source": {"kind": "generator", "generatorId": "iso4762",
                       "generatorVersion": 1, "params": {}},
            "placement": {"translate": [{"value": 0.0}, {"value": 0.0}, {"value": 0.0}]},
        }))
        .expect("PlaceComponentParams json")
    }

    #[test]
    fn status_walks_missing_mismatch_upgrade_linked() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.0.0", VALIDATED_PARAMS);
        reindex_library_at(dir.path()).unwrap();
        let zero = open_at(dir.path())
            .unwrap()
            .declared_revision("acme.part", "1.0.0")
            .unwrap();

        let status = |p: &PlaceComponentParams| component_status_for(dir.path(), p);
        assert_eq!(
            status(&placed_params("1.0.0", &zero)),
            ComponentStatusDto::Linked
        );
        assert_eq!(
            status(&placed_params("2.0.0", &zero)),
            ComponentStatusDto::DefinitionMissing
        );
        assert_eq!(
            status(&placed_params("1.0.0", "sha256:old")),
            ComponentStatusDto::RevisionMismatch {
                recorded: "sha256:old".into(),
                current: zero.clone()
            }
        );

        write_param_package(dir.path(), "acme.part", "1.1.0", VALIDATED_PARAMS);
        reindex_library_at(dir.path()).unwrap();
        assert_eq!(
            status(&placed_params("1.0.0", &zero)),
            ComponentStatusDto::UpgradeAvailable {
                version: "1.1.0".into()
            }
        );
        // Mismatch outranks an available upgrade.
        assert!(matches!(
            status(&placed_params("1.0.0", "sha256:old")),
            ComponentStatusDto::RevisionMismatch { .. }
        ));
    }

    #[test]
    fn status_reports_an_unreadable_manifest_distinctly_from_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.0.0", VALIDATED_PARAMS);
        reindex_library_at(dir.path()).unwrap();
        std::fs::write(
            dir.path().join("acme.part@1.0.0/component.toml"),
            "not [[ toml",
        )
        .unwrap();
        let zero = format!("sha256:{}", "0".repeat(64));
        assert!(matches!(
            component_status_for(dir.path(), &placed_params("1.0.0", &zero)),
            ComponentStatusDto::DefinitionUnreadable { .. }
        ));
        assert_eq!(
            component_status_for(dir.path(), &placed_params("3.0.0", &zero)),
            ComponentStatusDto::DefinitionMissing
        );
    }

    #[test]
    fn status_reads_the_manifest_fresh_not_the_cached_index() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.0.0", VALIDATED_PARAMS);
        reindex_library_at(dir.path()).unwrap();
        let zero = format!("sha256:{}", "0".repeat(64));
        let manifest = dir.path().join("acme.part@1.0.0/component.toml");
        let edited = std::fs::read_to_string(&manifest)
            .unwrap()
            .replace(&zero, &format!("sha256:{}", "1".repeat(64)));
        std::fs::write(&manifest, edited).unwrap();

        assert!(matches!(
            component_status_for(dir.path(), &placed_params("1.0.0", &zero)),
            ComponentStatusDto::RevisionMismatch { .. }
        ));
    }

    #[tokio::test]
    async fn statuses_cover_placed_and_detached_records_and_split_prepare_apply_agree() {
        let dir = tempfile::tempdir().unwrap();
        write_generator_package(dir.path(), "acme.part", "Part");
        reindex_library_at(dir.path()).unwrap();
        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));

        // Prepare is side-effect free: nothing is authored until apply.
        let prepared = prepare_place_component_at(
            dir.path(),
            &app_state,
            PlaceComponentRequest {
                component_id: "acme.part".into(),
                component_version: "1.0.0".into(),
                translate: [1.0, 2.0, 3.0],
                rotate: None,
                params: BTreeMap::new(),
                mate: None,
                configuration: None,
            },
        )
        .await
        .unwrap();
        assert!(prepared.blob.is_none());
        stage_prepared_blob(&app_state, &prepared).await.unwrap();
        assert!(app_state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .projection()
            .features
            .is_empty());
        let (_, projection) = stage_blob_and_apply(
            &app_state,
            prepared.blob,
            prepared.command,
            &prepared.runtime_session,
        )
        .await
        .unwrap();
        let placed_id = projection.features[0].id.clone();

        let statuses = component_statuses_at(Some(dir.path()), &app_state)
            .await
            .unwrap();
        assert_eq!(statuses, [(placed_id.clone(), ComponentStatusDto::Linked)]);

        detach_component_at(&app_state, placed_id.clone())
            .await
            .unwrap();
        let statuses = component_statuses_at(Some(dir.path()), &app_state)
            .await
            .unwrap();
        assert_eq!(statuses, [(placed_id, ComponentStatusDto::Detached)]);
    }

    const CONFIGURED_PARAMS: &str = r#"
length = { role = "free", value = 20, min = 2 }
head_d = { role = "table", from = "iso4762.dk" }

[configurations.Long]
length = 50

[configurations."Short 1"]
length = 5
"#;

    fn configured_package() -> ComponentPackage {
        write_package_text(CONFIGURED_PARAMS)
    }

    fn write_package_text(params: &str) -> ComponentPackage {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.0.0", params);
        let path = dir.path().join("acme.part@1.0.0/component.toml");
        package::parse(&std::fs::read_to_string(&path).unwrap(), &path).unwrap()
    }

    #[test]
    fn a_configuration_resolves_to_params_under_explicit_overrides() {
        let package = configured_package();
        let merged =
            configured_params(&package, "acme.part", "Long", BTreeMap::new()).expect("Long");
        assert_eq!(merged["length"], ComponentParamValue::Number(50.0));
        let explicit = BTreeMap::from([("length".to_string(), ComponentParamValue::Number(33.0))]);
        let merged = configured_params(&package, "acme.part", "Long", explicit).expect("Long");
        assert_eq!(merged["length"], ComponentParamValue::Number(33.0));
        let err = configured_params(&package, "acme.part", "Ghost", BTreeMap::new())
            .expect_err("unknown configuration")
            .to_string();
        assert!(err.contains("Ghost") && err.contains("Long"), "{err}");
    }

    #[tokio::test]
    async fn a_generator_places_a_configuration_as_its_params() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.0.0", CONFIGURED_PARAMS);
        reindex_library_at(dir.path()).unwrap();
        let listed = list_library_components_at(dir.path()).unwrap();
        assert_eq!(listed[0].configurations.len(), 2);
        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let prepared = prepare_place_component_at(
            dir.path(),
            &app_state,
            PlaceComponentRequest {
                component_id: "acme.part".into(),
                component_version: "1.0.0".into(),
                translate: [0.0; 3],
                rotate: None,
                params: BTreeMap::new(),
                mate: None,
                configuration: Some("Short 1".into()),
            },
        )
        .await
        .unwrap();
        let EditCommand::AddOperation {
            record:
                OperationRecord {
                    op: Operation::Known(KnownOperation::PlaceComponent(params)),
                    ..
                },
            ..
        } = prepared.command
        else {
            panic!("expected a PlaceComponent record");
        };
        assert_eq!(params.params["length"], ComponentParamValue::Number(5.0));
    }

    fn spec_with_configs(
        configs: &[(&str, &[(&str, ComponentParamValue)])],
        default: Option<&str>,
    ) -> NewComponentSpec {
        NewComponentSpec {
            id: "acme.part".into(),
            configurations: configs
                .iter()
                .map(|(name, kv)| {
                    (
                        name.to_string(),
                        kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
                    )
                })
                .collect(),
            default_configuration: default.map(String::from),
            ..Default::default()
        }
    }

    fn bound_parameters() -> BTreeMap<String, ParameterSpec> {
        BTreeMap::from([(
            "width".to_string(),
            ParameterSpec::free_variable("width", 40.0),
        )])
    }

    #[test]
    fn authored_configurations_refuse_bad_values_naming_the_configuration() {
        let ok = spec_with_configs(
            &[("Heavy", &[("width", ComponentParamValue::Number(60.0))])],
            Some("Heavy"),
        );
        let out = authored_configurations(&ok, &bound_parameters()).unwrap();
        assert_eq!(out["Heavy"]["width"], toml::Value::Float(60.0));

        let text = spec_with_configs(
            &[("Heavy", &[("width", ComponentParamValue::Text("x".into()))])],
            None,
        );
        let err = authored_configurations(&text, &bound_parameters())
            .unwrap_err()
            .to_string();
        assert!(err.contains("configuration `Heavy`"), "{err}");

        let unknown = spec_with_configs(
            &[("Heavy", &[("depth", ComponentParamValue::Number(1.0))])],
            None,
        );
        let err = authored_configurations(&unknown, &bound_parameters())
            .unwrap_err()
            .to_string();
        assert!(err.contains("`Heavy`") && err.contains("depth"), "{err}");

        let empty = spec_with_configs(&[("Heavy", &[])], None);
        assert!(authored_configurations(&empty, &bound_parameters()).is_err());
        let bad_name = spec_with_configs(
            &[("a/b", &[("width", ComponentParamValue::Number(1.0))])],
            None,
        );
        assert!(authored_configurations(&bad_name, &bound_parameters()).is_err());
        let ghost_default = spec_with_configs(
            &[("Heavy", &[("width", ComponentParamValue::Number(1.0))])],
            Some("Light"),
        );
        let err = authored_configurations(&ghost_default, &bound_parameters())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Light"), "{err}");
    }

    #[test]
    fn a_static_save_refuses_parameters_and_configurations() {
        let mut spec = NewComponentSpec {
            static_save: true,
            ..Default::default()
        };
        reject_static_save_extras(&spec).unwrap();
        spec.parameters.insert(
            "width".into(),
            FreeParameterInput {
                key: "width".into(),
                value: 1.0,
            },
        );
        assert!(reject_static_save_extras(&spec).is_err());
        spec.parameters.clear();
        spec.default_configuration = Some("X".into());
        assert!(reject_static_save_extras(&spec).is_err());
    }

    #[test]
    fn seat_frames_pick_a_deterministic_perpendicular_x() {
        let at = |z: [f64; 3]| {
            seat_frame(package::AttachmentFrame {
                origin: [1.0, 2.0, 3.0],
                z,
                x: [1.0, 0.0, 0.0],
            })
            .unwrap()
        };
        let up = at([0.0, 0.0, 5.0]);
        assert_eq!(
            (up.z, up.x, up.origin),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [1.0, 2.0, 3.0])
        );
        let along_x = at([-2.0, 0.0, 0.0]);
        assert_eq!((along_x.z, along_x.x), ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
        assert!(seat_frame(package::AttachmentFrame {
            origin: [0.0; 3],
            z: [0.0; 3],
            x: [1.0, 0.0, 0.0],
        })
        .is_none());
    }

    #[test]
    fn scratch_dirs_are_unique_and_removed_on_drop() {
        let (a, b) = (
            ScratchDir::create("test").unwrap(),
            ScratchDir::create("test").unwrap(),
        );
        assert_ne!(a.path(), b.path());
        let path = a.path().to_path_buf();
        assert!(path.is_dir());
        drop(a);
        assert!(!path.exists());
    }

    #[test]
    fn declared_defaults_come_from_the_document_and_derived_variables_are_refused() {
        let spec = |key: &str, value: f64| NewComponentSpec {
            parameters: BTreeMap::from([(
                "p".to_string(),
                FreeParameterInput {
                    key: key.into(),
                    value,
                },
            )]),
            ..Default::default()
        };
        let vars = BTreeMap::from([
            ("width".to_string(), (40.0, false)),
            ("post_h".to_string(), (20.0, true)),
        ]);
        let ok = document_bound_parameters(&spec("width", 999.0), &vars).unwrap();
        assert_eq!(ok["p"].number(), Some(40.0), "the client value is ignored");
        let err = document_bound_parameters(&spec("post_h", 20.0), &vars)
            .unwrap_err()
            .to_string();
        assert!(err.contains("derived from an expression"), "{err}");
        assert!(err.contains("expose its inputs instead"), "{err}");
        assert!(document_bound_parameters(&spec("gone", 1.0), &vars).is_err());
    }

    fn document_package() -> ComponentPackage {
        let toml = format!(
            r#"
[identity]
id = "acme.part"
version = "1.0.0"
revision = "sha256:{}"
[metadata]
name = "Part"
[source]
kind = "document"
file = "source.onecad"
geometry = "{}"
[parameters]
width = {{ role = "free", key = "width", value = 40.0 }}
[attachments]
top = {{ on = "face:top", accepts = ["plane"], frame = {{ origin = [0, 0, 5] }} }}
loose = {{ on = "face:loose", accepts = ["plane"] }}
"#,
            "0".repeat(64),
            "a".repeat(64)
        );
        package::parse(&toml, Path::new("component.toml")).unwrap()
    }

    #[test]
    fn a_framed_attachment_refuses_a_non_default_variant_only() {
        let package = document_package();
        let width =
            |w: f64| BTreeMap::from([("width".to_string(), ComponentParamValue::Number(w))]);
        refuse_variant_mate(&package, "top", &BTreeMap::new()).unwrap();
        refuse_variant_mate(&package, "top", &width(40.0)).unwrap();
        refuse_variant_mate(&package, "loose", &width(60.0)).unwrap();
        let err = refuse_variant_mate(&package, "top", &width(60.0))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(
                "Mating a non-default variant of an authored component isn't supported yet"
            ),
            "{err}"
        );
    }

    async fn reopen_same_document(state: &AppState) -> (String, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same.onecad");
        let (document_id, session) = {
            let mut guard = state.runtime.lock().await;
            let rt = guard.as_mut().unwrap();
            rt.save(&path, crate::api::save_meta()).unwrap();
            (rt.document_id(), rt.runtime_session())
        };
        let (engine, meshes, solver) = state.make_backend();
        let reopened = DocumentRuntime::open(&path, engine, meshes, solver).unwrap();
        assert_eq!(reopened.document_id(), document_id);
        assert_ne!(reopened.runtime_session(), session);
        *state.runtime.lock().await = Some(reopened);
        (document_id, session)
    }

    #[tokio::test]
    async fn prepared_draft_rejects_same_document_reopen_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        write_document_package(dir.path(), "acme.bracket", b"captured geometry");
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let req = || PlaceComponentRequest {
            component_id: "acme.bracket".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: None,
        };
        let generation = state
            .captured_library_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let prepared = prepare_place_component_at(dir.path(), &state, req())
            .await
            .unwrap();
        reopen_same_document(&state).await;
        assert!(matches!(
            finish_component_draft(&state, prepared, generation).await,
            Err(ApiError::StalePreview(_))
        ));
        assert!(state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .import_blob_shas()
            .is_empty());
        let generation = state
            .captured_library_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let prepared = prepare_place_component_at(dir.path(), &state, req())
            .await
            .unwrap();
        clear_component_draft_capture(&state);
        assert!(matches!(
            finish_component_draft(&state, prepared, generation).await,
            Err(ApiError::StalePreview(_))
        ));
        assert_eq!(captured_component_draft_bytes(&state), 0);
    }

    #[tokio::test]
    async fn equivalent_manifest_format_reuses_first_saved_archive() {
        let dir = tempfile::tempdir().unwrap();
        write_document_package(dir.path(), "acme.bracket", b"format invariant geometry");
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        for iteration in 0..2 {
            if iteration == 1 {
                let manifest = dir.path().join("acme.bracket/component.toml");
                let mut raw = std::fs::read_to_string(&manifest).unwrap();
                raw.push_str("\n# non-semantic formatting\n");
                std::fs::write(manifest, raw).unwrap();
            }
            place_component_at(
                dir.path(),
                &state,
                "acme.bracket".into(),
                "1.0.0".into(),
                [0.0; 3],
                None,
                Default::default(),
                None,
            )
            .await
            .unwrap();
            let guard = state.runtime.lock().await;
            let rt = guard.as_ref().unwrap();
            assert_eq!(rt.library_archive_checkpoint().0.len(), 1);
        }
    }

    #[tokio::test]
    async fn distinct_cancelled_drafts_do_not_stage_document_imports() {
        let dir = tempfile::tempdir().unwrap();
        let components = [
            ("acme.first", b"first captured geometry".as_slice()),
            ("acme.second", b"second captured geometry".as_slice()),
            ("acme.third", b"third captured geometry".as_slice()),
        ];
        for (id, bytes) in components {
            write_document_package(dir.path(), id, bytes);
        }
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        for (id, bytes) in components {
            let req = PlaceComponentRequest {
                component_id: id.into(),
                component_version: "1.0.0".into(),
                translate: [0.0; 3],
                rotate: None,
                params: Default::default(),
                mate: None,
                configuration: None,
            };
            prepare_component_draft_at(dir.path(), &state, req)
                .await
                .unwrap();
            assert!(captured_component_draft_bytes(&state) > bytes.len());
            let runtime = state.runtime.lock().await;
            assert!(runtime.as_ref().unwrap().import_blob_shas().is_empty());
            drop(runtime);
            clear_component_draft_capture(&state);
            assert_eq!(captured_component_draft_bytes(&state), 0);
            let runtime = state.runtime.lock().await;
            assert!(runtime.as_ref().unwrap().import_blob_shas().is_empty());
            drop(runtime);
        }
    }

    #[tokio::test]
    async fn cancellation_while_draft_preparation_is_pending_refuses_late_result() {
        let dir = tempfile::tempdir().unwrap();
        write_document_package(dir.path(), "acme.bracket", b"captured geometry");
        reindex_library_at(dir.path()).unwrap();
        let state = std::sync::Arc::new(pending_app_state());
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let req = PlaceComponentRequest {
            component_id: "acme.bracket".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: None,
        };
        let generation = state
            .captured_library_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let prepared = prepare_place_component_at(dir.path(), &state, req)
            .await
            .unwrap();
        let (release, pending) = tokio::sync::oneshot::channel();
        let held = state.clone();
        let task = tokio::spawn(async move {
            finish_draft_when_ready(
                &held,
                async {
                    pending.await.unwrap();
                    Ok(prepared)
                },
                generation,
            )
            .await
        });
        tokio::task::yield_now().await;
        clear_component_draft_capture(&state);
        release.send(()).unwrap();
        assert!(matches!(
            task.await.unwrap(),
            Err(ApiError::StalePreview(_))
        ));
        assert!(state
            .runtime
            .lock()
            .await
            .as_ref()
            .unwrap()
            .import_blob_shas()
            .is_empty());
        assert_eq!(captured_component_draft_bytes(&state), 0);
    }

    #[tokio::test]
    async fn draft_restage_rejects_same_document_reopen() {
        let dir = tempfile::tempdir().unwrap();
        write_document_package(dir.path(), "acme.bracket", b"captured geometry");
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let req = PlaceComponentRequest {
            component_id: "acme.bracket".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: None,
        };
        let draft = prepare_component_draft_at(dir.path(), &state, req)
            .await
            .unwrap();
        let (_, old_session) = reopen_same_document(&state).await;
        assert_eq!(
            captured_component_draft_session(&state, &draft),
            Some(old_session)
        );
        assert!(matches!(
            restage_component_blob_at(dir.path(), &state, &draft).await,
            Err(ApiError::StalePreview(_))
        ));
        assert!(captured_component_draft_bytes(&state) > b"captured geometry".len());
    }

    #[tokio::test]
    async fn stale_params_result_cannot_edit_same_document_reopen() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.0.0", VALIDATED_PARAMS);
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let (_, projection) = place_component_at(
            dir.path(),
            &state,
            "acme.part".into(),
            "1.0.0".into(),
            [0.0; 3],
            None,
            BTreeMap::new(),
            None,
        )
        .await
        .unwrap();
        let record = RecordId::from_str(&projection.features[0].id).unwrap();
        let current = {
            let guard = state.runtime.lock().await;
            serde_json::from_value::<PlaceComponentParams>(
                guard.as_ref().unwrap().operation_params(record).unwrap(),
            )
            .unwrap()
        };
        let (_, old_session) = reopen_same_document(&state).await;
        let live = {
            let guard = state.runtime.lock().await;
            guard.as_ref().unwrap().operation_params(record).unwrap()
        };
        assert_eq!(
            serde_json::from_value::<PlaceComponentParams>(live).unwrap(),
            current
        );
        let command = EditCommand::UpdateOperationParams {
            record,
            op: Operation::Known(KnownOperation::PlaceComponent(current.clone())),
        };
        assert!(matches!(
            stage_blob_and_apply_unchanged(&state, None, command, (record, &current), &old_session)
                .await,
            Err(ApiError::StalePreview(_))
        ));
    }

    #[tokio::test]
    async fn a_params_edit_computed_from_an_old_record_is_refused_as_stale() {
        let dir = tempfile::tempdir().unwrap();
        write_param_package(dir.path(), "acme.part", "1.0.0", VALIDATED_PARAMS);
        reindex_library_at(dir.path()).unwrap();
        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let (_, projection) = place_component_at(
            dir.path(),
            &app_state,
            "acme.part".into(),
            "1.0.0".into(),
            [0.0; 3],
            None,
            BTreeMap::new(),
            None,
        )
        .await
        .unwrap();
        let id = projection.features[0].id.clone();
        let record = RecordId::from_str(&id).unwrap();
        let old = {
            let guard = app_state.runtime.lock().await;
            let raw = guard.as_ref().unwrap().operation_params(record).unwrap();
            serde_json::from_value::<PlaceComponentParams>(raw).unwrap()
        };
        // Another edit lands first.
        set_component_params_at(
            dir.path(),
            &app_state,
            id,
            BTreeMap::from([("length".to_string(), ComponentParamValue::Number(33.0))]),
        )
        .await
        .unwrap();
        let op = Operation::Known(KnownOperation::PlaceComponent(old.clone()));
        let session = current_runtime_session(&app_state, "test").await.unwrap();
        let err = stage_blob_and_apply_unchanged(
            &app_state,
            None,
            EditCommand::UpdateOperationParams { record, op },
            (record, &old),
            &session,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::StalePreview(_)), "{err}");
    }
}
