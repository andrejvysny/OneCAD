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
fn feature_card_uses_readiness_after_row_edits_for_its_footer() {
    for enabled in [false, true] {
        run(false, |ui| {
            let output = FeatureCard::new("draft-readiness", "extrude", "Extrude")
                .ok_enabled(!enabled)
                .show(ui, |rows| rows.set_ok_enabled(enabled));
            let response = ui.ctx().read_response(output.ok_id.unwrap()).unwrap();
            assert_eq!(response.sense.senses_click(), enabled);
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

#[test]
fn feature_card_respects_cancellation_availability() {
    for enabled in [false, true] {
        for close in [false, true] {
            let ctx = egui::Context::default();
            theme::install(&ctx, theme::load(false).unwrap(), false);
            let mut target = Pos2::ZERO;
            let mut output = FeatureCardOutput::default();
            for pressed in [None, Some(true), Some(false)] {
                let events = pressed.map_or_else(Vec::new, |pressed| {
                    vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton {
                            pos: target,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                });
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                    events,
                    ..Default::default()
                };
                let mut frame = ctx.run_ui(input, |ui| {
                    target = ui.cursor().min + vec2(258.0, 22.0);
                    output = FeatureCard::new("cancel", "extrude", "Extrude")
                        .cancel_enabled(enabled)
                        .show(ui, |_| {});
                });
                frame.textures_delta.clear();
                if !close {
                    for shape in frame.shapes {
                        if let egui::epaint::Shape::Text(text) = shape.shape
                            && text.galley.job.text == "Cancel"
                        {
                            target = text.pos + text.galley.size() / 2.0;
                        }
                    }
                }
            }
            assert_eq!(output.close || output.cancel, enabled, "close={close}");
        }
    }
}

#[test]
fn long_feature_card_scrolls_parameters_with_fixed_header_and_actions() {
    for size in [vec2(900.0, 560.0), vec2(1280.0, 820.0)] {
        let ctx = egui::Context::default();
        theme::install(&ctx, theme::load(false).unwrap(), false);
        let card_top = egui::pos2(50.0, 100.0);
        let height = 296.0;
        let mut fixed = None;
        let mut last_visible = false;
        for pass in 0..8 {
            let events = if pass >= 3 {
                vec![
                    egui::Event::PointerMoved(card_top + vec2(150.0, 100.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        source: egui::MouseWheelSource::Wheel,
                        phase: egui::TouchPhase::Move,
                        delta: vec2(0.0, -1000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                Vec::new()
            };
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |_| {
                    crate::frame::floating(
                        egui::Id::unique("long-card"),
                        card_top,
                        egui::Order::Middle,
                    )
                    .show(&ctx, |ui| {
                        ui.set_max_height(height);
                        FeatureCard::new("long", "gear", "Gear")
                            .max_height(height)
                            .ok_label("Create ↵")
                            .show(ui, |rows| {
                                for index in 0..20 {
                                    rows.row("Value", |ui| {
                                        ui.label(format!("Parameter {index}"));
                                    });
                                }
                            });
                    });
                },
            );
            out.textures_delta.clear();
            if pass == 0 {
                continue;
            }
            let mut positions = Vec::new();
            for shape in &out.shapes {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    let bounds = Rect::from_min_size(text.pos, text.galley.size());
                    let visible = shape.clip_rect.contains_rect(bounds);
                    if matches!(
                        text.galley.job.text.as_str(),
                        "Gear" | "Cancel" | "Create ↵"
                    ) {
                        assert!(visible, "{} clipped", text.galley.job.text);
                        assert!(bounds.bottom() <= card_top.y + height);
                        positions.push(text.pos);
                    }
                    if text.galley.job.text == "Parameter 19" {
                        last_visible = visible;
                    }
                }
            }
            assert_eq!(positions.len(), 3);
            if pass == 2 {
                fixed = Some(positions.clone());
                assert!(!last_visible);
            }
            if pass > 2 {
                assert_eq!(Some(&positions), fixed.as_ref());
            }
        }
        assert!(last_visible, "last parameter unreachable at {size:?}");
    }
}
