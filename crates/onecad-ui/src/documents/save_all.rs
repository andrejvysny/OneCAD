//! ⌥⌘S: every document with a file saves in place, in the background; then each
//! one never saved is shown in turn and asks for a location — one dialog at a
//! time, so the user is never handed a stack of them. Cancelling a dialog stops
//! the remaining sequence; a failed save is reported and the sequence goes on.
use super::{TabId, set_current_tab};
use crate::{app::App, document_editor::DocumentEditor, document_flow::Request};
use std::collections::VecDeque;

#[derive(Default)]
pub(crate) struct SaveAll {
    /// Untitled documents still to be asked about, in tab order.
    pub(crate) queue: VecDeque<TabId>,
    /// The untitled document whose save is running.
    pub(crate) prompted: Option<TabId>,
    /// Documents left unsaved and why, reported once the sequence ends.
    pub(crate) unsaved: Vec<String>,
}

/// How the prompted document's save ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Finished {
    Saved,
    Cancelled,
    Failed,
}

fn unsaved(doc: &DocumentEditor) -> bool {
    doc.authoring_dirty || doc.document_state.as_ref().is_some_and(|s| s.dirty)
}

fn has_file(doc: &DocumentEditor) -> bool {
    doc.document_state
        .as_ref()
        .is_some_and(|s| s.path.is_some())
}

impl App {
    pub fn save_all(&mut self) {
        if self.parked {
            return;
        }
        self.tabs.save_all = SaveAll::default();
        let active = self.doc.tab;
        for tab in self.tab_order() {
            let Some(index) = self.tabs.parked.iter().position(|doc| doc.tab == tab) else {
                self.save_or_queue();
                continue;
            };
            std::mem::swap(&mut self.doc, &mut self.tabs.parked[index]);
            set_current_tab(tab);
            self.parked = true;
            self.save_or_queue();
            self.parked = false;
            std::mem::swap(&mut self.doc, &mut self.tabs.parked[index]);
            set_current_tab(active);
        }
        self.advance_save_all();
    }

    /// Saves the document in `self.doc` in place, or queues it for a location.
    /// A busy document is skipped, and named when the sequence ends.
    fn save_or_queue(&mut self) {
        if !unsaved(&self.doc) {
            return;
        }
        if self.busy() {
            self.skip_save("busy");
            return;
        }
        if !has_file(&self.doc) {
            self.tabs.save_all.queue.push_back(self.doc.tab);
            return;
        }
        self.document_request(Request::Save { save_as: false });
        if !self.doc.documents.running {
            let reason = self.doc.status.clone();
            self.skip_save(&reason);
        }
    }

    fn skip_save(&mut self, reason: &str) {
        let entry = format!("{} ({reason})", self.doc.editor_title());
        self.tabs.save_all.unsaved.push(entry);
    }

    /// Shows the next untitled document and starts its save (and dialog).
    fn advance_save_all(&mut self) {
        while let Some(tab) = self.tabs.save_all.queue.pop_front() {
            self.show_tab(tab);
            // Closed, or saved by the user, since Save All began.
            if self.doc.tab != tab || !unsaved(&self.doc) {
                continue;
            }
            self.document_request(Request::Save { save_as: false });
            if self.doc.documents.running {
                self.tabs.save_all.prompted = Some(tab);
                return;
            }
            let reason = self.doc.status.clone();
            self.skip_save(&reason);
        }
        self.report_save_all();
    }

    fn report_save_all(&mut self) {
        let unsaved = std::mem::take(&mut self.tabs.save_all.unsaved);
        if !unsaved.is_empty() {
            self.doc.command_status = true;
            self.doc.status = format!("Save All left unsaved: {}", unsaved.join(", "));
        }
    }

    /// `tab`'s file action finished; continues Save All if it was the document
    /// being asked about, even when that document has since closed.
    pub(crate) fn save_all_finished(&mut self, tab: TabId, finished: Finished) {
        if self.tabs.save_all.prompted != Some(tab) {
            return;
        }
        self.tabs.save_all.prompted = None;
        match finished {
            // The user stopped Save All at this document's dialog.
            Finished::Cancelled => self.tabs.save_all.queue.clear(),
            Finished::Failed => {
                let reason = self.doc.status.clone();
                self.skip_save(&reason);
            }
            Finished::Saved => {}
        }
        // Only the window's active document can bring the next one forward.
        if self.parked {
            self.tabs.save_all.queue.clear();
        }
        self.advance_save_all();
    }
}
