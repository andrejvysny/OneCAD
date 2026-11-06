//! Shared startup wiring for desktop and headless hosts.

use std::sync::Arc;

use onecad_core::regen::{CancelToken, RegenScheduler};

use crate::{
    api, autosave,
    host::{AppEvent, HostContext},
    imports, library,
    regen_driver::{regen_driver_with_started, RegenEmitter, RegenPostPublish, RegenStartEmitter},
    sketch_projection,
    state::AppState,
};

/// Starts the application lanes once, from inside the host's Tokio runtime.
pub fn start(state: &AppState, host: HostContext) -> Result<(), &'static str> {
    state
        .host
        .set(host.clone())
        .map_err(|_| "application host already started")?;
    start_library_initialization(state, &host)?;
    if state.services().claim_maintenance() {
        start_maintenance();
    }
    state.prewarm();
    let completion_host = host.clone();
    let emit: RegenEmitter = Arc::new(move |report, projection| {
        api::emit_regen_events(&completion_host, report, projection);
    });
    let start_host = host.clone();
    let on_started: RegenStartEmitter = Arc::new(move |runtime_session| {
        let _ = start_host.emit(AppEvent::RegenStarted(crate::dto::RegenStarted {
            runtime_session: runtime_session.to_owned(),
        }));
    });
    let runtime = state.runtime.clone();
    let projection_source = state.projection_provider();
    let post_publish: RegenPostPublish = Arc::new(move |cancel: CancelToken| {
        let runtime = runtime.clone();
        let projector = projection_source.read().unwrap().clone();
        Box::pin(async move {
            sketch_projection::refresh_projection_staleness(&runtime, &projector, &cancel).await;
        })
    });
    let driver = regen_driver_with_started(
        state.runtime.clone(),
        emit,
        on_started,
        post_publish,
        state.autosave_tick.clone(),
    );
    let (scheduler, handle) = RegenScheduler::new(driver);
    state
        .scheduler
        .set(handle)
        .map_err(|_| "application scheduler already started")?;
    state
        .tasks
        .track("regen-scheduler", tokio::spawn(scheduler.run()));
    start_autosave(state, host);
    Ok(())
}

fn start_library_initialization(state: &AppState, host: &HostContext) -> Result<(), &'static str> {
    let (completion, receiver) = tokio::sync::watch::channel(None);
    state
        .library_initialization
        .set(receiver)
        .map_err(|_| "library initialization already started")?;
    let root = library::library_root(host);
    state.tasks.spawn_terminal_blocking(move || {
        let result = root
            .ok_or_else(|| "library root is unavailable".to_string())
            .and_then(|root| library::seed_and_reindex_at(&root));
        completion.send_replace(Some(result));
    })
}

fn start_maintenance() {
    tokio::spawn(async {
        let _ = tokio::task::spawn_blocking(|| {
            imports::sweep_stale_workspaces(imports::STALE_WORKSPACE_AGE)
        })
        .await;
    });
}

fn start_autosave(state: &AppState, host: HostContext) {
    if let Some(app_data) = autosave::autosave_root(&host) {
        let lane = tokio::spawn(autosave::run_until(
            state.runtime.clone(),
            app_data,
            state.persistence.clone(),
            state.autosave_tick.subscribe(),
            (autosave::AUTOSAVE_DEBOUNCE, autosave::AUTOSAVE_MAX_AGE),
            crate::host_tasks::stopped(state.tasks.stop_signal()),
            move |event| {
                let _ = host.emit(AppEvent::Autosave(event));
            },
        ));
        state.tasks.track("autosave", lane);
    }
}

#[cfg(test)]
mod library_initialization_tests {
    use super::*;

    #[tokio::test]
    async fn await_handles_unstarted_pending_failure_and_closed_sender() {
        let state = AppState::default();
        assert!(state.await_library_initialization().await.is_ok());
        let (sender, receiver) = tokio::sync::watch::channel(None);
        state.library_initialization.set(receiver).unwrap();
        let waiter = state.await_library_initialization();
        tokio::pin!(waiter);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiter)
                .await
                .is_err()
        );
        sender.send_replace(Some(Err("seed failed".into())));
        assert_eq!(waiter.await.unwrap_err(), "seed failed");

        let closed = AppState::default();
        let (sender, receiver) = tokio::sync::watch::channel(None);
        closed.library_initialization.set(receiver).unwrap();
        drop(sender);
        assert!(closed.await_library_initialization().await.is_err());
    }
}
