use super::*;

fn keyboard_screen(width: f32) -> Rect {
    Rect::from_min_size(pos2(0.0, 32.0), vec2(width, 366.0))
}

fn assert_card_clears_chrome(chrome: Chrome, state: &UiState) {
    let card = chrome.feature_card;
    assert!(chrome.viewport.contains_rect(card), "{card:?}");
    assert!(card.height() >= 180.0, "{card:?}");
    assert_eq!(card.width(), CARD_W);
    let cube = Rect::from_min_size(chrome.view_cluster - vec2(CUBE, 0.0), vec2(CUBE, CUBE));
    let toolbar_band = Rect::from_min_size(
        chrome.viewport.min,
        vec2(chrome.viewport.width(), EDGE + 48.0),
    );
    for occupied in [
        cube,
        chrome.view_controls,
        chrome.inspector_toggle.unwrap(),
        Rect::from_min_size(chrome.nav_pill - vec2(0.0, 40.0), vec2(176.0, 40.0)),
        toolbar_band,
        if state.explorer_open {
            chrome.explorer
        } else {
            chrome.explorer_toggle
        },
    ] {
        assert!(!card.intersect(occupied).is_positive(), "{occupied:?}");
    }
    if state.inspector_open {
        assert!(!card.intersect(chrome.inspector).is_positive());
    }
}

#[test]
fn keyboard_landscape_card_fits_beside_navigation_in_all_panel_states() {
    for width in [1180.0, 1024.0] {
        for explorer_open in [false, true] {
            for inspector_open in [false, true] {
                let state = UiState {
                    explorer_open,
                    inspector_open,
                    ..Default::default()
                };
                let chrome = chrome(keyboard_screen(width), &state).with_feature_card();
                assert_card_clears_chrome(chrome, &state);
                assert_eq!(
                    chrome.feature_card.right(),
                    chrome.view_controls.left() - EDGE
                );
                assert_eq!(chrome.feature_card.top(), chrome.viewport.top() + 62.0);
                assert_eq!(
                    chrome.feature_card.height(),
                    if explorer_open && inspector_open {
                        200.0
                    } else {
                        250.0
                    }
                );
                assert_eq!(state.explorer_open, explorer_open);
                assert_eq!(state.inspector_open, inspector_open);
            }
        }
    }
}

#[test]
fn keyboard_air4_landscape_card_has_measured_available_bounds() {
    let chrome = chrome(keyboard_screen(1180.0), &UiState::default()).with_feature_card();
    assert_eq!(chrome.viewport.min, pos2(0.0, 76.0));
    assert_eq!(chrome.view_controls.left(), 724.0);
    assert_eq!(
        chrome.feature_card,
        Rect::from_min_max(pos2(434.0, 138.0), pos2(714.0, 338.0))
    );
}

#[test]
fn normal_height_and_exact_minimum_keep_existing_card_geometry() {
    for size in [
        vec2(900.0, 560.0),
        vec2(820.0, 1180.0),
        vec2(1180.0, 820.0),
        vec2(1024.0, 436.0),
    ] {
        for explorer_open in [false, true] {
            for inspector_open in [false, true] {
                let state = UiState {
                    explorer_open,
                    inspector_open,
                    ..Default::default()
                };
                let screen = Rect::from_min_size(pos2(0.0, 32.0), size);
                let base = chrome(screen, &state);
                let active = base.with_feature_card();
                assert_eq!(active.feature_card.left(), base.feature_card.left());
                assert_eq!(active.feature_card.top(), 278.0);
                assert_eq!(
                    active.feature_card.height(),
                    CARD_H.min(screen.bottom() - EDGE - 278.0)
                );
            }
        }
    }
}

#[test]
fn impossible_short_narrow_window_keeps_existing_fallback() {
    let chrome = chrome(keyboard_screen(820.0), &UiState::default()).with_feature_card();
    assert_eq!(
        chrome.feature_card,
        Rect::from_min_size(pos2(234.0, 278.0), vec2(CARD_W, 110.0))
    );
    assert!(chrome.compact_feature_card().is_none());
}

#[test]
fn insufficient_height_refuses_compact_card() {
    let screen = Rect::from_min_size(pos2(0.0, 32.0), vec2(1180.0, 345.0));
    let chrome = chrome(screen, &UiState::default()).with_feature_card();
    assert!(chrome.compact_feature_card().is_none());
    assert_eq!(chrome.feature_card.top(), 278.0);
    assert_eq!(chrome.feature_card.height(), 89.0);
}

#[test]
fn small_keyboard_height_changes_keep_usable_compact_card() {
    for (height, card_height) in [(365.0, 199.0), (346.0, 180.0)] {
        let screen = Rect::from_min_size(pos2(0.0, 32.0), vec2(1180.0, height));
        let state = UiState::default();
        let chrome = chrome(screen, &state).with_feature_card();
        assert_eq!(chrome.feature_card.top(), 138.0);
        assert_eq!(chrome.feature_card.height(), card_height);
        assert_card_clears_chrome(chrome, &state);
    }
}

fn render_compact_card(ctx: &egui::Context, chrome: Chrome) -> (egui::FullOutput, Rect) {
    use onecad_widgets::{components::overlays::FeatureCard, frame};
    let mut actual = Rect::NOTHING;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_max(chrome.titlebar.min, chrome.viewport.max)),
            ..Default::default()
        },
        |_| {
            actual = frame::floating(
                egui::Id::unique("compact-card"),
                chrome.feature_card.min,
                egui::Order::Middle,
            )
            .show(ctx, |ui| {
                ui.set_max_height(chrome.feature_card.height());
                FeatureCard::new("compact", "move", "Move / Rotate")
                    .width(chrome.feature_card.width())
                    .max_height(chrome.feature_card.height())
                    .ok_label("Create ↵")
                    .show(ui, |rows| {
                        for label in ["Parameter", "X", "Y", "Z"] {
                            rows.row(label, |ui| {
                                ui.add_sized(vec2(130.0, 30.0), egui::Label::new("4.00 mm"));
                            });
                        }
                    });
            })
            .response
            .rect;
        },
    );
    (output, actual)
}

#[test]
fn rendered_compact_card_keeps_footer_inside_card_and_keyboard_viewport() {
    for width in [1180.0, 1024.0] {
        for scale in [1.0, 2.0] {
            for height in [366.0, 346.0] {
                let ctx = egui::Context::default();
                onecad_widgets::theme::install(
                    &ctx,
                    onecad_widgets::theme::load(false).unwrap(),
                    false,
                );
                ctx.set_pixels_per_point(scale);
                let screen = Rect::from_min_size(pos2(0.0, 32.0), vec2(width, height));
                let chrome = chrome(screen, &UiState::default()).with_feature_card();
                for index in 0..3 {
                    let (mut output, actual) = render_compact_card(&ctx, chrome);
                    output.textures_delta.clear();
                    if index > 0 {
                        assert_rendered_contents(&output, actual, chrome);
                    }
                }
            }
        }
    }
}

fn assert_rendered_contents(output: &egui::FullOutput, actual: Rect, chrome: Chrome) {
    assert!(chrome.feature_card.contains_rect(actual), "{actual:?}");
    let (mut footer_count, mut visible_body) = (0, 0);
    for shape in &output.shapes {
        if let egui::epaint::Shape::Text(text) = &shape.shape {
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            if matches!(text.galley.job.text.as_str(), "Create ↵" | "Cancel") {
                footer_count += 1;
                assert!(shape.clip_rect.contains_rect(bounds));
                assert!(actual.contains_rect(bounds));
                assert!(chrome.viewport.contains_rect(bounds));
            }
            if text.galley.job.text == "4.00 mm" && shape.clip_rect.contains_rect(bounds) {
                visible_body += 1;
                assert!(shape.clip_rect.height() >= 80.0, "{:?}", shape.clip_rect);
            }
        }
    }
    assert_eq!(footer_count, 2);
    assert!(visible_body > 0);
}
