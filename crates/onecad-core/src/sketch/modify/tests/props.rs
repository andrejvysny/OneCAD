//! Property tests over random scenes: every successful plan applies through the
//! real edit path with total accounting and no dangling references; every
//! plan is reproducible; every refusal is repeatable and draws no ids.

use std::f64::consts::TAU;

use proptest::prelude::*;

use super::*;
use crate::document::variables::Scalar;
use crate::sketch::modify::{
    plan_extend, plan_trim, ExtendRequest, ModifyRefusal, TrimRequest, MIN_PIECE_MM,
};

#[derive(Debug, Clone)]
enum Kind {
    Line,
    Circle,
    Arc(f64, f64),
}

#[derive(Debug, Clone)]
struct Scene {
    kind: Kind,
    a: (f64, f64),
    b: (f64, f64),
    segments: Vec<((f64, f64), (f64, f64))>,
    circles: Vec<((f64, f64), f64)>,
    flags: [bool; 6],
    pick: f64,
}

fn coord() -> impl Strategy<Value = (f64, f64)> {
    (-20.0f64..20.0, -20.0f64..20.0)
}

fn scene() -> impl Strategy<Value = Scene> {
    let kind = prop_oneof![
        Just(Kind::Line),
        Just(Kind::Circle),
        (0.0f64..TAU, 0.3f64..6.0).prop_map(|(s, sw)| Kind::Arc(s, s + sw)),
    ];
    (
        kind,
        coord(),
        coord(),
        prop::collection::vec((coord(), coord()), 1..8),
        prop::collection::vec((coord(), 1.0f64..12.0), 0..3),
        prop::array::uniform6(prop::bool::weighted(0.3)),
        0.0f64..1.0,
    )
        .prop_map(|(kind, a, b, segments, circles, flags, pick)| Scene {
            kind,
            a,
            b,
            segments,
            circles,
            flags,
            pick,
        })
}

/// Builds the scene; returns the sketch, the target, and a pick on it.
fn build(sc: &Scene) -> (Sketch, EntityId, Vec2) {
    let mut b = B::new();
    let mut lines = Vec::new();
    for (p, q) in &sc.segments {
        lines.push(b.line(*p, *q).0);
    }
    let circles: Vec<EntityId> = sc.circles.iter().map(|(c, r)| b.circle(*c, *r).0).collect();
    let (target, pick, ends) = match sc.kind {
        Kind::Line => {
            let (l, s, e) = b.line(sc.a, sc.b);
            let t = sc.pick;
            let at = v(
                sc.a.0 + (sc.b.0 - sc.a.0) * t,
                sc.a.1 + (sc.b.1 - sc.a.1) * t,
            );
            (l, at, Some((s, e)))
        }
        Kind::Circle => {
            let r = 2.0 + sc.b.0.abs() / 2.0;
            let (c, _) = b.circle(sc.a, r);
            let ang = sc.pick * TAU;
            (c, v(sc.a.0 + r * ang.cos(), sc.a.1 + r * ang.sin()), None)
        }
        Kind::Arc(s0, s1) => {
            let r = 2.0 + sc.b.0.abs() / 2.0;
            let (arc, _) = b.arc(sc.a, r, s0, s1);
            let ang = s0 + sc.pick * (s1 - s0);
            (arc, v(sc.a.0 + r * ang.cos(), sc.a.1 + r * ang.sin()), None)
        }
    };
    constrain(&mut b, sc, target, ends, &lines, &circles);
    (b.s, target, pick)
}

fn constrain(
    b: &mut B,
    sc: &Scene,
    target: EntityId,
    ends: Option<(EntityId, EntityId)>,
    lines: &[EntityId],
    circles: &[EntityId],
) {
    let f = sc.flags;
    if let (true, Some((s, e))) = (f[0], ends) {
        let id = b.cid();
        b.constrain(Constraint::Distance {
            id,
            entity1: s,
            entity1_position: CurvePosition::Arbitrary,
            entity2: e,
            entity2_position: CurvePosition::Arbitrary,
            value: Scalar::new(5.0),
        });
        b.horizontal(target);
    }
    if let (true, Some((_, e))) = (f[1], ends) {
        b.fixed(e, CurvePosition::Arbitrary);
    }
    if let (true, Some(&other)) = (f[2], lines.first()) {
        let id = b.cid();
        b.constrain(Constraint::Equal {
            id,
            entity1: target,
            entity2: other,
        });
    }
    if let (true, Some(&c)) = (f[3], circles.first()) {
        let id = b.cid();
        b.constrain(Constraint::Tangent {
            id,
            entity1: target,
            entity2: c,
        });
    }
    if f[4] && ends.is_none() {
        let id = b.cid();
        b.constrain(Constraint::Radius {
            id,
            entity: target,
            value: Scalar::new(3.0),
        });
    }
    if f[5] {
        let mid = b.pt((sc.a.0 + sc.b.0) / 2.0, (sc.a.1 + sc.b.1) / 2.0);
        b.on_curve(mid, target);
    }
}

fn no_dangling(after: &Sketch) {
    for c in after.constraints() {
        for e in c.entities() {
            assert!(after.contains_entity(e), "{c:?} dangles");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(384))]

    #[test]
    fn trim_plans_apply_account_and_reproduce(sc in scene()) {
        let (sketch, target, pick) = build(&sc);
        let request = TrimRequest { entity: target, pick };
        let mut ids = SeqIds::new();
        match plan_trim(&sketch, &request, &mut ids) {
            Ok(plan) => {
                let after = apply(&sketch, &plan);
                check_accounting(&sketch, &after, &plan, target, &[]);
                no_dangling(&after);
                for piece in &plan.pieces {
                    let circle = matches!(sc.kind, Kind::Circle);
                    prop_assert_eq!(piece.keeps_target_id, piece.owns_start && !circle);
                }
                if let Kind::Line = sc.kind {
                    for piece in &plan.pieces {
                        let (p, q) = ends(&after, piece.id);
                        prop_assert!(((p.x - q.x).hypot(p.y - q.y)) >= MIN_PIECE_MM * 0.999);
                    }
                }
                let again = plan_trim(&sketch, &request, &mut SeqIds::new()).unwrap();
                prop_assert_eq!(format!("{again:?}"), format!("{plan:?}"));
            }
            Err(refusal) => {
                prop_assert_eq!(ids.drawn, 0);
                let again = plan_trim(&sketch, &request, &mut SeqIds::new()).unwrap_err();
                prop_assert_eq!(again, refusal);
            }
        }
    }

    #[test]
    fn extend_plans_apply_account_and_reproduce(sc in scene()) {
        let (sketch, target, pick) = build(&sc);
        let request = ExtendRequest { entity: target, pick };
        let mut ids = SeqIds::new();
        match plan_extend(&sketch, &request, &mut ids) {
            Ok(grown) => {
                prop_assert!(!matches!(sc.kind, Kind::Circle));
                let moved: Vec<EntityId> = match sketch.get_entity(target) {
                    Some(SketchEntity::Line { start, end, .. }) => vec![*start, *end],
                    _ => Vec::new(),
                };
                let after = apply(&sketch, &grown.plan);
                check_accounting(&sketch, &after, &grown.plan, target, &moved);
                no_dangling(&after);
                prop_assert_eq!(ids.drawn, 1, "only the weld id is minted");
                let again = plan_extend(&sketch, &request, &mut SeqIds::new()).unwrap();
                prop_assert_eq!(format!("{again:?}"), format!("{grown:?}"));
            }
            Err(refusal) => {
                prop_assert_eq!(ids.drawn, 0);
                if matches!(sc.kind, Kind::Circle) {
                    prop_assert!(
                        matches!(refusal, ModifyRefusal::UnsupportedKind { .. }),
                        "{:?}",
                        refusal
                    );
                }
                let again = plan_extend(&sketch, &request, &mut SeqIds::new()).unwrap_err();
                prop_assert_eq!(again, refusal);
            }
        }
    }
}
