use crate::{RenderError, Renderer};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(crate) enum Flight {
    Idle,
    Encoded,
    Submitted(Arc<AtomicBool>),
}
impl Flight {
    pub(crate) fn submitted(queue: &wgpu::Queue, wake: impl FnOnce() + Send + 'static) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let callback = Arc::clone(&done);
        queue.on_submitted_work_done(move || {
            callback.store(true, Ordering::Release);
            wake();
        });
        Self::Submitted(done)
    }
}
impl Renderer {
    /// Poll completion without waiting. One frame at a time bounds queue staging and prevents
    /// uniform writes for a later capture overwriting an earlier unsubmitted encoder.
    pub fn poll_idle(&mut self) -> Result<bool, RenderError> {
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| RenderError::Device(e.to_string()))?;
        if let Flight::Submitted(done) = &self.flight {
            if done.load(Ordering::Acquire) {
                self.flight = Flight::Idle;
            }
        }
        Ok(matches!(self.flight, Flight::Idle))
    }
    pub(crate) fn require_idle(&mut self) -> Result<(), RenderError> {
        if self.poll_idle()? {
            Ok(())
        } else {
            Err(RenderError::Busy)
        }
    }
    /// Call after ALL shared-queue submissions that use this frame, including the UI pass
    /// sampling output_view. For eframe this can be the next frame's raw input hook after
    /// the prior egui submission. Calling after only CAD submission does not fence UI use.
    /// Wake the host's on-demand loop from completion; do not wait on its UI thread.
    pub fn after_submit(
        &mut self,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<(), RenderError> {
        if !matches!(self.flight, Flight::Encoded) {
            return Err(RenderError::SubmissionState);
        }
        self.flight = Flight::submitted(&self.queue, wake);
        Ok(())
    }
    /// Drop the outstanding encoder before calling. Flush queued writes and fence their
    /// completion; a dropped frame must not leave unbounded staging or unlock early.
    pub fn abandon_frame(
        &mut self,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<(), RenderError> {
        if !matches!(self.flight, Flight::Encoded) {
            return Err(RenderError::SubmissionState);
        }
        self.queue.submit([]);
        self.flight = Flight::submitted(&self.queue, wake);
        Ok(())
    }
}
