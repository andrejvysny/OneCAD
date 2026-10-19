use super::*;
use crate::{MemoryEstimate, RenderLimits, SectionContourWorker};
use onecad_scene::{BodyId, MeshLimits, SceneBody, SceneLimits};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn budget() -> PreparationBudget {
    PreparationBudget {
        limits: RenderLimits::default(),
        resident: MemoryEstimate::default(),
        max_buffer: u64::MAX,
    }
}
fn scene() -> SceneSnapshot {
    let bytes = include_bytes!(
        "../../../fixtures/render/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
    );
    let body = SceneBody::new(
        BodyId::new(),
        1,
        Arc::from(bytes.as_slice()),
        MeshLimits::default(),
    )
    .unwrap();
    SceneSnapshot::new(
        SceneIdentity {
            session: 3,
            scene: 2,
        },
        vec![body],
        SceneLimits::default(),
    )
    .unwrap()
}
fn cut(scene: &SceneSnapshot) -> [f32; 4] {
    let bounds = scene.bounds().unwrap();
    [0.0, 0.0, 1.0, ((bounds.min.z + bounds.max.z) * 0.5) as f32]
}
fn collect(points: [[f32; 3]; 3], plane: [f32; 4]) -> Vec<Candidate> {
    let mut out = Vec::new();
    triangle(points, plane, |c| out.push(c));
    out
}

#[test]
fn triangle_cuts_preserve_plane_and_reversed_order() {
    let points = [[-2.0, -1.0, -1.0], [2.0, 0.0, 1.0], [-1.0, 2.0, 1.0]];
    let plane = [0.0, 0.0, 1.0, 0.0];
    let result = collect(points, plane);
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].segment,
        collect([points[2], points[1], points[0]], plane)[0].segment
    );
    assert_eq!(result[0].segment[2], 0.0);
    assert_eq!(result[0].segment[5], 0.0);
    let oblique = collect(points, [1.0, 2.0, 3.0, 0.0]);
    assert_eq!(oblique.len(), 1);
    for point in oblique[0].segment.as_chunks::<3>().0 {
        assert!(distance(*point, [1.0, 2.0, 3.0, 0.0]).abs() < 1e-6);
    }
    assert!(collect([[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]], plane).is_empty());
    assert!(collect([[0.0; 3], [0.0; 3], [1.0; 3]], plane).is_empty());
    assert_eq!(
        collect([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 1.0]], plane).len(),
        1
    );
    assert_eq!(
        collect([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], plane).len(),
        3
    );
}

#[test]
fn coplanar_patch_diagonals_cancel_and_nested_bodies_keep_boundaries() {
    let plane = [0.0, 0.0, 1.0, 0.0];
    let square = |scale: f32, body: u32| {
        let a = [-scale, -scale, 0.0];
        let b = [scale, -scale, 0.0];
        let c = [scale, scale, 0.0];
        let d = [-scale, scale, 0.0];
        let mut candidates = collect([a, b, c], plane);
        candidates.extend(collect([a, c, d], plane));
        for candidate in &mut candidates {
            candidate.body = body;
        }
        candidates
    };
    assert_eq!(resolve(square(2.0, 0)).unwrap().len(), 4);
    let mut nested = square(2.0, 0);
    nested.extend(square(1.0, 1));
    assert_eq!(resolve(nested).unwrap().len(), 8);
    let mut coincident = square(2.0, 0);
    coincident.extend(square(2.0, 1));
    assert_eq!(resolve(coincident).unwrap().len(), 4);
}

#[test]
fn retained_request_releases_source_lease_after_cancel() {
    struct Lease(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Release);
        }
    }
    let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hold = Arc::new(Lease(released.clone()));
    let scene = Arc::new(scene());
    let worker = SectionContourWorker::new().unwrap();
    worker
        .submit_retained(scene.clone(), None, budget(), cut(&scene), hold)
        .unwrap();
    worker.cancel();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !released.load(std::sync::atomic::Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "source lease survived canceled request"
        );
        std::thread::yield_now();
    }
}

#[test]
fn checked_geometry_contours_are_finite_deterministic_and_bounded() {
    let scene = scene();
    let plane = cut(&scene);
    let prepared = PreparedSectionContours::new(&scene, budget(), plane).unwrap();
    assert!(!prepared.segments().is_empty());
    assert!(prepared
        .segments()
        .iter()
        .all(|s| s.iter().all(|v| v.is_finite()) && s[2] == plane[3] && s[5] == plane[3]));
    let again = PreparedSectionContours::new(&scene, budget(), plane).unwrap();
    assert_eq!(prepared.segments(), again.segments());
    let disabled = PreparedSectionContours::new(&scene, budget(), [0.0; 4]).unwrap();
    assert!(disabled.segments().is_empty());
    let tiny = PreparationBudget {
        limits: RenderLimits {
            max_cpu_prepared_bytes: 1,
            ..Default::default()
        },
        ..budget()
    };
    assert!(matches!(
        PreparedSectionContours::new(&scene, tiny, plane),
        Err(RenderError::Budget("section CPU preparation"))
    ));
    assert!(PreparedSectionContours::new(&scene, budget(), [f32::NAN; 4]).is_err());
    assert!(prepared
        .with_preview(PreviewIdentity {
            base: SceneIdentity {
                session: 9,
                scene: 9
            },
            generation: 1
        })
        .is_err());
}

#[test]
fn canceled_preparation_and_latest_only_worker_fence_stale_results() {
    let scene = Arc::new(scene());
    let plane = cut(&scene);
    assert!(matches!(
        PreparedSectionContours::prepare(&scene, budget(), plane, &|| false),
        Err(RenderError::CaptureMismatch)
    ));
    let worker = SectionContourWorker::new().unwrap();
    let stale = worker.submit(scene.clone(), None, budget(), plane).unwrap();
    let mut latest_plane = plane;
    latest_plane[3] += 0.1;
    let preview = PreviewIdentity {
        base: scene.identity(),
        generation: 7,
    };
    let latest = worker
        .submit(scene, Some(preview), budget(), latest_plane)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(worker.poll(stale).is_none());
        if let Some(result) = worker.poll(latest) {
            let prepared = result.unwrap();
            assert_eq!(prepared.key(), latest);
            assert_eq!(prepared.key().preview, Some(preview));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker never completed latest request"
        );
        std::thread::yield_now();
    }
    worker.cancel();
    assert!(worker.poll(latest).is_none());
}

#[test]
fn nema_sections_use_exact_count_with_four_mebibyte_cpu_reservation() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render/nema17");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "mesh1"))
        .collect();
    paths.sort();
    let bodies = paths
        .into_iter()
        .map(|path| {
            SceneBody::new(
                BodyId::new(),
                1,
                Arc::from(std::fs::read(path).unwrap()),
                MeshLimits::default(),
            )
            .unwrap()
        })
        .collect();
    let scene = Arc::new(
        SceneSnapshot::new(
            SceneIdentity {
                session: 44,
                scene: 1,
            },
            bodies,
            SceneLimits::default(),
        )
        .unwrap(),
    );
    assert_eq!(scene.bodies().len(), 67);
    let budget = PreparationBudget {
        limits: RenderLimits {
            max_cpu_prepared_bytes: 4 << 20,
            ..Default::default()
        },
        ..budget()
    };
    let worker = SectionContourWorker::new().unwrap();
    let start = Instant::now();
    let key = worker
        .submit(scene.clone(), None, budget, cut(&scene))
        .unwrap();
    let deadline = start + Duration::from_secs(15);
    loop {
        if let Some(result) = worker.poll(key) {
            let prepared = result.unwrap();
            assert!(!prepared.segments().is_empty());
            assert!(prepared.cpu_bytes() < 4 << 20);
            eprintln!(
                "NEMA contours: {} segments, {} CPU bytes, {:?}",
                prepared.segments().len(),
                prepared.cpu_bytes(),
                start.elapsed()
            );
            break;
        }
        assert!(Instant::now() < deadline, "NEMA contour worker timed out");
        std::thread::yield_now();
    }
}
