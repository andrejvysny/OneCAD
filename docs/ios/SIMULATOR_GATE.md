# iPad simulator gate

`cargo xtask ios-simulator-gate <evidence-dir>` is the reproducible runner and
strict verifier for IP18 (ONECAD-465) and the evidence source for IP21
(ONECAD-468). It produces evidence classes 2–5 of
[ARCHITECTURE.md](ARCHITECTURE.md#qualification-policy). It never establishes
physical-device, Pencil-hardware, performance or release readiness.

## Inputs

| Variable | Meaning |
|---|---|
| `ONECAD_SIM_UDID` | optional explicit simulator; otherwise the gate reuses or creates `OneCAD Gate <device> <runtime>` (never erases other simulators) |
| `ONECAD_SIM_DEVICE_TYPE`, `ONECAD_SIM_RUNTIME` | defaults: newest installed iPadOS runtime, iPad Pro 13-inch (M5) |
| `ONECAD_WORKER_PATH` | Mac sidecar used by the independent geometry oracle and Mac round trip; must match the staged `packaging/binaries/onecad-worker-manifest.json` (hash, OCCT 8.0.1, fingerprint) and pass `--selftest`, as for `native-host-gate` |
| `ONECAD_OCCT_ROOT` | desktop OCCT for that sidecar |

## Steps (each command logged as `<name>.log` + `<name>.command.json`)

1. **Provenance**: `git rev-parse HEAD`, `git status --porcelain=v1` (a dirty
   tree is recorded and makes acceptance fail), `sw_vers`, `xcode-select -p`,
   `xcodebuild -version`, `xcodebuild -showsdks`, `xcrun simctl list runtimes
   -j`, `xcrun simctl list devices available -j`, `rustc -Vv`, `cargo -V`,
   `rustup target list --installed`, `xcodegen --version`.
2. **Run identity**: a fresh run id (`<utc>-<sha8>-<random>`) is passed to
   tests as `TEST_RUNNER_ONECAD_RUN_ID`; every artifact a test writes embeds it.
   Artifacts with another or no run id are stale and rejected.
3. **Kernel bundles**: `scripts/build-ios-worker.sh` for simulator
   (Acceptance: test hooks on) and device (Release: test hooks off); the
   embedded identity JSONs are copied into the evidence.
4. **Simulator selection**: resolve UDID, boot, wait for `bootstatus -b`.
5. **Build for testing** and **test without building** against the same
   products, `-testPlan iPadAcceptance`, explicit `-destination
   'platform=iOS Simulator,id=<udid>'`, `-derivedDataPath <dir>/DerivedData`,
   `-resultBundlePath <dir>/iPadAcceptance.xcresult`.
6. **Result parsing**: `xcrun xcresulttool get test-results tests` and
   `summary`; the executed test identifiers must equal the checked-in
   inventory exactly; zero failures; zero skips unless listed in the approved
   skip list with a reason (empty initially); exactly one run of each test.
7. **Artifacts**: copy the app data container's
   `Documents/acceptance/<run-id>/` (saved `.onecad`, exported STEP, semantic
   JSON, trace records) and export xcresult attachments (screenshots).
8. **Independent geometry oracle (Mac)**: run the ignored Rust test
   `ios_simulator_artifacts` in `onecad-app` with the real Mac sidecar
   (`ONECAD_REQUIRE_WORKER=1`). It opens each simulator-authored `.onecad`,
   regenerates it on macOS and checks volume, bounds, solid validity,
   feature/history and constraint/reference integrity against
   `fixtures/ios/acceptance/expectations.json`. It also writes the
   Mac-authored fixture that the simulator round-trip test opens, and reopens
   the simulator's re-save of that fixture.
9. **Release builds**: unsigned `generic/platform=iOS` Release
   (`CODE_SIGNING_ALLOWED=NO`) and simulator Release. For both products:
   `nm` shows no `onecad_ios_test_` symbol, `strings` shows no worker test-hook
   verb (`Debug.Busy`, `__slow`), `otool -l` shows the expected
   `LC_BUILD_VERSION` platform and `minos 18.0`, `otool -L` shows only system
   libraries and frameworks.
10. **acceptance.json** is written last and only when every check passed.

## Verifier rejects

Missing or extra tests, failed or skipped tests, duplicated executions, a
stale or missing run id, missing expected model artifacts, semantic artifacts
whose kernel identity is not the embedded simulator identity built in step 3
(fake/stub backends), geometry outside the declared tolerance, a dirty source
tree, Release products exporting test hooks, and a device product with the
wrong platform. Each rule has a negative unit test in `xtask`.

## Geometry tolerance

The reference plate (60 x 40 x 4 mm with one 6 mm through hole) has volume
`9600 - 36*pi` mm^3 = 9486.902... mm^3. OCCT `BRepGProp` volume on planar and
cylindrical faces is exact up to floating-point accumulation; the declared
tolerance is `1e-6` relative (about 0.0095 mm^3), with bounds checked to
`1e-7` mm absolute plus `1e-9` relative. Fillet and chamfer cases assert a
strict volume decrease and the analytic expectation for the modified edges
within `1e-4` relative, plus solid validity.

## Runner details

`cargo xtask ios-simulator-gate [--provenance-only | --verify-only] <dir>`,
run from the repository root (`crates/onecad-app` tests, scripts and Xcode
project are resolved there).

- The evidence directory must be new or empty (a used one is refused, so no
  earlier verdict, run id or artifact mixes in) and lie outside the source
  tree or in an ignored path: after step 9 the gate re-reads `git rev-parse
  HEAD` and `git status --porcelain=v1` (`source-final.log`,
  `source-status-final.log`) and rejects a run whose HEAD moved or whose
  tree became dirty while it ran.
- `--provenance-only` runs steps 1–2 and resolves the simulator read-only
  (it never creates or boots one); it builds nothing.
- `--verify-only` re-applies every verifier rule to an existing evidence
  directory (for example a downloaded one) and never writes
  `acceptance.json`. A full run writes `verification.json` with every
  rejection, then `acceptance.json` only when there is none.
- Simulator names drop `-inch` and parentheses: `OneCAD Gate iPad Pro 13 M5
  26.5`. A reused simulator is never erased, but the app is uninstalled
  before `test-without-building` so its data container (crash journal,
  recovery, recents, earlier runs' artifacts) does not carry over.
- Step 3 builds three bundles under `target/ios-worker/` (where
  `platforms/ios/scripts/build-rust.sh` looks): simulator Debug with test
  hooks for the Acceptance products, device Release and simulator Release
  for step 9. Their identity JSONs are copied to `kernel/`.
- Step 8 runs in two halves. The author half runs before step 5:
  `ONECAD_IOS_MAC_FIXTURE_OUT=<dir>/mac-authored` writes
  `mac-plate.onecad`, which reaches the native round-trip test as
  `TEST_RUNNER_ONECAD_MAC_FIXTURE` (`ONECAD_MAC_FIXTURE` inside the test).
  The verify half runs after step 7 with `ONECAD_IOS_ACCEPTANCE_DIR=<dir>/artifacts`
  and `ONECAD_IOS_MAC_FIXTURE`, and compares the re-save (`roundTripOf`)
  with the authored file: same document id, feature records and sketch
  definitions, different bytes. Each half gets only its own inputs and
  writes `oracle-author.json` / `oracle-verify.json`, which the gate
  re-checks against the expectations, the copied documents' hashes and the
  sidecar's OCCT fingerprint instead of trusting `passed`.
- The oracle's sketch rule solves every sketch on macOS (a reopened document
  carries no solve): `OverConstrained` or conflicting is rejected, and a
  case's `sketch` fixes status, DOF and entity/constraint counts.
- Bounds come from the vertices of the oracle's fine MESH1 tessellation: OCW1
  has no exact BRep bounding-box query. On planar extreme faces those are the
  BRep vertices, exact up to their f32 storage, so every bounds and extents
  comparison allows half an f32 ulp of each measured coordinate on top of
  `1e-7` absolute plus `1e-9` relative (about 1.9e-6 mm at 60 mm; a plate
  modelled at 13.7..73.7 carries 2.86e-6 mm on its 60 mm extent).
  Simulator-modelled cases check extents only, X and Y unordered, and only
  Mac-authored cases check absolute bounds.
- A measurement counts only when the regeneration `published` and the
  projection's `geometrySource` is `live` (not the container's cached meshes
  or an empty plan).
- Each required simulator model also needs the `write_semantic` record of
  the same name; its in-process summary (history, one healthy solid, volume,
  bounds, live geometry) is checked against the same case, so the simulator
  kernel's own geometry is verified, not only the Mac regeneration.
- `acceptance.json` claims evidence classes 3–5. Class 2 (synthetic replay)
  is not claimed until replay tests are inventoried and their traces
  required.
- Step 9's `strings` rule is a byte search of the Release executable (the
  product can exceed the 16 MiB log bound). `otool -l` and `otool -L` are
  logged as `otool-load-commands.log` and `otool-libraries.log` (one file on
  a case-insensitive volume otherwise).
- Artifacts follow [TESTING.md](TESTING.md#test-hook-commands-onecad_ios_testh):
  JSON records carry `runId` and `kernelIdentity`; every other file needs a
  `<file>.provenance.json` with a matching SHA-256 (trace records are sealed
  with `seal_artifact`). `kernelIdentity.embedded` and `.linked` must both
  equal `kernel/iphonesimulator-Debug-test-hooks.json`, `backend` must be
  `in-process` and the worker hello must match its axes.
- The checked-in inventory is `platforms/ios/acceptance-inventory.json`
  (`tests`, `approvedSkips` with reasons; empty); the expectations are
  `fixtures/ios/acceptance/expectations.json`, whose unknown keys the oracle
  refuses.
