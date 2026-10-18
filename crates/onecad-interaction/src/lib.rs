//! Pure interaction transitions. Hosts execute effects through onecad-app;
//! preview responses become visible only after their captured identity matches.

mod feedback;
mod model;
mod numeric_parameter;
mod reducer;
mod session;
mod transform;

pub mod sketch;

pub use model::*;
pub use numeric_parameter::{
    numeric_parameters, NumericParameter, NumericParameterDescriptor, ParameterAxis,
};
