//! In-editor library, placement and extension contributions (Final 11–12, Extensions 19–21):
//! UI facade only. Nothing here touches the document, bridge or worker.
mod extensions;
mod library;
mod notice;
mod placement;

use crate::{app::App, layout::Chrome, tool_registry::ToolId, ui_state::StartPage};

pub use extensions::footer;
pub use library::{list as library_list, mode_switch};

/// Mock placement of a library component; numbers stay text so NumberFields can edit them.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub name: String,
    pub linked: bool,
    pub x: String,
    pub y: String,
    pub z: String,
    pub rot: String,
    pub orient: bool,
    pub concentric: bool,
    pub snap: bool,
}

impl Placement {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            linked: true,
            x: "42".into(),
            y: "18".into(),
            z: "8".into(),
            rot: "90".into(),
            orient: true,
            concentric: false,
            snap: false,
        }
    }
}

/// Facade state for the editor-side library and extension UI.
#[derive(Clone, Debug, Default)]
pub struct EditorFacade {
    /// Explorer shows the library list instead of the model tree.
    pub library_mode: bool,
    pub search: String,
    /// Index into `library::SOURCES`.
    pub source: usize,
    /// "Socket head cap screw" row shows its size sub-rows.
    pub expanded: bool,
    pub placing: Option<Placement>,
    /// 19b popover above the Explorer footer.
    pub popover_open: bool,
    /// Extension tool shown active (visual only).
    pub active_tool: Option<ToolId>,
    /// 21b: Topology Check has "crashed".
    pub topology_failed: bool,
    pub log_copied: bool,
    /// Inspector "Convert to sheet metal…" pressed.
    pub converted: bool,
}

impl EditorFacade {
    pub fn begin_placement(&mut self, name: &str) {
        self.placing = Some(Placement::new(name));
    }

    /// Facade for the pill's extension tools; Check geometry simulates the crash.
    pub fn press_extension_tool(&mut self, id: ToolId) {
        match id {
            ToolId::CheckGeometry => self.topology_failed = true,
            _ => self.active_tool = (self.active_tool != Some(id)).then_some(id),
        }
    }

    pub fn dismiss_failure(&mut self) {
        self.topology_failed = false;
        self.log_copied = false;
    }
}

/// Placement hijacks the pill, so it must not start over a sketch or a running tool.
pub fn placement_allowed(app: &App) -> bool {
    !app.sketch_active() && app.interaction.tool.is_none()
}

/// Opens a Start-screen page; ignored while a command is in progress like the Home button.
pub(crate) fn open_start(app: &mut App, page: StartPage) {
    if !crate::bars::home_allowed(app) {
        return;
    }
    if page == StartPage::Library {
        crate::library_pages::go_library(&mut app.ui, 0);
    } else {
        app.ui.start_page = page;
    }
    app.ui.start_open = true;
}

/// Pill entry point for Insert component: the Explorer lists the library.
pub fn show_library(app: &mut App) {
    app.ui.editor_facade.library_mode = true;
    app.ui.explorer_open = true;
}

pub use placement::pill as placement_pill;

/// Floating facade surfaces drawn after the pill.
pub fn overlays(app: &mut App, ctx: &egui::Context, chrome: &Chrome) {
    extensions::popover(app, ctx, chrome);
    placement::overlay(app, ctx, chrome);
    notice::show(app, ctx, chrome);
}

/// Escape/Enter end placement and ⇧L opens the library page; handled before the model's
/// own Escape so ending placement never also clears the selection.
pub fn keys(app: &mut App, raw: &mut egui::RawInput, text: bool) {
    if text {
        return;
    }
    if process_keys(&mut app.ui.editor_facade, &mut raw.events) {
        open_start(app, StartPage::Library);
    }
}

fn process_keys(f: &mut EditorFacade, events: &mut Vec<egui::Event>) -> bool {
    let mut library = false;
    events.retain(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            return true;
        };
        let escape = *key == egui::Key::Escape;
        if escape && f.popover_open {
            f.popover_open = false;
            return false;
        }
        let ends = escape || (*key == egui::Key::Enter && !modifiers.command);
        if ends && f.placing.is_some() {
            f.placing = None;
            return false;
        }
        if *key == egui::Key::L && modifiers.shift && !modifiers.command && !modifiers.alt {
            library = true;
            return false;
        }
        true
    });
    library
}

#[cfg(test)]
mod tests;
