//! Title-bar workspace switcher and its facade dialogs (Final 02a, Extensions 19c–19e).
//! Display-only: no document, bridge or renderer state is touched.
mod customize;
mod layers;

use crate::{app::App, bars, ui_state::StartPage};
use egui::{Context, Response};
use onecad_widgets::components::{
    lists::menu_frame,
    shell::{WorkspaceItem, workspace_menu_with},
};

pub use customize::dialog as customize_dialog;
pub use layers::popover as layers_popover;

/// Facade state for the workspace menu, Customize workspace dialog and Layers popover.
#[derive(Clone, Debug)]
pub struct ShellFacade {
    pub customize_open: bool,
    /// Sheet Metal, Gears & Threads, CNC Prep.
    pub extensions_on: [bool; 3],
    pub layers_open: bool,
    /// Grid, Sketches, Datums & origin, Tool reach, Topology warnings.
    pub layers_on: [bool; 5],
    /// Topology Check extension switched on from the Layers popover.
    pub topology_on: bool,
}

impl Default for ShellFacade {
    fn default() -> Self {
        Self {
            customize_open: false,
            extensions_on: [true; 3],
            layers_open: false,
            layers_on: [true, true, false, false, false],
            topology_on: false,
        }
    }
}

impl ShellFacade {
    /// "Reset Design workspace": every extension contribution back on.
    pub fn reset_extensions(&mut self) {
        self.extensions_on = [true; 3];
    }

    /// "Turn on" for Topology Check: its layer becomes visible.
    pub fn turn_on_topology(&mut self) {
        self.topology_on = true;
        self.layers_on[4] = true;
    }
}

/// "Get more workspaces…": opens the Extensions page of the Start screen.
fn open_extensions(ui: &mut crate::ui_state::UiState) {
    ui.start_open = true;
    ui.start_page = StartPage::Extensions;
}

fn item<'a>(
    label: &'a str,
    icon: &'a str,
    key: &'a str,
    subtitle: Option<&'a str>,
    live: bool,
) -> WorkspaceItem<'a> {
    WorkspaceItem {
        label,
        icon,
        shortcut: Some(key),
        subtitle,
        enabled: live,
        active: label == "Design",
    }
}

/// Workspace popover anchored under the title-bar button (19c).
pub fn workspace_menu(app: &mut App, ctx: &Context, anchor: &Response) {
    if !crate::editor_facade::PREVIEW_FACADES {
        return;
    }
    let items = [
        item("Design", "body", "⌃1", None, true),
        item("Assembly", "component", "⌃2", None, false),
        item("Simulation", "study", "⌃3", None, false),
        item("Drawing", "viewFront", "⌃4", None, false),
    ];
    let ext = [item("CNC", "panelLeft", "⌃5", Some("CNC Prep"), true)];
    let more_ok = bars::home_allowed(app);
    let (mut customize, mut more) = (false, false);
    egui::Popup::menu(anchor).frame(menu_frame(ctx)).show(|ui| {
        // Rows fill available width; without a cap the popup inherits the title bar's width.
        ui.set_max_width(232.0);
        let out = workspace_menu_with(ui, &items, &ext, "Get more workspaces…", more_ok);
        customize = out.customize;
        more = out.manage && more_ok;
    });
    let id = egui::Popup::default_response_id(anchor);
    app.ui.workspace_menu_open = egui::Popup::is_id_open(ctx, id);
    if customize {
        app.ui.shell_facade.customize_open = true;
    }
    if more {
        open_extensions(&mut app.ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_state::UiState;

    #[test]
    fn get_more_workspaces_opens_extensions_page() {
        let mut ui = UiState::default();
        open_extensions(&mut ui);
        assert!(ui.start_open);
        assert_eq!(ui.start_page, StartPage::Extensions);
    }

    #[test]
    fn reset_turns_every_extension_on() {
        let mut f = ShellFacade {
            extensions_on: [false, true, false],
            ..ShellFacade::default()
        };
        f.reset_extensions();
        assert_eq!(f.extensions_on, [true; 3]);
    }

    #[test]
    fn topology_turn_on_shows_its_layer() {
        let mut f = ShellFacade::default();
        assert!(!f.layers_on[4] && !f.topology_on);
        f.turn_on_topology();
        assert!(f.layers_on[4] && f.topology_on);
    }
}
