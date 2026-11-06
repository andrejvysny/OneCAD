//! Assembly model errors.

use crate::occurrence::OccurrenceId;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AssemblyError {
    #[error("unknown occurrence {0}")]
    UnknownOccurrence(OccurrenceId),
    #[error("duplicate occurrence {0}")]
    DuplicateOccurrence(OccurrenceId),
    #[error("occurrence parent chain has a cycle at {0}")]
    Cycle(OccurrenceId),
    #[error("occurrence nesting exceeds depth limit {limit}")]
    DepthExceeded { limit: usize },
    #[error("occurrence count exceeds instance limit {limit}")]
    InstanceLimit { limit: usize },
    #[error("occurrence {0} has a non-finite transform")]
    NonFiniteTransform(OccurrenceId),
    #[error("occurrence {0} still has children")]
    HasChildren(OccurrenceId),
}
