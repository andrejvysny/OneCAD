//! g02 Shell: TitleBar, FloatingPanel, PanelToggle, WorkspaceButton + menu, StatusBar.

use egui::{CornerRadius, Id, RichText, Sense, Ui, UiBuilder, vec2};
use onecad_widgets::components::shell::{
    FloatingPanel, PanelToggle, Side, StatusBar, StatusItem, TitleBar, WorkspaceButton,
    WorkspaceItem, workspace_menu,
};
use onecad_widgets::components::tooltip::Tip;
use onecad_widgets::frame;
use onecad_widgets::metrics::{Elevation, RADIUS_XL};
use onecad_widgets::{Theme, fonts};

/// Card heading: `Name  c-id`, one-line description.
pub fn heading(ui: &mut Ui, t: &Theme, name: &str, id: &str, desc: &str) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(name)
                .size(13.0)
                .family(fonts::mono())
                .color(t.ink),
        );
        ui.label(
            RichText::new(id)
                .size(11.0)
                .family(fonts::mono())
                .color(t.ink5),
        );
    });
    ui.label(RichText::new(desc).size(12.5).color(t.ink3));
    ui.add_space(6.0);
}

/// Viewport-coloured stage the component sits on, like the sheet's grey wells.
pub fn stage(ui: &mut Ui, t: &Theme, size: egui::Vec2, add: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), t.canvas_model);
    let inner = rect.shrink(16.0);
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.set_clip_rect(rect);
        add(ui);
    });
}

pub fn note(ui: &mut Ui, t: &Theme, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(t.ink5));
}

/// Boolean demo state in egui temp storage.
pub fn flag(ui: &Ui, key: &str, default: bool) -> bool {
    let id = Id::unique(key);
    ui.data_mut(|d| *d.get_temp_mut_or(id, default))
}

pub fn set_flag(ui: &Ui, key: &str, value: bool) {
    ui.data_mut(|d| d.insert_temp(Id::unique(key), value));
}

pub fn show(ui: &mut Ui, t: &Theme, dark: bool) {
    title_bar(ui, t);
    panel(ui, t, dark);
    toggles(ui, t);
    workspace(ui, t);
    status(ui, t);
}

fn title_bar(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "TitleBar",
        "c-titlebar",
        "Drag region. Home, File, centred document name + dirty dot, undo/redo, command search.",
    );
    stage(ui, t, vec2(840.0, 108.0), |ui| {
        ui.set_width(808.0);
        let out = TitleBar::new("Bracket")
            .dirty(true)
            .suffix(Some("Workshop"))
            .can_redo(false)
            .drag_window(false)
            .show(ui);
        let hits = [
            (out.home, "Home"),
            (out.file.clicked(), "File"),
            (out.undo, "Undo"),
            (out.search, "Search"),
        ];
        if let Some((_, name)) = hits.iter().find(|(hit, _)| *hit) {
            ui.ctx()
                .data_mut(|d| d.insert_temp(Id::unique("g02-last"), (*name).to_owned()));
        }
        let last = ui
            .ctx()
            .data(|d| d.get_temp::<String>(Id::unique("g02-last")));
        ui.add_space(8.0);
        note(
            ui,
            t,
            &format!("last click: {}", last.as_deref().unwrap_or("-")),
        );
    });
    note(
        ui,
        t,
        "h 44 · titlebar bg · 72px traffic-light inset · search 220 wide",
    );
}

fn panel(ui: &mut Ui, t: &Theme, dark: bool) {
    heading(
        ui,
        t,
        "FloatingPanel",
        "c-panel",
        "Frame for Explorer and Inspector. Host owns frame, resize and visibility; content fills it.",
    );
    stage(ui, t, vec2(560.0, 180.0), |ui| {
        let origin = ui.max_rect().min;
        panel_static(ui, t, dark, origin);
        panel_live(ui, origin);
    });
    note(
        ui,
        t,
        "r 12 · 1px border · panel elevation · 97% white · 10px from edges · Explorer 248 · Inspector 288",
    );
}

fn panel_static(ui: &mut Ui, t: &Theme, dark: bool, origin: egui::Pos2) {
    let inline = egui::Rect::from_min_size(origin, vec2(220.0, 120.0));
    ui.scope_builder(UiBuilder::new().max_rect(inline), |ui| {
        frame::surface(t, dark, RADIUS_XL, Elevation::Panel).show(ui, |ui| {
            ui.set_min_size(vec2(218.0, 118.0));
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("content slot")
                        .size(11.0)
                        .family(fonts::mono())
                        .color(t.ink7),
                );
            });
        });
    });
}

/// Live area: slides 12px and fades over 160 ms.
fn panel_live(ui: &mut Ui, origin: egui::Pos2) {
    let open = flag(ui, "g02-panel-open", true);
    let rect = egui::Rect::from_min_size(origin + vec2(248.0, 0.0), vec2(220.0, 120.0));
    FloatingPanel::new(Id::unique("g02-demo-panel"), Side::Left)
        .open(open)
        .rect(rect)
        .show(ui.ctx(), |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Explorer")
                        .size(14.0)
                        .family(fonts::semibold()),
                );
            });
        });
    let btn = egui::Rect::from_min_size(origin + vec2(248.0, 124.0), vec2(140.0, 22.0));
    let label = if open {
        "Close live panel"
    } else {
        "Open live panel"
    };
    if ui.put(btn, egui::Button::new(label)).clicked() {
        set_flag(ui, "g02-panel-open", !open);
    }
}

fn toggles(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "PanelToggle",
        "c-ptoggle",
        "One button per side panel. Open = blue; closed = neutral. Dot means \"selection waiting\" on a hidden Inspector.",
    );
    stage(ui, t, vec2(560.0, 108.0), |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            let specs: [(&str, Side, bool, bool, bool); 5] = [
                ("open", Side::Left, true, false, false),
                ("closed", Side::Left, false, false, false),
                ("closed + dot", Side::Right, false, true, false),
                ("compact open", Side::Right, true, false, true),
                ("compact closed", Side::Left, false, false, true),
            ];
            for (label, side, open, dot, compact) in specs {
                let key = format!("g02-toggle-{label}");
                let open = flag(ui, &key, open);
                ui.vertical(|ui| {
                    let tip = Tip::new(if side == Side::Left {
                        "Explorer"
                    } else {
                        "Inspector"
                    })
                    .shortcut(Some(if side == Side::Left {
                        "⌘1"
                    } else {
                        "⌘2"
                    }));
                    let r = ui.add(
                        PanelToggle::new(side)
                            .open(open)
                            .dot(dot)
                            .compact(compact)
                            .tip(tip),
                    );
                    if r.clicked() {
                        set_flag(ui, &key, !open);
                    }
                    note(ui, t, label);
                });
            }
        });
    });
    note(ui, t, "44×44 · r 12 · ⌘1 / ⌘2 · in-panel variant 32×32 r 7");
}

fn workspace(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "WorkspaceButton + menu",
        "c-ws",
        "Floating dropdown above the Explorer. Lists built-in and extension workspaces with ⌃1–4.",
    );
    stage(ui, t, vec2(280.0, 390.0), |ui| {
        let open = flag(ui, "g02-ws-open", false);
        let r = ui.add(WorkspaceButton::new("Design", "Workspace · ⌃1", "body").open(open));
        if r.clicked() {
            set_flag(ui, "g02-ws-open", !open);
        }
        ui.add_space(8.0);
        if open {
            workspace_popover(ui, t);
        }
    });
    note(ui, t, "h 44 · r 12 · menu rows 30 · active row sel-bg");
}

fn workspace_popover(ui: &mut Ui, t: &Theme) {
    let items = [
        ("Design", "body", "⌃1"),
        ("Assembly", "component", "⌃2"),
        ("Simulation", "study", "⌃3"),
        ("Drawing", "viewFront", "⌃4"),
    ];
    let active = ui
        .ctx()
        .data(|d| d.get_temp::<usize>(Id::unique("g02-ws-active")))
        .unwrap_or(0);
    let rows: Vec<WorkspaceItem> = items
        .iter()
        .enumerate()
        .map(|(i, (label, icon, key))| WorkspaceItem {
            label,
            icon,
            shortcut: Some(key),
            enabled: true,
            active: i == active,
        })
        .collect();
    let ext = [WorkspaceItem {
        label: "Generative",
        icon: "extension",
        shortcut: None,
        enabled: true,
        active: false,
    }];
    let look = onecad_widgets::theme::look(ui.ctx());
    let mut picked = None;
    let max = ui
        .available_rect_before_wrap()
        .with_max_x(ui.min_rect().left() + 220.0);
    ui.scope_builder(UiBuilder::new().max_rect(max), |ui| {
        frame::surface(t, look.dark, RADIUS_XL, Elevation::Panel)
            .inner_margin(6)
            .show(ui, |ui| {
                picked = workspace_menu(ui, &rows, &ext).picked;
            });
    });
    if let Some(i) = picked {
        ui.ctx()
            .data_mut(|d| d.insert_temp(Id::unique("g02-ws-active"), i));
    }
}

fn status(ui: &mut Ui, t: &Theme) {
    heading(
        ui,
        t,
        "StatusBar",
        "c-status",
        "Full-width, 28px. Left: state / hint. Right: projection, coordinates, units. Sketch mode tints blue.",
    );
    stage(ui, t, vec2(840.0, 108.0), |ui| {
        ui.set_width(808.0);
        let persp = flag(ui, "g02-persp", true);
        let coords = [
            StatusItem {
                label: Some("X"),
                value: "42.10",
                mono: true,
            },
            StatusItem {
                label: Some("Y"),
                value: "18.00",
                mono: true,
            },
            StatusItem {
                label: Some("Z"),
                value: "8.00",
                mono: true,
            },
        ];
        let out = StatusBar::new()
            .dot(Some(t.dof_ok))
            .text("Preview ready · 38 ms")
            .projection(Some(persp))
            .items(&coords)
            .unit(Some("mm"))
            .show(ui);
        if let Some(p) = out.projection {
            set_flag(ui, "g02-persp", p);
        }
        ui.add_space(8.0);
        StatusBar::new()
            .sketch(true)
            .text("Line — click start point")
            .hint(Some("Esc to stop"))
            .items(&coords[..2])
            .unit(Some("mm"))
            .show(ui);
    });
    note(ui, t, "h 28 · 12/400 · values mono 11.5 · status dot 7");
}
