//! Extend geometry, identity and refusals.

use std::f64::consts::{FRAC_PI_2, PI};

use super::*;
use crate::document::variables::Scalar;
use crate::sketch::modify::{
    plan_extend, AddReason, DropReason, ExtendPlan, ExtendRequest, ModifyRefusal,
};

fn extend(s: &Sketch, entity: EntityId, x: f64, y: f64) -> Result<ExtendPlan, ModifyRefusal> {
    plan_extend(
        s,
        &ExtendRequest {
            entity,
            pick: v(x, y),
        },
        &mut SeqIds::new(),
    )
}

/// Line (0,0)→(4,0) with walls at x=6, x=9 and x=−3.
fn walled() -> (B, EntityId, EntityId, EntityId, EntityId, EntityId) {
    let mut b = B::new();
    let (l, s, e) = b.line((0.0, 0.0), (4.0, 0.0));
    let (near_wall, _, _) = b.line((6.0, -5.0), (6.0, 5.0));
    b.line((9.0, -5.0), (9.0, 5.0));
    let (back, _, _) = b.line((-3.0, -5.0), (-3.0, 5.0));
    (b, l, s, e, near_wall, back)
}

#[test]
fn a_line_end_grows_to_the_nearest_boundary_keeping_its_point() {
    let (mut b, l, s, e, near_wall, _) = walled();
    let h = b.horizontal(l);
    let grown = extend(&b.s, l, 3.5, 0.2).unwrap();
    let after = apply(&b.s, &grown.plan);
    check_accounting(&b.s, &after, &grown.plan, l, &[e]);
    assert_eq!(grown.end, CurvePosition::End);
    assert!(near(grown.tip, 6.0, 0.0));
    let (a, z) = ends(&after, l);
    assert!(near(a, 0.0, 0.0) && near(z, 6.0, 0.0));
    assert!(matches!(
        after.get_entity(l),
        Some(SketchEntity::Line { start, end, .. }) if (*start, *end) == (s, e)
    ));
    assert_eq!(after.get_constraint(h), b.s.get_constraint(h));
    assert_eq!(
        grown.plan.added_constraints[0].reason,
        AddReason::ExtendWeld {
            boundary: near_wall
        }
    );
}

#[test]
fn the_pick_selects_the_start_end() {
    let (b, l, s, _, _, back) = walled();
    let grown = extend(&b.s, l, 0.5, 0.0).unwrap();
    let after = apply(&b.s, &grown.plan);
    check_accounting(&b.s, &after, &grown.plan, l, &[s]);
    assert_eq!(grown.end, CurvePosition::Start);
    assert!(near(point(&after, s), -3.0, 0.0));
    assert_eq!(
        grown.plan.added_constraints[0].reason,
        AddReason::ExtendWeld { boundary: back }
    );
}

#[test]
fn growing_onto_a_boundary_end_point_welds_coincident() {
    let mut b = B::new();
    let (l, _, e) = b.line((0.0, 0.0), (4.0, 0.0));
    let (_, _, foot) = b.line((6.0, 5.0), (6.0, 0.0));
    let grown = extend(&b.s, l, 4.0, 0.0).unwrap();
    let after = apply(&b.s, &grown.plan);
    check_accounting(&b.s, &after, &grown.plan, l, &[e]);
    let weld = after
        .get_constraint(grown.plan.added_constraints[0].id)
        .unwrap();
    assert!(matches!(
        weld,
        Constraint::Coincident { point1, point2, .. } if (*point1, *point2) == (e, foot)
    ));
}

#[test]
fn an_arc_end_grows_around_its_circle_keeping_its_constraints() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, FRAC_PI_2);
    let (wall, _, _) = b.line((-10.0, 0.0), (0.0, 0.0));
    let radius = b.cid();
    b.constrain(Constraint::Radius {
        id: radius,
        entity: arc,
        value: Scalar::new(5.0),
    });
    let grown = extend(&b.s, arc, 0.5, 5.0).unwrap();
    let after = apply(&b.s, &grown.plan);
    check_accounting(&b.s, &after, &grown.plan, arc, &[]);
    assert_eq!(grown.end, CurvePosition::End);
    let (a0, a1) = arc_angles(&after, arc);
    assert!(a0.abs() < 1e-12 && (a1 - PI).abs() < 1e-9, "{a0} {a1}");
    assert_eq!(grown.plan.kept_constraints, vec![radius]);
    assert_eq!(after.get_constraint(radius), b.s.get_constraint(radius));
    assert!(matches!(
        after.get_constraint(grown.plan.added_constraints[0].id),
        Some(Constraint::OnCurve { point, point_position: CurvePosition::End, curve, .. })
            if *point == arc && *curve == wall
    ));
}

#[test]
fn an_arc_never_wraps_onto_itself() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    // Crosses the circle only inside the arc's own span.
    b.line((2.0, -1.0), (2.0, 10.0));
    assert_eq!(
        extend(&b.s, arc, 4.9, 0.5).unwrap_err(),
        ModifyRefusal::NoBoundaryHit(arc)
    );
}

#[test]
fn refusals_name_the_blocker_and_draw_no_ids() {
    let (mut b, l, s, e, _, _) = walled();
    let fixed = b.fixed(e, CurvePosition::Arbitrary);
    let (c, _) = b.circle((30.0, 0.0), 2.0);
    let (lonely, _, _) = b.line((0.0, 40.0), (1.0, 40.0));
    let (touch, _, _) = b.line((0.0, 60.0), (1.0, 60.0));
    b.circle((5.0, 62.0), 2.0);
    let shared_end = b.pt(0.0, 3.0);
    b.seg(s, shared_end);
    let cases = [
        (
            l,
            v(3.0, 0.0),
            ModifyRefusal::EndpointConstrained {
                entity: l,
                constraints: vec![fixed],
            },
        ),
        (
            l,
            v(1.0, 0.0),
            ModifyRefusal::EndpointShared {
                entity: l,
                point: s,
            },
        ),
        (l, v(2.0, 0.0), ModifyRefusal::AmbiguousPick(l)),
        (lonely, v(1.0, 40.0), ModifyRefusal::NoBoundaryHit(lonely)),
        (touch, v(1.0, 60.0), ModifyRefusal::TangentHit(touch)),
        (
            c,
            v(32.0, 0.0),
            ModifyRefusal::UnsupportedKind {
                entity: c,
                kind: "closed circle",
                action: "extended",
            },
        ),
    ];
    for (entity, pick, want) in cases {
        let mut ids = SeqIds::new();
        let got = plan_extend(&b.s, &ExtendRequest { entity, pick }, &mut ids).unwrap_err();
        assert_eq!(got, want);
        assert_eq!(ids.drawn, 0);
    }
}

#[test]
fn a_length_relation_on_the_line_blocks_the_extend() {
    let (mut b, l, _, _, _, back) = walled();
    let eq = b.cid();
    b.constrain(Constraint::Equal {
        id: eq,
        entity1: back,
        entity2: l,
    });
    assert_eq!(
        extend(&b.s, l, 3.5, 0.0).unwrap_err(),
        ModifyRefusal::EndpointConstrained {
            entity: l,
            constraints: vec![eq],
        }
    );
}

#[test]
fn a_locked_end_point_is_never_moved() {
    let mut b = B::new();
    let s = b.pt(0.0, 0.0);
    let id = EntityId(Uuid::from_u128(0xE0));
    b.add(SketchEntity::point(id, v(4.0, 0.0), false, true));
    let l = b.seg(s, id);
    b.line((6.0, -5.0), (6.0, 5.0));
    assert_eq!(
        extend(&b.s, l, 3.5, 0.0).unwrap_err(),
        ModifyRefusal::ReferenceLocked(id)
    );
}

#[test]
fn landing_on_a_foot_supersedes_its_on_curve_for_lines_and_arcs() {
    let mut b = B::new();
    let (l, _, e) = b.line((0.0, 0.0), (5.0, 0.0));
    let (_, _, foot) = b.line((8.0, 5.0), (8.0, 0.0));
    let on = b.on_curve(foot, l);
    let grown = extend(&b.s, l, 4.9, 0.0).unwrap();
    let after = apply(&b.s, &grown.plan);
    check_accounting(&b.s, &after, &grown.plan, l, &[e]);
    assert_eq!(
        dropped_reason(&grown.plan, on),
        Some(DropReason::SupersededByExtendWeld)
    );

    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, FRAC_PI_2);
    // Not tangent at (−5,0): it meets the circle there transversally.
    let (stem, _, _) = b.line((-8.0, 4.0), (-5.0, 0.0));
    let on = b.on_curve_at(stem, CurvePosition::End, arc);
    let radius = b.cid();
    b.constrain(Constraint::Radius {
        id: radius,
        entity: arc,
        value: Scalar::new(5.0),
    });
    let grown = extend(&b.s, arc, 0.5, 5.0).unwrap();
    let after = apply(&b.s, &grown.plan);
    check_accounting(&b.s, &after, &grown.plan, arc, &[]);
    assert_eq!(
        dropped_reason(&grown.plan, on),
        Some(DropReason::SupersededByExtendWeld)
    );
    assert_eq!(grown.plan.kept_constraints, vec![radius]);
    let (_, a1) = arc_angles(&after, arc);
    assert!((a1 - PI).abs() < 1e-9, "{a1}");
}

#[test]
fn an_extend_weld_supersedes_on_curve_on_any_coincident_member() {
    // C2: whichever member the weld picks, the OnCurve on the other restates it.
    for (arc_first, name_arc) in [(false, true), (false, false), (true, true), (true, false)] {
        let mut b = B::new();
        let (l, _, e) = b.line((0.0, 0.0), (5.0, 0.0));
        let on = junction(&mut b, l, arc_first, name_arc);
        let grown = extend(&b.s, l, 4.9, 0.0).unwrap();
        assert!(near(grown.tip, 8.0, 0.0));
        let after = apply(&b.s, &grown.plan);
        check_accounting(&b.s, &after, &grown.plan, l, &[e]);
        assert_eq!(
            dropped_reason(&grown.plan, on),
            Some(DropReason::SupersededByExtendWeld),
            "arc_first {arc_first}, name_arc {name_arc}"
        );
    }
}

#[test]
fn an_arc_growing_into_an_arc_of_its_own_circle_is_refused() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, FRAC_PI_2);
    let (boundary, _) = b.arc((0.0, 0.0), 5.0, 0.75 * PI, PI);
    b.line((-10.0, -1.0), (10.0, -1.0));
    assert_eq!(
        extend(&b.s, arc, 0.5, 5.0).unwrap_err(),
        ModifyRefusal::OverlapsBoundary {
            entity: arc,
            boundary
        }
    );
}

#[test]
fn a_collinear_boundary_stops_the_extend_or_refuses_when_already_on_it() {
    let mut b = B::new();
    let (l, _, e) = b.line((0.0, 0.0), (5.0, 0.0));
    let (_, p, _) = b.line((8.0, 0.0), (12.0, 0.0));
    b.line((20.0, -5.0), (20.0, 5.0));
    let grown = extend(&b.s, l, 4.9, 0.0).unwrap();
    let after = apply(&b.s, &grown.plan);
    check_accounting(&b.s, &after, &grown.plan, l, &[e]);
    assert!(near(grown.tip, 8.0, 0.0));
    assert!(matches!(
        after.get_constraint(grown.plan.added_constraints[0].id),
        Some(Constraint::Coincident { point2, .. }) if *point2 == p
    ));
    // A collinear segment that already starts at the end would be run along.
    let mut b = B::new();
    let (l, _, _) = b.line((0.0, 0.0), (5.0, 0.0));
    let (touching, _, _) = b.line((5.0, 0.0), (9.0, 0.0));
    b.line((20.0, -5.0), (20.0, 5.0));
    assert_eq!(
        extend(&b.s, l, 4.9, 0.0).unwrap_err(),
        ModifyRefusal::OverlapsBoundary {
            entity: l,
            boundary: touching
        }
    );
}

#[test]
fn an_arc_pick_half_way_round_its_gap_or_at_its_centre_is_ambiguous() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, FRAC_PI_2);
    b.line((-10.0, 0.0), (0.0, 0.0));
    let anti = 1.25 * PI;
    for d in [0.0, 1e-7, -1e-7] {
        let (x, y) = (5.0 * (anti + d).cos(), 5.0 * (anti + d).sin());
        assert_eq!(
            extend(&b.s, arc, x, y).unwrap_err(),
            ModifyRefusal::AmbiguousPick(arc),
            "gap pick offset {d}"
        );
    }
    assert_eq!(
        extend(&b.s, arc, 0.0, 0.0).unwrap_err(),
        ModifyRefusal::AmbiguousPick(arc)
    );
}
