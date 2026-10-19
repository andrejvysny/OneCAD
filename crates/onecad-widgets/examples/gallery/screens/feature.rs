//! Running-command screens: 05a Extrude and 06a Fillet / Chamfer (pill + card + value tag).

use egui::{Pos2, Rect, Stroke, Ui, pos2, vec2};
use onecad_widgets::Theme;
use onecad_widgets::components::actions::{PillAccent, PillMode, SelectionVariant, ToolState};
use onecad_widgets::components::inputs::{
    AddChip, NumberField, OptionIconGroup, ReferenceChip, Select, Toggle,
};
use onecad_widgets::components::lists::{Segment, SegmentedControl};
use onecad_widgets::components::overlays::{FeatureCard, ValueTag};
use onecad_widgets::components::shell::StatusItem;
use onecad_widgets::components::tooltip::Tip;

use super::chrome::{self, Base, Cx, PillSpec, Status, Tool};
use super::explorer::ExplorerMode;

fn base<'a>(explorer: ExplorerMode, status: Status<'a>) -> Base<'a> {
    Base {
        explorer: Some(explorer),
        inspector: None,
        sketch: false,
        dot: false,
        isolate: true,
        status,
    }
}

fn status<'a>(
    ms: &'a str,
    hint: &'a str,
    items: &'a [StatusItem<'a>],
    cx_ok: egui::Color32,
) -> Status<'a> {
    Status {
        sketch: false,
        dot: Some(cx_ok),
        text: ms,
        hint: Some(hint),
        persp: None,
        items,
    }
}

/// Card position relative to the viewport's top-right corner.
fn card_origin(cx: &Cx, right: f32, top: f32, width: f32) -> Pos2 {
    pos2(
        cx.lay.viewport.right() - right - width,
        cx.lay.viewport.top() + top,
    )
}

pub fn s05a(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let items = [StatusItem {
        label: Some("Z"),
        value: "22.00",
        mono: true,
    }];
    let st = status(
        "Preview ready · 38 ms",
        "Drag arrow · Tab cycles fields · ↵ OK · Esc cancel",
        &items,
        t.dof_ok,
    );
    let b = base(ExplorerMode::Region, st);
    chrome::screen(ui, t, dark, stage, &b, |ui, cx| {
        let spec = PillSpec {
            mode: PillMode::Model,
            accent: PillAccent::Selection,
            label: Some(("Region", "Sketch 3", SelectionVariant::Model)),
            groups: &[
                &[
                    Tool::new("extrude", "Extrude")
                        .key("E")
                        .state(ToolState::Running),
                    Tool::new("revolve", "Revolve"),
                    Tool::new("sweep", "Sweep"),
                    Tool::new("loft", "Loft"),
                ],
                &[
                    Tool::new("measure", "Measure"),
                    Tool::new("overflow", "More"),
                ],
            ],
        };
        chrome::pill(ui, cx, &spec);
        extrude_preview(ui, cx);
        let at = card_origin(cx, 139.0, 166.0, 280.0);
        chrome::at(ui, Rect::from_min_size(at, vec2(280.0, 400.0)), |ui| {
            extrude_card(ui)
        });
    });
}

/// Orange drag arrow + knob with the live value tag.
fn extrude_preview(ui: &mut Ui, cx: &Cx) {
    let o = cx.lay.stage.min.to_vec2();
    let (foot, tip) = (pos2(695.0, 468.0) + o, pos2(695.0, 400.0) + o);
    let p = ui.painter();
    p.line_segment([foot, tip], Stroke::new(2.0, cx.t.created_stroke));
    p.circle_filled(tip, 5.0, cx.t.created_stroke);
    p.circle_filled(foot, 8.0, cx.t.surface);
    p.circle_stroke(foot, 8.0, Stroke::new(1.5, cx.t.created_stroke));
    let tag = Rect::from_min_size(pos2(713.0, 335.0) + o, vec2(80.0, 30.0));
    chrome::at(ui, tag, |ui| {
        ui.add(ValueTag::new("14.00 mm"));
    });
}

fn extrude_card(ui: &mut Ui) {
    let (mut dir, mut op) = (0usize, 0usize);
    let (mut dist, mut draft) = ("14.00".to_owned(), "0.0".to_owned());
    let dirs = [
        Segment {
            label: "One side",
            icon: None,
        },
        Segment {
            label: "Symmetric",
            icon: None,
        },
        Segment {
            label: "Two",
            icon: None,
        },
    ];
    let ops = [
        ("union", Tip::new("Join")),
        ("subtract", Tip::new("Cut")),
        ("intersect", Tip::new("Intersect")),
        ("body", Tip::new("New body")),
    ];
    FeatureCard::new("screens-extrude", "extrude", "Extrude")
        .shortcut(Some("E"))
        .show(ui, |rows| {
            rows.row("Profile", |ui| {
                ReferenceChip::new("Sketch 3 · region 1")
                    .icon(Some("sketchEntity"))
                    .removable(true)
                    .show(ui);
            });
            rows.row("Direction", |ui| {
                ui.add(
                    SegmentedControl::new("screens-dir", &mut dir, &dirs)
                        .width(ui.available_width()),
                );
            });
            rows.row("Extent", |ui| {
                let opts = ["Distance", "Through all", "Up to face"];
                Select::new("screens-extent", opts[0], &opts)
                    .width(ui.available_width())
                    .show(ui);
            });
            rows.row("Distance", |ui| {
                focused_number(ui, "screens-dist", &mut dist, "mm")
            });
            rows.row("Operation", |ui| {
                ui.add(OptionIconGroup::new(&mut op, &ops));
            });
            rows.caption("Join with Bracket");
            rows.row("Draft", |ui| {
                ui.add(
                    NumberField::new("screens-draft", &mut draft)
                        .unit("°")
                        .width(ui.available_width()),
                );
            });
        });
}

/// NumberField that always shows the focus ring (the design's focused field).
fn focused_number(ui: &mut Ui, id: &str, text: &mut String, unit: &str) {
    let r = ui.add(
        NumberField::new(id, text)
            .unit(unit)
            .width(ui.available_width()),
    );
    r.request_focus();
}

pub fn s06a(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let items = [StatusItem {
        label: Some("Edge"),
        value: "80.00",
        mono: true,
    }];
    let st = status(
        "Preview ready · 61 ms",
        "Drag outward to round · inward to chamfer",
        &items,
        t.dof_ok,
    );
    let b = base(ExplorerMode::Picked("1 edge"), st);
    chrome::screen(ui, t, dark, stage, &b, |ui, cx| {
        let spec = PillSpec {
            mode: PillMode::Model,
            accent: PillAccent::Selection,
            label: Some(("Edge", "Bracket · 80.00 mm", SelectionVariant::Model)),
            groups: &[
                &[
                    Tool::new("fillet", "Fillet")
                        .key("F")
                        .state(ToolState::Running),
                    Tool::new("chamfer", "Chamfer"),
                ],
                &[
                    Tool::new("measure", "Measure"),
                    Tool::new("isolate", "Isolate"),
                    Tool::new("overflow", "More"),
                ],
            ],
        };
        chrome::pill(ui, cx, &spec);
        fillet_preview(ui, cx);
        let at = card_origin(cx, 105.0, 123.0, 276.0);
        chrome::at(ui, Rect::from_min_size(at, vec2(276.0, 300.0)), |ui| {
            fillet_card(ui)
        });
    });
}

/// Orange strip along the picked edge with its "R 6.00" tag.
fn fillet_preview(ui: &mut Ui, cx: &Cx) {
    let o = cx.lay.stage.min.to_vec2();
    let pts = [
        (577.0, 389.0),
        (607.0, 340.0),
        (1002.0, 522.0),
        (972.0, 572.0),
    ];
    let pts = pts.iter().map(|&(x, y)| pos2(x, y) + o).collect();
    let fill = cx.t.created_fill.gamma_multiply(0.5);
    let shape =
        egui::epaint::PathShape::convex_polygon(pts, fill, Stroke::new(2.5, cx.t.created_stroke));
    ui.painter().add(shape);
    let tag = Rect::from_min_size(pos2(773.0, 449.0) + o, vec2(80.0, 30.0));
    chrome::at(ui, tag, |ui| {
        ui.add(ValueTag::new("R 6.00"));
    });
}

fn fillet_card(ui: &mut Ui) {
    let (mut kind, mut tangent) = (0usize, true);
    let mut radius = "6.00".to_owned();
    let kinds = [
        Segment {
            label: "Fillet",
            icon: Some("fillet"),
        },
        Segment {
            label: "Chamfer",
            icon: Some("chamfer"),
        },
    ];
    FeatureCard::new("screens-fillet", "body", "Fillet")
        .width(276.0)
        .shortcut(Some("F"))
        .show(ui, |rows| {
            rows.row("Type", |ui| {
                ui.add(
                    SegmentedControl::new("screens-kind", &mut kind, &kinds)
                        .width(ui.available_width()),
                );
            });
            rows.row("Edges", |ui| {
                ReferenceChip::new("1 edge").removable(true).show(ui);
                ui.add(AddChip::new());
            });
            rows.row("Radius", |ui| {
                focused_number(ui, "screens-radius", &mut radius, "mm")
            });
            rows.row("Tangent chain", |ui| {
                ui.add(Toggle::new(&mut tangent));
            });
        });
}
