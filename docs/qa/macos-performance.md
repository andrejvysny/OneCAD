# macOS performance evidence

Use a release bundle on the physical target Mac. This procedure collects a baseline;
it does not establish a latency guarantee or an 8 ms result.

## Build and identify

From the exact source tree, record `git rev-parse HEAD`, `git status --short`, and
any dirty patch. Run `scripts/package-macos.sh` and retain its output. The script
builds the release host, stages the required Bambu Studio addon, verifies the
bundled worker against the embedded manifest and runs the worker self-test. Treat
any failure or missing worker/addon as a failed prerequisite; do not substitute a
debug worker. Record the build command, toolchain, OCCT fingerprint and SHA-256
of the final host, bundled worker, manifest and addon component. Keep this
companion record with the trace; session metadata does not contain these hashes.
Verify the packaged addon and worker together, retaining the complete output:

```sh
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$APP/Contents/MacOS/onecad-worker" \
  ONECAD_TEST_ADDON_COMPONENT="$APP/Contents/Resources/addons/dev.onecad.bambu-studio/extension.wasm" \
  cargo test -p onecad-extension-host --test bambu_pipeline -- --nocapture
```

This controlled launcher test does not prove a physical Bambu Studio import or print.

Record Mac model, CPU/GPU, memory, macOS version, power source and power mode,
thermal condition, display model, resolution, scale, refresh rate and viewport
size. Record other significant workloads. Use the same conditions and fixture
hashes for comparisons, and note changes during a run.

## Capture

Use a fresh dedicated temporary QA root, not a personal profile. Prepare its
`temporary` directory and native QA marker as required by `QaPaths`, then launch
the release executable inside the verified bundle with `--qa-root <root>` and,
when using a fixture, `--open <temporary-fixture.onecad>`. Set `TMPDIR` to
`<root>/temporary`, `ONECAD_LOG_DIR` to `<root>/logs`, and optionally
`ONECAD_LIBRARY_ROOT` to `<root>/data/library`. Set `ONECAD_PERF_TRACE=1` in the
process environment. The release host uses the bundled worker. Record the exact
command and fixture SHA-256. Preserve the QA profile and logs alongside evidence.
For example, with the verified bundle path in `APP`:

```sh
PERF_ROOT="$(mktemp -d /private/tmp/onecad-perf.XXXXXX)"
printf 'onecad-native-qa-v1\n' > "$PERF_ROOT/.onecad-native-qa"
mkdir -p "$PERF_ROOT/temporary" "$PERF_ROOT/logs"
TMPDIR="$PERF_ROOT/temporary" ONECAD_LOG_DIR="$PERF_ROOT/logs" \
  ONECAD_PERF_TRACE=1 "$APP/Contents/MacOS/onecad" --qa-root "$PERF_ROOT"
```

Add `--open <temporary-fixture.onecad>` when the workload requires a prepared
fixture. Keep the shell and its `PERF_ROOT` value for evidence collection.

With tracing enabled, the shared recorder writes `session-*.json` and numbered
`trace-*.csv` batches under `<root>/performance`. Without the exact value `1`, no
performance exporter starts and no performance directory is created by tracing.
Export runs periodically; normal host exit requests a nonblocking export. That
request is **not** a disk completion acknowledgement, and the final batch may be
lost on exit. Retain completed CSV files, metadata and logs; do not analyze
`.partial` files as completed batches. Capture enough steady-state data before
exiting and report missing batches, exporter errors and dropped records. Each CSV
header contains sampled loss/export counters; these can race with recording, and
absence of reported loss is not proof that the final buffer was persisted.

The schema and shared stage definitions are in [iPad performance evidence](../ios/PERFORMANCE.md#numeric-schema-1).
Only stages actually emitted by the desktop run apply. Do not infer desktop input
or frame events from iOS-only instrumentation. Preserve raw samples and report
sample counts, warm-up, p50/p95 and failures or cancellations; do not omit failed
runs from the results.

## Workloads and interpretation

Record a reproducible 10-definition, 20-occurrence assembly case: fixture identity,
constraints, actions, number of attempts and successful, failed and cancelled
outcomes. Record a separate 40-step modeling baseline with its exact document,
operation sequence, warm-up and repeated measurements. The existing
`crates/onecad-app/tests/perf_baseline.rs` supplies a 40-step real-worker replay
and early, middle and tail edits; run its `regen_baseline_forty_feature_part` test
with `ONECAD_REQUIRE_WORKER=1` and the verified worker. Record its Rust build
profile separately: its default test profile is not the release desktop host, and
its measurements do not establish interactive latency. Retain raw samples and
logs for both. If either fixture or a relevant timing stage is unavailable, mark
the case unmeasured rather than inventing a result. Compare only matching artifacts,
workloads and host conditions.

CPU spans measure their named code paths. `CadSubmit` is an internal queue
submission span; GPU completion callbacks report completion of queued work, not
physical presentation. `PreviewDisplayed` means renderer acceptance, and
`PreviewDrawSubmitted` means a draw was queued. None measures input-to-visible
latency. Use separately recorded presentation or external observation evidence
for visible outcomes; do not derive it from these events.
