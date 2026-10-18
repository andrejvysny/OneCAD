use crate::{
    document_runtime::{adopt_isolated, DocumentRuntime, RegenReport},
    dto::DocumentProjection,
};
use onecad_core::regen::{CancelToken, Outcome, RegenDirective, RegenRequest};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::{watch, Mutex};
/// The boxed future the regen driver hands the scheduler (the driver closure is a
/// distinct type per call, so it is type-erased for the scheduler's `select!`).
pub type BoxFut = Pin<Box<dyn Future<Output = Outcome> + Send>>;

/// A post-regen completion sink: called once per finished regen with the
/// [`RegenReport`] + refreshed [`DocumentProjection`]. The production wrapper
/// forwards typed events through [`crate::api::emit_regen_events`]; tests inject
/// a channel sink to assert the real scheduler/driver wiring without a UI.
pub type RegenEmitter = Arc<dyn Fn(&RegenReport, &DocumentProjection) + Send + Sync>;

/// A regen START sink: called once per job that actually got a prepared plan,
/// before the unlocked drive phase. The production wrapper emits
/// [`crate::events::REGEN_STARTED`] so the frontend can show a busy indicator; every
/// other caller passes the no-op ([`regen_driver_with_emitter`]).
pub type RegenStartEmitter = Arc<dyn Fn(&str) + Send + Sync>;

/// A post-PUBLISH sink, awaited after the driver's own post-publish work (the
/// DI-4 element re-bind) and therefore after the resolution ladder has had its
/// chance to re-bind persisted element ids.
///
/// Ordering is the whole reason this is a hook rather than a spawn inside the
/// completion sink: WP-P's projection staleness check re-projects from persisted
/// `ElementId`s, and probing before the re-bind would report a freshly-republished
/// projection as stale purely because its source had not been re-bound yet.
///
/// It takes the job's [`CancelToken`] and is SKIPPED outright when that has
/// already fired: a superseding regen is queued, its own publish will re-probe,
/// and paying for this one's worker round-trips would only delay it. Same
/// best-effort gate as [`mint_rollback_checkpoint`].
///
/// Every test harness passes the no-op ([`regen_driver_with_emitter`]).
pub type RegenPostPublish =
    Arc<dyn Fn(CancelToken) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Builds the app-layer regen driver over an explicit completion [`RegenEmitter`]
/// (plan "app layer runs the executor"), with NO start sink — the shape every
/// test harness wants. See [`regen_driver_with_started`] for the full seam.
///
/// **Fencing goes live (R-WP11):** the driver holds the single-writer lock only
/// for the two short phases that mutate the document — phase 1
/// ([`begin_regen`](DocumentRuntime::begin_regen): compile the plan + clone a
/// scratch session) and phase 3 ([`finish_regen`](DocumentRuntime::finish_regen):
/// commit or supersede). The slow worker IO (phase 2,
/// [`PreparedRegen::drive`](crate::document_runtime::PreparedRegen::drive)) runs
/// with the lock **released**, so an edit can land during it, advance the fencing
/// tokens, and supersede the stale prepare via the executor's revision gate.
/// Debounce/coalesce/preview-priority stay in the [`onecad_core::regen::RegenScheduler`] (policy only).
pub fn regen_driver_with_emitter(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    emit: RegenEmitter,
    autosave_tick: Arc<watch::Sender<u64>>,
) -> impl Fn(RegenDirective) -> BoxFut + Send + Sync + 'static {
    regen_driver_with_started(
        runtime,
        emit,
        Arc::new(|_| {}),
        Arc::new(|_| Box::pin(async {})),
        autosave_tick,
    )
}

/// [`regen_driver_with_emitter`] plus a [`RegenStartEmitter`] fired right after
/// [`begin_regen`](DocumentRuntime::begin_regen) hands back a prepared plan — the
/// ONE point where "a regen is really going to run" is known, and the lock is
/// already released. A no-op regen never reaches it (it has no work to be busy
/// about), which is why the pairing with the completion sink is one-directional.
pub fn regen_driver_with_started(
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    emit: RegenEmitter,
    on_started: RegenStartEmitter,
    post_publish: RegenPostPublish,
    autosave_tick: Arc<watch::Sender<u64>>,
) -> impl Fn(RegenDirective) -> BoxFut + Send + Sync + 'static {
    move |directive: RegenDirective| {
        let runtime = runtime.clone();
        let emit = emit.clone();
        let on_started = on_started.clone();
        let post_publish = post_publish.clone();
        let autosave_tick = autosave_tick.clone();
        Box::pin(async move {
            // Phase 1 (locked): compile the plan + clone the scratch session. An EMPTY
            // plan (a no-op-producing request — a rolled-back append, an already-current
            // timeline) still emits exactly ONE NoOp completion so the frontend awaiter's
            // `regen-finished{noop}` anti-hang terminal fires in production (finding 4):
            // returning silently here would strand the awaiter until its 8 s fallback.
            let prepared = {
                let mut guard = runtime.lock().await;
                let Some(rt) = guard.as_mut() else {
                    return Outcome::NoOp; // document closed while the job was queued.
                };
                if directive
                    .runtime_session
                    .as_deref()
                    .is_some_and(|expected| !rt.matches_runtime_session(expected))
                {
                    return Outcome::NoOp;
                }
                match rt.begin_regen(directive.request) {
                    Some(prepared) => prepared,
                    None => {
                        let report = rt.noop_report();
                        let projection = rt.projection();
                        drop(guard);
                        emit(&report, &projection);
                        return Outcome::NoOp;
                    }
                }
            };
            // The job has real work: announce it BEFORE the (slow) drive phase, so
            // the busy indicator is up for the whole time the worker is out.
            on_started(&prepared.runtime_session());
            // Phase 2 (UNLOCKED): drive the worker; concurrent edits may supersede.
            let cancel = directive.cancel.clone();
            let mut driven = prepared.drive(directive.cancel).await;
            // Phase 2.5 (locked) + 2.6 (UNLOCKED), WP-D1: a regen that HALTED at some
            // record gets ONE further from-0 pass that excludes that record's
            // unprovable closure, so a broken feature cannot wipe bodies outside it.
            // Only the second result is finished — the truncated one is never
            // published, so the user never sees it flash and no awaiter settles twice.
            let isolation = {
                let mut guard = runtime.lock().await;
                let Some(rt) = guard.as_mut() else {
                    return Outcome::NoOp; // document closed during the worker IO.
                };
                rt.begin_isolation_regen(&driven)
            };
            if let Some(prepared) = isolation {
                // Adopt the isolation pass ONLY if it published: a worker that dies
                // or is cancelled inside its window must cost the isolation benefit,
                // never the halted pass's own publish (`adopt_isolated`).
                driven = adopt_isolated(driven, prepared.drive(cancel.clone()).await);
            }
            // Phase 3 (locked): commit iff still current, then emit events.
            let (report, projection) = {
                let mut guard = runtime.lock().await;
                let Some(rt) = guard.as_mut() else {
                    return Outcome::NoOp; // document closed during the worker IO.
                };
                let report = rt.finish_regen(driven);
                let projection = rt.projection();
                (report, projection)
            };
            emit(&report, &projection);
            // A published regen produced new geometry outputs worth autosaving
            // (the debounce coalesces the edit-tick + this publish-tick).
            if report.published() {
                autosave_tick.send_modify(|v| *v = v.wrapping_add(1));
                // DI-4: a freshly opened document's persisted ElementIds are not in
                // the worker's partition — it is minted on demand and died with the
                // last process — so an authored face colour has nothing to paint.
                // This is the first moment a head snapshot exists to resolve
                // against. It is a no-op once the ids are bound, and deliberately
                // off the LOCKED phases: it does worker round trips of its own and
                // must never sit in the edit path.
                let rebind = {
                    let mut guard = runtime.lock().await;
                    guard.as_mut().and_then(|rt| {
                        rt.has_unbound_persisted_elements()
                            .then(|| rt.prepare_persisted_rebind())
                            .flatten()
                    })
                };
                if let Some(ticket) = rebind {
                    let result = ticket.drive().await;
                    let (bound, unresolved) = {
                        let mut guard = runtime.lock().await;
                        guard
                            .as_mut()
                            .map(|rt| rt.adopt_persisted_rebind(result))
                            .unwrap_or_default()
                    };
                    tracing::debug!(
                        target: "onecad_lib::regen",
                        bound,
                        unresolved,
                        "rebind: persisted element ids re-bound after publish"
                    );
                }
                // AFTER the re-bind, and with the lock released: the WP-P
                // projection staleness pass. It is the last thing a publish owes
                // and the first that must not hold anything up — so a job that
                // has already been superseded skips it entirely and lets the
                // queued regen's own publish re-probe (the `mint_rollback_
                // checkpoint` gate, for the same reason).
                if !cancel.is_cancelled() {
                    post_publish(cancel.clone()).await;
                }
            }
            // HISTORY-HARDEN H8: a published ROLLBACK is the one completion worth a
            // free checkpoint (see `mint_rollback_checkpoint`).
            if matches!(directive.request, RegenRequest::ToStep(_))
                && report.published()
                && !cancel.is_cancelled()
            {
                mint_rollback_checkpoint(&runtime).await;
            }
            report.outcome
        })
    }
}

/// Best-effort checkpoint mint at the head a just-published **rollback** settled on
/// (HISTORY-HARDEN H8).
///
/// ## Why here, and what it actually accelerates
///
/// It does NOT speed up repeat rollbacks — a `ToStep(k)` preview picks a checkpoint
/// at `≤ k − 1`, so a checkpoint AT `k` is invisible to it. What it accelerates is the
/// **next step of the rollback workflow**: rolling back to cursor `c` and then
/// inserting a feature at the cursor dirties `[c, len)`, which schedules
/// `ToEnd { from: c }`, whose checkpoint ceiling is exactly `c − 1 == k` — the
/// checkpoint minted here. The roll-back → insert → roll-forward loop is the whole
/// point of the rollback bar, and it used to replay from 0 on every iteration because
/// the only checkpoint mint in the product was on explicit save.
///
/// **Strictly best-effort (Invariant 7).** Skipped when another request is already
/// queued (the cancel token fired) so an interactive scrub of the rollback bar does not
/// pay a `SaveCheckpoint` per row; skipped when
/// [`prepare_checkpoint`](DocumentRuntime::prepare_checkpoint) refuses; and dropped by
/// [`adopt_checkpoint`](DocumentRuntime::adopt_checkpoint)'s fencing recheck if an edit
/// or a worker restart landed during the round-trip. A failed mint costs a replay,
/// never correctness.
///
/// The worker round-trip runs **unlocked** (the `save_document` five-phase pattern), so
/// the mint never stalls an edit; both re-locks re-check the document identity because
/// a close/open can interleave.
async fn mint_rollback_checkpoint(runtime: &Arc<Mutex<Option<DocumentRuntime>>>) {
    let (ticket, engine, document_id) = {
        let guard = runtime.lock().await;
        let Some(rt) = guard.as_ref() else {
            return;
        };
        let Some(ticket) = rt.prepare_checkpoint() else {
            return;
        };
        (ticket, rt.engine_arc(), rt.document_uuid())
    };
    let Ok(artifacts) = engine.save_checkpoint(ticket.step()).await else {
        return;
    };
    let mut guard = runtime.lock().await;
    if let Some(rt) = guard
        .as_mut()
        .filter(|rt| rt.document_uuid() == document_id)
    {
        rt.adopt_checkpoint(ticket, artifacts);
    }
}
