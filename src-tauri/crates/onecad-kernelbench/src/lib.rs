pub mod campaign;
pub mod case;
pub mod case_v2;
mod case_validation;
mod child;
pub mod cli;
pub mod digest;
mod metamorph;
pub mod prepared;
pub mod report;
pub mod result;
mod result_validation;
mod result_validation_blocks;
pub mod runner;
pub mod search;
pub mod suite;
pub mod suite_v2;

/// The FROZEN case format ([`case::Case`]). T0 and every committed regression are
/// pinned to it; [`case_v2`] is a separate type set read by its own version const.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub struct AppError {
    pub code: i32,
    pub message: String,
}

impl AppError {
    pub fn cli(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    pub fn environment(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: message.into(),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
