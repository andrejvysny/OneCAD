//! g05 Inputs: NumberField, Select, Toggle, ReferenceChip, AddChip, OptionIconGroup,
//! SearchField.

use egui::{Id, Ui, vec2};
use onecad_widgets::Theme;
use onecad_widgets::components::inputs::{
    AddChip, NumberField, OptionIconGroup, ReferenceChip, SearchField, Select, Toggle,
};
use onecad_widgets::components::tooltip::Tip;

use super::g_shell::{heading, note, stage};

pub fn show(ui: &mut Ui, t: &Theme, _dark: bool) {
    numbers(ui, t);
    select_toggle(ui, t);
    chips(ui, t);
    search(ui, t);
}

fn captioned(ui: &mut Ui, t: &Theme, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        add(ui);
        note(ui, t, label);
    });
}

/// Demo value kept in egui temp storage so the page needs no state struct.
fn state<T: Clone + Send + Sync + 'static>(ui: &Ui, key: &str, default: T) -> T {
    ui.data_mut(|d| d.get_temp_mut_or(Id::unique(key), default).clone())
}

fn store<T: Clone + Send + Sync + 'static>(ui: &Ui, key: &str, value: T) {
    ui.data_mut(|d| d.insert_temp(Id::unique(key), value));
}

/// Focused once on first frame so the page shows the focus ring.
fn focused_field(ui: &mut Ui, text: &mut String) {
    let r = ui.add(NumberField::new("g05-b", text).unit("mm"));
    if !state(ui, "g05-focused", false) {
        r.request_focus();
        store(ui, "g05-focused", true);
    }
}

fn numbers(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "NumberField",
        "c-num",
        "Accepts expressions (=width/2). Arrow keys step; ⇧ ×10. Unit stays in the field.",
    );
    stage(ui, t, vec2(840.0, 190.0), |ui| {
        let mut a = state(ui, "g05-num-a", "14.00".to_owned());
        let mut b = state(ui, "g05-num-b", "14.00".to_owned());
        let mut c = state(ui, "g05-num-c", "=d1/2".to_owned());
        let mut d = state(ui, "g05-num-d", "\u{2212}3".to_owned());
        let mut e = state(ui, "g05-num-e", "14.00".to_owned());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            captioned(ui, t, "default", |ui| {
                ui.add(NumberField::new("g05-a", &mut a).unit("mm"));
            });
            captioned(ui, t, "focus", |ui| focused_field(ui, &mut b));
            captioned(ui, t, "expression", |ui| {
                ui.add(NumberField::new("g05-c", &mut c).unit("mm"));
            });
            captioned(ui, t, "error", |ui| {
                let msg = "Distance must be greater than 0";
                ui.add(
                    NumberField::new("g05-d", &mut d)
                        .unit("mm")
                        .error(Some(msg))
                        .width(200.0),
                );
            });
            captioned(ui, t, "compact", |ui| {
                ui.add(NumberField::new("g05-e", &mut e).compact(true).width(90.0));
            });
        });
        for (k, v) in [("a", a), ("b", b), ("c", c), ("d", d), ("e", e)] {
            store(ui, &format!("g05-num-{k}"), v);
        }
    });
    note(
        ui,
        t,
        "h 30 · r 6 · focus ring 3px accent 15% · error danger · compact 26 r 5",
    );
}

fn select_toggle(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "Select + Toggle",
        "c-select",
        "Select for enums over 3 options; Toggle for booleans in property grids.",
    );
    stage(ui, t, vec2(560.0, 116.0), |ui| {
        let options = ["Distance", "Through all", "Up to face"];
        let join = ["Join", "Cut", "Intersect"];
        let mut sel = state(ui, "g05-sel", 0usize);
        let mut op = state(ui, "g05-op", 0usize);
        let mut on = state(ui, "g05-on", true);
        let mut off = state(ui, "g05-off", false);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            captioned(ui, t, "select", |ui| {
                if let Some(i) = Select::new("g05-select", options[sel], &options)
                    .show(ui)
                    .chosen
                {
                    sel = i;
                }
            });
            captioned(ui, t, "with icon", |ui| {
                if let Some(i) = Select::new("g05-join", join[op], &join)
                    .icon(Some("union"))
                    .width(130.0)
                    .show(ui)
                    .chosen
                {
                    op = i;
                }
            });
            captioned(ui, t, "off", |ui| {
                ui.add(Toggle::new(&mut off));
            });
            captioned(ui, t, "on", |ui| {
                ui.add(Toggle::new(&mut on));
            });
        });
        store(ui, "g05-sel", sel);
        store(ui, "g05-op", op);
        store(ui, "g05-on", on);
        store(ui, "g05-off", off);
    });
    note(ui, t, "select h 30 r 6 · toggle 30×18");
}

fn chips(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "ReferenceChip + OptionIconGroup",
        "c-ref",
        "Chips hold picked geometry (× removes, + adds). Icon groups pick an operation.",
    );
    stage(ui, t, vec2(560.0, 150.0), |ui| {
        let mut sel = state(ui, "g05-opt", 0usize);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ReferenceChip::new("Sketch 3 · region 1")
                .icon(Some("sketchEntity"))
                .removable(true)
                .show(ui);
            ReferenceChip::new("2 edges").icon(Some("select")).show(ui);
            ui.add(AddChip::new());
        });
        ui.add_space(12.0);
        let opts = [
            ("union", Tip::new("Join")),
            ("subtract", Tip::new("Cut")),
            ("intersect", Tip::new("Intersect")),
            ("body", Tip::new("New body")),
        ];
        ui.add(OptionIconGroup::new(&mut sel, &opts));
        note(ui, t, "operation: join · cut · intersect · new body");
        store(ui, "g05-opt", sel);
    });
    note(ui, t, "chip h 28 r 6 sel colours · option 40×32 r 6");
}

fn search(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "SearchField / FilterField",
        "c-search",
        "⌘K command search in the title bar; filter at the top of the Explorer.",
    );
    stage(ui, t, vec2(560.0, 116.0), |ui| {
        let mut q = state(ui, "g05-q1", String::new());
        let mut f = state(ui, "g05-q2", String::new());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            captioned(ui, t, "command search", |ui| {
                ui.add(
                    SearchField::new("g05-search", &mut q)
                        .placeholder("Search commands")
                        .shortcut(Some("⌘K")),
                );
            });
            captioned(ui, t, "filter", |ui| {
                ui.add(
                    SearchField::new("g05-filter", &mut f)
                        .placeholder("Filter")
                        .width(200.0),
                );
            });
        });
        store(ui, "g05-q1", q);
        store(ui, "g05-q2", f);
    });
    note(ui, t, "h 28 · r 6");
}
