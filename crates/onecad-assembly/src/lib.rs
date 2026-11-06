//! Pure assembly model for OneCAD (ADR 0020/0021/0022, ONECAD-340/342/341).
//!
//! Component definitions are regenerated once in their local frame; occurrences
//! place them with rigid transforms composed here, in Rust. Joints constrain
//! occurrence poses and are solved without a worker round trip.

pub mod error;
pub mod joint;
pub mod math;
pub mod occurrence;
pub mod solver;
