//! Explicit worker backend selection (docs/ios/ARCHITECTURE.md "Worker
//! backends"): which host every supervised worker of a process starts on.
//!
//! Production selection is explicit — a [`WorkerBackend`] on
//! [`ProcessServices`](crate::process::ProcessServices) — never inferred from
//! the environment. The default is the sidecar when that feature is on (macOS)
//! and the in-process kernel otherwise (iOS); the iOS host passes
//! [`EmbeddedKernelConfig::for_app_support`] so the crash journal is on. A
//! build with neither feature (a backend-agnostic crate compiled on its own)
//! gets [`WorkerBackend::Unavailable`], which never starts a kernel.

#[cfg(feature = "embedded-kernel")]
use std::path::{Path, PathBuf};

#[cfg(feature = "sidecar")]
use super::resolve_worker_path;
use super::{SupervisorConfig, WorkerManager};

#[cfg(feature = "embedded-kernel")]
pub use super::inprocess::{
    embedded_identity_json, in_process_counters, in_process_relaunch_required,
    linked_identity_json, InProcessCounters,
};

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
    /// No backend feature is on (a backend-agnostic consumer, such as
    /// `onecad-interaction`, built on its own): no kernel ever starts — the
    /// [`PendingBackend`](super::PendingBackend) lane. Never a production
    /// selection; the binaries enable a backend.
    #[cfg(not(any(feature = "sidecar", feature = "embedded-kernel")))]
    Unavailable,
}

// Not derivable in every build: without `sidecar` the default carries a config.
#[allow(clippy::derivable_impls)]
impl Default for WorkerBackend {
    fn default() -> Self {
        #[cfg(feature = "sidecar")]
        {
            Self::Sidecar
        }
        #[cfg(all(not(feature = "sidecar"), feature = "embedded-kernel"))]
        {
            Self::Embedded(EmbeddedKernelConfig::default())
        }
        #[cfg(not(any(feature = "sidecar", feature = "embedded-kernel")))]
        {
            Self::Unavailable
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
    #[cfg_attr(
        not(any(feature = "sidecar", feature = "embedded-kernel")),
        allow(dead_code)
    )]
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
            #[cfg(not(any(feature = "sidecar", feature = "embedded-kernel")))]
            Self::Unavailable => {
                let _ = role;
                None
            }
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
            #[cfg(not(any(feature = "sidecar", feature = "embedded-kernel")))]
            Self::Unavailable => WorkerManager::spawn_with_host(
                config,
                std::sync::Arc::new(super::host::NoKernelHost),
            ),
        }
    }
}

/// The in-process kernel's process-level configuration.
#[cfg(feature = "embedded-kernel")]
#[derive(Debug, Clone)]
pub struct EmbeddedKernelConfig {
    /// `<app_support>/kernel-journal` (docs/ios/ARCHITECTURE.md). `None`: no
    /// crash journal and no persisted poison (tests and tools only: an iOS
    /// release refuses to launch without it).
    pub journal_dir: Option<PathBuf>,
    /// A test override of the identity manifest (v2) the kernel must match,
    /// honoured in debug builds only; `None` (production) verifies the one
    /// embedded at build time, which release builds require.
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
    /// The production configuration: the crash journal at
    /// `<app_support>/kernel-journal` (docs/ios/ARCHITECTURE.md).
    #[must_use]
    pub fn for_app_support(app_support: &Path) -> Self {
        Self {
            journal_dir: Some(app_support.join(JOURNAL_DIR)),
            ..Self::default()
        }
    }

    fn spawn(&self, config: SupervisorConfig, role: WorkerRole) -> WorkerManager {
        let retire_deadline = config
            .wedge_deadline_execute_plan
            .max(config.wedge_deadline_readers)
            .max(config.wedge_deadline_default);
        let host = super::inprocess::InProcessHost::new(self.clone(), role, retire_deadline);
        WorkerManager::spawn_with_host(config, std::sync::Arc::new(host))
    }
}

/// The crash journal's directory under the app-support directory.
#[cfg(feature = "embedded-kernel")]
const JOURNAL_DIR: &str = "kernel-journal";

#[cfg(all(test, feature = "embedded-kernel"))]
mod tests {
    use super::*;

    #[test]
    fn the_production_config_journals_under_app_support() {
        let config =
            EmbeddedKernelConfig::for_app_support(Path::new("/c/Library/Application Support"));
        assert_eq!(
            config.journal_dir.as_deref(),
            Some(Path::new("/c/Library/Application Support/kernel-journal"))
        );
        assert!(
            config.identity.is_none(),
            "production verifies the embedded identity"
        );
    }
}
