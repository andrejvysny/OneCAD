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
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

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

use onecad_library::index::LibraryIndex;
use onecad_library::package::{
    self, ComponentPackage, ParameterRole, ParameterSpec, COMPONENT_MANIFEST_FILE,
};
use onecad_library::resolve::{ResolveRequest, ResolvedBlob, ResolvedSource};
use onecad_library::Library;

use crate::document_runtime::DocumentRuntime;
use crate::dto::{
    ComponentUpgradeDto, DocumentProjection, LibraryComponentDto, PlaceComponentMateInput,
    ProjectTemplateDto, ReindexReportDto, ReplaceComponentReportDto, ReplaceComponentResultDto,
};
use crate::error::ApiError;
use crate::export::GeometryExporter;
use crate::state::AppState;
use crate::worker::manager::SupervisorConfig;
use crate::worker::{resolve_worker_path, MeshProvider, SolverEngine, WorkerManager};

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
pub fn list_library_components_at(root: &Path) -> Result<Vec<LibraryComponentDto>, ApiError> {
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
        Ok(report) => tracing::info!(
            indexed = report.indexed,
            total = report.total,
            "library reindexed after seeding"
        ),
        Err(e) => tracing::warn!("library reindex after seeding failed: {e}"),
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
/// Builds the recorded [`ComponentMate`] from the gesture's raw pick evidence
/// plus the promoted (or pick-carried) element id. Pure — promotion happens at
/// the call site, under the runtime lock.
fn component_mate_from_input(
    input: &PlaceComponentMateInput,
    body: BodyId,
    element: ElementId,
    self_frame: Option<MateFrame>,
) -> ComponentMate {
    ComponentMate {
        self_attachment: input.self_attachment.clone(),
        self_frame,
        target: ElementRef {
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
    state: &State<'_, AppState>,
    input: &PlaceComponentMateInput,
    self_frame: Option<MateFrame>,
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
    let anchor = AnchorIntent {
        world_point: Vec3::new_unchecked(
            input.anchor_world_point[0],
            input.anchor_world_point[1],
            input.anchor_world_point[2],
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
    let Some(head) = rt.head_snapshot_id() else {
        return Err(ApiError::InvalidCommand(
            "placeComponent: cannot record a mate before the first publish — re-pick".into(),
        ));
    };
    let promoted = rt
        .promote_selection(
            head,
            body,
            vec![(TopoKey::new(&input.target_topo_key), Some(anchor))],
        )
        .await
        .map_err(|e| {
            ApiError::InvalidCommand(format!(
                "placeComponent: mate target could not be promoted ({e}) — re-pick"
            ))
        })?;
    let Some(first) = promoted.first() else {
        return Err(ApiError::InvalidCommand(format!(
            "placeComponent: mate target `{}` on body {} did not resolve at the head — re-pick",
            input.target_topo_key, input.target_body_id
        )));
    };
    Ok(component_mate_from_input(
        input,
        body,
        ElementId::new(&first.element_id),
        self_frame,
    ))
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

/// The testable core of [`place_component`] — `pub` for the same reason
/// [`save_as_component_at`] is: a worker-backed integration test drives the real
/// command logic rather than re-deriving it (`tauri::test::mock_app` supplies
/// the `State`, which the `#[tauri::command]` wrapper's `AppHandle` cannot be).
// One arg per spec §3.1 field the gesture carries — grouping them into an ad-hoc
// struct would just rename the problem (same precedent as `regen/executor.rs`).
#[allow(clippy::too_many_arguments)]
pub async fn place_component_at(
    root: &Path,
    state: &State<'_, AppState>,
    component_id: String,
    component_version: String,
    translate: [f64; 3],
    rotate: Option<TransformRotation>,
    free_params: BTreeMap<String, ComponentParamValue>,
    mate: Option<PlaceComponentMateInput>,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    // Promote FIRST (its own short lock): identity must exist before the record
    // that names it is authored; `stage_blob_and_apply` re-locks below.
    //
    // The attachment's own local frame is FROZEN into the mate here (WP-F1.1),
    // read from the package the placement is authored against — so the record
    // re-seats identically later even with the library deleted, and a package
    // revision that moves its attachment never silently moves this instance.
    let mate = match &mate {
        Some(input) => {
            let package =
                component_package_at("placeComponent", root, &component_id, &component_version)?;
            let self_frame = attachment_frame(&package, &input.self_attachment);
            Some(resolve_mate_input(state, input, self_frame).await?)
        }
        None => None,
    };
    let library = open_at(root)?;
    let (_version, entry) = library
        .get(&component_id, Some(&component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "placeComponent: unknown component {component_id}@{component_version}"
            ))
        })?;
    let revision = entry.revision.clone();
    let declared = declared_numeric_params(&entry.parameters);
    let resolved = library.resolve_source(&ResolveRequest {
        component_id: &component_id,
        component_version: &component_version,
        component_revision: &revision,
    })?;
    // Role enforcement at the PLACE site, not just the edit site (WP-A3): the
    // interactive gesture now sends free params of its own (auto-size on a hole
    // rim), and a placement that writes a non-free key would author a record
    // `set_component_params` would then refuse to touch.
    if !free_params.is_empty() {
        let package =
            component_package_at("placeComponent", root, &component_id, &component_version)?;
        check_free_params("placeComponent", &package, &component_id, &free_params)?;
    }
    let (source, blob) = component_source_from_resolved(resolved, &component_id, &declared)?;
    let source = source_with_free_params(source, &free_params, &component_id)?;

    let params = PlaceComponentParams {
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

/// Rejects any key that is not `role = "free"` in the component's own
/// signature (spec §3.1/§3.2). `onecad-core` cannot depend on the crate that
/// resolves a signature, so this is the only place the check can live; both
/// the place site and the edit site call it, and `what` names the caller in
/// the message.
fn check_free_params(
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
    Ok(())
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
/// A `document` component's params ARE editable — but only through
/// `set_component_params_at`, which re-bakes (WP-F1.3). The PLACEMENT gesture
/// deliberately has no such lane: it would pay a second worker and a full
/// replay for a drag, and the configurator is one click away afterwards.
fn source_with_free_params(
    source: ComponentSourceRef,
    free_params: &BTreeMap<String, ComponentParamValue>,
    component_id: &str,
) -> Result<ComponentSourceRef, ApiError> {
    if free_params.is_empty() {
        return Ok(source);
    }
    match source {
        ComponentSourceRef::Generator {
            generator_id,
            generator_version,
            extra,
            ..
        } => Ok(ComponentSourceRef::Generator {
            generator_id,
            generator_version,
            params: free_params.clone(),
            extra,
        }),
        ComponentSourceRef::Profile {
            sha256,
            codec,
            brep_format,
            mut params,
            extra,
        } => {
            params.extend(free_params.clone());
            Ok(ComponentSourceRef::Profile {
                sha256,
                codec,
                brep_format,
                params,
                extra,
            })
        }
        ComponentSourceRef::Embedded { .. } | ComponentSourceRef::Document { .. } => {
            Err(ApiError::InvalidCommand(format!(
                "placeComponent: {component_id} carries baked geometry, which has no free \
                 parameters to set"
            )))
        }
    }
}

/// A component's baked geometry, resolved out of the library and ready to be
/// staged into the placing document (WP-3.2).
struct StagedBlob {
    sha256: String,
    codec: ImportSourceCodec,
    format: Option<u32>,
    bytes: Vec<u8>,
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
        // WP-C: the blob is a canonical planar FACE, not a solid, and the
        // placement carries the length the worker prisms it to. That number
        // starts at the package's declared default and is overridden by the
        // gesture's free params (`source_with_free_params`).
        ResolvedSource::Profile { blob } => {
            let staged = staged_blob(blob, component_id)?;
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
            let staged = staged_blob(blob, component_id)?;
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

/// `params` carries the free-parameter overrides chosen during the gesture —
/// auto-size on a hole rim today (WP-A3), the configurator's own values later.
/// Absent ⇒ the generator's declared defaults, byte-identical to every prior
/// caller.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn place_component(
    app: AppHandle,
    state: State<'_, AppState>,
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
        &state,
        component_id,
        component_version,
        translate,
        rotate,
        params.unwrap_or_default(),
        mate,
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
    let declared = declared_numeric_params(&entry.parameters);
    let resolved = library.resolve_source(&ResolveRequest {
        component_id: &component_id,
        component_version: &component_version,
        component_revision: &revision,
    })?;
    let (source, blob) = component_source_from_resolved(resolved, &component_id, &declared)?;
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
    what: &str,
    root: &Path,
    component_id: &str,
    component_version: &str,
) -> Result<ComponentPackage, ApiError> {
    let library = open_at(root)?;
    let (_version, entry) = library
        .get(component_id, Some(component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "{what}: unknown component {component_id}@{component_version}"
            ))
        })?;
    let manifest_path = root.join(&entry.path).join(COMPONENT_MANIFEST_FILE);
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| ApiError::Internal(format!("read {}: {e}", manifest_path.display())))?;
    package::parse(&raw, &manifest_path).map_err(ApiError::from)
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
fn frozen_source_document(
    root: &Path,
    package: &ComponentPackage,
    current: &PlaceComponentParams,
    recorded_document_sha256: &str,
) -> Result<Vec<u8>, ApiError> {
    let library = open_at(root)?;
    let (_version, entry) = library
        .get(&current.component_id, Some(&current.component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "setComponentParams: unknown component {}@{}",
                current.component_id, current.component_version
            ))
        })?;
    let package_dir = root.join(&entry.path);
    library.resolve_source(&ResolveRequest {
        component_id: &current.component_id,
        component_version: &current.component_version,
        component_revision: &current.component_revision,
    })?;

    let package::SourceSpec::Document { file, .. } = &package.source else {
        return Err(ApiError::InvalidCommand(format!(
            "setComponentParams: {} is not a document-source component",
            current.component_id
        )));
    };
    let path = package_dir.join(file);
    let bytes = std::fs::read(&path).map_err(|e| {
        ApiError::Io(format!(
            "setComponentParams: read frozen source {}: {e}",
            path.display()
        ))
    })?;
    let actual = onecad_library::blob::sha256_hex(&bytes);
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
        // `check_free_params` already proved the key exists and is `role = free`.
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
    root: &Path,
    package: &ComponentPackage,
    current: &PlaceComponentParams,
    recorded_document_sha256: &str,
    merged: &BTreeMap<String, ComponentParamValue>,
) -> Result<StagedBlob, ApiError> {
    let overrides = variable_overrides(package, &current.component_id, merged)?;
    let frozen = frozen_source_document(root, package, current, recorded_document_sha256)?;

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
    let staged = match std::fs::write(&document_path, &frozen) {
        Ok(()) => {
            replay_and_bake(
                &document_path,
                &geometry_path,
                &overrides,
                &current.component_id,
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
    document_path: &Path,
    geometry_path: &Path,
    overrides: &[(String, f64)],
    component_id: &str,
) -> Result<StagedBlob, ApiError> {
    let bin = resolve_worker_path().ok_or_else(|| {
        ApiError::Internal(
            "setComponentParams: no geometry worker binary resolved — a re-bake needs one".into(),
        )
    })?;
    let worker = EphemeralWorker(WorkerManager::spawn(SupervisorConfig::production(bin)));
    if !worker.0.wait_ready(REBAKE_WORKER_READY_TIMEOUT).await {
        return Err(ApiError::Worker(format!(
            "setComponentParams: the re-bake worker for {component_id} never became ready"
        )));
    }

    let engine: Arc<dyn GeometryEngine> = Arc::new(worker.0.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(worker.0.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(worker.0.clone());
    let mut rt = DocumentRuntime::open(document_path, engine, meshes, solver).map_err(|e| {
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
    )
    .await?;
    if baked.solid_count != 1 {
        return Err(ApiError::InvalidCommand(format!(
            "setComponentParams: re-baking {component_id} produced {} solids; a component is \
             exactly one solid (spec §9) — the value cannot be applied",
            baked.solid_count
        )));
    }

    let bytes = std::fs::read(geometry_path).map_err(|e| {
        ApiError::Io(format!(
            "setComponentParams: read re-baked geometry for {component_id}: {e}"
        ))
    })?;
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
        bytes,
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

    let package = component_package_at(
        "setComponentParams",
        root,
        &current.component_id,
        &current.component_version,
    )?;
    check_free_params(
        "setComponentParams",
        &package,
        &current.component_id,
        &params,
    )?;

    let mut merged = current.params.clone();
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
            let staged = rebake_document_component(
                root,
                &package,
                &current,
                document_sha256.as_str(),
                &merged,
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
        ..current
    };
    let op = Operation::Known(KnownOperation::PlaceComponent(updated));

    // ONE undoable edit at the SAME `RecordId`, with the re-baked blob staged
    // under the same runtime lock — the `add_import_record` discipline: no regen
    // may see a record naming a blob that is not yet in the carrier.
    stage_blob_and_apply(
        state,
        blob,
        EditCommand::UpdateOperationParams { record, op },
    )
    .await
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
#[tauri::command]
pub async fn component_preview_mesh(
    app: AppHandle,
    state: State<'_, AppState>,
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
    component_preview_mesh_at(&root, preview, component_id, component_version, params).await
}

/// The testable core of [`component_preview_mesh`] — takes the preview engine
/// explicitly, so an integration test can drive it over a real worker (the
/// `#[tauri::command]` wrapper is pinned to the `Wry` runtime, and `AppState`'s
/// worker slots are only populated by the production factory).
pub async fn component_preview_mesh_at(
    root: &Path,
    preview: std::sync::Arc<dyn crate::worker::PreviewEngine>,
    component_id: String,
    component_version: String,
    params: Option<BTreeMap<String, ComponentParamValue>>,
) -> Result<crate::dto::PreviewResultDto, ApiError> {
    let library = open_at(root)?;
    let (_version, entry) = library
        .get(&component_id, Some(&component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "componentPreviewMesh: unknown component {component_id}@{component_version}"
            ))
        })?;
    let revision = entry.revision.clone();
    let declared = declared_numeric_params(&entry.parameters);
    let resolved = library.resolve_source(&ResolveRequest {
        component_id: &component_id,
        component_version: &component_version,
        component_revision: &revision,
    })?;
    let (source, blob) = component_source_from_resolved(resolved, &component_id, &declared)?;
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
#[tauri::command]
pub async fn list_templates(app: AppHandle) -> Result<Vec<ProjectTemplateDto>, ApiError> {
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
#[tauri::command]
pub async fn save_as_template(
    app: AppHandle,
    state: State<'_, AppState>,
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
#[tauri::command]
pub async fn new_from_template(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::dto::DocumentSnapshotDto, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("newFromTemplate: no app data dir".into()))?;
    let entry = onecad_library::template::get(&root, &id).ok_or_else(|| {
        ApiError::InvalidCommand(format!("newFromTemplate: unknown template {id}"))
    })?;

    let mut rt =
        crate::api::open_runtime_over_new_backend_blocking(&state, entry.document_path.clone())
            .await?;
    rt.detach_from_file(&entry.name);

    let (snapshot, projection) = {
        let mut guard = state.runtime.lock().await;
        *guard = Some(rt);
        let rt = guard.as_ref().unwrap();
        rt.adopt_current_epoch();
        (crate::api::snapshot_of(rt), rt.projection())
    };
    state.commit_backend();
    let _ = app.emit(crate::events::PROJECTION_UPDATED, &projection);
    // NOT recorded in recents: the user has not saved anything yet, and a
    // recents entry pointing INTO the library would reopen the template itself.
    crate::api::schedule_initial_regen(&state).await;
    Ok(snapshot)
}

/// The identity + metadata half of "Save as Component" (spec §7), as the
/// authoring UI supplies it. Everything about GEOMETRY is decided by the
/// backend: which body was baked, in which codec, and where the blob landed.
#[derive(Debug, Clone, serde::Deserialize)]
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
    /// The variable's value at authoring time — the package's declared default.
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
#[tauri::command]
pub async fn save_as_component(
    app: AppHandle,
    state: State<'_, AppState>,
    body_id: String,
    spec: NewComponentSpec,
    preview_png: Option<String>,
    union_solids: Option<bool>,
) -> Result<LibraryComponentDto, ApiError> {
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
/// Takes the runtime and the exporter EXPLICITLY rather than an `AppState`, so
/// an integration test can drive it over a real worker (`AppState`'s own
/// accessors are behind a `tauri::State`, and the `#[tauri::command]` wrapper
/// is pinned to the `Wry` runtime — the same constraint the `*_at` split in
/// this module already exists for).
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

    // A scratch directory per save, alongside the import workspaces (same
    // process-scoped temp root, same "this is worker hand-off material, not
    // user data" status).
    let scratch = std::env::temp_dir().join(format!(
        "onecad-authoring-{}-{}",
        std::process::id(),
        body_id.replace(['/', '\\'], "_")
    ));
    std::fs::create_dir_all(&scratch)
        .map_err(|e| ApiError::Internal(format!("saveAsComponent: scratch dir: {e}")))?;
    let document_path = scratch.join("source.onecad");
    let geometry_path = scratch.join("geometry.xbf");

    // Freeze the document under the runtime lock, write it outside — the same
    // split `save_document` uses, for the same reason (serialization must not
    // block an edit).
    let (payload, variable_names) = {
        let mut guard = runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("saveAsComponent".into()))?;
        if !rt.head_body_ids().contains(&body) {
            return Err(ApiError::InvalidCommand(format!(
                "saveAsComponent: {body_id} is not a body at head"
            )));
        }
        let variable_names: Vec<String> = rt.variables().iter().map(|v| v.name.clone()).collect();
        (
            rt.build_save_payload(
                crate::api::save_meta(),
                crate::document_runtime::SaveCaches::none(),
            ),
            variable_names,
        )
    };

    // A declared parameter that names no variable of the document being frozen
    // could never be honored by the re-bake — refused HERE, where the author can
    // still fix it, rather than at the first instance edit on someone else's
    // machine (the same discipline as the single-solid rule below).
    let mut parameters = BTreeMap::new();
    for (name, p) in &spec.parameters {
        if !variable_names.iter().any(|v| v == &p.key) {
            return Err(ApiError::InvalidCommand(format!(
                "saveAsComponent: parameter `{name}` binds to variable `{}`, which this document \
                 does not declare",
                p.key
            )));
        }
        if !p.value.is_finite() {
            return Err(ApiError::InvalidCommand(format!(
                "saveAsComponent: parameter `{name}` has a non-finite value"
            )));
        }
        parameters.insert(
            name.clone(),
            package::ParameterSpec::free_variable(&p.key, p.value),
        );
    }
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
        )
        .await?;
    if baked.solid_count != 1 {
        let _ = std::fs::remove_dir_all(&scratch);
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
    let _ = std::fs::remove_dir_all(&scratch);

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
    Ok(LibraryComponentDto {
        id: spec.id.clone(),
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
    })
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
    state: &State<'_, AppState>,
    record_id: String,
    component_id: String,
    component_version: String,
    free_params: BTreeMap<String, ComponentParamValue>,
) -> Result<(CommandOutcome, DocumentProjection, Option<String>), ApiError> {
    let record = RecordId::from_str(&record_id)
        .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {record_id:?}: {e}")))?;

    let current = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("replaceComponent".into()))?;
        let raw = rt.operation_params(record).ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "replaceComponent: no params for record {record_id}"
            ))
        })?;
        serde_json::from_value::<PlaceComponentParams>(raw).map_err(|e| {
            ApiError::InvalidCommand(format!(
                "replaceComponent: record {record_id} is not a placed component: {e}"
            ))
        })?
    };

    let library = open_at(root)?;
    let (_version, entry) = library
        .get(&component_id, Some(&component_version))
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "replaceComponent: unknown component {component_id}@{component_version}"
            ))
        })?;
    let revision = entry.revision.clone();
    let declared = declared_numeric_params(&entry.parameters);
    let resolved = library.resolve_source(&ResolveRequest {
        component_id: &component_id,
        component_version: &component_version,
        component_revision: &revision,
    })?;

    let package =
        component_package_at("replaceComponent", root, &component_id, &component_version)?;
    // Params are checked against the NEW signature, not the old one: a key the
    // old component called free may not exist here at all.
    check_free_params("replaceComponent", &package, &component_id, &free_params)?;

    let (source, blob) = component_source_from_resolved(resolved, &component_id, &declared)?;
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

    let (outcome, projection) = stage_blob_and_apply(
        state,
        blob,
        EditCommand::UpdateOperationParams { record, op },
    )
    .await?;
    Ok((outcome, projection, dropped_attachment))
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
#[tauri::command]
pub async fn replace_component(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: String,
    component_id: String,
    component_version: String,
    params: Option<BTreeMap<String, ComponentParamValue>>,
) -> Result<ReplaceComponentResultDto, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("replaceComponent: no app data dir".into()))?;
    let (outcome, projection, dropped_mate_attachment) = replace_component_at(
        &root,
        &state,
        record_id,
        component_id,
        component_version,
        params.unwrap_or_default(),
    )
    .await?;
    finish(&app, &state, &outcome, &projection);
    Ok(ReplaceComponentResultDto {
        projection,
        report: ReplaceComponentReportDto {
            dropped_mate_attachment,
        },
    })
}

/// Reports whether a newer version of the component behind `record_id` is
/// indexed. Read-only; see [`component_upgrade_available_at`].
#[tauri::command]
pub async fn component_upgrade_available(
    app: AppHandle,
    state: State<'_, AppState>,
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
            Default::default(),
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
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let body_uuid = uuid::Uuid::from_u128(0xb0d1);
        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
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

        let guard = app.state::<AppState>();
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
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

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
                &state,
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

        let guard = app.state::<AppState>();
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
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let err = place_component_at(
            dir.path(),
            &state,
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

        let guard = app.state::<AppState>();
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
    }

    /// Places `component_id`, then hand-authors a mate onto the resulting
    /// record. (`place_component_at` CAN author one since WP-H2, but its input
    /// shape wants pick evidence; writing the `ComponentMate` directly keeps
    /// these carry-rule tests independent of the promotion path.)
    async fn place_then_mate(
        root: &Path,
        state: &State<'_, AppState>,
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
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
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
            &state,
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
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        // Carried: the new package declares `shank_axis` too.
        let record_id = place_then_mate(dir.path(), &state, "acme.with.shank", "shank_axis").await;
        let (_o, _p, dropped) = replace_component_at(
            dir.path(),
            &state,
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
            &state,
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
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let mut free = BTreeMap::new();
        free.insert(
            "thread".to_string(),
            ComponentParamValue::Text("M8".to_string()),
        );
        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
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
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let mut free = BTreeMap::new();
        // `head_d` is `role = "table"` in the fixture — derived, never set.
        free.insert("head_d".to_string(), ComponentParamValue::Number(9.0));
        let err = place_component_at(
            dir.path(),
            &state,
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
            Default::default(),
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
            Default::default(),
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
        let err = set_component_params_at(dir.path(), &state, record_id, params)
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
        sha
    }

    async fn profile_state(dir: &Path) -> (tauri::App<tauri::test::MockRuntime>, String) {
        let sha = write_profile_package(dir, "acme.extrusion.2020", b"pretend canonical face");
        reindex_library_at(dir).unwrap();
        let app_state = pending_app_state();
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let app = tauri::test::mock_app();
        app.manage(app_state);
        (app, sha)
    }

    #[tokio::test]
    async fn a_profile_placement_with_no_free_params_lands_the_packages_declared_length() {
        let dir = tempfile::tempdir().unwrap();
        let (app, face_sha) = profile_state(dir.path()).await;
        let state: tauri::State<'_, AppState> = app.state();

        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
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
        let (app, face_sha) = profile_state(dir.path()).await;
        let state: tauri::State<'_, AppState> = app.state();

        let mut at_120 = BTreeMap::new();
        at_120.insert("length".to_string(), ComponentParamValue::Number(120.0));
        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
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
        set_component_params_at(dir.path(), &state, record_id, at_300)
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
        let (app, _sha) = profile_state(dir.path()).await;
        let state: tauri::State<'_, AppState> = app.state();

        let (_outcome, projection) = place_component_at(
            dir.path(),
            &state,
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
            let err = set_component_params_at(dir.path(), &state, record_id.clone(), params)
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
            Default::default(),
            None,
        )
        .await
        .expect("place");
        let record_id = projection.features[0].id.clone();

        let mut params = BTreeMap::new();
        params.insert("depth".to_string(), ComponentParamValue::Number(30.0));
        let err = set_component_params_at(dir.path(), &state, record_id, params)
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
        let app = tauri::test::mock_app();
        app.manage(app_state);
        let state: tauri::State<'_, AppState> = app.state();

        let err = place_component_at(
            dir.path(),
            &state,
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

        let guard = app.state::<AppState>();
        let rt_guard = guard.runtime.lock().await;
        let rt = rt_guard.as_ref().unwrap();
        assert!(
            rt.projection().features.is_empty(),
            "fail closed: nothing was authored"
        );
    }
}
