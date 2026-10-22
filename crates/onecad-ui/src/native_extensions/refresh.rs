//! One window-owned catalog request, independent of every document command lane.
use super::Reply;
use crate::{qa::QaPaths, service_runtime::ServiceRuntime};
use tokio::sync::oneshot;

pub(crate) struct Service {
    paths: QaPaths,
    pending: Option<oneshot::Receiver<Result<Reply, String>>>,
}

impl Service {
    pub(crate) fn new(paths: QaPaths) -> Self {
        Self {
            paths,
            pending: None,
        }
    }

    pub(crate) fn submit(&mut self, runtime: &ServiceRuntime, context: egui::Context) -> bool {
        let paths = self.paths.clone();
        self.spawn(runtime, context, async move {
            super::service::refresh(&paths).await
        })
    }

    fn spawn(
        &mut self,
        runtime: &ServiceRuntime,
        context: egui::Context,
        work: impl std::future::Future<Output = Result<Reply, String>> + Send + 'static,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let (sender, receiver) = oneshot::channel();
        self.pending = Some(receiver);
        // Runtime shutdown drains this task alongside the document actors.
        runtime.spawn_actor(async move {
            let _ = sender.send(work.await);
            context.request_repaint();
        });
        true
    }

    pub(crate) fn poll(&mut self) -> Option<Result<Reply, String>> {
        let result = match self.pending.as_mut()?.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return None,
            Err(oneshot::error::TryRecvError::Closed) => {
                Err("Extension refresh service stopped before replying".into())
            }
        };
        self.pending = None;
        Some(result)
    }
}

#[cfg(test)]
mod tests;
