//! Sketch-mode screens: 04a nothing selected (constraint menu open), 04b line selected.

use egui::{Rect, Ui, pos2, vec2};
use onecad_widgets::Theme;
use onecad_widgets::components::actions::{PillAccent, PillMode, SelectionVariant, ToolState};
use onecad_widgets::components::button::ButtonKind;
use onecad_widgets::components::lists::{MenuRow, MenuSection, menu_divider, menu_frame};
use onecad_widgets::components::overlays::{StripAction, StripVariant};
use onecad_widgets::components::shell::StatusItem;

use super::chrome::{self, Base, Cx, PillSpec, Status, Tool};
use super::explorer::ExplorerMode;

fn base<'a>(text: &'a str, hint: Option<&'a str>, items: &'a [StatusItem<'a>]) -> Base<'a> {
    Base {
        explorer: Some(ExplorerMode::Sketching),
        inspector: None,
        sketch: true,
        dot: false,
        isolate: false,
        status: Status {
            sketch: true,
            dot: None,
            text,
            hint,
            persp: None,
            items,
        },
    }
}

/// "Sketch 3 on Face 7 · 3 DOF left · Cancel · Finish sketch".
fn session_strip(ui: &mut Ui, cx: &Cx) {
    let actions = [
        StripAction {
            label: "Cancel",
            kind: ButtonKind::Secondary,
            enabled: true,
            tip: None,
        },
        StripAction {
            label: "Finish sketch",
            kind: ButtonKind::Primary,
            enabled: true,
            tip: None,
        },
    ];
    let status = Some(("3 DOF left", cx.t.warn, Some("underConstrained")));
    let head = ("sketch", "Sketch 3", Some("on Face 7"));
    chrome::strip(ui, cx, StripVariant::Sketch, head, status, &actions);
}

pub fn s04a(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let items = chrome::coords("70.00", "44.00", None);
    let b = base("Line — click start point · Esc to stop", None, &items);
    chrome::screen(ui, t, dark, stage, &b, |ui, cx| {
        let spec = PillSpec {
            mode: PillMode::Sketch,
            accent: PillAccent::None,
            label: None,
            groups: &[
                &[Tool::new("select", "Select").key("V")],
                &[
                    Tool::new("line", "Line")
                        .key("L")
                        .state(ToolState::SketchActive),
                    Tool::new("rect", "Rectangle"),
                    Tool::new("circle", "Circle"),
                    Tool::new("arc", "Arc"),
                    Tool::new("slot", "Slot"),
                    Tool::new("polygon", "Polygon"),
                    Tool::new("spline", "Spline"),
                    Tool::new("point", "Point"),
                ],
                &[
                    Tool::new("dimension", "Dimension").key("D"),
                    Tool::new("coincident", "Constraints").state(ToolState::Active),
                ],
                &[
                    Tool::new("trim", "Trim"),
                    Tool::new("extend", "Extend"),
                    Tool::new("sketchFillet", "Sketch fillet"),
                    Tool::new("sketchOffset", "Offset"),
                    Tool::new("project", "Project"),
                ],
            ],
        };
        chrome::pill(ui, cx, &spec);
        session_strip(ui, cx);
        constraint_menu(ui, cx);
    });
}

pub fn s04b(ui: &mut Ui, t: &Theme, dark: bool, stage: Rect) {
    let mut items = chrome::coords("40.00", "24.00", None);
    items.insert(
        0,
        StatusItem {
            label: None,
            value: "Under-constrained",
            mono: false,
        },
    );
    let b = base("1 line selected", Some("⇧-click to add"), &items);
    chrome::screen(ui, t, dark, stage, &b, |ui, cx| {
        let spec = PillSpec {
            mode: PillMode::Sketch,
            accent: PillAccent::SketchSelection,
            label: Some(("Line", "30.00 mm", SelectionVariant::Sketch)),
            groups: &[
                &[
                    Tool::new("horizontal", "Horizontal — applied")
                        .key("H")
                        .state(ToolState::Active),
                    Tool::new("vertical", "Vertical").off("Line is already horizontal"),
                    Tool::new("parallel", "Parallel"),
                    Tool::new("perpendicular", "Perpendicular"),
                    Tool::new("equal", "Equal"),
                    Tool::new("midpoint", "Midpoint"),
                    Tool::new("fix", "Fix"),
                    Tool::new("distance", "Distance"),
                ],
                &[
                    Tool::new("construction", "Construction"),
                    Tool::new("delete", "Delete"),
                ],
            ],
        };
        chrome::pill(ui, cx, &spec);
        session_strip(ui, cx);
    });
}

const GEOMETRIC: [(&str, &str, &str); 12] = [
    ("coincident", "Coincident", "C"),
    ("horizontal", "Horizontal", "H"),
    ("vertical", "Vertical", "V"),
    ("parallel", "Parallel", "⇧P"),
    ("perpendicular", "Perpendicular", "⇧L"),
    ("tangent", "Tangent", "T"),
    ("concentric", "Concentric", ""),
    ("equal", "Equal", "="),
    ("midpoint", "Midpoint", "M"),
    ("symmetric", "Symmetric", ""),
    ("pointOnCurve", "Point on curve", ""),
    ("fix", "Fix", "F"),
];

const DIMENSIONAL: [(&str, &str, &str); 3] = [
    ("distance", "Distance", "D"),
    ("angle", "Angle", "A"),
    ("radius", "Radius / diameter", "R"),
];

fn menu_rows(ui: &mut Ui, rows: &[(&str, &str, &str)]) {
    for (icon, label, key) in rows {
        let key = (!key.is_empty()).then_some(*key);
        ui.add(MenuRow::new(Some(icon), label).shortcut(key));
    }
}

/// Constraint list below the session strip, width 236.
fn constraint_menu(ui: &mut Ui, cx: &Cx) {
    let width = 236.0;
    let origin = pos2(cx.lay.pill_cx + 83.0, cx.lay.strip_top + 38.0 + 10.0);
    chrome::at(ui, Rect::from_min_size(origin, vec2(width, 600.0)), |ui| {
        menu_frame(ui.ctx()).show(ui, |ui| {
            ui.set_width(width - 14.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(MenuSection::new("GEOMETRIC"));
            menu_rows(ui, &GEOMETRIC);
            menu_divider(ui);
            ui.add(MenuSection::new("DIMENSIONAL"));
            menu_rows(ui, &DIMENSIONAL);
        });
    });
}
