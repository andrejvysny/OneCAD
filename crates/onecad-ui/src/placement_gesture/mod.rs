//! Native component placement: arm from the library, snap while hovering, place on click.
//!
//! One placement owns the editor at a time. It is a plain `PlaceComponent` operation in
//! the shared reducer (preview, Copy, Undo and history are the generic ones); this module
//! only decides what the pointer means (a snap), sends it as modeling options, and
//! keeps the gesture alive across a Shift+click repeat. Document switch, Home and any
//! other tool end it deterministically.
use crate::{
    app::App,
    diagnostics::{Source, State},
    library_catalog::{Classified, Placed, draft::ParamDraft},
};
use onecad_app::{dto::LibraryComponentDto, library::PlaceComponentRequest};
use onecad_core::{
    document::record::{ComponentParamValue as Value, KnownOperation, Operation},
    ids::{BodyId, RecordId},
};
use onecad_interaction::{
    Action, CommitStatus, FeatureOutcome, ModelSession, ModelingOptions, SessionPhase,
    modeling_records,
};
use snap::{IntentKey, Point, Sent, SnapIntent};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

/// A staging or classification request with no reply this long is abandoned.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

mod autosize;
pub(crate) mod card;
mod hover;
mod mate;
mod reconcile;
pub(crate) mod snap;
#[cfg(test)]
mod tests;

pub(crate) use hover::{keys, viewport};

/// Why the library's Insert stays off, or `None` when a placement can start.
pub(crate) fn gate(app: &App) -> Option<&'static str> {
    if !app.document_open() || app.doc.interaction.context.is_none() {
        return Some("Open a model first");
    }
    if app.sketch_active() {
        return Some("Finish the sketch first");
    }
    if !app.viewport_ready() || !app.doc.bridge.valid() {
        return Some("Geometry is unavailable");
    }
    if app.busy() || crate::operation_start::pending(app) || committing(app) {
        return Some("Wait for the current command to finish");
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The draft is being staged by the backend.
    Arming(Arm),
    /// The operation is open and follows the pointer.
    Armed { record: RecordId },
    /// Confirm was sent; the outcome decides between repeat and disarm.
    Committing { record: RecordId },
}

/// A staging request and what the editor looked like when it was sent: the reply
/// opens an operation only on an editor the user has not moved on in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Arm {
    pub sequence: u64,
    pub since: Instant,
    pub tool: Option<RecordId>,
    pub sketch: bool,
}

/// The one classification request on the wire.
#[derive(Clone, Debug)]
pub(crate) struct InFlight {
    pub sequence: u64,
    pub key: (BodyId, String),
    pub context: onecad_interaction::DocumentContext,
    pub since: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    pub body: BodyId,
    pub topology: String,
    /// The pick's world position.
    pub point: Point,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Knowledge {
    None,
    Pending,
    Known(Box<Classified>),
}

#[derive(Clone, Debug)]
pub(crate) struct HoverState {
    pub target: Option<Target>,
    pub class: Knowledge,
    pub in_flight: Option<InFlight>,
    /// The viewing ray at the pointer: origin and direction.
    pub ray: Option<(Point, Point)>,
}

/// The placement gesture's owner state.
pub(crate) struct ComponentPlacement {
    pub component: LibraryComponentDto,
    /// The editable free parameters; `params` is always their resolved full map.
    pub draft: ParamDraft,
    pub params: BTreeMap<String, Value>,
    /// Keys the user typed here; automatic sizing never overrides them.
    pub manual_params: BTreeSet<String>,
    pub copy: bool,
    /// The stored record being re-placed (Edit placement); `None` for a new placement.
    pub editing: Option<onecad_core::document::record::OperationRecord>,
    pub repeat_armed: bool,
    pub hover: HoverState,
    pub snap: SnapIntent,
    pub flipped: bool,
    pub attachment_cursor: usize,
    pub phase: Phase,
    pub notice: Option<String>,
    pub auto_note: Option<String>,
    pub(crate) sent: Option<Sent>,
    pub(crate) flip_key: Option<IntentKey>,
    /// Compatible attachments for the current target (Tab cycles them).
    pub(crate) compatible: usize,
}

impl ComponentPlacement {
    fn new(component: LibraryComponentDto, draft: ParamDraft, copy: bool) -> Option<Self> {
        let params = draft.resolve().ok()?;
        Some(Self {
            component,
            draft,
            params,
            manual_params: BTreeSet::new(),
            copy,
            editing: None,
            repeat_armed: false,
            hover: HoverState {
                target: None,
                class: Knowledge::None,
                in_flight: None,
                ray: None,
            },
            snap: SnapIntent::Ground { point: [0.0; 3] },
            flipped: false,
            attachment_cursor: 0,
            phase: Phase::Arming(Arm {
                sequence: 0,
                since: Instant::now(),
                tool: None,
                sketch: false,
            }),
            notice: None,
            auto_note: None,
            sent: None,
            flip_key: None,
            compatible: 0,
        })
    }
}

pub(crate) fn is_placement(tool: &ModelSession) -> bool {
    matches!(
        tool.record.op,
        Operation::Known(KnownOperation::PlaceComponent(_))
    )
}

/// A placement operation is open (in any phase).
pub(crate) fn placement_tool(app: &App) -> Option<&ModelSession> {
    app.doc
        .interaction
        .tool
        .as_ref()
        .filter(|t| is_placement(t))
}

fn committing(app: &App) -> bool {
    app.doc
        .interaction
        .tool
        .as_ref()
        .is_some_and(|t| t.phase != SessionPhase::Editing)
}

/// Whether `body` is produced by `record` (`body_<opId>` or a split child of it): a
/// placement being edited must never pick or mate to its own geometry.
pub(crate) fn own_body(body: BodyId, record: RecordId) -> bool {
    let parent = onecad_app::worker::wire::split_parts(body).map_or(body.0, |(op, _)| op);
    parent == record.0
}

/// The placement follows the pointer: its operation is open and editing.
pub(crate) fn armed(app: &App) -> bool {
    app.doc.placement.as_ref().is_some_and(|p| {
        matches!(p.phase, Phase::Armed { .. })
            && placement_tool(app).is_some_and(|t| t.phase == SessionPhase::Editing)
    })
}

/// The pill's Insert component: choose a component in the library, then place it.
pub(crate) fn open_library_page(app: &mut App) {
    if !crate::bars::home_allowed(app) {
        return;
    }
    cancel(app);
    crate::library_pages::open_library(&mut app.ui);
    app.ui.start_open = true;
    app.sync_active_budget();
}

/// Starts a placement of the detail page's component with its validated draft.
pub(crate) fn arm_from_detail(app: &mut App) {
    if let Some(reason) = gate(app) {
        app.doc.status = reason.into();
        return;
    }
    let catalog = &app.ui.library.catalog;
    let Some(view) = catalog.detail() else { return };
    let (Some(component), Some(draft)) = (view.component.clone(), view.draft.clone()) else {
        return;
    };
    let Some(placement) = ComponentPlacement::new(component, draft, false) else {
        app.doc.status = "Fix the invalid parameters first".into();
        return;
    };
    begin_arm(app, placement);
}

/// Cancels whatever tool is open through its normal Cancel, then asks the backend to
/// stage the draft. The editor is shown at once; the card appears with the reply.
fn begin_arm(app: &mut App, mut placement: ComponentPlacement) {
    if app.doc.interaction.tool.is_some() {
        crate::pill::cancel_tool(app);
        if app.doc.interaction.tool.is_some() {
            app.doc.status = "Finish the current operation first".into();
            return;
        }
    }
    let request = PlaceComponentRequest {
        component_id: placement.component.id.clone(),
        component_version: placement.component.version.clone(),
        translate: [0.0; 3],
        rotate: None,
        params: placement.params.clone(),
        mate: None,
        configuration: None,
    };
    let sequence = app.ui.library.catalog.request_prepare(request);
    placement.phase = Phase::Arming(Arm {
        sequence,
        since: Instant::now(),
        tool: app.doc.interaction.tool.as_ref().map(|t| t.tag.record_id),
        sketch: app.sketch_active(),
    });
    app.doc.placement = Some(placement);
    crate::library_catalog::flush(app);
    app.ui.start_open = false;
    app.doc.status = "Preparing component…".into();
    app.draw_dirty = true;
}

/// An arming placement owns the editor: no operation exists yet, so other starts must
/// find it and cancel it, never the reverse.
pub(crate) fn arming(app: &App) -> bool {
    app.doc
        .placement
        .as_ref()
        .is_some_and(|p| matches!(p.phase, Phase::Arming(_)))
}

/// Frame hook: applies backend replies and follows the operation's lifecycle.
pub(crate) fn poll(app: &mut App) {
    poll_at(app, Instant::now());
}

pub(crate) fn poll_at(app: &mut App, now: Instant) {
    let Some(mut placement) = app.doc.placement.take() else {
        return;
    };
    let keep = step(app, &mut placement, now);
    if keep {
        // A repeat has already installed its successor.
        if app.doc.placement.is_none() {
            app.doc.placement = Some(placement);
        }
        reconcile::reconcile(app);
        crate::library_catalog::flush(app);
    } else {
        app.ui.library.catalog.drop_placement();
    }
}

fn step(app: &mut App, p: &mut ComponentPlacement, now: Instant) -> bool {
    match p.phase.clone() {
        Phase::Arming(arm) => step_arming(app, p, &arm, now),
        Phase::Armed { record } => {
            if tool_is(app, record) {
                if committing(app) {
                    p.phase = Phase::Committing { record };
                }
                reconcile::take_classification(app, p, now);
                true
            } else {
                false
            }
        }
        Phase::Committing { record } => committed(app, p, record),
    }
}

fn tool_is(app: &App, record: RecordId) -> bool {
    placement_tool(app).is_some_and(|t| t.tag.record_id == record)
}

fn step_arming(app: &mut App, p: &mut ComponentPlacement, arm: &Arm, now: Instant) -> bool {
    let tool = app.doc.interaction.tool.as_ref().map(|t| t.tag.record_id);
    if tool != arm.tool || app.sketch_active() != arm.sketch {
        // The user's own work wins: it is left alone and the placement just ends.
        app.doc.status = "Placement dropped: another operation started first".into();
        return false;
    }
    let Some(reply) = app.ui.library.catalog.take_placement_reply(arm.sequence) else {
        if now.duration_since(arm.since) >= REQUEST_TIMEOUT {
            let message = "Preparing the component timed out; press Insert to retry";
            app.doc
                .diagnostics
                .record(Source::Model, State::Failed, message);
            app.doc.status = message.into();
            return false;
        }
        return true;
    };
    let result = match reply {
        Ok(Placed::Prepared(params)) => open(app, p, *params),
        Ok(Placed::Edit(component)) => open_edit(app, p, *component),
        Ok(_) => Err("unexpected library reply".to_owned()),
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => true,
        Err(error) => {
            let message = format!("Could not start placement: {error}");
            app.doc
                .diagnostics
                .record(Source::Model, State::Rejected, &message);
            app.doc.status = message;
            false
        }
    }
}

fn open(
    app: &mut App,
    p: &mut ComponentPlacement,
    params: onecad_core::document::record::PlaceComponentParams,
) -> Result<(), String> {
    let step = app
        .doc
        .display
        .as_ref()
        .map_or(0, |display| display.projection.applied_ops as u32);
    let name = p
        .draft
        .designation()
        .unwrap_or_else(|| p.component.name.clone());
    let record = modeling_records::component(RecordId::new(), step, &name, params);
    start_record(app, p, record, false)
}

/// Begins the operation for `record`: a new one, or the stored one being edited.
fn start_record(
    app: &mut App,
    p: &mut ComponentPlacement,
    record: onecad_core::document::record::OperationRecord,
    existing: bool,
) -> Result<(), String> {
    let context = app
        .doc
        .interaction
        .context
        .clone()
        .ok_or("the document is no longer open")?;
    if app.doc.interaction.tool.is_some() {
        return Err("another operation is running".into());
    }
    let id = record.record_id;
    app.doc.gesture += 1;
    let gesture = app.doc.gesture;
    app.dispatch(if existing {
        Action::BeginOperationEdit {
            record: Box::new(record),
            gesture,
            context,
        }
    } else {
        Action::BeginOperation {
            record: Box::new(record),
            fields: Vec::new(),
            focus: None,
            gesture,
            context,
        }
    });
    if !tool_is(app, id) {
        return Err(match &app.doc.interaction.status {
            CommitStatus::Rejected(reason) => reason.clone(),
            _ => "the operation could not start".into(),
        });
    }
    p.phase = Phase::Armed { record: id };
    if p.copy {
        send(app, ModelingOptions::ComponentCopy(true));
    }
    app.doc.status = "Hover a face, hole or rim; click to place, Esc to cancel".into();
    app.draw_dirty = true;
    Ok(())
}

/// Re-arms the gesture on an existing Linked instance: its stored placement stays the
/// ghost until the pointer picks a new target; Apply updates the same record.
pub(crate) fn edit_existing(app: &mut App, record_id: &str) {
    if let Some(reason) = gate(app) {
        app.doc.status = reason.into();
        return;
    }
    let found = app.doc.display.as_ref().and_then(|display| {
        display
            .records
            .iter()
            .find(|record| record.record_id.to_string() == record_id)
            .cloned()
    });
    let Some(record) = found else {
        app.doc.status = "Cannot edit placement: the instance record was not found".into();
        return;
    };
    let Operation::Known(KnownOperation::PlaceComponent(params)) = &record.op else {
        app.doc.status = "Only a linked placement can be edited".into();
        return;
    };
    let stub = stub_component(params);
    let Some(mut placement) =
        ComponentPlacement::new(stub, ParamDraft::new(Vec::new(), None), false)
    else {
        return;
    };
    let sequence = app.ui.library.catalog.request_prepare_edit(params.clone());
    placement.flipped = params.mate.as_ref().is_some_and(|m| m.flipped);
    placement.editing = Some(record);
    placement.phase = Phase::Arming(Arm {
        sequence,
        since: Instant::now(),
        tool: None,
        sketch: false,
    });
    app.doc.placement = Some(placement);
    crate::library_catalog::flush(app);
    app.doc.status = "Preparing the instance for editing…".into();
}

/// Identity-only stand-in until the definition arrives.
fn stub_component(
    params: &onecad_core::document::record::PlaceComponentParams,
) -> LibraryComponentDto {
    LibraryComponentDto {
        id: params.component_id.clone(),
        version: params.component_version.clone(),
        name: params.component_id.clone(),
        category: Vec::new(),
        tags: Vec::new(),
        source_kind: String::new(),
        revision: params.component_revision.clone(),
        generator_id: None,
        generator_version: None,
        attachments: BTreeMap::new(),
        parameters: BTreeMap::new(),
        designation: None,
        standard: None,
        unit: None,
        preview_path: None,
        configurations: Default::default(),
        default_configuration: None,
    }
}

/// The definition of the instance being edited has arrived: load its stored values.
fn open_edit(
    app: &mut App,
    p: &mut ComponentPlacement,
    component: LibraryComponentDto,
) -> Result<(), String> {
    let record = p.editing.clone().ok_or("no instance is being edited")?;
    let Operation::Known(KnownOperation::PlaceComponent(stored)) = &record.op else {
        return Err("the instance is no longer a placement".into());
    };
    p.draft = ParamDraft::from_component(&component);
    p.draft.load(&stored.params);
    p.params = p
        .draft
        .resolve()
        .map_err(|_| "the stored parameters no longer fit the definition")?;
    // Stored sizes are the user's: hovering a hole never re-sizes an existing instance.
    p.manual_params = stored.params.keys().cloned().collect();
    p.component = component;
    start_record(app, p, record, true)
}

/// Linked or Copy: session state of the open operation, kept for a repeat.
pub(crate) fn set_copy(app: &mut App, copy: bool) {
    if let Some(p) = app.doc.placement.as_mut() {
        p.copy = copy;
    }
    send(app, ModelingOptions::ComponentCopy(copy));
}

/// Sends modeling options to the open placement operation.
fn send(app: &mut App, options: ModelingOptions) {
    let Some(tag) = placement_tool(app).map(|t| t.tag.clone()) else {
        return;
    };
    app.dispatch(Action::ModelingOptionsChanged { tag, options });
}

fn committed(app: &mut App, p: &mut ComponentPlacement, record: RecordId) -> bool {
    if tool_is(app, record) {
        if !committing(app) {
            // Rejected: the operation is editable again.
            p.phase = Phase::Armed { record };
            p.repeat_armed = false;
        }
        return true;
    }
    let published = matches!(
        &app.doc.interaction.status,
        CommitStatus::Completed { tag, outcome: FeatureOutcome::Published, .. }
            if tag.record_id == record
    );
    if !(published && p.repeat_armed) {
        return false;
    }
    // Shift+click repeat: a fresh operation for the same component and parameters.
    let (Some(mut next), manual) = (
        ComponentPlacement::new(p.component.clone(), p.draft.clone(), p.copy),
        p.manual_params.clone(),
    ) else {
        return false;
    };
    next.manual_params = manual;
    begin_arm(app, next);
    app.doc.placement.is_some()
}

/// Ends the placement: the open operation is cancelled through its normal Cancel and
/// nothing stays armed. A commit already in flight finishes on its own.
pub(crate) fn cancel(app: &mut App) {
    if app.doc.placement.take().is_none() {
        return;
    }
    app.ui.library.catalog.drop_placement();
    if placement_tool(app).is_some_and(|t| t.phase == SessionPhase::Editing) {
        crate::pill::cancel_tool(app);
    }
}

/// Home may leave an editing placement: it is cancelled first.
pub(crate) fn cancellable(app: &App) -> bool {
    placement_tool(app).is_some_and(|t| t.phase == SessionPhase::Editing)
}

/// Confirm label for the feature card.
pub(crate) fn ok_label(tool: &ModelSession) -> Option<&'static str> {
    is_placement(tool).then_some("Place \u{21b5}")
}

/// The single commit entry for the card button, Enter and the click.
pub(crate) fn confirm(app: &mut App) {
    if placement_tool(app).is_some() {
        place(app, false);
    } else {
        app.dispatch(Action::Confirm);
    }
}

/// Sends Confirm for a click (Shift repeats) or Enter; false when it did not start.
pub(crate) fn place(app: &mut App, repeat: bool) -> bool {
    if !armed(app) {
        return false;
    }
    if let Some(p) = app.doc.placement.as_mut() {
        if matches!(p.hover.class, Knowledge::Pending) {
            app.doc.status = "Resolving the snap target…".into();
            return false;
        }
        p.repeat_armed = repeat && p.editing.is_none();
    }
    app.dispatch(Action::Confirm);
    let started = committing(app);
    if let (Some(p), true) = (app.doc.placement.as_mut(), started) {
        if let Phase::Armed { record } = p.phase {
            p.phase = Phase::Committing { record };
        }
    } else if let Some(p) = app.doc.placement.as_mut() {
        p.repeat_armed = false;
    }
    started
}
