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

/// Starts worker instances for one supervisor.
#[async_trait]
pub(crate) trait WorkerHost: Send + Sync + fmt::Debug + 'static {
    /// Starts one instance under `epoch` (log attribution only). Identity
    /// preflight (binary SHA-256, ABI) happens here, before anything runs.
    async fn launch(&self, epoch: u64) -> Result<Launched, String>;

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

/// A host that can never start anything — for supervisor bookkeeping tests.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct UnlaunchableHost;

#[cfg(test)]
#[async_trait]
impl WorkerHost for UnlaunchableHost {
    async fn launch(&self, _epoch: u64) -> Result<Launched, String> {
        Err("unlaunchable test host".into())
    }
}
