//! g06 Start screen: SidebarNavItem, ActionTile, RecentCard.

use egui::{Id, Ui, vec2};
use onecad_widgets::Theme;
use onecad_widgets::components::start::{ActionTile, RecentCard, SidebarNavItem};

use super::g_shell::{heading, note, stage};

pub fn show(ui: &mut Ui, t: &Theme, _dark: bool) {
    nav_items(ui, t);
    tiles(ui, t);
    recents(ui, t);
}

fn nav_items(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "SidebarNavItem",
        "c-navitem",
        "Start-screen navigation. Active uses selection colours.",
    );
    stage(ui, t, vec2(260.0, 150.0), |ui| {
        ui.set_width(200.0);
        let active = ui
            .ctx()
            .data(|d| d.get_temp::<usize>(Id::unique("g06-nav")))
            .unwrap_or(0);
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, (icon, label, enabled)) in [
            ("home", "Home", true),
            ("recent", "Recent", true),
            ("starred", "Starred", true),
            ("template", "Templates (soon)", false),
        ]
        .into_iter()
        .enumerate()
        {
            let r = ui.add(
                SidebarNavItem::new(icon, label)
                    .active(i == active)
                    .enabled(enabled),
            );
            if r.clicked() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(Id::unique("g06-nav"), i));
            }
        }
    });
    note(ui, t, "h 32 · r 6 · icon 18");
}

fn tiles(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "ActionTile",
        "c-tile",
        "Large create/open entry. Primary tile filled with accent.",
    );
    stage(ui, t, vec2(660.0, 130.0), |ui| {
        ui.horizontal(|ui| {
            for (primary, enabled, icon, title, sub) in [
                (true, true, "body", "New part", "⌘N"),
                (false, true, "import", "Open or import", "STEP, STL, 3MF"),
                (false, false, "component", "New assembly", "Unavailable"),
            ] {
                ui.allocate_ui(vec2(200.0, 92.0), |ui| {
                    ui.add(
                        ActionTile::new(icon, title, sub)
                            .primary(primary)
                            .enabled(enabled),
                    );
                });
            }
        });
    });
    note(ui, t, "h 92 · r 10 · icon well 52");
}

fn recents(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "RecentCard",
        "c-recent",
        "Project thumbnail, name, last edit. Hover lifts with the accent border.",
    );
    stage(ui, t, vec2(560.0, 250.0), |ui| {
        ui.horizontal(|ui| {
            ui.add(RecentCard::new("Mounting bracket", "12 min ago · Part"));
            ui.add(RecentCard::new("Hinge assembly", "Yesterday · Assembly").width(200.0));
        });
    });
    note(ui, t, "r 10 · thumb 150 · hover border #B9CDF2 + lift");
}
