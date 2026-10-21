use super::*;
use crate::{
    pill::{Hits, draw_groups, gate},
    tool_registry::{SelectionKind, ToolId, pill_tools},
    ui_state::UiState,
};
use egui::{Context, RawInput, Rect, Vec2, pos2, vec2};
use onecad_widgets::components::actions::SelectionLabel;

struct Capture {
    hits: Hits,
    viewport: Rect,
    outer: Rect,
}

fn frame(ctx: &Context, screen: Rect, kind: SelectionKind, offset: Option<f32>) -> Capture {
    input_frame(ctx, screen, kind, offset, vec![])
}

fn input_frame(
    ctx: &Context,
    screen: Rect,
    kind: SelectionKind,
    offset: Option<f32>,
    events: Vec<egui::Event>,
) -> Capture {
    let chrome = crate::layout::chrome(screen, &UiState::default());
    let bounds = toolbar_rect(ctx, &chrome);
    let groups = pill_tools(kind);
    let views: Vec<_> = groups
        .iter()
        .map(|group| group.iter().map(|_| gate(true, "")).collect())
        .collect();
    let mut capture = None;
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        },
        |ui| {
            let area = onecad_widgets::frame::floating(
                egui::Id::unique("tool-pill"),
                bounds.min,
                egui::Order::Middle,
            )
            .show(ctx, |ui| {
                let output = bounded_pill(
                    ui,
                    bounds.width(),
                    offset,
                    PillMode::Sketch,
                    PillAccent::SketchSelection,
                    |ui| {
                        SelectionLabel::new("Sketch entities", "12 selected").show(ui);
                        draw_groups(ui, groups, &views, None)
                    },
                );
                capture = Some(Capture {
                    hits: output.inner,
                    viewport: output.inner_rect,
                    outer: Rect::NOTHING,
                });
            });
            capture.as_mut().unwrap().outer = area.response.rect;
            let _ = ui;
        },
    );
    output.textures_delta.clear();
    capture.unwrap()
}

#[test]
fn all_tools_remain_reachable_without_covering_other_chrome() {
    for size in [
        vec2(900.0, 560.0),
        vec2(1280.0, 820.0),
        vec2(1600.0, 1000.0),
    ] {
        for dark in [false, true] {
            for scale in [1.0, 2.0] {
                for kind in [SelectionKind::SketchIdle, SelectionKind::SketchEntity] {
                    check_layout(size, dark, scale, kind);
                }
            }
        }
    }
}

#[test]
fn toolbar_lane_respects_inspector_and_running_feature_controls() {
    for width in [900.0, 1280.0, 1600.0] {
        for inspector_open in [true, false] {
            let state = UiState {
                inspector_open,
                ..Default::default()
            };
            let chrome = crate::layout::chrome(
                Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 560.0)),
                &state,
            );
            for chrome in [chrome, chrome.with_feature_card()] {
                let lane = toolbar_bounds(&chrome);
                assert!(!lane.intersects(chrome.explorer_toggle));
                assert!(!lane.intersects(chrome.view_controls));
                if inspector_open {
                    assert!(!lane.intersects(chrome.inspector));
                }
                assert!(chrome.viewport.contains_rect(lane));
            }
        }
    }
}

fn check_layout(size: Vec2, dark: bool, scale: f32, kind: SelectionKind) {
    let ctx = Context::default();
    ctx.set_pixels_per_point(scale);
    onecad_widgets::fonts::install(&ctx);
    onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(dark).unwrap(), dark);
    let screen = Rect::from_min_size(pos2(0.0, 0.0), size);
    let chrome = crate::layout::chrome(screen, &UiState::default());
    let bounds = toolbar_bounds(&chrome);
    assert!(!bounds.intersects(chrome.explorer_toggle));
    assert!(!bounds.intersects(chrome.inspector));
    assert!(bounds.right() < chrome.view_cluster.x - CUBE);
    let _ = frame(&ctx, screen, kind, Some(0.0));
    let initial = frame(&ctx, screen, kind, Some(0.0));
    assert!(
        bounds.expand(1.0).contains_rect(initial.viewport),
        "{size:?}: {:?} outside {bounds:?}, area {:?}",
        initial.viewport,
        initial.outer
    );
    assert!(bounds.expand(1.0).contains_rect(initial.outer));
    let expected = chrome.pill_centre_x.clamp(
        bounds.left() + initial.outer.width() / 2.0,
        bounds.right() - initial.outer.width() / 2.0,
    );
    assert!((initial.outer.center().x - expected).abs() < 1.0);
    let mut last = None;
    for (tool, rect) in &initial.hits.tools {
        let offset = Some((rect.center().x - initial.viewport.center().x).max(0.0));
        let _ = frame(&ctx, screen, kind, offset);
        let visible = frame(&ctx, screen, kind, offset);
        let target = visible
            .hits
            .tools
            .iter()
            .find(|(id, _)| id == tool)
            .unwrap()
            .1;
        assert!(
            visible.viewport.expand(1.0).contains_rect(target),
            "{size:?}, dark={dark}, scale={scale}, {tool:?}: {target:?} outside {:?}",
            visible.viewport
        );
        for pressed in [true, false] {
            let click = input_frame(
                &ctx,
                screen,
                kind,
                offset,
                vec![
                    egui::Event::PointerMoved(target.center()),
                    egui::Event::PointerButton {
                        pos: target.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if !pressed {
                assert_eq!(click.hits.clicked, Some(*tool));
            }
        }
        last = Some(*tool);
    }
    assert_eq!(
        last,
        Some(if kind == SelectionKind::SketchEntity {
            ToolId::Delete
        } else {
            ToolId::Constraints
        })
    );
}
