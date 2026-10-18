# In-process OCCT worker spike (P0.3, macOS)

Status: Phase 0 spike, run 2026-10-17 on macOS (Apple Silicon) against `d76857c2`. Changes are uncommitted. Plane: ONECAD-258 (program ONECAD-255).

## Current sidecar regression contract (ONECAD-272)

The October 17 findings below are historical. `ConstraintSolver::configureSystem`
now sets `GCS::NoDebug` in both sidecar and library builds; structured redundancy
results remain available. The CTest `solver_stdout_hygiene` launches the actual
sidecar through Unix pipes and exercises one and 512 redundant sketch upserts.
It checks OCW1 framing, response correlation, output sequence, sketch revision,
`OverConstrained`, three degrees of freedom, no conflicts, zero residual, and
clean EOF after Shutdown. This covers both exit-time flushing and a full stdio
buffer; an in-process test cannot establish this contract.

After building the worker, run:

```sh
ctest --test-dir "${ONECAD_WORKER_BUILD_DIR:-worker/build}" \
  -R '^solver_stdout_hygiene$' --output-on-failure
```

The static stdout scan still covers first-party `worker/src`; the executable
regression covers the vendored solver's behavior. This does not qualify native
UI input or the optional in-process experiment. Production, normal development
and the normal real-worker CI lane use the sidecar (ADR-0016).

## Historical investigation

**Scope change (user, 2026-10-17):** the iPad target is dropped; OneCAD targets the macOS desktop app only. The in-process worker was motivated by iPadOS, which cannot spawn the sidecar. On macOS it is now at most an optional development and CI host. The shipping macOS host stays the sidecar. No iOS work was done or is planned under this task.

This is the second revision of the spike. The first revision was reviewed on a fresh context; its findings and their resolution are at the end. This revision adds a bounded wait on the process-wide OCCT locks, moves the crash-circuit key fully worker-side (ABI 3), splits multi-line log messages, and fixes an fd leak on an exception path.

## Outcome

The same C++ worker runs in-process behind the same OCW1 wire through a C ABI (`onecad_worker_run`, plus `onecad_worker_run_config` at ABI version 3). The unchanged `onecad-protocol` `ProtocolClient` drives it over a socketpair. The in-process worker passes sketch → extrude → accept → tessellate with the sidecar's fingerprint (`0a6a1dce34181289`) and byte-identical meshes.

- **Latency:** op latency is the same as the sidecar. The handshake drops from about 50 ms to under 0.15 ms.
- **Concurrency:** two in-process workers doing STEP IO corrupted the heap before a process-wide lock existed (first revision's evidence, not re-run here). With the lock, 30 of 30 `concurrent 300` runs and 10 of 10 `concurrent-modeling 1000` runs passed in this revision.
- **Wedge inside a STEP call:** the lock is now taken with a bounded wait. While a sibling holds it, STEP and XCAF calls fail after the bound with a retriable `OP_FAILED` whose message starts with `OCCT_GLOBALS_BUSY`. Modeling and starting new workers are unaffected.
- **Crash journal:** written worker-side, synchronously on the kernel thread, before each op starts. The journal event now carries the op's own prefix hash and the fingerprint, so the record holds the finished crash-circuit key. The host does not arm anything per plan. The worker-built key equals the host's `plan_op_keys` rule.
- **Wedged op:** cannot be stopped in-process. It surfaces as "relaunch required", and the thread leaks while burning one core.
- **Restart:** run → Shutdown → run re-initializes cleanly over 5 cycles, including Shutdown with the host end held open.
- **Logging:** `Log.h`, the OCCT default messenger and the OCAF messenger reach `log_cb`, one newline-free line per call. PlaneGCS stdout is silenced in library builds. OCCT's direct `cout` writes are routed only by a bridge in the spike crate, because the stdout hygiene gate bans the identifier under `worker/src`.
- **Defect found in the existing sidecar:** PlaneGCS writes redundant-constraint diagnostics to **stdout**, the frame channel. After about 100 diagnoses the buffered bytes land inside the OCW1 stream. This was found and reproduced in the first revision and not re-run here. It is not fixed in the sidecar.

**Recommendation for macOS-only:** keep the sidecar as the only production host. Keep `onecad_worker_lib` and the C ABI, because they cost one `EXCLUDE_FROM_ALL` target and their changes are inert in the sidecar. Whether to build an in-process `WorkerHost` for CI or development is a user decision (see the recommendation section). If one is built, run one document worker plus at most one ingest worker, with no prewarm, and keep `occt_globals_mutex` in place.

## What was built

| Path | What |
|---|---|
| `worker/src/capi/onecad_worker.h` | C ABI, version 3. `onecad_worker_abi_version()`, `onecad_worker_run(in_fd, out_fd, log_cb, ctx, log_level)`, and `onecad_worker_run_config(const onecad_worker_config*)`. The config adds a crash-journal callback, which takes an event struct (op position, prefix hash, fingerprint), and `occt_lock_wait_ms`. The header documents the contract: fd ownership, the socket requirement, wedge behaviour, the process-global log sink, what is not routed, and STEP serialization. |
| `worker/src/capi/onecad_worker.cpp` | The ABI. It sets SO_NOSIGPIPE on `out_fd`, installs the log sink, level and lock wait, routes OCCT diagnostics, adapts the journal callback to `protocol::JournalHooks`, runs the shared loop and catches all exceptions (`ONECAD_WORKER_EXIT_INTERNAL` = 3). |
| `worker/src/capi/WorkerRuntime.{h,cpp}` | The verb handlers, verb table, hello payload, selftest and dispatch loop, moved verbatim out of `main.cpp`. Both hosts call this one copy. `route_occt_diagnostics` takes its own mutex, not the STEP lock, so a new worker can start while a sibling is wedged in STEP. In library mode `run_dispatch_loop` closes both fds on every exit path, including an exception. |
| `worker/src/capi/OcctLogPrinter.h` | A header-only `Message_Printer` that forwards each non-blank line, prefixed `occt: `, to `util/Log.h`. `make_diagnostic_printer()` always returns it in library builds. In the sidecar it returns the same `Message_PrinterOStream("cerr")` as before. |
| `worker/CMakeLists.txt` | `target_sources(onecad-worker … WorkerRuntime.cpp)`, and a new `onecad_worker_lib` (STATIC, `EXCLUDE_FROM_ALL`). The library recompiles `worker_core`'s source list with `ONECAD_WORKER_LIBRARY=1` and reads its usage requirements off `worker_core` with generator expressions. Configure fails if `worker_core` gains a PRIVATE-only requirement. It also generates `onecad_worker_lib.link`, a link manifest for non-CMake hosts. The default build and ctest are unchanged. |
| `spikes/inproc-worker/` | A standalone cargo crate: an empty `[workspace]`, its own `./target`, and it builds `--offline`. `build.rs` configures the worker with the CLAUDE.md provenance flags into `target/worker-lib-build` and links from the manifest. `csrc/cout_bridge.cpp` is the `cout` bridge. `csrc/lock_probe.cpp` holds `occt_globals_mutex` for the drill. `src/journal.rs` is the worker-side journal and its drill. |

Edits to existing worker files. All keep the sidecar's behaviour. Every library-only change is under `#ifdef ONECAD_WORKER_LIBRARY`:

1. `util/Log.h`: an optional process-global sink (`SinkFn`, `set_sink`, `sink_installed`, `set_level`, `emit_raw`). With a sink installed, each message is delivered one line per call with blank lines dropped. With no sink, `emit()` is byte-identical to before.
2. `main.cpp`: reduced to the process concerns (env level, endianness, argv, stdio). It calls `onecad::worker::{route_occt_diagnostics, run_selftest, run_dispatch_loop}` in the same order with the same log lines.
3. `io/OcctStaticGuard.{h,cpp}`:
   - New `occt_globals_mutex()`, a `std::timed_mutex`.
   - New `BoundedLock`, `kOcctGlobalsBusy` and `set_occt_lock_wait_ms()`. The lock wait defaults to 20 s, below the 30 s wedge deadline of the shortest STEP verb. These are inline in the header so `tests/CMakeLists.txt`'s standalone `occt_persistence_codec` target, which compiles `XcafCodec.cpp` without `OcctStaticGuard.cpp`, still links.
   - In library builds `OcctMessengerGuard` is a no-op. The default messenger is routed once at worker start; swapping the shared printer list per call would race with other workers printing through it.
   - The cerr printer comes from `make_diagnostic_printer()`.
4. `io/OcafApp.{h,cpp}`: `ocaf_mutex()` is now a `std::timed_mutex`, and the OCAF messenger's printer comes from `make_diagnostic_printer()`.
5. `io/StepRead.cpp` (`read_step`), `io/XcafRead.cpp` (`read_step_attributes`) and `io/ExportStep.cpp`: take `occt_globals_mutex()` through `BoundedLock` before `STEPControl_Controller::Init()` and hold it for the whole call, so the guards restore under it. `ExportStep`, `XcafRead` and `io/XcafCodec.cpp` take `ocaf_mutex()` through `BoundedLock` too. This applies to the sidecar as well, where both locks are uncontended: the first `try_lock` succeeds, which is the cost of the `lock_guard` it replaces. Lock order is the globals lock before `ocaf_mutex()`. The three STEP entry points are leaves, so the lock cannot self-deadlock.
6. `io/InspectStep.cpp`: a busy-lock failure from the read, attribute or codec pass returns a **retriable** `OP_FAILED`, instead of being degraded to "no attributes" or reported as non-retriable. This path is unreachable in the sidecar.
7. `protocol/Dispatcher.{h,cpp}`:
   - `close_input()` closes the reader fd. Only the library path calls it, after `run()` returned.
   - In library builds `close_reader_fd()` calls `shutdown(fd, SHUT_RD)` instead of `close(fd)`. It falls back to `close` only for a non-socket (see Risks).
   - `JournalHooks` and `set_journal_hooks()`. The kernel loop calls `job_begin` at dequeue and `job_end` after the handler returns. `plan_op_begin` carries the op's position in `args.ops` and its `prefixHashes[position]`, falling back to `expectedBaseHash` like Rust's `plan_op_keys`. `HandlerContext` gains a default-empty `op_begin`. The hooks are empty in the sidecar, so nothing is called.
   - The three lane threads are owned by a scope guard that stops and joins them on every exit from `run()`. Before, an exception after the threads started destroyed joinable `std::thread`s, which is `std::terminate`. The normal path joins at the same point as before.
8. `session/PlanExecutor.cpp`: calls `ctx.op_begin(step_index, position, op_id)` before each op's test hooks and work; it is a no-op when empty. The `__crash` `std::abort` is wrapped in `#ifndef ONECAD_WORKER_LIBRARY`: the string `__crash hook` occurs 0 times in `libonecad_worker_lib.a` and once in `onecad-worker`.
9. `sketch/solver/ConstraintSolver.cpp`: in library builds `configureSystem()` sets PlaneGCS `debugMode = GCS::NoDebug`. Every reachable `Base::Console().log`, the `vfprintf(stdout)` in `third_party/planegcs/Base/Console.h`, is gated on `debugMode`.
10. `elementmap/ElementMapPartition.cpp:537`: the `ONECAD_LADDER_DEBUG` `fprintf(stderr,…)` became `log::emit_raw`, which writes the same bytes when no sink is installed.

No SolidCraft code was adapted, so `ATTRIBUTION.md` is untouched.

## Reproduce

```bash
cmake --build worker/build --parallel 4          # existing targets
scripts/check-worker-stdout-hygiene.sh
ctest --test-dir worker/build --output-on-failure
cd spikes/inproc-worker && cargo build --release --offline   # also builds onecad_worker_lib
./target/release/inproc-worker-spike all
# or: bench | logs | cancel | step-lock | restart | concurrent [n] | concurrent-modeling [n] | journal | wedge
```

## Results

All numbers below were measured in this revision unless marked otherwise. The machine was shared with other agents; the load average ranged from 1.5 to 14 during the runs.

### Gates

- `cmake --build worker/build --parallel 4`: `[100%] Built target test_kernelbench_validators`. `ctest -N`: Total Tests 206.
- `ctest --test-dir worker/build --output-on-failure`, serial, after the last source change: **206/206 passed**, 61.80 s, load average about 6 to 7. An earlier run in this revision also passed 206/206 (73.88 s). The only later edits were comment changes in `onecad_worker.h` and `CMakeLists.txt`, followed by a successful rebuild.
- `scripts/check-worker-stdout-hygiene.sh`: `==> worker stdout hygiene: clean`.
- `onecad-worker --selftest`: `selftest OK (occt 8.0.1, build onecad-occt-8.0.1-b8f597c67781-kp1, fingerprint 0a6a1dce34181289, upsert dof=12)`.
- Spike: `cargo fmt --check` clean; `cargo clippy --release --offline --all-targets -- -D warnings` clean; `inproc-worker-spike all` exit 0, four times in this revision.
- A fresh `cmake -S worker -B <scratch>` configure succeeded and generated a 52-line link manifest.
- The first revision's build of the worker was broken by its own last edit: `test_occt_persistence` failed to link with undefined `BoundedLock` symbols. That was fixed by making `BoundedLock` header-only (edit 3).

### Happy path and latency

Both columns use the same `ProtocolClient` and the same requests, copied from `protocol/fixtures/execute_plan_ordinary.ndjson` ops 0–1: a 40×20 rectangle extruded 25 as NewBody, then `AcceptPrepared` and `Tessellate(body_op1)`. Each round opens a fresh session. The sidecar is `worker/build/onecad-worker` over pipes. Values are per-run medians; the ranges span 7 runs (4 `all`, 3 `bench`).

| ms, median per run (range over 7 runs) | in-process | sidecar |
|---|---|---|
| handshake, start → hello (n=5) | 0.063 – 0.132 | 48.4 – 66.2 |
| GetWorkerHead RTT (n=200) | 0.021 – 0.052 | 0.026 – 0.057 |
| OpenSession (n=30) | 0.032 – 0.083 | 0.032 – 0.119 |
| ExecutePlan (n=30) | 1.96 – 2.40 | 1.99 – 4.55 |
| AcceptPrepared (n=30) | 0.031 – 0.088 | 0.032 – 0.103 |
| Tessellate (n=30) | 0.32 – 0.45 | 0.32 – 1.01 |
| first ExecutePlan of the worker | 2.94 – 21.22 | 3.01 – 5.71 |

- **Correctness:** both hosts report fingerprint `0a6a1dce34181289`. Each run's 60 rounds produce exactly one distinct mesh sha256: 12 triangles, a 1516-byte MESH1.
- **Reading the ranges:** the high sidecar values all come from one `bench` run during a load spike. In the quietest runs the two hosts are within 0.05 ms of each other on every op.
- **Cold cost:** in-process, the first plan of the *process* pays OCCT's lazy initialization once. The 21.22 ms outlier was the first run right after the library was relinked; the other six were 2.94–7.45 ms. Later workers in the same process start warm. Prewarming a worker buys nothing in-process, because a handshake is under 0.15 ms.
- **Not measured:** large-payload throughput. The meshes are tiny, so these numbers are per-message cost only.

### Logging (task 2)

- **`Log.h`:** every `WLOG_*` line reaches `log_cb`; 237 lines in an `all` run. When the sink is installed, none reach stderr.
- **One line per call:** across each of two full `all` runs, 0 lines delivered to `log_cb` contained a newline or were empty. Before this revision, OCCT's multi-line banners arrived as one call with embedded newlines, and only the first line carried the `occt: ` prefix. The header already promised line splitting, but the code did not do it.
- **OCCT default messenger:** routed once at worker start to the bridge. One ExportStep plus InspectStep round produced 14 `occt: …` lines, for example `occt: ******        Statistics on Transfer (Write)                 ******`.
- **OCAF application messenger:** its printer is chosen once, when `ocaf_application()` is first created. In library builds it is always the bridge, which falls back to stderr through `Log.h` when no sink is installed.
- **The sink is process-global:** a stack of live `(cb, ctx)` pairs. The active sink is always the newest live run's callback and falls back to a survivor, so a callback is never called after its own run returns. Lines carry no worker identity.
- **PlaneGCS:** silenced in library builds (`NoDebug`), not routed. Routing would mean editing `third_party/planegcs/Base/Console.h`.
- **`cout` (not in the library):** OCCT 8.0.1 writes directly to the C++ `cout` stream in about 10 TKBRep files, for example `BinTools_ShapeSet.cxx:406`. In library mode that stream is the host's stdout. `spikes/inproc-worker/csrc/cout_bridge.cpp` swaps the stream's buffer for a line forwarder; it captured 1 of 1 probe line. It cannot move under `worker/src/capi/` while `check-worker-stdout-hygiene.sh` bans the identifier there. That needs a one-file exemption, which is an owner decision.

### (a) Wedged op

The wedge is simulated with `Debug.Busy {durationMs: 3600000}`, which spins the kernel lane with no cancel check.

| Probe | Observed |
|---|---|
| Status lane while wedged | `GetWorkerHead` in 0.07–0.15 ms, with `inflight={"ageMs":302,…,"verb":"Debug.Busy"}`. The existing wedge detector works unchanged. |
| `cancel(id)` | No terminal frame within 1 s, because the op ignores cancel |
| CPU | 1.00 CPU-s per wall-s: one core burns for as long as the process lives |
| `Shutdown` (2 s deadline) | Deadline exceeded: Shutdown is queued behind the wedged kernel lane |
| Host closes its socket end and waits 3 s | `onecad_worker_run` does not return: **RelaunchRequired**. `Dispatcher::run` joins its lanes. |
| Threads | 5 before, 7 after: the reader (blocked in join) and the spinning kernel thread leak |
| A new worker in the same process meanwhile | Works: handshake 0.1 ms, plan 2.5–2.7 ms |
| Cooperative cancel (`op1__slow` ExecutePlan) | Terminal `CANCELLED` 8.8–10.7 ms after the cancel |

**(a3) Wedge inside a STEP call.** A worker wedged inside a STEP call would hold `occt_globals_mutex` for the rest of the process's life. The drill simulates that by holding the mutex from the host (`csrc/lock_probe.cpp`), with a lock wait of 500 ms:

| Probe while the lock is held | Observed (5 runs) |
|---|---|
| `ExportStep` | `ok=false code=OpFailed retriable=true` after 500–501 ms, message `OCCT_GLOBALS_BUSY: ExportStep could not get occt_globals_mutex within 500 ms …`, and no file written |
| `InspectStep` | `ok=false code=OpFailed retriable=true` after 500–501 ms, message `InspectStep: OCCT_GLOBALS_BUSY: StepRead could not get …` |
| Modeling round on the same worker | Succeeds: plan 2.7–13.4 ms, tessellate 0.5–1.7 ms |
| A new worker started while the lock is held | Handshake 0.09–0.29 ms, plan 2.5–7.2 ms, `Exited(0)` |
| After release | `ExportStep` succeeds in 1.3–3.5 ms |

The first version of `BoundedLock` polled `try_lock` with 1 ms sleeps. Under real contention (`concurrent 300`) that cost throughput: 15 runs took 3.04–6.08 s of wall time. Switching both locks to `std::timed_mutex` and waiting in `try_lock_for` slices of at most 5 ms, which wakes the waiter as soon as the holder unlocks, brought 15 runs back to 2.67–2.99 s. That is the same range the first revision measured with a plain blocking mutex (10 of 12 runs at 2.70–2.91 s).

**"Relaunch required" design:**

- **Detection:** the existing detector (`manager.rs:1336-1408`, `wedged_inflight`) still decides when a worker is wedged.
- **Response.** Where the sidecar host calls `child.start_kill()`, the in-process host instead:
  1. Closes its socket end.
  2. Waits a bounded grace period for the thread.
  3. On expiry, moves the worker to a terminal `RelaunchRequired { verb, age_ms }` state and drops the JoinHandle, detaching the thread.
  4. Starts a fresh worker (a new epoch) so the user keeps working, up to a cap on leaked workers (for example 1). If the wedged verb was a STEP verb, the fresh worker's STEP calls fail as `OCCT_GLOBALS_BUSY`. The supervisor should map that to "import/export unavailable until relaunch" instead of retrying in a loop.
  5. Shows a persistent "restart OneCAD to reclaim CPU" prompt.
- **macOS:** the sidecar remains the production host because it can SIGKILL.

### (b) Crash journal

**Why a host-side cursor is wrong.** The first design advanced the cursor host-side on each `planStep` frame the host received. In a real in-process crash, the fault is asynchronous on the worker's kernel thread. Frames still in the socket buffer, or received but not yet processed, die with the process, so the cursor lags. With a poison threshold of 1, a lagging cursor poisons the wrong op immediately.

**Design (ABI 3).** The host writes nothing per op, and arms nothing per plan. The worker calls the journal callback synchronously on its kernel thread:

- `JOB_BEGIN(verb)` at dequeue: the host writes `inflight.json` = `{v:3, documentId, requestId, verb, cursor:null, opId:null, key:null}`.
- `PLAN_OP_BEGIN` before each op's work: the event carries the op id, its position in `args.ops`, its own `prefixHashes[position]`, and the fingerprint. The host stores `cursor`, `opId` and `key = prefixHash|opId|fingerprint`, which is the existing poison circuit's key format (`plan_op_keys`, `manager.rs:1991`).
- `JOB_END(verb)` after the handler returns, before the terminal frame: the host deletes the file.

On launch, a leftover journal with a `key` poisons that key, after checking that the key's op component equals the recorded `opId`. A journal without a key is logged as a crash in that verb and poisons nothing. Only the kernel lane is journaled; solver-lane (`Sketch*`) jobs are not.

**Drill.** A child copy of the spike (a host plus an in-process worker) sets SIGSEGV to `SIG_DFL` and raises the fault **on the worker's kernel thread**. Rust std's SIGSEGV handler returns for a fault outside a guard page, which would swallow an injected signal.

- `op1-stalled`: SIGSEGV via `pthread_kill` to the kernel thread 100 ms into `op1__slow`, while the host's event handling never runs. This stands in for a busy UI thread.
- `op1-live`: the same fault, with the host processing `planStep` frames as they arrive.
- `tess`: a plan completes, then SIGSEGV is raised on the kernel thread as the `Tessellate` job begins.

| Case | Child exit | Worker-side journal | Host-side cursor (old design) |
|---|---|---|---|
| op1-stalled | signal 11 | poison `op1__slow`; key equals the host rule | `op0`, the **wrong op** |
| op1-live | signal 11 | poison `op1__slow`; key equals the host rule | `op1__slow` |
| tess | signal 11 | crash in a Tessellate job, nothing poisoned | no journal |

In both op1 cases, reopening the same plan gates on `op1__slow`. Editing op1's distance from 25 to 30 changes the prefix hash, so the gate returns `None` and the circuit heals.

What the drill does not prove: the fault comes from the drill's own callback thread, not from real OCCT code. A crash between `JOB_END` and the next `JOB_BEGIN`, during the terminal-frame write, is attributed to nothing.

**Costs** (ranges of the per-run values over 4 `all` runs):

| Measurement | median | p95 |
|---|---|---|
| ExecutePlan without journal / with the callback writing records (n=30) | 2.01 – 2.52 ms / 2.26 – 2.67 ms | — |
| one journal record write on the kernel thread, tmp + rename (n=302) | 0.070 – 0.082 ms | 0.110 – 0.135 ms |
| write variant: tmp + `sync_all` (`F_FULLFSYNC`) + rename + dir sync (n=100) | 9.05 – 10.04 ms | 10.33 – 12.01 ms |
| write variant: tmp + rename, no fsync (n=100) | 0.105 – 0.234 ms | 0.126 – 0.385 ms |

No fsync is the recommendation. Surviving an app crash needs only the kernel page cache, which survives process death; fsync would add about 10 ms per op to the kernel lane. The journal is not power-loss safe, which is acceptable for a crash-loop breaker. The in-process poison threshold should be **1**, not 3, because each crash costs an app relaunch.

### (c) Process globals and concurrent workers

| Global | Where | Concurrent in-process workers | Re-init |
|---|---|---|---|
| `Interface_Static` STEP knobs, `STEPControl_Controller::Init()`, XSControl session state | `read_step`, `read_step_attributes`, `ExportStep`, and the STEP reader and writer internals for the whole transfer | A demonstrated heap-corruption crash without a lock (first revision). Now serialized by `occt_globals_mutex()` for the whole call, with a bounded wait. | Restored per call by `InterfaceStaticGuard` |
| `Message::DefaultMessenger()` printer list | `OcctMessengerGuard`, `route_occt_diagnostics` | Library builds: mutated once, at the first worker start, under `route_occt_diagnostics`' own mutex; the guard is a no-op. Concurrent prints through an unchanging list are reads. | Idempotent |
| OCAF application and `ocaf_mutex()` | `OcafApp.cpp`. The lazy `static Handle` init is unsynchronized, but every caller holds `ocaf_mutex()` first. | Serializes XCAF work across workers, with a bounded wait | Survives; reused |
| OCCT lock wait | `occt_lock_wait_storage()` | Shared: the last started run sets it | Set per run |
| Log sink and level | `Log.h` statics | Shared: lines mixed, last level wins | Restored per run |
| PlaneGCS `SolverReportingManager` | `GCS.cpp:298` magic static | Stateless without `_DEBUG_TO_FILE`: safe | n/a |
| PlaneGCS `Base::Console` | `Console.h` | Silent in library builds (`NoDebug`) | n/a |
| `Uuid` RNG | `util/Uuid.h:21` `thread_local` | Safe | n/a |
| Dispatcher, Session, SolverLane, PlaneGCS `System` | Per worker instance | Safe | Fresh per run |
| `cout` bridge (spike) | Process-global stream buffer | Shared, mutex-guarded | Idempotent |

Concurrent runs, each started standalone, with two workers doing the rounds at the same time:

| Mode, rounds per worker | Result in this revision |
|---|---|
| `concurrent 300` (plan + accept + tessellate + `ExportStep(AP242DIS)` + `InspectStep`) | **30 of 30 passed**, each with one distinct mesh sha256: 15 with the polling lock (wall 3.04–6.08 s) and 15 with the timed lock (wall 2.67–2.99 s) |
| `concurrent-modeling 1000` (plan + accept + tessellate only) | **10 of 10 passed**, wall 2.55–2.90 s, one distinct mesh sha256 each |

Carried over from the first revision and not re-run here: without the lock, 2 of 6 `concurrent 300` runs crashed with SIGSEGV (rc 139). The reviewer saw 5 of 7 crash (rc 133/133/139/134/133) and caught heap corruption in `Interface_Static::SetIVal` under `read_step_attributes` in lldb.

Limits of this evidence:

- The modeling drill covers sketch, extrude and tessellate only. Fillet, boolean, shell, pattern, import and the solver lane were not run concurrently.
- The audit found no other process-global that our code writes. OCCT internals were not audited beyond the STEP stack.
- No ThreadSanitizer run. OCCT is not instrumented, so it would see only our code.

### (d) Restart in the same process

Five cycles of start → model round → Shutdown in one process. On odd cycles the host keeps its socket end **open** across Shutdown, so the worker must wake its own blocked reader with `shutdown(SHUT_RD)`.

- Handshake 0.05–0.21 ms and plan 2.2–5.7 ms across 5 runs.
- Every cycle ended `Exited(0)`, including the held-open ones, with one distinct fingerprint and one distinct mesh sha256.

Re-init works. Nothing is left behind beyond the persistent OCCT globals above, which are designed to survive.

## Defect: PlaneGCS writes to the sidecar's stdout

Found and reproduced in the first revision; not re-run here.

`GCS.cpp:5622-5637,5651` logs `Sketcher::RedundantSolving-…` and `Sketcher Redundant solving: N redundants` through `Base::Console().log`, which is `vfprintf(stdout)`. The trigger is `debugMode == Minimal`, the default (`GCS.cpp:491`); the sidecar never changes it. The hygiene gate does not scan `third_party/`.

The probe sent raw OCW1 over pipes to `worker/build/onecad-worker`. Each SketchUpsert had a duplicated `Horizontal` constraint on one line, followed by a Shutdown.

- 1 upsert: `frames_parsed=3 foreign_bytes=152`. The text arrived after the last frame, flushed at exit.
- 200 upserts: `frames_parsed=109 stdout_bytes=120666 foreign_bytes=72157`. stdio's buffer flushed mid-stream, and the frame stream was corrupt from frame 110. Rust would see bad magic, which is fatal, and restart the worker. The reviewer reproduced it independently.

So any sketch with a redundant constraint corrupts the protocol after about 100 diagnoses. The library build is not affected. The sidecar fix is one line: remove the `#ifdef ONECAD_WORKER_LIBRARY` around `debugMode = GCS::NoDebug` in `ConstraintSolver::configureSystem`, or point `Base::Console` at `WLOG_*`. It is a behaviour change outside this task's remit and needs a test, so it is left for a Plane task. The hygiene gate should also scan `third_party/planegcs`. **This defect matters for the macOS product regardless of the in-process decision.**

## Recommended in-process model

With the iPad target dropped, nothing requires an in-process host. What it offers on macOS:

- **For:** a 50 ms cheaper worker start per spawn, which matters only for test suites that spawn many workers. In-process debugging of the C++ worker under the same debugger as the app. A host for future Rust-side scenario tests without a staged sidecar binary.
- **Against:** no crash or hang isolation; a second supervision path to keep correct (`RelaunchRequired`, the persistent journal); and the library-only `#ifdef` paths, which no CI lane builds today.

**Recommendation:** the sidecar is the only production host on macOS. Do not build an in-process `WorkerHost` in Phase 1b unless the user wants it for CI or development. If it is built, use **several workers with the process-wide OCCT-globals lock (bounded), capped at one document worker plus one transient ingest worker, with prewarm off.** The reasons:

- **The Rust side already runs several workers:** the live document worker (`state.rs:702`), a prewarmed worker (`state.rs:584`), and ephemeral library-ingest workers (`library.rs:1008`, `library_ingest.rs:568`). Ingest reads STEP while the document worker may export.
- **One worker with N sessions is a protocol change.** OCW1 has one implicit session per connection: `OpenSession` resets the document, and frames carry no session id. It would need a SCHEMA §13 bump on both tracks and would serialize ingest behind modeling on the one kernel lane.
- **The lock is a precondition, not an option.** Without it, two workers doing STEP IO corrupt the heap. With it, STEP sections serialize across workers, which is what one multi-session worker would do anyway, while modeling runs in parallel. The bounded wait keeps a wedged STEP call from wedging the other worker.
- **The cap bounds shared fate.** A wedge or crash takes every in-process worker down together.

The lock is already in the code, it is inert in the sidecar, and ctest exercises it through the STEP tests, so keeping it costs nothing even if no in-process host is built.

## What `WorkerHost` must abstract (if built)

From `manager.rs` (`spawn_and_connect` ~1178, `run_until_death` ~1347, `terminate_child` ~994):

1. **`start(epoch) -> (transport halves, log stream)`.**
   - Sidecar: spawn the child, pipes, the stderr forwarder.
   - In-process: a socketpair with SO_NOSIGPIPE on both ends (the worker's input must be a socket, see Risks), `dup` for the second fd, a dedicated 8 MiB thread, `log_cb` into `tracing` with `target: "worker"`, and the `cout` bridge installed once per process. The level and lock wait are passed through the ABI, not the environment.
   - The handshake and `ProtocolClient::connect` stay shared.
2. **`exited() -> Exit`**, a future. Sidecar: `child.wait()`. In-process: the thread's return code over a oneshot channel.
3. **`terminate(grace) -> Termination`.**
   - Sidecar: Shutdown, then SIGKILL, which always succeeds.
   - In-process: Shutdown, close the socket, wait for the grace period, then **`RelaunchRequired`**. This is a new terminal outcome the supervisor must handle: no epoch-bump retry loop on the leaked thread, a leaked-worker cap, and "import/export unavailable" when the wedged verb is a STEP verb.
4. **`capabilities`**: `can_kill`, `isolated`, `prewarm_useful`, `journal_required`. The supervisor uses these to pick the wedge policy, the poison threshold (3 or 1), and whether a crash journal is required. In-process it is required, because a crash takes the supervisor's in-memory circuit with it.
5. **Crash journal**, persistent for in-process: the worker-side `on_event(JOB_BEGIN | PLAN_OP_BEGIN | JOB_END)` callback and `recover_on_launch()`. The sidecar keeps its existing in-memory circuit, which is correct there because the surviving host drains the pipe after the child dies.

These stay identical: the ping and wedge detector (the status lane works in-process), fencing, epochs, the restart hook, and the protocol client.

## Risks

- **No isolation in-process.** An OCCT SIGSEGV or abort kills the app. The worker installs no `OSD::SetSignal`, so access violations are real signals. The journal only prevents crash loops.
- **Unstoppable wedges.** A leaked thread costs one core and its memory until relaunch. After a wedge inside a STEP or XCAF call, every worker's STEP and XCAF calls fail as busy until relaunch.
- **A blocking host log callback stalls every worker.** `log_cb` runs under the worker's process-wide log mutex, and the OCCT bridge also calls it while `occt_globals_mutex` is held. The ABI tells hosts to return quickly; a host must not do blocking IO in the callback.
- **Reader fd reuse (fixed for sockets).** Before the first revision's fix, Shutdown made the kernel thread `close()` the reader fd while the reader could still be inside, or about to re-enter, `read()` on that number. In a host process, another thread can be handed the freed number at once and the stale read would consume its bytes. This was found by reading the code and was not reproduced. Library builds now `shutdown(SHUT_RD)` a socket and close it only after `run()` returned. A **pipe** cannot be shut down, so for a pipe the old `close()` remains; the ABI header tells hosts to pass a socket.
- **`cout` bridge placement.** It needs the hygiene-gate exemption before it can live in the library. Until then every host must install it, or OCCT's direct `cout` lines reach the app's stdout.
- **Secondary thread stacks.** The kernel, solver and status lanes are `std::thread`s with macOS's default 512 KiB secondary stack, the same as in the sidecar.
- **Library target not in CI.** `onecad_worker_lib` is `EXCLUDE_FROM_ALL`, so no CI lane compiles it. The library-only `#ifdef` paths are exercised only by this spike. The first revision's final edit broke `test_occt_persistence` without anyone noticing until this revision's rebuild, which shows the cost of that gap. If the library is kept, a CI job that builds `--target onecad_worker_lib` (about 40 s) and runs `inproc-worker-spike all` should come with it.
- **Layering.** `io/OcctStaticGuard.cpp` and `io/OcafApp.cpp` (in `worker_core`) include `capi/OcctLogPrinter.h`, so core depends on capi. The header should move to `util/` or `io/` once path ownership allows.

## NOT DONE

- **iOS and simulator builds, and iPad memory (jetsam) measurements:** dropped with the iPad target.
- **Peak RSS in-process versus sidecar on macOS:** not measured.
- **Large-mesh throughput comparison:** the scenario mesh is 1.5 KB.
- **ThreadSanitizer, and concurrent drills beyond sketch, extrude, tessellate and STEP:** not run.
- **Re-running the unlocked-crash and PlaneGCS-stdout probes in this revision:** both are carried over from the first revision. The unlocked crash would need the lock removed again, and the PlaneGCS probe script is not part of the spike crate.
- **PlaneGCS stdout fix in the sidecar:** a behaviour change that needs its own task and test.
- **Routing PlaneGCS `Base::Console` to `log_cb`:** silenced instead, because routing needs an edit to `third_party/planegcs/Base/Console.h`.
- **Moving `cout_bridge.cpp` into `worker/src/capi/` or the ABI:** blocked on the hygiene-gate exemption, an owner decision.
- **CI coverage of `onecad_worker_lib`, and moving `OcctLogPrinter.h` out of `capi/`:** outside this task's paths.
- **Rust workspace tests:** not run, per the brief.

## Review findings and resolution

First revision, from the fresh-context review:

| Severity | Finding | Resolution |
|---|---|---|
| high | Concurrent in-process STEP IO crashes the host; the doc claimed it did not | Confirmed (2 of 6 runs crashed). Fixed with `occt_globals_mutex()`; 30 of 30 runs pass. |
| medium | The crash-journal drill could not fail; a host-side cursor lags in a real crash | Confirmed by the `op1-stalled` case. Replaced by the worker-side journal callback, with the fault raised on the worker thread. |
| medium | The reader fd was closed while the reader might still use the number | Fixed for sockets in library builds; the pipe limitation is documented. Not reproduced. |
| medium | Library-mode logging incomplete; the OCAF printer was sticky | The OCAF printer is always the bridge in library builds; PlaneGCS is silenced; the `cout` bridge is still spike-only and stated plainly. |
| low | `onecad_worker_lib` hand-copied `worker_core` requirements; no CI; layering | Requirements derived by generator expressions with a configure-time drift check. CI and layering left open. |
| low | Single-run numbers | Tables show ranges over several runs. |

Second revision, found while finishing it:

| Severity | Finding | Resolution |
|---|---|---|
| high | `BoundedLock`'s out-of-line definitions broke the link of `test_occt_persistence`, which compiles `XcafCodec.cpp` without `OcctStaticGuard.cpp` | Made header-only; ctest 206/206. |
| medium | A wedge inside a STEP call would block every other worker's STEP calls forever | Bounded wait with a retriable `OCCT_GLOBALS_BUSY` failure; proven by the `step-lock` drill. |
| medium | The 1 ms polling lock cost about 10% or more of STEP throughput under contention | Switched to `std::timed_mutex` with `try_lock_for` slices; throughput is back to plain-mutex levels. |
| medium | The ABI promised newline-free log lines, but multi-line OCCT messages arrived as one call, with the prefix only on the first line | Split per line in the sink and the bridge; 0 multi-line or empty lines in a full run. |
| low | `run_dispatch_loop` leaked both fds if an exception escaped before or during `run()`, so the host end never saw EOF | A scope guard closes them on every path in library mode. Not exercised by a drill. |
| low | The spike crate was still on ABI 2 while the library was on ABI 3 | Ported to ABI 3. The spike now refuses to run against a different ABI version. |
