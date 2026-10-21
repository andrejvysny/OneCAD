//! Trim geometry, identity and refusals.

use std::f64::consts::{FRAC_PI_2, PI};

use super::*;
use crate::sketch::modify::{
    plan_trim, AddReason, DropReason, ModifyRefusal, TrimRequest, MIN_PIECE_MM,
};

pub(super) fn trim(
    s: &Sketch,
    entity: EntityId,
    x: f64,
    y: f64,
) -> Result<ModifyPlan, ModifyRefusal> {
    plan_trim(
        s,
        &TrimRequest {
            entity,
            pick: v(x, y),
        },
        &mut SeqIds::new(),
    )
}

/// Line (0,0)→(10,0) crossed by vertical lines at x=3 and x=7.
fn crossed_line() -> (B, EntityId, EntityId, EntityId, EntityId, EntityId) {
    let mut b = B::new();
    let (l, s, e) = b.line((0.0, 0.0), (10.0, 0.0));
    let (v3, _, _) = b.line((3.0, -5.0), (3.0, 5.0));
    let (v7, _, _) = b.line((7.0, -5.0), (7.0, 5.0));
    (b, l, s, e, v3, v7)
}

#[test]
fn middle_trim_splits_a_line_and_keeps_the_start_piece_id() {
    let (mut b, l, s, e, v3, v7) = crossed_line();
    let h = b.horizontal(l);
    let plan = trim(&b.s, l, 5.0, 0.1).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);

    assert_eq!(plan.pieces.len(), 2);
    let (first, second) = (plan.pieces[0], plan.pieces[1]);
    assert!(first.keeps_target_id && first.owns_start && !first.owns_end);
    assert!(!second.keeps_target_id && second.owns_end && !second.owns_start);
    let (a, cut) = ends(&after, l);
    assert!(near(a, 0.0, 0.0) && near(cut, 3.0, 0.0));
    let (cut2, z) = ends(&after, second.id);
    assert!(near(cut2, 7.0, 0.0) && near(z, 10.0, 0.0));
    // Unchanged endpoints keep their point ids; cut endpoints are new points.
    match (after.get_entity(l), after.get_entity(second.id)) {
        (Some(SketchEntity::Line { start, .. }), Some(SketchEntity::Line { end, .. })) => {
            assert_eq!((*start, *end), (s, e));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(plan.added_points.len(), 2);
    // Horizontal copies onto both pieces: kept on the first, a copy on the second.
    assert_eq!(plan.kept_constraints, vec![h]);
    let copies: Vec<_> = plan
        .added_constraints
        .iter()
        .filter(|a| a.reason == AddReason::CopiedFrom { of: h })
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(
        after.get_constraint(copies[0].id),
        Some(&Constraint::Horizontal {
            id: copies[0].id,
            line: second.id
        })
    );
    // Each cut end is held OnCurve by the boundary that cut it.
    let welds: Vec<_> = plan
        .added_constraints
        .iter()
        .filter_map(|a| match a.reason {
            AddReason::CutWeld { boundary } => Some(boundary),
            _ => None,
        })
        .collect();
    assert_eq!(welds, vec![v3, v7]);
}

#[test]
fn end_trim_removes_the_orphaned_end_point_and_its_constraints() {
    let (mut b, l, _, e, _, _) = crossed_line();
    let fixed = b.fixed(e, CurvePosition::Arbitrary);
    let plan = trim(&b.s, l, 9.0, 0.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);
    assert_eq!(plan.pieces.len(), 1);
    assert!(plan.pieces[0].keeps_target_id);
    assert_eq!(plan.removed_entities, vec![e]);
    assert_eq!(dropped_reason(&plan, fixed), Some(DropReason::PointRemoved));
    let (_, cut) = ends(&after, l);
    assert!(near(cut, 7.0, 0.0));
}

#[test]
fn head_trim_gives_the_only_piece_a_fresh_id_and_keeps_a_shared_start() {
    let (mut b, l, s, _, _, _) = crossed_line();
    let other_end = b.pt(0.0, 5.0);
    let corner = b.seg(s, other_end);
    let plan = trim(&b.s, l, 1.0, 0.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);
    assert_eq!(plan.pieces.len(), 1);
    assert!(!plan.pieces[0].keeps_target_id && plan.pieces[0].owns_end);
    assert!(plan.removed_entities == vec![l], "shared start point stays");
    assert!(after.contains_entity(s) && after.contains_entity(corner));
}

#[test]
fn a_cut_at_a_boundary_end_point_welds_coincident_and_supersedes_on_curve() {
    let mut b = B::new();
    let (l, _, _) = b.line((0.0, 0.0), (10.0, 0.0));
    let (stem, _, foot) = b.line((5.0, 5.0), (5.0, 0.0));
    let on = b.on_curve(foot, l);
    let plan = trim(&b.s, l, 8.0, 0.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);
    assert_eq!(
        dropped_reason(&plan, on),
        Some(DropReason::SupersededByCutWeld)
    );
    let weld = plan
        .added_constraints
        .iter()
        .find(|a| a.reason == AddReason::CutWeld { boundary: stem })
        .unwrap();
    assert!(matches!(
        after.get_constraint(weld.id),
        Some(Constraint::Coincident { point2, .. }) if *point2 == foot
    ));
}

#[test]
fn an_uncrossed_line_is_deleted_whole_with_its_orphaned_points() {
    let mut b = B::new();
    let (l, s, e) = b.line((0.0, 0.0), (10.0, 0.0));
    let h = b.horizontal(l);
    let plan = trim(&b.s, l, 5.0, 0.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);
    assert!(plan.pieces.is_empty());
    assert_eq!(plan.removed_entities, vec![l, s, e]);
    assert_eq!(dropped_reason(&plan, h), Some(DropReason::EntityDeleted));
    assert!(after.entities().is_empty());
}

#[test]
fn a_circle_cut_twice_becomes_one_fresh_arc_keeping_its_dimensions() {
    let mut b = B::new();
    let (c, center) = b.circle((0.0, 0.0), 5.0);
    b.line((-10.0, 3.0), (10.0, 3.0));
    let radius = b.cid();
    b.constrain(Constraint::Radius {
        id: radius,
        entity: c,
        value: crate::document::variables::Scalar::new(5.0),
    });
    // Pick the top cap (above y=3): the lower 3/4 of the circle survives.
    let plan = trim(&b.s, c, 0.0, 5.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, c, &[]);
    assert_eq!(plan.pieces.len(), 1);
    let arc = plan.pieces[0];
    assert!(
        !arc.keeps_target_id,
        "circle → arc is a kind change: fresh id"
    );
    assert!(!after.contains_entity(c) && after.contains_entity(center));
    let (a0, a1) = arc_angles(&after, arc.id);
    let top = (3.0f64 / 5.0).asin();
    assert!(
        (a0 - (PI - top)).abs() < 1e-9 && (a1 - top).abs() < 1e-9,
        "{a0} {a1}"
    );
    assert!(matches!(
        after.get_constraint(radius),
        Some(Constraint::Radius { entity, .. }) if *entity == arc.id
    ));
    assert_eq!(plan.kept_constraints, vec![radius]);
}

#[test]
fn a_circle_touched_once_is_deleted_whole_with_its_center() {
    let mut b = B::new();
    let (c, center) = b.circle((0.0, 0.0), 5.0);
    b.line((-10.0, 5.0), (10.0, 5.0));
    let plan = trim(&b.s, c, 0.0, -5.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, c, &[]);
    assert!(plan.pieces.is_empty());
    assert_eq!(plan.removed_entities, vec![c, center]);
}

#[test]
fn a_small_circle_on_an_ellipse_is_cut_not_deleted_whole() {
    // Both crossings fall inside one 0.5 deg ellipse seed interval.
    let phi = 0.25f64.to_radians();
    let (cx, cy, r) = (100.0 * phi.cos(), 50.0 * phi.sin(), 0.1);
    let mut b = B::new();
    b.ellipse((0.0, 0.0), 100.0, 50.0, 0.0);
    let (c, center) = b.circle((cx, cy), r);
    let plan = trim(&b.s, c, cx - r, cy).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, c, &[]);
    assert_eq!(plan.pieces.len(), 1, "the outer arc survives");
    assert_eq!(plan.removed_entities, vec![c]);
    assert!(after.contains_entity(center));
}

#[test]
fn a_split_arc_keeps_its_id_on_the_start_piece_and_moves_end_roles() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    let (cross, _, _) = b.line((0.0, -10.0), (0.0, 10.0));
    let (tail, tail_end) = (b.pt(-5.0, 0.0), b.pt(-5.0, -4.0));
    b.seg(tail, tail_end);
    let weld = b.cid();
    b.constrain(Constraint::Coincident {
        id: weld,
        point1: arc,
        point2: tail,
        point1_position: CurvePosition::End,
        point2_position: CurvePosition::Arbitrary,
    });
    let plan = trim(&b.s, arc, 5.0 * 0.25f64.cos(), 5.0 * 0.25f64.sin());
    // Picking the first quarter removes it: one piece from the cut to the end.
    let plan = plan.unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, arc, &[]);
    assert_eq!(plan.pieces.len(), 1);
    let piece = plan.pieces[0];
    assert!(!piece.keeps_target_id && piece.owns_end);
    let (a0, a1) = arc_angles(&after, piece.id);
    assert!((a0 - FRAC_PI_2).abs() < 1e-9 && (a1 - PI).abs() < 1e-9);
    assert!(matches!(
        after.get_constraint(weld),
        Some(Constraint::Coincident { point1, .. }) if *point1 == piece.id
    ));
    let _ = cross;
}

#[test]
fn an_arc_middle_trim_copies_radius_but_concentric_only_once() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    b.line((-2.0, -10.0), (-2.0, 10.0));
    b.line((2.0, -10.0), (2.0, 10.0));
    let (other, _) = b.circle((0.0, 0.0), 9.0);
    let (radius, conc) = (b.cid(), b.cid());
    b.constrain(Constraint::Radius {
        id: radius,
        entity: arc,
        value: crate::document::variables::Scalar::new(5.0),
    });
    b.constrain(Constraint::Concentric {
        id: conc,
        entity1: arc,
        entity2: other,
    });
    let plan = trim(&b.s, arc, 0.0, 5.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, arc, &[]);
    assert_eq!(plan.pieces.len(), 2);
    assert!(plan.pieces[0].keeps_target_id);
    let copied = |of| {
        plan.added_constraints
            .iter()
            .filter(|a| a.reason == AddReason::CopiedFrom { of })
            .count()
    };
    assert_eq!((copied(radius), copied(conc)), (1, 0));
    assert_eq!(plan.kept_constraints, vec![radius, conc]);
}

#[test]
fn every_curve_kind_bounds_a_trim_but_points_do_not() {
    let mut b = B::new();
    let (l, _, _) = b.line((0.0, 0.0), (20.0, 0.0));
    let (construction, _, _) = b.line((2.0, -1.0), (2.0, 1.0));
    b.s = rebuild_with(&b.s, construction, |e| match e {
        SketchEntity::Line { construction, .. } => *construction = true,
        _ => unreachable!(),
    });
    let (locked, _, _) = b.line((6.0, -1.0), (6.0, 1.0));
    b.s = rebuild_with(&b.s, locked, |e| e.set_reference_locked(true));
    b.ellipse((14.0, 0.0), 2.0, 1.0, 0.0);
    b.pt(9.0, 0.0);
    let plan = trim(&b.s, l, 9.0, 0.5).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);
    let (_, left) = ends(&after, l);
    let (right, _) = ends(&after, plan.pieces[1].id);
    assert!(near(left, 6.0, 0.0), "locked line cuts: {left:?}");
    assert!(
        near(right, 12.0, 0.0),
        "ellipse cuts, the point does not: {right:?}"
    );
}

fn rebuild_with(s: &Sketch, id: EntityId, f: impl Fn(&mut SketchEntity)) -> Sketch {
    let mut out = Sketch::on_world_plane(s.id, "S", WorldPlane::XY);
    for e in s.entities() {
        let mut e = e.clone();
        if e.id() == id {
            f(&mut e);
        }
        out.add_entity(e).unwrap();
    }
    out
}

#[test]
fn refusals_name_the_reason_and_draw_no_ids() {
    let (mut b, l, s, _, _, _) = crossed_line();
    let ell = b.ellipse((30.0, 0.0), 3.0, 1.0, 0.0);
    let (locked, _, _) = b.line((0.0, 20.0), (10.0, 20.0));
    b.s = rebuild_with(&b.s, locked, |e| e.set_reference_locked(true));
    let unknown = EntityId(Uuid::from_u128(0xDEAD));
    let cases = [
        (unknown, v(0.0, 0.0), ModifyRefusal::UnknownEntity(unknown)),
        (s, v(0.0, 0.0), ModifyRefusal::NotACurve(s)),
        (locked, v(5.0, 20.0), ModifyRefusal::ReferenceLocked(locked)),
        (
            ell,
            v(33.0, 0.0),
            ModifyRefusal::UnsupportedKind {
                entity: ell,
                kind: "ellipse",
                action: "trimmed",
            },
        ),
        (l, v(3.0, 0.0), ModifyRefusal::AmbiguousPick(l)),
    ];
    for (entity, pick, want) in cases {
        let mut ids = SeqIds::new();
        let got = plan_trim(&b.s, &TrimRequest { entity, pick }, &mut ids).unwrap_err();
        assert_eq!(got, want);
        assert_eq!(ids.drawn, 0, "a refusal draws no ids");
    }
}

#[test]
fn a_piece_shorter_than_the_minimum_is_refused_not_absorbed() {
    let mut b = B::new();
    let (l, _, _) = b.line((0.0, 0.0), (10.0, 0.0));
    let x = 10.0 - MIN_PIECE_MM / 2.0;
    b.line((x, -1.0), (x, 1.0));
    assert_eq!(
        trim(&b.s, l, 5.0, 0.0).unwrap_err(),
        ModifyRefusal::DegeneratePiece(l)
    );
    // Trimming the stub itself is fine.
    assert!(trim(&b.s, l, 10.0 - MIN_PIECE_MM / 4.0, 0.0).is_ok());
}

#[test]
fn a_pick_past_a_line_end_selects_the_end_span_however_far() {
    let (b, l, _, e, _, _) = crossed_line();
    for x in [10.5, 40.0, 70.0] {
        let plan = trim(&b.s, l, x, 0.0).unwrap();
        assert_eq!(plan.removed_entities, vec![e], "pick at x={x}");
    }
    let plan = trim(&b.s, l, -50.0, 0.0).unwrap();
    assert!(
        !plan.pieces[0].owns_start,
        "a pick before the start trims the head"
    );
}

#[test]
fn a_pick_past_an_arc_end_selects_the_nearer_end_around_the_circle() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    b.line((0.0, -10.0), (0.0, 10.0));
    // Just below the start (angle −0.2) is nearer the start than the end.
    let start_side = trim(&b.s, arc, 5.0 * 0.2f64.cos(), -5.0 * 0.2f64.sin()).unwrap();
    assert!(!start_side.pieces[0].owns_start);
    let end_side = trim(&b.s, arc, -5.0 * 0.2f64.cos(), -5.0 * 0.2f64.sin()).unwrap();
    assert!(!end_side.pieces[0].owns_end);
}

#[test]
fn a_role_named_foot_at_a_cut_is_superseded_not_restated() {
    // Line foot named by the stem's End role, arc foot by the arc's Start role.
    let mut b = B::new();
    let (l, _, _) = b.line((0.0, 0.0), (10.0, 0.0));
    let (stem, _, _) = b.line((2.0, 5.0), (2.0, 0.0));
    let stem_on = b.on_curve_at(stem, CurvePosition::End, l);
    let (arc, _) = b.arc((11.0, 0.0), 3.0, PI, 1.5 * PI);
    let arc_on = b.on_curve_at(arc, CurvePosition::Start, l);
    let plan = trim(&b.s, l, 5.0, 0.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);
    for on in [stem_on, arc_on] {
        assert_eq!(
            dropped_reason(&plan, on),
            Some(DropReason::SupersededByCutWeld)
        );
    }
    let welded_arc = plan.added_constraints.iter().any(|a| {
        a.reason == AddReason::CutWeld { boundary: arc }
            && matches!(
                after.get_constraint(a.id),
                Some(Constraint::Coincident { point2, point2_position: CurvePosition::Start, .. })
                    if *point2 == arc
            )
    });
    assert!(welded_arc, "{plan:#?}");
}

#[test]
fn a_cut_weld_supersedes_on_curve_on_any_coincident_member() {
    for (arc_first, name_arc) in [(false, true), (false, false), (true, true), (true, false)] {
        let mut b = B::new();
        let (l, _, _) = b.line((0.0, 0.0), (10.0, 0.0));
        let on = junction(&mut b, l, arc_first, name_arc);
        let plan = trim(&b.s, l, 9.0, 0.0).unwrap();
        let after = apply(&b.s, &plan);
        check_accounting(&b.s, &after, &plan, l, &[]);
        assert_eq!(
            dropped_reason(&plan, on),
            Some(DropReason::SupersededByCutWeld),
            "arc_first {arc_first}, name_arc {name_arc}"
        );
    }
}

#[test]
fn a_cut_weld_supersedes_on_curve_through_a_centre_role() {
    for role_in_weld in [true, false] {
        let mut b = B::new();
        let (l, _, _) = b.line((0.0, 0.0), (10.0, 0.0));
        let on = centre_junction(&mut b, l, role_in_weld);
        let plan = trim(&b.s, l, 9.0, 0.0).unwrap();
        let after = apply(&b.s, &plan);
        check_accounting(&b.s, &after, &plan, l, &[]);
        assert_eq!(
            dropped_reason(&plan, on),
            Some(DropReason::SupersededByCutWeld),
            "role_in_weld {role_in_weld}"
        );
    }
}

#[test]
fn a_pick_half_way_round_an_arc_gap_or_at_a_centre_is_ambiguous() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, FRAC_PI_2);
    b.line((3.0, -10.0), (3.0, 10.0));
    let (circle, _) = b.circle((30.0, 0.0), 5.0);
    b.line((30.0, -10.0), (30.0, 10.0));
    let anti = 1.25 * PI;
    for d in [0.0, 1e-7, -1e-7] {
        let (x, y) = (5.0 * (anti + d).cos(), 5.0 * (anti + d).sin());
        assert_eq!(
            trim(&b.s, arc, x, y).unwrap_err(),
            ModifyRefusal::AmbiguousPick(arc),
            "gap pick offset {d}"
        );
    }
    assert_eq!(
        trim(&b.s, arc, 0.0, 0.0).unwrap_err(),
        ModifyRefusal::AmbiguousPick(arc)
    );
    assert_eq!(
        trim(&b.s, circle, 30.0, 0.0).unwrap_err(),
        ModifyRefusal::AmbiguousPick(circle)
    );
    // Clearly nearer one end still resolves.
    let clear = 1.1 * PI;
    assert!(trim(&b.s, arc, 5.0 * clear.cos(), 5.0 * clear.sin()).is_ok());
}
