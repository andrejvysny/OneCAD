use onecad_core::{
    document::refs::SketchRegionRef,
    ids::{RecordId, RegionId, SketchId, SnapshotId},
};
use onecad_interaction::*;
use onecad_scene::{Camera, SceneIdentity, ViewSnapshot, Viewport};
use uuid::Uuid;

pub fn context() -> DocumentContext {
    DocumentContext {
        document_id: "doc-a".into(),
        runtime_session: "runtime-a".into(),
        revision: 4,
        snapshot_id: SnapshotId(8),
        scene: SceneIdentity {
            session: 1,
            scene: 2,
        },
    }
}

pub fn selection(context: DocumentContext) -> ProfileSelection {
    let capture = ViewSnapshot::new(
        context.scene,
        1,
        Camera::default(),
        Viewport {
            width: 640.0,
            height: 480.0,
            scale_factor: 2.0,
        },
    )
    .unwrap()
    .identity();
    ProfileSelection {
        profile: SketchRegionRef {
            sketch: SketchId(Uuid::from_u128(1)),
            region: RegionId("region-1".into()),
            region_identity_version: Some(1),
            region_anchor: Some([3.0, 4.0]),
            extra: Default::default(),
        },
        context,
        capture,
    }
}

pub fn started() -> (Interaction, RequestTag) {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    state.dispatch(Action::SelectProfile(Box::new(selection(context()))));
    let effects = state.dispatch(Action::BeginExtrude {
        record_id: RecordId(Uuid::from_u128(2)),
        gesture: 9,
        step_index: 3,
    });
    let tag = evaluation_tag(&effects);
    (state, tag)
}

pub fn evaluation_tag(effects: &[Effect]) -> RequestTag {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::EvaluateExpression { tag, site, .. } => {
                assert_eq!(*site, "length");
                Some(tag.clone())
            }
            _ => None,
        })
        .expect("expression evaluation intent")
}

pub fn evaluate(state: &mut Interaction, tag: &RequestTag, value: f64) -> Vec<Effect> {
    state.dispatch(Action::Evaluated {
        tag: tag.clone(),
        result: Ok(EvaluatedExpressionDto {
            value,
            dimension: "length".into(),
            error: None,
        }),
    })
}

pub fn preview(tag: &RequestTag) -> PreviewResultDto {
    PreviewResultDto {
        snapshot_id: tag.context.snapshot_id.0,
        bodies: Vec::new(),
        body_events: Vec::new(),
        changed_bodies: Vec::new(),
        deleted_bodies: Vec::new(),
        needs_repair: Vec::new(),
    }
}

pub fn completion(tag: &RequestTag, source_revision: u64) -> RegenFinished {
    RegenFinished {
        runtime_session: tag.context.runtime_session.clone(),
        revision: source_revision + 1,
        source_revision,
        outcome: "published".into(),
        message: None,
        diagnostics: Vec::new(),
        failed_steps: Vec::new(),
        affected_bodies: Default::default(),
        repair_steps: Vec::new(),
    }
}

pub fn confirmed() -> (Interaction, RequestTag) {
    let (mut state, tag) = started();
    evaluate(&mut state, &tag, 20.0);
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(preview(&tag)),
    });
    let effects = state.dispatch(Action::Confirm);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::ApplyOperation { .. }))
            .count(),
        1
    );
    (state, tag)
}

pub fn editing() -> (Interaction, RequestTag) {
    use onecad_core::document::record::{KnownOperation, Operation};
    let (mut state, _) = started();
    let mut record = state.tool.as_ref().unwrap().record.clone();
    if let Operation::Known(KnownOperation::Extrude(params)) = &mut record.op {
        params.draft_angle_deg.value = 2.5;
    }
    state.dispatch(Action::Cancel);
    evaluation_tag(&state.dispatch(Action::BeginEdit {
        record: Box::new(record),
        gesture: 10,
        context: context(),
    }));
    // Untouched authored text is a no-op edit; a real change authors a command.
    let tag = evaluation_tag(&state.dispatch(Action::DraftChanged("8".into())));
    (state, tag)
}
