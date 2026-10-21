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

/// How long [`HostTasks::stop`] waits for one lane before abandoning it.
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
    /// Records a spawned lane so [`stop`](Self::stop) can await it.
    pub(crate) fn track(&self, name: &'static str, handle: JoinHandle<()>) {
        self.handles.lock().unwrap().push((name, handle));
    }

    /// A receiver that resolves once [`stop`](Self::stop) is called.
    pub(crate) fn stop_signal(&self) -> watch::Receiver<bool> {
        self.stop.subscribe()
    }

    /// Signals every lane to stop and awaits each one, bounded per lane.
    ///
    /// A lane that does not stop in time is aborted rather than waited on forever:
    /// a wedged lane must not hold the user's close hostage. Lanes only observe the
    /// signal between units of work, so an in-flight autosave write completes.
    pub(crate) async fn stop(&self) {
        self.stop_within(LANE_STOP_TIMEOUT).await;
    }

    async fn stop_within(&self, timeout: Duration) {
        self.stop.send_replace(true);
        let handles = std::mem::take(&mut *self.handles.lock().unwrap());
        for (name, mut handle) in handles {
            if tokio::time::timeout(timeout, &mut handle).await.is_err() {
                tracing::warn!(lane = name, "host lane did not stop in time; aborting");
                handle.abort();
            }
        }
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
    async fn a_lane_started_after_stop_sees_it_immediately() {
        let tasks = HostTasks::default();
        tasks.stop().await;
        tokio::time::timeout(Duration::from_secs(1), stopped(tasks.stop_signal()))
            .await
            .expect("late subscribers observe the latched stop");
    }
}
