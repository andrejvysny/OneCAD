use crate::bridge::{Bridge, Display};
use onecad_core::ids::{BodyId, RecordId, SketchId};
use onecad_interaction::selection_set::{SelectionItem, SelectionSet};
use onecad_interaction::{Interaction, RequestTag};
use onecad_scene::{Camera, ViewSnapshot};
use std::sync::Arc;

/// Per-document editing state; movable as a unit so documents can be swapped.
pub struct DocumentEditor {
    pub documents: crate::document_flow::Flow,
    pub(crate) items: crate::document_items::Flow,
    pub(crate) selection_preview: crate::item_selection::Preview,
    pub bridge: Bridge,
    pub display: Option<Arc<Display>>,
    pub document_state: Option<onecad_app::dto::DocumentState>,
    pub document_state_serial: u64,
    pub interaction: Interaction,
    pub sketch: onecad_interaction::sketch::SketchInteraction,
    pub(crate) sketch_candidate: Option<crate::sketch_candidates::Pending>,
    pub sketch_gesture: onecad_interaction::sketch::gesture::SketchGestureInteraction,
    pub(crate) gesture_followup: Option<crate::sketch_gestures::Followup>,
    pub(crate) gesture_expected: Option<onecad_app::api::PreviewContext>,
    pub(crate) armed_sketch_gesture: Option<crate::sketch_gestures::Armed>,
    pub camera: Camera,
    pub navigation: crate::navigation::Navigation,
    pub transform_gizmo: crate::transform_ui::TransformGizmo,
    pub(crate) extrude_handle: crate::extrude_handle::ExtrudeHandle,
    pub(crate) model_handle: crate::model_handle::ModelHandle,
    /// Complete modeling camera restored when a sketch closes.
    pub(crate) modeling_camera: Option<onecad_scene::Camera>,
    pub topology: crate::viewport_selection::Selection,
    /// Last drawable capture; `camera` may already contain pending navigation.
    pub view: Option<ViewSnapshot>,
    pub(crate) view_presentation: Option<crate::viewport_capture::Presentation>,
    pub fit_pending: bool,
    /// Typed model selection shared by viewport, Explorer, History and pill.
    pub selection: SelectionSet,
    /// Present only while an active tool feeds operand stages from picks.
    pub(crate) operand_staging: Option<crate::model_pick::Staging>,
    /// Staging for a Fillet, Chamfer or Offset Face the user chose, in flight.
    pub(crate) operation_start: Option<crate::operation_start::Pending>,
    pub numeric_id: Option<egui::Id>,
    pub status: String,
    pub command_status: bool,
    pub authoring_dirty: bool,
    pub gesture: u64,
    pub pending_preview: Option<(RequestTag, crate::viewport_updates::Preview)>,
    pub sections: crate::viewport_updates::Sections,
    pub clear_preview: bool,
    pub(crate) received_preview: Option<crate::viewport_updates::Preview>,
    pub sketch_entry: crate::sketch_entry::Entry,
    /// Identity of this document's tab within the window.
    pub tab: crate::documents::TabId,
    /// Switched back in after its prepared scene was uploaded: the viewport shows
    /// nothing and picks nothing until a re-prepared scene of its own arrives.
    pub awaiting_scene: bool,
    /// Its service actor has stopped (the document host is closed).
    pub stopped: bool,
    /// The tab this one was opened from, while its New/Open/Import is in flight:
    /// a cancelled or redirected request closes this tab and returns there.
    pub origin: Option<crate::documents::TabId>,
    /// Scene and eviction generation of the preparation this document awaits.
    pub prepare_in_flight: Option<(onecad_scene::SceneIdentity, u64)>,
    /// A close the user already decided while this document was parked (a Save
    /// that finished in the background); applied when it is shown again. The
    /// inner value is the runtime session to discard, if the decision was discard.
    pub close_when_active: Option<Option<String>>,
    /// What the close prompt lists, once the backend described it.
    pub close_changes: Option<onecad_app::document_runtime::UnsavedChanges>,
    /// When this tab was last made active (most-recent-first ⌃Tab order).
    pub last_active: u64,
    /// Snapshot of its viewport taken when it was last left (⌃Tab switcher).
    pub thumbnail: Option<crate::documents::Thumbnail>,
    /// This document's operation outcomes (file, worker, viewport, profile).
    pub diagnostics: crate::diagnostics::Diagnostics,
}

impl DocumentEditor {
    pub fn new(bridge: Bridge, status: String) -> Result<Self, String> {
        Ok(Self {
            documents: Default::default(),
            items: Default::default(),
            selection_preview: Default::default(),
            bridge,
            display: None,
            document_state: None,
            document_state_serial: 0,
            interaction: Interaction::default(),
            sketch: Default::default(),
            sketch_candidate: None,
            sketch_gesture: Default::default(),
            gesture_followup: None,
            gesture_expected: None,
            armed_sketch_gesture: None,
            camera: Camera::default(),
            navigation: Default::default(),
            transform_gizmo: Default::default(),
            extrude_handle: Default::default(),
            model_handle: Default::default(),
            modeling_camera: None,
            topology: Default::default(),
            view: None,
            view_presentation: None,
            fit_pending: true,
            selection: SelectionSet::default(),
            operand_staging: None,
            operation_start: None,
            numeric_id: None,
            status,
            command_status: false,
            authoring_dirty: false,
            gesture: 0,
            pending_preview: None,
            sections: crate::viewport_updates::Sections::new()?,
            clear_preview: true,
            received_preview: None,
            sketch_entry: Default::default(),
            tab: crate::documents::next_tab_id(),
            awaiting_scene: false,
            stopped: false,
            origin: None,
            diagnostics: Default::default(),
            prepare_in_flight: None,
            close_when_active: None,
            close_changes: None,
            last_active: 0,
            thumbnail: None,
        })
    }

    /// Whether closing this document needs the user's save/discard decision.
    pub fn needs_close_decision(&self) -> bool {
        let dirty = self.authoring_dirty
            || self
                .document_state
                .as_ref()
                .is_some_and(|state| state.dirty);
        let sketching = self.sketch.session.as_ref().is_some_and(|session| {
            session.phase != onecad_interaction::sketch::SketchPhase::Closed
        });
        dirty || sketching || self.interaction.tool.is_some()
    }

    /// The one body every whole-body or topology item belongs to; a face pick
    /// therefore still names the body Move acts on.
    pub fn selected_body(&self) -> Option<BodyId> {
        self.selection.single_body()
    }

    pub fn selected_feature(&self) -> Option<RecordId> {
        match self.selection.items() {
            [SelectionItem::Feature(id)] => Some(*id),
            _ => None,
        }
    }

    /// A selected sketch row, or the sketch owning the one selected profile.
    pub fn selected_sketch(&self) -> Option<SketchId> {
        match self.selection.items() {
            [SelectionItem::Sketch(id)] => Some(*id),
            [SelectionItem::Profile(profile)] => Some(profile.profile.sketch),
            _ => None,
        }
    }

    /// The one selected face picked in `scene`, as its MESH1 label.
    pub fn selected_face(&self, scene: onecad_scene::SceneIdentity) -> Option<(BodyId, String)> {
        match self.selection.items() {
            [SelectionItem::Face(face)] if face.scene == scene => {
                Some((face.body, crate::model_pick::label(&face.id).to_owned()))
            }
            _ => None,
        }
    }

    /// A whole-body item with no face or edge of this body beside it.
    pub fn whole_body_selected(&self, id: BodyId) -> bool {
        self.selection.body_tinted(id)
    }

    /// Document title for window/tab chrome; "Untitled" before a document exists.
    pub fn editor_title(&self) -> String {
        self.document_state
            .as_ref()
            .map_or("Untitled", |state| state.title.as_str())
            .to_owned()
    }
}
