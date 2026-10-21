//! Title-bar workspace switcher and its facade dialogs (Final 02a, Extensions 19c–19e).
//! Display-only: no document, bridge or renderer state is touched.
mod customize;
mod layers;

use crate::app::App;
use egui::{Context, Rect, Response};
use onecad_widgets::components::{
    lists::menu_frame,
    shell::{WorkspaceItem, workspace_menu_compact},
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

fn item<'a>(label: &'a str, icon: &'a str, key: &'a str, live: bool) -> WorkspaceItem<'a> {
    WorkspaceItem {
        label,
        icon,
        shortcut: Some(key),
        subtitle: None,
        enabled: live,
        active: label == "Design",
    }
}

/// Compact workspace popover below `bounds` (explorer header row or collapsed chip).
/// `anchor` owns the open/close toggle; the card matches `bounds` width (min 200).
pub fn workspace_menu(app: &mut App, ctx: &Context, anchor: &Response, bounds: Rect) {
    let live = crate::editor_facade::PREVIEW_FACADES;
    let items = [
        item("Design", "body", "⌃1", true),
        item("Assembly", "component", "⌃2", false),
        item("Simulation", "study", "⌃3", false),
        item("Drawing", "viewFront", "⌃4", false),
    ];
    let ext = [item("CNC", "panelLeft", "⌃5", live)];
    let hint =
        (!live).then_some("Native workspace integration is unavailable (ONECAD-301 / ONECAD-92)");
    let width = bounds.width().max(200.0);
    let frame = menu_frame(ctx).corner_radius(egui::CornerRadius::same(10));
    let mut customize = false;
    egui::Popup::menu(anchor)
        .anchor(bounds)
        .gap(4.0)
        .width(width)
        .frame(frame)
        .show(|ui| {
            // Frame margin is 6 per side.
            ui.set_width(width - 12.0);
            let out = workspace_menu_compact(ui, &items, &ext, ("Customize…", live), hint);
            customize = out.customize && live;
        });
    let id = egui::Popup::default_response_id(anchor);
    app.ui.workspace_menu_open = egui::Popup::is_id_open(ctx, id);
    if customize {
        app.ui.shell_facade.customize_open = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
