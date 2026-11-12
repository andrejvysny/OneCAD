---
name: repo-first-ready-stream-budgets
description: ONECAD-536 first-Ready replay state model + test seams (gated sidecar, FakeBackend disconnect/hold) and the mesh-stream request budgets (mesh_stream.rs, abort_connection, paced-peer tests)
metadata:
  type: project
---

Landed 2026-11-11 on fix/hardening-runtime-refs (uncommitted at hand-off; orchestrator commits).

**Why:** a regen that fails `NotConnected` publishes and rolls back nothing; the restart hook only
fires after a DEATH, so a cold first handshake left accepted history unregenerated. Separately,
only a per-stream byte cap bounded mesh chunk streams.

**How to apply:**

- Supervisor fires `first_ready_hook` on the Ready no death preceded (`pending_restart_epoch` None),
  restart hook on every other Ready — mutually exclusive by construction.
- Owed state lives in `document_runtime/ready_replay.rs`: `unreached` fence (set by a NotConnected
  finish with unchanged fencing, cleared by `begin_regen`/publish/restart) + `announced` epoch +
  published-history hash (`export_history_hash()` taken AFTER publish writebacks). Claim consumes.
- Race (Ready announced while the attempt was in flight): `finish_regen` downgrades the
  NotConnected to `Superseded` + `stranded_regen` → driver retry; UI awaiter skips superseded.
  `announced` is the FENCE at claim time, cleared by publish/restart/spend (stale-latch fix).
- Owed (2026-11-12): `unreached.rev >= regen_requested_at` (not `== current fence`), so
  rename/visibility/colour/AddSketch in the gap keep the replay owed. `claim` takes `requested`.
  `regen_requested_at` is set by `note_regen_hint`/`note_revert_request`, NOT the dirty
  floor: a draft appended past the rollback cursor dirties with `RegenHint::None`.
  Rule: note iff the caller enqueues. `cancel_sketch` NoNetStep creation revert notes too, but
  its `regen` is always None (discard refuses a sketch any record produces/consumes).
- Waiter predicate `initial_regen_needed()`: !attempted || (head differs && !in_flight &&
  !claimed && unreached None). `in_flight` set in `begin`, cleared in `finish_regen` and the
  two `begin_regen` noop returns. Never enqueue while in flight: a cancelled fresh-feature
  commit makes `provisional.rs::should_rollback` roll it back.
- Seams: `AppState::with_worker_spawner`, `WorkerManager::spawn_gated` (launch barrier + crash
  Notify, real sidecar), FakeBackend `disconnected` + `plan_hold`, `state::replay_owed_geometry`.
- Streams: `worker/manager/mesh_stream.rs` (`MeshStreams` folds, `CancelUnfinished` guard);
  `Shared::mesh_budget` set in tests via `Arc::get_mut(&mut shared(1))`; gauge
  `Shared::mesh_stream_bytes`. Violation ⇒ `client.abort_connection_for(c.id)`.
- Teardown cause (2026-11-12): `ProtocolClient::teardown()` = `PeerLost` | `Violation{culprit}`;
  `violated()` wakes `run_until_death` → `Death::ProtocolViolation` (prompt restart, no ping).
  `spare_bystander` spares a plan only if the culprit was outstanding OR among the last 64
  finished requests (`Pending::finished`, filled on Resp; a late chunk names a finished id) and
  not ExecutePlan/PreviewPlan (`violation_culprit_verb`, set in `fail_all`). Deterministic supervisor rig: `manager/teardown_tests.rs`
  (`ScriptedHost` over duplex, launch permits, plan_permit hold, ping 1 h).
- Client now tears down on a chunk whose request has no pending waiter (after its terminal).
- Paced-peer tests ack each data chunk by reading the client's `credit` frame.
