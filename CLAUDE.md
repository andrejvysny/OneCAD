# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

OneCAD — parametric CAD app. Non-destructive migration of `OneCAD-CPP` (~69k LOC, C++20/Qt6/OCCT) into a 4-layer Tauri stack. `OneCAD-CPP` is a read-only correctness oracle (see `corpus/README.md`), never edited.

Live state: `CURRENT_STATE.md`. Per-work-package tracker + gates: `TODO.md`. Both are current and authoritative — read them before planning work.

## Commands

Package manager is **bun** (`bun.lock`; CI uses `oven-sh/setup-bun`). A stale `package-lock.json` exists — ignore it.

### Frontend
```bash
bun install
bun run dev                 # vite, port 1420 strictPort (Tauri's dev port)
bun run build               # tsc && vite build
bun run test                # vitest run
bunx vitest run src/tools/sketch/snapEngine.test.ts   # single file
bunx vitest run -t "name of test"                     # single test
```

### App (needs the sidecar staged first, see below)
```bash
bun run tauri dev
```

### C++ worker sidecar
```bash
ONECAD_OCCT_ROOT=/path/to/occt-prefix scripts/build-worker.sh Release
                                      # configure + build + STAGE the sidecar
ctest --test-dir worker/build --output-on-failure
ctest --test-dir worker/build -R test_wp6_extrude   # single ctest
```
`build-worker.sh` copies `worker/build/onecad-worker` → `src-tauri/binaries/onecad-worker-<rust-host-triple>`. **`bundle.externalBin` makes Tauri's build script require that staged file to exist**, so staging must precede *any* cargo command that compiles the app crate (clippy and test both do). Build the worker first, always.

Deps: boost, eigen, nlohmann-json (`brew install boost eigen nlohmann-json`). OCCT is **8.0.1**, built from pinned source tag `V8_0_1` with `scripts/build-pinned-occt.sh`. Pass its prefix through `ONECAD_OCCT_ROOT`; CMake requires the selected version exactly and derives runtime paths from the resolved artifact. CI gates 8.0.1; 7.9.3 is comparison-only.

### Rust (run from `src-tauri/`)
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace
cargo test --test m2_gate            # single integration test target
```
`ONECAD_REQUIRE_WORKER=1` turns "no worker binary found" into a hard failure instead of a vacuously green skip — the worker-backed gates (`m2_gate`, `wire_contract`, `topology_rebind`, `real_worker_smoke`, `sketch_*`, `breadth_ops`, `checkpoints`, `worker_chaos`) must actually run. Without the env var, resolution falls back to `../worker/build/onecad-worker` (`worker::DEV_WORKER_PATH`).

### Playwright e2e (mock lane, no Rust/C++)
```bash
bun run e2e
bunx playwright test e2e/line.spec.ts
```
Runs the real Vite app in plain Chromium — no `__TAURI_INTERNALS__`, so `createClient()` falls back to the mock client + `localSolver`. Port defaults to **4177** (`E2E_PORT`), deliberately *not* 1420, because a running `tauri dev` would otherwise be silently tested instead. SwiftShader launch flags are load-bearing: without WebGL2 no `ViewportEngine` exists and no drawing flow runs. `playwright-cli` is installed system-wide.

## Architecture — 4 layers

```
src/            React 19 + Three.js + zustand frontend
  ↕ tauri invoke (DTOs only — the frontend never sees a wire envelope)
src-tauri/src/  Tauri app crate: #[tauri::command] wrappers → DocumentRuntime → WorkerManager
  ↕ trait seams (GeometryEngine / MeshProvider)
src-tauri/crates/onecad-core/   pure domain: document, history, regen, sketch, io. No Tauri.
  ↕ OCW1 frames over stdio (protocol/SCHEMA.md)
worker/         C++20 sidecar: OCCT 8.0.1 kernel + vendored PlaneGCS solver
```

Supporting crates: `onecad-protocol` (OCW1 codec, MESH1 validator, `ProtocolClient`), `onecad-worker-stub` (deterministic fake worker binary for chaos tests), `onecad-regen` (headless replay CLI, runs in CI).

### The protocol is the contract
`protocol/SCHEMA.md` is canonical and normative for **both** tracks. `protocol/mesh_format.md` covers MESH1. Changing either requires cross-track sign-off, a §14 changelog entry, and a fixture bump if shapes move. Key points:
- Transport is stdio, Rust parent → one worker child. **`stdout` carries frames only; every log goes to `stderr`.** No JavaScript on this path, so `u64` as a JSON number is safe.
- Frame = `OCW1` magic + `jsonLen` + `binLen` + JSON envelope + flat binary tail addressed by a `bin` name/off/len table. A malformed frame is fatal — no resync, tear down and restart the worker.
- Fencing is **`workerEpoch` + `expectedBaseHash` only** (decision D4). `documentRevision` is a Rust-owned advisory stamp; fencing on it would reject every legitimate post-edit regen.

### Identity rules (the correctness spine — violating these is a systemic defect)
- **Rust is the sole hash authority.** Planner hash is decoupled from the wire form and golden-pinned.
- **ElementIds are Rust-minted**, globally unique, and do **not** embed a BodyId. `TopoKey` (`"f:22"`) is snapshot-scoped evidence only, promoted on demand.
- **NewBody BodyIds are worker-minted** deterministic `body_<opId>` (decision D1). Rust *adopts* them from `planStep` `bodyEvents` at `AcceptPrepared`, validating format + uniqueness, and rejects the prepared plan on collision — never publishes it. All other body ids stay Rust-minted.
- At the wire layer **every** body-bearing param renders `body_<uuid>` (core serde is frozen). `intent` subtrees round-trip **verbatim** and are never rewritten.
- Region binding: a non-empty `regionId` MUST match or the op fails loudly with the available ids — a stale id must never silently bind the wrong profile. Empty falls back to first-region (V1). Legacy ids are sanitized on load with a migrate diagnostic.
- Resolution ladder: auto-bind requires score ≥0.85 **and** margin ≥0.10; a symmetric tie is `NeedsRepair`, not a guess. A fillet consumes its edge, so re-resolving it must `NeedsRepair` (auto-binding there is a mis-bind).

The whole point of this migration is fixing the legacy stack's unfixed defect (H5-B): parametric edits breaking topological naming. Deterministic `NeedsRepair` beats a silent wrong bind, everywhere.

### Rust app layer
`document_runtime.rs` is the **single writer** and is Tauri-free so it's testable without a webview. `#[tauri::command]` fns in `api/mod.rs` are thin delegating wrappers. Regen runs in three phases so a slow worker never blocks edits: `begin_regen` (locked, compiles plan + clones session) → `PreparedRegen::drive` (**unlocked**, executor runs on the clone) → `finish_regen` (locked, commits only if fencing tokens are unchanged, else reports `Superseded`). The backend sits behind `Arc<dyn GeometryEngine>` + `Arc<dyn MeshProvider>`; `PendingBackend` is the no-worker fallback.

### Frontend seams
- **`src/ipc/client.ts` — `CadClient` is the single seam to the backend.** Two impls: `mockClient` (in-memory, drives the whole UI with no backend) and `tauriClient`. Selected at runtime by presence of `__TAURI_INTERNALS__`. Keep additions to the interface append-only so both evolve together.
- **`src/ipc/localSolver.ts`** — the sketch-solver and drag-preview lanes are *shared by both clients*: the mock runs the identical lane logic over a local identity solve, while the tauri client routes the same calls to the worker's live gesture verbs (SCHEMA §7.4 — `SketchUpsert`/`BeginGesture`/`SolveDrag`/`EndGesture` are fully wired). Only `commit` differs, injected as a dependency. Don't duplicate lane logic into either client.
- **`src/ipc/angleUnits.ts`** — UI angle domain is **degrees**, wire domain is **radians**. Every deg↔rad conversion for a sketch dimension goes through this module; three marshalling sites depend on it agreeing (a past bug).
- Stores are zustand, one per concern (`src/stores/`). Features under `src/features/`, tools/FSMs under `src/tools/`.

### Viewport (`src/viewport/engine/README.md` has the full contract)
- Imperative Three.js class (`ViewportEngine`), **no react-three-fiber**; `ViewportRoot` is a thin React bridge.
- **HARD INVARIANT: world is Z-up, right-handed.** MESH1 vertex buffers are uploaded **verbatim** — the kernel already produces Z-up geometry. Never rotate `scene`/`bodiesRoot`/any root group to "fix" a Y-up look, and never bake an axis swap into ingestion. If something looks rotated the bug is upstream in the camera; rotating content corrupts picking, normals, and saved coordinates.
- Rendering is **on-demand**: `invalidate()` schedules exactly one rAF; idle means zero frames. Anything that must repaint calls it.
- `init()`/`dispose()` are idempotent (React 19 StrictMode double-invokes mount effects).
- `?vpdebug` in the URL exposes `window.__vpEngine` / `window.__vpFrames` — the e2e specs rely on this to raycast real handle positions.

### Styling
`src/styles/tokens.css` is the **sole** source of design colors (Tailwind v4 `@theme`, light values extracted verbatim from the design prototype). No raw hex literal may appear anywhere else — including test/jsdom fallbacks, which is why `palette.ts` falls back to `rgb()` mirrors rather than `#` hex. New color ⇒ new token in `tokens.css`, **with a value in BOTH theme blocks**. The "hex gate" is a grep verified manually at each gate (`grep -rn '#[0-9a-fA-F]\{6\}' src --include='*.ts' --include='*.tsx'` should be empty); it is *not* wired into `ci.yml`.

The file carries both themes: `@theme { … }` is light and the default, `:root[data-theme="dark"] { … }` overrides it. Tailwind v4 compiles color utilities to `var(--color-*)`, so redefining a property under that selector re-themes every utility — there are **no `dark:` variants anywhere**. The prototype has no dark variant, so dark values are authored (same precedent as `--color-body-fill`).

Two traps:
- **`--shadow-*` cannot be overridden per theme.** Tailwind INLINES those values at build time (`.shadow-ctrl{--tw-shadow:0 1px 3px var(--tw-shadow-color,#0000000f)}`), so nothing reads the token at runtime. Dark shadows go through `--tw-shadow-color`, and must use a universal selector because Tailwind registers it `@property{inherits:false}`.
- **The 3D viewport does not follow CSS.** `palette.ts` caches and materials are built once; see `src/viewport/engine/README.md` § Theming for the `refreshColors()` invariant. Adding a viewport color without wiring it there fails silently.

`src/theme/` owns resolution: `themes.ts` is the registry (light/dark/system), `themeController.ts` is the sole writer of `data-theme` on `<html>`. Only the PREFERENCE is persisted; the resolved value is derived at runtime.

## Testing conventions

Four suites, all expected green at a gate: vitest (frontend, colocated `*.test.ts(x)`), `cargo test --workspace` (unit + worker-backed integration under `src-tauri/tests/`), ctest (worker), Playwright (e2e, mock lane). `corpus/` holds characterization fixtures captured from `OneCAD-CPP` at frozen commit `b4ddccc` — every numeric expectation there carries a provenance citation to a binary recording or a source line. Preserve that discipline when adding cases.

## Debugging & logs

`logs/dev.jsonl` (repo root, gitignored) is the CURRENT `tauri dev` / debug-build session ONLY — truncated at every app start, first line always `session.start` (pid/version/filter/dir). Never written by `cargo test`, ctest, vitest, or Playwright. Lane = `target`: a Rust module path (`onecad_lib::…`) · `worker` (forwarded C++ stderr, field `epoch`, **no span context** — join via OCW1 `id` in the frame lane or `epoch`) · `fe` (forwarded webview events, fields `tag,seq,feTs,tMono,ctx`) · `onecad_protocol::frames` (tx/rx per OCW1 frame, debug-gated) · `panic`. Schema, tag taxonomy, and correlation keys: `docs/DEBUGGING.md`.

Knobs: `RUST_LOG` (default `info,onecad_lib=debug,onecad=debug,fe=debug,worker=debug` — the `fe=debug,worker=debug` directives are REQUIRED, a bare `info` silently drops both synthetic lanes); `ONECAD_LOG_DIR` (path, or `off` to kill the file lane; unset = repo `logs/` in debug builds, off in release); `ONECAD_WORKER_LOG` (`error|warn|info|debug`, default `info`, C++ worker's own min level); FE logger gate = `?trace` or a DEV build (never under vitest); `?vpdebug` additionally exposes `window.__vpEngine`/`__extrudePreview` and vpdebug-only `updateDebug` phase snapshots (the `fsm` tag itself is NOT vpdebug-gated).

First-look greps: `grep '"level":"ERROR"' logs/dev.jsonl`, `grep '"target":"worker"'`, `grep 'regen:'` (outcome + failed-step lines), `grep '"tag":"hint"'` (what the user actually saw on screen).

e2e (`e2e/fixtures.ts`): every spec auto-captures browser console + `pageerror`; on a failing test, or any pageerror at all, `test-results/<test>/` gets `console.log`, `pageerror.log`, `fe-logs.json` (the FE ring via `__logsDump()`), plus Playwright's own trace on retry.

White-box surfaces (dev builds only): `window.__logs`/`__logsDump()`/`__logsClear()`, `window.__stores` (12 keys — document/sketch/viewport/tool/settings/selection/measure/app/toolChip/worker/repair/sketchSelection), `window.__vpEngine` (`?vpdebug`), `window.__extrudePreview`.

Policy: never log a drag-frequency path (per-frame preview/pointer/solver ticks) on either side; `ctx`/context payloads are capped, never raw geometry. Full manual — schema, cookbook, failure-signature table, extension policy: `docs/DEBUGGING.md`.

## Knowledge graph (graphify)

`graphify-out/` holds a code knowledge graph (8631 nodes / 23017 edges / 302 communities) built from AST extraction over all four layers plus semantic extraction over the docs. Gitignored — rebuild locally, never commit it.

Use it before grepping:
- Codebase questions → `graphify query "<question>"`. Relationships → `graphify path "<A>" "<B>"`. One concept → `graphify explain "<name>"`. Each returns a scoped subgraph, far smaller than raw grep or `GRAPH_REPORT.md`.
- `graphify-out/GRAPH_REPORT.md` only for broad architecture review, or when query/path/explain come up short.
- `graphify-out/graph.html` is **community-aggregated** (302 nodes), not node-level — the full graph is over the 5000-node viz limit. Node-level detail needs `/graphify . --obsidian`.
- After code changes: `graphify update .` (AST-only, no API cost). After doc changes: `/graphify . --update`.

Blast-radius hint — highest-degree nodes are `EngineError`, `DocumentRuntime`, `ViewportEngine`, `ModelToolController`, `Sketch`, `OperationRecord`, `Envelope`, `Constraint`, `Session`, `ConstraintSolver`. Touching one is a cross-community change; check the layer boundaries in the Architecture section before assuming local scope.

**Limits — do not over-trust the graph:**
- ~1574 edges dangle at un-extracted endpoints (OCCT, STL, Three.js, and other externals). **Absence of an edge is not evidence of no coupling.**
- Extraction is per-language and cannot cross the stdio frame boundary, so Rust↔C++ coupling is largely invisible to it. `protocol/SCHEMA.md` stays the normative contract there — the graph never overrides it.
- The graph describes structure, not intent. `CURRENT_STATE.md`, `TODO.md`, and this file remain authoritative for *why*.

## Working conventions

- Commits happen at gate boundaries only, and `TODO.md` records the gate outcome + any flagged seams. Update it as part of the work, not after.
- `docs/PACKAGING.md` covers bundling; Mac signing/notarization verification is still open and needs a physical Mac.
