use egui::{Pos2, Rect, Ui, Widget, vec2};

use super::*;
use crate::components::tooltip::Tip;
use crate::theme;

fn ctx_for(dark: bool) -> egui::Context {
    let ctx = egui::Context::default();
    theme::install(&ctx, theme::load(dark).unwrap(), dark);
    ctx
}

fn frame(ctx: &egui::Context, time: f64, f: impl FnMut(&mut Ui)) {
    let input = egui::RawInput {
        time: Some(time),
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        ..Default::default()
    };
    let mut f = f;
    let mut out = ctx.run_ui(input, |ui| f(ui));
    out.textures_delta.clear();
}

#[test]
fn every_shell_component_renders_light_and_dark() {
    for dark in [false, true] {
        let ctx = ctx_for(dark);
        frame(&ctx, 0.0, |ui| {
            let out = TitleBar::new("Bracket")
                .dirty(true)
                .suffix(Some("Workshop"))
                .can_redo(false)
                .workspace(Some(WorkspaceSpec {
                    name: "Design",
                    icon: "body",
                    open: true,
                }))
                .show(ui);
            assert!(out.workspace.is_some());
            assert!(!out.home && !out.search && !out.undo && !out.redo);
            ui.add(PanelToggle::new(Side::Left).open(true));
            ui.add(
                PanelToggle::new(Side::Right)
                    .dot(true)
                    .tip(Tip::new("Inspector")),
            );
            ui.add(WorkspaceButton::new("Design", "Workspace · ⌃1", "body").open(true));
            ui.add(WorkspaceButton::compact("Design", "body").open(true));
            let items = [WorkspaceItem {
                label: "Design",
                icon: "body",
                shortcut: Some("⌃1"),
                subtitle: None,
                enabled: true,
                active: true,
            }];
            let m = workspace_menu(ui, &items, &items);
            assert!(m.picked.is_none() && !m.customize);
            let status = [StatusItem {
                label: Some("X"),
                value: "42.10",
                mono: true,
            }];
            let out = StatusBar::new()
                .sketch(dark)
                .dot(Some(egui::Color32::GREEN))
                .text("Preview ready")
                .hint(Some("Esc"))
                .projection(Some(true))
                .items(&status)
                .unit(Some("mm"))
                .show(ui);
            assert!(out.projection.is_none());
        });
    }
}

#[test]
fn compact_toggle_is_32_and_full_is_44() {
    let ctx = ctx_for(false);
    frame(&ctx, 0.0, |ui| {
        let r = PanelToggle::new(Side::Left).compact(true).ui(ui);
        assert_eq!(r.rect.size(), vec2(32.0, 32.0));
        let r = PanelToggle::new(Side::Left).ui(ui);
        assert_eq!(r.rect.size(), vec2(44.0, 44.0));
    });
}

#[test]
fn titlebar_is_44_high() {
    let ctx = ctx_for(false);
    frame(&ctx, 0.0, |ui| {
        let r = ui.scope(|ui| TitleBar::new("Doc").show(ui)).response;
        assert_eq!(r.rect.height(), 44.0);
    });
}

#[test]
fn closed_floating_panel_renders_nothing_after_animation() {
    let ctx = ctx_for(false);
    let rect = Rect::from_min_size(Pos2::new(10.0, 62.0), vec2(248.0, 300.0));
    let id = egui::Id::unique("explorer");
    let show = |ctx: &egui::Context, time: f64, open: bool| {
        let mut shown = None;
        frame(ctx, time, |_| {
            shown = FloatingPanel::new(id, Side::Left)
                .open(open)
                .rect(rect)
                .show(ctx, |ui| {
                    ui.label("content");
                });
        });
        shown
    };
    assert!(show(&ctx, 0.0, true).is_some());
    assert!(show(&ctx, 0.1, false).is_some(), "still animating out");
    assert!(show(&ctx, 1.0, false).is_none(), "fully closed");
    assert!(show(&ctx, 1.1, true).is_some(), "reopens");
}

#[test]
fn bare_titlebar_has_no_file_and_keeps_height() {
    for dark in [false, true] {
        let ctx = ctx_for(dark);
        frame(&ctx, 0.0, |ui| {
            let r = ui
                .scope(|ui| {
                    let out = TitleBar::new("Doc").dirty(true).bare(true).show(ui);
                    assert!(out.file.is_none() && out.workspace.is_none());
                    assert!(!out.home && !out.undo && !out.redo && !out.search);
                })
                .response;
            assert_eq!(r.rect.height(), 44.0);
        });
    }
}

#[test]
fn workspace_selector_paints_in_both_themes() {
    for dark in [false, true] {
        let ctx = ctx_for(dark);
        frame(&ctx, 0.0, |ui| {
            for open in [false, true] {
                let r = ui.add(WorkspaceSelector::new("Design", "body").open(open));
                assert_eq!(r.rect.height(), 36.0);
                let r = ui.add(WorkspaceSelector::new("Design", "body").compact(true));
                assert_eq!(
                    r.rect.size(),
                    vec2(WorkspaceSelector::COMPACT_W, WorkspaceSelector::COMPACT_H)
                );
            }
        });
    }
}

#[test]
fn compact_workspace_menu_rows_are_34_with_active_check() {
    use super::{WorkspaceItem, workspace_menu_compact};
    for dark in [false, true] {
        let ctx = ctx_for(dark);
        let mk = |label, active, enabled| WorkspaceItem {
            label,
            icon: "body",
            shortcut: Some("⌃1"),
            subtitle: None,
            enabled,
            active,
        };
        let items = [mk("Design", true, true), mk("Assembly", false, false)];
        let ext = [mk("CNC", false, true)];
        frame(&ctx, 0.0, |ui| {
            let top = ui.cursor().top();
            let out = workspace_menu_compact(ui, &items, &ext, ("Customize…", true), Some("hint"));
            assert!(out.picked.is_none() && out.picked_extension.is_none() && !out.customize);
            // 3 rows + footer at 34 each, 2 dividers at 9 each.
            let h = ui.min_rect().bottom() - top;
            assert_eq!(h, 4.0 * 34.0 + 2.0 * 9.0);
        });
        let shapes = |active: bool| {
            let items = [mk("Design", active, true)];
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                workspace_menu_compact(ui, &items, &[], ("Customize…", true), None);
            });
            out.textures_delta.clear();
            out.shapes.len()
        };
        // The active row paints the check polyline on top of the inactive shapes.
        assert!(shapes(true) > shapes(false));
    }
}
