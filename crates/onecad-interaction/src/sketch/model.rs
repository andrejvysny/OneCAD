use super::SketchTarget;
use crate::DocumentContext;
use onecad_app::dto::{CancelSketchDto, FinishSketchDto, SketchSessionDto, SketchUpsertDto};
use onecad_core::{
    edit::{EditCommand, SketchEditOp},
    ids::{ConstraintId, EntityId, SketchId},
    math::Vec2,
    sketch::{Constraint, Sketch, SketchPlane},
};
use onecad_scene::CaptureIdentity;

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
    Point,
    Line,
    Rectangle,
    CenterRectangle,
    Circle,
    Arc,
    ThreePointArc,
    Slot,
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
    Point {
        at: Vec2,
    },
    Line {
        start: Vec2,
        end: Vec2,
    },
    Rectangle {
        first: Vec2,
        opposite: Vec2,
    },
    Circle {
        center: Vec2,
        radius: f64,
    },
    Arc {
        arc: super::arc_geometry::AnalyticArc,
    },
    Slot {
        center_start: Vec2,
        center_end: Vec2,
        radius: f64,
    },
}

#[derive(Clone, Debug)]
pub struct PrimitiveAnchor {
    pub point: Vec2,
    pub snap: Option<super::inference::AcceptedSnap>,
}

/// A chain curve whose `role` endpoint is the chain anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainCurve {
    pub entity: EntityId,
    pub role: onecad_core::sketch::CurvePosition,
}

/// Numeric commands must retain the active method, including a Line's arc mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericCreationMethod {
    Tool(SketchTool),
    TangentArc,
}

impl SketchSession {
    pub fn numeric_method(&self) -> NumericCreationMethod {
        if self.tool == SketchTool::Line && self.draft.as_ref().is_some_and(|d| d.tangent_arc) {
            NumericCreationMethod::TangentArc
        } else {
            NumericCreationMethod::Tool(self.tool)
        }
    }
}

#[derive(Clone, Debug)]
pub struct SketchDraft {
    pub first: Vec2,
    pub anchor: Vec2,
    pub first_id: Option<EntityId>,
    /// An existing curve endpoint starts a chain without inventing a point entity.
    pub first_target: Option<SketchTarget>,
    pub anchor_id: Option<EntityId>,
    pub cursor: Vec2,
    pub segments: usize,
    pub anchor_snap: Option<super::inference::AcceptedSnap>,
    pub secondary: Option<PrimitiveAnchor>,
    pub preceding: Option<ChainCurve>,
    pub tangent_arc: bool,
}

/// The host supplies fresh ids; moving a preview never allocates identities.
#[derive(Clone, Debug)]
pub struct PrimitiveIds {
    pub entities: [EntityId; 16],
    pub constraints: [ConstraintId; 32],
}

#[derive(Clone, Debug)]
pub struct SketchSnapshot {
    pub history: Option<onecad_app::dto::SketchHistoryAvailability>,
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
    pub edit_session: Option<String>,
    pub history: Option<onecad_app::dto::SketchHistoryAvailability>,
    pub tool: SketchTool,
    pub construction: bool,
    pub phase: SketchPhase,
    pub draft: Option<SketchDraft>,
    /// Advances only when a new primitive draft starts, independent of captures and requests.
    pub draft_generation: u64,
    pub preview: Option<DraftPrimitive>,
    pub selection: Vec<super::SketchTarget>,
    pub accepted_capture: Option<CaptureIdentity>,
    pub selection_dragging: bool,
    pub repair_reentry: bool,
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
            edit_session: None,
            history: None,
            tool: SketchTool::Select,
            construction: false,
            phase: if create {
                SketchPhase::Creating
            } else {
                SketchPhase::Entering
            },
            draft: None,
            draft_generation: 0,
            preview: None,
            selection: Default::default(),
            accepted_capture: None,
            selection_dragging: false,
            repair_reentry: false,
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
        first: Option<EntityId>,
        first_target: Option<SketchTarget>,
        last: Option<EntityId>,
        preceding: ChainCurve,
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
    BeginCreated {
        sketch: Box<Sketch>,
        context: DocumentContext,
    },
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
    SetConstructionMode {
        key: SketchSessionKey,
        construction: bool,
    },
    SetTangentArc {
        key: SketchSessionKey,
        enabled: bool,
    },
    Move(CapturedSketchPoint),
    Click {
        point: CapturedSketchPoint,
        ids: PrimitiveIds,
    },
    ClickInferred {
        point: CapturedSketchPoint,
        ids: PrimitiveIds,
        snap: Option<Box<super::inference::AcceptedSnap>>,
    },
    BeginPointNumeric {
        key: SketchSessionKey,
    },
    PreviewNumeric {
        key: SketchSessionKey,
        capture: CaptureIdentity,
        dimensions: Box<super::CreationDimensions>,
    },
    CreateNumeric {
        key: SketchSessionKey,
        capture: CaptureIdentity,
        ids: PrimitiveIds,
        dimensions: Box<super::CreationDimensions>,
    },
    CreateNumericAuthored {
        key: SketchSessionKey,
        capture: CaptureIdentity,
        draft_generation: u64,
        ids: PrimitiveIds,
        dimensions: Box<super::CreationDimensions>,
        authored: Vec<super::CreationField>,
    },
    Escape,
    Enter,
    Finish,
    Discard,
    AcceptView {
        key: SketchSessionKey,
        document: DocumentContext,
        capture: CaptureIdentity,
    },
    SelectionDrag {
        key: SketchSessionKey,
        document: DocumentContext,
        capture: CaptureIdentity,
        active: bool,
    },
    Select {
        key: SketchSessionKey,
        document: DocumentContext,
        capture: CaptureIdentity,
        targets: Vec<super::SketchTarget>,
        mode: super::SelectionMode,
    },
    AddConstraints {
        key: SketchSessionKey,
        constraints: Vec<Constraint>,
    },
    SetDimension {
        key: SketchSessionKey,
        constraint: ConstraintId,
        value: onecad_core::document::variables::Scalar,
    },
    Repair {
        key: SketchSessionKey,
        ops: Vec<SketchEditOp>,
    },
    History(onecad_core::edit::HistoryDirection),
    HistoryResolved {
        tag: SketchTag,
        snapshot: SketchSnapshot,
        solve: Box<SketchSessionDto>,
        moved: bool,
        direction: onecad_core::edit::HistoryDirection,
    },
    ConstraintRefused {
        tag: SketchTag,
        message: String,
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
    ReenterForRepair {
        key: SketchSessionKey,
        document: DocumentContext,
    },
    FinishRefused {
        tag: SketchTag,
        message: String,
        diagnostics: Vec<onecad_core::regen::Diagnostic>,
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
    History {
        tag: SketchTag,
        edit_session: String,
        direction: onecad_core::edit::HistoryDirection,
    },
    AddConstraints {
        tag: SketchTag,
        constraints: Vec<Constraint>,
    },
    SetDimension {
        tag: SketchTag,
        constraint: ConstraintId,
        value: onecad_core::document::variables::Scalar,
    },
    Repair {
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
