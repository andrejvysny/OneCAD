# Debugging OneCAD (dev-observability manual)

Audience: an AI agent (Claude Code) debugging this app, not a human skimming for
vibes. Every claim below is either a direct read of the cited source line or an
empirical capture (marked as such). Optimized for grep-ability: copy commands,
don't paraphrase.

See `CLAUDE.md` § **Debugging & logs** for the condensed version. This is the
deep manual it points to.

## 1. The four lanes and where they come from

| Lane (`target`) | Producer | Source |
|---|---|---|
| Rust module path (e.g. `onecad_lib::document_runtime`, `onecad_lib::api`) | Every `tracing::info!/warn!/error!/debug!` call site with no explicit `target:` | `src-tauri/src/**/*.rs` |
| `ipc::request::handler` (nested under a Rust module target) | Tauri's own `tracing` cargo feature — a free `debug_span!` around **every** `#[tauri::command]`, current + future, zero per-command code | tauri-macros wrapper (external; enabled by workspace `Cargo.toml`) |
| `worker` | Forwarded C++ worker stderr, one JSONL event per line, level sniffed from the WLOG prefix | `src-tauri/src/worker/manager.rs` `forward_worker_stderr` (~line 604) |
| `fe` | Batched frontend log events re-emitted server-side | `src/debug/log.ts` (core) → `src/debug/logSink.ts` (batches, POSTs via `log_event`) → `src-tauri/src/api/mod.rs` `log_event`/`emit_fe_event` (~line 1474) |
| `onecad_protocol::frames` | OCW1 tx/rx frame trace, debug-gated (off by default) | `src-tauri/crates/onecad-protocol/src/client.rs` `trace_tx`/`trace_rx`/`trace_rx_frame` (~line 59) |
| `panic` | Rust panic hook, chained before the previous hook | `src-tauri/src/logging.rs` `install_panic_hook` |

The C++ worker itself never touches `dev.jsonl` — it only writes lines to its
own stderr (`worker/src/util/Log.h`, `WLOG_*` macros), which the Rust parent
pipes and re-emits under `target: "worker"`. `worker/src/protocol/Dispatcher.cpp`
`execute()` (~line 39) additionally times every verb: a success gets
`WLOG_DEBUG("verb '%s' id %llu ok in %lld ms", …)`, both `catch` arms and the
unknown-verb branch get `WLOG_ERROR` with the same `id`/elapsed shape.

## 2. Sink + session scoping

`logs/dev.jsonl` (repo root, `.gitignore`d) is created (`std::fs::File::create`
— truncating, not appending) at every `run()` — see `src-tauri/src/logging.rs`
`init()`/`open_sink()`. The **first line is always** a `session.start` event:

```json
{"timestamp":"2026-08-02T18:03:10.481221Z","level":"INFO","target":"onecad_lib::logging","pid":41213,"version":"0.1.0","filter":"info,onecad_lib=debug,onecad=debug,fe=debug,worker=debug","dir":"/Users/you/OneCAD-Tauri/logs","message":"session.start"}
```

So "this run" is literally "the whole file" — grep never spans two sessions.
Only `run()` calls `init()`; `cargo test`, ctest, vitest, and Playwright never
touch the file lane (`init_test_tracing()` is the stderr-only opt-in for unit
tests — see the module doc at the top of `logging.rs`). The file is only
written under a debug build (or an explicit `ONECAD_LOG_DIR`), i.e. `tauri dev`
or a locally-built app, not a release/packaged build by default.

## 3. Full JSONL schema

Base shape (`tracing-subscriber` json layer, `flatten_event(true)`,
`with_current_span(true)`, `with_span_list(false)`, `FmtSpan::CLOSE`):

```
{"timestamp":"<RFC3339 µs, UTC>","level":"INFO|WARN|ERROR|DEBUG","target":"…","message":"…", <event fields, flattened at top level>, "span":{"name":"…", <span fields>}}
```

**Reserved field names** — never reuse them as an event field, they collide
with the envelope: `timestamp`, `level`, `target`, `span`, `message`. `span` is
present only when the event fires inside an active span (`with_span_list(false)`
means only the CURRENT span is attached, never the whole stack — deliberate,
keeps lines short).

### 3a. A plain event line (regen postmortem, `document_runtime.rs::log_regen_outcome`)

```json
{"timestamp":"2026-08-02T18:03:11.482201Z","level":"INFO","target":"onecad_lib::document_runtime","job":"1f2e3a9c-...-b7","rev":42,"srcRev":41,"snapshot":91,"base":"a3f9c1e08b2d","steps":3,"changed":2,"removed":0,"failedSteps":0,"needsRepair":0,"message":"regen: published","span":{"name":"regen.drive","job":"1f2e3a9c-...-b7","rev":42,"epoch":3,"base":"a3f9c1e08b2d","steps":3}}
```

### 3b. A span-close line (`FmtSpan::CLOSE`)

```json
{"timestamp":"2026-08-02T18:03:11.483880Z","level":"INFO","target":"onecad_lib::document_runtime","message":"close","time.busy":"1.68ms","time.idle":"3.02µs","span":{"name":"regen.drive","job":"1f2e3a9c-...-b7","rev":42,"epoch":3,"base":"a3f9c1e08b2d","steps":3}}
```

**`time.busy`/`time.idle` are unit-suffixed STRINGS** (`"1.68ms"`, `"890µs"`,
`"2.34s"`) — a human hint from `tracing-subscriber`, not a machine-comparable
number. Every terminal event that matters (e.g. `regen.drive: done`) carries
its own explicit **numeric** `elapsed_ms` field instead — see
`document_runtime.rs` `PreparedRegen::drive` (~line 2083):
`elapsed_ms = started.elapsed().as_millis() as u64`. Pinned by
`logging.rs::tests::jsonl_schema_carries_the_documented_fields`.

### 3c. An `fe` line (`api/mod.rs::emit_fe_event`, ~line 1497)

```json
{"timestamp":"2026-08-02T18:03:10.912004Z","level":"DEBUG","target":"fe","seq":184,"tag":"ipc","feTs":1785700990911.2,"tMono":48213.556,"ctx":"{\"cmd\":\"apply_edit_command\",\"durMs\":12.4,\"args\":{\"command\":\"{recordId,operation}\"}}","message":"apply_edit_command"}
```

Gotcha: `ctx` here is a **JSON string containing JSON text**, not a nested
object. `FeLogEvent.ctx` is `Option<serde_json::Value>`, and `emit_fe_event`
stringifies it (`v.to_string()`, capped at `FE_CTX_CAP = 2048` bytes,
`cap_str`) before handing it to `tracing` as `ctx = ctx` (an `Option<&str>`) —
so it round-trips through `json.parse()` twice if you want the structured
data back. `message` is always the FE `msg` string, never the tag.

### 3d. A `worker` line (`worker/manager.rs::forward_worker_stderr`, ~line 604)

```json
{"timestamp":"2026-08-02T18:03:10.911500Z","level":"INFO","target":"worker","epoch":3,"message":"[2026-08-02 18:03:10.911] INFO  worker: kernel ready OCCT 7.9.3"}
```

Two timestamps live in this line on purpose: the outer `timestamp` is when
Rust *received/forwarded* the line; the `[2026-08-02 18:03:10.911]` inside
`message` is the C++ side's own clock (`worker/src/util/Log.h::write_timestamp`,
millisecond precision). They can legitimately differ by the pipe's buffering
latency — don't treat a mismatch as a bug. The level is *sniffed* from the
`WLOG` prefix (`sniff_worker_level`); anything without a recognized
`[ts] LEVEL` prefix (a raw OCCT message, the test stub) is logged at `INFO`,
never silently dropped. Lines over `MAX_WORKER_LINE` (8 KiB) are truncated
with a `…[truncated]` marker.

## 4. Tag taxonomy (FE, `target: "fe"` lines only)

Declared in `src/debug/log.ts` (top-of-file doc); "used" column is a source
grep, not the doc comment's aspiration:

| Tag | Meaning | Call sites |
|---|---|---|
| `ipc` | Every real-lane `invoke()` round trip, success (debug) or failure (warn), `{cmd, durMs, args}` / `{cmd, durMs, kind, message}` | `src/ipc/tauriClient.ts` `call<T>` (~line 327). Exempt: `solve_drag` (drag-frequency), `log_event` (would recurse) |
| `fsm` | A model-tool phase TRANSITION only (same-phase steps, i.e. drags, are silent by construction) | `src/tools/modelTools/fsmLog.ts` `withPhaseLog`; the *tool* name (`extrude`, `edgeOp`, `revolve`, `boolean`, `shell`, `linearPattern`, `circularPattern`, `mirror`) is a `ctx.tool` value under this ONE tag, not its own tag — `ModelToolController.ts` ~line 133-140 |
| `vp` | Viewport engine lifecycle: init/dispose, WebGL context lost/restored, WebGPU fallback, mesh-registry leak tripwire | `ViewportEngine.ts`, `renderer.ts`, `meshRegistry.ts`, `ViewportRoot.tsx` |
| `hint` | Every status-bar hint the user was actually shown (90+ call sites funnel through one store method); a `null` clear is silent | `src/stores/viewportStore.ts` `setStatusHint` (~line 172) — `log(severity==="error"?"warn":"debug", "hint", message, {severity, sticky})` |
| `err` | Uncaught FE errors: window `error`/`unhandledrejection`, and React render/lifecycle throws | `src/main.tsx` (~line 19-33), `src/app/ErrorBoundary.tsx` `componentDidCatch` |
| `worker` | FE-side mirror of worker lifecycle (state, epoch) — **distinct from the Rust `target:"worker"` lane**; this is `tag:"worker"` INSIDE `target:"fe"` | `src/stores/workerStore.ts` (~line 28) |
| `repair` | Needs-repair set changed (warn) / cleared (debug) | `src/stores/repairStore.ts` (~line 60, 65) |
| `sink` | `logSink.ts`'s own diagnostics about itself — deliberately NEVER forwarded to Rust (the second recursion guard, `ev.tag === "sink"` check) | `src/debug/logSink.ts` |
| `sketch` | Sketch session failures: enter/exit/switch, orphan cleanup, unmapped solved points | `src/tools/sketch/SketchController.ts`, `src/ipc/tauriClient.ts`, `src/ipc/sketchWireMap.ts` |
| `mesh` | Body mesh ingest failures | `src/viewport/mesh/meshSync.ts` (~line 295) |
| `boolean` / `extrude` / `revolve` / `measure` | Listed in the `log.ts` doc comment as "op/tool lanes" but are **not** standalone tags at any current call site — `boolean`/`extrude`/`revolve` surface only as `ctx.tool` under `fsm` (see above); `measure` has zero call sites today (the measure tool is read-only and unin­strumented) | n/a |

## 5. Enabling matrix

| Knob | Consumer | Default | Effect |
|---|---|---|---|
| `RUST_LOG` | app (`logging::env_filter`) + `onecad-regen` CLI | `info,onecad_lib=debug,onecad=debug,fe=debug,worker=debug` (`DEFAULT_FILTER`, `logging.rs`) | Standard `EnvFilter` syntax. The `fe=debug`/`worker=debug` directives are REQUIRED — both lanes emit at `debug`, so `RUST_LOG=info` alone silently drops them both (pinned by `logging.rs::tests::default_filter_keeps_the_synthetic_lanes_at_debug`) |
| `RUST_LOG=…,onecad_protocol::frames=debug` | `onecad-protocol` `client.rs` | frames lane off by default | Turns on tx/rx per OCW1 frame (`id`, `verb`, `elapsed_ms`, `ok`, `binLen`); `=trace` additionally logs bulk `chunk` frames (a mesh transfer is thousands of them) |
| `ONECAD_LOG_DIR` | app (`logging::log_dir`) | debug build: `<repo>/logs`; release: off | `off` (any case) kills the file lane even in a debug build; any other value is used as the directory verbatim (works in a packaged build too) |
| `ONECAD_WORKER_LOG` | C++ worker (`Log.h::init_level_from_env`) | `info` | `error\|warn\|info\|debug`, case-insensitive, read once as the literal first statement of `main()` (so it covers `--selftest` too); an unrecognized value falls back to `info` plus one `WLOG_WARN` |
| `?trace` (URL) or a DEV build | FE logger gate (`log.ts::gateOpen`) | on in `bun run dev`/e2e vite lane; off under vitest always | Enables `logDebug/Info/Warn/Error`, the console mirror, and `window.__logs*` |
| `?vpdebug` (URL) | `ViewportRoot.tsx` → `ModelToolController`/`ViewportEngine` `debug` flag | off | Exposes `window.__vpEngine`, `window.__extrudePreview`; also gates `updateDebug()`'s phase-SNAPSHOT debug events (the `fsm` tag's transition events are NOT gated by this — they fire regardless) |
| `ONECAD_WORKER_PATH` / `ONECAD_REQUIRE_WORKER` | Rust test harness (pre-existing, unrelated to logging) | — | See `CLAUDE.md` § Commands |

## 6. Correlation glossary

- **`revision`** (`rev` in logs) — a Rust-owned, monotonically-advisory stamp on the document. **Not** a fencing token — decision D4 fences on `workerEpoch` + `expectedBaseHash` only, so a `rev` mismatch by itself proves nothing about whether a regen actually landed.
- **`opId` / `recordId`** — the commit identity: `RecordId` (UUID-backed, `src-tauri/crates/onecad-core/src/ids.rs`). The wire DTO field is camelCase `recordId` (`src/ipc/types.ts`); the JSONL `regen: FAILED step` warn line shortens it to a bare `record` field (`document_runtime.rs::log_regen_outcome`, ~line 2016). A **successful** commit's `recordId` is NOT re-emitted in the outcome line (only counts: `changed`/`removed`) — trace a success via `job`/`rev`/timestamp windowing instead, or via the FE `ipc` line's `ctx.args` (top-level string/number args pass through; a nested command envelope collapses to `{key1,key2,…}` shape only — see `tauriClient.ts::summarizeValue`).
- **`workerEpoch`** (`epoch` in logs) — bumped on every worker restart (`WorkerManager` `Shared.epoch`, `fetch_add` on the death path before respawn). It is BOTH a fencing input (with `expectedBaseHash`) AND the join key between a `regen.drive` span (carries `epoch`) and the `worker` lane's forwarded stderr lines (also carry `epoch`, since the forwarder task is spawned with the epoch read at spawn time).
- **OCW1 `id`** — the frame-lane join key (field literally named `id`, not `reqId`, in `onecad_protocol::frames` tx/rx lines — `client.rs` `trace_tx`/`trace_rx`). Rust-assigned, monotonic per `ProtocolClient`. Frame lines do NOT carry `epoch` themselves; join them to a regen by enabling `frames=debug` together with the default filter — the tx/rx events happen to fall inside the `regen.drive` span's time window (the executor future is `.instrument(span)`-wrapped, `PreparedRegen::drive` ~line 2076) but are not literally nested `span` children in the JSON (no span context propagates across the stdio await boundary the same way).
- **`sketchId` vs `backendSketchId`** — `sketchId` (as used in FE code, e.g. `sketchMaps.get(frontendId)`) is a frontend-LOCAL key; `backendSketchId` is the real, Rust-recognized `SketchId`, minted by the frontend (`mintUuid()`) and adopted VERBATIM by the backend (id-adoption rule — `tauriClient.ts::ensureBackendSketch`, ~line 745-780). Any Rust-side log line's `sketchId` field is always the backend one.
- **`elapsed_ms` / `durMs` (numeric) vs `time.busy`/`time.idle` (string)** — see § 3b. Always prefer the numeric field for anything you compute on; the string is presentation-only.

## 7. Worker-lane no-span caveat

`forward_worker_stderr` is a **detached** `tokio::spawn` task (spawned once per
worker connection, outlives nothing but the pipe — `manager.rs` ~line 582), so
every `worker`-lane line carries **no `span` object at all**, regardless of
what Rust-side span is active when the worker happens to emit it. Do not
expect to find a worker line nested under a `regen.drive` span in the JSON —
join by `epoch` (same worker connection) and by wall-clock proximity, or by
`frames=debug`'s `id`/timing if you need frame-level precision.

## 8. Cookbook

All commands assume `cd` to the repo root; `logs/dev.jsonl` is the file
described in § 2. `jq` may not be installed — every recipe has a
grep/python fallback with no extra dependency.

**Did the app error this session at all?**
```bash
jq -c 'select(.level=="ERROR")' logs/dev.jsonl
# fallback:
grep '"level":"ERROR"' logs/dev.jsonl
python3 -c "import json,sys
for l in open('logs/dev.jsonl'):
    e = json.loads(l)
    if e.get('level') == 'ERROR': print(l.strip())"
```

**Why did a commit fail?** (the postmortem contract fires on EVERY finish path)
```bash
jq -c 'select(.message=="regen: FAILED step")' logs/dev.jsonl
grep 'regen: FAILED step' logs/dev.jsonl
# the summary line right before it (job/rev/steps/failedSteps/needsRepair):
grep '"message":"regen: failed"' logs/dev.jsonl
```

**Trace one `job`/`epoch`/`recordId` across every lane:**
```bash
jq -c --arg j "<job-uuid>" 'select(.job==$j or .span.job==$j)' logs/dev.jsonl
grep -F '<job-uuid>' logs/dev.jsonl   # cruder but catches the span-nested copy too
```

**Worker crash timeline (lifecycle + poison circuit):**
```bash
jq -c 'select(.target=="worker" or (.message|test("worker (ready|restarting|failed|crash circuit)")))' logs/dev.jsonl
grep -E '"target":"worker"|worker (ready|restarting|failed|crash circuit)' logs/dev.jsonl
```
A `CircuitOpen` line always carries `key` + `last_crash` — `worker/manager.rs`
`Shared::emit` (~line 204) looks up the message specifically so the log line
is self-contained; no need to scroll back for "why".

**Slow commands / slow regen** — do NOT `sort` on `time.busy` (it's a string
like `"1.68ms"` vs `"890µs"`, alphabetically nonsensical). Use the numeric
fields instead:
```bash
jq -c 'select(.message=="regen.drive: done") | {job, rev, elapsed_ms}' logs/dev.jsonl
jq -c 'select(.target=="fe" and .tag=="ipc") | select(.ctx | test("durMs"))' logs/dev.jsonl
python3 -c "import json
for l in open('logs/dev.jsonl'):
    e = json.loads(l)
    if e.get('message') == 'regen.drive: done': print(e['job'], e['elapsed_ms'])"
```

**What did the user actually see on screen?**
```bash
jq -c 'select(.tag=="hint")' logs/dev.jsonl   # note: target is "fe", tag is "hint"
grep '"tag":"hint"' logs/dev.jsonl
```

**Frame-level protocol debug** (run with `RUST_LOG=info,onecad_lib=debug,fe=debug,worker=debug,onecad_protocol::frames=debug`):
```bash
jq -c 'select(.target=="onecad_protocol::frames")' logs/dev.jsonl
grep '"target":"onecad_protocol::frames"' logs/dev.jsonl
```

## 9. Failure-signature table

| Signature | Meaning | Action |
|---|---|---|
| `regen: superseded` outcome | Fencing moved (another edit landed) during worker IO — a routine race under rapid edits, logged as `warn` ("window race — accepted lock-free, document advanced") | Not a bug by itself; only worth chasing if the FOLLOWING regen also fails |
| `"regen: FAILED downgraded to superseded (fencing moved)"` | An `EngineFailed` outcome whose fencing tokens moved before `finish_regen` ran — the real failure (`original_error` field) is dropped from the report on purpose, a later covering regen is already in flight | Read `original_error` on THIS warn line — it's the only place it's visible |
| `STALE_PREVIEW` / `"engine: stale preview"` (debug, not warn) | A drag-preview candidate raced a publish — expected, deliberately demoted below `warn` so `'"level":"WARN"'` stays a useful first-look grep (`error.rs` `From<EngineError> for ApiError`, ~line 69-79) | Ignore unless it never resolves (preview stuck) |
| `"worker crash circuit open (poison) — this plan now fails fast"` (ERROR) | Same op key crashed the worker `poison_threshold` times in a row — the plan now fails fast instead of retrying (`manager.rs::plan_circuit_open`) | The line carries `key` + `last_crash` — read the crash message, not just "circuit open" |
| `NeedsRepair` (never an ERROR — it's document *state* in the projection) | Deterministic by design: auto-bind requires score ≥0.85 AND margin ≥0.10; a symmetric tie or a fillet's consumed edge is `NeedsRepair`, never a guessed bind — see `CLAUDE.md` § Identity rules | Not a bug; the correctness spine working as intended |
| Worker process exits with code **2** | `Frame::BadMagic` / malformed OCW1 frame — fatal by protocol design, no resync (`worker/src/protocol/Frame.h` ~line 21, 46) | Look at the LAST `onecad_protocol::frames` tx/rx (if enabled) and the LAST `worker` lane lines before the exit |
| `"unknown verb '%s'"` (worker `WLOG_ERROR`) | Well-framed but protocol-illegal verb — a terminal `PROTOCOL_ERROR` resp, NOT a process exit (`Dispatcher.cpp::execute`, ~line 43-51) | Compare against `protocol/SCHEMA.md`'s verb list — likely a version skew between Rust and worker |
| `"dev log file unavailable; stderr only"` (warn, stderr-only — the file layer already failed so it can't reach `dev.jsonl`) | `ONECAD_LOG_DIR` unwritable/permission error (`logging.rs::init`) | Check the directory exists and is writable; the app still runs, just without the file lane |

## 10. White-box surfaces (dev builds only; gated per § 5)

- `window.__logs` — getter, chronological `LogEvent[]` snapshot of the FE ring (2000-cap, head-index).
- `window.__logsDump()` — same, pretty-printed JSON string. This is exactly what e2e's `fe-logs.json` attachment contains (see § 11).
- `window.__logsClear()` — empties the ring.
- `window.__stores` — 12 keys: `document, sketch, viewport, tool, settings, selection, measure, app, toolChip, worker, repair, sketchSelection` (`src/main.tsx` ~line 68-81).
- `window.__vpEngine` — only under `?vpdebug`; raycast/hit-test methods (`planePickerHitTest`, `probePick`, `hitExtrudeHandle`) and `debugSnapshot()` (theme/clearColor).
- `window.__datumVisuals` — DEV builds; the datum layer's own `hitTest(x, y)` / `ghostVisible`. It moved off `__vpEngine` when datum planes became a viewport CONTRIBUTION (`modules/modeling/datumViewport`), so the engine no longer has `datumHitTest`. Null until the contribution attaches.
- `window.__extrudePreview` — only under `?vpdebug`; `ModelToolController.updateDebug()` phase snapshot (`phase`, `revolvePhase`, `filletPhase`, `shellPhase`, `edgeOpKind`, `edgeOpAuto`, `edgeOpAxisSource`, `l1Present`).

## 11. e2e artifacts (`e2e/fixtures.ts`)

Every spec imports `test`/`expect` from `./fixtures` instead of
`@playwright/test`. An `auto: true` fixture (`forwardBrowserLogs`) buffers
`page.on("console")` and `page.on("pageerror")` for the whole test; after the
test body runs, if the test FAILED or ANY pageerror fired (even on an
otherwise-passing test), it writes three files under
`test-results/<test-title-slug>/`:

- `console.log` — `[type] text` per line, chronological.
- `pageerror.log` — one stack (or `String(err)`) per line; empty file if none fired.
- `fe-logs.json` — `window.__logsDump()`'s output (empty string, hence an
  effectively-empty file, if the surface wasn't installed — e.g. a non-DEV
  build in the lane).

Implementation note: `testInfo.attach(name, { body })` keeps the content as an
**in-memory Buffer only** — `normalizeAndSaveAttachment` in
`playwright/lib/util.js` copies bytes to disk exclusively for the `{ path }`
form. This project's reporter is `"list"`/`"line"` (`playwright.config.ts`
does not set `"html"`/`"json"`), which never serializes a body attachment to a
file, so `fixtures.ts` writes each file itself via
`fs.writeFile(testInfo.outputPath(name), …)` and pushes onto
`testInfo.attachments` directly with `{ path }` — the same pattern Playwright's
own "Automatic Fixture" doc uses. A `body`-only version of this fixture would
report attachments correctly in the terminal but leave nothing on disk to open
after the run — verified empirically while building this fixture.

Plus Playwright's own `trace: "on-first-retry"` (`playwright.config.ts`) on a
retried test, and the auto-generated `error-context.md` (Playwright core, not
this fixture) on every failure.

## 12. Extension policy

- **New FE tag**: add it to the taxonomy comment at the top of `src/debug/log.ts` first, then use it. Keep `docs/DEBUGGING.md` § 4 in sync.
- **Never** log a drag-frequency path — per-frame preview updates, pointer moves, solver ticks — on either side (Rust or FE). This is a hard policy, not a style preference, enforced by TWO separate exemption lists that must not be confused: Rust-side, `preview_op`/`solve_drag`/`get_mesh`/`get_sketch_regions` are the four commands that do NOT get a hand-`#[instrument(err(Display))]` (would ERROR-flood on every stale-preview drag race — see `api/mod.rs` R3 notes); FE-side, only `IPC_LOG_EXEMPT = {solve_drag, log_event}` (`tauriClient.ts` ~line 285) skips the `ipc`-tag log line — `preview_op`/`get_mesh`/`get_sketch_regions` DO get an `ipc` line per call today (they are not literally drag-frequency callers themselves; the tool that owns the drag frame is). FE's `fsmLog` only fires on a PHASE CHANGE, never a same-phase drag step, by construction.
- **`ctx` caps are enforced at entry**, not at read time: `MAX_STRING=500`, `MAX_ARRAY=20`, `MAX_DEPTH=2` (`log.ts`); Rust-side `FE_CTX_CAP=2048` bytes on the already-capped, already-stringified ctx (`api/mod.rs`). Don't rely on a caller to self-limit — the logger does it.
- **`logSink.ts` must never recurse**: it imports `invoke` directly from `@tauri-apps/api/core` (never through `tauriClient.ts`'s `call<T>`, which itself logs), and the `sink` tag is never forwarded (`ev.tag === "sink"` guard). If you add a new forwarding-adjacent module, preserve both invariants.
- **Reserved event-field names** (§ 3): `timestamp`, `level`, `target`, `span`, `message`. Never name a new structured field one of these.
- **The wire DTO between FE and Rust is frozen jointly**: `LogEvent` (`log.ts`) and `FeLogEvent` (`api/mod.rs`) must change together, same camelCase shape, same wave.

## 13. Known limitations

- **`vitest` is silenced by design** — `log.ts::underVitest()` closes the gate whenever `import.meta.env.MODE === "test"` or `process.env.VITEST` is set; ~30 test files drive the instrumented code paths and a console mirror there would bury real test output. The logger's OWN tests re-open the gate via `__resetLogForTests()`.
- **React 19 StrictMode double-mounts** — dev builds intentionally invoke mount effects twice, so you will see paired `vp` `"engine dispose"` + `"engine init"` lines (and similar) on first load; this is expected, not a leak (see `CLAUDE.md` § Viewport: `init()`/`dispose()` are idempotent for exactly this reason).
- **SIGINT loses ≤one buffer of JSONL tail** — the `WorkerGuard` (flushes the non-blocking JSONL writer) is dropped on `RunEvent::Exit`; a Ctrl-C on `tauri dev` never reaches that arm. Accepted trade-off — the stderr fmt layer is unbuffered, so the panic/crash lane never loses a line even then.
- **Multi-instance dev is not supported** — `dev.jsonl` is one fixed path, truncated at start. Two concurrent `tauri dev` processes will clobber/interleave each other's session in the same file. No escape hatch is implemented today (a `dev-<pid>.jsonl` naming scheme was discussed and deferred — see the DEV-OBSERVABILITY plan's "Unresolved questions").
