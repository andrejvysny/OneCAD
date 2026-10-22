---
name: repo-embedded-kernel
description: iPadOS in-process kernel lane (feat/ipad-kernel) — bundle script, env vars, test lanes, and the build/test traps hit while wiring it
metadata:
  type: project
---

Facts for working on the in-process OCCT worker (ADR-0018; procedures in docs/ios/KERNEL.md).

**Why:** each of these cost a failed run to discover; none is obvious from the code.

**How to apply:**

- Bundle: `scripts/build-ios-worker.sh --platform <iphonesimulator|iphoneos|macosx> --config Release [--test-hooks] --out <abs dir>`;
  Rust links it via `ONECAD_EMBEDDED_WORKER_DIR=<out>`. Unset ⇒ onecad-embedded-worker is a stub (NotLinked),
  so workspace clippy/test never need it. Mac in-process drills need the `macosx --test-hooks` bundle (Debug.Busy).
- In-process tests are LIB unit tests: `cargo test -p onecad-app --lib --features embedded-kernel worker::inprocess::tests`
  (+ `ONECAD_REQUIRE_WORKER=1`, `ONECAD_WORKER_PATH` for the sidecar parity case). Not part of default `cargo test`.
- Simulator: build with `IPHONEOS_DEPLOYMENT_TARGET=18.0 ... --target aarch64-apple-ios-sim --no-run`, then
  `SIMCTL_CHILD_<VAR>=... xcrun simctl spawn <udid> <test binary> <filter>` (SIMCTL_CHILD_ prefix passes env).
- `cargo:rustc-link-arg-tests` does NOT reach a lib's unit-test binary; use `cargo:rustc-link-arg` (rpath for desktop OCCT dylibs).
- A build script of a dependency cannot add link args to dependents' binaries — onecad-app/build.rs re-reads the link file.
- `wire::job_id_wire` debug_asserts a JobId is a u64 counter: `JobId(Uuid::new_v4())` in a test panics the plan task.
- onecad-app lib tests cannot build with `--release` (api/mod.rs test helpers are cfg(debug_assertions)); pre-existing.
- CMake `file(STRINGS ... REGEX)` returns `;` escaped as `\;` — a regex ending in `;` never matches the returned line.
- `ld -map` output and archives hold non-UTF-8 bytes: run sed/grep with `LC_ALL=C` or matches silently truncate.
- `libtool -static` embeds member mtimes; `ZERO_AR_DATE=1` makes the bundle SHA reproducible.
- OS thread counts move with parallel tests; assert on `in_process_counters()` (exact) and log the OS delta only.
- Any abandonment sets a STICKY process-wide `relaunch-required` latch (no further in-process launch, every
  in-process supervisor fails). A test that abandons must end with `clear_relaunch_latch(before)` (tests.rs) or every
  later in-process test in that binary fails.
- In-process retire waits the supervisor's LONGEST wedge deadline (180 s in `fast_config`/production) before
  abandoning; a test that retires a busy run must use `wedge_config()` (300 ms) or it sits for minutes.
- Feature unification: `sidecar` is onecad-app's DEFAULT feature, so any `{ workspace = true }` dependent drags it into
  the iOS graph and trips the compile_error; the fix needs the root workspace dep `default-features = false` plus
  desktop opt-ins (onecad-ui `desktop`, onecad-interaction dev-dep). Repro: `cargo check -p onecad-interaction
  --features onecad-app/embedded-kernel --target aarch64-apple-ios-sim`.
- `cargo clippy -p onecad-app --all-targets --target aarch64-apple-ios-sim` fails on integration tests that need the
  sidecar (`WorkerManager::spawn` is sidecar-gated); clippy the iOS lane with `--lib`.
- The bundle is linked `static:-bundle`: a staticlib consumer (Xcode) links `libonecad_worker_bundle.a -lc++` itself;
  `cargo rustc --crate-type staticlib -- --print native-static-libs` confirms the list.
