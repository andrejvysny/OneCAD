//! Keep the proven native prefix's ownership when checkpoint transport omits it.
use super::*;
use onecad_core::document::body::{BodyLifecycleEvent, BodyRegistry};

pub(super) fn restore(
    runtime: &mut DocumentRuntime,
    definition: &ComponentDef,
    prefix: usize,
    mut proven: BodyRegistry,
) -> Result<(), ApiError> {
    let before: BTreeSet<_> = proven.bodies().iter().map(|b| b.id).collect();
    let after: BTreeSet<_> = runtime.regen.bodies.bodies().iter().map(|b| b.id).collect();
    if before != after
        || before.len() != proven.bodies().len()
        || after.len() != runtime.regen.bodies.bodies().len()
    {
        return Err(invalid(
            "component checkpoint changed the body identity set",
        ));
    }
    if proven.bodies().iter().any(|body| {
        runtime
            .regen
            .bodies
            .get(body.id)
            .is_none_or(|current| current.health != body.health)
    }) {
        return Err(invalid("component continuation changed body health"));
    }
    let expected = expected_events(definition, prefix)?;
    let mut seen = BTreeSet::new();
    for entry in runtime
        .regen
        .bodies
        .log()
        .iter()
        .filter(|e| e.step_index >= prefix)
    {
        let BodyLifecycleEvent::Modified { body } = &entry.event else {
            return Err(invalid("component continuation changed body ownership"));
        };
        if !expected.contains(&(entry.step_index, *body)) || !seen.insert((entry.step_index, *body))
        {
            return Err(invalid(
                "component continuation has a duplicate or unexpected body event",
            ));
        }
    }
    if seen != expected {
        return Err(invalid("component continuation omitted a body event"));
    }
    for entry in runtime
        .regen
        .bodies
        .log()
        .iter()
        .filter(|e| e.step_index >= prefix)
    {
        let record = &definition.timeline.records()[entry.step_index];
        proven.fold(entry.step_index, record.record_id, entry.event.clone());
    }
    runtime.regen.bodies = proven;
    Ok(())
}

fn expected_events(
    definition: &ComponentDef,
    prefix: usize,
) -> Result<BTreeSet<(usize, BodyId)>, ApiError> {
    let suffix = definition
        .timeline
        .records()
        .get(prefix..definition.timeline.cursor())
        .ok_or_else(|| invalid("component continuation prefix is out of range"))?;
    let mut expected = BTreeSet::new();
    for (offset, record) in suffix.iter().enumerate() {
        let Operation::Known(KnownOperation::TransformBody(params)) = &record.op else {
            return Err(invalid(
                "component continuation lifecycle is not a transform",
            ));
        };
        if params.copy || record.suppressed || params.is_identity() || params.validate().is_err() {
            return Err(invalid(
                "component continuation has unsupported lifecycle semantics",
            ));
        }
        for body in &params.targets {
            expected.insert((prefix + offset, *body));
        }
    }
    if expected.is_empty() {
        return Err(invalid("component continuation has no body events"));
    }
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::{
        document::{
            body::BodyMeta,
            record::{OperationRecord, TransformBodyParams},
            variables::Scalar,
        },
        ids::RecordId,
    };

    fn fixture(
        events: Vec<(usize, BodyLifecycleEvent)>,
    ) -> (DocumentRuntime, ComponentDef, BodyRegistry) {
        let body = BodyId(Uuid::from_u128(11));
        let creator = RecordId(Uuid::from_u128(12));
        let retired = BodyId(Uuid::from_u128(10));
        let mut proven = BodyRegistry::new();
        proven.fold(0, creator, BodyLifecycleEvent::Created { body: retired });
        proven.fold(
            1,
            creator,
            BodyLifecycleEvent::Split {
                parent: retired,
                children: vec![body],
            },
        );
        let engine = Arc::new(crate::worker::PendingBackend);
        let mut runtime = DocumentRuntime::new_blank(engine.clone(), engine.clone(), engine);
        runtime
            .regen
            .bodies
            .register(BodyMeta::new(body, "checkpoint", RecordId(Uuid::nil())));
        let op = Operation::Known(KnownOperation::TransformBody(TransformBodyParams {
            targets: vec![body],
            translate: [Scalar::new(1.0), Scalar::new(0.0), Scalar::new(0.0)],
            rotate: Default::default(),
            copy: false,
            extra: Default::default(),
        }));
        let mut def = ComponentDef::new(ComponentDefId(Uuid::new_v4()), "native");
        def.timeline = onecad_core::history::Timeline::from_records(
            (0..3)
                .map(|index| {
                    OperationRecord::new(
                        RecordId(Uuid::from_u128(12 + index)),
                        index as u32,
                        "step",
                        op.clone(),
                    )
                })
                .collect(),
        );
        for (step, event) in events {
            runtime.regen.bodies.fold(step, creator, event);
        }
        (runtime, def, proven)
    }

    #[test]
    fn restored_prefix_retains_producer_aliases_and_lifecycle() {
        let body = BodyId(Uuid::from_u128(11));
        let (mut runtime, def, proven) = fixture(vec![(2, BodyLifecycleEvent::Modified { body })]);
        restore(&mut runtime, &def, 2, proven.clone()).unwrap();
        assert_eq!(
            runtime.regen.bodies.get(body).unwrap().created_by,
            RecordId(Uuid::from_u128(12))
        );
        assert_eq!(runtime.regen.bodies.aliases(), proven.aliases());
        assert_eq!(&runtime.regen.bodies.log()[..2], proven.log());
        assert_eq!(runtime.regen.bodies.log().len(), 3);
    }

    #[test]
    fn lifecycle_refuses_empty_duplicate_wrong_step_and_created_events() {
        let body = BodyId(Uuid::from_u128(11));
        for events in [
            vec![],
            vec![
                (2, BodyLifecycleEvent::Modified { body }),
                (2, BodyLifecycleEvent::Modified { body }),
            ],
            vec![(3, BodyLifecycleEvent::Modified { body })],
            vec![(2, BodyLifecycleEvent::Created { body })],
        ] {
            let (mut runtime, def, proven) = fixture(events);
            let before = runtime.regen.bodies.clone();
            assert!(restore(&mut runtime, &def, 2, proven).is_err());
            assert_eq!(runtime.regen.bodies, before);
        }
    }

    #[test]
    fn lifecycle_refuses_missing_targets_and_unexpected_bodies() {
        let body = BodyId(Uuid::from_u128(11));
        let other = BodyId(Uuid::from_u128(21));
        for wrong in [false, true] {
            let (mut runtime, mut def, mut proven) =
                fixture(vec![(2, BodyLifecycleEvent::Modified { body })]);
            let meta = BodyMeta::new(other, "other", RecordId(Uuid::from_u128(22)));
            proven.register(meta.clone());
            runtime.regen.bodies.register(meta);
            let mut records = def.timeline.records().to_vec();
            let Operation::Known(KnownOperation::TransformBody(params)) = &mut records[2].op else {
                unreachable!()
            };
            if wrong {
                params.targets = vec![other];
            } else {
                params.targets.push(other);
            }
            def.timeline = onecad_core::history::Timeline::from_records(records);
            let before = runtime.regen.bodies.clone();
            assert!(restore(&mut runtime, &def, 2, proven).is_err());
            assert_eq!(runtime.regen.bodies, before);
        }
    }

    #[test]
    fn lifecycle_never_clears_or_discards_quarantine() {
        use onecad_core::document::body::BodyHealth;
        let body = BodyId(Uuid::from_u128(11));
        for changed_prefix in [false, true] {
            let (mut runtime, def, mut proven) =
                fixture(vec![(2, BodyLifecycleEvent::Modified { body })]);
            if changed_prefix {
                proven.set_health(body, BodyHealth::Quarantined);
            } else {
                runtime
                    .regen
                    .bodies
                    .set_health(body, BodyHealth::Quarantined);
            }
            let before = runtime.regen.bodies.clone();
            assert!(restore(&mut runtime, &def, 2, proven).is_err());
            assert_eq!(runtime.regen.bodies, before);
        }
    }
}
