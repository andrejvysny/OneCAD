//! Synthetic native input over production worker and Metal, separate from physical qualification.
use super::*;
use onecad_core::{
    ids::EntityId,
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use onecad_interaction::sketch::{SketchPhase, SketchTool};
use onecad_scene::{Camera, DVec3, Projection};

#[path = "point_center_delivery_tests/support.rs"]
pub(super) mod support;
use support::*;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn point_tab_before_pointer_signed_zero_and_repeated_construction_clicks() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    for appearance in [
        crate::ui_state::Appearance::Light,
        crate::ui_state::Appearance::Dark,
    ] {
        host.app.ui.appearance = appearance;
        crate::appearance::sync(&mut host.app, &host.context);
        host.size = egui::vec2(900.0, 560.0);
        host.redraw();
        for label in ["Point", "Rectangle"] {
            assert!(
                host.screen().contains_rect(host.rect(label)),
                "{label} must fit minimum window"
            );
        }
    }
    host.size = egui::vec2(1280.0, 820.0);
    host.redraw();
    host.control("Point");
    assert_eq!(
        host.app.sketch.session.as_ref().unwrap().tool,
        SketchTool::Point
    );
    assert!(
        host.app.sketch.session.as_ref().unwrap().draft.is_none(),
        "Tab must work before a viewport pointer"
    );
    host.numeric("-12.5", "0");
    host.count(2).await;
    assert_numeric_point(host.sketch());
    host.xy(20.0, 15.0);
    host.count(3).await;
    host.xy(35.0, 25.0);
    host.count(4).await;
    for (x, y) in [(20.0, 15.0), (35.0, 25.0)] {
        assert!(
            host.sketch()
                .entities()
                .iter()
                .any(|entity| matches!(entity,
            SketchEntity::Point { at, construction: false, .. }
            if (at.x - x).abs() < 1e-5 && (at.y - y).abs() < 1e-5))
        );
    }
    assert_eq!(
        host.app.sketch.session.as_ref().unwrap().tool,
        SketchTool::Point
    );
    host.control("Inspector");
    host.control("Construction geometry");
    assert!(host.app.sketch.session.as_ref().unwrap().construction);
    host.xy(50.0, 35.0);
    host.count(5).await;
    let accepted = host.sketch().clone();
    assert!(accepted.entities().iter().any(|entity| matches!(entity, SketchEntity::Point { at, construction: true, .. } if (at.x-50.0).abs()<1e-5 && (at.y-35.0).abs()<1e-5)));
    host.history(false, 4).await;
    host.history(true, 5).await;
    assert_eq!(
        host.sketch(),
        &accepted,
        "point redo preserves identities and geometry"
    );
    host.stop().await;
}

fn assert_numeric_point(sketch: &Sketch) {
    let (point, at) = sketch
        .entities()
        .iter()
        .find_map(|entity| match entity {
            SketchEntity::Point {
                id,
                at,
                construction: false,
                ..
            } => Some((*id, *at)),
            _ => None,
        })
        .unwrap();
    near(at.x, -12.5);
    near(at.y, 0.0);
    let origin = sketch
        .entities()
        .iter()
        .find_map(|entity| match entity {
            SketchEntity::Point {
                id,
                at,
                construction: true,
                ..
            } if at.x == 0.0 && at.y == 0.0 => Some(*id),
            _ => None,
        })
        .unwrap();
    assert_eq!(sketch.constraints().len(), 3);
    assert_eq!(
        sketch
            .constraints()
            .iter()
            .filter(|constraint| matches!(constraint, Constraint::HorizontalDistance { .. }))
            .count(),
        1
    );
    assert_eq!(
        sketch
            .constraints()
            .iter()
            .filter(|constraint| matches!(constraint, Constraint::VerticalDistance { .. }))
            .count(),
        1
    );
    assert!(
        sketch
            .constraints()
            .iter()
            .any(|constraint| matches!(constraint,
        Constraint::Fixed { point: id, .. } if *id == origin))
    );
    for constraint in sketch.constraints() {
        match constraint {
            Constraint::HorizontalDistance {
                point1,
                point2,
                point1_position,
                point2_position,
                value,
                ..
            } => {
                assert_eq!((*point1, *point2), (origin, point));
                assert_eq!(
                    (*point1_position, *point2_position),
                    (CurvePosition::Arbitrary, CurvePosition::Arbitrary)
                );
                near(value.value, -12.5);
            }
            Constraint::VerticalDistance {
                point1,
                point2,
                value,
                ..
            } => {
                assert_eq!((*point1, *point2), (origin, point));
                near(value.value, 0.0);
            }
            _ => {}
        }
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn rectangle_method_chooser_center_numeric_full_dimensions_and_history() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Rectangle");
    assert_eq!(
        host.app.sketch.session.as_ref().unwrap().tool,
        SketchTool::Rectangle
    );
    host.app.ui.inspector_open = true;
    host.app.ui.inspector_tab = crate::ui_state::InspectorTab::Properties;
    for (from, to, tool) in [
        (
            "Corner rectangle",
            "Center rectangle",
            SketchTool::CenterRectangle,
        ),
        (
            "Center rectangle",
            "Corner rectangle",
            SketchTool::Rectangle,
        ),
        (
            "Corner rectangle",
            "Center rectangle",
            SketchTool::CenterRectangle,
        ),
    ] {
        host.control(from);
        host.control(to);
        assert_eq!(host.app.sketch.session.as_ref().unwrap().tool, tool);
    }
    host.xy(0.0, 0.0);
    host.numeric("80", "50");
    host.count(10).await;
    let accepted = host.sketch().clone();
    let (center, diagonal) = assert_center_rectangle(&accepted);
    host.app.ui.inspector_open = false;
    host.redraw();
    host.xy(55.0, 40.0);
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    host.history(false, 0).await;
    host.history(true, 10).await;
    assert_eq!(
        host.sketch(),
        &accepted,
        "center rectangle redo preserves all identities"
    );
    assert_eq!(assert_center_rectangle(host.sketch()), (center, diagonal));
    assert_small_center_defaults(&mut host).await;
    host.stop().await;
}

fn assert_center_rectangle(sketch: &Sketch) -> (EntityId, EntityId) {
    assert_eq!(sketch.constraints().len(), 7);
    let (center, diagonal) = sketch
        .constraints()
        .iter()
        .find_map(|constraint| match constraint {
            Constraint::Midpoint {
                point,
                point_position: CurvePosition::Arbitrary,
                line,
                ..
            } => Some((*point, *line)),
            _ => None,
        })
        .unwrap();
    let SketchEntity::Point {
        at, construction, ..
    } = sketch.get_entity(center).unwrap()
    else {
        panic!("center point expected")
    };
    assert!(*construction);
    near(at.x, 0.0);
    near(at.y, 0.0);
    let SketchEntity::Line {
        start,
        end,
        construction,
        ..
    } = sketch.get_entity(diagonal).unwrap()
    else {
        panic!("diagonal expected")
    };
    assert!(*construction);
    for id in [start, end] {
        let SketchEntity::Point { at, .. } = sketch.get_entity(*id).unwrap() else {
            panic!("diagonal corner expected")
        };
        near(at.x.abs(), 40.0);
        near(at.y.abs(), 25.0);
    }
    let mut dimensions: Vec<_> = sketch
        .constraints()
        .iter()
        .filter_map(|constraint| match constraint {
            Constraint::Distance { value, .. } => Some(value.value),
            _ => None,
        })
        .collect();
    dimensions.sort_by(f64::total_cmp);
    assert_eq!(dimensions, vec![50.0, 80.0]);
    assert_eq!(
        sketch
            .entities()
            .iter()
            .filter(|entity| matches!(
                entity,
                SketchEntity::Line {
                    construction: false,
                    ..
                }
            ))
            .count(),
        4
    );
    (center, diagonal)
}

async fn assert_small_center_defaults(host: &mut Host) {
    let before = host.sketch().clone();
    host.xy(55.0, 40.0);
    let pointer = host.project(55.2, 40.3);
    host.ui(vec![egui::Event::PointerMoved(pointer)]);
    host.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    assert!(crate::sketch_numeric::editor_open(&host.context));
    assert_eq!(host.sketch(), &before);
    host.ui(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    host.count(20).await;
    let added: Vec<_> = host
        .sketch()
        .entities()
        .iter()
        .filter(|entity| !before.contains_entity(entity.id()))
        .filter_map(|entity| match entity {
            SketchEntity::Point { at, .. } => Some(*at),
            _ => None,
        })
        .collect();
    let extent = |coordinate: fn(&Vec2) -> f64| {
        let min = added.iter().map(coordinate).fold(f64::INFINITY, f64::min);
        let max = added
            .iter()
            .map(coordinate)
            .fold(f64::NEG_INFINITY, f64::max);
        max - min
    };
    near(extent(|point| point.x), 0.4);
    near(extent(|point| point.y), 0.6);
    assert!(
        !host
            .sketch()
            .constraints()
            .iter()
            .filter(|constraint| !before.contains_constraint(constraint.id()))
            .any(|constraint| matches!(constraint, Constraint::Distance { .. })),
        "measured defaults must not become driving dimensions"
    );
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn numeric_modal_refuses_enter_and_reopen_after_viewport_failure() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Point");
    host.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    assert!(crate::sketch_numeric::editor_open(&host.context));
    host.replace("-1");
    let accepted = host.sketch().clone();
    let session = host.app.sketch.session.as_ref().unwrap();
    let history = session.history.clone();
    let revision = session.document.revision;
    host.app
        .fail_viewport("Injected native viewport failure".into());
    host.ui(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert!(!crate::sketch_numeric::editor_open(&host.context));
    for _ in 0..3 {
        host.ui(vec![]);
    }
    host.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    assert!(!crate::sketch_numeric::editor_open(&host.context));
    let session = host.app.sketch.session.as_ref().unwrap();
    assert_eq!(&session.authoritative, &accepted);
    assert_eq!(session.history, history);
    assert_eq!(session.document.revision, revision);
    assert_eq!(session.phase, SketchPhase::Active);
    host.stop().await;
}
