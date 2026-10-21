use onecad_core::ids::{BodyId, DatumPlaneId, ElementId, RecordId, RegionId, SketchId, TopoKey};
use onecad_interaction::selection_set::*;
use onecad_scene::SceneIdentity;
use uuid::Uuid;

#[allow(dead_code)]
mod support;

const SCENE: SceneIdentity = SceneIdentity {
    session: 1,
    scene: 2,
};
const NEXT_SCENE: SceneIdentity = SceneIdentity {
    session: 1,
    scene: 3,
};

fn body(n: u128) -> BodyId {
    BodyId(Uuid::from_u128(n))
}

fn topo(scene: SceneIdentity, b: u128, key: &str) -> TopologyRef {
    TopologyRef {
        scene,
        body: body(b),
        id: TopologyId::Snapshot(TopoKey::new(key)),
    }
}

fn face(b: u128, key: &str) -> SelectionItem {
    SelectionItem::Face(topo(SCENE, b, key))
}

fn edge(b: u128, key: &str) -> SelectionItem {
    SelectionItem::Edge(topo(SCENE, b, key))
}

fn profile(scene: SceneIdentity) -> SelectionItem {
    let mut context = support::context();
    context.scene = scene;
    SelectionItem::Profile(Box::new(support::selection(context)))
}

fn set_of(items: Vec<SelectionItem>) -> SelectionSet {
    let mut set = SelectionSet::default();
    set.apply(SelectionMode::Replace, items).unwrap();
    set
}

#[test]
fn every_item_reports_its_kind() {
    let cases = [
        (SelectionItem::Body(body(1)), SelectionKind::Body),
        (face(1, "f:1"), SelectionKind::Face),
        (edge(1, "e:1"), SelectionKind::Edge),
        (
            SelectionItem::Vertex(topo(SCENE, 1, "v:1")),
            SelectionKind::Vertex,
        ),
        (
            SelectionItem::Feature(RecordId(Uuid::from_u128(5))),
            SelectionKind::Feature,
        ),
        (
            SelectionItem::Sketch(SketchId(Uuid::from_u128(6))),
            SelectionKind::Sketch,
        ),
        (profile(SCENE), SelectionKind::Profile),
        (
            SelectionItem::Datum(DatumPlaneId(Uuid::from_u128(7))),
            SelectionKind::Datum,
        ),
    ];
    for (item, kind) in cases {
        assert_eq!(item.kind(), kind);
    }
}

#[test]
fn replace_dedups_and_keeps_first_occurrence_order() {
    let mut set = SelectionSet::default();
    let changed = set
        .apply(
            SelectionMode::Replace,
            vec![face(1, "f:2"), face(1, "f:1"), face(1, "f:2")],
        )
        .unwrap();
    assert!(changed);
    assert_eq!(set.items(), &[face(1, "f:2"), face(1, "f:1")]);
    assert_eq!(set.revision(), 1);
}

#[test]
fn add_appends_new_items_and_keeps_existing_positions() {
    let mut set = set_of(vec![face(1, "f:1"), face(1, "f:2")]);
    assert!(set
        .apply(SelectionMode::Add, vec![face(1, "f:2"), face(1, "f:3")])
        .unwrap());
    assert_eq!(
        set.items(),
        &[face(1, "f:1"), face(1, "f:2"), face(1, "f:3")]
    );
}

#[test]
fn toggle_removes_present_and_appends_absent() {
    let mut set = set_of(vec![face(1, "f:1"), face(1, "f:2")]);
    assert!(set
        .apply(SelectionMode::Toggle, vec![face(1, "f:1"), face(1, "f:3")])
        .unwrap());
    assert_eq!(set.items(), &[face(1, "f:2"), face(1, "f:3")]);
}

#[test]
fn toggle_with_repeated_input_applies_once() {
    let mut set = set_of(vec![face(1, "f:1")]);
    assert!(set
        .apply(SelectionMode::Toggle, vec![face(1, "f:1"), face(1, "f:1")])
        .unwrap());
    assert!(set.is_empty());
}

#[test]
fn subtract_removes_only_present_items() {
    let mut set = set_of(vec![face(1, "f:1"), face(1, "f:2")]);
    assert!(set
        .apply(
            SelectionMode::Subtract,
            vec![face(1, "f:2"), face(1, "f:9")]
        )
        .unwrap());
    assert_eq!(set.items(), &[face(1, "f:1")]);
    let revision = set.revision();
    assert!(!set
        .apply(SelectionMode::Subtract, vec![face(1, "f:9")])
        .unwrap());
    assert_eq!(set.revision(), revision);
}

#[test]
fn revision_moves_only_on_actual_change() {
    let mut set = set_of(vec![face(1, "f:1")]);
    assert_eq!(set.revision(), 1);
    assert!(!set
        .apply(SelectionMode::Replace, vec![face(1, "f:1")])
        .unwrap());
    assert!(!set.apply(SelectionMode::Add, vec![face(1, "f:1")]).unwrap());
    assert!(!set.apply(SelectionMode::Add, Vec::new()).unwrap());
    assert!(set.invalidate(|_| false).is_empty());
    assert_eq!(set.revision(), 1);
    assert!(set.clear());
    assert_eq!(set.revision(), 2);
    assert!(!set.clear());
    assert_eq!(set.revision(), 2);
}

#[test]
fn identity_distinguishes_kind_body_scene_and_id_form() {
    let element = SelectionItem::Face(TopologyRef {
        scene: SCENE,
        body: body(1),
        id: TopologyId::Element(ElementId::new("f:1")),
    });
    let set = set_of(vec![
        face(1, "f:1"),
        edge(1, "f:1"),
        face(2, "f:1"),
        SelectionItem::Face(topo(NEXT_SCENE, 1, "f:1")),
        element,
    ]);
    assert_eq!(set.len(), 5);
}

#[test]
fn profile_identity_ignores_capture_and_anchor() {
    let first = profile(SCENE);
    let mut repicked = first.clone();
    if let SelectionItem::Profile(p) = &mut repicked {
        p.profile.region_anchor = Some([9.0, 9.0]);
        p.context.revision += 1;
    }
    let mut set = set_of(vec![first.clone()]);
    assert!(!set
        .apply(SelectionMode::Add, vec![repicked.clone()])
        .unwrap());
    assert_eq!(set.items(), &[first]);
    assert!(set.apply(SelectionMode::Toggle, vec![repicked]).unwrap());
    assert!(set.is_empty());

    let mut other = profile(SCENE);
    if let SelectionItem::Profile(p) = &mut other {
        p.profile.region = RegionId("region-2".into());
    }
    assert_eq!(set_of(vec![profile(SCENE), other]).len(), 2);
}

#[test]
fn over_limit_is_refused_atomically() {
    let mut set = set_of(vec![face(1, "f:0")]);
    let many: Vec<_> = (1..=MAX_SELECTION_TARGETS)
        .map(|n| face(1, &format!("f:{n}")))
        .collect();
    assert_eq!(
        set.apply(SelectionMode::Add, many.clone()),
        Err(SelectionRefusal::TooMany {
            limit: MAX_SELECTION_TARGETS
        })
    );
    assert_eq!(set.items(), &[face(1, "f:0")]);
    assert_eq!(set.revision(), 1);
    assert!(set.apply(SelectionMode::Replace, many).unwrap());
    assert_eq!(set.len(), MAX_SELECTION_TARGETS);
}

#[test]
fn invalidate_returns_removed_items_in_order() {
    let mut set = set_of(vec![
        face(1, "f:1"),
        SelectionItem::Face(topo(NEXT_SCENE, 1, "f:2")),
        SelectionItem::Body(body(1)),
        profile(NEXT_SCENE),
    ]);
    let removed = set.invalidate(|item| item.is_stale_for(SCENE));
    assert_eq!(
        removed,
        vec![
            SelectionItem::Face(topo(NEXT_SCENE, 1, "f:2")),
            profile(NEXT_SCENE)
        ]
    );
    assert_eq!(set.items(), &[face(1, "f:1"), SelectionItem::Body(body(1))]);
    assert_eq!(set.revision(), 2);
    let kept_bodies = set.retain(|item| item.kind() == SelectionKind::Body);
    assert_eq!(kept_bodies, vec![face(1, "f:1")]);
    assert_eq!(set.revision(), 3);
}

#[test]
fn body_tint_yields_only_to_own_face_or_edge() {
    let a = SelectionItem::Body(body(1));
    let vertex = SelectionItem::Vertex(topo(SCENE, 1, "v:1"));
    assert!(set_of(vec![a.clone()]).body_tinted(body(1)));
    assert!(set_of(vec![a.clone(), face(2, "f:1")]).body_tinted(body(1)));
    assert!(set_of(vec![a.clone(), edge(2, "e:1")]).body_tinted(body(1)));
    assert!(!set_of(vec![a.clone(), face(1, "f:1")]).body_tinted(body(1)));
    assert!(!set_of(vec![a.clone(), edge(1, "e:1")]).body_tinted(body(1)));
    assert!(set_of(vec![a, vertex]).body_tinted(body(1)));
    assert!(!set_of(vec![face(1, "f:1")]).body_tinted(body(1)));
}

#[test]
fn kind_summary_and_typed_accessors() {
    let set = set_of(vec![
        edge(1, "e:1"),
        face(1, "f:1"),
        edge(1, "e:2"),
        SelectionItem::Body(body(1)),
    ]);
    assert_eq!(
        set.kinds().into_iter().collect::<Vec<_>>(),
        vec![
            SelectionKind::Body,
            SelectionKind::Face,
            SelectionKind::Edge
        ]
    );
    assert_eq!(set.single_kind(), None);
    assert_eq!(set.single_body(), Some(body(1)));
    assert_eq!(set.edges().count(), 2);
    assert_eq!(set.faces().count(), 1);
    assert_eq!(set.vertices().count(), 0);
    assert_eq!(set.bodies().collect::<Vec<_>>(), vec![body(1)]);
    assert_eq!(
        set.of_kind(SelectionKind::Edge)
            .cloned()
            .collect::<Vec<_>>(),
        vec![edge(1, "e:1"), edge(1, "e:2")]
    );

    let edges = set_of(vec![edge(1, "e:1"), edge(2, "e:1")]);
    assert_eq!(edges.single_kind(), Some(SelectionKind::Edge));
    assert_eq!(edges.single_body(), None);
    assert_eq!(SelectionSet::default().single_kind(), None);
    assert_eq!(SelectionSet::default().single_body(), None);
    let mixed = set_of(vec![face(1, "f:1"), profile(SCENE)]);
    assert_eq!(mixed.single_body(), None);
    assert_eq!(mixed.profiles().count(), 1);
}

fn fillet_plan() -> StagedOperands {
    StagedOperands::new(
        SCENE,
        vec![OperandStage::new(
            OperandRole::Edges,
            [SelectionKind::Edge],
            1,
            None,
        )],
    )
    .unwrap()
}

fn boolean_plan() -> StagedOperands {
    StagedOperands::new(
        SCENE,
        vec![
            OperandStage::new(OperandRole::Target, [SelectionKind::Body], 1, Some(1)),
            OperandStage::new(OperandRole::Tool, [SelectionKind::Body], 1, None),
        ],
    )
    .unwrap()
}

#[test]
fn plan_definition_errors_are_typed() {
    let stage = |min, max| OperandStage::new(OperandRole::Faces, [SelectionKind::Face], min, max);
    assert_eq!(
        StagedOperands::new(SCENE, Vec::new()),
        Err(OperandPlanError::Empty)
    );
    assert_eq!(
        StagedOperands::new(
            SCENE,
            vec![OperandStage::new(OperandRole::Axis, [], 1, None)]
        ),
        Err(OperandPlanError::NoAcceptedKinds(OperandRole::Axis))
    );
    assert_eq!(
        StagedOperands::new(SCENE, vec![stage(0, Some(0))]),
        Err(OperandPlanError::ZeroMax(OperandRole::Faces))
    );
    assert_eq!(
        StagedOperands::new(SCENE, vec![stage(2, Some(1))]),
        Err(OperandPlanError::MinAboveMax(OperandRole::Faces))
    );
    let mut seeded = stage(1, None);
    seeded.items.push(face(1, "f:1"));
    assert_eq!(
        StagedOperands::new(SCENE, vec![seeded]),
        Err(OperandPlanError::NotEmpty(OperandRole::Faces))
    );
}

#[test]
fn offer_refusal_matrix() {
    let mut plan = fillet_plan();
    assert_eq!(
        plan.offer(face(1, "f:1")),
        Err(OperandRefusal::WrongKind {
            role: OperandRole::Edges,
            kind: SelectionKind::Face
        })
    );
    assert_eq!(
        plan.offer(SelectionItem::Edge(topo(NEXT_SCENE, 1, "e:1"))),
        Err(OperandRefusal::Stale { scene: SCENE })
    );
    assert!(plan.offer(edge(1, "e:1")).is_ok());
    assert_eq!(
        plan.offer(edge(1, "e:1")),
        Err(OperandRefusal::Duplicate {
            role: OperandRole::Edges
        })
    );

    let mut boolean = boolean_plan();
    boolean.offer(SelectionItem::Body(body(1))).unwrap();
    assert_eq!(
        boolean.offer(SelectionItem::Body(body(1))),
        Err(OperandRefusal::Duplicate {
            role: OperandRole::Target
        })
    );
    boolean.activate(0).unwrap();
    assert_eq!(
        boolean.offer(SelectionItem::Body(body(2))),
        Err(OperandRefusal::TooMany {
            role: OperandRole::Target,
            max: 1
        })
    );
    assert_eq!(
        boolean.stages()[0].items,
        vec![SelectionItem::Body(body(1))]
    );
    assert!(boolean.stages()[1].items.is_empty());
}

#[test]
fn refused_offer_leaves_state_unchanged() {
    let mut plan = fillet_plan();
    plan.offer(edge(1, "e:1")).unwrap();
    let before = plan.clone();
    for item in [
        face(1, "f:1"),
        edge(1, "e:1"),
        SelectionItem::Edge(topo(NEXT_SCENE, 1, "e:2")),
    ] {
        assert!(plan.offer(item).is_err());
        assert_eq!(plan, before);
    }
}

#[test]
fn reference_roles_may_reuse_geometry_but_exclusive_roles_may_not() {
    let mut revolve = StagedOperands::new(
        SCENE,
        vec![
            OperandStage::new(OperandRole::Edges, [SelectionKind::Edge], 1, Some(1)),
            OperandStage::new(OperandRole::Axis, [SelectionKind::Edge], 1, Some(1)),
        ],
    )
    .unwrap();
    revolve.offer(edge(1, "e:1")).unwrap();
    assert_eq!(
        revolve.offer(edge(1, "e:1")),
        Err(OperandRefusal::Duplicate {
            role: OperandRole::Edges
        })
    );

    let mut hole = StagedOperands::new(
        SCENE,
        vec![
            OperandStage::new(OperandRole::Placement, [SelectionKind::Face], 1, Some(1)),
            OperandStage::new(OperandRole::Plane, [SelectionKind::Face], 1, Some(1)),
        ],
    )
    .unwrap();
    hole.offer(face(1, "f:1")).unwrap();
    assert!(hole.offer(face(1, "f:1")).is_ok());
    assert!(hole.is_complete());
}

#[test]
fn filling_a_bounded_stage_advances_and_completes() {
    let mut plan = boolean_plan();
    assert!(!plan.is_complete());
    assert_eq!(
        plan.offer(SelectionItem::Body(body(1))),
        Ok(Accepted {
            stage: 0,
            role: OperandRole::Target,
            advanced: true
        })
    );
    assert_eq!(plan.active_index(), 1);
    assert!(!plan.is_complete());
    for b in [2, 3] {
        assert_eq!(
            plan.offer(SelectionItem::Body(body(b))),
            Ok(Accepted {
                stage: 1,
                role: OperandRole::Tool,
                advanced: false
            })
        );
    }
    assert!(plan.is_complete());
    assert_eq!(plan.advance(), Err(OperandRefusal::NoNextStage));
}

#[test]
fn last_bounded_stage_does_not_advance_past_end() {
    let mut plan = StagedOperands::new(
        SCENE,
        vec![OperandStage::new(
            OperandRole::Profile,
            [SelectionKind::Profile],
            1,
            Some(1),
        )],
    )
    .unwrap();
    assert_eq!(plan.offer(profile(SCENE)).map(|a| a.advanced), Ok(false));
    assert_eq!(plan.active_index(), 0);
    assert!(plan.is_complete());
}

#[test]
fn manual_advance_and_activation_require_satisfied_predecessors() {
    let mut plan = StagedOperands::new(
        SCENE,
        vec![
            OperandStage::new(OperandRole::Target, [SelectionKind::Body], 1, Some(1)),
            OperandStage::new(OperandRole::Faces, [SelectionKind::Face], 1, None),
            OperandStage::new(OperandRole::Placement, [SelectionKind::Face], 0, None),
        ],
    )
    .unwrap();
    assert_eq!(
        plan.advance(),
        Err(OperandRefusal::Unsatisfied {
            role: OperandRole::Target,
            have: 0,
            min: 1
        })
    );
    assert_eq!(
        plan.activate(2),
        Err(OperandRefusal::Unsatisfied {
            role: OperandRole::Target,
            have: 0,
            min: 1
        })
    );
    assert_eq!(plan.activate(3), Err(OperandRefusal::NoSuchStage(3)));
    plan.offer(SelectionItem::Body(body(1))).unwrap();
    plan.offer(face(1, "f:1")).unwrap();
    plan.offer(face(1, "f:2")).unwrap();
    assert_eq!(plan.active().role, OperandRole::Faces);
    assert_eq!(plan.advance(), Ok(2));
    assert!(plan.is_complete());
    plan.activate(0).unwrap();
    assert_eq!(plan.active().role, OperandRole::Target);
}

#[test]
fn remove_and_toggle_act_on_active_stage_only() {
    let mut plan = boolean_plan();
    plan.offer(SelectionItem::Body(body(1))).unwrap();
    assert_eq!(plan.remove(&SelectionItem::Body(body(1))), None);
    assert_eq!(
        plan.toggle(SelectionItem::Body(body(1))),
        Err(OperandRefusal::Duplicate {
            role: OperandRole::Target
        })
    );
    assert!(matches!(
        plan.toggle(SelectionItem::Body(body(2))),
        Ok(Toggled::Added(Accepted {
            stage: 1,
            role: OperandRole::Tool,
            advanced: false
        }))
    ));
    assert_eq!(
        plan.toggle(SelectionItem::Body(body(2))),
        Ok(Toggled::Removed(SelectionItem::Body(body(2))))
    );
    assert!(plan.stages()[1].items.is_empty());
    plan.activate(0).unwrap();
    assert_eq!(
        plan.remove(&SelectionItem::Body(body(1))),
        Some(SelectionItem::Body(body(1)))
    );
    assert!(!plan.is_complete());
}

#[test]
fn scene_change_drops_stale_operands_with_typed_records_and_rewinds() {
    let mut plan = StagedOperands::new(
        SCENE,
        vec![
            OperandStage::new(OperandRole::Profile, [SelectionKind::Profile], 1, Some(1)),
            OperandStage::new(OperandRole::Target, [SelectionKind::Body], 0, Some(1)),
            OperandStage::new(OperandRole::Faces, [SelectionKind::Face], 1, None),
        ],
    )
    .unwrap();
    plan.offer(profile(SCENE)).unwrap();
    plan.offer(SelectionItem::Body(body(1))).unwrap();
    plan.offer(face(1, "f:1")).unwrap();
    assert_eq!(plan.active_index(), 2);
    assert!(plan.is_complete());

    let removed = plan.set_scene(NEXT_SCENE);
    assert_eq!(
        removed,
        vec![
            InvalidatedOperand {
                stage: 0,
                role: OperandRole::Profile,
                item: profile(SCENE)
            },
            InvalidatedOperand {
                stage: 2,
                role: OperandRole::Faces,
                item: face(1, "f:1")
            },
        ]
    );
    assert_eq!(plan.scene(), NEXT_SCENE);
    assert_eq!(plan.active_index(), 0);
    assert_eq!(plan.stages()[1].items, vec![SelectionItem::Body(body(1))]);
    assert!(!plan.is_complete());
    assert_eq!(
        plan.offer(face(1, "f:9")),
        Err(OperandRefusal::Stale { scene: NEXT_SCENE })
    );
    assert!(plan.offer(profile(NEXT_SCENE)).unwrap().advanced);
}

#[test]
fn predicate_invalidation_keeps_active_stage_when_predecessors_hold() {
    let mut plan = boolean_plan();
    plan.offer(SelectionItem::Body(body(1))).unwrap();
    plan.offer(SelectionItem::Body(body(2))).unwrap();
    plan.offer(SelectionItem::Body(body(3))).unwrap();
    let removed = plan.invalidate(|item| item.body() == Some(body(2)));
    assert_eq!(
        removed,
        vec![InvalidatedOperand {
            stage: 1,
            role: OperandRole::Tool,
            item: SelectionItem::Body(body(2))
        }]
    );
    assert_eq!(plan.active_index(), 1);
    assert!(plan.is_complete());
    assert!(plan.invalidate(|_| false).is_empty());
}

#[test]
fn persistent_items_are_never_scene_stale() {
    for item in [
        SelectionItem::Body(body(1)),
        SelectionItem::Feature(RecordId(Uuid::from_u128(5))),
        SelectionItem::Sketch(SketchId(Uuid::from_u128(6))),
        SelectionItem::Datum(DatumPlaneId(Uuid::from_u128(7))),
    ] {
        assert_eq!(item.scene(), None);
        assert!(!item.is_stale_for(NEXT_SCENE));
    }
    assert!(face(1, "f:1").is_stale_for(NEXT_SCENE));
    assert!(profile(SCENE).is_stale_for(NEXT_SCENE));
    assert!(!profile(SCENE).is_stale_for(SCENE));
}
