# Building the iPadOS host

Status: build procedure for the UIKit host in `platforms/ios` (Plane IP04
ONECAD-451, IP18 ONECAD-465). Boundaries live in [ARCHITECTURE.md](ARCHITECTURE.md);
test policy and identifiers in [TESTING.md](TESTING.md). This file asserts no
test or qualification result.

## Prerequisites

- Xcode 26.x with the iOS SDK and an iPadOS 18+ simulator runtime. The project
  was generated and verified with Xcode 26.6 (iOS SDK 26.5).
- XcodeGen 2.44.1 or newer (`brew install xcodegen`). The checked-in project is
  generated output; the spec is `platforms/ios/project.yml`.
- The repository Rust toolchain (`rust-toolchain.toml`) with both iOS targets:

  ```sh
  rustup target add aarch64-apple-ios aarch64-apple-ios-sim
  ```

- Static OCCT 8.0.1 for each destination, built by
  `scripts/build-ios-occt.sh`:

  ```sh
  scripts/build-ios-occt.sh --platform iphonesimulator \
      --prefix "$HOME/.onecad-occt/8.0.1-iphonesimulator" \
      --build-dir "$HOME/.onecad-occt-build/8.0.1-iphonesimulator"
  scripts/build-ios-occt.sh --platform iphoneos \
      --prefix "$HOME/.onecad-occt/8.0.1-iphoneos" \
      --build-dir "$HOME/.onecad-occt-build/8.0.1-iphoneos"
  ```

- The embedded worker bundle (`libonecad_worker_bundle.a`), produced by the
  kernel lane's `scripts/build-ios-worker.sh` into
  `target/ios-worker/<platform>-<Release|Debug>[-test-hooks]`. The Xcode build
  phase validates content provenance through that script on every invocation.
  Missing, stale or altered bundles are rebuilt before Rust compilation.

## Open, build and run

```sh
cd platforms/ios
xcodegen generate          # only after editing project.yml
open OneCAD.xcodeproj
```

Select the shared scheme **OneCADiPad** and an Apple-silicon iPad simulator
(or a connected iPad). Run uses Debug; Test uses Acceptance with
`iPadAcceptance.xctestplan`; Profile and Archive use Release.

Command line, from the repository root:

```sh
# Simulator, Debug
xcodebuild build -project platforms/ios/OneCAD.xcodeproj -scheme OneCADiPad \
  -configuration Debug -destination 'platform=iOS Simulator,id=<UDID>' \
  -derivedDataPath build/DerivedData

# Simulator, Acceptance products for testing
xcodebuild build-for-testing -project platforms/ios/OneCAD.xcodeproj -scheme OneCADiPad \
  -configuration Acceptance -destination 'platform=iOS Simulator,id=<UDID>' \
  -derivedDataPath build/DerivedData

# Generic device, Release, unsigned (compile/link only)
xcodebuild build -project platforms/ios/OneCAD.xcodeproj -scheme OneCADiPad \
  -configuration Release -destination 'generic/platform=iOS' \
  -derivedDataPath build/DerivedData CODE_SIGNING_ALLOWED=NO
```

`build/` is ignored. Concurrent heavy builds should cap parallelism
(`-jobs N`, `CARGO_BUILD_JOBS=N`).

Regenerating the project must be deterministic: `xcodegen generate` run twice
leaves `git status` clean. The test plan references targets by the IDs
XcodeGen derives from target names; renaming a target requires updating
`iPadAcceptance.xctestplan`.

## Configurations

| Configuration | Swift conditions | Rust profile / features | Worker bundle dir | Tests |
|---|---|---|---|---|
| Debug | `DEBUG` | dev, default features | `<platform>-Debug` | UI tests only (native test file compiles empty) |
| Acceptance | `DEBUG ONECAD_ACCEPTANCE` | dev, `onecad-ios/test-hooks` | `<platform>-Debug-test-hooks` | native + UI tests |
| Release | none | `--release`, default features | `<platform>-Release` | none |

The workspace dev profile optimizes `naga` at level 2. Unoptimized WGSL
lowering can overflow the physical iPad's 1 MiB main-thread stack during renderer
startup. Debug and Acceptance inherit this dependency override; application debug
checks remain enabled. Release retains its normal optimized profile. Qualify
startup on the physical device as well as the simulator after changing it.

Acceptance also defines `ONECAD_ACCEPTANCE=1` for the Clang importer so the
bridging headers include `onecad_ios_test.h`, disables dead-code stripping and
forces `_onecad_ios_test_command`/`_onecad_ios_test_string_free` into the app
(`-Wl,-u`) so the hosted `OneCADNativeTests` bundle binds them.

The project builds arm64 only (`ARCHS = arm64`, x86_64 excluded on the
simulator). Swift uses the Swift 6 language mode: the C ABI is
main-thread-only and complete concurrency checking keeps every bridge call on
the main actor.

The application and both test targets explicitly set `TARGETED_DEVICE_FAMILY`
to `2` (iPad). Keep this setting on each target: XcodeGen's target defaults
otherwise override the project-level value with `1,2`. After regenerating,
check `xcodebuild -showBuildSettings` for Debug, Acceptance and Release; the
application must report `TARGETED_DEVICE_FAMILY = 2`, and built application
`Info.plist` files must contain only `2` in `UIDeviceFamily`.

## Build phase contract

The app target's first phase, **Build Rust (onecad-ios)**, runs
`platforms/ios/scripts/build-rust.sh`:

1. Maps `PLATFORM_NAME` to the Rust target: `iphonesimulator` →
   `aarch64-apple-ios-sim`, `iphoneos` → `aarch64-apple-ios`. Any other
   platform, any architecture other than arm64 in `ARCHS`, or an empty
   `ARCHS` (every requested arch excluded, e.g. `ARCHS=x86_64`), fails.
2. Maps `CONFIGURATION`: Release → `--release`; Debug and Acceptance → dev
   profile; Acceptance adds `--features onecad-ios/test-hooks`. Unknown
   configurations fail.
3. Invokes `scripts/build-ios-worker.sh --platform <platform>
   --config <Release|Debug> --out <dir> --reuse-if-valid [--test-hooks]` for
   `target/ios-worker/<platform>-<Release|Debug>[-test-hooks]`. It reuses the
   bundle only when `onecad-worker-provenance.txt` matches current inputs and
   output hashes. Missing provenance, changed inputs or altered outputs trigger
   a fresh CMake configure and clean worker rebuild. Missing SDK/dependency
   inputs fail with a diagnostic; restore those inputs and rerun the same
   builder command. A build whose inputs change while it runs fails without
   publishing reusable provenance.
4. Runs `cargo build --locked -p onecad-ios --target <triple>` from the
   repository root with `CARGO_TARGET_DIR=target/ios-xcode/<platform>-<configuration>`
   (one directory per SDK and configuration) and
   `ONECAD_EMBEDDED_WORKER_DIR` set to the worker directory. `PATH` gains
   `${CARGO_HOME:-~/.cargo}/bin`; `SDKROOT` is removed from cargo's
   environment so host build scripts do not link against the iOS SDK.
5. Checks the built archive with `nm --no-llvm-bc`: Debug and Release fail if
   any `onecad_ios_test_` symbol is exported; Acceptance fails if none is.
6. Copies `libonecad_ios.a` and `libonecad_worker_bundle.a` into
   `$(BUILT_PRODUCTS_DIR)` only when their bytes changed. Both are linked by
   path from the target's link phase, so a changed archive relinks the app and
   an unchanged one does not.

The phase always runs (cargo tracks Rust sources itself) and declares the two
staged archives as outputs. `ENABLE_USER_SCRIPT_SANDBOXING = NO` on the app
target because the phase writes under `target/` and runs cargo. The app links
`-lc++` and the Metal, QuartzCore, UIKit, Foundation and UniformTypeIdentifiers
frameworks; MetalKit is not used.

`ONECAD_IOS_STUB_LIB_DIR=<dir>` (an explicit build-setting override, never set
by the project) is accepted only in **Debug**. Release, Acceptance and unknown
configurations reject it before archive inspection or staging. Debug skips
cargo and the worker and stages `libonecad_ios.a` and
`libonecad_worker_bundle.a` from `<dir>`, with a build warning. It exists only
to compile and link the Swift host without the Rust lanes; such a build proves
nothing about runtime behavior.

## Worker cache provenance

The local provenance sidecar hashes content, not mtimes or Git status. Its input
hash covers worker sources, CMake files, vendored solver/JSON sources, link probe,
protocol fixtures/schemas, Rust protocol and generated iOS ABI headers, worker
build/provenance/OCCT scripts and stdout-hygiene policy. Traversal includes new
files. It also covers complete selected OCCT and Eigen/Boost prefix contents,
including dependency identities, headers, CMake configuration and archive bytes.

Target inputs include platform, arm64, worker configuration, test-hook mode,
deployment target, SDK path/version/build/settings, resolved compiler/linker/libtool
bytes, compiler version, CMake/Ninja paths and versions, and explicit `CXXFLAGS`
and `LDFLAGS`. Dependency discovery uses the selected OCCT root and
`ONECAD_HEADER_PREFIXES` (semicolon-separated Eigen/Boost prefixes); ambient CMake
package roots, environment paths and registries cannot select an unhashed
replacement. Inherited compiler include/library search overrides (`CPATH`,
`CPLUS_INCLUDE_PATH`, `C_INCLUDE_PATH`, `OBJC_INCLUDE_PATH`, `LIBRARY_PATH`,
`COMPILER_PATH`, `GCC_EXEC_PREFIX`), toolchain files and compiler launchers are
not used. Parallelism and output
paths do not change artifact provenance. SDK/dependency content checks require
those installations even when the archive already exists.

Output hashes cover the worker archive, runtime identity manifest, C ABI header,
link directives and platform/linkage audit. The sidecar is published only after
the existing audit succeeds and a second input snapshot matches the first.
An unchanged valid cache retains its archive bytes and mtime. Explicit builder
invocations without `--reuse-if-valid` continue to rebuild. Release workers
reject `--test-hooks`; device/simulator and hook/non-hook outputs remain separate.

Run bounded contract tests without compiling the kernel or invoking an Xcode
build:

```sh
scripts/tests/ios-worker-provenance.test.sh
```

These tests use synthetic build tools to exercise cache reuse, input/output
invalidation, mid-build mutation and stub restrictions. They do not qualify
Mach-O metadata, real worker execution or Simulator modeling; those remain
required real-build gates in [SIMULATOR_GATE.md](SIMULATOR_GATE.md).

## Host conventions the Rust side relies on

Behavior of the UIKit host that `onecad_ios.h` leaves open:

- **Keyboard channels.** The visible focused-field proxy mirrors Rust's complete
  authored draft through `text_snapshot` / `text_apply` v1. Return, keypad Enter,
  Backspace and Tab without ⌘/⌃ use only this protocol, including system repeat;
  their `key_submit` edges are suppressed. Confirm/Tab commit composition and
  emit one fenced egui key after draft reconciliation. Printable input replaces
  the acknowledged selection or marked range. Other keys and ⌘/⌃ chords stay on
  `key_submit`; raw navigation/undo is captured in the next draft snapshot.
  Each UIPress is submitted at most once per edge; key-up follows key-down's
  channel. `repeat` is always 0. Legacy `text_insert` / `text_set_marked` entry
  points remain ABI-compatible but are not used by this UIKit proxy.
- **Host requests** are popped one at a time; the next is popped only after
  the previous is completed, so the backlog stays in Rust's queue. An
  OPEN_DOCUMENT copy that fails shows a native alert
  (`onecad.import.failed`) before the request is completed with the paths
  that did succeed (empty if none); re-importing an unchanged file reuses its
  earlier `Documents/Inbox` copy.
- **Terminal state.** After the first `ERR_PANIC`/`ERR_TERMINAL` the host
  stops calling Rust except `surface_detach`, `lifecycle` (so backgrounding
  still flushes autosave) and `destroy`, and swaps in the relaunch-required
  view on the next main-queue turn. The host keeps the attached
  `CAMetalLayer` alive until `surface_detach` returns OK or `destroy` returns.
- **Keyboard and safe area.** Keyboard frame changes send `keyboard_frame`
  and add the keyboard's overlap below the system safe area to the canvas
  safe area, which produces a `surface_resize`. `view_generation` bumps on
  every size, scale or safe-area change.

## Signing

- Simulator builds sign ad hoc and need no team.
- `CODE_SIGNING_ALLOWED=NO` with `generic/platform=iOS` produces an unsigned
  device build. That is compile/link evidence only (evidence class 5); it
  cannot be installed or run.
- Running on a physical iPad requires selecting a development team in Xcode
  (`DEVELOPMENT_TEAM` is intentionally not checked in). Signed device and
  distribution builds belong to ONECAD-469.
- The iPad app icon is an opaque 1024×1024 PNG in
  `platforms/ios/OneCAD/Assets.xcassets/AppIcon.appiconset`. Its source,
  `platforms/ios/Assets/app-icon-ipad.svg`, derives from the existing macOS brand
  master with the same cube, grid and palette, a full square background and no
  macOS shadow or mask. Regenerate with the repository's existing renderer:

  ```sh
  rsvg-convert -w 1024 -h 1024 platforms/ios/Assets/app-icon-ipad.svg \
    -o platforms/ios/OneCAD/Assets.xcassets/AppIcon.appiconset/AppIcon.png
  ```

  Validate the asset catalog and resulting icon in the signed archive before
  distribution; an unsigned build does not establish distribution readiness.
