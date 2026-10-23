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

The lifecycle selection (`app_logic_tests::`, `HOST_TESTS`) covers document
lifecycle, recovery, multi-document delivery, display budgets, sketch/model
workflows, input ownership and asynchronous service delivery. Its exact names
and count come from `HOST_TESTS`; do not use an older prose count as the inventory.
New reliability cases must be added there and pass this same strict gate.

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

For save/recovery/reopen and memory-pressure qualification, follow the
[reliability handoff](qa/MANUAL_RELEASE_GATES.md#reliability-acceptance-and-hardware-handoff).
A synthetic call to memory relief does not establish delivery of a native macOS
memory-pressure notification.
