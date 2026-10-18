# Debugging the native host

Run the native host through `make run` for isolated data, library and temporary
paths. This launches `onecad-ui`, shared application services and the real OCCT
worker. Native geometry defects require a real reproduction, worker logs and
identification of the owning layer before changing operation code.

## Evidence and logging

The host initializes `onecad_app::logging`. Set `ONECAD_LOG_DIR` to an explicit
writable directory for a controlled run; `make run` uses its QA profile's `logs`
directory. `dev.jsonl` is truncated at session start and records `session.start`
with PID, version, filter and output path. Retain the log before another launch.
The logger also writes formatted stderr. Tests do not initialize the host's
session log unless they explicitly install logging.

Useful targets are application module paths, `worker` for forwarded OCCT stderr,
`onecad_protocol::frames` for opt-in frame tracing, and `panic` for the chained
panic hook. `RUST_LOG` selects tracing verbosity; `ONECAD_WORKER_LOG` selects
worker stderr verbosity. Inspect exact source-level controls in
`crates/onecad-app/src/logging.rs` and `worker/src/util/Log.h`.

Worker stdout carries OCW1 frames only. Never add console diagnostics there.
Run `scripts/check-worker-stdout-hygiene.sh` after worker changes.

## Reproduce and locate

1. Capture the input document, worker build/fingerprint, native command, log
   directory and visible failure.
2. Distinguish application acceptance, regeneration completion, repair/failure,
   scene publication, render submission and host repaint acknowledgement.
3. Inspect complete error context and runtime identities. A command's success
   does not prove that fresh geometry was displayed.
4. Reproduce through a shared-service test when possible. `ONECAD_REQUIRE_WORKER=1`
   prevents a missing worker from quietly passing the test lane.
5. Change the owning layer and rerun the focused regression and native scenario.

Keep preview, manipulator, commit, geometry, repair, undo/redo and save/reopen
outcomes distinct. A redo/repaint acknowledgement defect must be reproduced
before changing its layer. Preserve the last valid scene when fresh geometry or
resource admission fails.

## Worker resolution and packaging

Debug builds use `ONECAD_WORKER_PATH`, then `worker/build/onecad-worker`, then a
worker beside the host executable. Release builds accept only the bundled
sidecar. Binary SHA-256, hello compatibility and OCCT fingerprint mismatches are
failures, not reasons to relax checks. See [packaging](PACKAGING.md) for the
required dylib rewrite, hash and signing order.

## Rendering evidence

Run CPU tests separately from required Metal qualification. The renderer workflow
sets `ONECAD_REQUIRE_GPU=1` and runs ignored GPU tests; unavailable hardware must
fail required qualification. Native host and headless captures use the same
renderer. Fixed-camera captures, reviewed goldens and declared tolerances are
needed for appearance acceptance. Small changed-pixel counts or successful GPU
submission alone do not establish that acceptance.

The retired React/Tauri, browser/mock, comparison and prototype rendering lanes
are no longer executable qualification paths. Historical captures and old logs
are static evidence and must be identified as such.
