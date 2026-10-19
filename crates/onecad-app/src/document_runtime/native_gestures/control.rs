use super::*;
use crate::{api::CommandCompletion, dto::*, error::ApiError, host::HostContext};
use std::sync::Mutex;
use tokio::sync::{oneshot, watch, Notify};

pub(crate) type EndCompletion = CommandCompletion<NativeGestureReceipt<NativeGestureEnd>>;
pub(crate) type UpdateReply =
    oneshot::Sender<Result<Option<NativeGesturePreview<DragSolveDto>>, ApiError>>;
pub(crate) type Registry = Arc<Mutex<Option<Arc<Control>>>>;

pub(crate) struct UpdateRequest {
    pub target: [f64; 2],
    pub seq: u64,
    pub reply: UpdateReply,
}

pub(crate) struct EndRequest {
    pub target: Option<[f64; 2]>,
    pub host: HostContext,
    pub reply: oneshot::Sender<Result<EndCompletion, ApiError>>,
}

#[derive(Default)]
struct Requests {
    cancelled: bool,
    terminal: bool,
    seq: u64,
    update: Option<UpdateRequest>,
    end: Option<EndRequest>,
}

pub(crate) enum Next {
    Cancel,
    Update(UpdateRequest),
    End(EndRequest),
    Wait,
}

pub(crate) struct Control {
    pub id: u64,
    pub scope: NativeGestureScope,
    requests: Mutex<Requests>,
    pub wake: Notify,
    pub ready: watch::Sender<Option<Result<NativeGesturePreview<BeginGestureDto>, ApiError>>>,
    pub settled: watch::Sender<Option<Result<(), ApiError>>>,
    // A poisoned slot stays bounded and blocks reuse rather than pretending remote cleanup succeeded.
    pub poisoned_claim: Mutex<Option<Claim>>,
}

impl Control {
    pub fn new(id: u64, scope: NativeGestureScope) -> Arc<Self> {
        Arc::new(Self {
            id,
            scope,
            requests: Mutex::new(Requests::default()),
            wake: Notify::new(),
            ready: watch::channel(None).0,
            settled: watch::channel(None).0,
            poisoned_claim: Mutex::new(None),
        })
    }

    pub fn cancel(&self) {
        self.requests.lock().unwrap().cancelled = true;
        self.wake.notify_one();
    }

    pub fn cancelled(&self) -> bool {
        self.requests.lock().unwrap().cancelled
    }

    pub fn update(&self, target: [f64; 2], reply: UpdateReply) -> Result<(), ApiError> {
        finite_target(Some(target))?;
        let mut requests = self.requests.lock().unwrap();
        if requests.cancelled || requests.terminal {
            return Err(refused("Native gesture is ending or cancelled"));
        }
        requests.seq = requests
            .seq
            .checked_add(1)
            .ok_or_else(|| refused("Native gesture update sequence exhausted"))?;
        let seq = requests.seq;
        if let Some(previous) = requests
            .update
            .replace(UpdateRequest { target, seq, reply })
        {
            let _ = previous.reply.send(Ok(None));
        }
        self.wake.notify_one();
        Ok(())
    }

    pub fn end(&self, request: EndRequest) -> Result<(), ApiError> {
        finite_target(request.target)?;
        let mut requests = self.requests.lock().unwrap();
        if requests.cancelled || requests.terminal {
            return Err(refused("Native gesture is ending or cancelled"));
        }
        requests.terminal = true;
        requests.end = Some(request);
        self.wake.notify_one();
        Ok(())
    }

    pub fn next(&self) -> Next {
        let mut requests = self.requests.lock().unwrap();
        if requests.cancelled {
            return Next::Cancel;
        }
        if let Some(update) = requests.update.take() {
            return Next::Update(update);
        }
        if let Some(end) = requests.end.take() {
            return Next::End(end);
        }
        Next::Wait
    }

    pub fn finish(&self, result: Result<(), ApiError>) {
        self.cancel();
        let mut requests = self.requests.lock().unwrap();
        let error = result
            .clone()
            .err()
            .unwrap_or_else(|| refused("Native gesture cancelled"));
        if let Some(update) = requests.update.take() {
            let _ = update.reply.send(Err(error.clone()));
        }
        if let Some(end) = requests.end.take() {
            let _ = end.reply.send(Err(error.clone()));
        }
        if self.ready.borrow().is_none() {
            self.ready.send_replace(Some(Err(error)));
        }
        self.settled.send_replace(Some(result));
    }

    pub fn receipt<T>(&self, value: T) -> NativeGestureReceipt<T> {
        NativeGestureReceipt {
            scope: self.scope.clone(),
            gesture_id: self.id,
            value,
        }
    }
}

pub(crate) fn refused(message: &str) -> ApiError {
    ApiError::InvalidCommand(message.into())
}

fn finite_target(target: Option<[f64; 2]>) -> Result<(), ApiError> {
    if target.is_some_and(|at| !at.iter().all(|v| v.is_finite())) {
        return Err(refused("Native gesture target must be finite"));
    }
    Ok(())
}

pub(crate) struct CancelOnDrop(pub Option<Arc<Control>>);
impl CancelOnDrop {
    pub fn new(control: Arc<Control>) -> Self {
        Self(Some(control))
    }
    pub fn disarm(mut self) {
        self.0.take();
    }
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(control) = self.0.take() {
            control.cancel();
        }
    }
}
