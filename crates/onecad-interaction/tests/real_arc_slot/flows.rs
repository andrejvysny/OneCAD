//! Arc and Line->tangent-arc chain workflows (see the parent for slot).
use super::*;
use crate::geom::{endpoint, pt};
use onecad_core::sketch::CurvePosition;
use std::f64::consts::FRAC_PI_2;

fn near(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

pub fn assert_arcs(sketch: &Sketch, radius: f64) {
    let p = ids(1000);
    let (_, r, start, end) = arc(sketch, p.entities[1]);
    near(r, radius, 1e-6);
    assert!(end > start && end - start < std::f64::consts::TAU);
    for role in [CurvePosition::Start, CurvePosition::End] {
        let e = endpoint(sketch, p.entities[1], role);
        near(geom::dist(e, arc(sketch, p.entities[1]).0), radius, 1e-6);
    }
    let (c2, r2, s2, e2) = arc(sketch, ids(3000).entities[1]);
    assert!(geom::dist(c2, [40.0, 0.0]) <= 1e-6);
    near(r2, 7.0, 1e-6);
    near(e2 - s2, FRAC_PI_2, 1e-6);
    assert_no_orphans(sketch);
    assert!(sketch.get_constraint(ids(1000).constraints[8]).is_some());
    assert!(sketch.get_constraint(ids(3000).constraints[8]).is_some());
}

pub async fn arc_flow(root: &Path) {
    let harness = new(root, "host").await;
    let mut state = begin(&harness, true).await;
    state.dispatch(SketchAction::SetTool(SketchTool::Arc));
    assert!(click_at(&mut state, xy(0.0, 0.0), 500).is_empty());
    assert!(click_at(&mut state, xy(10.0, 0.0), 500).is_empty());
    let effects = click_at(&mut state, xy(0.0, 10.0), 1000);
    let solve = once(&harness, &mut state, effects).await;
    eprintln!("arc bare dof {} {:?}", solve.dof, solve.status);
    assert_eq!(solve.dof, 5);
    let (center, r, start, end) = arc(&sketch(&harness).await, ids(1000).entities[1]);
    assert!(geom::dist(center, [0.0, 0.0]) <= 1e-6);
    near(r, 10.0, 1e-6);
    near(start, 0.0, 1e-9);
    near(end, FRAC_PI_2, 1e-9);
    let radius = Constraint::Radius {
        id: ids(1000).constraints[8],
        entity: ids(1000).entities[1],
        value: Scalar::new(10.0),
    };
    add(&harness, &mut state, vec![radius], 4).await;
    let c = ids(1000).constraints[8];
    edit_with_history(&harness, &mut state, c, 14.0, |s| {
        let (c, r, start, end) = arc(s, ids(1000).entities[1]);
        eprintln!("arc after radius 14: center {c:?} r {r} start {start} end {end}");
        near(r, 14.0, 1e-6);
        // Observed: PlaneGCS keeps the endpoints' proximity, not the angles.
        assert!(end > start && end - start < std::f64::consts::TAU);
    })
    .await;
    numeric_arc(&harness, &mut state).await;
    regions(&harness, 0).await;
    assert_arcs(&sketch(&harness).await, 14.0);
    close_visit(&harness).await;
    record_and_save(&harness, root, "arc").await;
    harness.close().await;
}

async fn numeric_arc(harness: &Harness, state: &mut SketchInteraction) {
    assert!(click_at(state, xy(40.0, 0.0), 500).is_empty());
    assert!(click_at(state, xy(45.0, 0.0), 500).is_empty());
    let session = state.session.as_ref().unwrap();
    let (key, capture) = (session.key.clone(), session.accepted_capture.unwrap());
    let before = sketch(harness).await;
    let effects = state.dispatch(SketchAction::CreateNumeric {
        key,
        capture,
        ids: ids(3000),
        dimensions: Box::new(CreationDimensions::Arc {
            radius: Scalar::new(7.0),
            sweep: FRAC_PI_2,
        }),
    });
    let solve = once(harness, state, effects).await;
    eprintln!("numeric arc total dof {}", solve.dof);
    assert_eq!(solve.dof, 8);
    history(harness, state, HistoryDirection::Undo).await;
    assert_eq!(sketch(harness).await, before);
    history(harness, state, HistoryDirection::Redo).await;
}

pub fn assert_chain(sketch: &Sketch) {
    let (first, second, third) = (ids(1000), ids(2000), ids(3000));
    assert_eq!(welds(sketch), 2);
    let t = tangents(sketch);
    assert_eq!(t.len(), 1);
    assert!(t[0].0 > 0.99 && t[0].1.abs() <= 1e-6, "{t:?}");
    assert_no_orphans(sketch);
    let (center, r, start, end) = arc(sketch, second.entities[1]);
    assert!(geom::dist(center, [20.0, 10.0]) <= 1e-6);
    near(r, 10.0, 1e-6);
    near(end - start, std::f64::consts::PI, 1e-6);
    let arc_end = endpoint(sketch, second.entities[1], CurvePosition::End);
    assert!(geom::dist(arc_end, pt(sketch, third.entities[0])) <= 1e-6);
    let closing = line(sketch, ids(4000).entities[2]);
    assert!(geom::dist(closing.1, pt(sketch, first.entities[0])) <= 1e-6);
    assert!(matches!(
        sketch.get_entity(ids(4000).entities[2]),
        Some(SketchEntity::Line { end, .. }) if *end == first.entities[0]
    ));
}

pub async fn chain_flow(root: &Path) {
    let harness = new(root, "host").await;
    let mut state = begin(&harness, true).await;
    state.dispatch(SketchAction::SetTool(SketchTool::Line));
    assert!(click_at(&mut state, xy(0.0, 0.0), 500).is_empty());
    let effects = click_at(&mut state, xy(20.0, 0.0), 1000);
    once(&harness, &mut state, effects).await;
    let key = state.session.as_ref().unwrap().key.clone();
    state.dispatch(SketchAction::SetTangentArc { key, enabled: true });
    let effects = click_at(&mut state, xy(20.0, 20.0), 2000);
    once(&harness, &mut state, effects).await;
    let effects = click_at(&mut state, xy(0.0, 20.0), 3000);
    once(&harness, &mut state, effects).await;
    let effects = click_at(&mut state, xy(0.0, 0.0), 4000);
    let solve = once(&harness, &mut state, effects).await;
    eprintln!("chain closed dof {} {:?}", solve.dof, solve.status);
    assert_chain(&sketch(&harness).await);
    regions(&harness, 1).await;
    finish_extrude_save(&harness, &mut state, root, "chain").await;
    harness.close().await;
}
