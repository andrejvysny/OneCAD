# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

OneCAD — a parametric, history-based CAD application. A non-destructive migration of `OneCAD-CPP` (~69k LOC, C++20/Qt6/OCCT) into a four-layer Tauri stack. `OneCAD-CPP` survives only as `corpus/`, a **read-only correctness oracle** frozen at commit `b4ddccc`; it is never edited.

**Authority order.** When two sources disagree, the earlier one wins:

1. `protocol/SCHEMA.md` and `protocol/mesh_format.md` — normative wire contracts for both tracks.
2. `docs/ARCHITECTURE.md` — architecture laws. `docs/adr/` (14 records) says why.
3. `CURRENT_STATE.md` — current milestone, gate status, decisions D1–D5.
4. `TODO.md` — reverse-chronological program and gate ledger. Not a backlog.
5. This file.
6. `OneCAD-modeling-correctness-roadmap/` — reviewed baseline and plan, **not** live-status authority. Within it, `04-live-implementation-delta.md` supersedes the baseline files.

`CURRENT_STATE.md` (~2,000 lines) and `TODO.md` (~8,000 lines) are both huge. Read the head and the named section you need — never the whole file.

---

## Operating mode

You are the **orchestrator**. You design, brief, review, and verify. Implementation goes to subagents; the final gate is re-run by you.

### Scope

Do only what the task asks. Don't add features, refactor, or introduce abstractions beyond what the task requires. A bug fix doesn't need surrounding cleanup and a one-shot operation usually doesn't need a helper. Don't design for hypothetical future requirements. Don't add error handling, fallbacks, or validation for scenarios that cannot happen — trust internal code and framework guarantees, and validate only at system boundaries (user input, external APIs, the OCW1 wire). Don't add feature flags or backwards-compatibility shims when you can change the code.

When the user is describing a problem, asking a question, or thinking out loud rather than requesting a change, the deliverable is your assessment. Report findings and stop. Don't apply a fix until asked.

Before running a command that changes system state, check that the evidence supports that specific action. A signal that pattern-matches to a known failure may have a different cause — this repo has a recorded history of exactly that (see the CI-only failures closed in `ec18de8`, which were a helper init race and a wedged wdio bridge, not the flake they resembled).

### Evidence

**Never report a gate you did not run.** Before reporting progress, audit each claim against a tool result from this session. Only report work you can point to evidence for; if something is not yet verified, say so explicitly. Report outcomes faithfully: if tests fail, say so with the output; if a step was skipped, say that; when something is done and verified, state it plainly without hedging.

Numbers go in reports as measured counts (`ctest 136/136`, `cargo test --workspace 1305 passed / 0 failed`), never as "all green". A subagent's claim that a gate passed is **not** evidence — re-run it yourself on the main thread before it goes in `TODO.md`. This is the house rule that produced the `eba2614` gate record and it is not negotiable.

Two specific traps that make a green run a lie:

- A `cargo test` without `ONECAD_REQUIRE_WORKER=1` silently skips every worker-backed gate. Check for skips.
- A gate run concurrently with another heavy job is invalid for attribution. A loaded run went 448/4 with all four passing in isolation afterwards; that run proved nothing.

### When to pause

Pause for the user only when the work genuinely requires them: a destructive or irreversible action, a real scope change, a gate only a human can run, or input only they can provide. If you hit one, ask and end the turn rather than ending on a promise.

Gates the user must run (record as owed in `TODO.md`, don't fake them): the autosave manual checklist, the merged-stack Tauri smoke, and "does the exported STEP open coloured in another CAD / the 3MF in a slicer".

Before ending a turn, check your last paragraph. If it is a plan, an analysis, a question, a list of next steps, or a promise about work you have not done, do that work now with tool calls.

You have ample context. Do not stop, summarize, or suggest a new session on account of context limits.

### Delegation

Delegate independent subtasks to subagents and keep working while they run. Intervene if a subagent goes off track or is missing relevant context.

- Work packages → `implementer`. Give each one the invariants that bind its layer, not the whole file.
- Risky work packages (identity, protocol, regen, fencing, viewport) → an independent `adversarial-reviewer` on a fresh context. Self-critique is weaker than a fresh reader.
- Gate runs → `gate-verifier`, which reports measured output only.
- Anything touching `protocol/SCHEMA.md` or the OCW1/MESH1 boundary → `protocol-auditor` before the change lands.

Maximum three implementation agents in parallel; every diff is orchestrator-reviewed and every gate orchestrator-re-verified. Subagents cannot review their own work.

Establish a checkpoint interval at the start of a long run and verify against the specification at each one, using a fresh-context verifier rather than re-reading your own reasoning.

### Communication

Terse shorthand is fine between tool calls. The final summary is different: it is for a reader who did not see any of it.

Open with the outcome — one sentence on what happened or what you found. Then supporting detail. Write complete sentences, spell out terms, and give each file, commit, flag, or identifier its own plain-language clause. Drop arrow chains and vocabulary you invented while working. If you have to choose between short and clear, choose clear.

---

## Commands

Package manager is **bun** (`bun.lock`; CI uses `oven-sh/setup-bun`). A stale, inconsistent `package-lock.json` is committed — never update, commit, or use it.

### Frontend

```bash
bun install
bun run dev                 # vite, port 1420 strictPort (Tauri's dev port)
bun run build               # tsc && vite build
bun run test                # vitest run (jsdom, colocated src/**/*.{test,spec}.{ts,tsx})
bun run test:watch
bunx vitest run src/tools/sketch/snapEngine.test.ts   # single file
bunx vitest run -t "name of test"                     # single test
```

### App

```bash
bun run tauri dev           # needs the sidecar staged first
```

### C++ worker sidecar

```bash
ONECAD_OCCT_ROOT=/path/to/occt-prefix scripts/build-worker.sh Release
                            # hygiene gate → configure → build → STAGE the sidecar
ctest --test-dir worker/build --output-on-failure
ctest --test-dir worker/build -R test_wp6_extrude      # single ctest
```

**Stage the worker before *any* cargo command that compiles the app crate** — clippy and test both do. `bundle.externalBin` makes Tauri's build script require the staged file to exist, and release Rust builds panic without the staged manifest. `build-worker.sh` copies `worker/build/onecad-worker` → `src-tauri/binaries/onecad-worker-<rust-host-triple>`.

Deps: boost, eigen, nlohmann-json (`brew install boost eigen nlohmann-json ninja`). OCCT is **8.0.1**, built from pinned source tag `V8_0_1` by `scripts/build-pinned-occt.sh`; pass its prefix as `ONECAD_OCCT_ROOT`. CMake requires the selected version exactly and derives runtime paths from the resolved artifact. CI gates 8.0.1; 7.9.3 is comparison-only (cross-version persistence). The Linux dev container uses conda-forge OCCT 7.9.3 at `/opt/occt793` because apt's 7.6.3 is too old.

Manual worker configure (`-DONECAD_WORKER_BUILD_DIR` must equal `-B`):

```bash
cmake -S worker -B worker/build -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DONECAD_OCCT_ROOT=<prefix> -DONECAD_OCCT_VERSION=8.0.1 \
  -DONECAD_OCCT_BUILD_ID=onecad-occt-8.0.1-b8f597c67781-kp1 \
  -DONECAD_WORKER_BUILD_DIR=worker/build
cmake --build worker/build --parallel 3
```

### Rust (from `src-tauri/`)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker ONECAD_REQUIRE_WORKER=1 cargo test --workspace
cargo test --test m2_gate            # single integration test target
```

`ONECAD_REQUIRE_WORKER=1` turns "no worker binary found" into a hard failure instead of a vacuously green skip. The worker-backed gates (`m2_gate`, `wire_contract`, `topology_rebind`, `real_worker_smoke`, `sketch_*`, `breadth_ops`, `checkpoints`, `worker_chaos`) must actually run. Without the env var, resolution falls back to `../worker/build/onecad-worker` (`worker::DEV_WORKER_PATH`).

### Playwright e2e (mock lane, no Rust/C++)

```bash
bun run e2e                              # chromium + webkit
bun run e2e -- --project=chromium
bunx playwright test e2e/line.spec.ts
bunx playwright install chromium webkit
```

Runs the real Vite app in plain Chromium/WebKit — no `__TAURI_INTERNALS__`, so `createClient()` falls back to the mock client plus `localSolver`. Port defaults to **4177** (`E2E_PORT`), deliberately not 1420, because a running `tauri dev` would otherwise be silently tested instead. SwiftShader launch flags are load-bearing: without WebGL2 there is no `ViewportEngine` and no drawing flow. `playwright-cli` is installed system-wide.

### Real-stack e2e (CI job `tauri-composition`)

```bash
bun run tauri build --features tauri-e2e --config src-tauri/tauri.e2e.conf.json --bundles app
bun run e2e:tauri                        # wdio against the bundled .app
```

`tauri build` flags must **not** follow `--` — everything after `--` is forwarded to cargo.

### QA verifiers (CI-enforced, fail on drift)

```bash
node scripts/verify-modeling-coverage.mjs
node scripts/verify-modeling-contracts.mjs
scripts/tests/verify-modeling-coverage.test.sh
scripts/check-worker-stdout-hygiene.sh
grep -rn '#[0-9a-fA-F]\{6\}' src --include='*.ts' --include='*.tsx'   # hex gate: must be empty
```

The hex gate is verified manually at each gate; it is *not* wired into `ci.yml`.

### Gate ladder

Pick the cheapest rung that actually covers the change. Say which rung you ran.

| Rung | Commands | Use for |
|---|---|---|
| **L0 — touch** | `bunx tsc --noEmit`, or `cargo fmt --all --check` + `cargo clippy --workspace --all-targets -- -D warnings` | Any edit, before anything else |
| **L1 — unit** | L0 + `bun run test` (FE) / `cargo test -p <crate>` (Rust) / `ctest -R <pattern>` (worker) | Single-layer change |
| **L2 — layer** | L1 + the full suite for the layer touched, plus `bunx playwright test <spec>` for a UI-visible change | Work-package completion |
| **L3 — gate** | ctest full · `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` · `bun run test` · `bun run e2e` (both projects, `retries: 0`) · fmt/clippy/hex · QA verifiers | Commit boundary. Costly — e2e alone is ~26 min |
| **L4 — real stack** | L3 + `tauri build` + `bun run e2e:tauri`, plus the user-run manual checks | Release / packaging |

`retries: 0` on the Playwright lane is policy. A red run is a defect. Do not re-add retries without naming the nondeterminism source.

---

## Architecture — four layers

```
src/            React 19 + Three.js + zustand frontend
  ↕ tauri invoke (camelCase projection DTOs — the frontend never sees a wire envelope)
src-tauri/src/  Tauri app crate: #[tauri::command] wrappers → DocumentRuntime → WorkerManager
  ↕ trait seams (GeometryEngine / MeshProvider)
src-tauri/crates/onecad-core/   pure domain: document, history, regen, sketch, io. No Tauri.
  ↕ OCW1 frames over stdio (protocol/SCHEMA.md)
worker/         C++20 sidecar: OCCT 8.0.1 kernel + vendored PlaneGCS solver
```

Supporting crates: `onecad-protocol` (OCW1 codec, MESH1 validator, `ProtocolClient`), `onecad-worker-stub` (deterministic fake worker binary for chaos tests), `onecad-regen` (headless replay CLI, runs in CI), `onecad-kernelbench`, `onecad-library` (no tauri, no network I/O).

Data flow per operation: FE invoke → Tauri command → `DocumentRuntime` plan → OCW1 `ExecutePlan` frame → worker regen → `document-changed` / `projection-updated` events → snapshot publish → Tessellate → MESH1 blob (Rust validates the header only and forwards bytes verbatim) → `MeshCache` keyed `(BodyId, Lod, generation)` → `ArrayBuffer` → zero-copy TS typed-array views → GPU buffers, **Z-up verbatim, never axis-swapped**.

### Key directories

| Path | Purpose |
|---|---|
| `src/stores/` | ~19 Zustand stores, one per domain slice (`documentStore`, `appStore`, `settingsStore`, `viewportStore`, `toolStore`, …) |
| `src/ipc/` | `CadClient` seam: `client.ts` (interface + `createClient()`), `tauriClient.ts`, `mockClient.ts`, `types.ts` (wire DTOs) |
| `src/viewport/` | Imperative Three.js engine (`ViewportEngine.ts`, `CameraRig.ts`), MESH1 parsing (`mesh/`), engine↔React bridge |
| `src/platform/` | Contribution framework: `registry.ts`, `slots.ts`, `services.ts`, `events.ts`, `shortcuts.ts` |
| `src/modules/` | Feature modules registering into the platform: `modeling/`, `library/` |
| `src/features/`, `src/ui/`, `src/tools/` | Screen-level UI, shared primitives, interaction state machines (`SketchController.ts`, `modelToolMachine.ts`) |
| `src/styles/` | `tokens.css` (Tailwind v4 `@theme`; sole hex source) + `globals.css` |
| `src-tauri/` | Tauri 2 app crate `onecad` (lib `onecad_lib`): `src/api/`, `src/worker/`, `document_runtime.rs`, `dto.rs`, `error.rs`, `events.rs` |
| `src-tauri/crates/` | Workspace members listed above |
| `worker/` | C++20 OCCT sidecar: `src/ops/` (one file per op), `src/kernel/`, `src/protocol/` (Dispatcher), `src/session/`, `tests/`, `cmake/`, `third_party/` (vendored nlohmann json + PlaneGCS; OCCT is **not** vendored) |
| `protocol/` | `SCHEMA.md` (OCW1, normative, ~278 KB — read sections), `mesh_format.md` (MESH1), `fixtures/` (executable NDJSON form of the spec) |
| `e2e/` | Playwright mock-client lane |
| `e2e-tauri/` | WebdriverIO lane against the real bundled `.app` |
| `corpus/` | Read-only oracle, frozen at `b4ddccc`; consumed by `corpus_executor.rs` + `scripts/verify-modeling-*.mjs` |
| `docs/` | `ARCHITECTURE.md`, `DEBUGGING.md`, `PACKAGING.md`, `adr/`, `qa/` (machine-verified manifests), postmortems |

---

## Invariants

Violating anything in this section is a systemic defect, not a bug.

### The protocol is the contract

`protocol/SCHEMA.md` is canonical and normative for **both** tracks; `protocol/mesh_format.md` covers MESH1. Changing either requires compatible Rust (`onecad-protocol`) and C++ (Dispatcher/ops) updates, an NDJSON fixture addition, a §14 changelog entry, cross-track sign-off, and a fixture bump if shapes move. `protocol/fixtures/` must stay executable by both lanes. Worker fingerprint `0a6a1dce34181289` must match across macOS and Linux — the build id is part of the seed.

- Transport is stdio, Rust parent → one worker child. **`stdout` carries frames only; every log goes to `stderr`.** No JavaScript on this path, so `u64` as a JSON number is safe.
- Frame = `OCW1` magic + `jsonLen` + `binLen` + JSON envelope + flat binary tail addressed by a `bin` name/off/len table. A malformed frame is fatal — no resync; tear down and restart the worker.
- Fencing is **`workerEpoch` + `expectedBaseHash` only** (decision D4). `documentRevision` is a Rust-owned advisory stamp; fencing on it would reject every legitimate post-edit regen.

### Identity — the correctness spine

- **Rust is the sole hash authority.** The planner hash is decoupled from the wire form and golden-pinned.
- **ElementIds are Rust-minted**, globally unique, and do **not** embed a BodyId. `TopoKey` (`"f:22"`) is snapshot-scoped evidence only, promoted on demand.
- **NewBody BodyIds are worker-minted** deterministic `body_<opId>` (decision D1). Rust *adopts* them from `planStep` `bodyEvents` at `AcceptPrepared`, validating format and uniqueness, and rejects the prepared plan on collision — never publishes it. All other body ids stay Rust-minted.
- At the wire layer **every** body-bearing param renders `body_<uuid>` (core serde is frozen). `intent` subtrees round-trip **verbatim** and are never rewritten.
- Region binding: a non-empty `regionId` MUST match or the op fails loudly with the available ids — a stale id must never silently bind the wrong profile. Empty falls back to first-region (V1). Legacy ids are sanitized on load with a migrate diagnostic.
- Resolution ladder: auto-bind requires score ≥0.85 **and** margin ≥0.10; a symmetric tie is `NeedsRepair`, not a guess. A fillet consumes its edge, so re-resolving it must `NeedsRepair` — auto-binding there is a mis-bind.

Other decisions: D2 STEP export lives in the worker · D3 `primary.topoKey` removed · D5 from-0 plans are always base-valid.

The entire point of this migration is fixing the legacy stack's unfixed defect (H5-B): parametric edits breaking topological naming. **Deterministic `NeedsRepair` beats a silent wrong bind, everywhere.**

### Viewport

Full contract: `src/viewport/engine/README.md`.

- Imperative Three.js class (`ViewportEngine`), **no react-three-fiber**; `ViewportRoot` is a thin React bridge.
- **HARD INVARIANT: world is Z-up, right-handed.** Both cameras have `up = (0,0,1)` (`engine/CameraRig.ts`); the grid is world XY at Z=0; MESH1 vertex buffers are uploaded **verbatim** because the kernel already produces Z-up geometry. Never rotate `scene` / `bodiesRoot` / any root group to "fix" a Y-up look, and never bake an axis swap into ingestion. If something looks rotated the bug is upstream in the camera; rotating content corrupts picking, normals, and saved coordinates.
- Rendering is **on-demand**: `invalidate()` schedules exactly one rAF; idle means zero frames. Anything that must repaint calls it. `renderOrder.ts` is the single painter's ladder.
- Every layer reading colors must expose `refreshColors()` and be registered in `ViewportEngine.applyTheme()` (negative-checked by `themeRefresh.test.ts`).
- `init()` / `dispose()` are idempotent — React 19 StrictMode double-invokes mount effects.
- WebGPU is experimental (`settingsStore.experimentalWebGpu`); three's `lines/*` fat-edge path is WebGL-only.
- `?vpdebug` exposes `window.__vpEngine` / `window.__vpFrames`; the e2e specs use this to raycast real handle positions.

### Styling

`src/styles/tokens.css` is the **sole** source of design colors (Tailwind v4 `@theme`, light values extracted verbatim from the design prototype). No raw hex literal may appear anywhere else — including test/jsdom fallbacks, which is why `palette.ts` falls back to `rgb()` mirrors. A new color means a new token in `tokens.css`, **with a value in BOTH theme blocks**. Hex is also permitted in icon SVG masters.

`@theme { … }` is light and the default; `:root[data-theme="dark"] { … }` overrides it. Tailwind v4 compiles color utilities to `var(--color-*)`, so redefining a property under that selector re-themes every utility — there are **no `dark:` variants anywhere**. The prototype has no dark variant, so dark values are authored (same precedent as `--color-body-fill`).

Two traps:

- **`--shadow-*` cannot be overridden per theme.** Tailwind INLINES those values at build time (`.shadow-ctrl{--tw-shadow:0 1px 3px var(--tw-shadow-color,#0000000f)}`), so nothing reads the token at runtime. Dark shadows go through `--tw-shadow-color` and must use a universal selector, because Tailwind registers it `@property{inherits:false}`.
- **The 3D viewport does not follow CSS.** `palette.ts` caches and materials are built once — see `src/viewport/engine/README.md` § Theming for the `refreshColors()` invariant. Adding a viewport color without wiring it there fails silently.

`src/theme/` owns resolution: `themes.ts` is the registry (light/dark/system), `themeController.ts` is the sole writer of `data-theme` on `<html>`. Only the PREFERENCE is persisted; the resolved value is derived at runtime. `index.html`'s no-FOUC theme script intentionally duplicates `settingsStore`'s storage key — keep them in sync.

### Architecture laws (Platform refactor, in progress)

`docs/ARCHITECTURE.md` is normative; `docs/adr/` records why. The migration is incremental — these bind NEW code even where old code has not moved behind them yet. `TODO.md` tracks which waves have landed.

- **Platform code MUST NOT depend on modeling implementation.** `src/platform/**` may not import `@/features`, `@/tools`, `@/modules`, or a modeling store.
- **New UI SHOULD register through a contribution registry**, not by adding an import to the editor shell. The shell knows slots, not features.
- **Cross-module communication MUST use public services**, never another module's internals.
- **New document state MUST belong to a module namespace**, and unknown module state MUST be preserved verbatim across load/save.
- **Document mutation MUST go through transactions** — there is no programmatic path that skips undo. Events announce; they never perform.
- **Do not expose OCCT worker APIs to addons.** The worker is `onecad.modeling`'s geometry service; no other module speaks OCW1.
- **Do not add addon-defined modeling operations.** `KnownOperation` stays a typed enum; `Operation::Opaque` is forward compatibility, not a plugin execution path.
- **Contribution order is explicit** (`group`, `priority`) and must not depend on module load order.
- **A contribution id must start with its owner's id.** Duplicate ids are a hard failure, never last-one-wins.
- `src/test/contracts/` holds frozen behavior contracts (toolbar, keymap, editor mount order, inspector sections). A refactor may change the probe that reads the production side; it may **not** edit a contract to go green.

Layer stack: Application Shell → OneCAD Platform (domain-neutral infra: documents, entities, transactions, commands, selection, workspaces, modules, events, resources, settings, persistence) → SDK (`@/platform` internal vs the narrower `@onecad/sdk` public) → built-in modules (`onecad.modeling` first) → external addons. Forbidden edges: Platform→modeling impl, Addon→modeling internals, Addon→OCCT worker, module→module private state, SDK→app impl.

---

## Code conventions

### TypeScript / frontend

- Strict TS; functional components and hooks only; no `any`; no raw hex in TS/TSX.
- **Zustand store pattern** (copy `src/stores/appStore.ts`): vanilla `createStore<State>()((set, get) => ({…}))` plus a typed `useXxxStore<T>(selector)` hook in the same file; export the mechanical pair `xxxStore` / `useXxxStore`. Only `settingsStore` is persisted (`persist`, key `onecad.settings`, version 10 — bump and migrate on reserved-field removal). No `devtools` middleware. Cross-store access via direct imports plus `store.getState()` or `.subscribe()`.
- `documentStore` is backend-authoritative: written only by `applySnapshot` / `applyChange` / `applySaveOutcome` from events. Store→engine handoff goes through the module singleton `getViewportEngine()`, never through stores.
- **`src/ipc/client.ts` — `CadClient` is the single seam to the backend.** Two impls: `mockClient` (in-memory, drives the whole UI with no backend) and `tauriClient`, selected at runtime by presence of `__TAURI_INTERNALS__` (prod builds compile-evict the mock via `evictMockClientInProd`). Keep additions to the interface append-only so both evolve together.
- **`src/ipc/localSolver.ts`** — the sketch-solver and drag-preview lanes are *shared by both clients*. The mock runs the identical lane logic over a local identity solve; the tauri client routes the same calls to the worker's live gesture verbs (SCHEMA §7.4: `SketchUpsert` / `BeginGesture` / `SolveDrag` / `EndGesture` are fully wired). Only `commit` differs, injected as a dependency. Don't duplicate lane logic into either client.
- **`src/ipc/angleUnits.ts`** — the UI angle domain is **degrees**, the wire domain is **radians**. Every deg↔rad conversion for a sketch dimension goes through this module; three marshalling sites depend on it agreeing. This was a past bug.
- **IPC lockstep** (drift is a bug): `CMD` / `EVT` const maps in `src/ipc/tauriClient.ts` ↔ `src-tauri/src/api/mod.rs` fns ↔ `generate_handler!` in `lib.rs`; event names ↔ `src-tauri/src/events.rs`; camelCase DTO shapes `ipc/types.ts` ↔ `dto.rs`. Invoke paths are snake_case (Rust fn names); events are kebab-case.
- **Errors-as-values**: store actions do `set({status:"loading"})` → `await client.x()` → `set({status:"ready"|"error"})`; catch → `logError(…)` plus set status, and **never rethrow**. The status is the report. Rust errors arrive as `ApiError {kind, message, diagnostics?}` with kinds `noDocument | invalidCommand | opFailed | stalePreview | worker | io | internal`.
- Styling: Tailwind v4 utilities over tokens (`className="bg-panel text-ink-3"`), composed with `cn()`. No CSS modules.
- Logging: `logError` / `logWarn` / `logDebug` (`src/debug/log`), `trace` (`src/debug/trace`); DEV installs an error forwarder to Rust `log_event`. The logger is off under vitest.
- E2E contract: `data-testid` probes (e.g. `data-testid="sketch-dof"`). The canvas has no per-entity DOM — assert through store-driven chrome.

### Rust

- `document_runtime.rs` is the **single writer** and is Tauri-free, so it is testable without a webview. `#[tauri::command]` fns in `api/mod.rs` are thin delegating wrappers. Regen runs in three phases so a slow worker never blocks edits: `begin_regen` (locked, compiles the plan and clones the session) → `PreparedRegen::drive` (**unlocked**, executor runs on the clone) → `finish_regen` (locked, commits only if fencing tokens are unchanged, else reports `Superseded`). The backend sits behind `Arc<dyn GeometryEngine>` + `Arc<dyn MeshProvider>`; `PendingBackend` is the no-worker fallback.
- Clippy-clean with `-D warnings`; prefer `&str` over `String` for params; `#[tracing::instrument]` on arg-identifying commands, never on drag-frequency ones.
- `onecad-core` **must not depend on tauri**; `onecad-library` has no tauri and no network I/O. Shared versions live in `[workspace.dependencies]`; crates opt in with `{ workspace = true }`.
- `ApiError` conversion is the choke point for `EngineError` → wire.
- Release worker resolution is bundled-only (`<exe_dir>/onecad-worker`); debug uses `ONECAD_WORKER_PATH` → `../worker/build/onecad-worker` → bundled.

### C++ worker

- C++20; `-Wall -Wextra`, no `-Werror`. Vendored code (planegcs) is added before the warning flags and with SYSTEM includes.
- `Session` / `PlanExecutor` handle transactional `ExecutePlan`; one file per kernel op under `src/ops/`.
- **stdout carries OCW1 frames only** — logs go via `WLOG_*` to stderr, and the OCCT cout printer is redirected to stderr (`redirect_occt_to_stderr`). Gated by `scripts/check-worker-stdout-hygiene.sh`.
- **Worker exit codes are protocol signals** — OCCT Draw/Test harness libs are filtered from the link.
- OCCT is a checksum-pinned external (8.0.1 primary, 7.9.3 for cross-version persistence), **not** vendored; a provenance mismatch (build id, version, build dir) is a CMake FATAL.
- Worker prototypes compile with `-UNDEBUG` so asserts stay live in Release.

### Tooling

- Pinned exact: `three` 0.185.1 + `@types/three` 0.185.1; WDIO 9.29.1. Otherwise caret: React 19, Vite 7, Vitest 4, TS ~5.8, Playwright 1.61, zustand 5.
- `tsconfig.json`: strict, `noUnusedLocals` / `noUnusedParameters`, `noFallthroughCasesInSwitch`, `moduleResolution: "bundler"`, `noEmit`. Aliases (`@`→`src/*`, `@onecad/sdk`→`src/sdk/index.ts`) live in both tsconfig paths and vite `resolve.alias`.
- Dev port 1420 is Tauri's (strictPort; `TAURI_DEV_HOST` for device testing, HMR ws 1421). Playwright deliberately uses 4177.

---

## Testing

Five lanes plus the oracle. The coverage manifest is **enforced, not aspirational** — `verify-modeling-coverage.mjs` fails on any cited-but-missing test file, ciJob, or cross-registry mismatch.

| Lane | Runner | What it covers |
|---|---|---|
| Frontend unit | Vitest 4 (jsdom), colocated `*.test.ts(x)` | Stores, controllers, viewport engine, IPC marshalling — against `mockClient` or `@tauri-apps/api/mocks` `mockIPC` (which exercises the real tauriClient) |
| Playwright | `e2e/`, chromium + webkit | Real Vite app, real WebGL, **mock backend**; port 4177; `retries: 0` is policy |
| Rust integration | `cargo test --workspace` | Real worker over OCW1 via `WorkerManager` / `DocumentRuntime` (`m2_gate.rs`, `corpus_executor.rs`), stub chaos drills (`worker_chaos.rs`), persistence/autosave; `tauri::test::mock_app` for command tests |
| Worker CTest | `ctest --test-dir worker/build` | 136+ targets: in-process `worker_core` exes, NDJSON harness fixtures (`worker_harness --worker … --fixture …`), `canonical_*` replay of Rust-authored `protocol/fixtures/` verbatim, `parity_*` gates vs `corpus/expected-values/` |
| WDIO | `e2e-tauri/`, CI `tauri-composition` | Real bundled `.app`: real Tauri IPC and real worker; one composition spec (sketch→extrude→fillet→undo→save/reopen) |

Conventions:

- `*.golden.test.ts(x)` are literal frozen copies — changing one requires a recorded user-visible decision in `TODO.md`.
- `*.probe.test.ts` reads the production side of a contract. `*.diag.spec.ts` / `zzdebug.spec.ts` are scratch, not gated.
- e2e specs import `test` from `e2e/fixtures.ts`, not raw `@playwright/test`, so console, `pageerror`, and `fe-logs.json` attach on failure.
- Rust tests use the `real_worker()` helper pattern: assert the env, refuse a silent skip.
- `corpus/` holds characterization fixtures frozen at `b4ddccc`; every numeric expectation carries a provenance citation to a binary recording or a source line. Preserve that discipline when adding cases.
- Determinism gates exist on purpose: edge-op NDJSON `cmp`, `test_executeplan_determinism`, corpus parity with terminator checks (Qt logs contain per-run UUIDs — compare termination and exit code, not bytes).

CI: GitHub-hosted macOS-14 lanes (frontend, rust-8-0-1, e2e-chromium, e2e-webkit, tauri-composition, occt-fingerprint, worker-8.0.1, persistence 7.9.3→8.0.1) plus self-hosted Linux (linux-worker, linux-kernelbench) with persistent OCCT and build trees kept **outside** the workspace, because in-workspace trees get wiped by `git clean -ffdx`. A CI red is not automatically a repo failure — GitHub-side `codeload` 429/503 at job setup has caused several.

---

## Debugging

`logs/dev.jsonl` (repo root, gitignored) is the CURRENT `tauri dev` / debug-build session ONLY. It is truncated at every app start; the first line is always `session.start` (pid, version, filter, dir). It is never written by `cargo test`, ctest, vitest, or Playwright.

Lane is the `target` field: a Rust module path (`onecad_lib::…`) · `worker` (forwarded C++ stderr, field `epoch`, **no span context** — join via the OCW1 `id` in the frame lane, or by `epoch`) · `fe` (forwarded webview events, fields `tag,seq,feTs,tMono,ctx`) · `onecad_protocol::frames` (tx/rx per OCW1 frame, debug-gated) · `panic`. Schema, tag taxonomy, and correlation keys: `docs/DEBUGGING.md`.

Knobs: `RUST_LOG` (default `info,onecad_lib=debug,onecad=debug,fe=debug,worker=debug` — the `fe=debug,worker=debug` directives are REQUIRED; a bare `info` silently drops both synthetic lanes) · `ONECAD_LOG_DIR` (a path, or `off` to kill the file lane; unset means the repo `logs/` in debug builds and off in release) · `ONECAD_WORKER_LOG` (`error|warn|info|debug`, default `info`) · the FE logger gate is `?trace` or a DEV build, never under vitest · `?vpdebug` additionally exposes `window.__vpEngine` / `__extrudePreview` and vpdebug-only `updateDebug` phase snapshots (the `fsm` tag itself is not vpdebug-gated).

First-look greps: `grep '"level":"ERROR"' logs/dev.jsonl` · `grep '"target":"worker"'` · `grep 'regen:'` (outcome and failed-step lines) · `grep '"tag":"hint"'` (what the user actually saw on screen).

On a failing e2e test, or any pageerror at all, `test-results/<test>/` gets `console.log`, `pageerror.log`, `fe-logs.json` (the FE ring via `__logsDump()`), plus Playwright's trace on retry.

White-box surfaces (dev builds only): `window.__logs` / `__logsDump()` / `__logsClear()`, `window.__stores` (12 keys: document, sketch, viewport, tool, settings, selection, measure, app, toolChip, worker, repair, sketchSelection), `window.__vpEngine` (`?vpdebug`), `window.__extrudePreview`.

Policy: never log a drag-frequency path (per-frame preview, pointer, or solver ticks) on either side. `ctx` payloads are capped and never carry raw geometry.

---

## Knowledge graph (graphify)

`graphify-out/` holds a code knowledge graph (8,631 nodes / 23,017 edges / 302 communities) built from AST extraction over all four layers plus semantic extraction over the docs. Gitignored — rebuild locally, never commit it.

Use it before grepping: codebase questions → `graphify query "<question>"`; relationships → `graphify path "<A>" "<B>"`; one concept → `graphify explain "<n>"`. Each returns a scoped subgraph far smaller than raw grep or `GRAPH_REPORT.md`. Use `GRAPH_REPORT.md` only for broad architecture review or when the three commands come up short. `graphify-out/graph.html` is community-aggregated (302 nodes), not node-level; node-level detail needs `/graphify . --obsidian`. After code changes run `graphify update .` (AST-only, no API cost); after doc changes, `/graphify . --update`.

Blast-radius hint — the highest-degree nodes are `EngineError`, `DocumentRuntime`, `ViewportEngine`, `ModelToolController`, `Sketch`, `OperationRecord`, `Envelope`, `Constraint`, `Session`, `ConstraintSolver`. Touching one is a cross-community change.

**Limits — do not over-trust the graph.** ~1,574 edges dangle at un-extracted endpoints (OCCT, STL, Three.js), so **absence of an edge is not evidence of no coupling**. Extraction is per-language and cannot cross the stdio frame boundary, so Rust↔C++ coupling is largely invisible to it — `protocol/SCHEMA.md` stays normative there and the graph never overrides it. The graph describes structure, not intent; `CURRENT_STATE.md`, `TODO.md`, and this file remain authoritative for *why*.

---

## Working conventions

- Commits happen at gate boundaries only. `TODO.md` records the gate outcome and any flagged seams — update it as part of the work, not after.
- Preserve unrelated worktree changes.
- `docs/PACKAGING.md` covers bundling. Mac signing and notarization verification is still open and needs a physical Mac.
- Known gaps worth knowing before planning: import is STEP-only (real, with XCAF colors — no IGES/STL import); Loft and Sweep are UNSUPPORTED at the worker (Shell, Patterns, and Mirror are live end-to-end); the exact L2 preview lane (`PreviewOp`) covers new-feature drafts only — re-edits are structurally L1-only because `PreviewOp` runs against the current head, and Shell has no L1 at all.