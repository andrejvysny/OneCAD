//! g01 Actions: Button, ToolButton, ToolPill, SelectionLabel.

use egui::{Ui, vec2};
use onecad_widgets::Theme;
use onecad_widgets::components::actions::{
    PillAccent, PillMode, SelectionLabel, SelectionVariant, ToolButton, ToolPill, ToolState,
    pill_divider,
};
use onecad_widgets::components::button::{Button, ButtonKind};

use super::g_shell::{heading, note, stage};

pub fn show(ui: &mut Ui, t: &Theme, _dark: bool) {
    buttons(ui, t);
    tool_buttons(ui, t);
    pills(ui, t);
    labels(ui, t);
}

/// Component with a small caption underneath.
fn captioned(ui: &mut Ui, t: &Theme, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        add(ui);
        note(ui, t, label);
    });
}

fn buttons(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "Button",
        "c-button",
        "Text actions. One primary per surface. Sentence case.",
    );
    let kinds = [
        ("primary", ButtonKind::Primary, "Finish sketch"),
        ("secondary", ButtonKind::Secondary, "Cancel"),
        ("ghost", ButtonKind::Ghost, "Roll to end"),
        ("danger", ButtonKind::Danger, "Delete"),
    ];
    stage(ui, t, vec2(840.0, 160.0), |ui| {
        for with_icon in [false, true] {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                for (name, kind, text) in kinds {
                    let icon = with_icon.then_some("check");
                    captioned(ui, t, name, |ui| {
                        ui.add(Button::new(text).kind(kind).icon(icon));
                    });
                }
                for (name, kind, text) in kinds.iter().take(2) {
                    let icon = with_icon.then_some("check");
                    let label = format!("{name} disabled");
                    captioned(ui, t, &label, |ui| {
                        ui.add(Button::new(text).kind(*kind).icon(icon).enabled(false));
                    });
                }
            });
            ui.add_space(14.0);
        }
    });
    note(
        ui,
        t,
        "h 30 · r 6 · px 12 · 12.5/400 (primary 600) · icon 14 · disabled 40%",
    );
}

fn tool_buttons(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "ToolButton",
        "c-toolbtn",
        "Icon-only tool in a pill. Tooltip carries name and shortcut.",
    );
    let states = [
        ("default", ToolState::Default, true),
        ("active tool", ToolState::Active, true),
        ("running", ToolState::Running, true),
        ("sketch active", ToolState::SketchActive, true),
        ("disabled", ToolState::Default, false),
    ];
    stage(ui, t, vec2(560.0, 112.0), |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            for (label, state, enabled) in states {
                captioned(ui, t, label, |ui| {
                    ui.add(ToolButton::new("extrude").state(state).enabled(enabled));
                });
            }
        });
    });
    note(
        ui,
        t,
        "34×34 · r 8 · icon 24 · running = orange (creates geometry)",
    );
}

fn pill_tools(ui: &mut Ui, icons: &[&str], split: usize) {
    for (i, icon) in icons.iter().enumerate() {
        if i == split {
            pill_divider(ui);
        }
        ui.add(ToolButton::new(icon));
    }
}

fn pills(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "ToolPill",
        "c-pill",
        "The one toolbar. Content follows selection: nothing → create/view; selection → only applicable actions behind a SelectionLabel.",
    );
    let specs = [
        ("idle", PillMode::Model, PillAccent::None),
        ("selection", PillMode::Model, PillAccent::Selection),
        ("sketch", PillMode::Sketch, PillAccent::None),
        (
            "sketch selection",
            PillMode::Sketch,
            PillAccent::SketchSelection,
        ),
        ("running", PillMode::Model, PillAccent::Running),
    ];
    stage(ui, t, vec2(840.0, 420.0), |ui| {
        for (label, mode, accent) in specs {
            captioned(ui, t, label, |ui| {
                ToolPill::new().mode(mode).accent(accent).show(ui, |ui| {
                    if accent == PillAccent::Selection {
                        SelectionLabel::new("Face", "Bracket · planar selection").show(ui);
                        pill_divider(ui);
                    }
                    pill_tools(ui, &["sketch", "extrude", "revolve", "hole", "fillet"], 1);
                });
            });
            ui.add_space(8.0);
        }
    });
    note(
        ui,
        t,
        "pad 4 · gap 2 · r 12 · centred over free viewport · tint by mode",
    );
}

fn labels(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "SelectionLabel",
        "c-sellabel",
        "Head of the pill when something is picked. × clears selection (same as Esc).",
    );
    let specs = [
        ("Face", "Bracket · planar face", SelectionVariant::Model),
        ("Edge", "80.00 mm edge", SelectionVariant::Model),
        ("Line", "30.00 mm sketch line", SelectionVariant::Sketch),
    ];
    stage(ui, t, vec2(560.0, 170.0), |ui| {
        for (kind, context, variant) in specs {
            let out = SelectionLabel::new(kind, context).variant(variant).show(ui);
            if out.clear {
                ui.ctx().request_repaint();
            }
            ui.add_space(8.0);
        }
    });
    note(
        ui,
        t,
        "h 34 · r 8 · type 12.5/600 + 12 secondary · sel-bg / orange in sketch",
    );
}
