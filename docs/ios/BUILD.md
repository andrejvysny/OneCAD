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
  phase runs that script itself when the bundle is missing and the script
  exists; otherwise it stops with an error naming the missing file.

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

Acceptance also defines `ONECAD_ACCEPTANCE=1` for the Clang importer so the
bridging headers include `onecad_ios_test.h`, disables dead-code stripping and
forces `_onecad_ios_test_command`/`_onecad_ios_test_string_free` into the app
(`-Wl,-u`) so the hosted `OneCADNativeTests` bundle binds them.

The project builds arm64 only (`ARCHS = arm64`, x86_64 excluded on the
simulator). Swift uses the Swift 6 language mode: the C ABI is
main-thread-only and complete concurrency checking keeps every bridge call on
the main actor.

## Build phase contract

The app target's first phase, **Build Rust (onecad-ios)**, runs
`platforms/ios/scripts/build-rust.sh`:

1. Maps `PLATFORM_NAME` to the Rust target: `iphonesimulator` →
   `aarch64-apple-ios-sim`, `iphoneos` → `aarch64-apple-ios`. Any other
   platform, or any architecture other than arm64 in `ARCHS`, fails.
2. Maps `CONFIGURATION`: Release → `--release`; Debug and Acceptance → dev
   profile; Acceptance adds `--features onecad-ios/test-hooks`. Unknown
   configurations fail.
3. Ensures `target/ios-worker/<platform>-<Release|Debug>[-test-hooks]/libonecad_worker_bundle.a`
   exists, invoking `scripts/build-ios-worker.sh --platform <platform>
   --config <Release|Debug> --out <dir> [--test-hooks]` when it does not.
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
by the project) skips cargo and the worker and stages `libonecad_ios.a` and
`libonecad_worker_bundle.a` from `<dir>`, with a build warning. It exists only
to compile and link the Swift host without the Rust lanes; such a build proves
nothing about runtime behavior.

## Signing

- Simulator builds sign ad hoc and need no team.
- `CODE_SIGNING_ALLOWED=NO` with `generic/platform=iOS` produces an unsigned
  device build. That is compile/link evidence only (evidence class 5); it
  cannot be installed or run.
- Running on a physical iPad requires selecting a development team in Xcode
  (`DEVELOPMENT_TEAM` is intentionally not checked in). Signed device and
  distribution builds belong to ONECAD-469.
