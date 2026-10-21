//! The constraint remap table, row by row.

use std::f64::consts::PI;

use super::*;
use crate::document::variables::Scalar;
use crate::sketch::modify::{plan_trim, AddReason, DropReason, TrimRequest};

fn trim(s: &Sketch, entity: EntityId, x: f64, y: f64) -> ModifyPlan {
    plan_trim(
        s,
        &TrimRequest {
            entity,
            pick: v(x, y),
        },
        &mut SeqIds::new(),
    )
    .unwrap()
}

/// Line (0,0)→(10,0) crossed at x=3 and x=7, plus a free reference line.
fn scene() -> (B, EntityId, EntityId, EntityId, EntityId) {
    let mut b = B::new();
    let (l, s, e) = b.line((0.0, 0.0), (10.0, 0.0));
    b.line((3.0, -5.0), (3.0, 5.0));
    b.line((7.0, -5.0), (7.0, 5.0));
    let (other, _, _) = b.line((0.0, 20.0), (5.0, 25.0));
    (b, l, s, e, other)
}

fn applied(b: &B, l: EntityId, plan: &ModifyPlan) -> Sketch {
    let after = apply(&b.s, plan);
    check_accounting(&b.s, &after, plan, l, &[]);
    after
}

#[test]
fn length_relations_of_a_trimmed_line_are_dropped() {
    let (mut b, l, s, e, other) = scene();
    let (equal, dist, mid) = (b.cid(), b.cid(), b.cid());
    b.constrain(Constraint::Equal {
        id: equal,
        entity1: l,
        entity2: other,
    });
    b.constrain(Constraint::Distance {
        id: dist,
        entity1: s,
        entity1_position: CurvePosition::Arbitrary,
        entity2: e,
        entity2_position: CurvePosition::Arbitrary,
        value: Scalar::new(10.0),
    });
    let free = b.pt(5.0, 5.0);
    b.constrain(Constraint::Midpoint {
        id: mid,
        point: free,
        point_position: CurvePosition::Arbitrary,
        line: l,
    });
    let plan = trim(&b.s, l, 5.0, 0.0);
    applied(&b, l, &plan);
    assert_eq!(
        dropped_reason(&plan, equal),
        Some(DropReason::LengthChanged)
    );
    assert_eq!(dropped_reason(&plan, dist), Some(DropReason::LengthChanged));
    assert_eq!(dropped_reason(&plan, mid), Some(DropReason::MidpointMoved));
}

#[test]
fn orientation_copies_but_single_target_dimensions_need_one_piece() {
    let (mut b, l, _, _, other) = scene();
    let (par, angle) = (b.cid(), b.cid());
    b.constrain(Constraint::Parallel {
        id: par,
        line1: other,
        line2: l,
    });
    b.constrain(Constraint::Angle {
        id: angle,
        line1: l,
        line2: other,
        value: Scalar::new(0.5),
    });
    let split = trim(&b.s, l, 5.0, 0.0);
    let after = applied(&b, l, &split);
    assert_eq!(split.kept_constraints, vec![par]);
    assert_eq!(
        dropped_reason(&split, angle),
        Some(DropReason::SplitAmbiguous)
    );
    let copy = split
        .added_constraints
        .iter()
        .find(|a| a.reason == AddReason::CopiedFrom { of: par })
        .unwrap();
    assert!(matches!(
        after.get_constraint(copy.id),
        Some(Constraint::Parallel { line1, line2, .. })
            if *line1 == other && *line2 == split.pieces[1].id
    ));
    // Trimming only the tail leaves one piece: the angle survives on it.
    let single = trim(&b.s, l, 9.0, 0.0);
    applied(&b, l, &single);
    assert!(single.kept_constraints.contains(&angle));
}

#[test]
fn on_curve_points_follow_their_piece_or_drop_with_the_span() {
    let (mut b, l, _, _, _) = scene();
    let (kept_pt, lost_pt) = (b.pt(8.5, 0.0), b.pt(5.0, 0.0));
    let kept = b.on_curve(kept_pt, l);
    let lost = b.on_curve(lost_pt, l);
    let plan = trim(&b.s, l, 4.0, 0.0);
    let after = applied(&b, l, &plan);
    assert_eq!(dropped_reason(&plan, lost), Some(DropReason::OnRemovedSpan));
    assert!(matches!(
        after.get_constraint(kept),
        Some(Constraint::OnCurve { curve, .. }) if *curve == plan.pieces[1].id
    ));
}

#[test]
fn tangent_survives_only_where_its_contact_survives() {
    let (mut b, l, _, _, _) = scene();
    // Off the line, so they bound nothing; contact = foot of the centre.
    let (near_c, _) = b.circle((8.5, 3.0), 1.0);
    let (far_c, _) = b.circle((5.0, -3.0), 1.0);
    let (t_kept, t_lost) = (b.cid(), b.cid());
    b.constrain(Constraint::Tangent {
        id: t_kept,
        entity1: l,
        entity2: near_c,
    });
    b.constrain(Constraint::Tangent {
        id: t_lost,
        entity1: far_c,
        entity2: l,
    });
    let plan = trim(&b.s, l, 4.0, 0.0);
    let after = applied(&b, l, &plan);
    assert_eq!(
        dropped_reason(&plan, t_lost),
        Some(DropReason::OnRemovedSpan)
    );
    assert!(matches!(
        after.get_constraint(t_kept),
        Some(Constraint::Tangent { entity1, .. }) if *entity1 == plan.pieces[1].id
    ));
}

#[test]
fn endpoint_tangency_follows_the_arc_piece_owning_that_end() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    b.line((0.0, -10.0), (0.0, 10.0));
    let start = b.pt(-5.0, 0.0);
    let tail_end = b.pt(-5.0, -4.0);
    let tail = b.seg(start, tail_end);
    let (weld, tan) = (b.cid(), b.cid());
    b.constrain(Constraint::Coincident {
        id: weld,
        point1: arc,
        point2: start,
        point1_position: CurvePosition::End,
        point2_position: CurvePosition::Arbitrary,
    });
    b.constrain(Constraint::TangentAtPoint {
        id: tan,
        entity1: arc,
        entity2: tail,
        entity1_position: CurvePosition::End,
        entity2_position: CurvePosition::Start,
    });
    let center_fix = b.fixed(arc, CurvePosition::Center);
    let plan = trim(&b.s, arc, 5.0, 0.5);
    let after = applied(&b, arc, &plan);
    let piece = plan.pieces[0].id;
    for id in [weld, tan, center_fix] {
        assert!(
            after
                .get_constraint(id)
                .unwrap()
                .entities()
                .contains(&piece),
            "{id} moved to the surviving piece"
        );
    }
}

#[test]
fn removing_the_owning_end_drops_role_constraints() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    b.line((0.0, -10.0), (0.0, 10.0));
    let fix_start = b.fixed(arc, CurvePosition::Start);
    let fix_end = b.fixed(arc, CurvePosition::End);
    let plan = trim(&b.s, arc, 5.0, 0.5);
    applied(&b, arc, &plan);
    assert_eq!(
        dropped_reason(&plan, fix_start),
        Some(DropReason::PointRemoved)
    );
    assert!(plan.kept_constraints.contains(&fix_end));
}

#[test]
fn equal_on_an_arc_is_kept_unsplit_and_dropped_split() {
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    b.line((-2.0, -10.0), (-2.0, 10.0));
    b.line((2.0, -10.0), (2.0, 10.0));
    let (other, _) = b.circle((20.0, 0.0), 5.0);
    let eq = b.cid();
    b.constrain(Constraint::Equal {
        id: eq,
        entity1: arc,
        entity2: other,
    });
    let split = trim(&b.s, arc, 0.0, 5.0);
    applied(&b, arc, &split);
    assert_eq!(dropped_reason(&split, eq), Some(DropReason::SplitAmbiguous));
    let single = trim(&b.s, arc, -4.9, 0.5);
    applied(&b, arc, &single);
    assert!(single.kept_constraints.contains(&eq));
}
