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
| `ONECAD_WORKER_PATH` | Mac sidecar used by the independent geometry oracle and Mac round trip |
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
