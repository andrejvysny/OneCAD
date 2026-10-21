use super::*;
use crate::ui_state::UiState;
use egui::{RawInput, Vec2};

fn frame(ctx: &Context, size: Vec2, events: Vec<egui::Event>) -> (Rect, Option<usize>) {
    strip_frame(ctx, size, events, true)
}

fn strip_frame(
    ctx: &Context,
    size: Vec2,
    events: Vec<egui::Event>,
    long: bool,
) -> (Rect, Option<usize>) {
    let chrome = crate::layout::chrome(
        Rect::from_min_size(egui::Pos2::ZERO, size),
        &UiState::default(),
    );
    let bounds = strip_rect(ctx, &chrome, true, true);
    let actions = [
        StripAction {
            label: "Discard sketch edits",
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
    let title = if long {
        "A deliberately long sketch name that cannot fit in the minimum window"
    } else {
        "Sketch"
    };
    let status = if long {
        "Under-constrained · 12 degrees of freedom"
    } else {
        "10 DOF"
    };
    let mut result = None;
    let mut output =
        ctx.run_ui(
            RawInput {
                screen_rect: Some(chrome.viewport.union(chrome.titlebar)),
                events,
                ..Default::default()
            },
            |_| {
                let area =
                    frame::floating(egui::Id::unique("session-strip"), bounds.min, Order::Middle)
                        .show(ctx, |ui| {
                            let scroll = bounded_strip(ui, bounds.width(), |ui| {
                                SessionStrip::new(StripVariant::Sketch)
                                    .icon(Some("sketch"))
                                    .title(title)
                                    .status(Some((status, egui::Color32::GRAY, None)))
                                    .actions(&actions)
                                    .show(ui)
                            });
                            result = Some((scroll.inner_rect, scroll.inner));
                        });
                assert!(bounds.expand(1.0).contains_rect(area.response.rect));
            },
        );
    output.textures_delta.clear();
    result.unwrap()
}

#[test]
fn finish_and_discard_remain_reachable_across_sizes_themes_and_scales() {
    for size in [
        vec2(900.0, 560.0),
        vec2(1280.0, 820.0),
        vec2(1600.0, 1000.0),
    ] {
        for dark in [false, true] {
            for scale in [1.0, 2.0] {
                check_actions(size, dark, scale);
            }
        }
    }
}

fn check_actions(size: Vec2, dark: bool, scale: f32) {
    let ctx = Context::default();
    ctx.set_pixels_per_point(scale);
    onecad_widgets::fonts::install(&ctx);
    onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(dark).unwrap(), dark);
    let _ = frame(&ctx, size, vec![]);
    let (viewport, _) = frame(&ctx, size, vec![]);
    for (expected, inset) in [(0, 190.0), (1, 50.0)] {
        let pos = pos2(viewport.right() - inset, viewport.center().y);
        assert!(viewport.contains(pos));
        for pressed in [true, false] {
            let (_, clicked) = frame(
                &ctx,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if !pressed {
                assert_eq!(
                    clicked,
                    Some(expected),
                    "{size:?}, dark={dark}, scale={scale}"
                );
            }
        }
    }
}

#[test]
fn strip_avoids_side_panels_cube_controls_and_navigation() {
    for width in [900.0, 1280.0, 1600.0] {
        for explorer_open in [true, false] {
            for inspector_open in [true, false] {
                let state = UiState {
                    explorer_open,
                    inspector_open,
                    ..Default::default()
                };
                let chrome = crate::layout::chrome(
                    Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 560.0)),
                    &state,
                );
                let bounds = strip_bounds(&chrome, explorer_open, inspector_open);
                if explorer_open {
                    assert!(!bounds.intersects(chrome.explorer));
                }
                if inspector_open {
                    assert!(!bounds.intersects(chrome.inspector));
                }
                assert!(!bounds.intersects(chrome.view_controls));
                let cube = Rect::from_min_size(
                    pos2(
                        chrome.view_cluster.x - crate::layout::CUBE,
                        chrome.view_cluster.y,
                    ),
                    Vec2::splat(crate::layout::CUBE),
                );
                assert!(!bounds.intersects(cube));
                assert!(chrome.viewport.contains_rect(bounds));
                assert!(bounds.bottom() < chrome.nav_pill.y - 38.0);
            }
        }
    }
}

#[test]
fn short_strip_centers_below_toolbar_after_content_measurement() {
    for width in [1280.0, 1600.0] {
        let ctx = Context::default();
        onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(false).unwrap(), false);
        let size = vec2(width, 820.0);
        let chrome = crate::layout::chrome(
            Rect::from_min_size(egui::Pos2::ZERO, size),
            &UiState::default(),
        );
        let _ = strip_frame(&ctx, size, vec![], false);
        let _ = strip_frame(&ctx, size, vec![], false);
        let (actual, _) = strip_frame(&ctx, size, vec![], false);
        let lane = strip_bounds(&chrome, true, true);
        let expected = crate::pill::center_x(&ctx, &chrome).clamp(
            lane.left() + actual.width() / 2.0,
            lane.right() - actual.width() / 2.0,
        );
        assert!((actual.center().x - expected).abs() < 1.0);
        assert!(actual.width() < lane.width());
    }
}
