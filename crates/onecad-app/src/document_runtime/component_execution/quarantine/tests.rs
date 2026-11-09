use super::*;
use onecad_core::document::repair::{LadderLevel, RepairItem, RepairReason};
use onecad_core::document::{
    body::BodyLifecycleEvent,
    component::ComponentDef,
    modules::{ModuleId, ModuleState},
};

const ID: &str = "el_00000000000000000000000000000001";
const DEFINITION: ComponentDefId = ComponentDefId(Uuid::from_u128(10));

fn fixture() -> (DocumentRuntime, QuarantineInput, Vec<RefResolution>) {
    let body = BodyId(Uuid::from_u128(2));
    let entry = ElementEntry::new(body, ElementKind::Face)
        .with_evidence(None, Some(serde_json::json!({"frozen": [1, 2, 3]})));
    let mut definition = ComponentDef::new(DEFINITION, "part");
    definition.elements.insert(ElementId::new(ID), entry);
    let mut document = Document::new(DocumentId(Uuid::from_u128(1)));
    document.definitions.insert(DEFINITION, definition.clone());
    let backend = Arc::new(crate::worker::PendingBackend);
    let mut runtime = DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend);
    runtime.session = onecad_core::edit::DocumentSession::new(document.clone());
    runtime.regen.elements = definition.elements;
    runtime.regen.bodies.fold(
        0,
        RecordId(Uuid::from_u128(3)),
        BodyLifecycleEvent::Created { body },
    );
    let proof = QuarantineInput {
        document: Box::new(document),
        generation: runtime.persisted_generation(),
        retention: Arc::new(()),
    };
    (runtime, proof, vec![resolution(body)])
}

fn resolution(body: BodyId) -> RefResolution {
    RefResolution {
        ref_id: ID.into(),
        snapshot_id: SnapshotId(1),
        revision: 0,
        body_id: Some(body),
        outcome: ResolveOutcome::NeedsRepair(RepairItem {
            step_index: 0,
            ref_id: ID.into(),
            element_id: Some(ElementId::new(ID)),
            ladder_failed: LadderLevel::Descriptor,
            reason: RepairReason::NoCandidates,
            candidates: vec![],
            scoring_version: None,
            anchor: None,
            ui_label: "missing face".into(),
            seeded: false,
            ordinal_anchor: None,
            seed_edge_id: None,
            resolved_axis: None,
            frozen_axis: None,
            resolved_sidedness: None,
        }),
    }
}

#[test]
fn genuine_orphan_answer_retains_identity_and_evidence() {
    let (mut runtime, proof, rows) = fixture();
    let old = runtime
        .regen
        .elements
        .get(&ElementId::new(ID))
        .unwrap()
        .clone();
    proof.apply(DEFINITION, &mut runtime, &rows).unwrap();
    let next = runtime.regen.elements.get(&ElementId::new(ID)).unwrap();
    assert!(next.is_quarantined());
    assert_eq!(next.body, old.body);
    assert_eq!(next.descriptor, old.descriptor);
    assert_eq!(next.anchor, old.anchor);
    assert!(!next.has_rebind_evidence());
}

#[test]
fn opaque_consumer_vetoes_entire_batch_before_working_mutation() {
    let (mut runtime, mut proof, rows) = fixture();
    proof.document.modules.insert(
        ModuleId::parse("org.example.future").unwrap(),
        ModuleState::new(1, serde_json::json!({"indirect": 42})),
    );
    let before = runtime.regen.elements.clone();
    assert!(proof.apply(DEFINITION, &mut runtime, &rows).is_err());
    assert_eq!(runtime.regen.elements, before);
}

#[test]
fn mixed_missing_body_and_surviving_body_refuses_new_quarantine() {
    let (mut runtime, mut proof, rows) = fixture();
    let missing = ElementEntry::new(BodyId(Uuid::from_u128(99)), ElementKind::Face)
        .with_evidence(None, Some(serde_json::json!({"unclassified": true})));
    proof
        .document
        .definitions
        .get_mut(&DEFINITION)
        .unwrap()
        .elements
        .insert(
            ElementId::new("el_00000000000000000000000000000002"),
            missing,
        );
    let before = runtime.regen.elements.clone();
    assert!(proof.apply(DEFINITION, &mut runtime, &rows).is_err());
    assert_eq!(runtime.regen.elements, before);
}

#[test]
fn nonrepair_or_missing_answers_never_reserve_an_identity() {
    for empty in [false, true] {
        let (mut runtime, proof, mut rows) = fixture();
        if empty {
            rows.clear();
        } else {
            rows[0].outcome = ResolveOutcome::Unchanged {
                element_id: Some(ElementId::new(ID)),
            };
        }
        let before = runtime.regen.elements.clone();
        assert!(proof.apply(DEFINITION, &mut runtime, &rows).is_err());
        assert_eq!(runtime.regen.elements, before);
    }
}

#[test]
fn persisted_generation_staging_and_transaction_changes_refuse_commit() {
    let (mut runtime, proof, rows) = fixture();
    let commit = proof.apply(DEFINITION, &mut runtime, &rows).unwrap();
    runtime.check_quarantine_commit(&commit).unwrap();
    runtime.persisted_generation += 1;
    assert!(runtime.check_quarantine_commit(&commit).is_err());
    runtime.persisted_generation -= 1;
    runtime.staged.insert(
        ElementId::new("el_staged"),
        ElementEntry::new(BodyId(Uuid::from_u128(3)), ElementKind::Face),
    );
    assert!(runtime.check_quarantine_commit(&commit).is_err());
    runtime.staged.clear();
    runtime.session.begin_transaction("uncommitted work");
    assert!(runtime.check_quarantine_commit(&commit).is_err());
}

#[test]
fn stale_consumer_proof_cannot_adopt_definition_or_publish_cache() {
    let (mut runtime, proof, rows) = fixture();
    let prior = runtime.session.document().definitions[&DEFINITION].clone();
    let source_fingerprint = fingerprint(&prior).unwrap();
    let commit = proof.apply(DEFINITION, &mut runtime, &rows).unwrap();
    let mut definition = prior;
    definition.elements = runtime.regen.elements.clone();
    let output = ComponentReplayOutput {
        id: DEFINITION,
        runtime_session: runtime.runtime_session(),
        revision: runtime.revision(),
        epoch: WorkerEpoch(2),
        source_fingerprint,
        definition,
        meshes: BTreeMap::new(),
        budget: MeshBudget::default(),
        snapshot: SnapshotId(1),
        minted: false,
        replay_context: [0; 32],
        identity_signature: Some(BTreeMap::new()),
        quarantine: Some(commit),
        _retention: Arc::new(()),
    };
    runtime.persisted_generation += 1;
    let before = serde_json::to_value(runtime.session.document()).unwrap();
    let state = (
        runtime.persisted_generation(),
        runtime.component_generation,
        runtime.is_dirty(),
        runtime.revision(),
        runtime.undo_depth(),
        runtime.redo_depth(),
    );
    assert!(runtime
        .commit_component_replay(output, WorkerEpoch(2))
        .is_err());
    assert_eq!(
        serde_json::to_value(runtime.session.document()).unwrap(),
        before
    );
    assert_eq!(
        (
            runtime.persisted_generation(),
            runtime.component_generation,
            runtime.is_dirty(),
            runtime.revision(),
            runtime.undo_depth(),
            runtime.redo_depth()
        ),
        state
    );
    assert!(runtime.component_cache.is_empty());
}
