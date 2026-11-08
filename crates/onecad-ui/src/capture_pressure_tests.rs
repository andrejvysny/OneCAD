//! Public host captures must reclaim disposable preparation before waiting on themselves.
use super::*;
#[path = "capture_pressure_tests/cancellation.rs"]
mod cancellation;
use cancellation::{close_while_prepare_waits, repeat_cancelled_preparation};
use onecad_core::sketch::WorldPlane;
use onecad_interaction::sketch::{SketchAction, SketchPhase};

async fn visible_until(host: &mut Host, ready: impl Fn(&App) -> bool) {
    for _ in 0..4000 {
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

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn ten_definitions_twenty_occurrences_reach_metal_and_release() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("assembly.onecad");
    assembly_fixture(&path);
    let mut host = ipad(root.path()).await;
    host.redraw();
    let prior = read_output(&host);
    host.app.document_request(Request::Open(Some(path)));
    visible_until(&mut host, |app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|d| d.scene.bodies().len() == 20)
    })
    .await;
    host.redraw();
    let display = host.app.doc.display.as_ref().unwrap();
    assert_eq!(display.assembly.definitions.len(), 11); // Root plus ten embedded definitions.
    assert_eq!(display.assembly.occurrences.len(), 20);
    assert!(display.scene.bodies()[0].shares_geometry(&display.scene.bodies()[1]));
    let pixels = read_output(&host);
    assert_ne!(
        pixels, prior,
        "accepted assembly must change the rendered viewport"
    );
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[..3] != pixels[..3])
    );
    let budget = host.app.doc.bridge.shared.host_budget.clone();
    if let Some(path) = std::env::var_os("ONECAD_RENDER_CAPTURE_DIR") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        let [width, height] = host.app.renderer.size();
        let file =
            std::fs::File::create(path.join("assembly-10-definitions-20-occurrences.png")).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        let snapshot = budget.snapshot();
        let portions: Vec<_> = snapshot
            .portions
            .iter()
            .map(|p| {
                serde_json::json!({
                    "owner": p.owner, "kind": format!("{:?}", p.kind), "bytes": p.bytes,
                })
            })
            .collect();
        std::fs::write(
            path.join("assembly-ledger.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "dimensions": [width, height], "definitions": 10, "occurrences": 20,
                "used": snapshot.used, "portions": portions,
                "worker": std::env::var("ONECAD_WORKER_PATH").unwrap(), "renderer": "Metal",
            }))
            .unwrap(),
        )
        .unwrap();
    }
    eprintln!(
        "Metal 10 definitions / 20 occurrences: {:?}",
        budget.snapshot()
    );
    host.stop().await;
    assert_eq!(budget.snapshot().used, 0);
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn denied_prepare_preserves_viewport_and_retries_after_release() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed_with(root.path(), RenderProfile::Mobile).await;
    let budget = host.app.doc.bridge.shared.host_budget.clone();
    budget.activate();
    let display = host.app.doc.display.as_ref().unwrap();
    let identity = display.scene.identity();
    let source = crate::viewport_updates::Source {
        scene: display.scene.clone(),
        lease: display._lease.clone(),
    };
    let remaining = crate::budget::Policy::for_profile(RenderProfile::Mobile).aggregate
        - budget.snapshot().used;
    let full = budget
        .reserve_kind(remaining, crate::budget::Kind::Preparation)
        .unwrap();
    host.app
        .doc
        .bridge
        .submit(crate::bridge::Command::Prepare(source.clone()))
        .unwrap();
    host.app.doc.prepare_in_flight = Some(identity);
    wait_prepare(&mut host, |app| {
        app.doc.prepare_in_flight.is_none() && app.doc.status.contains("budget")
    })
    .await;
    assert_eq!(host.app.renderer.scene_identity(), Some(identity));
    assert_eq!(
        host.app.doc.display.as_ref().unwrap().scene.identity(),
        identity
    );
    drop(full);
    host.app
        .doc
        .bridge
        .submit(crate::bridge::Command::Prepare(source.clone()))
        .unwrap();
    host.app.doc.prepare_in_flight = Some(identity);
    wait_prepare(&mut host, |app| app.doc.prepare_in_flight.is_none()).await;
    assert_eq!(host.app.renderer.scene_identity(), Some(identity));
    drop(source);
    repeat_cancelled_preparation(&mut host).await;
    host.stop().await;
    assert_eq!(budget.snapshot().used, 0);
}

async fn wait_prepare(host: &mut Host, done: impl Fn(&App) -> bool) {
    for _ in 0..1500 {
        logic_tick(&mut host.app, &host.context, &mut host.frame, false);
        host.ui(vec![]);
        if done(&host.app) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("prepare did not finish: {}", host.app.doc.status);
}

fn assembly_fixture(path: &std::path::Path) {
    use onecad_core::document::{
        Document,
        component::{
            ComponentDef, ComponentDefId, Occurrence, OccurrenceId, Quat, RigidTransform, Vec3d,
        },
    };
    use onecad_core::ids::DocumentId;
    use onecad_core::io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../onecad-app/tests/fixtures/qa_plate_hole.onecad");
    let source = ContainerReader::open(&fixture).unwrap();
    let source = source.document();
    let mut document = Document::new(DocumentId(uuid::Uuid::new_v4()));
    for d in 0..10u128 {
        let definition = ComponentDefId(uuid::Uuid::from_u128(0x1000 + d));
        let mut component = ComponentDef::new(definition, "Plate");
        component.variables = source.variables.clone();
        component.sketches = source.sketches.clone();
        component.sketch_visibility = source.sketch_visibility.clone();
        component.datum_planes = source.datum_planes.clone();
        component.timeline = source.timeline.clone();
        document.definitions.insert(definition, component);
        for n in 0..2u128 {
            document
                .assembly
                .occurrences
                .insert(Occurrence {
                    id: OccurrenceId(uuid::Uuid::from_u128(0x2000 + 2 * d + n)),
                    def: definition,
                    parent: None,
                    local: RigidTransform::new(
                        Quat::IDENTITY,
                        Vec3d::new(n as f64 * 50.0, d as f64 * 50.0, 0.0),
                    ),
                    grounded: false,
                    name: format!("Plate {d} {n}"),
                })
                .unwrap();
        }
    }
    ContainerWriter::save(
        path,
        &document,
        &ContainerCaches::none(),
        &SaveMeta::default(),
    )
    .unwrap();
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
    let mut steady = None;
    for cycle in 0..20 {
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
        let display = host.app.doc.display.as_ref().unwrap();
        eprintln!(
            "handoff cycle {cycle}: bodies={} records={} profiles={} sketches={} features={} {cpu:?}",
            display.scene.bodies().len(),
            display.records.len(),
            display.profiles.len(),
            display.projection.sketches.len(),
            display.projection.features.len()
        );
        let first = *steady.get_or_insert(cpu.used);
        assert!(
            cpu.used <= first + (4 << 20),
            "handoff retention grew: {cpu:?}"
        );
        assert!(cpu.used <= crate::budget::Policy::for_profile(RenderProfile::Mobile).aggregate);
        assert_eq!(cpu.waiting, 0, "{cpu:?}");
        assert!(!cpu.degraded, "{cpu:?}");
        assert!(host.app.display_ledger().peak_total() <= host.app.display_budget.aggregate);
        assert_eq!(
            host.app.doc.display.as_ref().unwrap().scene.bodies().len(),
            1
        );
    }
    let budget = host.app.doc.bridge.shared.host_budget.clone();
    close_while_prepare_waits(&mut host, &budget).await;
    let next_budget = host.app.doc.bridge.shared.host_budget.clone();
    close_while_prepare_waits(&mut host, &next_budget).await;
    host.stop().await;
    let after = budget.snapshot();
    eprintln!("handoff after close: {after:?}");
    assert_eq!(after.used, 0, "handoff leases survived close: {after:?}");
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
        .find(|portion| portion.kind == crate::budget::Kind::PreparedGeometry)
        .unwrap();
    let bytes = host.app.doc.display.as_ref().unwrap().scene.bodies()[0].retained_geometry();
    for _ in 0..2 {
        sketch_cycle(&mut host).await;
        assert!(live(&shared, original.id));
        assert!(bytes.shares_source(
            &host.app.doc.display.as_ref().unwrap().scene.bodies()[0].retained_geometry()
        ));
    }
    drop(bytes);
    failed_capture_preserves_cache_and_document(&mut host).await;
    let source_id = retained_owners_release_only_after_their_final_drop(&mut host, &original);
    host.stop().await;
    assert!(!live(&shared, source_id));
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
    // Viewport frames refresh the renderer budget while capture runs off-thread.
    shared
        .test_cpu_preparation_limit
        .store(0, std::sync::atomic::Ordering::Release);
    host.ui(vec![]);
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
    shared
        .test_cpu_preparation_limit
        .store(u64::MAX, std::sync::atomic::Ordering::Release);
}

fn retained_owners_release_only_after_their_final_drop(
    host: &mut Host,
    original: &crate::budget::Portion,
) -> u64 {
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
                && portion.kind == crate::budget::Kind::DisplayGeometry
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
    assert!(
        live(&shared, source.id),
        "the accepted CPU scene still owns its source guard"
    );
    source.id
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
