//! An admitted checkpoint owns its snapshot, recovery pair and operation leases.
use super::*;

/// A Save As of an older revision changes where recovery should later save, but
/// does not supersede the newer checkpoint's bytes or original-source provenance.
pub(crate) fn retarget_recovery(
    root: &Path,
    runtime: &DocumentRuntime,
    source_paths: Vec<PathBuf>,
) {
    let id = runtime.document_uuid();
    if !autosave_path(root, id).is_file() {
        return;
    }
    let old: Option<SessionMarker> =
        std::fs::read(onecad_core::io::recovery::marker_path(root, id))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let marker = SessionMarker {
        document_id: id,
        pid: std::process::id(),
        opened_path: runtime.path().map(Path::to_path_buf),
        source_paths,
        title: Some(runtime.title().to_owned()),
        last_autosave: old.map_or_else(String::new, |marker| marker.last_autosave),
    };
    if let Err(error) = write_marker(root, &marker) {
        tracing::warn!(%error, "save: could not retarget recovery metadata");
    }
}

pub(super) async fn attempt_admitted(
    runtime: &Mutex<Option<DocumentRuntime>>,
    app_data: &Path,
    expected_session: Option<&str>,
    lane: tokio::sync::OwnedMutexGuard<()>,
    ownership: Option<Ownership>,
) -> AutosaveAttempt {
    let now = now_rfc3339();
    let meta = SaveMeta {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        occt_fingerprint: None,
        created: now.clone(),
        modified: now.clone(),
    };
    // ── Under the runtime lock: snapshot only. No IO, no serialization. ──
    //    `SaveCaches::none()`: an autosave writes no mesh/preview cache. It fires on
    //    a timer against a document the user is actively editing, and its container
    //    is only ever read by crash recovery — which regens from 0 regardless. The
    //    "paint at open" caches belong to an explicit save.
    let (payload, doc_id, opened, title, leases, source_paths) = {
        let mut guard = runtime.lock().await;
        let Some(rt) = guard.as_mut() else {
            return AutosaveAttempt::Skipped; // no document open.
        };
        if expected_session.is_some_and(|session| !rt.matches_runtime_session(session))
            || !rt.is_dirty()
        {
            return AutosaveAttempt::Skipped; // nothing at risk.
        }
        let doc_id = rt.document_uuid();
        let leases = match ownership
            .as_ref()
            .map(|owner| owner.lease(doc_id))
            .transpose()
        {
            Ok(leases) => leases,
            Err(error) => {
                return AutosaveAttempt::Failed(failed_event(
                    &autosave_path(app_data, doc_id),
                    format!("checkpoint ownership: {error}"),
                ))
            }
        };
        let source_paths = ownership
            .as_ref()
            .map_or_else(Vec::new, Ownership::source_paths);
        let opened = rt.path().map(Path::to_path_buf);
        let title = rt.title().to_string();
        (
            rt.build_save_payload(meta, SaveCaches::none()),
            doc_id,
            opened,
            title,
            leases,
            source_paths,
        )
    };
    let path = autosave_path(app_data, doc_id);

    let marker = SessionMarker {
        document_id: doc_id,
        pid: std::process::id(),
        opened_path: opened,
        source_paths,
        last_autosave: now,
        title: Some(title),
    };
    let target = path.clone();
    let root = app_data.to_owned();
    let work = crate::persistence::spawn(
        lane,
        leases.unwrap_or_default(),
        move |admission| async move {
            tokio::task::spawn_blocking(move || {
                let _admission = admission;
                std::fs::create_dir_all(autosave_dir(&root))?;
                checkpoint_store::preserve_pair(
                    &target,
                    &onecad_core::io::recovery::marker_path(&root, doc_id),
                    || {
                        DocumentRuntime::write_payload(&target, &payload)?;
                        write_marker(&root, &marker)
                    },
                )
            })
            .await
        },
    )
    .await;
    match work {
        Ok(Ok(Ok(()))) => AutosaveAttempt::Written(AutosaveEvent {
            path: path.to_string_lossy().into_owned(),
            at_ms: now_ms(),
            error: None,
        }),
        Ok(Ok(Err(error))) => {
            AutosaveAttempt::Failed(failed_event(&path, format!("write checkpoint: {error}")))
        }
        other => {
            AutosaveAttempt::Failed(failed_event(&path, format!("checkpoint task: {other:?}")))
        }
    }
}

/// An [`AutosaveEvent`] describing a failed attempt at `path`.
fn failed_event(path: &Path, message: String) -> AutosaveEvent {
    AutosaveEvent {
        path: path.to_string_lossy().into_owned(),
        at_ms: now_ms(),
        error: Some(message),
    }
}
