use crate::NumericParameter;
use onecad_core::{
    document::{record::OperationRecord, refs::SketchRegionRef, variables::Scalar},
    edit::EditCommand,
    ids::{RecordId, SketchId, SnapshotId},
};
use onecad_scene::{CaptureIdentity, SceneIdentity};

pub use onecad_app::dto::{EvaluatedExpressionDto, PreviewResultDto, RegenFinished};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentContext {
    pub document_id: String,
    pub runtime_session: String,
    pub revision: u64,
    pub snapshot_id: SnapshotId,
    pub scene: SceneIdentity,
}

impl DocumentContext {
    pub fn same_runtime(&self, other: &Self) -> bool {
        self.document_id == other.document_id && self.runtime_session == other.runtime_session
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileSelection {
    pub profile: SketchRegionRef,
    pub context: DocumentContext,
    pub capture: CaptureIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestTag {
    pub context: DocumentContext,
    pub record_id: RecordId,
    pub gesture: u64,
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DraftStatus {
    Pending,
    Valid(Scalar),
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewStatus {
    Empty,
    /// Existing-feature edits need rollback-prefix preview support in onecad-app.
    UnavailableForEdit,
    Pending,
    Ready,
    NeedsRepair,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Editing,
    Committing,
    Accepted,
    Completed,
}

#[derive(Clone, Debug)]
pub struct ModelSession {
    pub tag: RequestTag,
    pub record: OperationRecord,
    pub editing_existing: bool,
    pub parameter: NumericParameter,
    pub draft: String,
    pub draft_status: DraftStatus,
    pub preview: PreviewStatus,
    pub phase: SessionPhase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureOutcome {
    Published,
    Noop,
    NeedsRepair,
    Failed(String),
    Cancelled,
}

#[derive(Clone, Debug)]
pub enum CommitStatus {
    Idle,
    Pending(RequestTag),
    Rejected(String),
    Accepted {
        tag: RequestTag,
        revision: u64,
        rollback_token: Option<String>,
    },
    /// Authoring succeeded, but no authoritative regeneration result arrived.
    CompletionUnavailable {
        tag: RequestTag,
        accepted_revision: u64,
        rollback_token: Option<String>,
        message: String,
    },
    Completed {
        tag: RequestTag,
        accepted_revision: u64,
        rollback_token: Option<String>,
        outcome: FeatureOutcome,
        completion: Box<RegenFinished>,
    },
}

#[derive(Debug)]
pub enum Action {
    ContextChanged(Option<DocumentContext>),
    SelectProfile(Box<ProfileSelection>),
    /// Deselect authoring inputs without changing a tool or the document.
    ClearSelection,
    BeginExtrude {
        record_id: RecordId,
        gesture: u64,
        step_index: u32,
    },
    /// The host supplies a fresh record identity and complete promoted inputs.
    BeginOperation {
        record: Box<OperationRecord>,
        parameter: NumericParameter,
        gesture: u64,
        context: DocumentContext,
    },
    BeginEdit {
        record: Box<OperationRecord>,
        gesture: u64,
        context: DocumentContext,
    },
    BeginParameterEdit {
        record: Box<OperationRecord>,
        parameter: NumericParameter,
        gesture: u64,
        context: DocumentContext,
    },
    DraftChanged(String),
    ExtrudeModeChanged(onecad_core::document::record::BooleanMode),
    /// Host admits only visible, healthy bodies from this accepted document context.
    ExtrudeTargetChanged {
        body: Option<onecad_core::ids::BodyId>,
        context: DocumentContext,
    },
    FlipExtrude,
    TransformDraftChanged {
        translate: [f64; 3],
        axis: [f64; 3],
        angle_degrees: f64,
    },
    Evaluated {
        tag: RequestTag,
        result: Result<EvaluatedExpressionDto, String>,
    },
    PreviewFinished {
        tag: RequestTag,
        result: Result<PreviewResultDto, String>,
    },
    Confirm,
    Cancel,
    CommitAccepted {
        tag: RequestTag,
        revision: u64,
        rollback_token: Option<String>,
    },
    CommitRejected {
        tag: RequestTag,
        message: String,
    },
    CompletionUnavailable {
        tag: RequestTag,
        message: String,
    },
    RegenFinished {
        tag: RequestTag,
        completion: Box<RegenFinished>,
    },
}

#[derive(Debug)]
pub enum Effect {
    EvaluateExpression {
        tag: RequestTag,
        expression: String,
        site: &'static str,
    },
    Preview {
        tag: RequestTag,
        operation: Box<onecad_core::document::record::Operation>,
        sketch_id: Option<SketchId>,
        expected_snapshot: SnapshotId,
    },
    PublishPreview {
        tag: RequestTag,
        result: Box<PreviewResultDto>,
    },
    ClearPreview {
        gesture: u64,
    },
    ApplyOperation {
        tag: RequestTag,
        command: Box<EditCommand>,
        hide_sketch: Option<SketchId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoRoute {
    Text,
    Sketch,
    Model,
}

pub fn undo_route(text_focused: bool, sketch_session: bool) -> UndoRoute {
    if text_focused {
        UndoRoute::Text
    } else if sketch_session {
        UndoRoute::Sketch
    } else {
        UndoRoute::Model
    }
}

#[derive(Debug)]
pub struct Interaction {
    pub context: Option<DocumentContext>,
    pub selection: Option<ProfileSelection>,
    pub tool: Option<ModelSession>,
    pub status: CommitStatus,
    pub(crate) sequence: u64,
    pub(crate) deferred_completion: Option<(RequestTag, Box<RegenFinished>)>,
}

impl Default for Interaction {
    fn default() -> Self {
        Self {
            context: None,
            selection: None,
            tool: None,
            status: CommitStatus::Idle,
            sequence: 0,
            deferred_completion: None,
        }
    }
}
