//! Bounded admission precedes application scheduling; durable results retain their permit.
pub(crate) use crate::event_mailbox::{BackendEvent, EventMailbox};
use crate::{profiles::Profile, qa::QaPaths, services};
use onecad_app::{
    dto::DocumentProjection,
    host::{AppEvent, EventSink, HostContext, HostPaths},
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
    pub prepared: Mutex<Option<onecad_render::PreparedScene>>,
    pub _lease: Arc<crate::budget::Lease>,
    pub records: Vec<OperationRecord>,
    pub render_receipt: Option<crate::render_receipt::RenderReceipt>,
}

#[derive(Debug)]
pub enum Command {
    Open,
    Document(crate::document_flow::Request),
    Recovery {
        document_id: String,
        choice: crate::startup_recovery::Choice,
    },
    Sketch(Box<crate::sketch_services::Request>),
    Apply {
        tag: RequestTag,
        command: Box<EditCommand>,
        hide_sketch: Option<String>,
    },
    Undo,
    Redo,
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
}

pub struct CandidateDelivery {
    pub action: Action,
    pub prepared: Option<onecad_render::PreparedScene>,
    pub lease: Option<crate::budget::Lease>,
    pub scene: Option<Arc<SceneSnapshot>>,
}

pub enum Message {
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
    pub display: Mutex<Option<Arc<Display>>>,
    pub recents: Mutex<Vec<onecad_app::dto::RecentProjectDto>>,
    pub recovery: Mutex<crate::startup_recovery::State>,
    pub candidate: Mutex<Option<(u64, Candidate)>>,
    pub candidate_result: Mutex<Option<(u64, CandidateDelivery)>>,
    pub display_epoch: std::sync::atomic::AtomicU64,
    pub failure: Mutex<Option<String>>,
    pub activity: Mutex<Option<String>>,
    pub render_receipt: Mutex<Option<crate::render_receipt::RenderReceipt>>,
    pub valid: std::sync::atomic::AtomicBool,
    pub stopping: std::sync::atomic::AtomicBool,
    pub shutdown: Notify,
    pub recovery_flush: Notify,
    pub budget: Mutex<onecad_render::PreparationBudget>,
    pub host_budget: Arc<crate::budget::HostBudget>,
    generation: std::sync::atomic::AtomicU64,
    notify: Notify,
    context: egui::Context,
}

impl Shared {
    pub(crate) fn new(context: egui::Context, budget: onecad_render::PreparationBudget) -> Self {
        Self {
            display: Mutex::new(None),
            recents: Mutex::new(Vec::new()),
            recovery: Mutex::new(Default::default()),
            candidate: Mutex::new(None),
            candidate_result: Mutex::new(None),
            display_epoch: std::sync::atomic::AtomicU64::new(0),
            failure: Mutex::new(None),
            activity: Mutex::new(None),
            render_receipt: Mutex::new(None),
            budget: Mutex::new(budget),
            host_budget: Arc::new(crate::budget::HostBudget::default()),
            valid: std::sync::atomic::AtomicBool::new(false),
            stopping: std::sync::atomic::AtomicBool::new(false),
            shutdown: Notify::new(),
            recovery_flush: Notify::new(),
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
    pub fn wake(&self) {
        self.context.request_repaint();
    }
    pub fn release_text_focus(&self, id: egui::Id) {
        self.context.memory_mut(|memory| memory.surrender_focus(id));
    }
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(std::sync::atomic::Ordering::Acquire) == generation
    }
    pub fn publish_display(&self, display: Display) {
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
    commands: async_mpsc::Sender<Envelope>,
    permits: Arc<Semaphore>,
    pub shared: Arc<Shared>,
    pub messages: mpsc::Receiver<Message>,
}

impl Bridge {
    pub fn start(
        paths: QaPaths,
        context: egui::Context,
        budget: onecad_render::PreparationBudget,
    ) -> Result<Self, String> {
        let (commands, receiver) = async_mpsc::channel(ADMISSION);
        let (sender, messages) = mpsc::channel();
        let shared = Arc::new(Shared::new(context, budget));
        let backend_shared = shared.clone();
        std::thread::Builder::new()
            .name("onecad-native-services".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(actor(paths, backend_shared, receiver, sender)),
                    Err(error) => {
                        let _ = sender.send(Message::StartupFailed(format!(
                            "native service runtime: {error}"
                        )));
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            commands,
            permits: Arc::new(Semaphore::new(ADMISSION)),
            shared,
            messages,
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

    pub fn candidate(&self, candidate: Candidate) {
        let generation = self
            .shared
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        if let Ok(mut slot) = self.shared.candidate.lock() {
            *slot = Some((generation, candidate));
        }
        self.shared.notify.notify_one();
    }

    pub fn request_shutdown(&self) {
        self.shared
            .stopping
            .store(true, std::sync::atomic::Ordering::Release);
        self.shared.shutdown.notify_one();
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
                    self.events
                        .emit(BackendEvent::WorkerFailed)
                        .map_err(|e| e.to_string())?;
                } else if status.state == "ready" {
                    self.failed.store(false, Ordering::Release);
                }
                format!("Geometry worker: {}", status.state)
            }
            AppEvent::ProjectionUpdated(projection) => {
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
            AppEvent::Autosave(_) => "Autosave updated".to_owned(),
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
    pub state: AppState,
    pub host: HostContext,
    pub paths: QaPaths,
    pub events: Arc<EventMailbox>,
    pub shared: Arc<Shared>,
    pub startup_opened: bool,
    pub scene_revision: u64,
    pub previous_scene: Option<(Arc<SceneSnapshot>, Arc<crate::budget::Lease>)>,
    pub preparation_cache: onecad_render::PreparationCache,
    pub preparation_lease: Option<Arc<crate::budget::Lease>>,
}

async fn actor(
    paths: QaPaths,
    shared: Arc<Shared>,
    mut receiver: async_mpsc::Receiver<Envelope>,
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
        dialogs: Arc::new(crate::dialogs::NativeDialogs::new(
            paths.root.join("documents"),
        )),
    };
    let state = AppState::default();
    if let Err(error) = state.start(host.clone()) {
        let _ = sender.send(Message::StartupFailed(error.into()));
        shared.wake();
        return;
    }
    let mut backend = Backend {
        state,
        host,
        paths,
        events: event_rx,
        shared: shared.clone(),
        startup_opened: false,
        scene_revision: 0,
        previous_scene: None,
        preparation_cache: Default::default(),
        preparation_lease: None,
    };
    loop {
        tokio::select! {
            _=shared.shutdown.notified()=>{break;}
            _=shared.recovery_flush.notified()=>{
                if let Some(root) = onecad_app::autosave::autosave_root(&backend.host)
                    && let Some(event) = onecad_app::autosave::autosave_current_reporting(
                        &backend.state.runtime, &root, &backend.state.persistence,
                    ).await
                    && let Some(error) = event.error
                {
                    eprintln!("Viewport recovery checkpoint failed: {error}");
                }
            }
            envelope = receiver.recv() => {
                let Some(envelope) = envelope else { break; };
                services::execute(&mut backend,envelope.command,envelope.permit,&sender).await;
                if shared.stopping.load(std::sync::atomic::Ordering::Acquire) { break; }
            }
            event = backend.events.next() => {
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
                        _=shared.shutdown.notified()=>{backend.state.retire_all();break;}
                    }
                }
            }
        }
    }
    receiver.close();
    while let Ok(envelope) = receiver.try_recv() {
        let name = command_name(&envelope.command);
        let tag = match envelope.command {
            Command::Apply { tag, .. } => Some(tag),
            _ => None,
        };
        let _ = sender.send(Message::Terminal {
            command: name,
            tag,
            result: Err("Command cancelled before execution during shutdown".into()),
            _permit: envelope.permit,
        });
    }
    let _ = onecad_app::api::close_document(&backend.state, backend.host.clone()).await;
    backend.state.retire_all();
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
        Command::Open => "Open",
        Command::Document(_) => "Document",
        Command::Recovery { .. } => "Recovery",
        Command::Sketch(_) => "Sketch",
        Command::Apply { .. } => "Apply",
        Command::Undo => "Undo",
        Command::Redo => "Redo",
    }
}

pub fn record_id(raw: &str) -> Result<RecordId, String> {
    raw.parse()
        .map_err(|error| format!("invalid record identity: {error}"))
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
