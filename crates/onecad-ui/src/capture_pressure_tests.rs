//! Public host captures must reclaim disposable preparation before waiting on themselves.
use super::*;
use onecad_core::sketch::WorldPlane;
use onecad_interaction::sketch::{SketchAction, SketchPhase};

async fn visible_until(host: &mut Host, ready: impl Fn(&App) -> bool) {
    for _ in 0..1000 {
        logic_tick(&mut host.app, &host.context, &mut host.frame, false);
        host.ui(vec![]);
        let adopted = host.app.doc.display.as_ref().is_some_and(|display| {
            host.app.renderer.scene_identity() == Some(display.scene.identity())
                && display.prepared.lock().unwrap().is_none()
        });
        if ready(&host.app) && adopted && host.app.renderer.poll_idle().unwrap() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("visible capture did not progress: {}", host.app.doc.status);
}

async fn sketch_cycle(host: &mut Host) {
    host.app.new_sketch_on_plane(WorldPlane::XY);
    visible_until(host, |app| {
        app.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    host.redraw();
    let revision = host.app.doc.display.as_ref().unwrap().projection.revision;
    host.app.sketch_dispatch(SketchAction::Finish);
    visible_until(host, |app| {
        !app.sketch_active()
            && !app.busy()
            && app.doc.bridge.valid()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|display| display.projection.revision > revision)
    })
    .await;
    host.redraw();
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn three_documents_repeat_unchanged_geometry_sketch_captures_and_undo() {
    let root = tempfile::tempdir().unwrap();
    let mut host = ipad(root.path()).await;
    let first = host.app.doc.tab;
    for parked in 1..=2 {
        host.app.document_request(Request::New);
        visible_until(&mut host, |app| {
            app.tabs.parked.len() == parked && app.document_open() && !app.busy()
        })
        .await;
        host.app.ui.start_open = false;
        host.redraw();
    }
    host.app.activate_tab(first);
    visible_until(&mut host, |app| app.doc.prepare_in_flight.is_none()).await;
    host.redraw();
    for _ in 0..3 {
        sketch_cycle(&mut host).await;
        let before = host.app.doc.display.as_ref().unwrap().projection.undo_depth;
        host.app.submit(crate::bridge::Command::Undo);
        visible_until(&mut host, |app| {
            app.doc.bridge.valid()
                && app.doc.display.as_ref().unwrap().projection.undo_depth < before
        })
        .await;
        host.redraw();
        let cpu = host.app.doc.bridge.shared.host_budget.snapshot();
        assert!(cpu.used <= crate::budget::Policy::for_profile(RenderProfile::Mobile).aggregate);
        assert_eq!(cpu.waiting, 0, "{cpu:?}");
        assert!(!cpu.degraded, "{cpu:?}");
        assert!(host.app.display_ledger().peak_total() <= host.app.display_budget.aggregate);
        assert_eq!(
            host.app.doc.display.as_ref().unwrap().scene.bodies().len(),
            1
        );
    }
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn nonpressure_cache_reuse_failure_and_final_prepared_gpu_guard_owners() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed_with(root.path(), RenderProfile::Mobile).await;
    let shared = host.app.doc.bridge.shared.clone();
    let original = shared
        .host_budget
        .snapshot()
        .portions
        .into_iter()
        .find(|portion| portion.kind == crate::budget::Kind::Preparation)
        .unwrap();
    let bytes = host.app.doc.display.as_ref().unwrap().scene.bodies()[0]
        .bytes()
        .clone();
    for _ in 0..2 {
        sketch_cycle(&mut host).await;
        assert!(live(&shared, original.id));
        assert!(std::sync::Arc::ptr_eq(
            &bytes,
            host.app.doc.display.as_ref().unwrap().scene.bodies()[0].bytes()
        ));
    }
    failed_capture_preserves_cache_and_document(&mut host).await;
    retained_owners_release_only_after_their_final_drop(&mut host, &original);
    host.stop().await;
}

fn live(shared: &crate::bridge::Shared, id: u64) -> bool {
    shared
        .host_budget
        .snapshot()
        .portions
        .iter()
        .any(|portion| portion.id == id)
}

async fn failed_capture_preserves_cache_and_document(host: &mut Host) {
    let shared = host.app.doc.bridge.shared.clone();
    let before = host.app.doc.document_state.clone().unwrap();
    let scene = host.app.doc.display.as_ref().unwrap().scene.identity();
    let cached = cache_bytes(&host.app.doc);
    let budget = shared.preparation_budget().unwrap();
    *shared.budget.lock().unwrap() = budget.limit_cpu_prepared_bytes(0);
    assert!(
        shared
            .host_budget
            .has_capacity(crate::budget::DISPLAY_RESERVATION)
    );
    host.app.new_sketch_on_plane(WorldPlane::XY);
    visible_until(host, |app| app.doc.status.contains("CPU preparation")).await;
    assert!(!shared.valid.load(std::sync::atomic::Ordering::Acquire));
    let current = host.app.doc.document_state.as_ref().unwrap();
    assert_eq!(current.document_id, before.document_id);
    assert_eq!(current.runtime_session, before.runtime_session);
    assert_eq!(current.dirty, before.dirty);
    assert!(current.revision >= before.revision);
    assert_eq!(
        host.app.doc.display.as_ref().unwrap().scene.identity(),
        scene
    );
    assert_eq!(cache_bytes(&host.app.doc), cached);
    *shared.budget.lock().unwrap() = budget;
}

fn retained_owners_release_only_after_their_final_drop(
    host: &mut Host,
    original: &crate::budget::Portion,
) {
    let shared = host.app.doc.bridge.shared.clone();
    let prepared = retained_copy(host);
    drop(shared.take_preparation_cache());
    assert!(
        live(&shared, original.id),
        "prepared geometry still owns its guard"
    );
    let source = shared
        .host_budget
        .snapshot()
        .portions
        .into_iter()
        .find(|portion| {
            portion.admission == original.admission
                && portion.kind == crate::budget::Kind::DisplaySource
        })
        .unwrap();
    let remaining = crate::budget::Policy::for_profile(RenderProfile::Mobile).aggregate
        - shared.host_budget.snapshot().used;
    let full = shared
        .host_budget
        .reserve_kind(remaining, crate::budget::Kind::Preparation)
        .unwrap();
    assert!(
        shared
            .host_budget
            .reserve_kind(1, crate::budget::Kind::Preparation)
            .is_err()
    );
    drop(prepared);
    assert!(!live(&shared, original.id));
    assert!(
        shared
            .host_budget
            .reserve_kind(64 << 20, crate::budget::Kind::Preparation)
            .is_ok()
    );
    drop(full);
    assert!(
        live(&shared, source.id),
        "the GPU body still owns its source guard"
    );
    let empty = onecad_scene::SceneSnapshot::new(
        onecad_scene::SceneIdentity {
            session: 0,
            scene: 0,
        },
        vec![],
        Default::default(),
    )
    .unwrap();
    host.app.renderer.replace_scene(&empty).unwrap();
    assert!(!live(&shared, source.id));
}

fn retained_copy(host: &Host) -> onecad_render::PreparedScene {
    let shared = &host.app.doc.bridge.shared;
    let display = host.app.doc.display.as_ref().unwrap();
    let retention = onecad_render::PreparationRetention {
        source: Some(display._lease.clone()),
        preparation: Some(std::sync::Arc::new(
            shared
                .host_budget
                .reserve_kind(
                    crate::budget::MAX_PREPARED_BYTES as usize,
                    crate::budget::Kind::Preparation,
                )
                .unwrap(),
        )),
    };
    let mut cache = shared.take_preparation_cache();
    let prepared = onecad_render::PreparedScene::new_cached_retained(
        &display.scene,
        shared.preparation_budget().unwrap(),
        &mut cache,
        retention,
    )
    .unwrap();
    shared.restore_preparation_cache(cache);
    prepared
}
