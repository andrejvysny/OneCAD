//! Container mesh/preview cache gate — "reopening a document paints last-saved
//! geometry IMMEDIATELY", end to end.
//!
//! An explicit save embeds the head snapshot's coarse MESH1 blobs (and a viewport
//! `preview.png`) in the `.onecad` container; the next open reads them back and
//! serves them from [`DocumentRuntime::get_mesh`] until the from-0 regen publishes.
//! `open_render.rs` pins the happy path through the real scheduler. This file pins
//! the boundaries, which is where a cache turns from an optimization into a lie:
//!
//! * **staleness is fatal to the whole set** — a container whose `opsHash` no longer
//!   matches its records describes geometry the timeline no longer produces, so NOT
//!   ONE blob may be painted (`stale_container_serves_no_cached_geometry`);
//! * **a publish retires them** — once real geometry exists the saved bytes are
//!   dropped and every fetch is answered at the live generation
//!   (`publish_retires_the_container_cache`);
//! * **the autosave lane writes none of it** — no `meshes/`, no `preview.png`, even
//!   while the live runtime is holding one (`autosave_writes_no_cache_sections`);
//! * **a corrupt blob is skipped, not served** — a `meshes/` entry that is not valid
//!   MESH1 never reaches the viewport (`non_mesh1_cache_entries_are_skipped`);
//! * **the start screen reads previews without opening documents**
//!   (`recents_thumbnails_come_from_the_container_preview`).
//!
//! The [`MAX_SAVED_MESH_BYTES`] truncation and the preview carry-forward rule are
//! pinned in `document_runtime/tests.rs`
//! (`saved_mesh_budget_truncates_deterministically`,
//! `save_without_a_capture_carries_the_preview_forward`), where the private
//! `mesh_cache` can be stuffed directly. Worker-backed tests here are
//! REQUIRE_WORKER-guarded (CI hard-fails without a worker; local dev skips
//! cleanly); the container-only tests need no worker and always run.
//!
//! [`MAX_SAVED_MESH_BYTES`]: onecad_lib::document_runtime::MAX_SAVED_MESH_BYTES

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, DocumentId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::{
    self, ContainerCaches, ContainerWriter, MeshCache, SaveMeta, MESHES_DIR, PREVIEW_PATH,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport, SaveCaches};
use onecad_lib::dto::RecentProjectDto;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::{
    resolve_worker_path, MeshProvider, PendingBackend, SolverEngine, WorkerManager,
};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors checkpoints.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} set but no binary there"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail here)"
    );
    None
}

async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "worker must connect + OpenSession"
    );
    wm
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

fn open_over(wm: &WorkerManager, path: &Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen container")
}

async fn regen(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-7.9.3".into()),
        created: "2026-08-05T00:00:00Z".into(),
        modified: "2026-08-05T00:00:00Z".into(),
    }
}

// ── Op builders (a fully-constrained rectangle extruded to a box) ─────────────

const SK_A: u128 = 0xA00;
const EX_A: u128 = 0xA01;

fn xy_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id, x, y| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    };
    pt(&mut sk, p0s, x0, y0);
    pt(&mut sk, p0e, x0 + w, y0);
    pt(&mut sk, p1s, x0 + w, y0);
    pt(&mut sk, p1e, x0 + w, y0 + h);
    pt(&mut sk, p2s, x0 + w, y0 + h);
    pt(&mut sk, p2e, x0, y0 + h);
    pt(&mut sk, p3s, x0, y0 + h);
    pt(&mut sk, p3e, x0, y0);
    for (id, a, b) in [
        (l0, p0s, p0e),
        (l1, p1s, p1e),
        (l2, p2s, p2e),
        (l3, p3s, p3e),
    ] {
        sk.add_entity(SketchEntity::line(id, a, b, false)).unwrap();
    }
    let coincident = |sk: &mut Sketch, id, a, b| {
        sk.add_constraint(Constraint::Coincident {
            id,
            point1: a,
            point2: b,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    };
    coincident(&mut sk, c(1), p0e, p1s);
    coincident(&mut sk, c(2), p1e, p2s);
    coincident(&mut sk, c(3), p2e, p3s);
    coincident(&mut sk, c(4), p3e, p0s);
    sk.add_constraint(Constraint::Horizontal { id: c(5), line: l0 })
        .unwrap();
    sk.add_constraint(Constraint::Horizontal { id: c(6), line: l2 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(7), line: l1 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(8), line: l3 })
        .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: c(9),
        point: p0s,
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(x0, y0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: p0s,
        point1_position: CurvePosition::Arbitrary,
        point2: p0e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: p1s,
        point1_position: CurvePosition::Arbitrary,
        point2: p1e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = onecad_lib::worker::wire::sketch_wire(sk);
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(SketchOpParams {
            sketch: sk.id,
            plane: xy_plane_ref(),
            entities: entities.as_array().cloned().unwrap_or_default(),
            constraints: constraints.as_array().cloned().unwrap_or_default(),
            host_face: None,
            extra: Default::default(),
        })),
    )
}

fn extrude_record(rec: u128, sketch: SketchId, dist: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""),
                region_identity_version: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(dist),
            draft_angle_deg: Scalar::new(0.0),
            mode: ExtrudeMode::Blind,
            boolean_mode: BooleanMode::NewBody,
            target_body: None,
            target_face: None,
            two_directions: false,
            mode2: ExtrudeMode::Blind,
            distance2: Scalar::new(0.0),
            target_face2: None,
            extra: Default::default(),
        })),
    )
}

/// Builds sketch+extrude into `rt`, regens, and returns the published body.
async fn build_and_regen(rt: &mut DocumentRuntime) -> BodyId {
    let sa = SketchId(Uuid::from_u128(0xA));
    for record in [
        sketch_record(SK_A, &rect_sketch(sa, 0x1000, 0.0, 0.0, 40.0, 20.0)),
        extrude_record(EX_A, sa, 25.0),
    ] {
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .expect("AddOperation");
    }
    let report = regen(rt).await;
    let snap = published(&report, "build");
    assert_eq!(snap.bodies.len(), 1, "one box body");
    snap.bodies[0].body
}

// ── Container surgery + inspection ───────────────────────────────────────────

/// Every non-directory entry name inside a container.
fn zip_entry_names(path: &Path) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let f = archive.by_index(i).unwrap();
        if !f.is_dir() {
            names.push(f.name().to_string());
        }
    }
    names
}

/// Rewrites every entry of a container through `edit`, which may mutate the bytes
/// of the entry it is handed (the tamper primitive; the manifest is an entry too).
fn rewrite_entries(path: &Path, mut edit: impl FnMut(&str, &mut Vec<u8>)) {
    let bytes = std::fs::read(path).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        entries.push((name, buf));
    }
    drop(archive);
    let out = std::fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(out);
    for (name, mut buf) in entries {
        edit(&name, &mut buf);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file(&name, opts).unwrap();
        zip.write_all(&buf).unwrap();
    }
    zip.finish().unwrap();
}

/// Desynchronizes the container's cache-freshness token from its records — the
/// state a document reaches when its timeline moved on under a stale cache set.
///
/// Done by rewriting `manifest.json`'s `opsHash`, NOT by tampering
/// `timeline/ops.jsonl`: staleness is `ops_hash(document.json records) !=
/// manifest.opsHash`, and `ops.jsonl` is a DERIVED projection whose corruption only
/// raises an `ops-jsonl-integrity` warning (`document.json` wins). Tampering the
/// projection would prove nothing about cache freshness.
fn desync_ops_hash(path: &Path) {
    rewrite_entries(path, |name, buf| {
        if name == "manifest.json" {
            let mut m: serde_json::Value = serde_json::from_slice(buf).unwrap();
            m["opsHash"] = serde_json::json!("deadbeefdeadbeef");
            *buf = serde_json::to_vec(&m).unwrap();
        }
    });
}

/// A 64-byte MESH1 blob with an empty section table — the smallest thing
/// `validate_mesh_blob` accepts.
fn minimal_mesh1() -> Vec<u8> {
    let mut buf = vec![0u8; 64];
    buf[0..4].copy_from_slice(&0x4D45_5348u32.to_le_bytes()); // magic
    buf[4..6].copy_from_slice(&1u16.to_le_bytes()); // version
    buf
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker-backed: the cache is real geometry, and it is retired correctly
// ─────────────────────────────────────────────────────────────────────────────

/// A container whose `opsHash` no longer matches its records has caches that
/// describe a document state the timeline does not produce. Painting ANY of them
/// would show the user geometry their model does not have — so the whole set is
/// dropped, and the pre-publish window is a miss again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_container_serves_no_cached_geometry() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let fresh = tmp.path().join("fresh.onecad");
    let stale = tmp.path().join("stale.onecad");

    let body = {
        let wm = spawn_worker(bin.clone()).await;
        let mut rt = runtime_over(&wm);
        let body = build_and_regen(&mut rt).await;
        rt.save(&fresh, save_meta()).expect("save");
        std::fs::copy(&fresh, &stale).unwrap();
        wm.shutdown().await;
        body
    };

    // Control: the untouched container DOES serve its cache.
    let wm = spawn_worker(bin).await;
    {
        let mut rt = open_over(&wm, &fresh);
        assert_eq!(rt.projection().geometry_source, "cached");
        let bytes = rt
            .get_mesh(body, Lod::Fine, None)
            .await
            .expect("fresh container serves its cache");
        onecad_protocol::mesh::validate_mesh_blob(&bytes)
            .expect("the served bytes are a valid MESH1 blob");
    }

    // The stale twin serves nothing at all.
    desync_ops_hash(&stale);
    let mut rt = open_over(&wm, &stale);
    assert_eq!(
        rt.projection().geometry_source,
        "none",
        "a stale container has no paintable geometry"
    );
    assert!(
        rt.get_mesh(body, Lod::Fine, None).await.is_none(),
        "not one stale blob may be served"
    );
    // …and it still opens, regens, and publishes normally.
    let report = regen(&mut rt).await;
    published(&report, "stale reopen");
    assert_eq!(rt.projection().geometry_source, "live");
    assert!(rt.get_mesh(body, Lod::Fine, None).await.is_some());

    wm.shutdown().await;
}

/// The container cache lives exactly until the first publish. After it, fetches are
/// answered at the LIVE generation — including generation-pinned ones, which the
/// container cache can never answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publish_retires_the_container_cache() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("model.onecad");

    let body = {
        let wm = spawn_worker(bin.clone()).await;
        let mut rt = runtime_over(&wm);
        let body = build_and_regen(&mut rt).await;
        rt.save(&path, save_meta()).expect("save");
        wm.shutdown().await;
        body
    };

    let wm = spawn_worker(bin).await;
    let mut rt = open_over(&wm, &path);

    let cached = rt
        .get_mesh(body, Lod::Fine, None)
        .await
        .expect("pre-publish hit");

    let report = regen(&mut rt).await;
    let generation = published(&report, "open regen").generation;
    assert!(generation >= 1, "generations are minted from 1");

    let live = rt
        .get_mesh(body, Lod::Fine, None)
        .await
        .expect("post-publish hit");
    onecad_protocol::mesh::validate_mesh_blob(&live).expect("valid MESH1");
    assert!(
        rt.get_mesh(body, Lod::Fine, Some(generation))
            .await
            .is_some(),
        "the live generation is addressable by number"
    );
    // Deterministic replay ⇒ the same box ⇒ the same bytes. The point is not that
    // they differ, it is that they now come from the published snapshot: a
    // generation-pinned fetch would have MISSED before the publish.
    assert_eq!(
        cached.len(),
        live.len(),
        "the saved cache described the same body the replay produced"
    );

    wm.shutdown().await;
}

/// An autosave is a crash snapshot, read only by a recovery path that regens from
/// 0. It must not pay the container bytes for caches nobody reads — and it must not
/// inherit the live document's preview either.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn autosave_writes_no_cache_sections() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: no worker binary (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let explicit = tmp.path().join("explicit.onecad");
    let auto = tmp.path().join("auto.onecad");

    let wm = spawn_worker(bin).await;
    let mut rt = runtime_over(&wm);
    build_and_regen(&mut rt).await;

    // An explicit save WITH a capture — so the runtime is holding a live preview
    // when the autosave runs, which is exactly when an inherited one would leak.
    let payload = rt.build_save_payload(
        save_meta(),
        SaveCaches {
            meshes: true,
            preview_png: Some(b"\x89PNG\r\n\x1a\n-capture".to_vec()),
        },
    );
    DocumentRuntime::write_payload(&explicit, &payload).expect("explicit write");
    rt.write_autosave(&auto, save_meta()).expect("autosave");
    wm.shutdown().await;

    let explicit_names = zip_entry_names(&explicit);
    assert!(
        explicit_names.iter().any(|n| n.starts_with(MESHES_DIR)),
        "the explicit save embeds meshes, got {explicit_names:?}"
    );
    assert!(
        explicit_names.iter().any(|n| n == PREVIEW_PATH),
        "the explicit save embeds the capture, got {explicit_names:?}"
    );

    let auto_names = zip_entry_names(&auto);
    let cache_entries: Vec<&String> = auto_names
        .iter()
        .filter(|n| {
            n.starts_with(MESHES_DIR) || n.starts_with("checkpoints/") || *n == PREVIEW_PATH
        })
        .collect();
    assert!(
        cache_entries.is_empty(),
        "an autosave container must carry NO cache section, got {cache_entries:?} in {auto_names:?}"
    );
}

/// A `meshes/` entry that is not valid MESH1 — a truncated write, a foreign
/// producer, bit rot the manifest hash happens to still match because the whole
/// container was re-signed — is skipped, never forwarded to the viewport. MESH1
/// travels verbatim (Invariant 5), so this validator is the only thing standing
/// between a corrupt blob and the GPU.
#[test]
fn non_mesh1_cache_entries_are_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("mixed.onecad");
    let doc = onecad_core::document::Document::new(DocumentId(Uuid::from_u128(0xD0C)));
    let good = BodyId(Uuid::from_u128(0xB0));
    let bad = BodyId(Uuid::from_u128(0xB1));

    let caches = ContainerCaches {
        meshes: vec![
            MeshCache {
                body: good,
                lod: "fine".into(),
                bytes: Arc::new(minimal_mesh1()),
            },
            MeshCache {
                body: bad,
                lod: "fine".into(),
                bytes: Arc::new(b"this is definitely not MESH1".to_vec()),
            },
        ],
        ..ContainerCaches::none()
    };
    ContainerWriter::save(&path, &doc, &caches, &save_meta()).expect("write");

    let engine: Arc<dyn GeometryEngine> = Arc::new(PendingBackend);
    let meshes: Arc<dyn MeshProvider> = Arc::new(PendingBackend);
    let solver: Arc<dyn SolverEngine> = Arc::new(PendingBackend);
    let mut rt = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");

    assert_eq!(
        rt.projection().geometry_source,
        "cached",
        "the ONE valid blob still counts as paintable geometry"
    );
    let served = tokio_block_on(rt.get_mesh(good, Lod::Fine, None));
    assert_eq!(
        served.as_deref().map(Vec::as_slice),
        Some(minimal_mesh1().as_slice()),
        "the valid blob is served verbatim"
    );
    assert!(
        tokio_block_on(rt.get_mesh(bad, Lod::Fine, None)).is_none(),
        "the non-MESH1 blob was skipped at open and is never served"
    );
}

/// The start screen's thumbnails come from each project's `preview.png`, read
/// WITHOUT opening the document (no parse, no migration, no cross-validation) — and
/// a project without one keeps `None` so the frontend's hatch placeholder renders.
#[test]
fn recents_thumbnails_come_from_the_container_preview() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = onecad_core::document::Document::new(DocumentId(Uuid::from_u128(0xD0C)));
    let png = b"\x89PNG\r\n\x1a\nfake-thumbnail-bytes".to_vec();

    let with_preview = tmp.path().join("with.onecad");
    ContainerWriter::save(
        &with_preview,
        &doc,
        &ContainerCaches {
            preview_png: Some(png.clone()),
            ..ContainerCaches::none()
        },
        &save_meta(),
    )
    .unwrap();

    let without = tmp.path().join("without.onecad");
    ContainerWriter::save(&without, &doc, &ContainerCaches::none(), &save_meta()).unwrap();

    let deleted = tmp.path().join("deleted.onecad");

    let entries: Vec<RecentProjectDto> = [&with_preview, &without, &deleted]
        .iter()
        .map(|p| RecentProjectDto {
            id: p.to_string_lossy().into_owned(),
            name: "proj".into(),
            path: p.to_string_lossy().into_owned(),
            modified_at: "2026-08-05T00:00:00Z".into(),
            thumbnail: None,
            has_recovery: false,
        })
        .collect();

    let filled = onecad_lib::recents::with_thumbnails(entries);
    let thumb = filled[0].thumbnail.as_deref().expect("preview → thumbnail");
    assert!(
        thumb.starts_with("data:image/png;base64,"),
        "the DTO carries a data URL, got {thumb}"
    );
    {
        use base64::Engine as _;
        let b64 = thumb.rsplit_once(',').unwrap().1;
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap(),
            png,
            "the data URL round-trips the exact PNG bytes"
        );
    }
    assert_eq!(
        filled[1].thumbnail, None,
        "no preview section ⇒ no thumbnail"
    );
    assert_eq!(filled[2].thumbnail, None, "missing file ⇒ no thumbnail");

    // The lightweight lane also refuses a tampered preview outright.
    rewrite_entries(&with_preview, |name, buf| {
        if name == PREVIEW_PATH {
            *buf = b"swapped".to_vec();
        }
    });
    assert_eq!(
        container::read_preview(&with_preview),
        None,
        "a tampered preview fails the manifest hash"
    );
}

/// Runs one future to completion on a scratch runtime (the two container-only tests
/// are `#[test]`, not `#[tokio::test]`, but `get_mesh` is `async`).
fn tokio_block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(fut)
}
