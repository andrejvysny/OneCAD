//! UIKit scene/app lifecycle onto the editor. Backgrounding requests an autosave
//! flush, releases the renderer's targets and refuses frames and input (iOS forbids GPU
//! work in the background); the first foreground frame recreates the targets from the
//! retained scene. A memory warning drops parked tabs' display caches and, when the
//! active document is over its display allowance, lowers the scene scale one step.
//! Termination cancels host dialogs, awaits the flush and stops the document hosts
//! within the shutdown grace.
use crate::app::{Failure, IosApp, Outcome, Phase, SHUTDOWN_GRACE};
use crate::ffi::{self, Status};

/// What one lifecycle event does. Pure, so the policy is tested without a GPU or
/// kernel; [`IosApp::lifecycle`] applies it in field order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Plan {
    /// Cancel contacts on focus loss and report the focus change to egui.
    pub focus: Option<bool>,
    pub flush_autosave: bool,
    pub release_targets: bool,
    pub relieve_memory: bool,
    pub shutdown: bool,
    pub phase: Option<Phase>,
    pub theme: Option<egui::Theme>,
}

pub(crate) fn plan(phase: Phase, event: u32) -> Result<Plan, Failure> {
    if phase == Phase::Terminated && event != ffi::MEMORY_WARNING {
        return Err(Failure::new(Status::State, "the app has terminated"));
    }
    let none = Plan::default();
    Ok(match event {
        ffi::WILL_RESIGN_ACTIVE => Plan {
            focus: Some(false),
            ..none
        },
        ffi::DID_ENTER_BACKGROUND => Plan {
            focus: Some(false),
            flush_autosave: true,
            release_targets: true,
            phase: Some(Phase::Background),
            ..none
        },
        // Targets come back with the first frame; nothing to allocate before it.
        ffi::WILL_ENTER_FOREGROUND => Plan {
            phase: Some(Phase::Inactive),
            ..none
        },
        ffi::DID_BECOME_ACTIVE => Plan {
            focus: Some(true),
            ..none
        },
        ffi::MEMORY_WARNING => Plan {
            relieve_memory: true,
            ..none
        },
        ffi::WILL_TERMINATE => Plan {
            shutdown: true,
            phase: Some(Phase::Terminated),
            ..none
        },
        ffi::TRAIT_DARK => Plan {
            theme: Some(egui::Theme::Dark),
            ..none
        },
        ffi::TRAIT_LIGHT => Plan {
            theme: Some(egui::Theme::Light),
            ..none
        },
        _ => {
            return Err(Failure::new(
                Status::InvalidArgument,
                "unknown lifecycle event",
            ));
        }
    })
}

impl IosApp {
    pub fn lifecycle(&mut self, event: u32) -> Outcome {
        let plan = plan(self.phase, event)?;
        if let Some(focused) = plan.focus {
            self.focus(focused);
        }
        // Not awaited: the actors write on their own threads while the main thread
        // returns to UIKit (the host may hold a background task).
        if plan.flush_autosave
            && let Some(editor) = &self.editor
        {
            editor.flush_recovery();
        }
        if let Some(editor) = self.editor.as_mut().filter(|_| plan.release_targets) {
            let released = editor.release_render_targets(&self.ctx);
            tracing::info!(released, "lifecycle: background released render targets");
        }
        if let Some(editor) = self.editor.as_mut().filter(|_| plan.relieve_memory) {
            let relief = editor.relieve_memory_pressure();
            tracing::info!(
                released_bytes = relief.released,
                lowered_scale = ?relief.lowered_scale,
                "lifecycle: memory warning released display caches"
            );
        }
        if plan.shutdown {
            self.shutdown(SHUTDOWN_GRACE);
        }
        if let Some(phase) = plan.phase {
            self.phase = phase;
        }
        if plan.theme.is_some() {
            self.theme = plan.theme;
        }
        self.ctx.request_repaint();
        Ok(())
    }

    /// Focus loss cancels live contacts; egui sees the window focus change too.
    fn focus(&mut self, focused: bool) {
        if !focused {
            self.input
                .cancel_all(onecad_interaction::touch::CancelReason::SceneInactive);
        }
        self.input.push_event(egui::Event::WindowFocused(focused));
        self.phase = if focused {
            Phase::Active
        } else {
            Phase::Inactive
        };
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
