//! Trim against same-carrier boundaries: collinear lines cut, arcs of the
//! target's own circle refuse, and which crossing a cluster welds to.

use std::f64::consts::PI;

use super::trim::trim;
use super::*;
use crate::sketch::modify::{AddReason, ModifyRefusal};

#[test]
fn a_same_circle_arc_end_wins_its_cluster_and_refuses() {
    // A line created first ends where the same-circle arc starts; the overlap
    // must still be the representative, so the cut is refused.
    let mut b = B::new();
    let at = |a: f64, r: f64| (r * a.cos(), r * a.sin());
    b.line(at(PI / 3.0, 9.0), at(PI / 3.0, 5.0));
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    let (boundary, _) = b.arc((0.0, 0.0), 5.0, PI / 3.0, 2.0 * PI / 3.0);
    let (x, y) = at(0.15, 5.0);
    assert_eq!(
        trim(&b.s, arc, x, y).unwrap_err(),
        ModifyRefusal::OverlapsBoundary {
            entity: arc,
            boundary
        }
    );
}

#[test]
fn a_collinear_end_wins_its_cluster_as_the_weld() {
    // A stem created first ends where a collinear segment starts: the weld
    // goes to the collinear segment's end point.
    let mut b = B::new();
    let (l, _, _) = b.line((0.0, 0.0), (20.0, 0.0));
    b.line((12.0, 5.0), (12.0, 0.0));
    let (seg, p, _) = b.line((12.0, 0.0), (15.0, 0.0));
    let plan = trim(&b.s, l, 6.0, 0.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);
    let weld = plan.added_constraints[0];
    assert_eq!(weld.reason, AddReason::CutWeld { boundary: seg });
    assert!(matches!(
        after.get_constraint(weld.id),
        Some(Constraint::Coincident { point2, .. }) if *point2 == p
    ));
}

#[test]
fn a_same_circle_arc_overlap_refuses_the_trim() {
    // C7: the pick lies on the overlap with an arc of the same circle; a
    // full circle there is refused alike.
    let mut b = B::new();
    let (arc, _) = b.arc((0.0, 0.0), 5.0, 0.0, PI);
    let id = b.cid();
    b.constrain(Constraint::Radius {
        id,
        entity: arc,
        value: crate::document::variables::Scalar::new(5.0),
    });
    let (boundary, _) = b.arc((0.0, 0.0), 5.0, PI / 3.0, 2.0 * PI / 3.0);
    let overlaps = ModifyRefusal::OverlapsBoundary {
        entity: arc,
        boundary,
    };
    assert_eq!(trim(&b.s, arc, 0.0, 5.0).unwrap_err(), overlaps);
    // Cut at an overlap end from outside it, the piece would run along it.
    assert_eq!(trim(&b.s, arc, 5.0, 0.5).unwrap_err(), overlaps);
    let (circle, _) = b.circle((0.0, 0.0), 5.0);
    assert_eq!(
        trim(&b.s, circle, 0.0, -5.0).unwrap_err(),
        ModifyRefusal::OverlapsBoundary {
            entity: circle,
            boundary: arc
        }
    );
}

#[test]
fn a_collinear_boundary_cuts_at_its_end_points() {
    let mut b = B::new();
    let (l, _, _) = b.line((0.0, 0.0), (20.0, 0.0));
    let (seg, p, q) = b.line((8.0, 0.0), (12.0, 0.0));
    let plan = trim(&b.s, l, 10.0, 0.0).unwrap();
    let after = apply(&b.s, &plan);
    check_accounting(&b.s, &after, &plan, l, &[]);
    assert_eq!(plan.pieces.len(), 2);
    let (_, cut) = ends(&after, plan.pieces[0].id);
    let (cut2, _) = ends(&after, plan.pieces[1].id);
    assert!(near(cut, 8.0, 0.0) && near(cut2, 12.0, 0.0));
    let welded: Vec<EntityId> = plan
        .added_constraints
        .iter()
        .filter(|a| a.reason == AddReason::CutWeld { boundary: seg })
        .filter_map(|a| match after.get_constraint(a.id) {
            Some(Constraint::Coincident { point2, .. }) => Some(*point2),
            _ => None,
        })
        .collect();
    assert_eq!(welded, vec![p, q]);
    // A pick outside the overlap trims only up to it.
    let head = trim(&b.s, l, 4.0, 0.0).unwrap();
    assert_eq!(head.pieces.len(), 1);
    let after = apply(&b.s, &head);
    let (start, _) = ends(&after, head.pieces[0].id);
    assert!(near(start, 8.0, 0.0));
}
