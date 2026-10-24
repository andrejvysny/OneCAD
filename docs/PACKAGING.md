# Native macOS packaging

The native host is `onecad-ui`; `onecad-render::Renderer` is its CAD renderer.
The Tauri/React application and Tauri bundler are retired. Producing a native
bundle does not establish production cutover or workflow parity.

Run on macOS with Rust, CMake and pinned OCCT installed:

```sh
rustup target add wasm32-unknown-unknown
ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" scripts/package-macos.sh
```

An optional argument chooses the output `.app` path. The default is
`target/release/bundle/macos/onecad.app` (respecting Cargo's target directory).
The script does not install or publish the result.

The bundled executable starts the desktop profile without QA arguments or
environment variables. On macOS it stores application data and configuration in
`~/Library/Application Support/OneCAD/`, caches in `~/Library/Caches/OneCAD/`,
and logs in `~/Library/Logs/OneCAD/`. It uses the OS temporary directory and
holds an exclusive profile lock until all hosts and settings writes finish.
`--open /absolute/path/part.onecad` opens that file in place. Native Save As and
export dialogs may choose other directories. Explicit `--qa-root` launches retain
their temporary-profile confinement and environment requirements; sandboxed
hosts retain their OS-provided container boundaries.

## Minimum macOS version

The first macOS release targets Apple silicon and macOS 26 or later. Packaging
derives `LSMinimumSystemVersion` from the highest arm64 macOS `minos` in the
actual bundled host, worker and dynamic libraries. It inspects packaged Mach-O
code before signing the outer app, refuses absent, malformed or ambiguous build
metadata, non-macOS code and missing arm64 slices, and has no lower-version
override. Unsupported bundle symlinks are refused rather than bypassing inspection.

The current worker and OCCT libraries require macOS 26.0 even when the Rust host
declares a lower build minimum. The bundle therefore declares `26.0.0`; this
does not establish execution qualification on every macOS 26 version. Record the
OS versions actually tested separately. Supporting an older macOS release
requires rebuilding and qualifying the dependencies for that deployment target,
not lowering the bundle declaration. The inspection helper can also check an
existing bundle without changing its files:

```sh
scripts/macos-minimum.sh /absolute/output/OneCAD.app
```

## Signing modes

The default `ONECAD_SIGNING_IDENTITY=-` produces an ad-hoc local bundle. For the
first macOS download release, build on Apple silicon and select an installed
**Developer ID Application** certificate explicitly, by its exact name or SHA-1:

```sh
security find-identity -v -p codesigning
ONECAD_SIGNING_IDENTITY='Developer ID Application: Your Name (TEAMID)' \
  scripts/package-macos.sh /absolute/output/OneCAD.app
```

The signing preflight runs before worker or Cargo builds. It refuses missing,
ambiguous, Apple Development and other non-Developer-ID identities. The private
key must already be available in an unlocked keychain; do not put signing or
notarization secrets in this repository. Intel distribution remains unqualified.
Developer ID mode enables the hardened runtime and secure timestamp on every
signed Mach-O and the outer app. All bundled OCCT dylibs and the worker use that
same identity. The host and outer app receive only the exception in
[`host-entitlements.plist`](../packaging/macos/host-entitlements.plist).

The host needs `com.apple.security.cs.allow-unsigned-executable-memory` for the
pinned Wasmtime 50 Cranelift addon runtime. Its
[allocation path](https://github.com/bytecodealliance/wasmtime/blob/ebe5e8df1f4e12c5fc2f5954926881d8d61367d8/crates/wasmtime/src/runtime/vm/sys/unix/mmap.rs#L46-L53)
creates an ordinary private read/write mapping, and its
[publication path](https://github.com/bytecodealliance/wasmtime/blob/ebe5e8df1f4e12c5fc2f5954926881d8d61367d8/crates/wasmtime/src/runtime/vm/sys/unix/mmap.rs#L140-L184)
changes it to read/execute with `mprotect`, without `MAP_JIT`. Apple's
[unsigned executable memory entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.allow-unsigned-executable-memory)
covers executable memory outside `MAP_JIT`; its
[`allow-jit` entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.allow-jit)
is specific to `MAP_JIT`. The worker has no runtime exception. Library validation
and the other hardened-runtime protections remain enabled. This source-based
choice still requires a real signed-host addon execution smoke test; verify it
again when Wasmtime's code-memory implementation changes.

## Notarization and download artifact

Packaging does not submit anything to Apple. After the signed bundle passes
native startup, addon execution and the required modeling/GPU gates, the release
owner may perform the following steps using their own credentials. Store a
notarytool keychain profile through `xcrun notarytool store-credentials` using
Apple's supported interactive or App Store Connect API-key flow. The commands
below reference only its profile name; credentials stay outside the repository.

```sh
app=/absolute/output/OneCAD.app
submission=/absolute/output/OneCAD-notary.zip
ditto -c -k --sequesterRsrc --keepParent "$app" "$submission"
xcrun notarytool submit "$submission" --keychain-profile OneCAD-notary --wait
```

Submission is an external upload and must be authorized by the release owner.
Record its submission ID and require status **Accepted**. On a failure, inspect
the status and log, fix the cause, and produce a newly signed submission:

```sh
xcrun notarytool info SUBMISSION_ID --keychain-profile OneCAD-notary
xcrun notarytool log SUBMISSION_ID --keychain-profile OneCAD-notary /absolute/output/notary-log.json
```

Only after acceptance, staple and validate the ticket, assess the app, and create
a fresh download ZIP containing the stapled app:

```sh
xcrun stapler staple "$app"
xcrun stapler validate "$app"
codesign --verify --strict --verbose=2 "$app"
spctl --assess --type execute --verbose=2 "$app"
ditto -c -k --sequesterRsrc --keepParent "$app" /absolute/output/OneCAD-macos-arm64.zip
shasum -a 256 /absolute/output/OneCAD-macos-arm64.zip
```

Do not re-sign the worker, host or app after notarization. Stapling adds the ticket
without changing the worker executable or its embedded manifest digest. Test the
downloaded ZIP on a Gatekeeper-enabled Mac, including ordinary startup and addon
execution; successful signing alone is not notarization or release qualification.
These commands follow Apple's
[notarization workflow](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).


## Integrity order

1. `scripts/build-worker.sh Release` builds and stages the sidecar at
   `packaging/binaries/onecad-worker-<host-triple>`, with a versioned JSON manifest.
2. The script places that worker under `Contents/MacOS/onecad-worker` and runs
   `bundle-dylibs.sh` to copy its non-system dylib closure into `Contents/Frameworks`.
   Install names change first; rewritten dylibs are signed before the worker.
   Install names and the selected signing identity change the worker bytes.
3. The rewritten, signed worker is re-staged. Its SHA-256 replaces the manifest's
   binary digest while preserving protocol, worker, solver, quantization and
   OCCT version/fingerprint fields.
4. `cargo build --release -p onecad-ui` embeds this final manifest through
   `onecad-app/build.rs`. The native executable is installed as
   `Contents/MacOS/onecad`; `Info.plist` names it as the bundle entry point.
5. The source addons built by `scripts/build-addons.sh` earlier in the script
   use locked Rust dependencies and validated Wasm components. These fresh
   generated packages are copied to `Contents/Resources/addons/`;
   stale packages from earlier builds are removed. The installed host locates
   these resources relative to its executable. Each bundled addon is installed
   disabled and requires the same package/capability review as a local package.
6. The host executable and outer bundle are signed last, using the same selected
   identity. Their signing never recursively re-signs the worker. The script verifies
   signatures, checks that bundled worker SHA-256 still matches the manifest,
   and executes the bundled worker's self-test.

Do not sign with `codesign --deep`: re-signing a nested worker changes its bytes
and invalidates the embedded digest. Release builds require the staged manifest
and resolve only a sidecar beside the host executable. Worker startup checks its
binary digest and hello compatibility; OCCT identity checks remain mandatory.
Debug builds permit the explicit worker environment override and development
fallback. A debug launch is not proof of release bundle resolution.

## Qualification

CI retains relocated-worker linkage smoke, pinned OCCT fingerprint tests,
cross-version persistence, shared-service backend suites, and required Metal
renderer qualification. A complete native bundle still needs ordinary desktop
startup and native workflow validation, plus isolated QA acceptance where required;
see `crates/onecad-ui/README.md`.
Do not infer that from `--selftest`, CPU tests or successful packaging alone.

Release source/relink materials are prepared by
`cargo xtask prepare-release-materials` and verified by
`cargo xtask verify-release-materials`. Their source inventory includes the
native crates and assets; they do not attest production readiness or publish
anything automatically.
