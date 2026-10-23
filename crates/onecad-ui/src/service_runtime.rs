//! The one Tokio runtime and process services every document bridge shares.
//!
//! A worker's supervisor is spawned onto whichever runtime starts it, so a worker
//! pre-warmed under one bridge and adopted by another must not die with the first
//! bridge's runtime. Every bridge therefore runs on this single runtime, which lives
//! on its own thread until the last bridge holding it is dropped.
use onecad_app::process::ProcessServices;
use onecad_app::worker::WorkerBackend;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Bound on finishing in-flight blocking work when the runtime stops.
const RUNTIME_STOP_TIMEOUT: Duration = Duration::from_secs(2);
/// Bound on one bridge actor finishing its document close once the runtime stops.
const ACTOR_DRAIN_TIMEOUT: Duration = Duration::from_secs(15);

type Actors = Vec<JoinHandle<()>>;

pub struct ServiceRuntime {
    handle: Handle,
    services: Arc<ProcessServices>,
    budget: Arc<crate::budget::BudgetAuthority>,
    actors: Mutex<Actors>,
    stop: Mutex<Option<oneshot::Sender<Actors>>>,
    /// Disconnects when the runtime thread has exited (its sender is dropped).
    exit: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl ServiceRuntime {
    /// A runtime on the build's default backend (isolated tests).
    #[cfg(test)]
    pub fn start() -> Result<Arc<Self>, String> {
        Self::start_with(WorkerBackend::default())
    }

    /// A runtime whose workers all run on `backend` (the iPadOS host passes the
    /// in-process kernel with its crash journal).
    #[cfg(test)]
    pub fn start_with(backend: WorkerBackend) -> Result<Arc<Self>, String> {
        Self::start_with_profile(backend, onecad_render::RenderProfile::Desktop)
    }
    pub fn start_with_profile(
        backend: WorkerBackend,
        profile: onecad_render::RenderProfile,
    ) -> Result<Arc<Self>, String> {
        let (handle_tx, handle_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = oneshot::channel::<Actors>();
        let services = Arc::new(ProcessServices::with_backend(backend));
        let drained = services.clone();
        let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("onecad-native-services".into())
            .spawn(move || {
                let _exit = exit_tx;
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    // Shared by up to `MAX_OPEN_DOCUMENTS` hosts; CPU-heavy scene
                    // preparation runs on the blocking pool, not these threads.
                    .worker_threads(4)
                    // Service work is identified by thread name (repaint attribution).
                    .thread_name("onecad-native-services")
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = handle_tx.send(Err(format!("native service runtime: {error}")));
                        return;
                    }
                };
                let _ = handle_tx.send(Ok(runtime.handle().clone()));
                runtime.block_on(async {
                    // Every bridge's actor closes its document (recovery reconcile,
                    // worker reap) once its bridge is gone; let that finish first.
                    for actor in stop_rx.await.unwrap_or_default() {
                        let _ = tokio::time::timeout(ACTOR_DRAIN_TIMEOUT, actor).await;
                    }
                    drained.retire_warm();
                    // Let the polite Shutdown frame leave before the runtime stops.
                    tokio::task::yield_now().await;
                });
                runtime.shutdown_timeout(RUNTIME_STOP_TIMEOUT);
            })
            .map_err(|error| error.to_string())?;
        let handle = handle_rx
            .recv()
            .map_err(|_| "native service runtime exited during startup".to_string())??;
        Ok(Arc::new(Self {
            handle,
            services,
            budget: crate::budget::BudgetAuthority::new(profile),
            actors: Mutex::new(Vec::new()),
            stop: Mutex::new(Some(stop_tx)),
            exit: Mutex::new(Some(exit_rx)),
        }))
    }

    /// The runtime thread's exit, observed once the last holder drops the runtime
    /// (`recv` reports disconnection when the thread has finished). Taken once.
    pub fn take_exit(&self) -> Option<std::sync::mpsc::Receiver<()>> {
        self.exit.lock().ok().and_then(|mut exit| exit.take())
    }

    pub fn services(&self) -> &Arc<ProcessServices> {
        &self.services
    }

    pub(crate) fn budget(&self) -> &Arc<crate::budget::BudgetAuthority> {
        &self.budget
    }

    /// Spawns a bridge actor the runtime waits for before it stops.
    pub fn spawn_actor(&self, actor: impl std::future::Future<Output = ()> + Send + 'static) {
        let handle = self.handle.spawn(actor);
        if let Ok(mut actors) = self.actors.lock() {
            actors.retain(|actor| !actor.is_finished());
            actors.push(handle);
        }
    }
}

impl Drop for ServiceRuntime {
    fn drop(&mut self) {
        let actors = self
            .actors
            .lock()
            .map(|mut actors| std::mem::take(&mut *actors))
            .unwrap_or_default();
        if let Some(stop) = self.stop.lock().ok().and_then(|mut stop| stop.take()) {
            let _ = stop.send(actors);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_runtime_owns_one_budget_across_document_views() {
        let runtime = ServiceRuntime::start().unwrap();
        let first = runtime.budget().owner().unwrap();
        let second = runtime.budget().owner().unwrap();
        let lease = first.reserve(crate::budget::DISPLAY_RESERVATION).unwrap();
        assert_eq!(second.snapshot().used, crate::budget::DISPLAY_RESERVATION);
        assert_ne!(first.owner_id(), second.owner_id());
        drop(lease);
        assert_eq!(second.snapshot().used, 0);
    }
    #[test]
    fn tasks_spawned_on_the_runtime_run_until_it_is_dropped() {
        let runtime = ServiceRuntime::start().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        runtime.spawn_actor(async move {
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(runtime);
    }
}
