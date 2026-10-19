use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct GestureScope {
    pub key: SketchSessionKey,
    pub document: DocumentContext,
    pub edit_session: String,
    pub capture: CaptureIdentity,
}

impl GestureScope {
    pub fn from_session(session: &SketchSession) -> Result<Self, String> {
        if session.phase != SketchPhase::Active
            || session.tool != SketchTool::Select
            || session.draft.is_some()
            || session.selection_dragging
        {
            return Err(
                "Gesture requires an active Select tool without another draft or marquee".into(),
            );
        }
        let capture = session
            .accepted_capture
            .ok_or("Gesture requires an accepted native capture")?;
        let edit_session = session
            .edit_session
            .as_ref()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or("Gesture requires a current application edit session")?;
        if session.key.sketch_id != session.authoritative.id
            || session.key.document_id != session.document.document_id
            || session.key.runtime_session != session.document.runtime_session
            || session.plane != session.authoritative.effective_plane()
            || !super::super::validate_sketch_plane(session.plane)
        {
            return Err("Gesture session has inconsistent sketch identity or plane".into());
        }
        if capture.scene != session.document.scene {
            return Err("Gesture capture belongs to another document scene".into());
        }
        Ok(Self {
            key: session.key.clone(),
            document: session.document.clone(),
            edit_session: edit_session.clone(),
            capture,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GestureTag {
    pub scope: GestureScope,
    pub gesture: u64,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    Point,
    ArcEnd,
    Radius,
    EntityBody,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GestureIntent {
    pub target: SketchTarget,
    pub kind: GestureKind,
    pub entity: EntityId,
    pub role: Option<CurvePosition>,
    pub drag_point: EntityId,
    pub grab: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GesturePhase {
    PendingBegin,
    Ready,
    PendingUpdate,
    PendingEnd,
    Cancelling,
}

#[derive(Debug)]
pub enum GestureAction {
    Begin {
        scope: GestureScope,
        target: SketchTarget,
        grab: Vec2,
    },
    Move {
        tag: GestureTag,
        point: Vec2,
    },
    End {
        tag: GestureTag,
        point: Vec2,
    },
    Cancel {
        tag: GestureTag,
    },
    Ready {
        tag: GestureTag,
        worker_id: u64,
        preview: Box<Sketch>,
    },
    Updated {
        tag: GestureTag,
        worker_id: u64,
        worker_sequence: u64,
        preview: Box<Sketch>,
    },
    Completed {
        tag: GestureTag,
        worker_id: u64,
        moved: bool,
    },
    Cancelled {
        tag: GestureTag,
    },
    Failed {
        tag: GestureTag,
        message: String,
    },
    Sync,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GestureEffect {
    Begin {
        tag: GestureTag,
        intent: GestureIntent,
    },
    Update {
        tag: GestureTag,
        worker_id: u64,
        point: Vec2,
    },
    End {
        tag: GestureTag,
        worker_id: u64,
        point: Vec2,
    },
    Cancel {
        tag: GestureTag,
        worker_id: Option<u64>,
    },
}

pub(super) struct Gesture {
    pub tag: GestureTag,
    pub phase: GesturePhase,
    pub intent: GestureIntent,
    pub baseline: Sketch,
    pub preview: Option<Box<Sketch>>,
    pub worker_id: Option<u64>,
    pub worker_sequence: u64,
    pub latest: Option<Vec2>,
    pub end: Option<Vec2>,
}

#[derive(Default)]
pub struct SketchGestureInteraction {
    pub(super) gesture: Option<Gesture>,
    pub(super) serial: u64,
    pub(super) error: Option<String>,
    pub(super) moved: Option<bool>,
}

impl SketchGestureInteraction {
    pub fn active(&self) -> bool {
        self.gesture.as_ref().is_some_and(|gesture| {
            matches!(
                gesture.phase,
                GesturePhase::PendingBegin | GesturePhase::Ready | GesturePhase::PendingUpdate
            )
        })
    }
    pub fn busy(&self) -> bool {
        self.gesture.is_some()
    }
    pub fn tag(&self) -> Option<&GestureTag> {
        self.gesture.as_ref().map(|gesture| &gesture.tag)
    }
    pub fn intent(&self) -> Option<&GestureIntent> {
        self.gesture.as_ref().map(|gesture| &gesture.intent)
    }
    pub fn phase(&self) -> Option<GesturePhase> {
        self.gesture.as_ref().map(|gesture| gesture.phase)
    }
    pub fn preview(&self) -> Option<&Sketch> {
        self.gesture.as_ref()?.preview.as_deref()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn moved(&self) -> Option<bool> {
        self.moved
    }
}
