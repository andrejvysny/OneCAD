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
  cargo xtask native-host-gate /private/tmp/onecad-native-host-gate
```

The gate sets `ONECAD_REQUIRE_WORKER=1` and `ONECAD_REQUIRE_GPU=1`. It runs two
explicitly filtered selections, each with its own exact inventory declared in
`xtask/src/evidence/host_names.rs`, which is the authoritative list.

The lifecycle selection (`app_logic_tests::`, `HOST_TESTS`) has fifty-five cases
covering occluded delivery (one),
document lifecycle (three), overlays (one), diagnostics (three), existing
dimensions (one), the synthetic From-New plate workflow (one), and numeric/IME
routing (one), invalid-profile Finish closing the sketch and reporting the
overlap (one), and marquee preservation across redraw with camera-change cancellation (one), Point
numeric/repeated construction placement (one), Center Rectangle method/full-size
numeric creation and history (one), and numeric refusal after viewport failure
(one), point/radius/body gesture history and cancellation (three), pre-threshold
camera and Escape ownership (two), stationary-pointer service repaint delivery
(one), and prepared-arc center/endpoint gestures plus accepted-End capture failure
recovery (three), Arc method numeric creation/refusal (one), expression-driven
Slot full-width creation and construction/history (one), clockwise/counterclockwise
tangent Arc H/V snapping with endpoint roles (one), and tangent chain keyboard,
drag, existing-endpoint seeding, cancellation and numeric Radius (one), and
exactly-once drag release with accepted chain closure/profile publication (one).
Nine batched-input regression cases cover ordered modifier delivery,
pointer/camera cancellation and ordinary split-frame clicks.
Further cases cover initially unselected sketch/history row selection,
double-click and explicit Edit, world-plane and checked planar-face entry,
frame alignment and whole-visit discard, viewport world-plane choice, Home
without a document, and restored-control capability context. Ten document tab
cases cover second live tabs, re-preparation on switch, tab close, per-document
crash restore, quit prompts, background close decisions, tab ordering and
Ctrl+Tab cycling, unpreparable-switch fallback, and the new-tab Home and close
prompt.

The service selection (`document_services::tests::home_lifecycle_tests::` and
`bridge::tests::`, `SERVICE_TESTS`) has four cases: three no-document Home
lifecycle cases and the production bridge actor no-document start and explicit
New capture. These cases emit no adapter record, so Metal provenance for the run
comes from the lifecycle selection.

The gate first verifies both Cargo test inventories, then executes each
selection serially with `--ignored --nocapture --test-threads=1`. Every expected
test must execute exactly once and pass. Missing, duplicate, unexpected, skipped,
and zero-test executions fail the gate. New ignored host tests require an
explicit inventory update. Other ignored `onecad-ui` tests outside both filters
are not gate members. The required lifecycle Metal evidence count follows the
lifecycle inventory size; repeated application setup within one test may emit
additional adapter records.
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
