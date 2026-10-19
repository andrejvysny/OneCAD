//! Model-mode screens: 02a idle, 03a face selected, 03b panels hidden, 07a history.

use egui::{Rect, Stroke, Ui, pos2};
use onecad_widgets::Theme;
use onecad_widgets::components::actions::{PillAccent, PillMode, SelectionVariant, ToolState};
use onecad_widgets::components::button::ButtonKind;
use onecad_widgets::components::overlays::{StripAction, StripVariant};

use super::chrome::{self, Base, Cx, PillSpec, Status, Tool};
use super::explorer::ExplorerMode;
use super::inspector::InspectorView;

pub fn s02a(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let items = chrome::coords("42.10", "18.00", Some("8.00"));
    let base = Base {
        explorer: Some(ExplorerMode::Idle),
        inspector: None,
        sketch: false,
        dot: false,
        isolate: false,
        status: Status {
            sketch: false,
            dot: None,
            text: "Ready",
            hint: Some("1 body · 2 sketches"),
            persp: Some(true),
            items: &items,
        },
    };
    chrome::screen(ui, t, dark, stage, &base, |ui, cx| {
        let spec = PillSpec {
            mode: PillMode::Model,
            accent: PillAccent::None,
            label: None,
            groups: &[
                &[Tool::new("select", "Select")
                    .key("V")
                    .state(ToolState::Active)],
                &[
                    Tool::new("sketch", "Sketch").key("S"),
                    Tool::new("datum", "Datum plane"),
                    Tool::new("offsetPlane", "Offset plane"),
                ],
                &[
                    Tool::new("gear", "Generators — gear, spring, thread"),
                    Tool::new("component", "Insert component"),
                    Tool::new("import", "Import"),
                ],
                &[
                    Tool::new("measure", "Measure"),
                    Tool::new("section", "Section"),
                ],
            ],
        };
        chrome::pill(ui, cx, &spec);
    });
}

fn face_pill(ui: &mut Ui, cx: &Cx) {
    let spec = PillSpec {
        mode: PillMode::Model,
        accent: PillAccent::Selection,
        label: Some(("Face", "Bracket · planar", SelectionVariant::Model)),
        groups: &[
            &[
                Tool::new("sketch", "Sketch on face").key("S"),
                Tool::new("extrude", "Extrude").key("E"),
                Tool::new("offsetFace", "Offset face"),
                Tool::new("hole", "Hole"),
                Tool::new("draft", "Draft"),
                Tool::new("shell", "Shell"),
            ],
            &[
                Tool::new("material", "Material"),
                Tool::new("measure", "Measure"),
            ],
            &[
                Tool::new("isolate", "Isolate"),
                Tool::new("eyeOff", "Hide"),
                Tool::new("overflow", "More"),
            ],
        ],
    };
    chrome::pill(ui, cx, &spec);
}

/// Flat stand-in for the picked face (the real viewport is Three.js).
fn mock_face(ui: &Ui, cx: &Cx) {
    let o = cx.lay.stage.min.to_vec2();
    let pts = [
        (321.0, 469.0),
        (500.0, 387.0),
        (925.0, 557.0),
        (718.0, 654.0),
    ];
    let pts = pts.iter().map(|&(x, y)| pos2(x, y) + o).collect();
    let fill = cx.t.sel_border.gamma_multiply(0.9);
    let shape = egui::epaint::PathShape::convex_polygon(pts, fill, Stroke::new(2.0, cx.t.accent));
    ui.painter().add(shape);
}

fn face_base<'a>(
    items: &'a [onecad_widgets::components::shell::StatusItem<'a>],
    panels: bool,
) -> Base<'a> {
    Base {
        explorer: panels.then_some(ExplorerMode::Picked("1 face")),
        inspector: panels.then_some(InspectorView::Properties),
        sketch: false,
        dot: !panels,
        isolate: true,
        status: Status {
            sketch: false,
            dot: None,
            text: "1 face selected",
            hint: None,
            persp: Some(true),
            items,
        },
    }
}

pub fn s03a(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let items = chrome::coords("40.00", "30.00", Some("8.00"));
    chrome::screen(ui, t, dark, stage, &face_base(&items, true), |ui, cx| {
        mock_face(ui, cx);
        face_pill(ui, cx);
    });
}

pub fn s03b(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let items = chrome::coords("40.00", "30.00", Some("8.00"));
    chrome::screen(ui, t, dark, stage, &face_base(&items, false), |ui, cx| {
        mock_face(ui, cx);
        face_pill(ui, cx);
    });
}

pub fn s07a(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let base = Base {
        explorer: Some(ExplorerMode::Idle),
        inspector: Some(InspectorView::History),
        sketch: false,
        dot: false,
        isolate: false,
        status: Status {
            sketch: false,
            dot: None,
            text: "Rolled back · 1 feature hidden",
            hint: None,
            persp: Some(true),
            items: &[],
        },
    };
    chrome::screen(ui, t, dark, stage, &base, |ui, cx| {
        history_pill(ui, cx);
        let roll = [StripAction {
            label: "Roll to end",
            kind: ButtonKind::Secondary,
            enabled: true,
            tip: None,
        }];
        let head = (
            "rolledBack",
            "Rolled back",
            Some("to Hole 2 — new features insert here"),
        );
        chrome::strip(ui, cx, StripVariant::Neutral, head, None, &roll);
    });
}

fn history_pill(ui: &mut Ui, cx: &Cx) {
    let spec = PillSpec {
        mode: PillMode::Model,
        accent: PillAccent::Selection,
        label: Some(("Extrude 2", "feature", SelectionVariant::Model)),
        groups: &[
            &[
                Tool::new("edit", "Edit"),
                Tool::new("rolledBack", "Roll back to here"),
                Tool::new("suppressed", "Suppress"),
                Tool::new("duplicate", "Duplicate"),
            ],
            &[Tool::new("delete", "Delete"), Tool::new("overflow", "More")],
        ],
    };
    chrome::pill(ui, cx, &spec);
}
