//! UIKit scene/app lifecycle onto the editor. Backgrounding requests an autosave
//! flush and refuses frames and input (iOS forbids GPU work in the background);
//! termination cancels host dialogs, awaits the flush and stops the document
//! hosts within the shutdown grace.
use crate::app::{Failure, IosApp, Outcome, Phase, SHUTDOWN_GRACE};
use crate::ffi::{self, Status};

impl IosApp {
    pub fn lifecycle(&mut self, event: u32) -> Outcome {
        if self.phase == Phase::Terminated && event != ffi::MEMORY_WARNING {
            return Err(Failure::new(Status::State, "the app has terminated"));
        }
        match event {
            ffi::WILL_RESIGN_ACTIVE => self.focus(false),
            ffi::DID_ENTER_BACKGROUND => {
                self.focus(false);
                // Not awaited: the actors write on their own threads while the main
                // thread returns to UIKit (the host may hold a background task).
                if let Some(editor) = &self.editor {
                    editor.flush_recovery();
                }
                self.phase = Phase::Background;
            }
            ffi::WILL_ENTER_FOREGROUND => self.phase = Phase::Inactive,
            ffi::DID_BECOME_ACTIVE => self.focus(true),
            ffi::MEMORY_WARNING => {
                if let Some(editor) = &mut self.editor {
                    editor.release_caches();
                }
            }
            ffi::WILL_TERMINATE => {
                self.shutdown(SHUTDOWN_GRACE);
                self.phase = Phase::Terminated;
            }
            ffi::TRAIT_DARK => self.theme = Some(egui::Theme::Dark),
            ffi::TRAIT_LIGHT => self.theme = Some(egui::Theme::Light),
            _ => {
                return Err(Failure::new(
                    Status::InvalidArgument,
                    "unknown lifecycle event",
                ));
            }
        }
        self.ctx.request_repaint();
        Ok(())
    }

    /// Focus loss cancels live contacts; egui sees the window focus change too.
    fn focus(&mut self, focused: bool) {
        if !focused {
            self.input.cancel_all();
        }
        self.input.push_event(egui::Event::WindowFocused(focused));
        self.phase = if focused {
            Phase::Active
        } else {
            Phase::Inactive
        };
    }
}
