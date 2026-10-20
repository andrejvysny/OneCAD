//! Fully constrained controls retain exact analytic solid expectations.
use super::*;
use onecad_core::{
    document::variables::{Unit, Variable},
    ids::VariableId,
    sketch::CurvePosition,
};

#[test]
fn numeric_slot_expression_dimensioned_solid_and_fresh_reopen() {
    isolated(
        "dimensioned::numeric_slot_expression_dimensioned_solid_and_fresh_reopen",
        "slot-dimensioned",
        |root, phase| {
            Box::pin(async move {
                if phase.ends_with("-reopen") {
                    reopen(&root, "slot-dimensioned", 2, 6.0, assert_slot).await;
                } else {
                    slot(&root).await;
                }
            })
        },
    );
}

#[test]
fn dimensioned_tangent_profile_solid_and_fresh_reopen() {
    isolated(
        "dimensioned::dimensioned_tangent_profile_solid_and_fresh_reopen",
        "chain-dimensioned",
        |root, phase| {
            Box::pin(async move {
                if phase.ends_with("-reopen") {
                    reopen(&root, "chain-dimensioned", 2, 0.0, |s, _| assert_chain(s)).await;
                } else {
                    chain(&root).await;
                }
            })
        },
    );
}

fn fixed(seed: u128, index: usize, point: EntityId, at: Vec2) -> Constraint {
    Constraint::Fixed {
        id: ids(seed).constraints[index],
        point,
        point_position: CurvePosition::Arbitrary,
        at,
    }
}

async fn slot(root: &Path) {
    let harness = new(root, "host").await;
    slot_variable(&harness).await;
    let mut state = begin(&harness, true).await;
    state.dispatch(SketchAction::SetTool(SketchTool::Slot));
    assert!(click_at(&mut state, xy(0.0, 0.0), 500).is_empty());
    assert!(click_at(&mut state, xy(30.0, 0.0), 500).is_empty());
    numeric_slot(&harness, &mut state).await;
    let p = ids(1000);
    add(
        &harness,
        &mut state,
        vec![
            fixed(5000, 0, p.entities[4], xy(0.0, 0.0)),
            fixed(5000, 1, p.entities[5], xy(30.0, 0.0)),
        ],
        0,
    )
    .await;
    let sketch = sketch(&harness).await;
    assert_slot(&sketch, 6.0);
    assert_eq!(
        sketch.get_constraint(p.constraints[9]).unwrap().value(),
        Some(&Scalar::with_expr(12.0, "slot_width"))
    );
    finish_extrude_save(&harness, &mut state, root, "slot-dimensioned").await;
    harness.close().await;
}

async fn chain(root: &Path) {
    let harness = new(root, "host").await;
    let mut state = begin(&harness, true).await;
    state.dispatch(SketchAction::SetTool(SketchTool::Line));
    assert!(click_at(&mut state, xy(0.0, 0.0), 500).is_empty());
    for (seed, point) in [
        (1000, xy(20.0, 0.0)),
        (2000, xy(20.0, 20.0)),
        (3000, xy(0.0, 20.0)),
        (4000, xy(0.0, 0.0)),
    ] {
        if seed == 2000 {
            let key = state.session.as_ref().unwrap().key.clone();
            state.dispatch(SketchAction::SetTangentArc { key, enabled: true });
        }
        let effects = click_at(&mut state, point, seed);
        once(&harness, &mut state, effects).await;
    }
    add(&harness, &mut state, chain_dimensions(), 0).await;
    assert_chain(&sketch(&harness).await);
    finish_extrude_save(&harness, &mut state, root, "chain-dimensioned").await;
    harness.close().await;
}

async fn slot_variable(harness: &Harness) {
    api::apply_edit_command(
        &harness.state,
        harness.host.clone(),
        EditCommand::AddVariable {
            variable: Variable {
                id: VariableId(Uuid::from_u128(0x2a002)),
                name: "slot_width".into(),
                value: Scalar::new(12.0),
                unit: Unit::Mm,
            },
        },
    )
    .await
    .unwrap();
}

async fn numeric_slot(harness: &Harness, state: &mut SketchInteraction) {
    let value = api::evaluate_expression(&harness.state, "slot_width".into(), "length".into())
        .await
        .unwrap();
    assert!(value.error.is_none(), "{value:?}");
    assert_eq!(value.value, 12.0);
    // Bare variables are scalar expressions, admitted as millimetres at length sites.
    assert_eq!(value.dimension, "scalar");
    let session = state.session.as_ref().unwrap();
    let effects = state.dispatch(SketchAction::CreateNumeric {
        key: session.key.clone(),
        capture: session.accepted_capture.unwrap(),
        ids: ids(1000),
        dimensions: Box::new(CreationDimensions::Slot {
            width: Scalar::with_expr(value.value, "slot_width"),
        }),
    });
    let solve = once(harness, state, effects).await;
    assert_eq!(solve.dof, 4);
}

fn chain_dimensions() -> Vec<Constraint> {
    vec![
        fixed(5000, 0, ids(1000).entities[0], xy(0.0, 0.0)),
        fixed(5000, 1, ids(1000).entities[1], xy(20.0, 0.0)),
        fixed(5000, 2, ids(3000).entities[1], xy(0.0, 20.0)),
        Constraint::Radius {
            id: ids(5000).constraints[3],
            entity: ids(2000).entities[1],
            value: Scalar::new(10.0),
        },
        Constraint::VerticalPoints {
            id: ids(5000).constraints[4],
            point1: ids(3000).entities[0],
            point1_position: CurvePosition::Arbitrary,
            point2: ids(1000).entities[1],
            point2_position: CurvePosition::Arbitrary,
        },
    ]
}
