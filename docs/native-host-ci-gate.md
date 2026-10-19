# Automated native host lifecycle gate

`native-renderer.yml` runs separate renderer and native-host jobs on every master
push, pull request, manual dispatch, and weekly schedule. The macOS host job builds
the checksum-pinned OCCT 8.0.1 production worker and stages its compatibility
manifest before compiling the host. The job has a 90-minute timeout, reuses the
pinned OCCT and Rust caches, and limits worker compilation to three jobs.

Run the same host gate locally after building and staging the pinned production
worker with build ID `onecad-occt-8.0.1-b8f597c67781-kp1`:

```sh
ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
  bun scripts/native-host-gate.mjs /private/tmp/onecad-native-host-gate
```

The gate sets `ONECAD_REQUIRE_WORKER=1` and `ONECAD_REQUIRE_GPU=1`. It requires
the exact ignored `app_logic_tests` inventory declared in
`scripts/native-host-gate.mjs`: twenty-five cases covering occluded delivery (one),
document lifecycle (three), overlays (one), diagnostics (three), existing
dimensions (one), the synthetic From-New plate workflow (one), and numeric/IME
routing (one), invalid-profile Finish refusal with repair re-entry (one), and
marquee preservation across redraw with camera-change cancellation (one), Point
numeric/repeated construction placement (one), Center Rectangle method/full-size
numeric creation and history (one), and numeric refusal after viewport failure
(one), point/radius/body gesture history and cancellation (three), pre-threshold
camera and Escape ownership (two), stationary-pointer service repaint delivery
(one), and prepared-arc center/endpoint gestures plus accepted-End capture failure
recovery (three).
It first verifies the Cargo test inventory, then executes all cases
serially with `--ignored --nocapture --test-threads=1`. Every expected test must
execute exactly once and pass. Missing, duplicate, unexpected, skipped, and
zero-test executions fail the gate. New ignored host tests require an explicit
inventory update. The required Metal evidence count follows the inventory size;
repeated application setup within one test may emit additional adapter records.
The shared test setup
requires an explicit existing worker and a Metal adapter. An additional execution
with a missing worker must fail in the selected test with the expected prerequisite
error; compilation failure or zero tests cannot satisfy that negative check.

Evidence contains source revision and dirty status, Rust and Cargo versions,
macOS version, worker SHA-256 and compatibility manifest, pinned OCCT metadata,
worker build/selftest logs, adapter identity, exact commands, test inventory,
positive results, and negative prerequisite results. `acceptance.json` is written
only after all checks pass. CI checks required files are nonempty and uploads the
directory with `if-no-files-found: error`, retaining evidence for 14 days. A local
run does not establish that GitHub accepted an artifact upload.

These are automated host lifecycle tests using eframe test frames, actual shared
application services, production OCCT, and Metal. They are distinct from renderer
acceptance, service-only tests, and physical macOS input qualification. They do
not prove native dialogs, complete viewport workflows, clean AppKit teardown, or
ONECAD-273 checkpoint acceptance. Those physical gates remain separate.
