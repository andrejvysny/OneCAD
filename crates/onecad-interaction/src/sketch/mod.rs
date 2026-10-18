//! Native sketch interaction subset. Geometry drafts are local; only completed
//! batches become typed application effects. Solves remain application-owned.

mod drawing;
mod geometry;
mod lifecycle;
mod model;
mod primitives;
mod replies;

pub use geometry::*;
pub use model::*;
