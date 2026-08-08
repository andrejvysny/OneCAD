# Repository Guidelines

## Project Structure

OneCAD is a four-layer parametric CAD application. `src/` contains the React 19, Zustand, and Three.js frontend; tests are usually colocated as `*.test.ts(x)`. `src-tauri/` is both the Tauri app and Rust workspace; pure domain crates live in `src-tauri/crates/`. `worker/` is the C++20 OCCT sidecar and its CTest suite. `protocol/SCHEMA.md` and `protocol/mesh_format.md` define the OCW1/MESH1 contract. Playwright specs are in `e2e/`; `corpus/` is a read-only legacy correctness oracle.

Read `CURRENT_STATE.md` and `TODO.md` before changing code. Record gate results and follow-ups in `TODO.md`; preserve unrelated worktree changes.

## Build, Test, and Development

Use Bun; `package-lock.json` is stale. Common frontend commands are:

```bash
bun install
bun run dev       # Vite on port 1420
bun run build     # TypeScript check and production bundle
bun run test      # Vitest
bun run e2e       # Playwright mock-client lane
```

Build and stage the worker before running Cargo commands that compile the app:

```bash
scripts/build-worker.sh Release
ctest --test-dir worker/build --output-on-failure
cd src-tauri
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace
```

The staging step is required because Tauri bundles the worker as an external sidecar. Use `ONECAD_REQUIRE_WORKER=1` for worker-backed gates so a missing binary cannot produce a skipped pass.

## Coding and Contract Rules

Use strict TypeScript, functional React components, and existing naming/style patterns. Keep `src/styles/tokens.css` as the sole color source: do not add raw hex literals to TypeScript/TSX. The viewport is imperative Three.js and permanently Z-up/right-handed; do not rotate root scene groups to compensate for camera issues.

Treat protocol documents as normative. Wire changes require compatible Rust/C++ updates, fixtures, and cross-track sign-off. Worker `stdout` carries frames only; write logs to `stderr`.

## Tests, Commits, and Reviews

Run the smallest relevant suite first, then appropriate frontend, worker, and Rust gates. Add focused regression tests beside changed code; use Playwright for browser flows. CI runs these lanes on macOS.

Use concise imperative commits, commonly `feat(scope): description`, `fix: description`, or `docs: description`. Commit only at a completed gate. Do not auto-commit, push, or pull. PRs should describe behavior, tests, protocol/fixture implications, and screenshots for UI changes; link the relevant issue when available.
