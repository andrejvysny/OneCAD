//! Explicit worker backend selection (docs/ios/ARCHITECTURE.md "Worker
//! backends"): which host every supervised worker of a process starts on.
//!
//! Production selection is explicit — a [`WorkerBackend`] on
//! [`ProcessServices`](crate::process::ProcessServices) — never inferred from
//! the environment. The default is the sidecar when that feature is on (macOS)
//! and the in-process kernel otherwise (iOS).

#[cfg(feature = "embedded-kernel")]
use std::path::PathBuf;

#[cfg(feature = "sidecar")]
use super::resolve_worker_path;
use super::{SupervisorConfig, WorkerManager};

#[cfg(feature = "embedded-kernel")]
pub use super::inprocess::{in_process_counters, InProcessCounters};

/// Where this process's workers run.
#[derive(Debug, Clone)]
pub enum WorkerBackend {
    /// The `onecad-worker` child process, resolved by
    /// [`resolve_worker_path`](super::resolve_worker_path) (macOS).
    #[cfg(feature = "sidecar")]
    Sidecar,
    /// The in-process OCCT kernel (iPadOS; macOS tests).
    #[cfg(feature = "embedded-kernel")]
    Embedded(EmbeddedKernelConfig),
}

// Not derivable in every build: without `sidecar` the default carries a config.
#[allow(clippy::derivable_impls)]
impl Default for WorkerBackend {
    fn default() -> Self {
        #[cfg(feature = "sidecar")]
        {
            Self::Sidecar
        }
        #[cfg(not(feature = "sidecar"))]
        {
            Self::Embedded(EmbeddedKernelConfig::default())
        }
    }
}

/// What a supervised worker is for. Picks its supervision policy and, in
/// process, whether it counts against the auxiliary instance cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerRole {
    /// An open document's worker, or the pre-warmed one waiting to become it.
    Document,
    /// A Component Library re-bake's scratch worker.
    Rebake,
    /// A Component Library ingest's worker (batch ping budget).
    Ingest,
}

impl WorkerRole {
    /// The supervision policy for this role (unchanged from the sidecar-only
    /// call sites: production, or the ingest batch budget).
    fn config(self, binary: std::path::PathBuf) -> SupervisorConfig {
        match self {
            Self::Document | Self::Rebake => SupervisorConfig::production(binary),
            Self::Ingest => crate::library_ingest::ingest_supervisor_config(binary),
        }
    }
}

impl WorkerBackend {
    /// Spawns a supervised worker for `role` under its production policy, or
    /// `None` when this backend has none to give (no sidecar binary resolves) —
    /// the caller's `PendingBackend` lane.
    #[must_use]
    pub fn spawn(&self, role: WorkerRole) -> Option<WorkerManager> {
        match self {
            #[cfg(feature = "sidecar")]
            Self::Sidecar => {
                resolve_worker_path().map(|path| WorkerManager::spawn(role.config(path)))
            }
            #[cfg(feature = "embedded-kernel")]
            Self::Embedded(config) => Some(config.spawn(role.config(PathBuf::new()), role)),
        }
    }

    /// Spawns a supervised worker under an explicit policy (tests and tools).
    /// The sidecar uses `config.binary`; the in-process kernel ignores it.
    #[must_use]
    pub fn spawn_with(&self, config: SupervisorConfig, role: WorkerRole) -> WorkerManager {
        let _ = role;
        match self {
            #[cfg(feature = "sidecar")]
            Self::Sidecar => WorkerManager::spawn(config),
            #[cfg(feature = "embedded-kernel")]
            Self::Embedded(embedded) => embedded.spawn(config, role),
        }
    }
}

/// The in-process kernel's process-level configuration.
#[cfg(feature = "embedded-kernel")]
#[derive(Debug, Clone)]
pub struct EmbeddedKernelConfig {
    /// `<app_support>/kernel-journal` (docs/ios/ARCHITECTURE.md). `None`: no
    /// crash journal and no persisted poison (tests, tools, ephemeral lanes).
    pub journal_dir: Option<PathBuf>,
    /// The identity manifest (v2) the kernel must match; `None` uses the one
    /// embedded at build time. Release builds require one.
    pub identity: Option<String>,
    /// Worker log level forwarded to `tracing` target `worker`.
    pub log_level: onecad_embedded_worker::LogLevel,
    /// Bounded wait for the process-wide OCCT locks, ms; 0 = the 20 s default.
    pub occt_lock_wait_ms: u32,
}

#[cfg(feature = "embedded-kernel")]
impl Default for EmbeddedKernelConfig {
    fn default() -> Self {
        Self {
            journal_dir: None,
            identity: None,
            log_level: onecad_embedded_worker::LogLevel::Info,
            occt_lock_wait_ms: 0,
        }
    }
}

#[cfg(feature = "embedded-kernel")]
impl EmbeddedKernelConfig {
    fn spawn(&self, config: SupervisorConfig, role: WorkerRole) -> WorkerManager {
        let host = super::inprocess::InProcessHost::new(self.clone(), role);
        WorkerManager::spawn_with_host(config, std::sync::Arc::new(host))
    }
}
