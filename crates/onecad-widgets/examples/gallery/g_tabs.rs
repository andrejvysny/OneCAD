//! Multi-document chrome: TitleBar tab strip, TabClosePopover and TabSwitcher.

use egui::{Id, Rect, Ui, vec2};
use onecad_widgets::Theme;
use onecad_widgets::components::overlays::{SwitcherItem, TabClosePopover, TabSwitcher};
use onecad_widgets::components::shell::{TabSpec, TabStrip, TitleBar};

use super::g_shell::{flag, heading, note, set_flag, stage};

const TITLES: [&str; 4] = [
    "Bracket",
    "Gearbox assembly",
    "Housing with a very long document title that truncates",
    "Shaft",
];

fn tab_state(ui: &Ui) -> u64 {
    ui.ctx()
        .data(|d| d.get_temp::<u64>(Id::unique("tabs-active")))
        .unwrap_or(1)
}

fn specs(active: u64) -> [TabSpec<'static>; 4] {
    let mk = |i: usize| TabSpec {
        id: i as u64,
        title: TITLES[i],
        icon: if i == 1 { "component" } else { "body" },
        dirty: i == 1,
        active: i as u64 == active,
        busy: i == 3,
    };
    [mk(0), mk(1), mk(2), mk(3)]
}

pub fn tabs(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "TitleBar tabs",
        "c-titlebar",
        "Tab mode: Home, undo, redo left; document tabs centred; search right. Dirty dot, hover ×, middle-click closes.",
    );
    let active = tab_state(ui);
    for home_active in [false, true] {
        stage(ui, t, vec2(840.0, 76.0), |ui| {
            ui.set_width(808.0);
            let tabs = specs(active);
            let strip = TabStrip {
                tabs: &tabs,
                home_active,
                can_add: true,
            };
            let out = TitleBar::new("")
                .tabs(Some(strip))
                .drag_window(false)
                .show(ui);
            if let Some(id) = out.activate.filter(|_| !home_active) {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(Id::unique("tabs-active"), id));
            }
            if !home_active {
                if let Some(id) = out.close {
                    set_flag(ui, "tabs-confirm", true);
                    let rect = out.tab_rects.iter().find(|(i, _)| *i == id).map(|r| r.1);
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(Id::unique("tabs-anchor"), rect));
                }
                popover(ui, t);
            }
        });
    }
    note(
        ui,
        t,
        "tab h 28 · pad 10 · min width 96 · click × on a tab to open the confirm popover",
    );
}

fn popover(ui: &mut Ui, _t: &Theme) {
    if !flag(ui, "tabs-confirm", false) {
        return;
    }
    let anchor = ui
        .ctx()
        .data(|d| d.get_temp::<Option<Rect>>(Id::unique("tabs-anchor")))
        .flatten();
    let Some(anchor) = anchor else { return };
    let out = TabClosePopover::new("Gearbox assembly", anchor)
        .body(Some(
            "3 changes since last save: Mate 4, Gear pair, Housing offset.",
        ))
        .show(ui.ctx(), Id::unique("tabs-popover"));
    if out.save || out.discard || out.cancel {
        set_flag(ui, "tabs-confirm", false);
    }
}

pub fn switcher(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "TabSwitcher",
        "c-tab-switcher",
        "Ctrl+Tab card grid. Host owns keys; click picks a card. Toggle to show the overlay.",
    );
    let shown = flag(ui, "tabs-switcher", false);
    if ui
        .button(if shown {
            "Hide switcher"
        } else {
            "Show switcher"
        })
        .clicked()
    {
        set_flag(ui, "tabs-switcher", !shown);
    }
    if !shown {
        return;
    }
    let sel = ui
        .ctx()
        .data(|d| d.get_temp::<usize>(Id::unique("tabs-sel")))
        .unwrap_or(1);
    let item = |title, icon, dirty, home| SwitcherItem {
        title,
        icon,
        dirty,
        thumbnail: None,
        home,
    };
    let items = [
        item("Home", "home", false, true),
        item("Bracket", "body", false, false),
        item("Gearbox assembly", "component", true, false),
        item("Housing", "body", false, false),
        item("Shaft", "body", true, false),
    ];
    let out = TabSwitcher::new(&items, sel).show(ui.ctx(), Id::unique("tabs-switcher-area"));
    if let Some(i) = out.clicked {
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::unique("tabs-sel"), i));
    }
}
