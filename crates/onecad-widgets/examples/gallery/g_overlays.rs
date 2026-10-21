//! g03 Viewport overlays: ViewCluster, NavPill, SessionStrip, ValueTag + HUD, FeatureCard.

use egui::{Id, RichText, Ui, vec2};
use onecad_widgets::components::button::ButtonKind;
use onecad_widgets::components::overlays::{
    FeatureCard, NavButton, NavPill, SessionStrip, SquareButton, StripAction, StripVariant,
    ValueTag, ValueTagTone, ViewCluster,
};
use onecad_widgets::components::tooltip::Tip;
use onecad_widgets::icon::{self, IconStyle};
use onecad_widgets::{Theme, fonts};

use super::g_shell::{flag, heading, note, set_flag, stage};

pub fn show(ui: &mut Ui, t: &Theme, dark: bool) {
    view(ui, t, dark);
    nav(ui, t);
    strip(ui, t);
    valtag(ui, t);
    card(ui, t);
    super::g_tabs::switcher(ui, t);
}

fn view(ui: &mut Ui, t: &Theme, dark: bool) {
    heading(
        ui,
        t,
        "ViewCluster",
        "c-view",
        "Top-right: view cube, display mode, grid (toggle), snap. Moves left of the Inspector.",
    );
    stage(ui, t, vec2(200.0, 270.0), |ui| {
        let grid = flag(ui, "g03-grid", true);
        let buttons = [
            SquareButton {
                icon: "shadedEdges",
                on: false,
                enabled: true,
                tip: Tip::new("Display mode"),
            },
            SquareButton {
                icon: "grid",
                on: grid,
                enabled: true,
                tip: Tip::new("Grid").shortcut(Some("G")),
            },
            SquareButton {
                icon: "snap",
                on: false,
                enabled: false,
                tip: Tip::new("Snap").reason(Some("Needs a sketch")),
            },
        ];
        let out = ViewCluster::new(&buttons).show(ui, |ui, rect, _| {
            let style = IconStyle {
                tint: t.ink4,
                bg: t.canvas_model,
                grey: false,
            };
            icon::paint(ui.painter(), rect.shrink(4.0), "viewIso", dark, style, 1.0);
        });
        if out.clicked == Some(1) {
            set_flag(ui, "g03-grid", !grid);
        }
    });
    note(ui, t, "cube 72 · buttons 36 · r 8 · gap 8");
}

fn nav(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "NavPill",
        "c-nav",
        "Bottom-left: home, fit, isolate, section, layers. Isolate disabled without a body selection.",
    );
    stage(ui, t, vec2(260.0, 80.0), |ui| {
        let spec = [
            ("home", "Home", true),
            ("fit", "Fit", true),
            ("isolate", "Isolate", false),
            ("section", "Section", true),
            ("layers", "Layers", true),
        ];
        let layers = flag(ui, "g03-layers", false);
        let buttons: Vec<NavButton> = spec
            .iter()
            .map(|(icon, name, enabled)| NavButton {
                icon,
                enabled: *enabled,
                active: *icon == "layers" && layers,
                tip: Tip::new(name).reason((!*enabled).then_some("Select a body first")),
            })
            .collect();
        if NavPill::new(&buttons).show(ui) == Some(4) {
            set_flag(ui, "g03-layers", !layers);
        }
    });
    note(ui, t, "buttons 32 · icon 20 · pad 3 · r 8");
}

fn strip(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "SessionStrip",
        "c-strip",
        "Mode banner under the pill while a session is open (sketch, rollback). Carries status + exit actions.",
    );
    stage(ui, t, vec2(560.0, 130.0), |ui| {
        let sketch = [
            StripAction {
                label: "Cancel",
                kind: ButtonKind::Secondary,
                enabled: true,
                tip: Some(Tip::new("Discard sketch edits")),
            },
            StripAction {
                label: "Finish",
                kind: ButtonKind::Primary,
                enabled: true,
                tip: None,
            },
        ];
        SessionStrip::new(StripVariant::Sketch)
            .icon(Some("sketch"))
            .title("Sketch 3")
            .status(Some(("3 DOF", t.warn, Some("underConstrained"))))
            .actions(&sketch)
            .show(ui);
        ui.add_space(10.0);
        let rolled = [StripAction {
            label: "Roll to end",
            kind: ButtonKind::Ghost,
            enabled: true,
            tip: None,
        }];
        SessionStrip::new(StripVariant::Neutral)
            .icon(Some("rolledBack"))
            .title("Rolled back")
            .subtitle(Some("to Hole 2"))
            .actions(&rolled)
            .show(ui);
    });
    note(ui, t, "h 38 · r 10 · sketch = sketch-chrome bg");
}

fn valtag(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "ValueTag + HUD",
        "c-valtag",
        "Live value at a drag handle. HUD adds the operation and confirm.",
    );
    stage(ui, t, vec2(360.0, 170.0), |ui| {
        ui.add(ValueTag::new("14.00 mm"));
        ui.add_space(8.0);
        ui.add(ValueTag::new("-2.50 mm").tone(ValueTagTone::Modify));
        ui.add_space(12.0);
        ui.add(ValueTag::new("14.00 mm").hud("union"));
    });
    note(ui, t, "tag h 26 r 6 · 1.5px orange · mono 12/600");
}

fn card(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "FeatureCard",
        "c-card",
        "Parameters for the running feature, placed beside its handle. Header · grid of fields · Cancel / OK.",
    );
    stage(ui, t, vec2(330.0, 280.0), |ui| {
        let op = ui
            .ctx()
            .data(|d| d.get_temp::<usize>(Id::unique("g03-op")))
            .unwrap_or(0);
        let out = FeatureCard::new("g03-extrude", "extrude", "Extrude")
            .shortcut(Some("E"))
            .show(ui, |rows| {
                rows.row("Distance", |ui| {
                    ui.label(
                        RichText::new("14.00 mm")
                            .size(12.5)
                            .family(fonts::mono())
                            .color(t.ink2),
                    );
                });
                rows.row("Operation", |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (i, name) in ["union", "subtract", "intersect", "body"]
                        .into_iter()
                        .enumerate()
                    {
                        let sel = i == op;
                        let b = egui::Button::new(RichText::new(name).size(11.0)).selected(sel);
                        if ui.add(b).clicked() {
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(Id::unique("g03-op"), i));
                        }
                    }
                });
                rows.caption("Drag the handle or type a value.");
            });
        if out.ok || out.cancel || out.close {
            ui.ctx().request_repaint();
        }
    });
    note(
        ui,
        t,
        "w 280 · r 12 · header 44 · label col 76 · row gap 10 · card elevation",
    );
}
