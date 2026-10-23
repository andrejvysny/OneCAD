# iPadOS embedded kernel: build, identity, journal and failure policy

Code-coupled procedures for the in-process OCCT worker (ADR-0019, IP02/IP03).
Boundaries and policy are frozen in [ARCHITECTURE.md](ARCHITECTURE.md); this
file says how they are built and implemented. It records no test result —
evidence lives in Plane (ONECAD-449, ONECAD-450).

## Pieces

| Piece | Path | Role |
|---|---|---|
| Worker library | `worker/` target `onecad_worker_lib` | The OCW1 worker behind C ABI v3 (`worker/src/capi/onecad_worker.h`) |
| Bundle script | `scripts/build-ios-worker.sh` | Builds the library for one platform, merges one static bundle, writes the identity |
| Link crate | `crates/onecad-embedded-worker` | Links the bundle; safe `run`/`abi_version`/identity API; stub when unlinked |
| Supervision | `crates/onecad-app/src/worker/` | `WorkerHost` seam: `sidecar.rs` (macOS) and `inprocess.rs` (iPadOS), `identity.rs`, `kernel_journal.rs`, `backend.rs` |

## Build

1. Static OCCT per platform (device and simulator are separate Mach-O
   platforms, never one lipo slice). The script verifies the source archive's
   SHA-256 before using it and writes `share/onecad/occt-build.json` and
   `occt-build.cmake` (the three `set()` lines the worker's fingerprint seed
   reads):

   ```sh
   scripts/build-ios-occt.sh --platform iphonesimulator \
       --prefix "$HOME/.onecad-occt/8.0.1-iphonesimulator" \
       --build-dir "$HOME/.onecad-occt-build/8.0.1-iphonesimulator"
   scripts/build-ios-occt.sh --platform iphoneos \
       --prefix "$HOME/.onecad-occt/8.0.1-iphoneos" \
       --build-dir "$HOME/.onecad-occt-build/8.0.1-iphoneos"
   ```

2. The worker bundle:

   ```sh
   scripts/build-ios-worker.sh --platform <iphonesimulator|iphoneos|macosx> \
       --config <Debug|Release> [--test-hooks] --out <absolute dir>
   ```

   It configures `worker/` library-only (`ONECAD_WORKER_LIBRARY_ONLY`, forced
   for `CMAKE_SYSTEM_NAME=iOS`; arm64, deployment 18.0, the platform SDK) against
   the matching OCCT prefix — the static iOS prefix, or for `macosx` the desktop
   shared OCCT (`ONECAD_OCCT_ROOT`, default `~/.onecad-occt/8.0.1`) — and builds
   `onecad_worker_lib` plus `onecad_worker_link_probe`. Emitted into `--out`:

   | File | Content |
   |---|---|
   | `libonecad_worker_bundle.a` | `libtool -static` (`ZERO_AR_DATE=1`) of the worker library, PlaneGCS and — iOS — exactly the OCCT toolkit archives the probe's linker map pulled objects from. `macosx` bundles no OCCT; it links the desktop dylibs the probe still references after `-dead_strip_dylibs`. |
   | `onecad_worker.h` | The C ABI header |
   | `onecad-embedded-kernel.json` | Identity manifest v2 (below) |
   | `onecad-embedded-link.txt` | `kind=value` link directives for `build.rs` |
   | `audit.txt` | The audit commands and their output |

   System libraries are derived, not listed by hand: the bundle is linked into
   a probe with libc++ alone, and the frameworks the CMake probe link used are
   added only if that leaves symbols undefined. (OCCT 8.0.1 for iOS needs none.)

   `--test-hooks` compiles `Debug.Busy` and the `__slow` op hook into the
   library (`ONECAD_WORKER_TEST_HOOKS`) and records `"testHooks": true`. A
   release iOS build refuses such a bundle (`onecad-app/build.rs` and the host
   preflight). Without it the library answers `Debug.Busy` with
   `PROTOCOL_ERROR unknown verb`.

3. Rust. `ONECAD_EMBEDDED_WORKER_DIR` names the `--out` directory; set
   `IPHONEOS_DEPLOYMENT_TARGET=18.0` so Rust objects match the bundle:

   ```sh
   # macOS in-process tests (real kernel; hooks bundle for the wedge drills)
   ONECAD_EMBEDDED_WORKER_DIR=<macosx --test-hooks out> ONECAD_REQUIRE_EMBEDDED_KERNEL=1 \
   ONECAD_REQUIRE_WORKER=1 \
   ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
       cargo test -p onecad-app --lib --features embedded-kernel worker::inprocess::tests
   # iOS type check (no kernel needed: the link crate builds a stub)
   cargo check -p onecad-app --no-default-features --features embedded-kernel \
       --target aarch64-apple-ios-sim
   # Simulator: build the test binary, run it in a booted simulator
   IPHONEOS_DEPLOYMENT_TARGET=18.0 ONECAD_EMBEDDED_WORKER_DIR=<iphonesimulator out> \
       cargo test -p onecad-app --lib --no-default-features --features embedded-kernel \
       --target aarch64-apple-ios-sim --no-run
   xcrun simctl boot <udid>
   SIMCTL_CHILD_ONECAD_REQUIRE_EMBEDDED_KERNEL=1 xcrun simctl spawn <udid> \
       target/aarch64-apple-ios-sim/debug/deps/onecad_app-<hash> worker::inprocess::tests
   # Device: compile/link only
   IPHONEOS_DEPLOYMENT_TARGET=18.0 ONECAD_EMBEDDED_WORKER_DIR=<iphoneos out> \
       cargo build -p onecad-app --no-default-features --features embedded-kernel \
       --target aarch64-apple-ios --release
   ```

   Without `ONECAD_EMBEDDED_WORKER_DIR` the link crate is a stub
   (`NotLinked`), so `cargo check/clippy/test --workspace` on macOS never needs
   the kernel: the in-process tests skip unless
   `ONECAD_REQUIRE_EMBEDDED_KERNEL=1` requires it (`ONECAD_REQUIRE_WORKER`
   requires only the sidecar); an iOS release build fails in `build.rs` instead.

4. Xcode (a `staticlib` consumer such as `onecad-ios`). The link crate emits
   the bundle as `static:-bundle`: it is NOT copied into the Rust rlib or into
   `libonecad_ios.a` (130 MB would be duplicated per crate), so the final link
   names it next to the Rust archive, from the same `--out` directory:

   ```text
   libonecad_ios.a  <out>/libonecad_worker_bundle.a  -lc++
   ```

   plus every `framework=`/`lib=` line of `<out>/onecad-embedded-link.txt`
   (none for OCCT 8.0.1 on iOS). `cargo rustc --crate-type staticlib -- --print
   native-static-libs` prints the same list for a given build.

## Identity manifest v2 (`onecad-embedded-kernel.json`)

| Field | Source |
|---|---|
| `manifestVersion` | `2` |
| `kind` | `"embedded"` |
| `abiVersion` | `ONECAD_WORKER_ABI_VERSION` (`onecad_worker.h`) |
| `protocolVersion`, `workerVersion`, `quantizationVersion` | `kProtocolVersion`, `kWorkerVersion`, `kQuantizationVersion` (`capi/WorkerRuntime.cpp`) |
| `solverPolicyVersion` | `kSolverPolicyVersion` (`util/PolicyVersions.h`) |
| `occt.version`, `occt.buildId` | the configured OCCT build |
| `occt.fingerprint` | `hex16(fnv1a64(seed))` over the exact `ONECAD_OCCT_FINGERPRINT_SEED` the build used — the worker's own rule |
| `platform` | `iphonesimulator` \| `iphoneos` \| `macosx` |
| `sdkVersion`, `deploymentTarget` | `xcrun --show-sdk-version`, the target used |
| `archiveSha256` | SHA-256 of `libonecad_worker_bundle.a` (provenance) |
| `testHooks` | whether `Debug.Busy`/`__slow` are compiled in |

The versions are parsed out of the worker sources at configure time
(`worker/cmake/EmbeddedIdentity.cmake`); configure fails when a pattern does
not match exactly once. The fingerprint differs per platform (the OCCT build
id carries the platform); it keys disposable caches and poison only.

Verification at start (`InProcessHost::launch` → `inprocess/preflight.rs`,
before any thread exists): `onecad_worker_abi_version() == 3`; a release iOS
build has a crash-journal directory; the identity — the copy
`onecad-app/build.rs` embedded, which must equal the link crate's copy —
parses as v2, names that ABI and THIS binary's platform (`iphonesimulator`,
`iphoneos` or `macosx`, from the Rust target); release iOS refuses
`testHooks`. `EmbeddedKernelConfig.identity` is a test override honoured in
debug builds only (a release accepts it only when it IS the linked identity).
After the hello, the shared handshake checks protocol, quantization, solver
policy, worker version, OCCT version and fingerprint against it — the same rule
as the sidecar manifest. A debug build without an identity skips the hello
check with a warning. `onecad-app/build.rs` fails a release build with `embedded-kernel`
when the identity is absent or names another platform. The sidecar manifest
path (`packaging/binaries/onecad-worker-manifest.json`, v1) is unchanged.

## Worker library contract (ABI 3, unchanged layout)

- `onecad_worker_run`, `_run_config`, `_abi_version` are `noexcept`; the log
  sink, level and lock wait are set up inside the `try`; the fds are closed on
  every path that never reached the dispatch loop.
- Lane threads (kernel, solver, status) get an explicit 8 MiB stack in library
  builds (`util/LaneThread.h`); the sidecar keeps the platform default.
- `JOB_END` fires after the job's terminal frame was written, so result
  serialization is inside the journal window.
- `OCCT_GLOBALS_BUSY`: every process-wide lock site refuses `OP_FAILED`,
  `retriable: true`, message starting with the prefix (`io/OcctBusy.h`). A
  plan whose op hits it (ImportStep, PlaceComponent) is refused as a whole —
  no `planStep`, no scratch — never recorded as an `opFailed` step.

## Crash journal (`<app_support>/kernel-journal/`)

`EmbeddedKernelConfig.journal_dir`. The iOS host passes
`EmbeddedKernelConfig::for_app_support(<app_support>)` (sets
`<app_support>/kernel-journal`) to `ProcessServices::with_backend`; `None`
disables the journal for tests and tools, and a release iOS launch refuses it.
One `KernelJournal` per directory per process serializes every instance's
writes.

`inflight.json` — this launch's admitted kernel and solver jobs, including queued
jobs, rewritten by temp + rename (no fsync) on every journal event and removed
when none is left:

```json
{ "version": 2, "session": "<per-launch uuid>", "complete": true,
  "jobs": [ { "instance": 1, "epoch": 3, "requestId": 41, "verb": "ExecutePlan",
              "kind": "kernel",
              "opId": "<record id>", "stepIndex": 1, "opPosition": 1,
              "key": "<prefixHash>|<opId>|<fingerprint>",
              "fingerprint": "<hex16>", "startedMs": 1761100000000 } ] }
```

`opId`/`stepIndex`/`opPosition`/`key` are `null` until a plan op begins. The
session token separates a previous launch's record from a live sibling
instance's without asking about pids. Identity is `(session, instance, epoch,
requestId, kind)`; the verb is checked on completion. Progress and completion
cannot alter a sibling job, even within the same host or across generations.
`kind` is `kernel` or `solver`; status probes are excluded. ABI 3 already carries
the unique request ID and verb, so its layout and OCW1 version remain unchanged.
Callbacks may overlap on reader, kernel and solver threads. `JOB_BEGIN` precedes
mailbox admission; `JOB_END` follows terminal serialization/write attempt, including
cancelled and superseded jobs. A failed write closes a physically completed job;
its caller still reports completion unknown and never retries automatically. Active clocks still start at dequeue.

Version 1 journals migrate by deriving kind from verb and marking evidence
incomplete: legacy writers did not record solver overlap. Version 1 poison counts
retain at most one suspicion; only complete version 2 evidence can quarantine.
Unknown versions,
truncated/malformed files and unsupported plan records never invent a guilty op.
Reads are bounded to 1 MiB, identity strings to 128 bytes, admitted journal
records to 512 and poison entries to 128. `complete: false` marks known incomplete recording (overflow or earlier
write failure); such leftovers cannot quarantine a plan. Every unfinished record,
including non-plan, queued, other-generation and other-fingerprint work, makes
attribution ambiguous before plan-key filtering. Repeated ambiguous suspicion
remains below the quarantine threshold. A valid, complete, single supported plan
record alone may advance its strike count.

Best-effort rename cannot guarantee that a failed write or immediate process loss
leaves the newest completeness marker on disk. Leftovers therefore remain
suspected interruptions; neither a successful callback nor a readable old file
proves the recorded operation caused process loss.

`poison.json` — persisted strikes:

```json
{ "version": 2, "entries": { "<key>": { "strikes": 1, "opId": "<record id>", "lastMs": 0 } },
  "appliedSession": "<the leftover session these strikes include>" }
```

Recovery runs at the first handshake of the process (the leftover file is
read when the journal opens and is replaced only then): each distinct leftover
op key under the current fingerprint gets one strike; a record without an op
(a non-plan job: termination, jetsam, backgrounding mid-Tessellate) poisons
nothing; a record under another fingerprint is discarded; strikes under other
fingerprints are pruned. `poison.json` (with `appliedSession`) is written
BEFORE `inflight.json` is replaced, so a launch killed in between neither loses
the strike nor applies the same leftover twice.

Attribution with several jobs: a leftover holding more than one unfinished job
cannot identify a guilty op, so its plan suspicions are capped at threshold − 1
(an ambiguous leftover never opens a circuit). Every key that holds a strike runs
alone among this process's plan dispatches (`ExecutePlan` and `PreviewPlan` take
`KernelJournal::plan_permit`: exclusive for a struck key, shared otherwise, held
to the terminal). Other solver/non-plan jobs can still overlap, so this ordering
does not guarantee unambiguous evidence. A successful plan heals its keys; only a
complete, single supported plan record can advance a suspicion to quarantine.

Every instance seeds its circuit from `poison.json`; at
`IN_PROCESS_POISON_THRESHOLD` (2) strikes a key's circuit is open and
`stream_plan` / `preview_plan` fail that plan fast — two, so one innocent kill
during a long op does not block it. A prepared plan heals its keys;
`clear_worker_circuit` clears the file too. A write failure is logged (rate-limited), counted, never blocks the
op, and reaches worker status (`WorkerLifecycle::JournalDegraded`,
`WorkerStatusDto.journalWriteFailures`).

## Failure policy implementation

- `WorkerHost::launch` / `WorkerInstance::{exited, terminate, force_kill}` are
  the only backend-specific parts; handshake, hello and solver-policy checks,
  epoch and `expectedBaseHash` fencing, poison keys and plan gates are shared.
- In-process stop: `SHUT_RDWR` on a dup of the host socket (EOF to the worker
  whoever holds the tokio halves), then a bounded wait for the run thread's
  exit signal: 1 s on a wedge or ping timeout; on retirement the longest §8
  op deadline of the supervisor's policy (`ExecutePlan` 180 s in production) —
  a retired run finishes its current op before it returns, and only a run
  still busy past the deadline it would have been wedge-stopped at is stuck.
- A run that does not return is ABANDONED: counted, its thread left running,
  and a process-wide latch is set (`in_process_relaunch_required()`, sticky
  until relaunch — ADR-0019 decision 4: the app is relaunch-required). From
  then on: no in-process launch starts a thread (`InProcessHost::launch` and
  the supervisor loop both refuse); the supervisor that abandoned fails
  `Death::Abandoned` → `WorkerState::Failed` with a reason containing
  `relaunch-required`; every other in-process supervisor stops its healthy
  run at its next ping tick and fails with the latch's reason
  (`Death::RelaunchRequired`). Reasons reach `WorkerManager::failure_reason`
  and `WorkerStatusDto.reason`. The epoch is bumped (fencing). A run that does
  return after its socket closed is a normal death and restarts like the
  sidecar. The sidecar never sets the latch.
- Retirement sends `Shutdown`, closes the socket and waits the retire
  deadline. A launch that is still waiting for a run slot observes retirement
  and starts nothing (`launch` is cancel-safe).
- Run cap (`inprocess.rs`): at most THREE document runs (open tabs; the
  pre-warmed worker is the next tab's and counts among them) and ONE auxiliary
  run (Component Library re-bake or ingest) are live at once — four kernels at
  most. A slot is held until the run thread returns, so retiring and abandoned
  runs keep theirs. A launch waits up to 30 s for a slot, then counts as a
  start failure. The brief's "one in-process instance per process" for
  pre-warm/ephemeral workers is read as: one auxiliary, and the warm worker
  inside the document cap (a single process-wide instance would contradict
  the frozen two-or-three-tab design).
- Logs: the ABI log callback goes to `tracing` target `worker` with the
  instance's epoch. The worker's sink is process-global (the newest live run's
  callback receives every run's lines), so the epoch attributes an instance
  lifetime, not necessarily the producing run. OCCT writes to C++ `cout` are not
  routed (see `onecad_worker.h`).
- `onecad-app` features: `sidecar` (default; `tokio/process`) and
  `embedded-kernel` (`tokio/net`, the link crate). `target_os = "ios"` with
  `sidecar` and iOS without `embedded-kernel` are compile errors. A build with
  neither compiles (a backend-agnostic crate such as `onecad-interaction` on
  its own) and has `WorkerBackend::Unavailable`, which never starts a kernel.
  `ProcessServices::with_backend(WorkerBackend::Embedded(...))` is the explicit
  selection; `ProcessServices::new()` takes the build default. Every production
  worker — document, pre-warm, re-bake and the app's ingest
  (`api::ingest_components`) — starts on the process's backend; only the
  standalone `onecad-library-ingest` tool uses the build default.
- Feature unification: `sidecar` is a DEFAULT feature, so any crate in an iOS
  graph that depends on `onecad-app` with default features turns it on and
  trips the iOS compile error. The iOS graph therefore needs the workspace
  dependency `onecad-app = { path = "crates/onecad-app", default-features =
  false }`, with desktop leaves opting in: `onecad-regen`,
  `onecad-library-ingest` and `onecad-mesh-fixture` already request
  `features = ["sidecar"]`; `onecad-ui` must carry `onecad-app/sidecar` in its
  `desktop` feature and `onecad-interaction` needs it as a dev-dependency (its
  `real_extrude_modes` test spawns the sidecar).
- iOS app-layer gates: recents removal uses `std::fs::remove_file` (no trash),
  `reveal` is a no-op returning `Ok`, and `autosave::pid_alive` uses the
  same-pid rule (one app process; `kill` would answer `EPERM` for a recycled
  foreign pid). Residual: a relaunch that receives the previous launch's pid
  defers that marker's recovery offer to the next launch.

## Readiness and ordinary solver supervision

The manager publishes lifecycle state, worker epoch, connection and validated
hello under one lock. A usable Ready tuple requires the current connection to
remain open and its handshake/session startup to have succeeded. Recovery retires
Ready before dropping the connection or advancing the epoch. No intermediate
tuple is observable; retirement remains terminal and does not advance the epoch.
Handshake plus initial `OpenSession` share the configured ping timeout (5 s in
production). A failed or cancelled startup explicitly stops and awaits its
instance before any replacement; abandonment is terminal. Previous-launch journal
recovery precedes the first new `OpenSession` journal event.
The restart hook remains deferred until the replacement reaches Ready. Requests
and replies pinned to an older connection or epoch cannot publish results.

`GetWorkerHead.inflightJobs` adds at most two active records (`kernel` and
`solver`), each with request identity and its own age/progress clocks. Legacy
`inflight` remains the kernel-only view; absent `inflightJobs` falls back to it.
Ordinary `SketchUpsert` and `SketchRegions` use the existing default supervision
budget (30 s in production), including host dispatch/write/queue delay. An
answered status ping cannot excuse an overdue solver. Expiry reports completion
unknown and signals only the matching generation/connection's supervisor. It does
not retry the accepted request. An unresponsive in-process run is abandoned once
and leaves the process terminal `relaunch-required`, with no replacement thread.

Fault qualification uses `ONECAD_REQUIRE_EMBEDDED_FAULTS=1`; missing linked kernel
or test hooks fails. `__testSolverDelayMs` (maximum 10000 ms) delays an ordinary
Upsert/Regions request only in a hooks build. `__testSolverIgnoreCancel` makes the
delay deliberately non-cooperative for bounded abandonment tests. These are
controlled injections, not physical crash/jetsam evidence.
