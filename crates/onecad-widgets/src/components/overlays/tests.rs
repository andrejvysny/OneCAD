use egui::{Color32, Pos2, Rect, Ui, Widget, vec2};

use super::*;
use crate::components::button::ButtonKind;
use crate::components::tooltip::Tip;
use crate::theme;

fn run(dark: bool, mut f: impl FnMut(&mut Ui)) {
    let ctx = egui::Context::default();
    theme::install(&ctx, theme::load(dark).unwrap(), dark);
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| f(ui));
    out.textures_delta.clear();
}

fn view_and_nav(ui: &mut Ui) {
    let buttons = [
        SquareButton {
            icon: "grid",
            on: true,
            enabled: true,
            tip: Tip::new("Grid"),
        },
        SquareButton {
            icon: "snap",
            on: false,
            enabled: false,
            tip: Tip::new("Snap").reason(Some("No sketch")),
        },
    ];
    let mut slot = Rect::NOTHING;
    let out = ViewCluster::new(&buttons).show(ui, |_, r, _| slot = r);
    assert_eq!(slot.size(), vec2(72.0, 72.0));
    assert!(out.clicked.is_none());
    let mut painted = false;
    let before = ui.cursor().min.y;
    ViewCluster::new(&buttons)
        .no_cube()
        .show(ui, |_, _, _| painted = true);
    assert!(!painted);
    let used = ui.cursor().min.y - before - ui.spacing().item_spacing.y;
    assert!((used - (2.0 * 36.0 + 8.0)).abs() < 1.0, "{used}");
    let nav = [NavButton {
        icon: "home",
        enabled: true,
        active: false,
        tip: Tip::new("Home"),
    }];
    assert!(NavPill::new(&nav).show(ui).is_none());
}

fn strips(ui: &mut Ui) {
    let actions = [
        StripAction {
            label: "Cancel",
            kind: ButtonKind::Secondary,
            enabled: true,
            tip: None,
        },
        StripAction {
            label: "Finish",
            kind: ButtonKind::Primary,
            enabled: true,
            tip: None,
        },
    ];
    for variant in [StripVariant::Sketch, StripVariant::Neutral] {
        let strip = SessionStrip::new(variant)
            .icon(Some("sketch"))
            .title("Sketch 3")
            .subtitle(Some("to Hole 2"))
            .status(Some(("3 DOF", Color32::RED, Some("underConstrained"))))
            .actions(&actions);
        assert!(strip.show(ui).is_none());
    }
}

fn tags_and_card(ui: &mut Ui) {
    ValueTag::new("14.00 mm").ui(ui);
    ValueTag::new("14.00 mm").tone(ValueTagTone::Modify).ui(ui);
    let hud = ValueTag::new("14.00 mm").hud("union").ui(ui);
    assert!(!hud.clicked());
    let mut called = false;
    let out = FeatureCard::new("extrude", "extrude", "Extrude")
        .shortcut(Some("E"))
        .show(ui, |rows| {
            called = true;
            rows.row("Distance", |ui| {
                ui.label("14.00");
            });
            rows.caption("Hint");
        });
    assert!(called && !out.ok && !out.cancel && !out.close);
}

#[test]
fn every_overlay_renders_light_and_dark() {
    for dark in [false, true] {
        run(dark, |ui| {
            view_and_nav(ui);
            strips(ui);
            tags_and_card(ui);
        });
    }
}

#[test]
fn feature_card_has_requested_width() {
    run(false, |ui| {
        let before = ui.cursor().min;
        FeatureCard::new("w", "extrude", "Extrude").show(ui, |_| {});
        let used = ui.min_rect();
        assert!(used.min.x <= before.x + 0.01);
        assert!((used.width() - 280.0).abs() < 1.5, "{}", used.width());
    });
}
