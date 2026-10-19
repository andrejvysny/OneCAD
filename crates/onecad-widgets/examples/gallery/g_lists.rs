//! g04 Panels & lists: TreeRow, SectionLabel, SegmentedControl, MenuList, PropertyRow,
//! HistoryRow, RollbackMarker.

use egui::{Id, Ui, UiBuilder, vec2};
use onecad_widgets::Theme;
use onecad_widgets::components::lists::{
    HistoryRow, HistoryState, MenuRow, MenuSection, PropertyRow, RollbackMarker, RowState,
    SectionLabel, Segment, SegmentedControl, TreeRow, menu_divider, menu_frame,
};

use super::g_shell::{heading, note, stage};

pub fn show(ui: &mut Ui, t: &Theme, _dark: bool) {
    tree(ui, t);
    tabs(ui, t);
    menu(ui, t);
    props(ui, t);
    history(ui, t);
}

fn tree(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "TreeRow + SectionLabel",
        "c-tree",
        "Explorer rows. Selection mirrors the viewport; editing marks the open sketch; reference geometry is muted.",
    );
    stage(ui, t, vec2(280.0, 340.0), |ui| {
        ui.set_width(248.0);
        ui.add(SectionLabel::new("BODIES"));
        ui.add(
            TreeRow::new("body", "Bracket")
                .meta(Some("Al 6061"))
                .expanded(true),
        );
        ui.add(TreeRow::new("body", "Hover").meta(Some("hover me")));
        ui.add(
            TreeRow::new("select", "Selected")
                .meta(Some("1 face"))
                .state(RowState::Selected),
        );
        ui.add(
            TreeRow::new("sketch", "Sketch 3")
                .meta(Some("editing"))
                .state(RowState::Editing)
                .trailing(Some("eyeOn")),
        );
        ui.add(
            TreeRow::new("rect", "Rectangle · 4 lines")
                .child(true)
                .trailing(Some("eyeOff")),
        );
        ui.add(
            TreeRow::new("origin", "Reference")
                .meta(Some("reference"))
                .state(RowState::Reference),
        );
        ui.add(TreeRow::new("origin", "Origin").child(true));
        ui.add(TreeRow::new("body", "Disabled").enabled(false));
    });
    note(
        ui,
        t,
        "h 32 · r 6 · icon 18 · 13/400 · meta mono 11 · child indent 30",
    );
}

fn pick(ui: &Ui, key: &str) -> usize {
    ui.data_mut(|d| *d.get_temp_mut_or(Id::unique(key), 0usize))
}

fn seg<'a>(label: &'a str, icon: Option<&'a str>) -> Segment<'a> {
    Segment { label, icon }
}

fn tabs(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "SegmentedControl",
        "c-tabs",
        "Inspector tabs, small enums (direction), projection. 2–4 options.",
    );
    let tabs = [seg("Properties", None), seg("History", None)];
    let dir = [
        seg("One side", None),
        seg("Symmetric", None),
        seg("Two", None),
    ];
    let kind = [
        seg("Fillet", Some("fillet")),
        seg("Chamfer", Some("chamfer")),
    ];
    stage(ui, t, vec2(560.0, 176.0), |ui| {
        let (mut a, mut b, mut c) = (pick(ui, "g04-a"), pick(ui, "g04-b"), pick(ui, "g04-c"));
        ui.add(SegmentedControl::new("g04-tabs", &mut a, &tabs).width(240.0));
        ui.add_space(12.0);
        ui.add(SegmentedControl::new("g04-dir", &mut b, &dir));
        ui.add_space(12.0);
        ui.add(SegmentedControl::new("g04-kind", &mut c, &kind));
        for (key, v) in [("g04-a", a), ("g04-b", b), ("g04-c", c)] {
            ui.data_mut(|d| d.insert_temp(Id::unique(key), v));
        }
    });
    note(ui, t, "h 30 · track #F0F2F5 r 7 · thumb white r 5");
}

fn menu(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "MenuList",
        "c-menu",
        "Popover list: section labels, rows with icon + shortcut, dividers, disabled rows when not applicable.",
    );
    stage(ui, t, vec2(280.0, 290.0), |ui| {
        let width = 236.0;
        let rect = ui.available_rect_before_wrap();
        let rect = egui::Rect::from_min_size(rect.min, vec2(width, rect.height()));
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            menu_frame(ui.ctx()).show(ui, |ui| {
                ui.set_width(width - 14.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(MenuSection::new("GEOMETRIC"));
                ui.add(MenuRow::new(Some("coincident"), "Coincident").shortcut(Some("C")));
                ui.add(MenuRow::new(Some("horizontal"), "Horizontal").shortcut(Some("H")));
                ui.add(MenuRow::new(Some("tangent"), "Tangent").shortcut(Some("T")));
                menu_divider(ui);
                ui.add(MenuSection::new("DIMENSIONAL"));
                ui.add(MenuRow::new(Some("distance"), "Distance").shortcut(Some("D")));
                ui.add(
                    MenuRow::new(Some("radius"), "Radius")
                        .shortcut(Some("R"))
                        .enabled(false),
                );
                ui.add(MenuRow::new(None, "Checked row").checked(true));
            });
        });
    });
    note(
        ui,
        t,
        "w 236 · row 30 r 6 · icon 20 · key mono 11 · card elevation",
    );
}

fn props(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "PropertyRow",
        "c-prop",
        "Inspector key / value. Read-only values right-aligned mono; editable values use fields.",
    );
    stage(ui, t, vec2(320.0, 152.0), |ui| {
        ui.set_width(288.0);
        ui.add(PropertyRow::new("Area").value("3 093.14 mm²"));
        ui.add(PropertyRow::new("Normal").value("+Z"));
        ui.add(PropertyRow::new("Material").value("Aluminium"));
    });
    note(ui, t, "label col 96 · row 30 · label ink-4 · value mono");
}

fn history(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "HistoryRow + RollbackMarker",
        "c-hist",
        "Feature timeline in the Inspector. Double-click a value to edit; drag the marker to roll back.",
    );
    stage(ui, t, vec2(360.0, 340.0), |ui| {
        ui.set_width(328.0);
        let id = Id::unique("g04-hist-edit");
        let mut text = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or("45.00".into()));
        ui.spacing_mut().item_spacing.y = 0.0;
        let rows = [
            (3, "extrude", "Extrude 1", "8 mm", HistoryState::Default),
            (4, "extrude", "Extrude 2", "", HistoryState::Editing),
            (5, "hole", "Hole 1", "", HistoryState::Selected),
            (6, "chamfer", "Chamfer 1", "C1", HistoryState::Failed),
            (7, "fillet", "Fillet 1", "R6", HistoryState::Suppressed),
        ];
        for (i, icon, label, meta, state) in rows {
            let meta = (!meta.is_empty()).then_some(meta);
            let edit = (state == HistoryState::Editing).then_some(&mut text);
            HistoryRow::new(i, icon, label)
                .meta(meta)
                .state(state)
                .show(ui, edit);
        }
        ui.add(RollbackMarker::new());
        HistoryRow::new(8, "fillet", "Fillet 2")
            .meta(Some("R2"))
            .state(HistoryState::RolledBack)
            .show(ui, None);
        ui.data_mut(|d| d.insert_temp(id, text));
    });
    note(
        ui,
        t,
        "h 36 · index mono 11 · states: default · selected · editing · failed (danger) · suppressed · rolled back 45%",
    );
}
