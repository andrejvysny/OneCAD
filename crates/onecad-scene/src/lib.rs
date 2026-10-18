//! Immutable CPU scene and synchronous picking authority, independent of UI/GPU.
//!
//! Math uses f64; this does not restore precision already lost in MESH1 f32 data.
//! Deterministic BVHs are built with immutable scenes; topology IDs remain
//! snapshot-scoped and the brute-force picker remains a validation oracle.

#![cfg_attr(
    not(test),
    deny(clippy::panic, clippy::unwrap_used, clippy::expect_used)
)]

mod camera;
mod fit;
mod index;
mod picking;
mod scene;

pub use camera::*;
pub use fit::*;
pub use glam::{DMat4, DVec2, DVec3};
pub use onecad_core::ids::BodyId;
pub use onecad_protocol::mesh::{MeshLimits, MeshView};
pub use picking::*;
pub use scene::*;

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("invalid camera or viewport: {0}")]
    InvalidView(&'static str),
    #[error("invalid scene mesh: {0}")]
    InvalidMesh(#[from] onecad_protocol::mesh::MeshDecodeError),
    #[error("CPU scene budget exceeded: {0}")]
    BudgetExceeded(&'static str),
    #[error("captured view does not match scene or requested capture")]
    CaptureMismatch,
    #[error("duplicate body identity in scene")]
    DuplicateBody,
}
