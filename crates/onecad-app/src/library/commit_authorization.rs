//! Budgeted, fenced authorization of mutable placement parameters.
use super::*;
use sha2::{Digest, Sha256};

pub(crate) struct CommitAuthorization {
    generation: u64,
    digest: [u8; 32],
}

struct HashWriter(Sha256);
impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn digest<T: serde::Serialize + ?Sized>(value: &T) -> Result<[u8; 32], ApiError> {
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value)
        .map_err(|error| ApiError::InvalidCommand(format!("component authorization: {error}")))?;
    Ok(writer.0.finalize().into())
}

fn component_source(command: &EditCommand) -> Option<&Operation> {
    match command {
        EditCommand::AddOperation { record, .. } => match &record.op {
            Operation::Known(
                KnownOperation::PlaceComponent(_) | KnownOperation::DetachComponent(_),
            ) => Some(&record.op),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn preview_source_matches(
    base: &ComponentSourceRef,
    candidate: &ComponentSourceRef,
    requested: &BTreeMap<String, ComponentParamValue>,
) -> bool {
    if base == candidate {
        return true;
    }
    match (base, candidate) {
        (
            ComponentSourceRef::Generator {
                generator_id: a,
                generator_version: av,
                extra: ae,
                ..
            },
            ComponentSourceRef::Generator {
                generator_id: b,
                generator_version: bv,
                params,
                extra: be,
            },
        ) => a == b && av == bv && ae == be && params == requested,
        (
            ComponentSourceRef::Profile {
                sha256: a,
                codec: ac,
                brep_format: af,
                params: ap,
                extra: ae,
            },
            ComponentSourceRef::Profile {
                sha256: b,
                codec: bc,
                brep_format: bf,
                params: bp,
                extra: be,
            },
        ) => {
            a == b
                && ac == bc
                && af == bf
                && ae == be
                && ap
                    .iter()
                    .all(|(key, value)| bp.get(key) == Some(requested.get(key).unwrap_or(value)))
                && requested
                    .iter()
                    .all(|(key, value)| bp.get(key) == Some(value))
                && bp
                    .keys()
                    .all(|key| ap.contains_key(key) || requested.contains_key(key))
        }
        _ => false,
    }
}

pub(crate) fn reject_generic_add(command: &EditCommand) -> Result<(), ApiError> {
    if component_source(command).is_some() {
        return Err(ApiError::InvalidCommand(
            "component placement requires authenticated applyOperation".into(),
        ));
    }
    Ok(())
}

pub(crate) struct UpdateAuthorization {
    session: String,
    revision: onecad_core::ids::DocumentRevision,
    record: RecordId,
    old_digest: [u8; 32],
    command_digest: [u8; 32],
}

fn same_component_identity(a: &PlaceComponentParams, b: &PlaceComponentParams) -> bool {
    a.component_id == b.component_id
        && a.component_version == b.component_version
        && a.component_revision == b.component_revision
        && a.extra == b.extra
}

pub(crate) fn check_existing_update(
    runtime: &DocumentRuntime,
    command: &EditCommand,
    receipt: Option<&UpdateAuthorization>,
) -> Result<(), ApiError> {
    let EditCommand::UpdateOperationParams { record, op } = command else {
        return Ok(());
    };
    let old = runtime.operation_authority(*record);
    let direct = match (old, op) {
        (
            Some(Operation::Known(KnownOperation::PlaceComponent(a))),
            Operation::Known(KnownOperation::PlaceComponent(b)),
        ) => same_component_identity(a, b) && a.params == b.params && a.source == b.source,
        (
            Some(Operation::Known(KnownOperation::DetachComponent(a))),
            Operation::Known(KnownOperation::DetachComponent(b)),
        ) => a.source == b.source && a.extra == b.extra,
        (
            Some(Operation::Known(
                KnownOperation::PlaceComponent(_) | KnownOperation::DetachComponent(_),
            )),
            _,
        )
        | (
            _,
            Operation::Known(
                KnownOperation::PlaceComponent(_) | KnownOperation::DetachComponent(_),
            ),
        ) => false,
        _ => return Ok(()),
    };
    if direct {
        return Ok(());
    }
    if let (Some(receipt), Some(old)) = (receipt, old) {
        if receipt.record == *record
            && runtime.matches_runtime_session(&receipt.session)
            && runtime.revision() == receipt.revision
            && digest(old)? == receipt.old_digest
            && digest(command)? == receipt.command_digest
        {
            return Ok(());
        }
    }
    Err(ApiError::InvalidCommand(
        "component source or parameters require an authenticated library edit".into(),
    ))
}

pub(crate) async fn authorize_update(
    state: &AppState,
    command: &EditCommand,
) -> Result<Option<UpdateAuthorization>, ApiError> {
    let EditCommand::UpdateOperationParams {
        record,
        op: Operation::Known(KnownOperation::PlaceComponent(candidate)),
    } = command
    else {
        return Ok(None);
    };
    {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("component update".into()))?;
        let old = runtime
            .operation_authority(*record)
            .ok_or_else(|| ApiError::InvalidCommand("component record is unavailable".into()))?;
        let Operation::Known(KnownOperation::PlaceComponent(current)) = old else {
            return Err(ApiError::InvalidCommand(
                "component record changed kind".into(),
            ));
        };
        if !same_component_identity(current, candidate) {
            return Err(ApiError::InvalidCommand(
                "component package identity changed".into(),
            ));
        }
        if current.params == candidate.params && current.source == candidate.source {
            return Ok(None);
        }
    }
    let scratch = state
        .archive_budget()
        .reserve(
            MAX_CAPTURED_DRAFT_BYTES as usize,
            crate::resource_admission::ArchiveUse::Scratch,
        )
        .map_err(ApiError::InvalidCommand)?;
    let (current, session, revision, old_digest, archive) = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("component update".into()))?;
        let old = runtime
            .operation_authority(*record)
            .ok_or_else(|| ApiError::InvalidCommand("component record is unavailable".into()))?;
        let Operation::Known(KnownOperation::PlaceComponent(current)) = old else {
            return Err(ApiError::InvalidCommand(
                "component record changed kind".into(),
            ));
        };
        if !same_component_identity(current, candidate) {
            return Err(ApiError::InvalidCommand(
                "component package identity changed".into(),
            ));
        }
        if current.params == candidate.params && current.source == candidate.source {
            return Ok(None);
        }
        let archive = runtime
            .captured_library_archive(
                &current.component_id,
                &current.component_version,
                &current.component_revision,
            )?
            .ok_or_else(|| {
                ApiError::InvalidCommand(
                    "component archive unavailable for authenticated edit".into(),
                )
            })?;
        (
            current.clone(),
            runtime.runtime_session(),
            runtime.revision(),
            digest(old)?,
            archive,
        )
    };
    let budget = state.archive_budget();
    let pinned_revision = current.component_revision.clone();
    let (snapshot, _retention) = tokio::task::spawn_blocking(move || {
        crate::library_archive::decode(&archive.0, &pinned_revision, &archive.1, &budget)
    })
    .await
    .map_err(|error| {
        ApiError::Internal(format!(
            "component update authorization task failed: {error}"
        ))
    })??;
    check_source(
        &snapshot.package,
        &current.component_id,
        &current.source,
        &candidate.source,
        Some(&candidate.params),
    )?;
    let command_digest = digest(command)?;
    drop(scratch);
    Ok(Some(UpdateAuthorization {
        session,
        revision,
        record: *record,
        old_digest,
        command_digest,
    }))
}

fn check_source(
    package: &ComponentPackage,
    component_id: &str,
    base: &ComponentSourceRef,
    candidate: &ComponentSourceRef,
    requested: Option<&BTreeMap<String, ComponentParamValue>>,
) -> Result<(), ApiError> {
    match (base, candidate) {
        (ComponentSourceRef::Generator { .. }, ComponentSourceRef::Generator { params, .. })
        | (ComponentSourceRef::Profile { .. }, ComponentSourceRef::Profile { params, .. }) => {
            let free = if let Some(requested) = requested {
                requested.clone()
            } else {
                params
                    .iter()
                    .filter(|(key, _)| {
                        package
                            .parameters
                            .get(*key)
                            .is_some_and(|spec| spec.role == ParameterRole::Free)
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            };
            validate_component_params("placeComponent", package, component_id, &free)?;
            if source_with_free_params(base.clone(), &free, component_id)? != *candidate {
                return Err(ApiError::InvalidCommand(
                    "component source changed outside permitted parameters".into(),
                ));
            }
        }
        (ComponentSourceRef::Document { params, .. }, _) => {
            if base != candidate || requested.is_some_and(|requested| requested != params) {
                return Err(ApiError::InvalidCommand(
                    "baked component source changed".into(),
                ));
            }
        }
        (ComponentSourceRef::Embedded { .. }, _) => {
            if base != candidate || requested.is_some_and(|requested| !requested.is_empty()) {
                return Err(ApiError::InvalidCommand(
                    "baked component source changed".into(),
                ));
            }
        }
        _ => {
            return Err(ApiError::InvalidCommand(
                "component source kind changed".into(),
            ))
        }
    }
    Ok(())
}

pub(crate) async fn authorize(
    state: &AppState,
    commands: &[EditCommand],
    session: &str,
) -> Result<Option<CommitAuthorization>, ApiError> {
    if !commands
        .iter()
        .any(|command| component_source(command).is_some())
    {
        return Ok(None);
    }
    let scratch = state
        .archive_budget()
        .reserve(
            MAX_CAPTURED_DRAFT_BYTES as usize,
            crate::resource_admission::ArchiveUse::Scratch,
        )
        .map_err(ApiError::InvalidCommand)?;
    let (generation, archive, base) = {
        let slot = state.captured_library_draft.lock().unwrap();
        let draft = slot
            .as_ref()
            .filter(|draft| draft.runtime_session == session)
            .ok_or_else(|| {
                ApiError::StalePreview("component draft expired before authorization".into())
            })?;
        (
            component_draft_generation(state),
            draft.archive.clone().ok_or_else(|| {
                ApiError::StalePreview("component archive already committed".into())
            })?,
            draft.source.clone(),
        )
    };
    let budget = state.archive_budget();
    let decoded = tokio::task::spawn_blocking(move || {
        crate::library_archive::decode(
            &archive.reference,
            &archive.revision,
            &archive.bytes,
            &budget,
        )
        .map(|(snapshot, retention)| (snapshot, retention, archive))
    })
    .await
    .map_err(|error| {
        ApiError::Internal(format!("component authorization task failed: {error}"))
    })??;
    let (snapshot, _retention, archive) = decoded;
    for command in commands {
        match component_source(command) {
            Some(Operation::Known(KnownOperation::PlaceComponent(params))) => {
                if params.component_id != archive.reference.component_id
                    || params.component_version != archive.reference.component_version
                    || params.component_revision != archive.revision
                {
                    return Err(ApiError::InvalidCommand(
                        "component package identity changed".into(),
                    ));
                }
                check_source(
                    &snapshot.package,
                    &params.component_id,
                    &base,
                    &params.source,
                    Some(&params.params),
                )?;
            }
            Some(Operation::Known(KnownOperation::DetachComponent(params))) => {
                check_source(
                    &snapshot.package,
                    &archive.reference.component_id,
                    &base,
                    &params.source,
                    None,
                )?;
            }
            _ => {}
        }
    }
    let digest = digest(commands)?;
    drop(scratch);
    Ok(Some(CommitAuthorization { generation, digest }))
}

pub(crate) fn check(
    state: &AppState,
    commands: &[EditCommand],
    session: &str,
    draft: Option<&CapturedDraft>,
    authorization: Option<&CommitAuthorization>,
) -> Result<(), ApiError> {
    if !commands
        .iter()
        .any(|command| component_source(command).is_some())
    {
        return Ok(());
    }
    let authorization = authorization
        .ok_or_else(|| ApiError::StalePreview("component authorization missing".into()))?;
    if draft.is_none_or(|draft| draft.runtime_session != session || draft.archive.is_none())
        || component_draft_generation(state) != authorization.generation
        || digest(commands)? != authorization.digest
    {
        return Err(ApiError::StalePreview(
            "component draft changed before commit".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> ComponentPackage {
        let manifest = format!(
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
thread = {{ role = "free", value = "M3", domain = ["M3", "M6"] }}
length = {{ role = "free", value = 20, min = 2 }}
locked = {{ role = "table", from = "table.key" }}
"#,
            "0".repeat(64)
        );
        package::parse(&manifest, Path::new("component.toml")).unwrap()
    }

    fn generated(thread: &str) -> ComponentSourceRef {
        ComponentSourceRef::Generator {
            generator_id: "g".into(),
            generator_version: 1,
            params: BTreeMap::from([
                ("thread".into(), ComponentParamValue::Text(thread.into())),
                ("length".into(), ComponentParamValue::Number(20.0)),
            ]),
            extra: Default::default(),
        }
    }

    #[test]
    fn dynamic_parameters_are_validated_for_linked_and_copy_commits() {
        let package = package();
        let base = generated("M3");
        let candidate = generated("M6");
        let ComponentSourceRef::Generator { params, .. } = &candidate else {
            unreachable!()
        };
        assert!(preview_source_matches(&base, &base, &BTreeMap::new()));
        assert!(preview_source_matches(&base, &candidate, params));
        assert!(check_source(&package, "acme.part", &base, &candidate, Some(params)).is_ok());
        assert!(check_source(&package, "acme.part", &base, &candidate, None).is_ok());
        for (key, value) in [
            ("thread", ComponentParamValue::Text("M9".into())),
            ("length", ComponentParamValue::Number(1.0)),
            ("locked", ComponentParamValue::Number(2.0)),
            ("unknown", ComponentParamValue::Bool(true)),
        ] {
            let mut forged = params.clone();
            forged.insert(key.into(), value);
            let mutated = base.clone().with_free_params(&forged).unwrap();
            assert!(
                check_source(&package, "acme.part", &base, &mutated, Some(&forged)).is_err(),
                "{key}"
            );
            assert!(
                check_source(&package, "acme.part", &base, &mutated, None).is_err(),
                "copy {key}"
            );
        }
        let mut other = candidate.clone();
        if let ComponentSourceRef::Generator { generator_id, .. } = &mut other {
            *generator_id = "forged".into();
        }
        assert!(!preview_source_matches(&base, &other, params));
        assert!(check_source(&package, "acme.part", &base, &other, Some(params)).is_err());
    }

    #[test]
    fn receipt_rejects_changed_commands_generation_and_session() {
        let state = AppState::default();
        let source = generated("M3");
        let ComponentSourceRef::Generator { params, .. } = &source else {
            unreachable!()
        };
        let params = PlaceComponentParams {
            component_id: "acme.part".into(),
            component_version: "1.0.0".into(),
            component_revision: format!("sha256:{}", "0".repeat(64)),
            params: params.clone(),
            source: source.clone(),
            mate: None,
            placement: FrozenPlacement {
                translate: [Scalar::new(0.0), Scalar::new(0.0), Scalar::new(0.0)],
                rotate: Default::default(),
            },
            extra: Default::default(),
        };
        let record = OperationRecord::new(
            RecordId::new(),
            0,
            "Place",
            Operation::Known(KnownOperation::PlaceComponent(params.clone())),
        );
        let commands = vec![EditCommand::AddOperation {
            record,
            at_cursor: true,
        }];
        let draft = CapturedDraft {
            workspace: None,
            archive: Some(crate::library_archive::CapturedArchive {
                reference: crate::library_archive::ArchiveRef {
                    component_id: params.component_id.clone(),
                    component_version: params.component_version.clone(),
                    archive_sha256: "a".repeat(64),
                    extra: Default::default(),
                },
                revision: params.component_revision.clone(),
                bytes: Arc::new(onecad_core::io::library::ArchiveBytes::untracked(Vec::new())),
            }),
            source,
            component_id: params.component_id,
            component_version: params.component_version,
            component_revision: params.component_revision,
            source_sha: None,
            runtime_session: "session".into(),
            blob: None,
        };
        let receipt = CommitAuthorization {
            generation: component_draft_generation(&state),
            digest: digest(&commands).unwrap(),
        };
        assert!(check(&state, &commands, "session", Some(&draft), Some(&receipt)).is_ok());
        assert!(check(&state, &commands, "other", Some(&draft), Some(&receipt)).is_err());
        let mut changed = commands.clone();
        if let EditCommand::AddOperation { record, .. } = &mut changed[0] {
            record.name = "changed".into();
        }
        assert!(check(&state, &changed, "session", Some(&draft), Some(&receipt)).is_err());
        begin_component_draft_capture(&state).unwrap();
        assert!(check(&state, &commands, "session", Some(&draft), Some(&receipt)).is_err());
    }

    #[test]
    fn baked_sources_cannot_be_replaced_or_relabelled() {
        let package = package();
        let base = ComponentSourceRef::Embedded {
            sha256: "a".repeat(64),
            codec: ImportSourceCodec::Step,
            brep_format: None,
            extra: Default::default(),
        };
        let mut changed = base.clone();
        if let ComponentSourceRef::Embedded { sha256, .. } = &mut changed {
            *sha256 = "b".repeat(64);
        }
        assert!(check_source(&package, "acme.part", &base, &base, Some(&BTreeMap::new())).is_ok());
        assert!(check_source(&package, "acme.part", &base, &changed, None).is_err());
    }
}
