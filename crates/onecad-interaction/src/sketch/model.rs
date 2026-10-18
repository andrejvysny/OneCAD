use crate::DocumentContext;
use onecad_app::dto::{CancelSketchDto, FinishSketchDto, SketchSessionDto, SketchUpsertDto};
use onecad_core::{
    edit::{EditCommand, SketchEditOp},
    ids::{ConstraintId, EntityId, SketchId},
    math::Vec2,
    sketch::{Constraint, Sketch, SketchPlane},
};
use onecad_scene::CaptureIdentity;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SketchSessionKey {
    pub document_id: String,
    pub runtime_session: String,
    pub sketch_id: SketchId,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SketchTag {
    pub key: SketchSessionKey,
    pub sequence: u64,
    pub document: DocumentContext,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CapturedSketchPoint {
    pub key: SketchSessionKey,
    pub capture: CaptureIdentity,
    pub point: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchTool {
    Select,
    Line,
    Rectangle,
    Circle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchPhase {
    Creating,
    Entering,
    Active,
    NeedsRefresh,
    Submitting,
    Accepted,
    Finishing,
    Discarding,
    Closed,
    Faulted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishReason {
    Explicit,
    Escape,
    DiscardRefused,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DraftPrimitive {
    Line { start: Vec2, end: Vec2 },
    Rectangle { first: Vec2, opposite: Vec2 },
    Circle { center: Vec2, radius: f64 },
}

#[derive(Clone, Debug)]
pub struct SketchDraft {
    pub first: Vec2,
    pub anchor: Vec2,
    pub first_id: Option<EntityId>,
    pub anchor_id: Option<EntityId>,
    pub cursor: Vec2,
    pub segments: usize,
}

/// The host supplies fresh ids; moving a preview never allocates identities.
#[derive(Clone, Debug)]
pub struct PrimitiveIds {
    pub entities: [EntityId; 8],
    pub constraints: [ConstraintId; 4],
}

#[derive(Clone, Debug)]
pub struct SketchSnapshot {
    pub document: DocumentContext,
    pub sketch: Box<Sketch>,
}

#[derive(Clone, Debug)]
pub enum SketchStatus {
    Starting,
    Entered(Box<SketchSessionDto>),
    Pending,
    Accepted,
    Solved(Box<SketchUpsertDto>),
    Refused(String),
    Failed {
        accepted: Option<bool>,
        message: String,
    },
    Finished {
        result: Box<FinishSketchDto>,
        kept_reason: Option<String>,
    },
    Discarded(CancelSketchDto),
}

#[derive(Clone, Debug)]
pub struct SketchSession {
    pub key: SketchSessionKey,
    pub document: DocumentContext,
    pub plane: SketchPlane,
    pub authoritative: Sketch,
    pub tool: SketchTool,
    pub phase: SketchPhase,
    pub draft: Option<SketchDraft>,
    pub preview: Option<DraftPrimitive>,
    pub selection: BTreeSet<EntityId>,
    pub status: SketchStatus,
    pub request: SketchTag,
    pub(crate) batch: Option<PendingBatch>,
    pub(crate) created_here: bool,
    pub(crate) kept_reason: Option<String>,
    pub(crate) pending_exit: Option<ExitIntent>,
}

impl SketchSession {
    pub(crate) fn starting(sketch: Sketch, tag: SketchTag, create: bool) -> Self {
        Self {
            key: tag.key.clone(),
            document: tag.document.clone(),
            plane: sketch.effective_plane(),
            authoritative: sketch,
            tool: SketchTool::Select,
            phase: if create {
                SketchPhase::Creating
            } else {
                SketchPhase::Entering
            },
            draft: None,
            preview: None,
            selection: Default::default(),
            status: SketchStatus::Starting,
            request: tag,
            batch: None,
            created_here: create,
            kept_reason: None,
            pending_exit: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ExitIntent {
    Finish(FinishReason),
    Discard,
}

#[derive(Clone, Debug)]
pub(crate) enum AfterPrimitive {
    Reset,
    ContinueLine {
        first: EntityId,
        last: EntityId,
        segments: usize,
    },
    Preserve,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingBatch {
    pub tag: SketchTag,
    pub after: AfterPrimitive,
    pub deferred: Option<(SketchSnapshot, SketchUpsertDto)>,
    pub entities: Vec<EntityId>,
    pub constraints: Vec<ConstraintId>,
}

#[derive(Debug)]
pub enum SketchAction {
    ContextChanged(Option<DocumentContext>),
    Begin {
        sketch: Box<Sketch>,
        create: bool,
        context: DocumentContext,
    },
    Created {
        tag: SketchTag,
    },
    Entered {
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: Box<SketchSessionDto>,
    },
    Refreshed {
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: Box<SketchSessionDto>,
    },
    SetTool(SketchTool),
    Move(CapturedSketchPoint),
    Click {
        point: CapturedSketchPoint,
        ids: PrimitiveIds,
    },
    Escape,
    Enter,
    Finish,
    Discard,
    Select {
        key: SketchSessionKey,
        entities: Vec<EntityId>,
    },
    AddConstraints {
        key: SketchSessionKey,
        constraints: Vec<Constraint>,
    },
    BatchAccepted {
        tag: SketchTag,
    },
    Solved {
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: Box<SketchUpsertDto>,
    },
    Failed {
        tag: SketchTag,
        message: String,
    },
    Finished {
        tag: SketchTag,
        result: Box<FinishSketchDto>,
    },
    Discarded {
        tag: SketchTag,
        result: CancelSketchDto,
    },
}

#[derive(Debug)]
pub enum SketchEffect {
    CreateSketch {
        tag: SketchTag,
        command: Box<EditCommand>,
    },
    EnterSketch {
        tag: SketchTag,
        created_here: bool,
    },
    /// Read get_sketch and a fenced typed snapshot; return through Refreshed.
    RefreshSketch {
        tag: SketchTag,
    },
    Upsert {
        tag: SketchTag,
        ops: Vec<SketchEditOp>,
    },
    /// The host uses cancel_sketch(discard=false), then finish_sketch.
    FinishSketch {
        tag: SketchTag,
        reason: FinishReason,
    },
    DiscardSketch {
        tag: SketchTag,
    },
}

#[derive(Debug, Default)]
pub struct SketchInteraction {
    pub context: Option<DocumentContext>,
    pub session: Option<SketchSession>,
    pub(crate) sequence: u64,
}

impl SketchInteraction {
    pub fn text_undo_route(&self, text_focused: bool) -> crate::UndoRoute {
        crate::undo_route(
            text_focused,
            self.session
                .as_ref()
                .is_some_and(|session| session.phase != SketchPhase::Closed),
        )
    }
}
