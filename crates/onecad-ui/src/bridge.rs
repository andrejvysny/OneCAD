//! Bounded admission precedes application scheduling; durable results retain their permit.
pub(crate) use crate::event_mailbox::{BackendEvent, EventMailbox};
use crate::{profiles::Profile, qa::QaPaths, service_runtime::ServiceRuntime, services};
use onecad_app::{
    dto::DocumentProjection,
    host::{AppEvent, DialogPort, EventSink, HostContext, HostPaths},
    process::ProcessServices,
    state::AppState,
};
use onecad_core::{document::record::OperationRecord, edit::EditCommand, ids::RecordId};
use onecad_interaction::{Action, RequestTag};
use onecad_scene::SceneSnapshot;
use std::sync::{Arc, Mutex, mpsc};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc as async_mpsc};

const ADMISSION: usize = 8;

pub struct Display {
    pub epoch: u64,
    pub projection: DocumentProjection,
    pub context: onecad_app::api::PreviewContext,
    pub scene: Arc<SceneSnapshot>,
    pub profiles: Vec<Profile>,
    pub profile_anchors: Vec<crate::profile_capture::Anchor>,
    pub profile_failures: Vec<crate::profile_capture::Failure>,
    pub prepared: Mutex<Option<onecad_render::PreparedScene>>,
    pub records: Vec<OperationRecord>,
    pub render_receipt: Option<crate::render_receipt::RenderReceipt>,
    pub _lease: Arc<crate::budget::Lease>,
}

#[derive(Debug)]
pub enum Command {
    #[cfg(test)]
    InspectSketch(Box<crate::gesture_services::testing::InspectionRequest>),
    #[cfg(test)]
    InspectBody(Box<crate::body_inspection_tests_support::Request>),
    Open,
    Document(crate::document_flow::Request),
    Items(crate::document_items::Request),
    Recovery {
        document_id: String,
        choice: crate::startup_recovery::Choice,
    },
    Sketch(Box<crate::sketch_services::Request>),
    /// Non-dirtying operand staging; FIFO with `Apply` on this lane.
    Operands(Box<crate::operand_services::Request>),
    Repair {
        tag: RequestTag,
        repair: Box<onecad_app::api::InputRepair>,
    },
    Apply {
        tag: RequestTag,
        command: Box<EditCommand>,
        hide_sketch: Option<String>,
    },
    Undo,
    Redo,
    /// Re-prepares an already-captured scene for upload, keeping its identity, so
    /// a document switched back into the viewport is drawn without a new capture
    /// (a new identity would reset the document's active tool and selection).
    Prepare(crate::viewport_updates::Source),
    /// The steps since the last save, for the close prompt.
    DescribeChanges,
}

#[derive(Debug)]
pub enum Candidate {
    Evaluate {
        tag: RequestTag,
        expression: String,
        site: &'static str,
    },
    Preview {
        tag: RequestTag,
        operation: Box<onecad_core::document::record::Operation>,
        sketch_id: Option<String>,
        snapshot: onecad_core::ids::SnapshotId,
    },
    PreviewInputRepair {
        tag: RequestTag,
        repair: Box<onecad_app::api::InputRepair>,
    },
    PreviewExistingEdit {
        tag: RequestTag,
        operation: Box<onecad_core::document::record::Operation>,
    },
}

pub struct CandidateDelivery {
    pub action: Action,
    pub prepared: Option<onecad_render::PreparedScene>,
    pub scene: Option<Arc<SceneSnapshot>>,
    pub lease: Option<Arc<crate::budget::Lease>>,
}

pub enum Message {
    Items {
        sequence: u64,
        result: Result<crate::document_items::Reply, String>,
        _permit: Option<OwnedSemaphorePermit>,
    },
    Gesture {
        delivery: Box<crate::gesture_services::Delivery>,
        _permit: Option<OwnedSemaphorePermit>,
    },
    Document {
        result: Result<crate::document_flow::Outcome, String>,
        _permit: OwnedSemaphorePermit,
    },
    Sketch {
        delivery: Box<crate::sketch_services::Delivery>,
        _permit: OwnedSemaphorePermit,
    },
    StartupFailed(String),
    Stopped,
    Changes {
        runtime_session: String,
        changes: onecad_app::document_runtime::UnsavedChanges,
        _permit: OwnedSemaphorePermit,
    },
    Prepared {
        identity: onecad_scene::SceneIdentity,
        result: Result<onecad_render::PreparedScene, String>,
        _permit: OwnedSemaphorePermit,
    },
    Accepted {
        tag: RequestTag,
        revision: u64,
        rollback_token: Option<String>,
    },
    Terminal {
        command: &'static str,
        tag: Option<RequestTag>,
        result: Result<crate::outcome::CommandOutcome, String>,
        _permit: OwnedSemaphorePermit,
    },
}

pub struct Shared {
    pub(crate) input_trace: Arc<crate::input_trace::Trace>,
    /// The started document host, for acceptance probes; cleared when the actor
    /// ends so the host context's sink (which holds this `Shared`) is released.
    #[cfg(feature = "semantic-probe")]
    pub(crate) probe: Mutex<Option<crate::host::DocumentAccess>>,
    #[cfg(test)]
    pub fail_next_gesture_capture: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub gesture_end_arrived: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub gesture_cancel_observed: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub gesture_end_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    pub gesture_events: Arc<Mutex<Option<Arc<EventMailbox>>>>,
    pub display: Mutex<Option<Arc<Display>>>,
    pub document_state: Mutex<crate::document_state::Slot>,
    pub discard_on_shutdown: Mutex<Option<String>>,
    pub recents: Mutex<Vec<onecad_app::dto::RecentProjectDto>>,
    pub recovery: Mutex<crate::startup_recovery::State>,
    pub candidate: Mutex<Option<(u64, Candidate)>>,
    pub candidate_result: Mutex<Option<(u64, CandidateDelivery)>>,
    pub display_epoch: std::sync::atomic::AtomicU64,
    pub failure: Mutex<Option<String>>,
    pub activity: Mutex<Option<String>>,
    pub(crate) autosave_status: Mutex<crate::autosave_status::Slot>,
    pub render_receipt: Mutex<Option<crate::render_receipt::RenderReceipt>>,
    pub valid: std::sync::atomic::AtomicBool,
    pub stopping: std::sync::atomic::AtomicBool,
    pub shutdown: Notify,
    pub recovery_flush: Arc<Notify>,
    pub(crate) recovery_requests: Arc<Mutex<Vec<crate::recovery_flush::Ticket>>>,
    pub budget: Mutex<onecad_render::PreparationBudget>,
    /// The last preparation's geometry, reused by the next one. Shared so a window can
    /// count and drop a parked document's cache (`display_budget`); a preparation moves
    /// it out and back, never holding the lock while it works.
    pub preparation_cache: Mutex<onecad_render::PreparationCache>,
    pub host_budget: Arc<crate::budget::HostBudget>,
    /// The document host's process identity, set once its actor has started.
    pub host: std::sync::atomic::AtomicU64,
    generation: std::sync::atomic::AtomicU64,
    notify: Notify,
    pub(crate) context: egui::Context,
}

impl Shared {
    #[cfg(test)]
    pub(crate) fn new(context: egui::Context, budget: onecad_render::PreparationBudget) -> Self {
        Self::with_host_budget(
            context,
            budget,
            Arc::new(crate::budget::HostBudget::default()),
        )
    }
    fn with_host_budget(
        context: egui::Context,
        budget: onecad_render::PreparationBudget,
        host_budget: Arc<crate::budget::HostBudget>,
    ) -> Self {
        Self {
            input_trace: crate::input_trace::Trace::from_environment(&context),
            #[cfg(feature = "semantic-probe")]
            probe: Mutex::new(None),
            #[cfg(test)]
            fail_next_gesture_capture: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            gesture_end_arrived: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            gesture_cancel_observed: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            gesture_end_gate: Mutex::new(None),
            gesture_events: Arc::new(Mutex::new(None)),
            display: Mutex::new(None),
            document_state: Mutex::new(Default::default()),
            discard_on_shutdown: Mutex::new(None),
            recents: Mutex::new(Vec::new()),
            recovery: Mutex::new(Default::default()),
            candidate: Mutex::new(None),
            candidate_result: Mutex::new(None),
            display_epoch: std::sync::atomic::AtomicU64::new(0),
            failure: Mutex::new(None),
            activity: Mutex::new(None),
            autosave_status: Mutex::new(Default::default()),
            render_receipt: Mutex::new(None),
            budget: Mutex::new(budget),
            preparation_cache: Mutex::new(Default::default()),
            host_budget,
            host: std::sync::atomic::AtomicU64::new(0),
            valid: std::sync::atomic::AtomicBool::new(false),
            stopping: std::sync::atomic::AtomicBool::new(false),
            shutdown: Notify::new(),
            recovery_flush: Arc::new(Notify::new()),
            recovery_requests: Arc::new(Mutex::new(Vec::new())),
            generation: std::sync::atomic::AtomicU64::new(0),
            notify: Notify::new(),
            context,
        }
    }
    pub fn preparation_budget(&self) -> Result<onecad_render::PreparationBudget, String> {
        self.budget
            .lock()
            .map(|budget| *budget)
            .map_err(|_| "renderer preparation budget poisoned".into())
    }
    pub(crate) fn take_preparation_cache(&self) -> onecad_render::PreparationCache {
        self.preparation_cache
            .lock()
            .map(|mut cache| std::mem::take(&mut *cache))
            .unwrap_or_default()
    }
    pub(crate) fn restore_preparation_cache(&self, cache: onecad_render::PreparationCache) {
        if let Ok(mut slot) = self.preparation_cache.lock() {
            *slot = cache;
        }
    }
    pub fn wake(&self) {
        self.input_trace
            .record("service.wake", serde_json::Value::Null);
        self.context.request_repaint();
    }
    pub fn release_text_focus(&self, id: egui::Id) {
        self.context.memory_mut(|memory| memory.surrender_focus(id));
    }
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(std::sync::atomic::Ordering::Acquire) == generation
    }
    pub fn publish_display(&self, display: Display) {
        self.input_trace.capture("service.capture", &display);
        let epoch = display.epoch;
        crate::handoff::with_current(
            &self.display,
            &self.display_epoch,
            &self.stopping,
            epoch,
            |slot| {
                *slot = Some(Arc::new(display));
                self.valid.store(true, std::sync::atomic::Ordering::Release);
                if let Ok(mut failure) = self.failure.lock() {
                    *failure = None;
                }
            },
        );
        self.wake();
    }
    pub fn invalidate(&self, message: Option<String>) {
        crate::handoff::invalidate(&self.display, &self.display_epoch, &self.valid, || {
            if let Some(message) = message
                && let Ok(mut failure) = self.failure.lock()
            {
                *failure = Some(message);
            }
        });
        self.host_budget.wake();
        self.wake();
    }
    pub fn accept_display(&self, display: &Display) -> bool {
        crate::handoff::with_current(
            &self.display,
            &self.display_epoch,
            &self.stopping,
            display.epoch,
            |_| {
                self.valid.store(true, std::sync::atomic::Ordering::Release);
            },
        )
    }
    pub fn publish_candidate(&self, generation: u64, result: CandidateDelivery) {
        if self.is_current(generation) {
            if let Ok(mut slot) = self.candidate_result.lock() {
                *slot = Some((generation, result));
            }
            self.wake();
        }
    }
}

struct Envelope {
    command: Command,
    permit: OwnedSemaphorePermit,
}

pub struct Bridge {
    gestures: async_mpsc::Sender<crate::gesture_services::Request>,
    gesture_permit: Arc<Semaphore>,
    commands: async_mpsc::Sender<Envelope>,
    permits: Arc<Semaphore>,
    pub shared: Arc<Shared>,
    pub messages: mpsc::Receiver<Message>,
    // Last field: the runtime must outlive the channels above when the bridge drops.
    _runtime: Arc<ServiceRuntime>,
}

impl Bridge {
    /// A bridge on its own private service runtime (isolated tests).
    #[cfg(all(test, feature = "desktop"))]
    pub fn start(
        paths: QaPaths,
        context: egui::Context,
        budget: onecad_render::PreparationBudget,
    ) -> Result<Self, String> {
        let dialogs = Arc::new(crate::dialogs::NativeDialogs::new(paths.documents.clone()));
        Self::start_on(ServiceRuntime::start()?, paths, dialogs, context, budget)
    }

    /// A bridge whose document host shares `runtime` and its process services with
    /// every other bridge started on it.
    pub fn start_on(
        runtime: Arc<ServiceRuntime>,
        paths: QaPaths,
        dialogs: Arc<dyn DialogPort>,
        context: egui::Context,
        budget: onecad_render::PreparationBudget,
    ) -> Result<Self, String> {
        let (commands, receiver) = async_mpsc::channel(ADMISSION);
        let (gestures, gesture_receiver) = async_mpsc::channel(crate::gesture_services::CAPACITY);
        let (sender, messages) = mpsc::channel();
        let shared = Arc::new(Shared::with_host_budget(
            context,
            budget,
            runtime.budget().owner()?,
        ));
        runtime.spawn_actor(actor(
            paths,
            dialogs,
            runtime.services().clone(),
            shared.clone(),
            receiver,
            gesture_receiver,
            sender,
        ));
        Ok(Self {
            gestures,
            gesture_permit: Arc::new(Semaphore::new(1)),
            commands,
            permits: Arc::new(Semaphore::new(ADMISSION)),
            shared,
            messages,
            _runtime: runtime,
        })
    }

    pub fn submit(&self, command: Command) -> Result<(), String> {
        if self
            .shared
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("Native host is shutting down".into());
        }
        admit(&self.commands, &self.permits, command)
    }

    pub fn submit_gesture(
        &self,
        effect: onecad_interaction::sketch::gesture::GestureEffect,
        expected: onecad_app::api::PreviewContext,
    ) -> Result<(), String> {
        if self
            .shared
            .stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("Native host is shutting down".into());
        }
        let permit = if matches!(
            effect,
            onecad_interaction::sketch::gesture::GestureEffect::Begin { .. }
        ) {
            Some(
                self.gesture_permit
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| "Another native gesture owns admission".to_string())?,
            )
        } else {
            None
        };
        self.gestures
            .try_send(crate::gesture_services::Request {
                effect,
                expected,
                permit,
            })
            .map_err(|_| "Native gesture input queue is full or closed".into())
    }

    pub fn candidate(&self, candidate: Candidate) {
        let generation = self
            .shared
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        if let Ok(mut slot) = self.shared.candidate.lock() {
            *slot = Some((generation, candidate));
        }
        self.shared.host_budget.wake();
        self.shared.notify.notify_one();
    }

    pub fn request_shutdown(&self) {
        self.shared
            .stopping
            .store(true, std::sync::atomic::Ordering::Release);
        self.shared.host_budget.close();
        self.shared.shutdown.notify_one();
    }
    pub fn request_discard_shutdown(&self, runtime_session: &str) {
        if let Ok(mut intent) = self.shared.discard_on_shutdown.lock() {
            *intent = Some(runtime_session.to_owned());
        }
        self.request_shutdown();
    }
    pub fn valid(&self) -> bool {
        self.shared.valid.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn render_submitted(&self, queue: &wgpu::Queue, display: &Display) {
        if self.valid()
            && let Some(receipt) = &display.render_receipt
        {
            receipt.submitted(queue);
        }
    }

    pub fn cancel_candidates(&self) {
        self.shared
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if let Ok(mut slot) = self.shared.candidate.lock() {
            *slot = None;
        }
        if let Ok(mut slot) = self.shared.candidate_result.lock() {
            *slot = None;
        }
        self.shared.host_budget.wake();
    }

    #[cfg(test)]
    pub(crate) fn emit_autosave_for_test(&self, event: onecad_app::autosave::AutosaveEvent) {
        let sink = Sink {
            events: Arc::new(EventMailbox::default()),
            shared: self.shared.clone(),
            failed: std::sync::atomic::AtomicBool::new(false),
        };
        sink.emit(AppEvent::Autosave(event)).unwrap();
    }

    pub fn take_display(&self) -> Option<Arc<Display>> {
        self.shared.display.lock().ok()?.take()
    }
    pub fn take_candidate(&self) -> Option<CandidateDelivery> {
        crate::handoff::take_current(
            &self.shared.candidate_result,
            &self.shared.generation,
            &self.shared.valid,
            &self.shared.stopping,
        )
    }
}

struct Sink {
    events: Arc<EventMailbox>,
    shared: Arc<Shared>,
    failed: std::sync::atomic::AtomicBool,
}
impl EventSink for Sink {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        let summary = match event {
            AppEvent::RegenFinished(result) => {
                crate::gesture_services::observe(&self.shared, BackendEvent::Regen(result.clone()));
                self.events
                    .emit(BackendEvent::Regen(result))
                    .map_err(|e| e.to_string())?;
                "Geometry updated".to_owned()
            }
            AppEvent::WorkerStatus(status) => {
                if status.state == "restarting" {
                    self.shared.invalidate(None);
                }
                if status.state == "failed" && !self.failed.swap(true, Ordering::AcqRel) {
                    crate::gesture_services::observe(&self.shared, BackendEvent::WorkerFailed);
                    self.events
                        .emit(BackendEvent::WorkerFailed)
                        .map_err(|e| e.to_string())?;
                } else if status.state == "ready" {
                    self.failed.store(false, Ordering::Release);
                }
                format!("Geometry worker: {}", status.state)
            }
            AppEvent::ProjectionUpdated(projection) => {
                if let Ok(mut slot) = self.shared.document_state.lock() {
                    slot.observe(&projection);
                }
                self.events
                    .observe_projection(&projection.runtime_session, projection.revision);
                self.shared.invalidate(None);
                "Document updating".to_owned()
            }
            AppEvent::RegenStarted(_) => "Regenerating".to_owned(),
            AppEvent::DocumentChanged(event) => {
                if let Ok(mut receipt) = self.shared.render_receipt.lock() {
                    *receipt = crate::render_receipt::RenderReceipt::from_event(event);
                }
                "Geometry changed".to_owned()
            }
            AppEvent::Autosave(event) => {
                if let Ok(mut slot) = self.shared.autosave_status.lock() {
                    slot.record(event);
                }
                "Recovery checkpoint status changed".to_owned()
            }
            AppEvent::NeedsRepair(_) => "Feature needs repair".to_owned(),
            AppEvent::SketchSolved(_) => "Sketch solved".to_owned(),
        };
        if let Ok(mut activity) = self.shared.activity.lock() {
            *activity = Some(summary);
        }
        self.shared.wake();
        Ok(())
    }
}
pub(crate) struct Backend {
    pub state: Arc<AppState>,
    pub host: HostContext,
    pub paths: QaPaths,
    pub events: Arc<EventMailbox>,
    pub shared: Arc<Shared>,
    pub startup_opened: bool,
    pub scene_revision: u64,
    pub previous_scene: Option<(Arc<SceneSnapshot>, Arc<crate::budget::Lease>)>,
}

async fn actor(
    paths: QaPaths,
    dialogs: Arc<dyn DialogPort>,
    services: Arc<ProcessServices>,
    shared: Arc<Shared>,
    mut receiver: async_mpsc::Receiver<Envelope>,
    mut gesture_receiver: async_mpsc::Receiver<crate::gesture_services::Request>,
    sender: mpsc::Sender<Message>,
) {
    let events = Arc::new(EventMailbox::default());
    let event_rx = events.clone();
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(paths.data.clone()),
            app_config: Some(paths.config.clone()),
        },
        events: Arc::new(Sink {
            events,
            shared: shared.clone(),
            failed: std::sync::atomic::AtomicBool::new(false),
        }),
        dialogs,
    };
    let state = Arc::new(AppState::with_services(services));
    shared
        .host
        .store(state.host_id(), std::sync::atomic::Ordering::Release);
    if let Err(error) = state.start(host.clone()) {
        let _ = sender.send(Message::StartupFailed(error.into()));
        shared.wake();
        return;
    }
    #[cfg(feature = "semantic-probe")]
    if let Ok(mut probe) = shared.probe.lock() {
        *probe = Some(crate::host::DocumentAccess {
            state: state.clone(),
            host: host.clone(),
            documents: paths.documents.clone(),
            runtime: tokio::runtime::Handle::current(),
        });
    }
    let (recovery_stop, recovery_task) =
        crate::recovery_flush::spawn(state.clone(), host.clone(), shared.clone());
    let mut backend = Backend {
        state,
        host,
        paths,
        events: event_rx,
        shared: shared.clone(),
        startup_opened: false,
        scene_revision: 0,
        previous_scene: None,
    };
    let mut gestures = crate::gesture_services::Service::new();
    loop {
        let gesture_ending = gestures.ending();
        tokio::select! {
            _=shared.shutdown.notified()=>{break;}
            request = gesture_receiver.recv() => {
                if let Some(request) = request { gestures.submit(&backend, request, &sender).await; shared.wake(); }
            }
            result = gestures.next() => {
                if let Some(result) = result {
                    gestures.receive(&mut backend, result, &sender).await;
                    shared.input_trace.record("service.gesture_finished", serde_json::Value::Null);
                    shared.wake();
                }
            }
            envelope = receiver.recv() => {
                let Some(envelope) = envelope else { break; };
                let name = command_name(&envelope.command);
                shared.input_trace.record("service.command_started", serde_json::json!({"command": name}));
                services::execute(&mut backend,envelope.command,envelope.permit,&sender).await;
                shared.input_trace.record("service.command_finished", serde_json::json!({"command": name}));
                if shared.stopping.load(std::sync::atomic::Ordering::Acquire) { break; }
            }
            event = backend.events.next(), if !gesture_ending => {
                match event {
                    BackendEvent::Regen(_)=>match services::load_display(&mut backend).await {
                        Ok(display)=>shared.publish_display(display),
                        Err(error)=>services::invalidate(&shared,format!("Scene refresh failed: {error}")),
                    },
                    BackendEvent::WorkerFailed if services::current_worker_failed(&backend.state)=>services::invalidate(&shared,"Geometry worker failed".into()),
                    BackendEvent::WorkerFailed=>{},
                }
            }
            _ = shared.notify.notified() => {
                let candidate = shared.candidate.lock().ok().and_then(|mut slot|slot.take());
                if let Some((generation,candidate)) = candidate {
                    // Keep one worker preview until completion; cancellation invalidates its delivery.
                    tokio::select! {
                        _=services::candidate(&backend,generation,candidate)=>{},
                        _=shared.shutdown.notified()=>{backend.state.retire_host_workers();break;}
                    }
                }
            }
        }
    }
    shared
        .stopping
        .store(true, std::sync::atomic::Ordering::Release);
    shared.host_budget.close();
    let _ = recovery_stop.send(());
    let _ = recovery_task.await;
    gestures.shutdown().await;
    gesture_receiver.close();
    receiver.close();
    while let Ok(envelope) = receiver.try_recv() {
        let name = command_name(&envelope.command);
        let tag = match envelope.command {
            Command::Apply { tag, .. } | Command::Repair { tag, .. } => Some(tag),
            _ => None,
        };
        let _ = sender.send(Message::Terminal {
            command: name,
            tag,
            result: Err("Command cancelled before execution during shutdown".into()),
            _permit: envelope.permit,
        });
    }
    let discard = shared
        .discard_on_shutdown
        .lock()
        .ok()
        .and_then(|mut intent| intent.take());
    let closed =
        onecad_app::api::shutdown_host(&backend.state, backend.host.clone(), discard.as_deref())
            .await;
    if let Err(error) = closed {
        eprintln!("Document close failed; recovery retained: {error}");
    }
    #[cfg(feature = "semantic-probe")]
    if let Ok(mut probe) = shared.probe.lock() {
        *probe = None;
    }
    let _ = sender.send(Message::Stopped);
    shared.wake();
}

fn admit(
    commands: &async_mpsc::Sender<Envelope>,
    permits: &Arc<Semaphore>,
    command: Command,
) -> Result<(), String> {
    let permit = permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| "Command queue full; wait for current operation".to_string())?;
    commands
        .try_send(Envelope { command, permit })
        .map_err(|error| error.to_string())
}

pub fn command_name(command: &Command) -> &'static str {
    match command {
        #[cfg(test)]
        Command::InspectBody(_) => "InspectBody",
        #[cfg(test)]
        Command::InspectSketch(_) => "InspectSketch",
        Command::Open => "Open",
        Command::Document(_) => "Document",
        Command::Items(_) => "Delete",
        Command::Recovery { .. } => "Recovery",
        Command::Sketch(_) => "Sketch",
        Command::Operands(_) => "Operands",
        Command::Apply { .. } => "Apply",
        Command::Repair { .. } => "Repair",
        Command::Undo => "Undo",
        Command::Redo => "Redo",
        Command::Prepare(_) => "Prepare",
        Command::DescribeChanges => "DescribeChanges",
    }
}

pub fn record_id(raw: &str) -> Result<RecordId, String> {
    raw.parse()
        .map_err(|error| format!("invalid record identity: {error}"))
}

#[cfg(all(test, feature = "desktop"))]
#[path = "bridge_tests.rs"]
mod tests;
