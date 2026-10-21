//! Several open documents in one window.
//!
//! The active document's editor is `App.doc`; every other open document is parked
//! here with its own bridge, document host and worker. Switching swaps editors, so
//! all existing code keeps addressing "the current document" through `app.doc`.
//!
//! Parked documents keep running — regeneration and autosave continue — so every
//! frame their messages are applied (`App.parked` set, which suppresses window-level
//! effects). That also returns their admission permits; an undrained parked bridge
//! would otherwise fill its queue and strand an accepted edit. Their published
//! displays are left in the bridge's latest-only slot and adopted on activation.
use crate::document_editor::DocumentEditor;
use crate::{app::App, bridge::Bridge, qa::QaPaths, service_runtime::ServiceRuntime};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

mod save_all;
pub(crate) use save_all::Finished as SaveAllFinished;

pub type TabId = u64;

static NEXT_TAB: AtomicU64 = AtomicU64::new(1);
thread_local! {
    /// The tab whose editor is in `App.doc` right now (active, or parked while
    /// drained). Per thread because each window's UI runs on its own thread; a
    /// process-wide value would leak between windows (and between parallel tests).
    static CURRENT_TAB: std::cell::Cell<TabId> = const { std::cell::Cell::new(0) };
}

/// An egui memory id for transient per-document widget state (a numeric field
/// draft, a marquee, a dimension editor), so one document's half-finished
/// interaction never appears in another's tab. Window-level state keeps plain ids.
pub fn scoped_id(name: &'static str) -> egui::Id {
    egui::Id::unique(name).with(CURRENT_TAB.get())
}

pub(crate) fn set_current_tab(tab: TabId) {
    CURRENT_TAB.set(tab);
}

pub fn next_tab_id() -> TabId {
    NEXT_TAB.fetch_add(1, Ordering::Relaxed)
}

/// A document's viewport as it looked when it was last left, for the ⌃Tab
/// switcher. The GPU texture lives as long as the egui registration using it.
pub struct Thumbnail {
    pub texture: egui::TextureId,
    pub(crate) _source: wgpu::Texture,
}

/// What tab chrome needs to draw one open document.
#[derive(Clone, Debug, PartialEq)]
pub struct TabSummary {
    pub tab: TabId,
    pub title: String,
    pub dirty: bool,
    pub active: bool,
    pub busy: bool,
    pub awaiting_scene: bool,
    /// It holds a document (an empty host waiting for New/Open is not a tab).
    pub open: bool,
    /// Its geometry worker or scene refresh failed while it was in the background.
    pub failed: bool,
}

/// A background New/Open/Import that ended without a document, reported in the
/// active tab once its own tab has closed.
struct Settled {
    tab: TabId,
    diagnostic: Option<(crate::diagnostics::FileAction, crate::diagnostics::State)>,
}

pub struct Tabs {
    runtime: Arc<ServiceRuntime>,
    paths: QaPaths,
    budget: onecad_render::PreparationBudget,
    /// Open documents other than the active one, in tab order around it.
    pub(crate) parked: Vec<DocumentEditor>,
    /// Closed documents whose hosts are still shutting down.
    closing: Vec<DocumentEditor>,
    /// Monotonic activation counter (most-recent-first order).
    activations: u64,
    /// Parked provisional tabs whose New/Open/Import ended without a document.
    settled: Vec<Settled>,
    /// Tabs parked with a tool's evaluation and preview dropped; re-issued once
    /// shown and free of a file action.
    stranded: Vec<TabId>,
    pub(crate) save_all: save_all::SaveAll,
}

impl Tabs {
    pub fn new(
        runtime: Arc<ServiceRuntime>,
        paths: QaPaths,
        budget: onecad_render::PreparationBudget,
    ) -> Self {
        Self {
            runtime,
            paths,
            budget,
            parked: Vec::new(),
            closing: Vec::new(),
            activations: 0,
            settled: Vec::new(),
            stranded: Vec::new(),
            save_all: Default::default(),
        }
    }

    pub fn bridge(&self, context: egui::Context, startup: bool) -> Result<Bridge, String> {
        let mut paths = self.paths.clone();
        // Only the first window bridge may open the launch document.
        paths.open_on_start &= startup;
        let bridge = Bridge::start_on(self.runtime.clone(), paths, context, self.budget)?;
        if !startup && let Ok(mut recovery) = bridge.shared.recovery.lock() {
            // Crash recovery is discovered once per window, by the startup bridge.
            recovery.checked = true;
        }
        Ok(bridge)
    }

    /// Whether every closed tab's host has finished shutting down.
    #[cfg(test)]
    pub fn all_closing_stopped(&self) -> bool {
        self.closing.is_empty()
    }

    fn all_stopped(&self) -> bool {
        self.parked
            .iter()
            .chain(&self.closing)
            .all(|doc| doc.stopped)
    }
}

impl App {
    /// Every open document in tab order, the active one included.
    pub fn tab_summaries(&self) -> Vec<TabSummary> {
        let summary = |doc: &DocumentEditor, active: bool| TabSummary {
            tab: doc.tab,
            title: doc.editor_title(),
            dirty: doc.document_state.as_ref().is_some_and(|state| state.dirty),
            active,
            busy: doc.documents.running || doc.items.blocks(),
            awaiting_scene: doc.awaiting_scene,
            open: doc.document_state.is_some(),
            // Left in its slot so activation reports it in full; only peeked here.
            failed: doc
                .bridge
                .shared
                .failure
                .lock()
                .is_ok_and(|failure| failure.is_some()),
        };
        let mut tabs: Vec<_> = self
            .tabs
            .parked
            .iter()
            .map(|doc| summary(doc, false))
            .collect();
        tabs.insert(self.active_index(), summary(&self.doc, true));
        tabs
    }

    /// Opens a new, empty document tab and makes it active. The caller then
    /// submits the New/Open/Import request to it.
    pub fn open_tab(&mut self) -> Result<(), String> {
        let bridge = self.tabs.bridge(self.context(), false)?;
        let editor = DocumentEditor::new(bridge, String::new())?;
        if self.parked {
            return Err("A background document cannot open tabs".into());
        }
        self.deactivate();
        let previous = std::mem::replace(&mut self.doc, editor);
        self.tabs.parked.push(previous);
        self.tabs.parked.sort_by_key(|doc| doc.tab);
        self.activated();
        Ok(())
    }

    /// Makes `tab` the active document. Unknown tabs are ignored.
    pub fn activate_tab(&mut self, tab: TabId) {
        if self.parked {
            return; // Only the window's active editor changes tabs.
        }
        let Some(index) = self.tabs.parked.iter().position(|doc| doc.tab == tab) else {
            return;
        };
        self.deactivate();
        std::mem::swap(&mut self.doc, &mut self.tabs.parked[index]);
        // Tab order is opening order; parked editors stay sorted by tab id.
        self.tabs.parked.sort_by_key(|doc| doc.tab);
        self.activated();
    }

    /// Activates the document `step` tabs away from the active one, wrapping.
    pub fn cycle_tab(&mut self, step: isize) {
        let count = self.tabs.parked.len() + 1;
        if count < 2 || self.busy() {
            return;
        }
        let order: Vec<TabId> = self.tab_order();
        let current = self.active_index() as isize;
        let target = (current + step).rem_euclid(count as isize) as usize;
        self.activate_tab(order[target]);
    }

    pub(crate) fn tab_order(&self) -> Vec<TabId> {
        let mut order: Vec<TabId> = self.tabs.parked.iter().map(|doc| doc.tab).collect();
        order.insert(self.active_index(), self.doc.tab);
        order
    }

    /// Closes the active document's host and activates a neighbour; the window
    /// keeps at least one (possibly empty) document. `discard` names the runtime
    /// session the user chose to discard.
    pub fn close_active_tab(&mut self, discard: Option<&str>) -> Result<(), String> {
        if self.parked {
            // A parked document's close decision (Save, then close) waits for it
            // to be shown again; the swap in `with_parked` must not lose its slot.
            self.doc.close_when_active = discard.map_or(Some(None), |s| Some(Some(s.into())));
            return Ok(());
        }
        self.deactivate();
        // The right-hand neighbour takes over, else the left-hand one, else a fresh
        // empty document so the window always has an editor.
        let replacement = match self.tabs.parked.is_empty() {
            true => DocumentEditor::new(self.tabs.bridge(self.context(), false)?, String::new())?,
            false => {
                let index = self.active_index().min(self.tabs.parked.len() - 1);
                self.tabs.parked.remove(index)
            }
        };
        let closing = std::mem::replace(&mut self.doc, replacement);
        match discard {
            Some(session) => closing.bridge.request_discard_shutdown(session),
            None => closing.bridge.request_shutdown(),
        }
        self.tabs.closing.push(closing);
        self.activated();
        if self.doc.document_state.is_none() && self.doc.display.is_none() {
            self.ui.start_open = true;
        }
        Ok(())
    }

    /// The active tab's position in tab order.
    pub(crate) fn active_index(&self) -> usize {
        self.tabs
            .parked
            .iter()
            .filter(|doc| doc.tab < self.doc.tab)
            .count()
    }

    /// Requests every open document's host to stop (window quit).
    pub fn stop_all_tabs(&mut self) {
        for doc in &self.tabs.parked {
            doc.bridge.cancel_candidates();
            doc.bridge.request_shutdown();
        }
    }

    /// Whether every document host in the window has stopped.
    pub fn all_tabs_stopped(&self) -> bool {
        self.doc.stopped && self.tabs.all_stopped()
    }

    /// Applies each parked document's messages, then drops closed hosts.
    pub fn drain_parked(&mut self, ctx: &egui::Context) {
        for index in 0..self.tabs.parked.len() {
            self.with_parked(index, ctx, false);
        }
        for index in 0..self.tabs.closing.len() {
            self.with_parked(index, ctx, true);
        }
        for settled in std::mem::take(&mut self.tabs.settled) {
            if let Some(index) = self
                .tabs
                .parked
                .iter()
                .position(|doc| doc.tab == settled.tab)
            {
                let doc = self.tabs.parked.remove(index);
                doc.bridge.request_shutdown();
                self.report_settled(&doc.status, settled.diagnostic);
                self.tabs.closing.push(doc);
            }
        }
        self.tabs
            .stranded
            .retain(|tab| self.doc.tab == *tab || self.tabs.parked.iter().any(|d| d.tab == *tab));
        self.republish_stranded();
        let (stopped, closing): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tabs.closing)
            .into_iter()
            .partition(|doc| doc.stopped);
        self.tabs.closing = closing;
        for doc in stopped {
            self.release_thumbnail(doc.thumbnail);
        }
    }

    /// Keeps the outgoing document's last drawn frame for the ⌃Tab switcher. Only
    /// a frame that really shows this document is kept; the blit is ordered after
    /// the frame's submission on the same queue.
    fn capture_thumbnail(&mut self) {
        const MAX: [u32; 2] = [336, 212];
        let shown = self.doc.display.as_ref().is_some_and(|display| {
            !self.doc.awaiting_scene
                && self.renderer.scene_identity() == Some(display.scene.identity())
        });
        if !shown {
            return;
        }
        let Some(snapshot) =
            self.renderer
                .snapshot_thumbnail(&self.gpu.device, &self.gpu.queue, MAX)
        else {
            return;
        };
        let texture = self.gpu.renderer.write().register_native_texture(
            &self.gpu.device,
            &snapshot.view,
            wgpu::FilterMode::Linear,
        );
        let previous = self.doc.thumbnail.replace(Thumbnail {
            texture,
            _source: snapshot.texture,
        });
        self.release_thumbnail(previous);
    }

    fn release_thumbnail(&mut self, thumbnail: Option<Thumbnail>) {
        if let Some(thumbnail) = thumbnail {
            self.gpu.renderer.write().free_texture(&thumbnail.texture);
        }
    }

    /// The background tab is gone, so its outcome is the active tab's to show,
    /// named by its request. An already-open answer keeps the user where they are.
    fn report_settled(
        &mut self,
        status: &str,
        diagnostic: Option<(crate::diagnostics::FileAction, crate::diagnostics::State)>,
    ) {
        use crate::diagnostics::FileAction;
        let request = match diagnostic.map(|(action, _)| action) {
            Some(FileAction::New) => "New document",
            Some(FileAction::Import) => "Import",
            _ => "Open",
        };
        self.doc.command_status = true;
        self.doc.status = format!("{request} (background tab): {status}");
        if let Some((action, state)) = diagnostic {
            self.doc.diagnostics.record(
                crate::diagnostics::Source::File(action),
                state,
                &self.doc.status,
            );
        }
    }

    /// Re-issues the active tool's draft dropped when its tab was parked. Held
    /// while a file action or decision blocks authoring (as a draft change is),
    /// and retried every frame until it is not.
    fn republish_stranded(&mut self) {
        let action = onecad_interaction::Action::Republish;
        if self.parked || self.doc.documents.blocks_authoring(&action) {
            return;
        }
        let tab = self.doc.tab;
        let before = self.tabs.stranded.len();
        self.tabs.stranded.retain(|stranded| *stranded != tab);
        if self.tabs.stranded.len() != before {
            self.dispatch(action);
        }
    }

    fn with_parked(&mut self, index: usize, ctx: &egui::Context, closing: bool) {
        let active = self.doc.tab;
        let slot = if closing {
            &mut self.tabs.closing[index]
        } else {
            &mut self.tabs.parked[index]
        };
        std::mem::swap(&mut self.doc, slot);
        set_current_tab(self.doc.tab);
        self.parked = true;
        self.drain_messages(ctx);
        self.parked = false;
        set_current_tab(active);
        // Located again by identity: the drain must never have reordered the lists,
        // but a slot found by position would silently swap the wrong editor back.
        let list = if closing {
            &mut self.tabs.closing
        } else {
            &mut self.tabs.parked
        };
        // Tab mutators refuse while parked, so the placeholder is always there; if
        // not, the drained editor simply stays active and no editor is lost.
        if let Some(slot) = list.iter_mut().find(|doc| doc.tab == active) {
            std::mem::swap(&mut self.doc, slot);
        }
        debug_assert_eq!(self.doc.tab, active);
    }

    /// Applies every queued message for the editor in `doc`. A disconnected
    /// channel means its service actor ended without `Stopped` (startup failure or
    /// panic): treated as stopped, so quitting can never wait on it forever.
    pub(crate) fn drain_messages(&mut self, ctx: &egui::Context) {
        self.sync_document_state();
        loop {
            match self.doc.bridge.messages.try_recv() {
                Ok(message) => {
                    self.message(message, ctx);
                    self.sync_document_state();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.doc.stopped = true;
                    if self.stopping && self.all_tabs_stopped() {
                        self.may_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    break;
                }
            }
        }
    }

    /// Leaves the active document in a state that can be parked: transient
    /// gestures and previews are cancelled; tools, sketch mode and selection stay.
    fn deactivate(&mut self) {
        self.capture_thumbnail();
        // A parked document keeps its authored values, never a half-finished drag.
        crate::parameter_drag::abandon(self, crate::parameter_drag::Outcome::Lost);
        if self.doc.interaction.tool.is_some() && !self.tabs.stranded.contains(&self.doc.tab) {
            self.tabs.stranded.push(self.doc.tab);
        }
        self.cancel_sketch_gesture();
        self.doc.bridge.cancel_candidates();
        self.doc.sections.cancel();
        self.doc.pending_preview = None;
        self.doc.clear_preview = true;
    }

    /// Prepares the newly active document for display: picks are disabled until
    /// its own scene is drawn, and a consumed scene is re-prepared, same identity.
    fn activated(&mut self) {
        set_current_tab(self.doc.tab);
        self.tabs.activations += 1;
        self.doc.last_active = self.tabs.activations;
        self.doc.view = None;
        self.doc.clear_preview = true;
        self.draw_dirty = true;
        if let Some(state) = &self.doc.document_state {
            onecad_app::api::set_render_demand(&state.document_id);
        }
        // Never show the previous document's image under this tab.
        self.doc.awaiting_scene = self.doc.display.is_some();
        let consumed = self.doc.display.as_ref().filter(|display| {
            display
                .prepared
                .lock()
                .map_or(true, |prepared| prepared.is_none())
        });
        if let Some(scene) = consumed.map(|display| display.scene.clone())
            && self.doc.prepare_in_flight != Some(scene.identity())
        {
            let identity = scene.identity();
            match self
                .doc
                .bridge
                .submit(crate::bridge::Command::Prepare(scene))
            {
                Ok(()) => self.doc.prepare_in_flight = Some(identity),
                Err(error) => self.display_unavailable(error),
            }
        }
        if let Some(close) = self.doc.close_when_active.take() {
            let _ = self.close_active_tab(close.as_deref());
            return;
        }
        // Parking dropped the tool's evaluation and preview; its context is the
        // retained display's, so the re-issued draft previews against this scene.
        // Not held for `awaiting_scene`: preview upload is fenced on the scene's
        // identity, and an occluded window never clears that flag.
        self.republish_stranded();
        self.ui.start_open = self.doc.document_state.is_none() && self.doc.display.is_none();
        self.context().request_repaint();
    }

    /// The switched-in document cannot be drawn from its retained capture: drop
    /// it rather than wait forever; the next publication of the document redraws.
    pub(crate) fn display_unavailable(&mut self, error: String) {
        self.doc.status = format!("Document display: {error}");
        self.doc.awaiting_scene = false;
        self.doc.prepare_in_flight = None;
        self.doc.display = None;
        self.draw_dirty = true;
    }

    fn context(&self) -> egui::Context {
        self.doc.bridge.shared.context.clone()
    }

    /// The tab whose document host is `host`, if it is open in this window.
    pub fn tab_for_host(&self, host: onecad_app::process::HostId) -> Option<TabId> {
        let owns = |doc: &DocumentEditor| doc.bridge.shared.host.load(Ordering::Acquire) == host;
        std::iter::once(&self.doc)
            .chain(&self.tabs.parked)
            .find(|doc| owns(doc))
            .map(|doc| doc.tab)
    }

    /// Closes a tab opened for a New/Open/Import that produced no document, and
    /// returns to `focus`'s tab (an already-open document) or the tab it came from.
    pub(crate) fn settle_provisional_tab(
        &mut self,
        focus: Option<onecad_app::process::HostId>,
        diagnostic: Option<(crate::diagnostics::FileAction, crate::diagnostics::State)>,
    ) {
        let Some(origin) = self.doc.origin else {
            return;
        };
        if self.document_open() {
            return;
        }
        if self.parked {
            // The user left before it answered. With no document it has no tab
            // to return to, so its host closes once the parked drain ends
            // (a parked editor cannot be swapped out mid-drain) and its outcome
            // is reported where the user is; an already-open file is not focused.
            self.doc.origin = None;
            self.tabs.settled.push(Settled {
                tab: self.doc.tab,
                diagnostic,
            });
            return;
        }
        let status = std::mem::take(&mut self.doc.status);
        if let Err(error) = self.close_active_tab(None) {
            self.doc.status = error;
            return;
        }
        let target = focus
            .and_then(|host| self.tab_for_host(host))
            .unwrap_or(origin);
        self.activate_tab(target);
        self.doc.status = status;
    }

    /// ⌘T: Home in focus, to start or open the next document. Nothing is created
    /// until the user picks something there.
    pub fn open_home(&mut self) {
        if crate::bars::home_allowed(self) {
            self.ui.start_page = crate::ui_state::StartPage::Home;
            self.ui.start_open = true;
        }
    }

    /// The tab holding the file at `path` (the file the user opened, even when
    /// the host works on an isolated copy), if it is open in this window.
    pub fn tab_for_path(&self, path: &std::path::Path) -> Option<TabId> {
        let host = self
            .tabs
            .runtime
            .services()
            .reservations()
            .path_owner(path)?;
        self.tab_for_host(host)
    }

    /// A tab's close button: the document is shown, then closed (asking first
    /// when it holds unsaved work, so the prompt sits on its own tab).
    pub fn close_tab_from_strip(&mut self, tab: TabId) {
        if self.busy() {
            return;
        }
        let home = self.ui.start_open;
        self.show_tab(tab);
        if self.doc.tab != tab {
            return;
        }
        let decide = self.doc.needs_close_decision();
        self.request_close_tab();
        // A clean document closes at once; Home stays where the user was.
        if home && !decide && self.document_open() {
            self.ui.start_open = true;
        }
    }

    /// Leaves Home for `tab`.
    pub fn show_tab(&mut self, tab: TabId) {
        if tab != self.doc.tab {
            self.activate_tab(tab);
        }
        if self.doc.tab == tab {
            self.ui.start_open = false;
            self.draw_dirty = true;
        }
    }

    /// The editor of `tab`, active or parked.
    pub fn editor(&self, tab: TabId) -> Option<&DocumentEditor> {
        std::iter::once(&self.doc)
            .chain(&self.tabs.parked)
            .find(|doc| doc.tab == tab)
    }

    /// Open documents, most recently active first (⌃Tab order; Home goes last).
    /// A tab still waiting for its first document is not a destination.
    pub fn recent_tab_order(&self) -> Vec<TabId> {
        let mut docs: Vec<&DocumentEditor> = std::iter::once(&self.doc)
            .chain(&self.tabs.parked)
            .filter(|doc| doc.document_state.is_some())
            .collect();
        docs.sort_by_key(|doc| std::cmp::Reverse(doc.last_active));
        docs.into_iter().map(|doc| doc.tab).collect()
    }

    /// Asks the backend what the close prompt should list for the active document.
    pub(crate) fn describe_changes(&mut self) {
        self.doc.close_changes = None;
        if let Err(error) = self
            .doc
            .bridge
            .submit(crate::bridge::Command::DescribeChanges)
        {
            tracing::debug!(%error, "close prompt change list unavailable");
        }
    }
}

/// The close prompt's body: "3 changes since last save: Mate 4, Gear pair, …".
pub fn changes_text(changes: Option<&onecad_app::document_runtime::UnsavedChanges>) -> String {
    const LISTED: usize = 3;
    let Some(changes) = changes.filter(|changes| changes.count > 0) else {
        return "Your unsaved changes will be lost if you don't save them.".into();
    };
    let steps = if changes.count == 1 {
        "change"
    } else {
        "changes"
    };
    match &changes.labels {
        Some(labels) if !labels.is_empty() => {
            let shown: Vec<&str> = labels
                .iter()
                .rev()
                .take(LISTED)
                .map(String::as_str)
                .collect();
            let more = if labels.len() > LISTED { ", …" } else { "" };
            format!(
                "{} {steps} since last save: {}{more}.",
                changes.count,
                shown.join(", ")
            )
        }
        _ => format!("{} {steps} since last save.", changes.count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_app::document_runtime::UnsavedChanges;

    #[test]
    fn the_close_prompt_names_the_latest_changes_first() {
        let changes = UnsavedChanges {
            labels: Some(vec![
                "Sketch 1".into(),
                "Mate 4".into(),
                "Gear pair".into(),
                "Housing offset".into(),
            ]),
            count: 4,
        };
        assert_eq!(
            changes_text(Some(&changes)),
            "4 changes since last save: Housing offset, Gear pair, Mate 4, …."
        );
        let one = UnsavedChanges {
            labels: Some(vec!["Fillet 2".into()]),
            count: 1,
        };
        assert_eq!(
            changes_text(Some(&one)),
            "1 change since last save: Fillet 2."
        );
        let undone = UnsavedChanges {
            labels: None,
            count: 2,
        };
        assert_eq!(changes_text(Some(&undone)), "2 changes since last save.");
    }
}
