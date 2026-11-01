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
    let result = guarded(run(&backend.host, &backend.state, request)).await;
    let _ = sender.send(Message::Library {
        sequence,
        result,
        _permit: permit,
    });
    backend.shared.wake();
}

pub(super) async fn run(
    host: &HostContext,
    state: &AppState,
    request: Request,
) -> Result<Reply, String> {
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
        Request::Prepare { request, .. } => library::prepare_component_draft(host, state, *request)
            .await
            .map(|params| Reply::Prepared(Box::new(params)))
            .map_err(|error| error.to_string()),
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
    library::restage_component_blob(host, state, &params)
        .await
        .map_err(|error| format!("the component's geometry is unavailable: {error}"))?;
    library::component_detail(
        host.clone(),
        params.component_id.clone(),
        params.component_version.clone(),
    )
    .await
    .map(|detail| Reply::Edit(Box::new(detail.component)))
    .map_err(|error| format!("the component definition is unavailable: {error}"))
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
