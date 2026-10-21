//! Native sketch interaction subset. Geometry drafts are local; only completed
//! batches become typed application effects. Solves remain application-owned.

mod center_rectangle;
mod drawing;
mod geometry;
pub mod inference;
mod inferred_creation;
mod lifecycle;
mod model;
mod mutations;
mod primitives;
mod replies;

pub use geometry::*;
pub use model::*;

mod selection;
pub use selection::{SelectionMode, SketchTarget, MAX_SELECTION_TARGETS};

mod numeric_creation;
pub use numeric_creation::{CreationDimensions, CreationField};

pub mod gesture;

pub mod arc_geometry;

mod slot_creation;
mod staged_creation;
mod staged_inference;
mod staged_numeric;
mod tangent_continuation;
