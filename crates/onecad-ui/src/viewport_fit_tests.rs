use super::*;
use crate::ui_state::UiState;
use onecad_scene::{Camera, DVec3, Projection, SceneIdentity};

#[test]
fn fit_keeps_eight_bounds_corners_outside_open_sidebars() {
    for size in [
        egui::vec2(1280.0, 820.0),
        egui::vec2(820.0, 1180.0),
        egui::vec2(900.0, 560.0),
    ] {
        for origin in [egui::Pos2::ZERO, egui::pos2(37.0, 61.0)] {
            let screen = egui::Rect::from_min_size(origin, size);
            for explorer_open in [true, false] {
                for inspector_open in [true, false] {
                    let state = UiState {
                        explorer_open,
                        inspector_open,
                        ..Default::default()
                    };
                    for projection in [Projection::Perspective, Projection::Orthographic] {
                        for offset in [DVec3::new(0.0, 0.0, 400.0), Camera::default().offset] {
                            assert_fit(
                                screen,
                                &state,
                                Camera {
                                    projection,
                                    offset,
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn fit_refuses_overlapping_sidebars_without_full_viewport_fallback() {
    let state = UiState {
        explorer_open: true,
        inspector_open: true,
        ..Default::default()
    };
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(556.0, 560.0));
    let rect = crate::layout::chrome(screen, &state).viewport;
    let viewport = Viewport {
        width: f64::from(rect.width()),
        height: f64::from(rect.height()),
        scale_factor: 1.0,
    };
    let bounds = Bounds::new(DVec3::ZERO, DVec3::new(80.0, 50.0, 8.0)).unwrap();
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let camera = Camera {
            projection,
            ..Default::default()
        };
        assert!(fit_for_layout(bounds, camera, viewport, screen, rect, &state).is_err());
    }
}

fn assert_fit(screen: egui::Rect, state: &UiState, camera: Camera) {
    let chrome = crate::layout::chrome(screen, state);
    let rect = chrome.viewport;
    let viewport = Viewport {
        width: f64::from(rect.width()),
        height: f64::from(rect.height()),
        scale_factor: 2.0,
    };
    let bounds = Bounds::new(DVec3::ZERO, DVec3::new(80.0, 50.0, 8.0)).unwrap();
    let fit = fit_for_layout(bounds, camera, viewport, screen, rect, state).unwrap();
    let fitted = Camera {
        target: fit.target,
        offset: camera.offset.normalize() * fit.distance,
        ..camera
    };
    let view = ViewSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 1,
        },
        1,
        fitted,
        viewport,
    )
    .unwrap();
    let left = if state.explorer_open { 258.0 } else { 10.0 };
    let right = viewport.width - if state.inspector_open { 298.0 } else { 10.0 };
    for corner in bounds.corners() {
        let point = view
            .project(corner)
            .expect("fitted corner must remain inside clip planes")
            .logical;
        assert!(
            point.x >= left - 1e-8
                && point.x <= right + 1e-8
                && point.y >= -1e-8
                && point.y <= viewport.height + 1e-8,
            "corner {corner:?} projects to {point:?}, safe x {left}..{right}, viewport {rect:?}, panels ({}, {}), camera {camera:?}",
            state.explorer_open,
            state.inspector_open
        );
    }
}
