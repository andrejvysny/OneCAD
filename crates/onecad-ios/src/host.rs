//! Host requests: the editor's file dialogs, exports and clipboard/URL output
//! become a bounded queue the UIKit host polls after a frame reports
//! `ONECAD_IOS_FRAME_HOST_REQUESTS`. A dialog awaits its completion on a oneshot
//! channel; closing the queue (destroy) completes every waiter as cancelled.
use crate::ffi;
use onecad_app::host::{DialogPort, FileDialog};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

pub const CAPACITY: usize = 64;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub id: u64,
    pub kind: u32,
    pub multiple: bool,
    pub label: String,
    pub extensions: String,
    pub path: String,
    pub suggested_name: String,
    pub text: String,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    pending: VecDeque<Request>,
    waiting: HashMap<u64, oneshot::Sender<Vec<String>>>,
    closed: bool,
}

pub struct Queue {
    inner: Mutex<Inner>,
    ctx: egui::Context,
}

impl Queue {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            ctx,
        }
    }

    /// Queues `request` (its id is assigned here); `None` when full or closed.
    pub fn push(
        &self,
        mut request: Request,
        completion: Option<oneshot::Sender<Vec<String>>>,
    ) -> Option<u64> {
        let mut inner = self.inner.lock().ok()?;
        if inner.closed || inner.pending.len() + inner.waiting.len() >= CAPACITY {
            return None;
        }
        inner.next_id += 1;
        request.id = inner.next_id;
        if let Some(completion) = completion {
            inner.waiting.insert(request.id, completion);
        }
        inner.pending.push_back(request.clone());
        drop(inner);
        // Wakes the host (from any thread) so the next frame reports the request.
        self.ctx.request_repaint();
        Some(request.id)
    }

    pub fn pop(&self) -> Option<Request> {
        self.inner.lock().ok()?.pending.pop_front()
    }

    pub fn has_pending(&self) -> bool {
        self.inner
            .lock()
            .is_ok_and(|inner| !inner.pending.is_empty())
    }

    /// Completes a popped request; `false` for an unknown or finished id.
    pub fn complete(&self, id: u64, paths: Vec<String>) -> bool {
        let sender = self
            .inner
            .lock()
            .ok()
            .and_then(|mut inner| inner.waiting.remove(&id));
        match sender {
            Some(sender) => {
                // The awaiting dialog may have been dropped with its document.
                let _ = sender.send(paths);
                true
            }
            None => false,
        }
    }

    /// Cancels every waiter and refuses new requests.
    pub fn close(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.closed = true;
            inner.pending.clear();
            inner.waiting.clear();
        }
    }
}

/// `DialogPort` for document services: Open/Import become OPEN_DOCUMENT and
/// Save As/Export become SAVE_DOCUMENT (a name inside the sandbox Documents
/// directory); an exported file is then published with EXPORT_FILE.
pub struct Dialogs(pub Arc<Queue>);

#[async_trait::async_trait]
impl DialogPort for Dialogs {
    async fn pick(&self, request: FileDialog) -> Vec<String> {
        let (sender, receiver) = oneshot::channel();
        let kind = if request.save {
            ffi::REQ_SAVE_DOCUMENT
        } else {
            ffi::REQ_OPEN_DOCUMENT
        };
        let suggested_name = if request.save {
            format!(
                "Untitled.{}",
                request.extensions.first().map_or("onecad", String::as_str)
            )
        } else {
            String::new()
        };
        let queued = Request {
            kind,
            multiple: request.multiple,
            label: request.label,
            extensions: request.extensions.join(","),
            suggested_name,
            ..Default::default()
        };
        if self.0.push(queued, Some(sender)).is_none() {
            tracing::warn!("host request queue full or closed; dialog cancelled");
            return Vec::new();
        }
        receiver.await.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogs_wait_for_completion_and_close_cancels() {
        let queue = Arc::new(Queue::new(egui::Context::default()));
        let dialogs = Dialogs(queue.clone());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let picked = std::thread::spawn({
            let queue = queue.clone();
            move || loop {
                if let Some(request) = queue.pop() {
                    assert_eq!(request.kind, ffi::REQ_SAVE_DOCUMENT);
                    assert_eq!(request.extensions, "step");
                    assert_eq!(request.suggested_name, "Untitled.step");
                    assert!(queue.complete(request.id, vec!["/docs/a.step".into()]));
                    assert!(!queue.complete(request.id, Vec::new()));
                    break;
                }
                std::thread::yield_now();
            }
        });
        let paths = runtime.block_on(dialogs.pick(FileDialog::new("STEP", &["step"], true)));
        picked.join().unwrap();
        assert_eq!(paths, ["/docs/a.step"]);
        queue.close();
        let cancelled =
            runtime.block_on(dialogs.pick(FileDialog::new("OneCAD", &["onecad"], false)));
        assert!(cancelled.is_empty());
    }

    #[test]
    fn queue_is_bounded() {
        let queue = Queue::new(egui::Context::default());
        for _ in 0..CAPACITY {
            assert!(queue.push(Request::default(), None).is_some());
        }
        assert!(queue.push(Request::default(), None).is_none());
        assert!(queue.pop().is_some_and(|request| request.id == 1));
    }
}
