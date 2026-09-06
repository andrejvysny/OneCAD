---
name: repo-supervisor-and-stub-lanes
description: Fan-out and gotchas when touching SupervisorConfig, the GeometryEngine trait, the export seam, or the onecad-worker-stub lanes
metadata:
  type: project
---

Widening `SupervisorConfig` costs **7** struct-literal sites (no `..Default`):
`manager.rs` (`production` + the unit-test `shared()`), and
`tests/{worker_chaos,solver_stub,worker_lifetime,worker_connect_race,worker_epoch,worker_stderr_capture}.rs`.
Widening `WorkerHead` costs 4 (`wire::parse_open_session`, `parse_worker_head`,
`src/document_runtime/tests.rs` ×2, `crates/onecad-core/tests/support/mod.rs` ×2).

**Why:** the same exhaustive-construction rule as the params structs — a new
supervision knob must not be silently defaulted into a drill that then proves
nothing.

**How to apply:** `cargo check --workspace --all-targets` enumerates every site;
give test configs the PRODUCTION value of a new deadline unless the drill is
about it, so a suite never dies of something it is not testing.

Related mechanics:

- **`GeometryEngine` is implemented by 5+ types** (`WorkerManager`,
  `AdoptingEngine`, `PendingBackend`, and fakes in `src/document_runtime/tests.rs`
  + `crates/onecad-core/tests/support/mod.rs`). A method that only the transport
  can implement belongs there with a **default** body; but `AdoptingEngine` must
  then FORWARD it explicitly — it is what the regen path actually holds, so
  inheriting the default silently disables the behaviour.
- `onecad-core`'s tokio has features `sync, time, macros` only — **no `rt`**, so
  nothing in that crate can `tokio::spawn`. Work that must be detached from a
  `Drop` has to cross a synchronous trait seam into the app crate.
- `EngineError` has **no** `StalePreview` variant: `STALE_PREVIEW` folds into
  `OpFailed { code: OpFailureCode::StalePreview }`, which `error.rs` renders as
  the `stalePreview` `ApiError` kind (and demotes to `debug`).
- The `GeometryExporter` trait (`src/export.rs`) is implemented by `WorkerManager`
  and `PendingBackend` and called from `api/mod.rs`, `library.rs`,
  `library_ingest.rs` plus ~8 tests — a signature change there is a ~15-site edit.
- `src-tauri/crates/onecad-worker-stub` runs THREE threads (reader routes →
  kernel, status answers `GetWorkerHead`). `StubState` and stdout share ONE mutex
  so `seq` order matches byte order; chaos sleeps must stay OUTSIDE that lock or
  the status lane is gagged and every liveness drill measures the wrong thing.
  Hooks are process-wide env vars read per request, so a drill that needs a fresh
  counter must spawn a fresh worker.
- The stub's **single kernel lane serves frames in the order Rust writes them**, so
  no stub drill can ever show a Rust FRAME-ORDERING bug red. Test such gates at the
  unit level against `Shared` instead, and say so in the test's doc comment.
- Optional wire fences are **omitted keys, never `null`**: the worker refuses a
  present-but-malformed `snapshotId` with `PROTOCOL_ERROR`, which restarts it. Any
  new optional arg must be added with `if let Some(..) { args["k"] = .. }`, never a
  `json!` literal or a serde field that can serialize `Value::Null`.
- Long chaos drills are a suite-wide hazard: a 20 s stub hold inside `worker_chaos`
  starved `convergence_drill_kill_mid_plan_repeatedly` (which retries under
  `wait_ready(2 s)`) under full-workspace parallelism, while both passed in
  isolation. Keep a liveness drill's duration a small multiple of the budget it is
  proving.
