//! Gallery "Screens" tab: static mocks of the OneCAD Final screens (README 01a–07a),
//! composed only from `onecad-widgets` components. A screen is drawn with child `Ui`s
//! inside a fixed 1280×800 stage; the viewport is a flat fill, not the real scene.

mod chrome;
mod editor;
mod explorer;
mod feature;
mod inspector;
mod layout;
mod sketch;
mod start;

use egui::{Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder};
use onecad_widgets::Theme;

type ScreenFn = fn(&mut Ui, &Theme, bool, Rect);

/// (id, caption from OneCAD Final.dc.html, renderer) in README order.
const SCREENS: &[(&str, &str, ScreenFn)] = &[
    (
        "01a",
        "Start — full-window home with sidebar and visual recent grid.",
        start::s01a,
    ),
    (
        "02a",
        "Floating Explorer, full-bleed viewport. Nothing selected — the pill only offers actions that need no selection.",
        editor::s02a,
    ),
    (
        "03a",
        "Face selected: pill swaps to face actions, Inspector shows Properties.",
        editor::s03a,
    ),
    (
        "03b",
        "Both panels hidden: viewport edge to edge; toggles remain, Inspector toggle carries the selection dot.",
        editor::s03b,
    ),
    (
        "04a",
        "Sketch, nothing picked: drawing tools; constraints as a list menu (shown open). Session strip under the pill.",
        sketch::s04a,
    ),
    (
        "04b",
        "Sketch, line picked: the pill becomes that line's valid constraints. Session strip stays put.",
        sketch::s04b,
    ),
    (
        "05a",
        "Extrude: parameter card beside the drag handle. The pill shows region actions with Extrude running.",
        feature::s05a,
    ),
    (
        "06a",
        "Fillet / Chamfer: edge picked, Fillet running, R6 preview in orange.",
        feature::s06a,
    ),
    (
        "07a",
        "History: feature picked, rolled back above Fillet 1, Extrude 2 distance edited in place.",
        editor::s07a,
    ),
];

pub fn show(ui: &mut Ui, t: &Theme, dark: bool) {
    let key = Id::unique("screens-selected");
    let mut sel = ui.data(|d| d.get_temp::<usize>(key)).unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        for (i, (name, _, _)) in SCREENS.iter().enumerate() {
            ui.selectable_value(&mut sel, i, *name);
        }
    });
    ui.data_mut(|d| d.insert_temp(key, sel));
    let Some((name, caption, draw)) = SCREENS.get(sel) else {
        return;
    };
    ui.label(*caption);
    ui.add_space(8.0);
    let (stage, _) = ui.allocate_exact_size(layout::STAGE, Sense::hover());
    let child = UiBuilder::new().max_rect(stage).id_salt(*name);
    ui.scope_builder(child, |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(stage));
        draw(ui, t, dark, stage);
    });
    ui.painter().rect_stroke(
        stage,
        0.0,
        Stroke::new(1.0, t.border_strong),
        StrokeKind::Outside,
    );
}
