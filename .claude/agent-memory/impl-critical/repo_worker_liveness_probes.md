---
name: repo-worker-liveness-probes
description: OneCAD worker liveness/fencing probe mechanics — stub is single-threaded, how to drive the Dispatcher and PlanExecutor in-process, restore/mesh assertions
metadata:
  type: project
---

Facts that make WP-H-shaped probes (liveness, restore fencing, executor hazards) cheap to write.

**Why:** these seams have no obvious test entry point; each of these was found by reading, not by grep.

**How to apply:**

- `onecad-worker-stub` is a SINGLE blocking `read_frame_blocking` loop (`run()` → `handle_req`). Any
  hook that sleeps inside `handle_req` blocks the request-read path, so `GetWorkerHead` pings go
  unanswered — same shape as the real worker's KERNEL lane serialising the ping behind a running job.
  A stub "status thread" would have to be added deliberately.
- Chaos configs: `fast_config` in `worker_chaos.rs` gives the whole `SupervisorConfig`; override
  `ping_interval` / `ping_timeout` / `max_missed_pings` per test. `ping_interval: 60s` disables the
  liveness kill for drills that need the worker to survive a slow op.
- `WorkerManager` implements `GeometryEngine` (so `get_worker_head()`, `save_checkpoint(step)`,
  `restore_checkpoint(RestoreRequest)`), `MeshProvider` (`fetch_mesh`), and `SolverEngine` — a test
  can drive all three verbs directly without `DocumentRuntime`. `RestoreCheckpoint` is keyed on
  `stepIndex` alone at the worker; `checkpointId` is echoed but unused there.
- MESH1 assertions need no parser: header is LE with `vertexCount` at 0x08 and the f32 bbox at
  0x20..0x38 (`bboxMaxZ` at 0x34).
- Driving the C++ Dispatcher in-process: `Dispatcher::run(in_fd, out_fd, nullptr)` on a thread over
  two `pipe()`s. `stamp_and_write` is private, so this is the only way to observe the frames it
  writes. `run()` drains both lanes before returning, so closing the write end of the input pipe is a
  safe way to end it; read the output pipe after `join()`.
- Driving `PlanExecutor` in-process: the `run_plan` helper shape in `test_wp5_plan.cpp`
  (`handle_execute_plan(session, Envelope::request(...), ctx)`). `HandlerContext::emit` is the
  per-`planStep` callback, so a test can flip the `CancelToken` at an exact point in the plan.
  `Session::store_prepared` + `accept_prepared` are public and only check jobId + epoch, so a test
  can seat an arbitrary body (pathological shapes included) in the head and then run an INCREMENTAL
  plan (`expectedBaseHash` == the head hash) to have it cloned into scratch.
- `tess::tessellate_body` does NOT throw on an unbounded planar face or a surfaceless face — both
  return `ok=false` / an empty mesh. Making `attach_tessellate` throw needs a production hook:
  since WP-H that hook exists as `artifacts.tessellate.__testThrow: true` (house style, like the
  `__crash` / `__slow` / `__fail` op-id substrings in `execute_ops`).
- Since WP-H the Dispatcher has THREE loops and `GetWorkerHead` is routed to the STATUS one
  (`register_status_verb`), so a PIPELINED request stream no longer has one global FIFO frame
  order and a `GetWorkerHead` result carries a wall-clock `inflight.ageMs`. Any gate that
  byte-compares a pipelined transcript must exclude that verb — this broke
  `check_project_to_sketch_plane_frames.sh`, whose drain verb is now `Debug.Busy
  {"durationMs":0}` (kernel lane, one deterministic frame, no side effects). `Debug.Busy` with a
  nonzero duration is also the cheapest way to hold the kernel lane busy in a test.
- `Session::fence_and_clone` takes a 5th DEFAULTED param (`const BaseCheckpointRef*`), so the
  ~10 existing test callers still compile unchanged; keep new optional params defaulted there.
