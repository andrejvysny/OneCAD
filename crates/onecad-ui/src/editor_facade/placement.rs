//! Placement mode (Final 11b): pill, numeric card and decorative gizmo. No document change.
use crate::{app::App, layout::Chrome};
use egui::{Align2, Color32, Id, Order, Pos2, Stroke, Ui, pos2, vec2};
use onecad_widgets::{
    components::{
        actions::{
            PillAccent, SelectionLabel, SelectionVariant, ToolButton, ToolPill, ToolState,
            pill_divider,
        },
        button::{Button, ButtonKind},
        inputs::NumberField,
        lists::{Segment, SegmentedControl},
        tooltip::Tip,
    },
    fonts, frame,
    metrics::Elevation,
    theme::look,
};

const CARD_W: f32 = 244.0;

/// Placement pill; replaces the selection pill while a component is being placed.
pub fn pill(app: &mut App, ctx: &egui::Context, chrome: &Chrome) {
    let Some(mut p) = app.ui.editor_facade.placing.take() else {
        return;
    };
    let (mut done, mut cancel) = (false, false);
    let pos = pos2(chrome.pill_centre_x, chrome.viewport.top() + 10.0);
    frame::floating(Id::unique("placement-pill"), pos, Order::Middle)
        .pivot(Align2::CENTER_TOP)
        .show(ctx, |ui| {
            ToolPill::new()
                .accent(PillAccent::Selection)
                .show(ui, |ui| {
                    let label =
                        SelectionLabel::new("Placing", &p.name).variant(SelectionVariant::Model);
                    cancel = label.show(ui).clear;
                    pill_divider(ui);
                    controls(ui, &mut p);
                    pill_divider(ui);
                    let place = Button::new("Place ↵")
                        .kind(ButtonKind::Primary)
                        .icon(Some("check"));
                    done = ui.add(place).clicked();
                });
        });
    if !(done || cancel) {
        app.ui.editor_facade.placing = Some(p);
    }
}

fn controls(ui: &mut Ui, p: &mut super::Placement) {
    let mut mode = usize::from(!p.linked);
    let segments = [
        Segment {
            label: "Linked",
            icon: None,
        },
        Segment {
            label: "Copy",
            icon: None,
        },
    ];
    ui.add(SegmentedControl::new("placement-mode", &mut mode, &segments).width(122.0));
    p.linked = mode == 0;
    pill_divider(ui);
    let toggles = [
        ("viewIso", "Orient to face", &mut p.orient),
        ("component", "Snap concentric", &mut p.concentric),
        ("snap", "Snap", &mut p.snap),
    ];
    for (icon, tip, on) in toggles {
        let state = if *on {
            ToolState::Active
        } else {
            ToolState::Default
        };
        if ui
            .add(ToolButton::new(icon).state(state).tip(Tip::new(tip)))
            .clicked()
        {
            *on = !*on;
        }
    }
}

/// Numeric card and gizmo hint at the viewport centre while placing.
pub fn overlay(app: &mut App, ctx: &egui::Context, chrome: &Chrome) {
    if app.ui.editor_facade.placing.is_none() {
        return;
    }
    let centre = chrome.viewport.center();
    gizmo(ctx, centre);
    let right = if app.ui.inspector_open {
        chrome.inspector.left() - 10.0
    } else {
        chrome.viewport.right() - 10.0
    };
    let x = (centre.x + 110.0).min(right - CARD_W);
    let Some(p) = app.ui.editor_facade.placing.as_mut() else {
        return;
    };
    let l = look(ctx);
    frame::floating(
        Id::unique("placement-card"),
        pos2(x, centre.y + 20.0),
        Order::Middle,
    )
    .show(ctx, |ui| {
        frame::surface(&l.theme, l.dark, 12, Elevation::Card)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(CARD_W - 24.0);
                card(ui, p, &l.theme);
            });
    });
}

fn card(ui: &mut Ui, p: &mut super::Placement, t: &onecad_widgets::theme::Theme) {
    ui.spacing_mut().item_spacing = vec2(6.0, 8.0);
    ui.label(
        egui::RichText::new("Placement")
            .size(13.0)
            .family(fonts::semibold())
            .color(t.ink),
    );
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 12.0) / 3.0;
        for (id, text, unit) in [
            ("x", &mut p.x, "X"),
            ("y", &mut p.y, "Y"),
            ("z", &mut p.z, "Z"),
        ] {
            ui.add(
                NumberField::new(("placement", id), text)
                    .unit(unit)
                    .width(w),
            );
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Rotate Z").size(12.0).color(t.ink3));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                NumberField::new(("placement", "rot"), &mut p.rot)
                    .unit("°")
                    .width(80.0),
            );
        });
    });
    ui.label(
        egui::RichText::new("On Face 7 · Tab cycles fields")
            .size(11.5)
            .color(t.ink5),
    );
}

/// Decorative axes: nothing here is interactive or tied to a transform.
fn gizmo(ctx: &egui::Context, c: Pos2) {
    let t = look(ctx).theme;
    let area = frame::floating(
        Id::unique("placement-gizmo"),
        c - vec2(80.0, 80.0),
        Order::Middle,
    )
    .interactable(false);
    area.show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(160.0, 160.0), egui::Sense::hover());
        let o = rect.center();
        let p = ui.painter();
        p.circle_stroke(o, 35.0, Stroke::new(1.5, t.sel_border));
        let axes: [(Pos2, Color32); 3] = [
            (o + vec2(0.0, -62.0), t.axis_z),
            (o + vec2(62.0, 0.0), t.axis_x),
            (o + vec2(-45.0, 35.0), t.axis_y),
        ];
        for (end, col) in axes {
            p.line_segment([o, end], Stroke::new(2.0, col));
            p.circle_filled(end, 5.0, col);
        }
        p.circle_filled(o, 5.0, t.floating);
        p.circle_stroke(o, 4.5, Stroke::new(2.0, t.running_border));
    });
}
