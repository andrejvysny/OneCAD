use super::*;

#[test]
fn capacity_advice_respects_mobile_headroom_without_allocating_or_retiring_live_work() {
    let authority = BudgetAuthority::new(onecad_render::RenderProfile::Mobile);
    let parked = authority.owner().unwrap();
    let active = authority.owner().unwrap();
    active.activate();
    let live = parked.reserve(576 << 20).unwrap();
    let before = authority.snapshot();
    assert!(!parked.has_capacity(1));
    assert!(active.has_capacity(PREVIEW_RESERVATION));
    assert!(!active.has_capacity(PREVIEW_RESERVATION + 1));
    let after = authority.snapshot();
    assert_eq!(after.used, before.used);
    assert_eq!(after.portions.len(), before.portions.len());
    assert_eq!(after.waiting, 0);
    parked.close();
    assert!(!parked.has_capacity(0));
    assert_eq!(authority.snapshot().used, 576 << 20);
    drop(live);
    assert_eq!(authority.snapshot().used, 0);
}

#[test]
fn split_reservation_releases_only_retired_allocation_owners() {
    let budget = Arc::new(HostBudget::default());
    for (whole, source_bytes) in [
        (DISPLAY_RESERVATION, DISPLAY_SOURCE_RESERVATION),
        (PREVIEW_RESERVATION, PREVIEW_SOURCE_RESERVATION),
    ] {
        let admitted = budget.reserve(whole).unwrap();
        let (source, preparation) = admitted.split(source_bytes).unwrap();
        assert_eq!(budget.snapshot().used, whole);
        let preparation = Arc::new(preparation);
        let canceled_worker = preparation.clone();
        drop(preparation);
        drop(source);
        assert_eq!(budget.snapshot().used, MAX_PREPARED_BYTES as usize);
        drop(canceled_worker);
        assert_eq!(budget.snapshot().used, 0);
    }
    assert!(
        budget
            .reserve(DISPLAY_RESERVATION)
            .unwrap()
            .split(DISPLAY_RESERVATION + 1)
            .is_err()
    );
    assert_eq!(budget.snapshot().used, 0);
}
#[test]
fn old_pending_and_building_payloads_share_one_reservation_limit() {
    let budget = Arc::new(HostBudget::default());
    let current = budget.reserve(DISPLAY_RESERVATION).unwrap();
    let pending = budget.reserve(DISPLAY_RESERVATION).unwrap();
    let building = budget.reserve(DISPLAY_RESERVATION).unwrap();
    let preview = budget.reserve(PREVIEW_RESERVATION).unwrap();
    assert!(budget.reserve(PREVIEW_RESERVATION).is_err());
    drop(pending);
    drop(building);
    assert!(budget.reserve(PREVIEW_RESERVATION).is_ok());
    drop(current);
    drop(preview);
    assert_eq!(budget.snapshot().used, 0);
}
#[test]
fn canceled_contour_job_keeps_its_host_reservation_until_execution_retires() {
    let budget = Arc::new(HostBudget::default());
    let _current = budget.reserve(DISPLAY_RESERVATION).unwrap();
    let _pending = budget.reserve(DISPLAY_RESERVATION).unwrap();
    let _building = budget.reserve(DISPLAY_RESERVATION).unwrap();
    let _preview = budget.reserve(PREVIEW_RESERVATION).unwrap();
    let host = Arc::new(budget.reserve(MAX_PREPARED_BYTES as usize).unwrap());
    let executing_job = host.clone();
    drop(host);
    assert!(budget.reserve(MAX_PREPARED_BYTES as usize).is_err());
    drop(executing_job);
    assert!(budget.reserve(MAX_PREPARED_BYTES as usize).is_ok());
}
#[test]
fn nema_checked_scene_fits_display_reservation_with_bvh_storage() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render/nema17");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "mesh1")
        })
        .collect();
    paths.sort();
    let bodies = paths
        .into_iter()
        .map(|path| {
            onecad_scene::SceneBody::new(
                onecad_scene::BodyId::new(),
                1,
                Arc::from(std::fs::read(path).unwrap()),
                onecad_scene::MeshLimits::default(),
            )
            .unwrap()
        })
        .collect();
    let scene = onecad_scene::SceneSnapshot::new(
        onecad_scene::SceneIdentity {
            session: 1,
            scene: 1,
        },
        bodies,
        onecad_scene::SceneLimits {
            max_resident_bytes: MAX_DISPLAY_SCENE_BYTES,
            max_bodies: MAX_BODIES,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(scene.bodies().len(), 67);
    assert!(
        scene.resident_bytes() + MAX_PREPARED_BYTES as usize + MAX_PROFILE_BYTES + DISPLAY_METADATA
            <= DISPLAY_RESERVATION
    );
}
