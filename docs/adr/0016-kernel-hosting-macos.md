# 0016 — The OCCT worker stays a sidecar process on macOS

- Status: Accepted hosting direction; packaging implementation and verification open. ONECAD-261 remains blocked by ONECAD-256/257/259; no gate completion is claimed.
- Date: 2026-10-17
- Plane: ONECAD-261 (P0.6), program ONECAD-255. Evidence: ONECAD-258 (in-process spike) and ONECAD-256 (licensing).
- Scope: the macOS desktop app. The iPad/iOS target, which was the only reason an in-process host would be required, was dropped from scope by the user on 2026-10-17.

## Context

The kernel is the C++20 worker: OCCT 8.0.1 plus vendored PlaneGCS, speaking
OCW1 over stdio to one Rust parent. ADR-0015 replaces the UI. This record decides
where the worker runs under that UI, and how its LGPL components are shipped.

The Phase 0 spike (`docs/design/native-ui/inprocess-worker-spike.md`, second
revision, with a reported fresh-context review) made the same worker run
in-process. The results below are historical, not rerun for this reconciliation:

- **Entry point:** a C ABI at version 3: `onecad_worker_run` and
  `onecad_worker_run_config`. It is compiled as a new `onecad_worker_lib`
  (STATIC, `EXCLUDE_FROM_ALL`) from `worker_core`'s sources with
  `ONECAD_WORKER_LIBRARY=1`.
- **Driver:** the unchanged `onecad-protocol` `ProtocolClient`, over a
  socketpair.
- **Result:** the same fingerprint, `0a6a1dce34181289`, and byte-identical
  meshes.

What was measured:

| Probe | Sidecar | In-process |
|---|---|---|
| Handshake, median per run (7 runs) | 48.4–66.2 ms | 0.063–0.132 ms |
| ExecutePlan / Tessellate | 1.99–4.55 / 0.32–1.01 ms | 1.96–2.40 / 0.32–0.45 ms (same op cost; the high sidecar values come from one loaded run) |
| Crash in OCCT code | The child dies; the host survives and its in-memory poison circuit holds | **The app dies.** A worker-side crash journal (written synchronously on the kernel thread before each op, keyed `prefixHash\|opId\|fingerprint`) poisons the right op on the next launch. A host-side cursor poisoned the wrong op (`op0`) in the stalled drill. |
| Wedged op (`Debug.Busy`) | Shutdown, then SIGKILL, which always succeeds | Cannot be stopped. `onecad_worker_run` never returns; the thread leaks and burns 1.00 CPU-s per wall-s until relaunch. The outcome is **RelaunchRequired**. A second worker in the same process still works. |
| Two workers doing STEP IO at once | Separate processes, so no shared globals | Heap corruption without a lock: 2 of 6 runs crashed, and the reviewer saw 5 of 7 (first-revision evidence, not re-run). With a bounded process-wide `occt_globals_mutex`: 30 of 30 `concurrent 300` runs passed, and 10 of 10 `concurrent-modeling 1000`. |
| Wedge inside a STEP call | n/a | While a sibling holds the lock, `ExportStep` and `InspectStep` fail as a retriable `OP_FAILED` (`OCCT_GLOBALS_BUSY`) after the bound. Modeling and new worker starts still work. The reviewer found that `ImportStep`, `PlaceComponent` and xcaf `ExportGeometry` return the busy failure as **non-retriable**, so a transient wait fails that plan step. |
| Restart in the same process | n/a | 5 of 5 start → model → Shutdown cycles were clean. |
| Logging | stderr | `Log.h` and the OCCT and OCAF messengers reach `log_cb`, one line per call. PlaneGCS is silenced. OCCT's direct `cout` writes need a bridge that the stdout hygiene gate does not allow under `worker/src`. |

Historical results reported after the spike's worker edits (not rerun for this ADR): ctest 206/206, `onecad-worker --selftest`
OK, and the stdout hygiene gate clean. Every library-only change is under
`#ifdef ONECAD_WORKER_LIBRARY`, with one exception the reviewer found: the
Dispatcher's lane scope guard. On the exception path, the sidecar now joins its
lanes before it terminates.

The spike also found a **sidecar defect that does not depend on this decision**.
PlaneGCS writes redundant-constraint diagnostics to stdout, which is the frame
channel. After about 100 diagnoses those bytes land inside the OCW1 stream and
corrupt it (first revision, reproduced by the reviewer).

## Decision (accepted hosting direction)

1. **The sidecar is the host for production, normal development and normal
   real-worker CI on macOS.** The app spawns
   `onecad-worker` as today, and keeps SIGKILL as the wedge remedy and process
   death as the crash boundary. The supervision code in `manager.rs` (wedge
   detector, fencing, epochs, poison circuit with threshold 3) carries over to
   `onecad-app` through its public application contracts. Preserve the supervision
   semantics; extraction is not presumed to be a mechanical unchanged move.
2. **Preserve the in-process experiment as investigation evidence.** Keep the
   `onecad_worker_lib` target, C ABI, bounded locks and spike material. They
   remain outside required app composition and do not replace the sidecar in
   normal development or real-worker CI. Maintaining or extending an in-process
   `WorkerHost` is not a migration prerequisite.
3. **An in-process host needs separate authorization and verification before
   reliance.** The spike documents app-wide crash loss, unkillable wedged threads
   (`RelaunchRequired`), worker-side crash journals, process-global locks and
   logging constraints. Its unresolved findings include non-retriable busy
   paths, post-loop crash attribution and `SinkScope` construction outside the
   `try`. Preserve these findings; this ADR neither declares them fixed nor
   requires an in-process CI lane or integration to close native UI migration.
4. **Keep PlaneGCS stdout protection independent of hosting.**
   `ConstraintSolver::configureSystem` sets `debugMode = GCS::NoDebug` in every
   build. The real-sidecar `solver_stdout_hygiene` regression checks redundant
   solves and shutdown flushing over OCW1 pipes. The first-party static scan
   remains supplemented by this executable check of vendored solver behavior.
   ONECAD-272 owns this correction, independently of experimental hosting.

### LGPL packaging direction (desktop; decisions approved 2026-10-18)

The user approved D1 (static PlaneGCS with corresponding source/relink materials),
D4 (source or source offer before any distribution, plus an OCCT source mirror)
and D5 (canonical copyright spelling "Andrej Vyšný") on 2026-10-18. The following
retains the recovered licensing investigation as implementation input. Approval
does not prove compliance, packaging qualification or publication; verify the
actual release artifacts, notices, source materials and relink procedure.

From `docs/design/native-ui/licensing.md`, corrected where its reviewer refuted
it:

- **OCCT, LGPL-2.1 with the Open CASCADE exception, as shared libraries.** The
  49 `@rpath/libTK*.8.0.dylib` files are bundled into `Contents/Frameworks/`
  (§6(b)). Users can swap them: the app's SHA-256 pin covers the worker binary
  only, and the OCCT fingerprint that `verify_hello` checks is a compile-time
  constant. A runtime-derived fingerprint would need a licensing review.
  Hardened runtime with library validation, under notarization, would reject
  third-party-signed dylibs. Re-signing the bundle stays a valid path under
  LGPL-2.1; document it.
- **PlaneGCS, LGPL-2.1-or-later, linked statically into `onecad-worker`.** This
  is the approved D1 direction, with corresponding source/relink materials.
  The recovered review identifies §6(d) or §6(c) for distribution rather than
  treating a source link alone as §6(a) compliance:
  - the place each binary is offered from also offers the tagged source of the
    commit it was built from;
  - GitHub release source archives qualify, because PlaneGCS is in-tree with no
    submodules;
  - otherwise, ship a written offer.

  The two-pass `scripts/package-macos.sh` is the documented relink procedure,
  because the app pins the worker's SHA. Switching PlaneGCS to a shared library
  is a one-line CMake change, worth making only if a closed build ever becomes
  possible.
- **Approved release rule (D4; 2026-10-18).** No binary reaches anyone, beta builds included, before
  the repository is public or a source offer is in place. Mirror the OCCT
  `V8_0_1` source tarball with each release. Eigen 5.0.1 and Boost 1.90 come
  from unpinned Homebrew, so record their versions per release.
- **Notices.** `Contents/Resources/licenses/` holds the LGPL-2.1 text, the OCCT
  exception, the OCCT and PlaneGCS copyright lines, and `THIRD_PARTY_NOTICES`.
  The About screen names Open CASCADE Technology (the exception's "prominent
  notice") and PlaneGCS.
- **BOLTS** fastener data (LGPL-2.1+) is compiled into the app binary
  (`onecad-library` tables) as well as the worker. The same tagged-source
  compliance must name the app binary too, or the data must be recorded as
  uncopyrightable facts that are still attributed.
- **Why hosting matters to review.** The sidecar keeps OCCT/PlaneGCS in a
  separate binary. An in-process host changes the linked artifact and would
  require a separate licensing/packaging review. ADR-0015 selects egui; Slint's
  historical licensing comparison does not reopen toolkit selection.

## Consequences

- Crash and hang isolation stay what they are today. The user-visible failure
  model does not change with the UI rewrite.
- Each sidecar spawn keeps its 48–66 ms handshake. That is paid per worker
  start, not per op, and prewarm already hides it for the document worker.
- The preserved library target remains experimental; its historical spike
  results do not establish current required-app or CI acceptance.
- `docs/PACKAGING.md`'s open item still applies: Mac signing and notarization
  are unverified.

## Cost

Choosing the sidecar gives up in-process speed and single-debugger convenience,
which matter only to tests. Keeping the C ABI keeps a second entry point alive
that someone has to keep building.
