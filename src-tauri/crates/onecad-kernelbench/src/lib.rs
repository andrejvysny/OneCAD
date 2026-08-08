pub mod campaign;
pub mod case;
mod case_validation;
mod child;
pub mod cli;
pub mod digest;
mod metamorph;
pub mod report;
pub mod result;
mod result_validation;
mod result_validation_blocks;
pub mod runner;
pub mod search;
pub mod suite;

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
