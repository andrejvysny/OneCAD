# AGENTS.md — OneCAD-Tauri

High-signal guide for OpenCode sessions. Full architecture and correctness rules live in `CLAUDE.md`; live state and gate tracker in `CURRENT_STATE.md` and `TODO.md`.

## Toolchain

- Use **bun** (`bun.lock`). `package-lock.json` is stale — ignore it.
- Rust workspace root is `src-tauri/`, not repo root. Run cargo commands from there.
- macOS CI uses Homebrew OCCT 7.9.3 (`brew install opencascade boost eigen nlohmann-json`). apt's OCCT 7.6.3 is too old; the Linux dev container uses conda-forge OCCT at `/opt/occt793`.
- No `opencode.json` exists; follow this file plus `CLAUDE.md`.

## Critical build order

1. Build/stage the C++ worker **first**:
   ```bash
   scripts/build-worker.sh Release   # default
   ```
   This copies `worker/build/onecad-worker` → `src-tauri/binaries/onecad-worker-<rust-host-triple>`.
2. Then run frontend or Rust commands. Tauri's `bundle.externalBin` makes the build script require the staged sidecar; **any cargo command that compiles the app crate (clippy, test, build) fails without it**.

## Everyday commands

### Frontend
```bash
bun install
bun run dev          # Vite, port 1420 strictPort
bun run build        # tsc && vite build
bun run test         # vitest run
bunx vitest run src/tools/sketch/snapEngine.test.ts
bunx vitest run -t "test name"
```

### e2e (mock lane only — no Tauri/C++)
```bash
bun run e2e
bunx playwright test e2e/line.spec.ts
```
- Default port is **4177** (`E2E_PORT`), deliberately not 1420.
- SwiftShader launch flags are load-bearing; without WebGL2 the `ViewportEngine` never starts and no drawing flow runs.

### C++ worker
```bash
scripts/build-worker.sh Release
ctest --test-dir worker/build --output-on-failure
ctest --test-dir worker/build -R test_wp6_extrude
```

### Rust (run from `src-tauri/`)
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace
cargo test --test m2_gate
```
- `ONECAD_REQUIRE_WORKER=1` turns a missing worker into a hard failure; use it whenever the worker-backed gates (`m2_gate`, `wire_contract`, `topology_rebind`, `sketch_*`, etc.) must actually run.
- Without it, resolution falls back to `../worker/build/onecad-worker` (`worker::DEV_WORKER_PATH`).

### Headless replay CLI
```bash
cargo run -p onecad-regen -- path/to/doc.onecad
```
Replays a saved container through the same `DocumentRuntime`/`WorkerManager` path as the app.

## Architecture seams

- 4 layers: `src/` React/Three.js frontend → `src-tauri/src/` Tauri command wrappers → `src-tauri/crates/onecad-core/` pure domain → `worker/` C++20 OCCT sidecar. OCW1 frames over stdio; canonical contract is `protocol/SCHEMA.md`.
- `src/ipc/client.ts` (`CadClient`) is the **single backend seam**. `mockClient` runs in browser/vitest/Playwright; `tauriClient` is selected by `__TAURI_INTERNALS__`. Keep additions append-only so both impls stay in sync.
- `src/ipc/localSolver.ts` is shared by both clients for sketch solve and drag preview; only `commit` differs.
- `src/ipc/angleUnits.ts` is the single source for sketch deg↔rad conversions; three marshalling sites depend on it.
- `src/viewport/engine/` is an imperative Three.js engine, no react-three-fiber. Hard invariant: **world is Z-up, right-handed**; never rotate `scene`/`bodiesRoot`/any root group to "fix" camera orientation.
- `src/styles/tokens.css` is the sole color source. No raw hex in `*.ts`/`*.tsx` (including test fallbacks). Verify with:
  ```bash
  grep -rn '#[0-9a-fA-F]\{6\}' src --include='*.ts' --include='*.tsx'
  ```

## Workflow conventions

- Read `CURRENT_STATE.md` and `TODO.md` before planning any change.
- `TODO.md` records gate outcomes; update it as part of the work, not after.
- Commits happen at gate boundaries only.
- `corpus/` is a read-only oracle from `OneCAD-CPP` at frozen commit `b4ddccc`; never edit it.

## Knowledge graph

- `graphify-out/` exists (gitignored). Prefer `graphify query "..."`, `graphify path "A" "B"`, or `graphify explain "Name"` before raw grep. Update after code changes with `graphify update .`.
