use super::*;

async fn await_budget_waiter(host: &mut Host, budget: &std::sync::Arc<crate::budget::HostBudget>) {
    for _ in 0..400 {
        if budget.snapshot().waiting == 1 {
            return;
        }
        host.ui(vec![]);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("prepare never queued: {:?}", budget.snapshot());
}

pub(super) async fn repeat_cancelled_preparation(host: &mut Host) {
    let shared = host.app.doc.bridge.shared.clone();
    let budget = shared.host_budget.clone();
    let owner = budget.owner_id();
    let aggregate = crate::budget::Policy::for_profile(RenderProfile::Mobile).aggregate;
    let mut steady = None;
    for cycle in 0..3 {
        let display = host.app.doc.display.as_ref().unwrap();
        let identity = display.scene.identity();
        let source = crate::viewport_updates::Source {
            scene: display.scene.clone(),
            lease: display._lease.clone(),
        };
        let baseline = budget.snapshot();
        assert_eq!(baseline.waiting, 0, "cycle {cycle}: {baseline:?}");
        assert!(baseline.owners.contains_key(&owner));
        let full = budget
            .reserve_kind(aggregate - baseline.used, crate::budget::Kind::Preparation)
            .unwrap();
        host.app
            .doc
            .bridge
            .submit(crate::bridge::Command::Prepare(source.clone()))
            .unwrap();
        host.app.doc.prepare_in_flight = Some(identity);
        await_budget_waiter(host, &budget).await;
        let held = budget.snapshot();
        assert_eq!(held.used, aggregate, "cycle {cycle}: {held:?}");
        assert_eq!(
            held.owners.values().map(|entry| entry.used).sum::<usize>(),
            held.used
        );
        assert_eq!(host.app.renderer.scene_identity(), Some(identity));
        assert_eq!(
            host.app.doc.display.as_ref().unwrap().scene.identity(),
            identity
        );
        shared.invalidate(None);
        assert_eq!(host.app.renderer.scene_identity(), Some(identity));
        drop(full);
        prepare_receipt(host, identity, false).await;
        visible_until(host, |app| app.doc.bridge.valid() && !app.busy()).await;
        retry_preparation(host).await;
        drop(source);
        let settled = budget.snapshot();
        assert_eq!(settled.waiting, 0, "cycle {cycle}: {settled:?}");
        assert!(!settled.degraded, "cycle {cycle}: {settled:?}");
        let first = *steady.get_or_insert(settled.used);
        assert_eq!(settled.used, first, "cycle {cycle}: {settled:?}");
        assert!(settled.used <= aggregate);
        eprintln!("handoff cancellation cycle {cycle}: held={held:?}; settled={settled:?}");
    }
}

async fn prepare_receipt(host: &mut Host, expected: onecad_scene::SceneIdentity, success: bool) {
    for _ in 0..1500 {
        match host.app.doc.bridge.messages.try_recv() {
            Ok(message @ crate::bridge::Message::Prepared { .. }) => {
                if let crate::bridge::Message::Prepared {
                    identity, result, ..
                } = &message
                {
                    assert_eq!(*identity, expected);
                    match result {
                        Ok(prepared) => {
                            assert!(success, "cancelled preparation succeeded");
                            assert_eq!(prepared.identity(), expected);
                        }
                        Err(error) => {
                            assert!(!success, "retry failed: {error}");
                            assert!(error.contains("admission cancelled"), "{error}");
                        }
                    }
                }
                host.app.message(message, &host.context);
                assert!(host.app.doc.prepare_in_flight.is_none());
                return;
            }
            Ok(_) => panic!("unexpected service message while awaiting preparation receipt"),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => panic!("service disconnected"),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await
            }
        }
    }
    panic!("preparation produced no terminal receipt");
}

async fn retry_preparation(host: &mut Host) {
    let display = host.app.doc.display.as_ref().unwrap();
    let identity = display.scene.identity();
    let source = crate::viewport_updates::Source {
        scene: display.scene.clone(),
        lease: display._lease.clone(),
    };
    host.app
        .doc
        .bridge
        .submit(crate::bridge::Command::Prepare(source))
        .unwrap();
    host.app.doc.prepare_in_flight = Some(identity);
    prepare_receipt(host, identity, true).await;
    host.redraw();
    assert_eq!(host.app.renderer.scene_identity(), Some(identity));
}

pub(super) async fn close_while_prepare_waits(
    host: &mut Host,
    budget: &std::sync::Arc<crate::budget::HostBudget>,
) {
    let owner = budget.owner_id();
    let source_display = host.app.doc.display.as_ref().unwrap();
    let identity = source_display.scene.identity();
    let source = crate::viewport_updates::Source {
        scene: source_display.scene.clone(),
        lease: source_display._lease.clone(),
    };
    let session = host
        .app
        .doc
        .document_state
        .as_ref()
        .unwrap()
        .runtime_session
        .clone();
    let before = budget.snapshot();
    let aggregate = crate::budget::Policy::for_profile(RenderProfile::Mobile).aggregate;
    let full = budget
        .reserve_kind(aggregate - before.used, crate::budget::Kind::Preparation)
        .unwrap();
    host.app
        .doc
        .bridge
        .submit(crate::bridge::Command::Prepare(source.clone()))
        .unwrap();
    host.app.doc.prepare_in_flight = Some(identity);
    await_budget_waiter(host, budget).await;
    let held = budget.snapshot();
    assert_eq!(held.used, aggregate, "{held:?}");
    assert!(held.owners.get(&owner).is_some_and(|entry| entry.used > 0));
    assert_eq!(host.app.renderer.scene_identity(), Some(identity));
    host.app.close_active_tab(Some(&session)).unwrap();
    let closing = budget.snapshot();
    assert!(
        closing
            .owners
            .get(&owner)
            .is_some_and(|entry| entry.used > 0),
        "{closing:?}"
    );
    drop(full);
    visible_until(host, |app| app.document_open() && !app.busy()).await;
    let retained = budget.snapshot();
    assert_eq!(retained.waiting, 0, "{retained:?}");
    assert!(
        retained
            .owners
            .get(&owner)
            .is_some_and(|entry| entry.used > 0),
        "{retained:?}"
    );
    drop(source);
    for _ in 0..400 {
        let after = budget.snapshot();
        if !after.owners.contains_key(&owner) {
            eprintln!(
                "handoff replacement owner {owner}: retained={retained:?}; released={after:?}"
            );
            return;
        }
        logic_tick(&mut host.app, &host.context, &mut host.frame, false);
        host.ui(vec![]);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!(
        "closed document retained ledger bytes: {:?}",
        budget.snapshot()
    );
}
