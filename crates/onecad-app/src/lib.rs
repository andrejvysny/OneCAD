//! Host-independent application services over OneCAD's single-writer runtime.

// Worker backends (docs/ios/ARCHITECTURE.md): iOS cannot spawn the sidecar and
// has no other kernel; every build needs at least one backend.
#[cfg(all(target_os = "ios", feature = "sidecar"))]
compile_error!(
    "onecad-app: the `sidecar` backend cannot target iOS (no child processes); build with \
     --no-default-features --features embedded-kernel"
);
#[cfg(all(target_os = "ios", not(feature = "embedded-kernel")))]
compile_error!("onecad-app: iOS requires the `embedded-kernel` feature");
#[cfg(not(any(feature = "sidecar", feature = "embedded-kernel")))]
compile_error!("onecad-app: enable a worker backend (`sidecar` and/or `embedded-kernel`)");

pub mod api;
pub mod autosave;
pub mod document_runtime;
pub mod dto;
pub mod error;
pub mod events;
pub mod export;
pub mod export_threemf;
pub mod host;
mod host_claims;
mod host_tasks;
pub mod imports;
pub mod library;
pub mod library_ingest;
pub mod library_seed;
pub mod library_seed_templates;
pub mod lifecycle;
pub mod logging;
pub mod mesh_cache;
pub mod process;
pub mod read;
pub mod recents;
pub mod regen_driver;
pub mod reservations;
pub mod sketch_candidate;
pub mod sketch_projection;
pub mod state;
pub mod worker;

pub use regen_driver::*;
