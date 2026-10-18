# Native macOS packaging

The native host is `onecad-ui`; `onecad-render::Renderer` is its CAD renderer.
The Tauri/React application and Tauri bundler are retired. Producing a native
bundle does not establish production cutover or workflow parity.

Run on macOS with Rust, Bun, CMake and pinned OCCT installed:

```sh
ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" scripts/package-macos.sh
```

An optional argument chooses the output `.app` path. The default is
`target/release/bundle/macos/onecad.app` (respecting Cargo's target directory).
The script does not install or publish the result.

## Integrity order

1. `scripts/build-worker.sh Release` builds and stages the sidecar at
   `packaging/binaries/onecad-worker-<host-triple>`, with a versioned JSON manifest.
2. The script places that worker under `Contents/MacOS/onecad-worker` and runs
   `bundle-dylibs.sh` to copy its non-system dylib closure into `Contents/Frameworks`.
   Install names and signatures change the worker bytes.
3. The rewritten, signed worker is re-staged. Its SHA-256 replaces the manifest's
   binary digest while preserving protocol, worker, solver, quantization and
   OCCT version/fingerprint fields.
4. `cargo build --release -p onecad-ui` embeds this final manifest through
   `onecad-app/build.rs`. The native executable is installed as
   `Contents/MacOS/onecad`; `Info.plist` names it as the bundle entry point.
5. Only the host executable and outer bundle are signed. The script verifies
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
renderer qualification. A complete native bundle launch still needs its real
QA environment and native workflow validation; see `crates/onecad-ui/README.md`.
Do not infer that from `--selftest`, CPU tests or successful packaging alone.

Release source/relink materials are prepared by
`scripts/prepare-release-materials.ts` and verified by
`scripts/verify-release-materials.ts`. Their source inventory includes the
native crates and assets; they do not attest production readiness or publish
anything automatically.
