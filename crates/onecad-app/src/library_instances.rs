//! Component-instance management over the open document: list placed
//! instances with their effective/default/override parameters, plan and apply
//! version upgrades for many linked instances as ONE undoable edit, and reset
//! overrides.
//!
//! Nothing here swaps an instance by itself (spec §0 invariant 4): an upgrade
//! is planned read-only, shown to the user, and applied only if the live plan
//! still equals the approved one.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::str::FromStr;

use onecad_core::document::record::{
    ComponentParamValue, ComponentSourceRef, KnownOperation, Operation, OperationRecord,
    PlaceComponentParams,
};
use onecad_core::edit::{CommandOutcome, EditCommand};
use onecad_core::ids::RecordId;
use onecad_library::package::{ComponentPackage, SourceSpec};
use onecad_library::Library;

use crate::dto::{
    ComponentInstanceDto, ComponentStatusDto, DocumentProjection, DroppedOverrideDto,
    UpgradeItemDto,
};
use crate::error::ApiError;
use crate::host::HostContext;
use crate::library::{
    component_package_at, component_status_with, finish, free_defaults, library_root, open_at,
    prepare_replace_component_at, set_component_params_removing_at, validate_component_params,
};
use crate::state::AppState;

type Params = BTreeMap<String, ComponentParamValue>;

// ── Listing ─────────────────────────────────────────────────────────────────

fn source_kind(source: &ComponentSourceRef) -> &'static str {
    match source {
        ComponentSourceRef::Generator { .. } => "generator",
        ComponentSourceRef::Embedded { .. } => "embedded",
        ComponentSourceRef::Document { .. } => "document",
        ComponentSourceRef::Profile { .. } => "profile",
    }
}

fn source_params(source: &ComponentSourceRef) -> Params {
    match source {
        ComponentSourceRef::Generator { params, .. }
        | ComponentSourceRef::Document { params, .. }
        | ComponentSourceRef::Profile { params, .. } => params.clone(),
        ComponentSourceRef::Embedded { .. } => Params::new(),
    }
}

fn render_value(value: &ComponentParamValue) -> String {
    match value {
        ComponentParamValue::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => {
            format!("{}", *n as i64)
        }
        ComponentParamValue::Number(n) => n.to_string(),
        ComponentParamValue::Text(t) => t.clone(),
        ComponentParamValue::Bool(b) => b.to_string(),
    }
}

/// Fills `{key}` placeholders from `effective`; unknown placeholders stay as written.
fn resolve_designation(template: &str, effective: &Params) -> String {
    let mut out = template.to_string();
    for (key, value) in effective {
        out = out.replace(&format!("{{{key}}}"), &render_value(value));
    }
    out
}

fn linked_instance(
    library: Option<&Library>,
    record: &OperationRecord,
    placed: &PlaceComponentParams,
) -> ComponentInstanceDto {
    let entry = library
        .and_then(|l| l.get(&placed.component_id, Some(&placed.component_version)))
        .map(|(_, entry)| entry);
    let defaults = entry.map_or_else(Params::new, |e| free_defaults(&e.parameters));
    let mut effective = defaults.clone();
    effective.extend(placed.params.clone());
    let overrides = placed
        .params
        .iter()
        .filter(|(key, value)| defaults.get(*key) != Some(*value))
        .map(|(key, _)| key.clone())
        .collect();
    let designation = entry
        .and_then(|e| e.designation.as_deref())
        .map(|template| resolve_designation(template, &effective));
    let status = match library {
        Some(library) => component_status_with(library, placed),
        None => ComponentStatusDto::DefinitionMissing,
    };
    ComponentInstanceDto {
        record_id: record.record_id.to_string(),
        name: record.name.clone(),
        component_id: placed.component_id.clone(),
        component_version: placed.component_version.clone(),
        source_kind: source_kind(&placed.source).into(),
        linked: true,
        designation,
        effective,
        defaults,
        overrides,
        status,
    }
}

fn detached_instance(
    record: &OperationRecord,
    source: &ComponentSourceRef,
) -> ComponentInstanceDto {
    ComponentInstanceDto {
        record_id: record.record_id.to_string(),
        name: record.name.clone(),
        // A detached copy keeps geometry provenance only, not library identity.
        component_id: String::new(),
        component_version: String::new(),
        source_kind: source_kind(source).into(),
        linked: false,
        designation: None,
        effective: source_params(source),
        defaults: Params::new(),
        overrides: Vec::new(),
        status: ComponentStatusDto::Detached,
    }
}

/// Every `PlaceComponent`/`DetachComponent` record, in history order.
async fn component_records(state: &AppState, what: &str) -> Result<Vec<OperationRecord>, ApiError> {
    let guard = state.runtime.lock().await;
    let rt = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument(what.into()))?;
    Ok(rt
        .projection()
        .features
        .iter()
        .filter(|f| matches!(f.op_type.as_str(), "PlaceComponent" | "DetachComponent"))
        .filter_map(|f| rt.operation_record(RecordId::from_str(&f.id).ok()?))
        .collect())
}

/// Lists placed and detached component instances with resolved parameters.
/// Read-only; the library is opened once. `None` root reads every linked
/// instance as `DefinitionMissing`.
pub async fn component_instances_at(
    root: Option<&Path>,
    state: &AppState,
) -> Result<Vec<ComponentInstanceDto>, ApiError> {
    let records = component_records(state, "componentInstances").await?;
    let library = root.and_then(|r| Library::open(r).ok());
    Ok(records
        .iter()
        .filter_map(|record| match &record.op {
            Operation::Known(KnownOperation::PlaceComponent(placed)) => {
                Some(linked_instance(library.as_ref(), record, placed))
            }
            Operation::Known(KnownOperation::DetachComponent(detached)) => {
                Some(detached_instance(record, &detached.source))
            }
            _ => None,
        })
        .collect())
}

pub async fn component_instances(
    app: HostContext,
    state: &AppState,
) -> Result<Vec<ComponentInstanceDto>, ApiError> {
    component_instances_at(library_root(&app).as_deref(), state).await
}

// ── Upgrade planning ────────────────────────────────────────────────────────

/// An upgrade item plus what apply needs and must re-verify: the component to
/// swap to and the exact record state the plan was computed from.
struct Planned {
    item: UpgradeItemDto,
    component_id: String,
    snapshot: Option<PlaceComponentParams>,
}

fn refused(record_id: &str, from: &str, to: &str, reason: String) -> UpgradeItemDto {
    UpgradeItemDto {
        record_id: record_id.to_string(),
        from_version: from.to_string(),
        to_version: to.to_string(),
        to_revision: String::new(),
        kept_overrides: Params::new(),
        dropped_overrides: Vec::new(),
        dropped_mate_attachment: None,
        applicable: false,
        refusal: Some(reason),
    }
}

/// Splits the instance's OVERRIDES into those the target package accepts and
/// those it cannot carry.
///
/// An override is a record param whose value differs from the package default
/// the instance was placed against (`old`). A param equal to that default is
/// not an override: it follows the target's default after the upgrade and is
/// reported as neither kept nor dropped. The verdict depends on the TARGET's
/// source kind: a baked (`embedded`/`document`) target has no free parameters,
/// so every override is dropped with the reason apply would otherwise refuse.
fn reconcile_overrides(
    placed: &PlaceComponentParams,
    old: &ComponentPackage,
    target: &ComponentPackage,
) -> (Params, Vec<DroppedOverrideDto>) {
    let baked_kind = match target.source {
        SourceSpec::Embedded { .. } => Some("embedded"),
        SourceSpec::Document { .. } => Some("document"),
        SourceSpec::Generator { .. } | SourceSpec::Profile { .. } => None,
    };
    let old_defaults = free_defaults(&old.parameters);
    let mut kept = Params::new();
    let mut dropped = Vec::new();
    for (key, value) in &placed.params {
        if old_defaults.get(key) == Some(value) {
            continue;
        }
        let verdict = match baked_kind {
            Some(kind) => Err(format!(
                "{kind} source has no free parameters to carry across versions"
            )),
            None => {
                let one = Params::from([(key.clone(), value.clone())]);
                validate_component_params("upgrade", target, &placed.component_id, &one)
                    .map_err(|e| e.to_string())
            }
        };
        match verdict {
            Ok(()) => {
                kept.insert(key.clone(), value.clone());
            }
            Err(reason) => dropped.push(DroppedOverrideDto {
                key: key.clone(),
                value: value.clone(),
                reason,
            }),
        }
    }
    (kept, dropped)
}

/// The version an upgrade moves to, or why it cannot. Explicit targets must be
/// indexed and STRICTLY newer: an upgrade never moves an instance backwards.
fn resolve_target(
    library: &Library,
    placed: &PlaceComponentParams,
    requested: Option<&str>,
) -> Result<String, (String, String)> {
    let from = &placed.component_version;
    let fail = |to: &str, why: String| Err((to.to_string(), why));
    match component_status_with(library, placed) {
        ComponentStatusDto::DefinitionMissing => {
            return fail(
                from,
                "current definition is missing from the library".into(),
            )
        }
        ComponentStatusDto::DefinitionUnreadable { reason } => {
            return fail(from, format!("current definition is unreadable: {reason}"))
        }
        _ => {}
    }
    let versions = library.index().versions(&placed.component_id);
    let to = match requested {
        Some(v) => v.to_string(),
        None => library
            .index()
            .newer_than(&placed.component_id, from)
            .map_or_else(|| from.clone(), |(v, _)| v.to_string()),
    };
    let position = |v: &str| versions.iter().position(|x| *x == v);
    match (position(from), position(&to)) {
        (_, None) => fail(&to, format!("version {to} is not in the library")),
        (Some(a), Some(b)) if b > a => Ok(to),
        _ => fail(&to, format!("{to} is not newer than {from}")),
    }
}

fn plan_placed(
    root: &Path,
    library: &Library,
    record_id: &str,
    placed: PlaceComponentParams,
    requested: Option<&str>,
) -> Planned {
    let from = placed.component_version.clone();
    let component_id = placed.component_id.clone();
    let plan = |item, snapshot| Planned {
        item,
        component_id: component_id.clone(),
        snapshot,
    };
    let to = match resolve_target(library, &placed, requested) {
        Ok(to) => to,
        Err((to, why)) => return plan(refused(record_id, &from, &to, why), None),
    };
    let package = match component_package_at("upgrade", root, &component_id, &to) {
        Ok(p) => p,
        Err(e) => return plan(refused(record_id, &from, &to, e.to_string()), None),
    };
    let old = match component_package_at("upgrade", root, &component_id, &from) {
        Ok(p) => p,
        Err(e) => return plan(refused(record_id, &from, &to, e.to_string()), None),
    };
    let (kept_overrides, dropped_overrides) = reconcile_overrides(&placed, &old, &package);
    let dropped_mate_attachment = placed
        .mate
        .as_ref()
        .map(|m| m.self_attachment.clone())
        .filter(|name| !package.attachments.contains_key(name));
    let item = UpgradeItemDto {
        record_id: record_id.to_string(),
        from_version: from,
        to_version: to,
        to_revision: package.identity.revision.clone(),
        kept_overrides,
        dropped_overrides,
        dropped_mate_attachment,
        applicable: true,
        refusal: None,
    };
    plan(item, Some(placed))
}

/// Reads each requested record once, under one lock: `None` marks a record that
/// is not a linked `PlaceComponent`.
async fn read_placed(
    state: &AppState,
    ids: &[String],
) -> Result<Vec<Option<PlaceComponentParams>>, ApiError> {
    let guard = state.runtime.lock().await;
    let rt = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("componentUpgrades".into()))?;
    ids.iter()
        .map(|id| {
            let record = RecordId::from_str(id)
                .map_err(|e| ApiError::InvalidCommand(format!("bad recordId {id:?}: {e}")))?;
            Ok(rt.operation_record(record).and_then(|r| match r.op {
                Operation::Known(KnownOperation::PlaceComponent(p)) => Some(p),
                _ => None,
            }))
        })
        .collect()
}

async fn plan_targets(
    root: &Path,
    state: &AppState,
    targets: Vec<(String, Option<String>)>,
) -> Result<Vec<Planned>, ApiError> {
    let mut seen = HashSet::new();
    let targets: Vec<_> = targets
        .into_iter()
        .filter(|(id, _)| seen.insert(id.clone()))
        .collect();
    let ids: Vec<String> = targets.iter().map(|(id, _)| id.clone()).collect();
    let placed = read_placed(state, &ids).await?;
    let library = open_at(root)?;
    Ok(targets
        .into_iter()
        .zip(placed)
        .map(|((id, requested), placed)| match placed {
            Some(p) => plan_placed(root, &library, &id, p, requested.as_deref()),
            None => Planned {
                item: refused(
                    &id,
                    "",
                    requested.as_deref().unwrap_or(""),
                    "not a linked component instance".into(),
                ),
                component_id: String::new(),
                snapshot: None,
            },
        })
        .collect())
}

/// Read-only upgrade plan for `record_ids`. `target_version` `None` means the
/// newest indexed version of each instance's component.
pub async fn plan_component_upgrades_at(
    root: &Path,
    state: &AppState,
    record_ids: Vec<String>,
    target_version: Option<String>,
) -> Result<Vec<UpgradeItemDto>, ApiError> {
    let targets = record_ids
        .into_iter()
        .map(|id| (id, target_version.clone()))
        .collect();
    let planned = plan_targets(root, state, targets).await?;
    Ok(planned.into_iter().map(|p| p.item).collect())
}

pub async fn plan_component_upgrades(
    app: HostContext,
    state: &AppState,
    record_ids: Vec<String>,
    target_version: Option<String>,
) -> Result<Vec<UpgradeItemDto>, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("planComponentUpgrades: no app data dir".into()))?;
    plan_component_upgrades_at(&root, state, record_ids, target_version).await
}

// ── Upgrade apply ───────────────────────────────────────────────────────────

fn stale() -> ApiError {
    ApiError::InvalidCommand("upgrade plan is stale".into())
}

/// Applies `approved` as ONE transaction (one undo step, all-or-nothing).
///
/// Fails closed: the approved items are re-planned against the live document
/// and library, and ANY difference (versions, kept/dropped sets, applicability)
/// refuses the whole batch. Non-applicable approved items are verified and
/// skipped. Each record is re-read under the final lock, so a record edited
/// since the plan also refuses.
pub async fn apply_component_upgrades_at(
    root: &Path,
    state: &AppState,
    approved: Vec<UpgradeItemDto>,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let targets = approved
        .iter()
        .map(|i| (i.record_id.clone(), Some(i.to_version.clone())))
        .collect();
    let planned = plan_targets(root, state, targets).await?;
    let replanned: Vec<&UpgradeItemDto> = planned.iter().map(|p| &p.item).collect();
    if replanned.len() != approved.len() || replanned.iter().zip(&approved).any(|(a, b)| *a != b) {
        return Err(stale());
    }
    let planned: Vec<Planned> = planned.into_iter().filter(|p| p.item.applicable).collect();
    if planned.is_empty() {
        return Err(ApiError::InvalidCommand(
            "no applicable component upgrades".into(),
        ));
    }
    let mut commands = Vec::with_capacity(planned.len());
    let mut blobs = Vec::new();
    for p in &planned {
        let prepared = prepare_replace_component_at(
            root,
            state,
            p.item.record_id.clone(),
            p.component_id.clone(),
            p.item.to_version.clone(),
            p.item.kept_overrides.clone(),
        )
        .await?;
        commands.push(prepared.command);
        blobs.extend(prepared.blob);
    }
    commit_upgrades(root, state, &planned, blobs, commands).await
}

/// Stages every blob and applies the transaction under one runtime lock, after
/// re-checking each record still equals the state the plan was built from.
async fn commit_upgrades(
    root: &Path,
    state: &AppState,
    planned: &[Planned],
    blobs: Vec<crate::library::StagedBlob>,
    commands: Vec<EditCommand>,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let mut guard = state.runtime.lock().await;
    let rt = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("applyComponentUpgrades".into()))?;
    let library = open_at(root)?;
    for (p, command) in planned.iter().zip(&commands) {
        // The library moves independently of the document: the target must still
        // declare the revision the user approved, and the record we authored
        // must name that same revision.
        let declared = library
            .declared_revision(&p.component_id, &p.item.to_version)
            .map_err(|_| stale())?;
        let authored = match command {
            EditCommand::UpdateOperationParams {
                op: Operation::Known(KnownOperation::PlaceComponent(c)),
                ..
            } => Some(c.component_revision.as_str()),
            _ => None,
        };
        if declared != p.item.to_revision || authored != Some(p.item.to_revision.as_str()) {
            return Err(stale());
        }
        let record = RecordId::from_str(&p.item.record_id)
            .map_err(|e| ApiError::InvalidCommand(e.to_string()))?;
        let live = rt.operation_record(record).and_then(|r| match r.op {
            Operation::Known(KnownOperation::PlaceComponent(live)) => Some(live),
            _ => None,
        });
        if live != p.snapshot {
            return Err(stale());
        }
    }
    for blob in &blobs {
        blob.stage_into(rt)?;
    }
    let outcome = rt.apply_transaction("Update components", commands)?;
    Ok((outcome, rt.projection()))
}

pub async fn apply_component_upgrades(
    app: HostContext,
    state: &AppState,
    approved: Vec<UpgradeItemDto>,
) -> Result<DocumentProjection, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("applyComponentUpgrades: no app data dir".into()))?;
    let (outcome, projection) = apply_component_upgrades_at(&root, state, approved).await?;
    finish(&app, state, &outcome, &projection);
    Ok(projection)
}

// ── Reset overrides ─────────────────────────────────────────────────────────

/// Removes override keys (`None` = all) so the instance falls back to the
/// package defaults. One undo step, same validation and re-bake as
/// `set_component_params_at`.
pub async fn reset_component_overrides_at(
    root: &Path,
    state: &AppState,
    record_id: String,
    keys: Option<Vec<String>>,
) -> Result<(CommandOutcome, DocumentProjection), ApiError> {
    let current = read_placed(state, std::slice::from_ref(&record_id))
        .await?
        .pop()
        .flatten()
        .ok_or_else(|| {
            ApiError::InvalidCommand(format!(
                "resetComponentOverrides: record {record_id} is not a placed component"
            ))
        })?;
    let removed: Vec<String> = match keys {
        Some(keys) => keys
            .into_iter()
            .filter(|k| current.params.contains_key(k))
            .collect(),
        None => current.params.keys().cloned().collect(),
    };
    if removed.is_empty() {
        return Err(ApiError::InvalidCommand(
            "resetComponentOverrides: no overrides to reset".into(),
        ));
    }
    set_component_params_removing_at(root, state, record_id, Params::new(), &removed).await
}

pub async fn reset_component_overrides(
    app: HostContext,
    state: &AppState,
    record_id: String,
    keys: Option<Vec<String>>,
) -> Result<DocumentProjection, ApiError> {
    let root = library_root(&app)
        .ok_or_else(|| ApiError::Internal("resetComponentOverrides: no app data dir".into()))?;
    let (outcome, projection) = reset_component_overrides_at(&root, state, record_id, keys).await?;
    finish(&app, state, &outcome, &projection);
    Ok(projection)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::document_runtime::DocumentRuntime;
    use crate::library::{
        copy_placement_commands, place_component_at, reindex_library_at, set_component_params_at,
    };
    use crate::state::BackendBundle;
    use crate::worker::PendingBackend;

    use super::*;

    const ID: &str = "me.test.screw";

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

    /// v1.0.0 declares `thread` (M3..M8), `length` and `thread_detail`; v1.1.0
    /// narrows `thread` to M3..M6 and drops `thread_detail` as a free parameter.
    fn write_version(root: &Path, version: &str) {
        let v11 = version == "1.1.0";
        let (domain, detail) = if v11 {
            (r#"["M3", "M4", "M5", "M6"]"#, "")
        } else {
            (
                r#"["M3", "M4", "M5", "M6", "M8"]"#,
                "thread_detail = { role = \"free\", value = \"cosmetic\", domain = [\"cosmetic\", \"modeled\"] }",
            )
        };
        let dir = root.join(format!("{ID}@{version}"));
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"
[identity]
id = "{ID}"
version = "{version}"
revision = "sha256:{}"

[metadata]
name = "Test screw"
designation = "T {{thread}}x{{length}}"

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1

[parameters]
thread = {{ role = "free", value = "M6", domain = {domain} }}
length = {{ role = "free", value = 20, min = 2 }}
{detail}
"#,
            if v11 { "1" } else { "0" }.repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
    }

    fn num(v: f64) -> ComponentParamValue {
        ComponentParamValue::Number(v)
    }

    fn text(v: &str) -> ComponentParamValue {
        ComponentParamValue::Text(v.into())
    }

    async fn place(root: &Path, state: &AppState, params: Params) -> String {
        let (_o, projection) = place_component_at(
            root,
            state,
            ID.into(),
            "1.0.0".into(),
            [0.0; 3],
            None,
            params,
            None,
        )
        .await
        .expect("place");
        projection.features.last().unwrap().id.clone()
    }

    /// Library with v1.0.0 only; document with A (length 30, thread M8),
    /// B (thread_detail modeled) and a Copy. v1.1.0 is added afterwards.
    async fn fixture() -> (tempfile::TempDir, AppState, [String; 3]) {
        let dir = tempfile::tempdir().unwrap();
        write_version(dir.path(), "1.0.0");
        reindex_library_at(dir.path()).unwrap();
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        let a = place(
            dir.path(),
            &state,
            Params::from([("length".into(), num(30.0)), ("thread".into(), text("M8"))]),
        )
        .await;
        let b = place(
            dir.path(),
            &state,
            Params::from([("thread_detail".into(), text("modeled"))]),
        )
        .await;
        let copy = place(dir.path(), &state, Params::new()).await;
        {
            let mut guard = state.runtime.lock().await;
            let rt = guard.as_mut().unwrap();
            let record = rt
                .operation_record(RecordId::from_str(&copy).unwrap())
                .unwrap();
            let cmds = copy_placement_commands(record, None).unwrap();
            // Replace the third placement with its detached twin, same record.
            let EditCommand::AddOperation { record: twin, .. } = &cmds[0] else {
                panic!("one AddOperation");
            };
            let Operation::Known(op) = twin.op.clone() else {
                panic!("known op");
            };
            rt.apply(EditCommand::UpdateOperationParams {
                record: RecordId::from_str(&copy).unwrap(),
                op: Operation::Known(op),
            })
            .expect("detach in place");
        }
        write_version(dir.path(), "1.1.0");
        reindex_library_at(dir.path()).unwrap();
        (dir, state, [a, b, copy])
    }

    async fn placed(state: &AppState, id: &str) -> PlaceComponentParams {
        read_placed(state, &[id.to_string()])
            .await
            .unwrap()
            .pop()
            .flatten()
            .expect("placed")
    }

    #[tokio::test]
    async fn lists_instances_with_effective_defaults_overrides_and_status() {
        let (dir, state, [a, b, copy]) = fixture().await;
        let list = component_instances_at(Some(dir.path()), &state)
            .await
            .unwrap();
        assert_eq!(list.len(), 3);
        let (ia, ib, ic) = (&list[0], &list[1], &list[2]);
        assert_eq!(
            (&ia.record_id, &ib.record_id, &ic.record_id),
            (&a, &b, &copy)
        );
        assert!(ia.linked && ib.linked && !ic.linked);
        assert_eq!(ia.overrides, ["length", "thread"]);
        assert_eq!(ia.designation.as_deref(), Some("T M8x30"));
        assert_eq!(ia.defaults["length"], num(20.0));
        assert_eq!(ia.effective["length"], num(30.0));
        assert_eq!(ib.overrides, ["thread_detail"]);
        assert_eq!(ib.designation.as_deref(), Some("T M6x20"));
        assert!(matches!(
            ia.status,
            ComponentStatusDto::UpgradeAvailable { ref version } if version == "1.1.0"
        ));
        assert_eq!(ic.status, ComponentStatusDto::Detached);
        assert_eq!(ic.source_kind, "generator");
        assert!(ic.component_id.is_empty() && ic.defaults.is_empty());

        let none = component_instances_at(None, &state).await.unwrap();
        assert_eq!(none[0].status, ComponentStatusDto::DefinitionMissing);
        assert!(none[0].defaults.is_empty() && none[0].designation.is_none());
    }

    #[tokio::test]
    async fn plan_reports_kept_and_dropped_and_excludes_the_copy() {
        let (dir, state, [a, b, copy]) = fixture().await;
        let plan = plan_component_upgrades_at(
            dir.path(),
            &state,
            vec![a.clone(), b.clone(), copy.clone()],
            None,
        )
        .await
        .unwrap();
        assert_eq!(plan.len(), 3);
        assert!(plan[0].applicable && plan[1].applicable && !plan[2].applicable);
        assert_eq!(
            (plan[0].from_version.as_str(), plan[0].to_version.as_str()),
            ("1.0.0", "1.1.0")
        );
        assert_eq!(
            plan[0].kept_overrides,
            Params::from([("length".into(), num(30.0))])
        );
        assert_eq!(plan[0].dropped_overrides.len(), 1);
        assert_eq!(plan[0].dropped_overrides[0].key, "thread");
        assert!(plan[0].dropped_overrides[0].reason.contains("domain"));
        assert!(plan[1].kept_overrides.is_empty());
        assert_eq!(plan[1].dropped_overrides[0].key, "thread_detail");
        assert_eq!(
            plan[2].refusal.as_deref(),
            Some("not a linked component instance")
        );

        let same =
            plan_component_upgrades_at(dir.path(), &state, vec![a.clone()], Some("1.0.0".into()))
                .await
                .unwrap();
        assert!(!same[0].applicable);
        assert!(same[0].refusal.as_deref().unwrap().contains("not newer"));
        let missing = plan_component_upgrades_at(dir.path(), &state, vec![a], Some("9.9.9".into()))
            .await
            .unwrap();
        assert!(!missing[0].applicable);
    }

    #[tokio::test]
    async fn apply_is_one_undo_step_and_redo_reapplies() {
        let (dir, state, [a, b, copy]) = fixture().await;
        let before_a = placed(&state, &a).await;
        let before_b = placed(&state, &b).await;
        let plan = plan_component_upgrades_at(
            dir.path(),
            &state,
            vec![a.clone(), b.clone(), copy.clone()],
            None,
        )
        .await
        .unwrap();
        let depth = state.runtime.lock().await.as_ref().unwrap().undo_depth();

        apply_component_upgrades_at(dir.path(), &state, plan)
            .await
            .unwrap();
        let (ua, ub) = (placed(&state, &a).await, placed(&state, &b).await);
        assert_eq!(
            (ua.component_version.as_str(), ub.component_version.as_str()),
            ("1.1.0", "1.1.0")
        );
        assert_eq!(ua.params, Params::from([("length".into(), num(30.0))]));
        assert!(ub.params.is_empty());
        {
            let mut guard = state.runtime.lock().await;
            let rt = guard.as_mut().unwrap();
            assert_eq!(rt.undo_depth(), depth + 1, "one undo step");
            rt.undo().expect("undo");
        }
        assert_eq!(placed(&state, &a).await, before_a);
        assert_eq!(placed(&state, &b).await, before_b);
        {
            let mut guard = state.runtime.lock().await;
            guard.as_mut().unwrap().redo().expect("redo");
        }
        assert_eq!(placed(&state, &a).await, ua);
        assert_eq!(placed(&state, &b).await, ub);
    }

    #[tokio::test]
    async fn a_stale_plan_is_refused_and_nothing_changes() {
        let (dir, state, [a, b, _]) = fixture().await;
        let plan = plan_component_upgrades_at(dir.path(), &state, vec![a.clone(), b.clone()], None)
            .await
            .unwrap();
        set_component_params_at(
            dir.path(),
            &state,
            a.clone(),
            Params::from([("length".into(), num(40.0))]),
        )
        .await
        .unwrap();
        let (live_a, live_b) = (placed(&state, &a).await, placed(&state, &b).await);

        let err = apply_component_upgrades_at(dir.path(), &state, plan.clone())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("upgrade plan is stale"), "{err}");
        assert_eq!(placed(&state, &a).await, live_a);
        assert_eq!(placed(&state, &b).await, live_b);

        // A hand-edited approval is refused the same way.
        let fresh = plan_component_upgrades_at(dir.path(), &state, vec![a, b], None)
            .await
            .unwrap();
        let mut forged = fresh.clone();
        forged[1].dropped_overrides.clear();
        let err = apply_component_upgrades_at(dir.path(), &state, forged)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
        apply_component_upgrades_at(dir.path(), &state, fresh)
            .await
            .expect("fresh plan applies");
    }

    #[tokio::test]
    async fn reset_removes_overrides_in_one_undo_step() {
        let (dir, state, [a, _, copy]) = fixture().await;
        let depth = state.runtime.lock().await.as_ref().unwrap().undo_depth();
        reset_component_overrides_at(dir.path(), &state, a.clone(), Some(vec!["length".into()]))
            .await
            .unwrap();
        let after = placed(&state, &a).await;
        assert_eq!(after.params, Params::from([("thread".into(), text("M8"))]));
        reset_component_overrides_at(dir.path(), &state, a.clone(), None)
            .await
            .unwrap();
        let list = component_instances_at(Some(dir.path()), &state)
            .await
            .unwrap();
        assert!(list[0].overrides.is_empty());
        assert_eq!(list[0].effective, list[0].defaults);
        assert_eq!(
            state.runtime.lock().await.as_ref().unwrap().undo_depth(),
            depth + 2
        );
        let again = reset_component_overrides_at(dir.path(), &state, a, None).await;
        assert!(again.unwrap_err().to_string().contains("no overrides"));
        let on_copy = reset_component_overrides_at(dir.path(), &state, copy, None).await;
        assert!(on_copy.is_err());
    }

    /// v1.0.0 defaults `thread_detail` to `cosmetic`, v1.1.0 to `simplified`.
    fn write_detail_version(root: &Path, version: &str, default: &str) {
        let dir = root.join(format!("{ID}@{version}"));
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"
[identity]
id = "{ID}"
version = "{version}"
revision = "sha256:{}"

[metadata]
name = "Test screw"

[source]
kind = "generator"
generator = "iso4762"
generator_version = 1

[parameters]
thread_detail = {{ role = "free", value = "{default}", domain = ["cosmetic", "simplified", "modeled"] }}
"#,
            if version == "1.1.0" { "1" } else { "0" }.repeat(64)
        );
        std::fs::write(dir.join("component.toml"), toml).unwrap();
    }

    async fn blank_state() -> AppState {
        let state = pending_app_state();
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
        state
    }

    #[tokio::test]
    async fn a_param_equal_to_the_old_default_follows_the_new_default_on_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        write_detail_version(dir.path(), "1.0.0", "cosmetic");
        reindex_library_at(dir.path()).unwrap();
        let state = blank_state().await;
        let pinned = place(
            dir.path(),
            &state,
            Params::from([("thread_detail".into(), text("cosmetic"))]),
        )
        .await;
        let real = place(
            dir.path(),
            &state,
            Params::from([("thread_detail".into(), text("modeled"))]),
        )
        .await;
        write_detail_version(dir.path(), "1.1.0", "simplified");
        reindex_library_at(dir.path()).unwrap();

        let plan = plan_component_upgrades_at(
            dir.path(),
            &state,
            vec![pinned.clone(), real.clone()],
            None,
        )
        .await
        .unwrap();
        assert!(plan[0].applicable && plan[0].kept_overrides.is_empty());
        assert!(
            plan[0].dropped_overrides.is_empty(),
            "an old-default value is not an override: neither kept nor dropped"
        );
        assert_eq!(
            plan[1].kept_overrides,
            Params::from([("thread_detail".into(), text("modeled"))])
        );
        apply_component_upgrades_at(dir.path(), &state, plan)
            .await
            .unwrap();

        assert!(placed(&state, &pinned).await.params.is_empty());
        let list = component_instances_at(Some(dir.path()), &state)
            .await
            .unwrap();
        assert_eq!(list[0].effective["thread_detail"], text("simplified"));
        assert!(list[0].overrides.is_empty());
        assert_eq!(list[1].effective["thread_detail"], text("modeled"));
        assert_eq!(list[1].overrides, ["thread_detail"]);
    }

    #[tokio::test]
    async fn a_package_resaved_with_new_content_after_the_plan_is_refused() {
        let (dir, state, [a, _, _]) = fixture().await;
        let plan = plan_component_upgrades_at(dir.path(), &state, vec![a.clone()], None)
            .await
            .unwrap();
        assert!(!plan[0].to_revision.is_empty());
        // Same id@version, new content (a different declared revision).
        let manifest = dir.path().join(format!("{ID}@1.1.0/component.toml"));
        let edited = std::fs::read_to_string(&manifest)
            .unwrap()
            .replace(&"1".repeat(64), &"2".repeat(64));
        std::fs::write(&manifest, edited).unwrap();
        reindex_library_at(dir.path()).unwrap();

        let before = placed(&state, &a).await;
        let err = apply_component_upgrades_at(dir.path(), &state, plan)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("upgrade plan is stale"), "{err}");
        assert_eq!(placed(&state, &a).await, before);
    }

    #[tokio::test]
    async fn a_generator_to_document_upgrade_drops_overrides_and_applies() {
        let dir = tempfile::tempdir().unwrap();
        write_version(dir.path(), "1.0.0");
        reindex_library_at(dir.path()).unwrap();
        let state = blank_state().await;
        let id = place(
            dir.path(),
            &state,
            Params::from([("length".into(), num(30.0))]),
        )
        .await;
        // v1.1.0 is a baked `document` package of the same component.
        let mut library = Library::open(dir.path()).unwrap();
        let mut package = component_package_at("test", dir.path(), ID, "1.0.0").unwrap();
        package.identity.version = "1.1.0".into();
        package.parameters.clear();
        library
            .save_component(onecad_library::NewComponent {
                package,
                document: b"frozen".to_vec(),
                geometry: b"baked".to_vec(),
                geometry_codec: "xbf".into(),
                geometry_format: Some(1),
                preview_png: None,
            })
            .unwrap();

        let plan = plan_component_upgrades_at(dir.path(), &state, vec![id.clone()], None)
            .await
            .unwrap();
        assert!(plan[0].applicable, "{:?}", plan[0].refusal);
        assert!(plan[0].kept_overrides.is_empty());
        assert_eq!(plan[0].dropped_overrides.len(), 1);
        assert_eq!(
            plan[0].dropped_overrides[0].reason,
            "document source has no free parameters to carry across versions"
        );
        apply_component_upgrades_at(dir.path(), &state, plan)
            .await
            .expect("the plan that said applicable applies");
        let after = placed(&state, &id).await;
        assert_eq!(after.component_version, "1.1.0");
        assert!(after.params.is_empty());
        assert!(matches!(after.source, ComponentSourceRef::Document { .. }));
    }

    #[tokio::test]
    async fn a_legacy_manifest_key_default_resolves_the_designation() {
        // The pre-SEED_VERSION-6 seed shape: `key = "M6"` and no `value`.
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("onecad.std.iso4032@1.0.0");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("component.toml"),
            format!(
                r#"
[identity]
id = "onecad.std.iso4032"
version = "1.0.0"
revision = "sha256:{}"
[metadata]
name = "Hex Nut"
designation = "ISO 4032 {{thread}}"
[source]
kind = "generator"
generator = "iso4032"
generator_version = 1
[parameters]
thread = {{ role = "free", key = "M6", domain = ["M5", "M6", "M8"] }}
"#,
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            ),
        )
        .unwrap();
        reindex_library_at(dir.path()).unwrap();
        let state = blank_state().await;
        place_component_at(
            dir.path(),
            &state,
            "onecad.std.iso4032".into(),
            "1.0.0".into(),
            [0.0; 3],
            None,
            Params::new(),
            None,
        )
        .await
        .expect("place");
        let list = component_instances_at(Some(dir.path()), &state)
            .await
            .unwrap();
        assert_eq!(list[0].defaults["thread"], text("M6"));
        assert_eq!(list[0].designation.as_deref(), Some("ISO 4032 M6"));
    }
}
