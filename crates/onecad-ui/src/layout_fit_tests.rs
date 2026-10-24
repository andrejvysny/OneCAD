use super::*;

#[test]
fn fit_work_area_is_viewport_local_and_keeps_full_vertical_extent() {
    let screen = Rect::from_min_size(pos2(37.0, 61.0), vec2(1280.0, 820.0));
    for explorer_open in [false, true] {
        for inspector_open in [false, true] {
            let state = UiState {
                explorer_open,
                inspector_open,
                ..Default::default()
            };
            let viewport = chrome(screen, &state).viewport;
            let safe = fit_work_area(screen, viewport, &state).unwrap();
            let left = if explorer_open { 258.0 } else { 10.0 };
            let right = 1280.0 - if inspector_open { 298.0 } else { 10.0 };
            assert_eq!(
                [safe.x, safe.y, safe.width, safe.height],
                [left, 0.0, right - left, 776.0]
            );
        }
    }
}

#[test]
fn fit_work_area_refuses_zero_or_overlapping_sidebar_space() {
    for (explorer_open, inspector_open, minimum_width) in [
        (false, false, 20.0),
        (true, false, 268.0),
        (false, true, 308.0),
        (true, true, 556.0),
    ] {
        let state = UiState {
            explorer_open,
            inspector_open,
            ..Default::default()
        };
        for width in [minimum_width - 1.0, minimum_width] {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 820.0));
            assert!(fit_work_area(screen, chrome(screen, &state).viewport, &state).is_none());
        }
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(minimum_width + 1.0, 820.0));
        assert_eq!(
            fit_work_area(screen, chrome(screen, &state).viewport, &state)
                .unwrap()
                .width,
            1.0
        );
    }
}

#[test]
fn fit_work_area_intersects_actual_viewport_and_rejects_invalid_extent() {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 820.0));
    let state = UiState::default();
    let viewport = Rect::from_min_max(pos2(300.0, 81.0), pos2(960.0, 791.0));
    let safe = fit_work_area(screen, viewport, &state).unwrap();
    assert_eq!(
        [safe.x, safe.y, safe.width, safe.height],
        [0.0, 0.0, 660.0, 710.0]
    );
    assert!(fit_work_area(screen, Rect::ZERO, &state).is_none());
    assert!(
        fit_work_area(
            screen,
            Rect::from_min_size(Pos2::ZERO, vec2(f32::NAN, 10.0)),
            &state
        )
        .is_none()
    );
}
