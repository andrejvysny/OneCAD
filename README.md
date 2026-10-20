# OneCAD

OneCAD is a native Rust CAD application with a C++20 OCCT geometry worker.
`onecad-render::Renderer` is the sole CAD rendering implementation; the native
host and headless captures share it. Application services own document history,
regeneration, persistence, import/export and worker supervision.

The React/Tauri application, Three.js renderers and comparison/prototype
renderers have been retired. This consolidation does not establish production
cutover, appearance acceptance or workflow parity with the retired editor.
Historical screenshots are static evidence only.

## Development

Use the repository-root Cargo workspace. Install pinned OCCT using
`scripts/build-pinned-occt.sh`; then build and stage the worker:

```sh
ONECAD_OCCT_ROOT="$HOME/.onecad-occt/8.0.1" scripts/build-worker.sh Release
make run
```

`make run` creates an isolated QA profile and, when needed, a synthetic constrained
plate through shared application services. Host arguments and launch requirements
are documented in [native host instructions](crates/onecad-ui/README.md).

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" cargo test --workspace
ctest --test-dir worker/build --output-on-failure
```

Required Metal qualification runs the renderer's ignored GPU tests; missing GPU
hardware must fail. CPU tests and successful captures alone do not establish
appearance acceptance. See `.github/workflows/native-renderer.yml`.

## Layout and contracts

Shared application services live in `crates/onecad-app`; backend regression tests
and STEP fixtures are under its `tests` directory. Native UI, interaction, scene
and renderer crates preserve their separate responsibilities. The checked MESH1
protocol preserves Z-up geometry and topology identities. Wire contracts are
normative in [SCHEMA](protocol/SCHEMA.md) and [mesh format](protocol/mesh_format.md).

Theme/icon sources live in `assets/native`; renderer fixtures in `fixtures/render`.
Fixture generation uses the real worker through shared application services:

```sh
cargo run -p onecad-mesh-fixture -- --worker "$PWD/worker/build/onecad-worker" --out "$PWD/fixtures/render"
```

Worker binaries and integrity manifests are staged in ignored
`packaging/binaries`. [Native packaging](docs/PACKAGING.md) preserves binary hash,
hello compatibility and OCCT fingerprint checks.

Repository automation runs through the Rust `xtask` package:

```sh
cargo test -p xtask
cargo xtask check-rust-layers
cargo xtask verify-modeling-coverage
cargo xtask verify-modeling-contracts
```

Project status and durable decisions live in the existing private
[ONECAD Plane project](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/issues/).
Code-coupled documentation stays in Git. See [AGENTS.md](AGENTS.md),
[architecture](docs/ARCHITECTURE.md), [debugging](docs/DEBUGGING.md),
[attribution](ATTRIBUTION.md) and `THIRD_PARTY_NOTICES`.
