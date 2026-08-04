//! Edit layer: the [`EditCommand`] vocabulary, the single-writer
//! [`DocumentSession`], memento undo/redo, and command outcomes.

pub mod command;
pub mod outcome;
pub mod session;
pub mod undo;

pub use command::{EditCommand, InputPath, InputRef, SketchEditOp, VisibilityTarget};
pub use outcome::{CommandOutcome, ProjectionDelta, RegenHint};
pub use session::{gate_items_for_moved, DocumentSession};
pub use undo::{AppliedEdit, Inverse, Txn, UndoOutcome, UndoStack, UNDO_CAP};
