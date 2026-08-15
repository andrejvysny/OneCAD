# Repository Guidelines

## Project Overview

OneCAD is a four-layer parametric CAD application: a React 19 + Three.js frontend, a pure-Rust regeneration/recovery core, and a C++20 OCCT worker sidecar. The wire contract is normative — every message is defined once in `protocol/SCHEMA.md` (OCW1 framed stdio protocol) and `protocol/mesh_format.md` (MESH1 binary mesh format), implemented verbatim on both sides, and pinned by cross-track suites. There is **no JS on the kernel path**: the frontend only sees projection DTOs over Tauri IPC, and the worker carries OCW1 frames on stdout only (logs go to stderr).

Read `CURRENT_STATE.md` (current milestone, gate status, decisions D1–D5) and `TODO.md` (reverse-chronological program/gate ledger, not a backlog) before changing code. Both are huge — read heads/sections only. Record gate results and follow-ups in `TODO.md`; preserve unrelated worktree changes.

## Architecture & Data Flow

Strictly decoupled layers (normative: `docs/ARCHITECTURE.md`, plus ADRs in `docs/adr/`):

1. **Frontend** `src/` — React 19 functional components, Zustand 5 vanilla stores, imperative Three.js viewport, Tailwind v4 design tokens. Zustand stores hold backend-authoritative projections; a mock client drives the UI with no backend in dev/e2e.
2. **IPC** — Tauri v2 commands with camelCase projection DTOs. Frontend never sees raw OCW1 envelopes; Rust never sends kernel internals upward.
3. **Rust core** `src-tauri/` — thin `#[tauri::command]` API over a per-document single-writer `DocumentRuntime` (3-phase regen driver: locked begin → unlocked worker drive → locked commit, with fencing tokens and rollback checkpoints). `WorkerManager` supervises the C++ sidecar. Pure domain lives in `src-tauri/crates/`; the root `onecad` crate is a Tauri shell.
4. **Worker** `worker/` — C++20 + OCCT 8.0.1 sidecar. `Session`/`PlanExecutor` handle transactional `ExecutePlan`; one file per kernel op under `src/ops/`. **No JS on this path.**

Data flow per operation: FE invoke → Tauri command → `DocumentRuntime` plan → OCW1 `ExecutePlan` frame → worker regen → `document-changed`/`projection-updated` events → snapshot publish → Tessellate → MESH1 blob (Rust validates header only, forwards bytes verbatim) → `MeshCache` keyed `(BodyId, Lod, generation)` → `ArrayBuffer` → zero-copy TS typed-array views → GPU buffers, **Z-up verbatim, never axis-swapped**.

Platform architecture (in progress): Application Shell → OneCAD Platform (domain-neutral infra: documents/entities/transactions/commands/selection/workspaces/modules/events/resources/settings/persistence) → SDK (`@/platform` internal vs narrower `@onecad/sdk` public) → built-in modules (`onecad.modeling` first) → external addons. Forbidden edges: Platform→modeling impl, Addon→modeling internals, Addon→OCCT worker, module→module private state, SDK→app impl. The C++ worker is the geometry service implementation of `onecad.modeling` — no other module speaks OCW1. All mutation goes through transactions (programmatic == user path); events announce, never perform; deterministic `NeedsRepair` beats silent wrong bind. Fencing is `workerEpoch` + `expectedBaseHash` ONLY; `documentRevision` is advisory.

## Key Directories

| Path | Purpose |
|---|---|
| `src/stores/` | ~19 Zustand stores, one per domain slice (`documentStore`, `appStore`, `settingsStore`, `viewportStore`, `toolStore`, …) |
| `src/ipc/` | `CadClient` seam: `client.ts` (interface + `createClient()`), `tauriClient.ts` (real IPC), `mockClient.ts` (in-memory kernel for dev/e2e), `types.ts` (wire DTOs) |
| `src/viewport/` | Imperative Three.js engine (`ViewportEngine.ts`, `CameraRig.ts`), MESH1 parsing (`mesh/`), engine↔React bridge |
| `src/platform/` | Contribution framework: `registry.ts`, `slots.ts`, `services.ts`, `events.ts`, `shortcuts.ts` |
| `src/modules/` | Feature modules registering into the platform: `modeling/` (register.ts, tools.ts, ids.ts), `library/` |
| `src/features/`, `src/ui/`, `src/tools/` | Screen-level UI, shared primitives, interaction state machines (`SketchController.ts`, `modelToolMachine.ts`) |
| `src/styles/` | `tokens.css` (Tailwind v4 `@theme`; sole hex color source) + `globals.css` |
| `src-tauri/` | Tauri 2 app crate `onecad` (lib `onecad_lib`): `src/api/` (all commands), `src/worker/`, `src/document_runtime.rs`, `src/dto.rs`, `src/error.rs`, `src/events.rs` |
| `src-tauri/crates/` | Workspace members: `onecad-core` (pure domain, **no tauri dep**), `onecad-protocol` (OCW1 framing), `onecad-worker-stub` (chaos drills), `onecad-regen` (headless replay CLI), `onecad-kernelbench`, `onecad-library` (no network/tauri) |
| `worker/` | C++20 OCCT sidecar: `src/ops/` (one file per op), `src/kernel/`, `src/protocol/` (Dispatcher), `src/session/`, `tests/`, `cmake/`, `third_party/` (vendored nlohmann json + PlaneGCS; OCCT is NOT vendored) |
| `protocol/` | `SCHEMA.md` (OCW1, normative, 264KB — read sections), `mesh_format.md` (MESH1), `fixtures/` (executable NDJSON form of the spec) |
| `e2e/` | Playwright mock-client lane (real Vite app, real WebGL, mock backend) |
| `e2e-tauri/` | WebdriverIO lane against the real bundled `.app` (real Tauri IPC + real worker) |
| `corpus/` | Read-only correctness oracle: legacy OneCAD-CPP behavior frozen at commit b4ddcccc, consumed by `corpus_executor.rs` + `scripts/verify-modeling-*.mjs` |
| `scripts/` | Build/stage/verify tooling (see Commands) |
| `docs/` | `ARCHITECTURE.md` (laws), `DEBUGGING.md` (log lanes, env knobs), `PACKAGING.md`, `adr/` (14 records), `qa/` (machine-verified manifests) |

## Development Commands

Use Bun. `package-lock.json` is committed but stale and inconsistent — never touch it; `bun.lock` is the source of truth.

```bash
bun install
bun run dev          # Vite on port 1420 (strictPort — must be free; tauri dev needs it)
bun run build        # tsc && vite build
bun run test         # vitest run (jsdom, colocated src/**/*.{test,spec}.{ts,tsx})
bun run test:watch
bun run e2e          # playwright test (chromium+webkit; port 4177, NOT 1420)
bun run e2e -- --project=chromium | --project=webkit
bunx playwright install chromium webkit
```

**Stage the worker before any cargo command that compiles the app crate** — `bundle.externalBin` and `build.rs` hard-fail without the staged sidecar:

```bash
scripts/build-worker.sh Release   # hygiene gate → cmake build → stage src-tauri/binaries/onecad-worker-<triple> + manifest
ctest --test-dir worker/build --output-on-failure
cd src-tauri
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace
```

`ONECAD_REQUIRE_WORKER=1` makes a missing worker a hard failure (CI); unset locally = quiet skip — **don't trust a green `cargo test` without checking for skips**. Release Rust builds panic without the staged manifest (run `scripts/build-worker.sh Release` first). `tauri build` flags must NOT follow `--` (everything after `--` is forwarded to cargo).

Worker build (OCCT must exist first via `scripts/build-pinned-occt.sh`; env `ONECAD_OCCT_ROOT`):

```bash
cmake -S worker -B worker/build -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DONECAD_OCCT_ROOT=<prefix> -DONECAD_OCCT_VERSION=8.0.1 \
  -DONECAD_OCCT_BUILD_ID=onecad-occt-8.0.1-b8f597c67781-kp1 \
  -DONECAD_WORKER_BUILD_DIR=worker/build     # must equal -B dir
cmake --build worker/build --parallel 3
```

Real-stack e2e (CI job `tauri-composition` only): `bun run tauri build --features tauri-e2e --config src-tauri/tauri.e2e.conf.json --bundles app` then `bun run e2e:tauri`.

QA verifiers (run by CI, fail on drift): `node scripts/verify-modeling-coverage.mjs`, `node scripts/verify-modeling-contracts.mjs`, `scripts/tests/verify-modeling-coverage.test.sh`, `scripts/check-worker-stdout-hygiene.sh`.

## Code Conventions & Common Patterns

### TypeScript / Frontend

- Strict TS; functional components + hooks only; no `any`; no raw hex literals in TS/TSX (hex lives only in `src/styles/tokens.css` + icon SVG masters; `viewport/palette.ts` fallbacks use `rgb()` mirrors — keep in step by hand).
- **Zustand store pattern** (copy `src/stores/appStore.ts`): vanilla `createStore<State>()((set, get) => ({...}))` + typed hook `useXxxStore<T>(selector)` in the same file; export the mechanical pair `xxxStore` / `useXxxStore`. Only `settingsStore` is persisted (`persist`, key `onecad.settings`, version 10 — bump + migrate on reserved-field removal). No `devtools` middleware. Cross-store access via direct imports + `store.getState()` or `.subscribe()`.
- `documentStore` is backend-authoritative: written only by `applySnapshot`/`applyChange`/`applySaveOutcome` from events. Store→engine handoff goes through the module singleton `getViewportEngine()`, never through stores.
- Styling: Tailwind v4 utilities over tokens (`className="bg-panel text-ink-3"`), composed with `cn()`. No CSS modules. Dark theme = `:root[data-theme="dark"]` redefining the same custom properties.
- **IPC lockstep** (drift = bug): `CMD`/`EVT` const maps in `src/ipc/tauriClient.ts` ↔ `src-tauri/src/api/mod.rs` fns ↔ `generate_handler!` in lib.rs; event names ↔ `src-tauri/src/events.rs`; camelCase DTO shapes `ipc/types.ts` ↔ `dto.rs`. Invoke paths are snake_case (Rust fn names), events kebab-case.
- **Errors-as-values**: store actions `set({status:"loading"})` → `await client.x()` → `set({status:"ready"|"error"})`; catch → `logError(...)` + set status, **never rethrow** ("the status is the report"). Rust errors arrive as `ApiError {kind, message, diagnostics?}` — kinds `noDocument | invalidCommand | opFailed | stalePreview | worker | io | internal`.
- Logging: `logError/logWarn/logDebug` (`src/debug/log`), `trace` (`src/debug/trace`); DEV installs an error forwarder to Rust `log_event`. Logger is off under vitest.
- E2E contract: `data-testid` probes (e.g. `data-testid="sketch-dof"`); canvas has no per-entity DOM — assert through store-driven chrome.

### Viewport (imperative Three.js)

- **HARD INVARIANT: Z-up, right-handed.** Both cameras `up = (0,0,1)` (`engine/CameraRig.ts`); grid = world XY at Z=0; MESH1 buffers uploaded verbatim. **Never rotate scene roots or axis-swap buffers** to fake Y-up.
- Render loop is on-demand: `invalidate()` → one `requestAnimationFrame`; zero idle frames. `renderOrder.ts` is the single painter's ladder.
- Every layer reading colors must expose `refreshColors()` and be registered in `ViewportEngine.applyTheme()` (negative-checked by `themeRefresh.test.ts`).
- WebGPU is experimental (`settingsStore.experimentalWebGpu`); three's `lines/*` fat-edge path is WebGL-only.
- StrictMode double-mount: engine `init()`/`dispose()` must stay idempotent.

### Rust

- Clippy-clean with `-D warnings`; `&str` over `String` for params; `#[tracing::instrument]` on arg-identifying commands, never on drag-frequency ones.
- `onecad-core` **must not depend on tauri**; `onecad-library` no tauri + no network I/O. Shared versions in `[workspace.dependencies]`, crates opt in with `{ workspace = true }`.
- `ApiError` conversion is the choke point for `EngineError` → wire.
- Release worker resolution is bundled-only (`<exe_dir>/onecad-worker`); debug uses `ONECAD_WORKER_PATH` → `../worker/build/onecad-worker` → bundled.

### C++ worker

- C++20; `-Wall -Wextra`, no `-Werror`. Vendored code (planegcs) added before warning flags + SYSTEM includes.
- **stdout carries OCW1 frames only** — logs via `WLOG_*` to stderr; OCCT cout printer redirected to stderr (`redirect_occt_to_stderr`). Gated by `scripts/check-worker-stdout-hygiene.sh`.
- **Worker exit codes are protocol signals** — OCCT Draw/Test harness libs are filtered from the link.
- OCCT is checksum-pinned external (8.0.1 primary, 7.9.3 for cross-version persistence), NOT vendored; provenance (build id/version/build dir) mismatch is a CMake FATAL.

### Wire protocol changes

`protocol/SCHEMA.md` is normative. Any change requires: compatible Rust (`onecad-protocol`) + C++ (Dispatcher/ops) updates, NDJSON fixture addition + cross-track sign-off (both `harness_*` in CTest and the Rust harness replay it), and `protocol/fixtures/` stays executable-by-both-lanes. Worker fingerprint `0a6a1dce34181289` must match across macOS/Linux (build id is part of the seed).

## Important Files

- `src/ipc/client.ts` + `tauriClient.ts` + `mockClient.ts` — the single frontend↔backend seam; `createClient()` picks impl on `window.__TAURI_INTERNALS__` (prod builds compile-evict the mock via `evictMockClientInProd`).
- `src/main.tsx`, `src/App.tsx` — entry + start/editor split (DEV exposes `window.__stores`).
- `src/stores/appStore.ts`, `documentStore.ts`, `settingsStore.ts` — store patterns to copy.
- `src/viewport/ViewportRoot.tsx`, `engine/ViewportEngine.ts`, `engine/README.md` — viewport invariants.
- `src/styles/tokens.css` — sole color source; `src/test/setup.ts`, `src/test/renderWithPlatform.tsx`, `src/test/contracts/` — test infra.
- `src-tauri/src/lib.rs` — lifecycle, `generate_handler!`, sidecar prewarm, autosave; `api/mod.rs` — all commands; `worker/mod.rs` — binary resolution; `worker/manager.rs` — supervision/circuit breaker.
- `src-tauri/crates/onecad-protocol/src/framing.rs` — OCW1 framing (magic `OCW1`, 12-byte LE header, no-resync-after-bad-frame policy).
- `protocol/SCHEMA.md`, `protocol/mesh_format.md`, `protocol/fixtures/*.ndjson`.
- `docs/ARCHITECTURE.md` (laws), `docs/DEBUGGING.md` (log lanes + env knobs), `docs/qa/modeling-operation-{contracts,coverage}.json` (machine-verified manifests), `corpus/README.md`.
- `vite.config.ts` (aliases `@`→`src/*`, `@onecad/sdk`→`src/sdk/index.ts`; vitest config; port 1420), `playwright.config.ts` (mock lane), `src-tauri/tauri.conf.json` + `tauri.e2e.conf.json`.

## Runtime/Tooling Preferences

- **Bun required** — package manager and script runner everywhere (CI: `bun install --frozen-lockfile`). `package-lock.json` is stale/inconsistent; never update or commit it.
- Pinned exact: `three` 0.185.1 + `@types/three` 0.185.1; WDIO 9.29.1. Otherwise caret: React 19, Vite 7, Vitest 4, TS ~5.8, Playwright 1.61, zustand 5.
- `tsconfig.json`: strict, `noUnusedLocals/Parameters`, `noFallthroughCasesInSwitch`, `moduleResolution: "bundler"`, `noEmit`; aliases live in both tsconfig paths and vite `resolve.alias`.
- Dev-server port 1420 is Tauri's (strictPort, `TAURI_DEV_HOST` for device testing, HMR ws 1421); Playwright deliberately uses 4177.
- `index.html` no-FOUC theme script intentionally duplicates `settingsStore`'s storage key — keep in sync.
- Recommended VSCode: tauri-vscode + rust-analyzer. `opencode.json`: bash `ask` by default, `rm` denied.

## Testing & QA

Five lanes + the oracle; a coverage manifest is **enforced, not aspirational** (`verify-modeling-coverage.mjs` fails on any cited-but-missing test file, ciJob, or cross-registry mismatch):

| Lane | Runner | What it covers |
|---|---|---|
| Frontend unit | Vitest 4 (jsdom), colocated `*.test.ts(x)` | Stores, controllers, viewport engine, IPC marshalling — against in-memory `mockClient` or `@tauri-apps/api/mocks` `mockIPC` (which exercises the real tauriClient) |
| Playwright | `e2e/`, chromium + webkit | Real Vite app + real WebGL, **mock backend** (`mockClient` + `localSolver`); port 4177; **`retries: 0` is policy** — a red run is a defect, don't re-add retries without a named nondeterminism source |
| Rust integration | `cargo test --workspace` | Real worker over OCW1 via `WorkerManager`/`DocumentRuntime` (`m2_gate.rs`, `corpus_executor.rs`), stub chaos drills (`worker_chaos.rs`), persistence/autosave; `tauri::test::mock_app` for command tests |
| Worker CTest | `ctest --test-dir worker/build` | ~72+ targets: in-process `worker_core` exes, NDJSON harness fixtures (`worker_harness --worker … --fixture …`), `canonical_*` replay of Rust-authored `protocol/fixtures/` verbatim (cross-track interop), `parity_*` gates vs `corpus/expected-values/` |
| WDIO | `e2e-tauri/`, CI `tauri-composition` | Real bundled `.app`: real Tauri IPC + real worker, one composition spec (sketch→extrude→fillet→undo→save/reopen) |

Test conventions: golden contract tests (`*.golden.test.ts(x)`) are literal frozen copies — changing one requires a recorded user-visible decision in `TODO.md`; `*.probe.test.ts` reads production side of a contract; `*.diag.spec.ts`/`zzdebug.spec.ts` are scratch, not gated. e2e specs use `e2e/fixtures.ts` (`test` from there, not raw `@playwright/test`) to attach console/pageerror/`fe-logs.json` on failure. Rust tests use the `real_worker()` helper pattern (env assert, refuse silent skip). Worker prototypes compile with `-UNDEBUG` so asserts stay live in Release.

Determinism gates exist on purpose: edge-op NDJSON `cmp`, `test_executeplan_determinism`, corpus parity with terminator checks (Qt logs contain per-run UUIDs — compare termination + exit code, not bytes).

Current known state (2026-08-15): last e2e run RED — 4 `filletChamfer.spec.ts` failures (locked-type drag bottoming at 2 instead of 0.1) on both browsers; investigate before treating e2e as green. Kernel semantic-publication hardening close-out is the active milestone (branch `kernel/semantic-publication-hardening`, PR #4). CI: GitHub-hosted macOS-14 lanes (frontend, rust-8-0-1, e2e-*, tauri-composition, persistence 7.9.3→8.0.1) + self-hosted Linux (linux-worker, linux-kernelbench) with persistent OCCT/build trees outside the workspace (in-workspace trees get wiped by `git clean -ffdx`).
