//! Backend side of the library lane: calls the real application services.
use super::{Classified, ClassifyTarget, PreviewSummary, Reply, Request, thumbs};
use crate::bridge::{Backend, Message};
use onecad_app::{api, host::HostContext, library, state::AppState};
use onecad_protocol::mesh::validate_mesh_blob;
use std::{
    future::Future,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::mpsc,
    task::{Context, Poll},
};
use tokio::sync::OwnedSemaphorePermit;

pub(crate) async fn execute(
    backend: &mut Backend,
    request: Request,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    let sequence = request.sequence();
    let result = if matches!(request, Request::Prepare { .. }) {
        let host = backend.host.clone();
        let state = backend.state.clone();
        let shared = backend.shared.clone();
        tokio::spawn(async move { prepare_admitted(host, state, shared, request).await })
            .await
            .unwrap_or_else(|error| Err(format!("component preparation stopped: {error}")))
    } else {
        guarded(run(&backend.host, &backend.state, request)).await
    };
    let _ = sender.send(Message::Library {
        sequence,
        result,
        _permit: permit,
    });
    backend.shared.wake();
}

async fn prepare_admitted(
    host: HostContext,
    state: std::sync::Arc<AppState>,
    shared: std::sync::Arc<crate::bridge::Shared>,
    mut request: Request,
) -> Result<Reply, String> {
    use crate::budget::Kind;
    const CAPTURE_BYTES: usize = 32 << 20;
    let generation = match &request {
        Request::Prepare {
            generation: Some(generation),
            ..
        } => *generation,
        _ => library::begin_component_draft_capture(&state).map_err(|error| error.to_string())?,
    };
    if let Request::Prepare {
        generation: ticket, ..
    } = &mut request
    {
        *ticket = Some(generation);
    }
    shared.relieve_cpu_pressure(CAPTURE_BYTES);
    let lease = shared
        .host_budget
        .admit_while(CAPTURE_BYTES, Kind::LibraryDraft, || {
            !shared.stopping.load(std::sync::atomic::Ordering::Acquire)
                && library::component_draft_generation(&state) == generation
        })
        .await?;
    let result = guarded(run(&host, &state, request)).await;
    let mut retained = shared
        .library_capture_lease
        .lock()
        .map_err(|_| "Component capture lease lock poisoned")?;
    if library::component_draft_generation(&state) != generation {
        return Err("Component preparation superseded or cancelled".into());
    }
    if result.is_ok() && !shared.stopping.load(std::sync::atomic::Ordering::Acquire) {
        let bytes = library::captured_component_draft_source_bytes(&state);
        lease.shrink_to(bytes)?;
        *retained = (bytes > 0).then_some(lease);
    } else {
        library::clear_component_draft_capture(&state);
        *retained = None;
    }
    result
}

pub(super) async fn run(
    host: &HostContext,
    state: &AppState,
    request: Request,
) -> Result<Reply, String> {
    if matches!(
        &request,
        Request::Search { .. }
            | Request::Detail { .. }
            | Request::Reindex { .. }
            | Request::Preview { .. }
            | Request::Prepare { .. }
    ) {
        state.await_library_initialization().await?;
    }
    match request {
        Request::Search { query, .. } => library::search_library_components(host.clone(), query)
            .await
            .map(Reply::Searched)
            .map_err(|error| error.to_string()),
        Request::Detail { id, version, .. } => library::component_detail(host.clone(), id, version)
            .await
            .map(|detail| Reply::Detail(Box::new(detail)))
            .map_err(|error| error.to_string()),
        Request::Reindex { .. } => library::reindex_library(host.clone())
            .await
            .map(Reply::Reindexed)
            .map_err(|error| error.to_string()),
        Request::Preview {
            id,
            version,
            params,
            ..
        } => {
            let preview =
                library::component_preview_mesh(host.clone(), state, id, version, Some(params))
                    .await
                    .map_err(|error| error.to_string())?;
            summarize(&preview).map(Reply::Previewed)
        }
        Request::Thumbnails { items, .. } => decode_all(items).await.map(Reply::Thumbnails),
        Request::Prepare {
            request,
            generation,
            ..
        } => {
            let prepared = match generation {
                Some(generation) => {
                    library::prepare_component_draft_in_generation(
                        host, state, *request, generation,
                    )
                    .await
                }
                None => library::prepare_component_draft(host, state, *request).await,
            };
            prepared
                .map(|params| Reply::Prepared(Box::new(params)))
                .map_err(|error| error.to_string())
        }
        Request::PrepareEdit { params, .. } => prepare_edit(host, state, *params).await,
        Request::Classify { target, .. } => classify(state, *target)
            .await
            .map(|known| Reply::Classified(Box::new(known))),
    }
}

/// An existing instance's geometry must be in the document and its definition known,
/// else the edit is refused: attachments come from the definition.
async fn prepare_edit(
    host: &HostContext,
    state: &AppState,
    params: onecad_core::document::record::PlaceComponentParams,
) -> Result<Reply, String> {
    let session = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or("the document is closed")?
            .runtime_session()
    };
    library::restage_component_blob(host, state, &params)
        .await
        .map_err(|error| format!("the component's geometry is unavailable: {error}"))?;
    let archived = library::component_detail_archived(
        state,
        &params.component_id,
        &params.component_version,
        &params.component_revision,
    )
    .await
    .map_err(|error| format!("the component definition is unavailable: {error}"))?;
    let detail = match archived {
        Some(detail) => detail,
        None => {
            state.await_library_initialization().await?;
            library::component_detail_pinned(
                host.clone(),
                params.component_id.clone(),
                params.component_version.clone(),
                params.component_revision.clone(),
            )
            .await
            .map_err(|error| format!("the component definition is unavailable: {error}"))?
        }
    };
    {
        let guard = state.runtime.lock().await;
        if guard
            .as_ref()
            .is_none_or(|runtime| !runtime.matches_runtime_session(&session))
        {
            return Err("the document changed while preparing the component edit".into());
        }
    }
    Ok(Reply::Edit(Box::new(detail.component)))
}

/// Classifies the target and, when a mate can seat on it, mints its identity once.
async fn classify(state: &AppState, target: ClassifyTarget) -> Result<Classified, String> {
    let class = api::classify_element(
        state,
        target.body_id.clone(),
        target.element_id.clone().unwrap_or_default(),
        target.topo_key.clone(),
        target.fence.clone(),
    )
    .await
    .map_err(|error| error.to_string())?;
    let seatable = class.as_ref().is_some_and(|c| c.frame.is_some());
    // Hover identity is STAGED, never persisted: only a committed record that
    // names it adopts it, so sweeping over faces leaves the document clean.
    let element_id = match (
        &target.element_id,
        seatable,
        &target.topo_key,
        &target.fence,
    ) {
        (Some(id), true, _, _) => Some(id.clone()),
        (None, true, Some(key), Some(fence)) => Some(
            library::stage_mate_target(state, &target.body_id, key, target.point, fence)
                .await
                .map_err(|error| match error {
                    onecad_app::error::ApiError::StalePreview(_) => {
                        "The snap target is stale; move the pointer to retry".to_string()
                    }
                    other => other.to_string(),
                })?,
        ),
        _ => None,
    };
    Ok(Classified { class, element_id })
}

/// Decoding is CPU work and file I/O: kept off the service actor's async thread.
async fn decode_all(
    items: Vec<(thumbs::Key, std::path::PathBuf)>,
) -> Result<Vec<(thumbs::Key, Result<thumbs::Decoded, String>)>, String> {
    tokio::task::spawn_blocking(move || {
        items
            .into_iter()
            .map(|(key, path)| (key, thumbs::decode(&path)))
            .collect()
    })
    .await
    .map_err(|error| format!("preview decode stopped: {error}"))
}

const NO_GEOMETRY: &str = "preview returned no usable geometry";

/// One previewed body's header facts.
#[derive(Clone, Copy, Debug)]
pub(super) struct BodyBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub triangles: u32,
}

/// Union of the bodies' bounding boxes. A mesh the checked decoder refuses is an error,
/// and so is a result with a non-finite box or no triangles: it is not a zero-size part.
fn summarize(preview: &onecad_app::dto::PreviewResultDto) -> Result<PreviewSummary, String> {
    let mut bodies = Vec::with_capacity(preview.bodies.len());
    for body in &preview.bodies {
        let header = validate_mesh_blob(&body.mesh)
            .map_err(|error| format!("Preview mesh refused: {error}"))?;
        bodies.push(BodyBounds {
            min: header.bbox_min,
            max: header.bbox_max,
            triangles: header.triangle_count,
        });
    }
    combine(&bodies)
}

pub(super) fn combine(bodies: &[BodyBounds]) -> Result<PreviewSummary, String> {
    let usable = |b: &BodyBounds| {
        b.min.iter().chain(&b.max).all(|v| v.is_finite())
            && (0..3).all(|axis| b.max[axis] >= b.min[axis])
    };
    let triangles: u64 = bodies.iter().map(|b| u64::from(b.triangles)).sum();
    if bodies.is_empty() || triangles == 0 || !bodies.iter().all(usable) {
        return Err(NO_GEOMETRY.into());
    }
    let size = std::array::from_fn(|axis| {
        let low = bodies
            .iter()
            .map(|b| b.min[axis])
            .fold(f32::INFINITY, f32::min);
        let high = bodies
            .iter()
            .map(|b| b.max[axis])
            .fold(f32::NEG_INFINITY, f32::max);
        f64::from(high - low)
    });
    Ok(PreviewSummary {
        size,
        bodies: bodies.len(),
        triangles,
    })
}

/// Turns a panic in `work` into an error, so the lane always gets its reply.
pub(crate) async fn guarded<T>(work: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    struct Guard<F>(Pin<Box<F>>);
    impl<F: Future> Future for Guard<F> {
        type Output = std::thread::Result<F::Output>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let poll = std::panic::catch_unwind(AssertUnwindSafe(|| self.0.as_mut().poll(cx)));
            match poll {
                Ok(Poll::Pending) => Poll::Pending,
                Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
                Err(panic) => Poll::Ready(Err(panic)),
            }
        }
    }
    match Guard(Box::pin(work)).await {
        Ok(result) => result,
        Err(panic) => {
            let why = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            Err(format!("library service stopped unexpectedly: {why}"))
        }
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostPaths};
    use std::{collections::BTreeMap, sync::Arc};

    struct Ports;
    impl EventSink for Ports {
        fn emit(&self, _: AppEvent) -> Result<(), String> {
            Ok(())
        }
    }
    #[async_trait::async_trait]
    impl DialogPort for Ports {
        async fn pick(&self, _: FileDialog) -> Vec<String> {
            Vec::new()
        }
    }

    fn host(path: &std::path::Path) -> HostContext {
        let ports = Arc::new(Ports);
        HostContext {
            paths: HostPaths {
                app_data: Some(path.to_owned()),
                app_config: Some(path.join("config")),
            },
            events: ports.clone(),
            dialogs: ports,
        }
    }

    fn shared() -> Arc<crate::bridge::Shared> {
        Arc::new(crate::bridge::Shared::new(
            egui::Context::default(),
            onecad_render::PreparationBudget::cpu_validation(),
        ))
    }

    fn request(generation: u64) -> Request {
        Request::Prepare {
            sequence: generation,
            generation: Some(generation),
            request: Box::new(library::PlaceComponentRequest {
                component_id: "missing".into(),
                component_version: "1".into(),
                translate: [0.0; 3],
                rotate: None,
                params: BTreeMap::new(),
                mate: None,
                configuration: None,
            }),
        }
    }

    #[tokio::test]
    async fn superseded_cleanup_does_not_cancel_queued_generation() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("unread-library");
        let state = Arc::new(AppState::default());
        let shared = shared();
        let first = library::begin_component_draft_capture(&state).unwrap();
        let runtime_guard = state.runtime.lock().await;
        let pending = prepare_admitted(host(&data), state.clone(), shared.clone(), request(first));
        tokio::pin!(pending);
        std::future::poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(shared.host_budget.snapshot().used, 32 << 20);
        let second = library::begin_component_draft_capture(&state).unwrap();
        let queued = request(second);
        drop(runtime_guard);
        assert!(
            matches!(pending.await, Err(error) if error == "Component preparation superseded or cancelled")
        );
        assert_eq!(library::component_draft_generation(&state), second);
        let result = prepare_admitted(host(&data), state.clone(), shared.clone(), queued).await;
        assert!(matches!(result, Err(error) if error.contains("no document open")));
        assert!(!data.exists());
        assert_eq!(shared.host_budget.snapshot().used, 0);
    }

    #[tokio::test]
    async fn cancelled_before_service_refuses_before_source_access() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("unread-library");
        let state = Arc::new(AppState::default());
        let shared = shared();
        let generation = library::begin_component_draft_capture(&state).unwrap();
        library::clear_component_draft_capture(&state);
        let result =
            prepare_admitted(host(&data), state, shared.clone(), request(generation)).await;
        assert!(matches!(result, Err(error) if error.contains("admission cancelled")));
        assert!(!data.exists());
        assert_eq!(shared.host_budget.snapshot().used, 0);
    }
    #[tokio::test]
    async fn edit_details_reopen_from_archive_without_library() {
        use onecad_core::document::record::{KnownOperation, Operation, OperationRecord};
        use onecad_core::edit::EditCommand;
        use onecad_core::ids::{RecordId, SnapshotId};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let package_dir = root.join("example.offline");
        std::fs::create_dir_all(&package_dir).unwrap();
        std::fs::write(package_dir.join("source.onecad"), b"frozen source").unwrap();
        let sha = onecad_library::blob::BlobStore::new(&root)
            .put(b"offline geometry")
            .unwrap();
        let raw = format!(
            "[identity]\nid = \"example.offline\"\nversion = \"1.0.0\"\nrevision = \"sha256:{}\"\n[metadata]\nname = \"Offline\"\n[source]\nkind = \"document\"\nfile = \"source.onecad\"\ngeometry = \"sha256:{sha}\"\ngeometry_codec = \"brep\"\ngeometry_format = 4\n",
            "0".repeat(64)
        );
        std::fs::write(package_dir.join("component.toml"), &raw).unwrap();
        let pkg = onecad_library::package::parse(&raw, &package_dir).unwrap();
        onecad_library::package::write_package(&package_dir, &pkg).unwrap();
        library::reindex_library_at(&root).unwrap();
        let state = AppState::new(Arc::new(|| {
            let pending = Arc::new(onecad_app::worker::PendingBackend);
            (
                pending.clone(),
                pending.clone(),
                pending.clone(),
                pending.clone(),
                pending.clone(),
                pending.clone(),
                pending.clone(),
                pending.clone(),
                pending.clone(),
                pending,
            )
        }));
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(
            onecad_app::document_runtime::DocumentRuntime::new_blank(engine, meshes, solver),
        );
        let params = library::prepare_component_draft_at(
            &root,
            &state,
            library::PlaceComponentRequest {
                component_id: "example.offline".into(),
                component_version: "1.0.0".into(),
                translate: [0.0; 3],
                rotate: None,
                params: BTreeMap::new(),
                mate: None,
                configuration: None,
            },
        )
        .await
        .unwrap();
        let context = {
            let guard = state.runtime.lock().await;
            let rt = guard.as_ref().unwrap();
            api::PreviewContext {
                document_id: rt.document_id(),
                runtime_session: rt.runtime_session(),
                revision: rt.revision(),
                worker_epoch: rt.fencing().get().1,
                snapshot_id: rt.head_snapshot_id().unwrap_or(SnapshotId(0)),
                expected_base_hash: onecad_core::regen::HistoryPrefixHash::empty(),
            }
        };
        let command = EditCommand::AddOperation {
            record: OperationRecord::new(
                RecordId::new(),
                0,
                "Offline",
                Operation::Known(KnownOperation::PlaceComponent(params.clone())),
            ),
            at_cursor: false,
        };
        api::apply_operation_in_context(&state, host(temp.path()), vec![command], None, &context)
            .await
            .unwrap();
        let saved = temp.path().join("saved.onecad");
        state
            .runtime
            .lock()
            .await
            .as_mut()
            .unwrap()
            .save(&saved, Default::default())
            .unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        library::clear_component_draft_capture(&state);
        let (engine, meshes, solver) = state.make_backend();
        *state.runtime.lock().await = Some(
            onecad_app::document_runtime::DocumentRuntime::open(&saved, engine, meshes, solver)
                .unwrap(),
        );
        let Reply::Edit(detail) = prepare_edit(&host(temp.path()), &state, params)
            .await
            .unwrap()
        else {
            panic!("edit detail expected")
        };
        assert_eq!(detail.name, "Offline");
    }
}
