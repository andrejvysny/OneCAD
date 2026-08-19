# Packaging (M3)

The OneCAD app ships as a Tauri bundle that embeds the C++ OCCT sidecar
(`onecad-worker`) alongside the main executable. This document is the end-to-end
packaging story: how the sidecar is built, how Tauri bundles it, how the worker's
non-system dylibs are folded into the `.app` on macOS, and the clean-Mac
verification checklist that must pass before a release is signed.

macOS (Apple Silicon) is the target platform. Linux `deb` bundling is wired and
smoke-tested (see below), but a packaged Linux build would additionally need
OCCT shipped inside the bundle — out of M3 scope.

## Just build it

```bash
scripts/package-macos.sh --install
```

That is the whole procedure. The sections below explain each step, and § 0
explains why the naive one-pass version of them produces an app that refuses its
own worker.

## 0. The two-pass build, and the trap it avoids

**A single-pass build is always broken.** `build-worker.sh` writes a manifest
binding the STAGED sidecar to its SHA-256, and a release build embeds that
manifest and refuses any sidecar whose bytes disagree (`worker/manifest.rs`
`verify_binary`). But `bundle-dylibs.sh`'s entire job is to REWRITE the sidecar —
`install_name_tool` over every dependency, then a re-sign — so after bundling the
bytes cannot still match.

Measured, not theorised. A bundle built by following the old version of this
document launched, retried four times and gave up:

```
worker restarting  reason="start failed: bundled worker SHA-256 mismatch:
                   expected e1c6e1a3…, got 045fe7bd…"
worker failed (no worker)  reason="backoff exhausted after 4 tries: …"
```

The window opens, and there is no geometry backend behind it at all.

The order that works — the one `ci.yml`'s `tauri-composition` job has always
used, and which lived nowhere else until `scripts/package-macos.sh`:

```
scripts/build-worker.sh                  # 1. build + stage sidecar, write manifest
        │
        ▼
bun run tauri build --bundles app        # 2. SEED app
        │
        ▼
scripts/bundle-dylibs.sh <app>           # 3. fold the dylib closure in
        │                                #    ← this rewrites + re-signs the sidecar
        ▼
re-stage the BUNDLED sidecar,            # 4. recompute binarySha256 from the bytes
recompute the manifest hash              #    that will actually ship
        │
        ▼
bun run tauri build --bundles app        # 5. LOCKSTEP app — manifest now matches
        │
        ▼
restore Frameworks/ + codesign --force --sign -   # 6. NOT --deep (see § 4.1)
        │
        ▼
verify: hash equality + worker --selftest
```

## 1. Build + stage the worker

```bash
scripts/build-worker.sh          # Release by default; Debug|Release accepted
```

This configures + builds `worker/` via CMake and copies the result to
`src-tauri/binaries/onecad-worker-<rust-host-triple>` — the exact name Tauri's
`bundle.externalBin` expects (the triple suffix is stripped at install time, so
the bundled binary is plain `onecad-worker`). It also runs that exact binary's
hello, hashes it, and writes `onecad-worker-manifest.json` with SHA-256, protocol
axes, and OCCT fingerprint. Release Cargo builds require and embed that manifest.

> **Run this before any `cargo` command that compiles the app crate**
> (`check`/`clippy`/`test`/`build`): `bundle.externalBin` makes Tauri's build
> script fail hard when the staged sidecar is missing
> (`resource path binaries/onecad-worker-<triple> doesn't exist`). CI stages it
> as its first step for the same reason.

## 2. externalBin bundling

`src-tauri/tauri.conf.json` declares the sidecar:

```json
"bundle": {
  "externalBin": ["binaries/onecad-worker"]
}
```

Tauri resolves `binaries/onecad-worker-<triple>` at bundle time and places the
sidecar next to the main `onecad` executable:

- macOS `.app`: `Contents/MacOS/onecad-worker`
- Linux `.deb`: `/usr/bin/onecad-worker` (beside `/usr/bin/onecad`)
- Windows: `onecad-worker.exe` beside `onecad.exe`

### How the app finds the worker

`src-tauri/src/worker/mod.rs::resolve_worker_path` is build-mode strict:

1. **release builds:** only `<exe_dir>/onecad-worker` (`.exe` on Windows). Env
   overrides and dev-tree fallbacks are ignored;
2. **debug/tests:** `ONECAD_WORKER_PATH`, then the dev-tree build, then the
   executable-adjacent sidecar.

Before a release worker is spawned, the app verifies its SHA-256 against the
embedded manifest. The hello must then match every embedded protocol, worker,
quantization, solver, OCCT-version, and OCCT-fingerprint value before `Ready`.

In debug, the dev-tree build is preferred over the adjacent staged copy.
   Rationale: `tauri dev` copies the staged `src-tauri/binaries/` sidecar beside
   the debug executable, and that staged copy drifts stale silently (it is only
   refreshed by `scripts/build-worker.sh`) — the dev-tree build is the source of
   truth. When the dev build shadows a staged sidecar, a debug-only warning logs
   both paths.

If none exist the app boots with `PendingBackend` rather than spawning a missing
binary. The resolution core (`resolve_worker_path_from`) is a pure function with
unit tests pinning relocated bundles and proving release never selects env/dev.

## 3. macOS dylib bundling

The worker links Homebrew OCCT (`/opt/homebrew/lib`), which is absent on a clean
Mac. `scripts/bundle-dylibs.sh` makes the `.app` self-contained:

```bash
scripts/bundle-dylibs.sh path/to/OneCAD.app
```

It:

1. locates the worker in `Contents/MacOS/` (`onecad-worker` or `onecad-worker-*`);
2. computes the transitive non-system dylib closure via `otool -L`
   (`/usr/lib` + `/System` skipped). An `@rpath/…` install name — which is what
   the pinned source-built OCCT uses — is resolved through the owning binary's
   own `LC_RPATH` entries, so a prefix like `~/.onecad-occt/8.0.1/lib` is
   followed just as a Homebrew path would be;
3. copies each dylib into `Contents/Frameworks/`;
4. rewrites install names to `@rpath/<name>` (`install_name_tool -change` on the
   worker + every copied dylib; `-id @rpath/<name>` on each copied dylib);
5. adds `@executable_path/../Frameworks` to the worker's rpath (tolerating an
   already-present rpath);
6. ad-hoc re-signs (`codesign --force --sign -`) every Mach-O it touched, since
   `install_name_tool` invalidates signatures.

It is macOS-only (refuses to run otherwise) and idempotent — a second run is a
no-op because the worker's deps then resolve through `@rpath` and no longer look
bundleable.

The worker's rpath (`/opt/homebrew/lib` + `@executable_path/../Frameworks`) is
already baked in by `worker/CMakeLists.txt`, so the in-tree dev binary finds
Homebrew OCCT while the bundled binary finds `Contents/Frameworks/`.

## 4. Signing — what this project actually does

**The shipped path today is an AD-HOC signed bundle, and that is a decision, not
an omission.** There is no Apple Developer ID for this project, so there is
nothing to notarize with; recording a Developer ID recipe as if it were the
procedure made the doc read as done when no bundle had ever been installed.

### 4.1 Ad-hoc (the current path)

`bundle-dylibs.sh` already ad-hoc-signs every Mach-O it rewrites. Sign the outer
bundle the same way so the app launches as one coherent signature:

```bash
codesign --force --sign - src-tauri/target/release/bundle/macos/onecad.app
codesign --verify --strict --verbose=2 \
  src-tauri/target/release/bundle/macos/onecad.app     # → "satisfies its Designated Requirement"
```

**Never `--deep` here.** `--deep` re-signs nested Mach-Os, which changes the
sidecar's bytes and re-breaks the embedded manifest exactly as § 0 describes —
that is how the trap was first hit. `bundle-dylibs.sh` has already signed every
binary it touched; the outer signature is the only one still missing.

An ad-hoc signature satisfies `codesign --verify` but **not** Gatekeeper:

```bash
spctl --assess --type execute --verbose src-tauri/target/release/bundle/macos/onecad.app
# → "rejected (the code is valid but does not seem to be an app)"  — EXPECTED
```

So first launch needs one explicit approval, on the machine that built it:

```bash
xattr -dr com.apple.quarantine /Applications/onecad.app   # only if the bundle was zipped/copied
open /Applications/onecad.app                              # or right-click → Open, once
```

This is enough to install the app locally and use it daily. It is **not** enough
to hand the `.app` to anyone else — a bundle copied to another Mac carries the
quarantine bit and will be refused until that machine's owner overrides it.

### 4.2 Developer ID + notarization (follow-up, NOT done)

Prerequisites this project does not currently have: an Apple Developer Program
membership, a "Developer ID Application" certificate in the login keychain, and
an app-specific password (or an API key) for `notarytool`. With those, the
release path becomes:

```bash
codesign --force --deep --options runtime \
  --sign "Developer ID Application: <TEAM>" path/to/onecad.app
xcrun notarytool submit path/to/onecad.dmg \
  --apple-id <APPLE_ID> --team-id <TEAM_ID> --password <APP_PASSWORD> --wait
xcrun stapler staple path/to/onecad.app
```

Tauri can sign during `tauri build` when `APPLE_CERTIFICATE` /
`APPLE_SIGNING_IDENTITY` are set; either way the `bundle-dylibs.sh` re-sign of
the sidecar must happen **before** the outer bundle is signed, because
`install_name_tool` invalidates whatever signature it finds.

Until that membership exists, §5's clean-Mac checklist is unreachable by
construction: its first assertion is that Gatekeeper accepts the bundle, and an
ad-hoc signature never will.

## 5. Clean-Mac verification checklist (deferred, run on a Mac)

This is the M3 gate that cannot be verified on Linux — it must run on a Mac
**without Homebrew** (or with Homebrew's OCCT uninstalled) to prove the bundle is
self-contained. Run each step and expect the stated result:

1. **Bundle + sign on the build Mac**

   ```bash
   scripts/build-worker.sh Release
   bun run tauri build            # produces the .app + .dmg
   scripts/bundle-dylibs.sh src-tauri/target/release/bundle/macos/onecad.app
   # then codesign + notarize per §4
   ```

2. **Copy the signed `.app` to a clean Mac** with no Homebrew on `PATH` and no
   `/opt/homebrew/lib` OCCT. Verify Gatekeeper accepts it:

   ```bash
   spctl --assess --type execute --verbose /Applications/onecad.app   # → accepted
   ```

3. **Run the bundled worker's self-test from inside the `.app`** — this exercises
   `hello` + a PlaneGCS `SketchUpsert` in-process and returns exit 0 only if the
   bundled OCCT/PlaneGCS dylibs load through `@rpath/../Frameworks`:

   ```bash
   /Applications/onecad.app/Contents/MacOS/onecad-worker --selftest
   echo $?    # expect 0
   ```

   A non-zero exit or a dyld "image not found" message means a dylib is missing
   from `Contents/Frameworks/` — re-run `bundle-dylibs.sh` and re-check the
   `otool -L` closure.

4. **Confirm the dylib closure resolves via rpath** (no lingering
   `/opt/homebrew` references):

   ```bash
   otool -L /Applications/onecad.app/Contents/MacOS/onecad-worker \
     | grep -E '/opt/homebrew|/usr/local'    # expect no output
   ```

5. **STEP-export stdout hygiene** — the worker's protocol lane owns `stdout`; any
   OCCT/STEP-writer chatter leaking to `stdout` would corrupt the OCW1 frame
   stream. The worker already guards this: `worker/tests/test_wp6_exportstep.cpp`
   (ctest `wp6_exportstep`) captures fd 1 across `handle_export_step()` and
   asserts **zero bytes** hit the real `stdout` (STEP diagnostics are redirected
   to `stderr` by `main.cpp`'s `redirect_occt_to_stderr()`). On the clean Mac,
   drive an extrude → STEP export through the running app and confirm the export
   succeeds while the frame stream stays intact (protocol frames only on
   `stdout`; any OCCT noise appears on `stderr`).

Until all five pass on a clean Mac, the M3 packaging gate stays open.

## Linux `deb` smoke (CI-friendly, non-gating)

The bundling path is exercised on Linux to prove `externalBin` staging works end
to end:

```bash
scripts/build-worker.sh Release          # stages the linux-gnu sidecar
bun run tauri build --bundles deb        # release compile + .deb under target/release/bundle/deb/
dpkg-deb -c src-tauri/target/release/bundle/deb/*.deb | grep onecad-worker
```

The `.deb` contains `onecad-worker` next to `onecad` in `/usr/bin`. Tauri also
copies the sidecar to `src-tauri/target/release/onecad-worker`; a Linux self-test
needs the conda OCCT on the loader path:

```bash
LD_LIBRARY_PATH=/opt/occt793/lib src-tauri/target/release/onecad-worker --selftest
echo $?    # expect 0
```

A packaged Linux build would need those OCCT libs bundled (as `bundle-dylibs.sh`
does for macOS) — out of M3 scope, since macOS is the target platform.
