//! Sketch constraint applicability derives from the authoritative single selection.
use crate::{app::App, layout::Chrome, ui_state::UiState};
use egui::{Context, Order, Pos2, Rect, pos2};
use onecad_widgets::{
    components::lists::{MenuRow, MenuSection, menu_divider, menu_frame},
    frame,
};

const NOT_YET: &str = "Not available yet";
const WIDTH: f32 = 236.0;
/// Session strip top offset (62) + strip height (38) + gap (8).
const BELOW_STRIP: f32 = 62.0 + 38.0 + 8.0;

const GEOMETRIC: [(&str, &str); 12] = [
    ("coincident", "Coincident"),
    ("horizontal", "Horizontal"),
    ("vertical", "Vertical"),
    ("parallel", "Parallel"),
    ("perpendicular", "Perpendicular"),
    ("tangent", "Tangent"),
    ("concentric", "Concentric"),
    ("equal", "Equal"),
    ("midpoint", "Midpoint"),
    ("symmetric", "Symmetric"),
    ("pointOnCurve", "Point on curve"),
    ("fix", "Fix"),
];

const DIMENSIONAL: [(&str, &str); 3] = [
    ("distance", "Distance"),
    ("angle", "Angle"),
    ("radius", "Diameter"),
];

pub fn toggle(state: &mut UiState) {
    state.constraints_open = !state.constraints_open;
}

fn outside(menu: Rect, anchor: Option<Rect>, click: Pos2) -> bool {
    !menu.contains(click) && !anchor.is_some_and(|a| a.contains(click))
}

fn tool(icon: &str) -> Option<crate::tool_registry::ToolId> {
    use crate::tool_registry::ToolId;
    match icon {
        "horizontal" => Some(ToolId::Horizontal),
        "vertical" => Some(ToolId::Vertical),
        "fix" => Some(ToolId::Fix),
        "distance" => Some(ToolId::Distance),
        "radius" => Some(ToolId::Diameter),
        _ => None,
    }
}

fn rows(
    app: &App,
    ui: &mut egui::Ui,
    list: &[(&str, &str)],
) -> Option<crate::tool_registry::ToolId> {
    let mut clicked = None;
    for (icon, label) in list {
        let action = tool(icon);
        let enabled =
            action.is_some_and(|action| crate::constraint_authoring::available(app, action));
        let response = ui.add(MenuRow::new(Some(icon), label).enabled(enabled));
        if enabled && response.clicked() {
            clicked = action;
        } else if !enabled {
            response.on_hover_text(if action.is_some() {
                "Select one applicable sketch entity first"
            } else {
                NOT_YET
            });
        }
    }
    clicked
}

/// `anchor` is the Constraints pill button; the menu left-aligns to it.
pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome, anchor: Option<Rect>) {
    if !app.ui.constraints_open {
        return;
    }
    // The button only exists in the sketch-idle pill; hide the menu with it.
    let Some(button) = anchor.filter(|_| app.sketch_active()) else {
        app.ui.constraints_open = false;
        return;
    };
    let vp = chrome.viewport;
    let x = button
        .left()
        .min(vp.right() - WIDTH - 10.0)
        .max(vp.left() + 10.0);
    let pos = pos2(x, vp.top() + BELOW_STRIP);
    let mut clicked = None;
    let area =
        frame::floating(egui::Id::unique("constraints-menu"), pos, Order::Middle).show(ctx, |ui| {
            menu_frame(ctx).show(ui, |ui| {
                ui.set_width(WIDTH - 14.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(MenuSection::new("GEOMETRIC"));
                clicked = rows(app, ui, &GEOMETRIC);
                menu_divider(ui);
                ui.add(MenuSection::new("DIMENSIONAL"));
                clicked = rows(app, ui, &DIMENSIONAL).or(clicked);
            });
        });
    if let Some(tool) = clicked {
        crate::constraint_authoring::activate(app, ctx, tool);
    }
    let click = ctx.input(|i| {
        i.pointer
            .any_pressed()
            .then(|| i.pointer.interact_pos())
            .flatten()
    });
    if click.is_some_and(|p| outside(area.response.rect, Some(button), p)) {
        app.ui.constraints_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn toggle_flips() {
        let mut s = UiState::default();
        assert!(!s.constraints_open);
        toggle(&mut s);
        assert!(s.constraints_open);
        toggle(&mut s);
        assert!(!s.constraints_open);
    }

    #[test]
    fn click_outside_ignores_menu_and_button() {
        let menu = Rect::from_min_size(pos2(100.0, 100.0), vec2(236.0, 400.0));
        let button = Rect::from_min_size(pos2(100.0, 50.0), vec2(34.0, 34.0));
        assert!(!outside(menu, Some(button), pos2(120.0, 200.0)));
        assert!(!outside(menu, Some(button), pos2(110.0, 60.0)));
        assert!(outside(menu, Some(button), pos2(600.0, 600.0)));
        assert!(outside(menu, None, pos2(110.0, 60.0)));
    }

    #[test]
    fn every_icon_exists() {
        for (icon, _) in GEOMETRIC.iter().chain(DIMENSIONAL.iter()) {
            assert!(onecad_widgets::icon::exists(icon), "{icon}");
        }
    }
}
