//! The background lanes one document host owns, so closing that host stops them.
//!
//! With one host per process the lanes simply died with the runtime. Several hosts
//! share one runtime, so a closed host must stop its own scheduler and autosave loop
//! explicitly — otherwise they would outlive it, holding its runtime slot, host
//! context and caches until the process exits.

use std::sync::Mutex as StdMutex;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;

/// How long [`HostTasks::begin_stop`]'s drain waits before abandoning one lane.
const LANE_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// Join handles plus a stop signal for the lanes a host spawned.
pub(crate) struct HostTasks {
    handles: StdMutex<Vec<(&'static str, JoinHandle<()>)>>,
    stop: watch::Sender<bool>,
    /// Set while the host is being torn down: no new document may be installed.
    pub(crate) closing: std::sync::atomic::AtomicBool,
}

impl Default for HostTasks {
    fn default() -> Self {
        Self {
            handles: StdMutex::new(Vec::new()),
            stop: watch::channel(false).0,
            closing: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl HostTasks {
    /// Records a spawned lane so [`begin_stop`](Self::begin_stop) can drain it.
    pub(crate) fn track(&self, name: &'static str, handle: JoinHandle<()>) {
        self.handles.lock().unwrap().push((name, handle));
    }

    /// A receiver that resolves once [`begin_stop`](Self::begin_stop) is called.
    pub(crate) fn stop_signal(&self) -> watch::Receiver<bool> {
        self.stop.subscribe()
    }

    /// Signals stop and spawns an owned drain, bounded per lane.
    ///
    /// A lane that does not stop in time is aborted rather than waited on forever:
    /// a wedged lane must not hold the user's close hostage. Lanes only observe the
    /// signal between units of work, so an in-flight autosave write completes.
    pub(crate) fn begin_stop(&self) -> JoinHandle<()> {
        self.stop_with_timeout(LANE_STOP_TIMEOUT)
    }

    /// Stop admission synchronously before a shutdown waiter can be cancelled.
    pub(crate) fn request_stop(&self) {
        self.stop.send_replace(true);
    }

    fn stop_with_timeout(&self, timeout: Duration) -> JoinHandle<()> {
        self.request_stop();
        let handles = std::mem::take(&mut *self.handles.lock().unwrap());
        tokio::spawn(async move {
            for (name, mut handle) in handles {
                if tokio::time::timeout(timeout, &mut handle).await.is_err() {
                    tracing::warn!(lane = name, "host lane did not stop in time; aborting");
                    handle.abort();
                }
            }
        })
    }

    #[cfg(test)]
    pub(crate) async fn stop(&self) {
        self.begin_stop().await.unwrap();
    }

    #[cfg(test)]
    async fn stop_within(&self, timeout: Duration) {
        self.stop_with_timeout(timeout).await.unwrap();
    }

    /// Number of lanes still tracked (tests / introspection).
    #[cfg(test)]
    pub(crate) fn tracked(&self) -> usize {
        self.handles.lock().unwrap().len()
    }
}

/// Resolves once `stop` flips to `true` (or its sender is gone).
pub(crate) async fn stopped(mut stop: watch::Receiver<bool>) {
    while !*stop.borrow_and_update() {
        if stop.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_signals_and_awaits_every_tracked_lane() {
        let tasks = HostTasks::default();
        let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel();
        for _ in 0..3 {
            let stop = tasks.stop_signal();
            let done = done_tx.clone();
            tasks.track(
                "lane",
                tokio::spawn(async move {
                    stopped(stop).await;
                    let _ = done.send(());
                }),
            );
        }
        assert_eq!(tasks.tracked(), 3);
        tasks.stop().await;
        assert_eq!(tasks.tracked(), 0, "stopped lanes are no longer tracked");
        for _ in 0..3 {
            done_rx.try_recv().expect("every lane observed the stop");
        }
    }

    #[tokio::test]
    async fn a_lane_ignoring_the_signal_is_aborted_after_the_timeout() {
        let tasks = HostTasks::default();
        let stuck = tokio::spawn(std::future::pending::<()>());
        let abort = stuck.abort_handle();
        tasks.track("stuck", stuck);
        tasks.stop_within(Duration::from_millis(20)).await;
        tokio::task::yield_now().await;
        assert!(abort.is_finished(), "a wedged lane must not block close");
    }

    #[tokio::test]
    async fn dropping_shutdown_waiter_still_aborts_a_wedged_lane() {
        let tasks = HostTasks::default();
        let stuck = tokio::spawn(std::future::pending::<()>());
        let abort = stuck.abort_handle();
        tasks.track("stuck", stuck);
        drop(tasks.stop_with_timeout(Duration::from_millis(20)));
        assert_eq!(tasks.tracked(), 0, "drain owns every admitted handle");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !abort.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached drain must still abort the lane");
    }

    #[tokio::test]
    async fn a_lane_started_after_stop_sees_it_immediately() {
        let tasks = HostTasks::default();
        tasks.stop().await;
        tokio::time::timeout(Duration::from_secs(1), stopped(tasks.stop_signal()))
            .await
            .expect("late subscribers observe the latched stop");
    }
}
