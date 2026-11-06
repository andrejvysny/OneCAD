//! Immutable semantic package closures carried alongside a document.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use onecad_core::document::modules::{ModuleId, ModuleState};
use onecad_core::document::Document;
use onecad_core::edit::EditCommand;
use onecad_core::io::library::LibraryArchives;
use serde::{Deserialize, Serialize};

use crate::document_runtime::DocumentRuntime;
use crate::error::ApiError;

const MODULE: &str = "onecad.library";
const LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchiveRef {
    pub component_id: String,
    pub component_version: String,
    pub archive_sha256: String,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

pub(crate) type ArchiveHandle = (ArchiveRef, Arc<onecad_core::io::library::ArchiveBytes>);

#[derive(Clone)]
pub(crate) struct CapturedArchive {
    pub reference: ArchiveRef,
    pub revision: String,
    pub bytes: Arc<onecad_core::io::library::ArchiveBytes>,
}

pub(crate) fn capture(
    snapshot: &onecad_library::resolve::ResolvedSnapshot,
    limit: u64,
    budget: &crate::resource_admission::ArchiveBudget,
) -> Result<CapturedArchive, ApiError> {
    let blob_bytes = match &snapshot.source {
        onecad_library::resolve::ResolvedSource::Generator { .. } => 0,
        onecad_library::resolve::ResolvedSource::Embedded { blob }
        | onecad_library::resolve::ResolvedSource::Profile { blob }
        | onecad_library::resolve::ResolvedSource::Document { blob, .. } => blob.bytes.len() as u64,
    };
    let payload = (snapshot.manifest.len() as u64)
        .checked_add(
            snapshot
                .document
                .as_ref()
                .map_or(0, |bytes| bytes.len() as u64),
        )
        .and_then(|n| n.checked_add(blob_bytes))
        .ok_or_else(|| ApiError::InvalidCommand("component archive size overflow".into()))?;
    // Fixed ZIP headers, names and footer fit within this bound for three entries.
    let archive_bound = payload
        .checked_add(4096)
        .ok_or_else(|| ApiError::InvalidCommand("component archive size overflow".into()))?;
    if payload
        .checked_add(archive_bound)
        .is_none_or(|n| n > limit.min(LIMIT))
    {
        return Err(ApiError::InvalidCommand(
            "component source and archive exceed the 32 MiB limit".into(),
        ));
    }
    let admitted = usize::try_from(archive_bound)
        .map_err(|_| ApiError::InvalidCommand("archive size overflow".into()))?;
    let retention = budget
        .reserve(admitted, crate::resource_admission::ArchiveUse::Retained)
        .map_err(ApiError::InvalidCommand)?;
    let _scratch = budget
        .reserve(
            usize::try_from(payload)
                .map_err(|_| ApiError::InvalidCommand("archive scratch overflow".into()))?,
            crate::resource_admission::ArchiveUse::Scratch,
        )
        .map_err(ApiError::InvalidCommand)?;
    let mut bytes = onecad_core::io::library::ArchiveBytes::zeroed(admitted, retention);
    let output = onecad_library::archive::BoundedWriter::new(bytes.buffer_mut());
    let output = onecad_library::archive::encode_into(snapshot, archive_bound, output)?;
    let used = output.len();
    bytes.set_used(used)?;
    Ok(CapturedArchive {
        reference: ArchiveRef {
            component_id: snapshot.package.identity.id.clone(),
            component_version: snapshot.package.identity.version.clone(),
            archive_sha256: onecad_library::blob::sha256_hex(bytes.as_slice()),
            extra: BTreeMap::new(),
        },
        revision: snapshot.package.identity.revision.clone(),
        bytes: Arc::new(bytes),
    })
}

pub(crate) fn decode(
    entry: &ArchiveRef,
    revision: &str,
    bytes: &[u8],
    budget: &crate::resource_admission::ArchiveBudget,
) -> Result<
    (
        onecad_library::resolve::ResolvedSnapshot,
        crate::resource_admission::Retention,
    ),
    ApiError,
> {
    let reserve = bytes
        .len()
        .checked_mul(2)
        .and_then(|size| size.checked_add(4096))
        .ok_or_else(|| ApiError::InvalidCommand("archive decode size overflow".into()))?;
    let retention = budget
        .reserve(reserve, crate::resource_admission::ArchiveUse::Scratch)
        .map_err(ApiError::InvalidCommand)?;
    let request = onecad_library::resolve::ResolveRequest {
        component_id: &entry.component_id,
        component_version: &entry.component_version,
        component_revision: revision,
    };
    let snapshot = onecad_library::archive::decode(bytes, &entry.archive_sha256, &request, LIMIT)
        .map_err(|error| {
        ApiError::InvalidCommand(format!("component source archive unavailable: {error}"))
    })?;
    Ok((snapshot, retention))
}

fn module_id() -> ModuleId {
    ModuleId::parse(MODULE).expect("fixed module identity")
}

pub(crate) fn references(document: &Document) -> Result<BTreeMap<String, ArchiveRef>, ApiError> {
    let Some(module) = document.modules.get(&module_id()) else {
        return Ok(BTreeMap::new());
    };
    if module.schema_version != 1 {
        return Err(ApiError::InvalidCommand(
            "unsupported component archive metadata version".into(),
        ));
    }
    if !module.payload.is_object() {
        return Err(ApiError::InvalidCommand(
            "invalid component archive metadata".into(),
        ));
    }
    let Some(entries) = module.payload.get("snapshots") else {
        return Ok(BTreeMap::new());
    };
    serde_json::from_value(entries.clone())
        .map_err(|e| ApiError::InvalidCommand(format!("invalid component archive metadata: {e}")))
}

/// Preserve the document's first verified closure for a semantic revision, even
/// when a later catalog manifest differs only in non-semantic formatting.
pub(crate) fn select_archive(
    rt: &DocumentRuntime,
    incoming: &CapturedArchive,
) -> Result<CapturedArchive, ApiError> {
    let reference = &incoming.reference;
    let Some((existing, bytes)) = rt.captured_library_archive(
        &reference.component_id,
        &reference.component_version,
        &incoming.revision,
    )?
    else {
        return Ok(incoming.clone());
    };
    if !rt.library_archive_verified(
        &existing.archive_sha256,
        &incoming.revision,
        &existing.component_id,
        &existing.component_version,
    ) {
        return Err(ApiError::InvalidCommand(
            "carried component archive has not been verified".into(),
        ));
    }
    Ok(CapturedArchive {
        reference: existing,
        revision: incoming.revision.clone(),
        bytes,
    })
}

pub(crate) fn metadata_command(
    rt: &DocumentRuntime,
    archives: &[&CapturedArchive],
) -> Result<EditCommand, ApiError> {
    let id = module_id();
    let mut state = rt
        .module_state(&id)
        .unwrap_or_else(|| ModuleState::new(1, serde_json::json!({})));
    if state.schema_version != 1 {
        return Err(ApiError::InvalidCommand(
            "unsupported component archive metadata version".into(),
        ));
    }
    let object = state
        .payload
        .as_object_mut()
        .ok_or_else(|| ApiError::InvalidCommand("invalid component archive metadata".into()))?;
    let entries = object
        .entry("snapshots")
        .or_insert_with(|| serde_json::json!({}));
    let entries = entries
        .as_object_mut()
        .ok_or_else(|| ApiError::InvalidCommand("invalid component archive metadata".into()))?;
    for archive in archives {
        let value = serde_json::to_value(&archive.reference)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        if entries
            .get(&archive.revision)
            .is_some_and(|existing| existing != &value)
        {
            return Err(ApiError::InvalidCommand(
                "component revision has conflicting archive".into(),
            ));
        }
        entries.insert(archive.revision.clone(), value);
    }
    Ok(EditCommand::SetModuleState {
        module: id,
        state: Some(state),
    })
}

/// Prune known unreferenced revisions only when no opaque module may refer to them.
pub(crate) fn prune_metadata(document: &mut Document) {
    if !references(document).is_ok_and(|refs| refs.values().all(|entry| entry.extra.is_empty())) {
        return;
    }
    if document.modules.keys().any(|id| id != &module_id()) {
        return;
    }
    let mut used = BTreeSet::new();
    for record in document.timeline.records().iter().chain(
        document
            .definitions
            .values()
            .flat_map(|definition| definition.timeline.records()),
    ) {
        if let onecad_core::document::record::Operation::Known(
            onecad_core::document::record::KnownOperation::PlaceComponent(params),
        ) = &record.op
        {
            used.insert(params.component_revision.clone());
        }
    }
    let Some(module) = document.modules.get_mut(&module_id()) else {
        return;
    };
    if module.schema_version != 1 {
        return;
    }
    let Some(object) = module.payload.as_object_mut() else {
        return;
    };
    if object.keys().any(|key| key != "snapshots") {
        return;
    }
    let Some(entries) = object
        .get_mut("snapshots")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    entries.retain(|revision, _| used.contains(revision));
    if entries.is_empty() {
        document.modules.remove(&module_id());
    }
}

pub(crate) fn snapshot_for_save(
    document: &Document,
    archives: &LibraryArchives,
    known: &BTreeSet<String>,
) -> LibraryArchives {
    let has_opaque = document.modules.iter().any(|(id, state)| {
        id != &module_id()
            || state.schema_version != 1
            || state
                .payload
                .as_object()
                .is_none_or(|object| object.keys().any(|key| key != "snapshots"))
    });
    if has_opaque
        || !references(document).is_ok_and(|refs| refs.values().all(|entry| entry.extra.is_empty()))
    {
        return archives.clone();
    }
    let refs = references(document).unwrap_or_default();
    let current: BTreeSet<_> = refs
        .values()
        .map(|entry| entry.archive_sha256.as_str())
        .collect();
    archives
        .iter()
        .filter(|(sha, _)| !known.contains(*sha) || current.contains(sha.as_str()))
        .map(|(sha, bytes)| (sha.clone(), Arc::clone(bytes)))
        .collect()
}

pub(crate) fn require_complete(
    document: &Document,
    archives: &LibraryArchives,
) -> Result<(), String> {
    for (revision, entry) in references(document).map_err(|e| e.to_string())? {
        let bytes = archives
            .get(&entry.archive_sha256)
            .ok_or_else(|| format!("component archive unavailable: {revision}"))?;
        let request = onecad_library::resolve::ResolveRequest {
            component_id: &entry.component_id,
            component_version: &entry.component_version,
            component_revision: &revision,
        };
        onecad_library::archive::decode(bytes, &entry.archive_sha256, &request, LIMIT)
            .map_err(|e| format!("component archive unavailable: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::ids::DocumentId;

    #[test]
    fn future_nested_archive_metadata_is_preserved_conservatively() {
        let mut document = Document::new(DocumentId::new());
        let sha = "a".repeat(64);
        let payload = serde_json::json!({"snapshots": {"sha256v2:revision": {
            "componentId": "example.part", "componentVersion": "1.0.0", "archiveSha256": sha,
            "futureReference": {"anything": true}
        }}});
        document
            .modules
            .insert(module_id(), ModuleState::new(1, payload.clone()));
        let known = BTreeSet::from([sha.clone()]);
        let archives = LibraryArchives::from([(
            sha,
            Arc::new(onecad_core::io::library::ArchiveBytes::untracked(
                b"opaque".to_vec(),
            )),
        )]);
        prune_metadata(&mut document);
        assert_eq!(document.modules[&module_id()].payload, payload);
        assert_eq!(snapshot_for_save(&document, &archives, &known).len(), 1);
    }
}
