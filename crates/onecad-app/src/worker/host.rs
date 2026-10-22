//! The [`WorkerHost`] seam: how a worker instance is started, how its end is
//! observed and how it is stopped — the ONLY things the macOS sidecar and the
//! iPadOS in-process kernel do differently (docs/ios/ARCHITECTURE.md "Worker
//! backends"). Handshake, `verify_hello`, solver policy, fencing, poison and
//! plan gates stay in the shared supervisor (`manager.rs`).

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use super::identity::HelloAxes;

/// The worker's OCW1 byte stream, as the shared handshake consumes it.
pub(crate) type Reader = Box<dyn AsyncRead + Unpin + Send>;
pub(crate) type Writer = Box<dyn AsyncWrite + Unpin + Send>;

/// One started worker instance, before the OCW1 handshake.
pub(crate) struct Launched {
    pub(crate) reader: Reader,
    pub(crate) writer: Writer,
    pub(crate) instance: Box<dyn WorkerInstance>,
    /// The release identity the hello must match (`None`: debug without one).
    pub(crate) expected: Option<HelloAxes>,
}

/// What a forced stop achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(any(feature = "sidecar", feature = "embedded-kernel")),
    allow(dead_code)
)]
pub(crate) enum KillOutcome {
    /// The worker is gone (a reaped child, or a run thread that returned).
    Reaped,
    /// An in-process run that did not return: its thread is left behind for the
    /// rest of the process (no safe way to stop a thread inside OCCT).
    #[cfg_attr(not(feature = "embedded-kernel"), allow(dead_code))]
    Abandoned,
}

/// A persisted crash-circuit entry to seed on connect (in-process journal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PoisonSeed {
    /// `prefixHash|opId|fingerprint` — the `plan_op_keys` format.
    pub(crate) key: String,
    pub(crate) strikes: u32,
    /// Whether the persisted strikes reached the host's threshold.
    pub(crate) open: bool,
    pub(crate) message: String,
}

/// Held for one plan dispatch (dispatch to terminal); dropping it lets the
/// next ordered dispatch go.
pub(crate) type PlanPermit = Box<dyn Send>;

/// Starts worker instances for one supervisor.
#[async_trait]
pub(crate) trait WorkerHost: Send + Sync + fmt::Debug + 'static {
    /// Starts one instance under `epoch` (log attribution only). Identity
    /// preflight (binary SHA-256, ABI) happens here, before anything runs.
    /// Cancel-safe: dropping the future before it resolves starts nothing.
    async fn launch(&self, epoch: u64) -> Result<Launched, String>;

    /// `Some(reason)` once this process may start no further instance on this
    /// host (an in-process run was abandoned: docs/ios/ARCHITECTURE.md "In-process
    /// failure policy"). Terminal for every supervisor that observes it.
    /// Default: never (a sidecar can always be killed and replaced).
    fn relaunch_required(&self) -> Option<String> {
        None
    }

    /// Orders one plan dispatch with `keys` (its crash-circuit candidates)
    /// against the other instances' dispatches; held until the plan's
    /// terminal. Default: no ordering.
    async fn plan_permit(&self, keys: &[String]) -> Option<PlanPermit> {
        let _ = keys;
        None
    }

    /// Persisted circuit entries to seed after a handshake under `fingerprint`.
    /// Default: none (the sidecar's circuit is in memory only).
    fn poison_seeds(&self, fingerprint: &str) -> Vec<PoisonSeed> {
        let _ = fingerprint;
        Vec::new()
    }

    /// A prepared plan healed these keys (`record_success`).
    fn poison_healed(&self, keys: &[String]) {
        let _ = keys;
    }

    /// The user cleared every circuit (`clear_circuit`).
    fn poison_cleared(&self) {}

    /// Crash-journal writes that failed so far (surfaced in worker status).
    fn journal_write_failures(&self) -> u64 {
        0
    }
}

/// One running worker instance, owned by the supervisor's run loop.
#[async_trait]
pub(crate) trait WorkerInstance: Send {
    /// Resolves when the worker ended on its own. Cancel-safe.
    async fn exited(&mut self);

    /// Retire path (a graceful `Shutdown` was already sent): give it `grace`
    /// to end, then stop it as [`force_kill`](Self::force_kill) does.
    async fn terminate(&mut self, grace: Duration);

    /// Ping-timeout / wedge path.
    async fn force_kill(&mut self) -> KillOutcome;
}

/// A host that can never start anything: a build with no backend feature (a
/// backend-agnostic consumer compiled on its own), and supervisor bookkeeping
/// tests.
#[cfg(any(test, not(any(feature = "sidecar", feature = "embedded-kernel"))))]
#[derive(Debug)]
pub(crate) struct NoKernelHost;

#[cfg(any(test, not(any(feature = "sidecar", feature = "embedded-kernel"))))]
#[async_trait]
impl WorkerHost for NoKernelHost {
    async fn launch(&self, _epoch: u64) -> Result<Launched, String> {
        Err(
            "no worker backend is compiled into this build (features `sidecar`, `embedded-kernel`)"
                .into(),
        )
    }
}
