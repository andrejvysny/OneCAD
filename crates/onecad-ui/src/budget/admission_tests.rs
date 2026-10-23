use super::*;
fn desktop() -> Arc<BudgetAuthority> {
    BudgetAuthority::new(onecad_render::RenderProfile::Desktop)
}
fn mobile() -> Arc<BudgetAuthority> {
    BudgetAuthority::new(onecad_render::RenderProfile::Mobile)
}
async fn queued(authority: &BudgetAuthority, count: usize) {
    for _ in 0..100 {
        if authority.snapshot().waiting == count {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!(
        "expected {count} native admission tickets, got {}",
        authority.snapshot().waiting
    );
}
fn waiting(
    owner: Arc<HostBudget>,
    bytes: usize,
    kind: Kind,
) -> tokio::task::JoinHandle<Result<Lease, String>> {
    tokio::spawn(async move { owner.admit_while(bytes, kind, || true).await })
}

#[test]
fn three_document_views_compete_and_closing_does_not_release_live_work() {
    let authority = desktop();
    let first = authority.owner().unwrap();
    let second = authority.owner().unwrap();
    let third = authority.owner().unwrap();
    assert_ne!(first.owner_id(), second.owner_id());
    let a = first.reserve(DISPLAY_RESERVATION).unwrap();
    let b = second.reserve(DISPLAY_RESERVATION).unwrap();
    let c = third.reserve(DISPLAY_RESERVATION).unwrap();
    let preview = first.reserve(PREVIEW_RESERVATION).unwrap();
    assert_eq!(authority.snapshot().used, (3 * 160 + 192) << 20);
    assert!(second.reserve(PREVIEW_RESERVATION).is_err());
    second.close();
    drop(second);
    assert!(third.reserve(PREVIEW_RESERVATION).is_err());
    drop(b);
    let incoming = third.reserve(PREVIEW_RESERVATION).unwrap();
    assert_eq!(authority.snapshot().owners.len(), 2);
    drop((a, c, preview, incoming));
    assert_eq!(authority.snapshot().used, 0);
}

#[test]
fn split_portions_preserve_admission_identity_owner_kind_and_total_once() {
    let authority = desktop();
    let owner = authority.owner().unwrap();
    let (source, preparation) = owner
        .reserve_kind(DISPLAY_RESERVATION, Kind::Display)
        .unwrap()
        .split_kinds(
            DISPLAY_SOURCE_RESERVATION,
            Kind::DisplaySource,
            Kind::Preparation,
        )
        .unwrap();
    let source = Arc::new(source);
    let copied = source.clone();
    let snapshot = authority.snapshot();
    assert_eq!(snapshot.portions.len(), 2);
    assert_ne!(snapshot.portions[0].id, snapshot.portions[1].id);
    assert_eq!(
        snapshot.portions[0].admission,
        snapshot.portions[1].admission
    );
    assert_eq!(snapshot.used, DISPLAY_RESERVATION);
    assert_eq!(
        snapshot.owners[&owner.owner_id()].kinds[&Kind::DisplaySource],
        DISPLAY_SOURCE_RESERVATION
    );
    drop((source, preparation));
    assert_eq!(authority.snapshot().used, DISPLAY_SOURCE_RESERVATION);
    drop(copied);
    assert_eq!(authority.snapshot().used, 0);
}

#[test]
fn mobile_active_headroom_survives_inactive_admission_and_owner_changes() {
    let authority = mobile();
    let background = authority.owner().unwrap();
    let active = authority.owner().unwrap();
    active.activate();
    let parked = background.reserve(576 << 20).unwrap();
    assert!(background.reserve(1).is_err());
    let work = active.reserve(PREVIEW_RESERVATION).unwrap();
    let snapshot = authority.snapshot();
    assert_eq!(snapshot.used, HOST_BYTES);
    assert_eq!(snapshot.inactive, 576 << 20);
    assert_eq!(snapshot.active, Some(active.owner_id()));
    background.activate();
    assert_eq!(authority.snapshot().used, HOST_BYTES); // Existing leases are not revoked.
    drop(work);
    assert!(active.reserve(PREVIEW_RESERVATION).is_ok());
    drop(parked);
}

#[tokio::test]
async fn oldest_eligible_ticket_wins_and_sync_work_cannot_jump_it() {
    let authority = desktop();
    let first = authority.owner().unwrap();
    let second = authority.owner().unwrap();
    let retained = first.reserve(HOST_BYTES - DISPLAY_RESERVATION).unwrap();
    let retiring = first.reserve(DISPLAY_RESERVATION).unwrap();
    let a = waiting(first.clone(), DISPLAY_RESERVATION, Kind::Display);
    queued(&authority, 1).await;
    let b = waiting(second.clone(), 64 << 20, Kind::Preparation);
    queued(&authority, 2).await;
    drop(retiring);
    assert!(second.reserve_kind(64 << 20, Kind::Contour).is_err());
    let a = a.await.unwrap().unwrap();
    assert_eq!(authority.snapshot().waiting, 1);
    assert!(!b.is_finished());
    drop(a);
    let b = b.await.unwrap().unwrap();
    assert_eq!(authority.snapshot().waiting, 0);
    drop((b, retained));
}

#[tokio::test]
async fn ineligible_inactive_ticket_does_not_block_active_work() {
    let authority = mobile();
    let background = authority.owner().unwrap();
    let active = authority.owner().unwrap();
    active.activate();
    let parked = background.reserve(576 << 20).unwrap();
    let inactive = waiting(background, DISPLAY_RESERVATION, Kind::Display);
    queued(&authority, 1).await;
    let admitted = waiting(active, PREVIEW_RESERVATION, Kind::Preview)
        .await
        .unwrap()
        .unwrap();
    assert!(!inactive.is_finished());
    assert_eq!(authority.snapshot().waiting, 1);
    inactive.abort();
    assert!(inactive.await.unwrap_err().is_cancelled());
    assert_eq!(authority.snapshot().waiting, 0);
    assert_eq!(authority.snapshot().used, HOST_BYTES);
    drop((admitted, parked));
}

#[tokio::test]
async fn queued_cancellation_and_owner_close_leave_live_leases_charged() {
    let authority = desktop();
    let first = authority.owner().unwrap();
    let second = authority.owner().unwrap();
    let live = first.reserve(HOST_BYTES).unwrap();
    let canceled = waiting(second.clone(), DISPLAY_RESERVATION, Kind::Display);
    queued(&authority, 1).await;
    assert!(
        second
            .admit_while(1, Kind::Display, || true)
            .await
            .unwrap_err()
            .contains("already waits")
    );
    canceled.abort();
    assert!(canceled.await.unwrap_err().is_cancelled());
    assert_eq!(authority.snapshot().waiting, 0);
    let closing = waiting(second.clone(), PREVIEW_RESERVATION, Kind::Preview);
    queued(&authority, 1).await;
    second.close();
    assert!(closing.await.unwrap().unwrap_err().contains("cancelled"));
    assert_eq!(authority.snapshot().used, HOST_BYTES);
    drop(live);
    assert_eq!(authority.snapshot().used, 0);
}

#[tokio::test]
async fn generation_change_wakes_and_removes_only_stale_waiting_ticket() {
    let authority = desktop();
    let first = authority.owner().unwrap();
    let second = authority.owner().unwrap();
    let live = first.reserve(HOST_BYTES).unwrap();
    let current = Arc::new(AtomicBool::new(true));
    let captured = current.clone();
    let view = second.clone();
    let stale = tokio::spawn(async move {
        view.admit_while(DISPLAY_RESERVATION, Kind::Display, || {
            captured.load(Ordering::Acquire)
        })
        .await
    });
    queued(&authority, 1).await;
    current.store(false, Ordering::Release);
    second.wake();
    assert!(stale.await.unwrap().unwrap_err().contains("cancelled"));
    assert_eq!(authority.snapshot().waiting, 0);
    assert_eq!(authority.snapshot().used, HOST_BYTES);
    drop(live);
}

#[tokio::test]
async fn admission_expires_within_three_second_budget_and_removes_ticket() {
    let authority = desktop();
    let first = authority.owner().unwrap();
    let second = authority.owner().unwrap();
    let _live = first.reserve(HOST_BYTES).unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        second.admit_while(DISPLAY_RESERVATION, Kind::Display, || true),
    )
    .await
    .unwrap();
    assert!(
        result
            .unwrap_err()
            .contains("display work was not admitted")
    );
    assert_eq!(authority.snapshot().waiting, 0);
}

#[tokio::test]
async fn admission_queue_bound_refuses_overflow_without_dropping_existing_tickets() {
    let authority = desktop();
    let holder = authority.owner().unwrap();
    let live = holder.reserve(HOST_BYTES).unwrap();
    let mut pending = Vec::new();
    for count in 1..=MAX_WAITERS {
        pending.push(waiting(authority.owner().unwrap(), 1, Kind::Display));
        queued(&authority, count).await;
    }
    let overflow = authority
        .owner()
        .unwrap()
        .admit_while(1, Kind::Display, || true)
        .await;
    assert!(overflow.unwrap_err().contains("queue is full"));
    assert_eq!(authority.snapshot().waiting, MAX_WAITERS);
    for task in pending {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    assert_eq!(authority.snapshot().waiting, 0);
    assert_eq!(authority.snapshot().used, HOST_BYTES);
    drop(live);
}

#[tokio::test]
async fn poisoned_authority_refuses_new_work_but_retires_known_allocations() {
    let authority = desktop();
    let owner = authority.owner().unwrap();
    let live = owner.reserve(HOST_BYTES).unwrap();
    let waiting = waiting(owner.clone(), DISPLAY_RESERVATION, Kind::Display);
    queued(&authority, 1).await;
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = authority.state.lock().unwrap();
        panic!("injected native budget poison");
    }));
    assert!(authority.snapshot().degraded);
    assert!(owner.reserve(1).is_err());
    assert!(authority.owner().is_err());
    assert!(
        waiting
            .await
            .unwrap()
            .unwrap_err()
            .contains("ledger unavailable")
    );
    assert_eq!(authority.snapshot().used, HOST_BYTES);
    drop(live);
    assert_eq!(authority.snapshot().used, 0);
}

#[test]
fn exhausted_identity_refuses_split_and_releases_only_its_original_portion() {
    let authority = desktop();
    let owner = authority.owner().unwrap();
    let live = owner.reserve(DISPLAY_RESERVATION).unwrap();
    authority.state().next = u64::MAX;
    assert!(
        live.split(DISPLAY_SOURCE_RESERVATION)
            .unwrap_err()
            .contains("identity exhausted")
    );
    let snapshot = authority.snapshot();
    assert!(snapshot.degraded);
    assert_eq!(snapshot.used, 0);
    assert!(snapshot.portions.is_empty());
}
