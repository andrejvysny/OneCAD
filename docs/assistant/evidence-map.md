# Assistant program — evidence map

**Compiled:** 2026-09-14, branch `claude/loving-cori-rz1v4i`, base `c947636`.
**Scope of this document:** what was *measured* in this environment, what is *known from
source inspection*, and what is *unrun*. It is the WP0 deliverable the implementation guide
requires before feature work.

Nothing in this file is an estimate. A row that says "unrun" was not run.

---

## 1. Environment — what this container can and cannot do

| Lane | Status | Evidence |
|---|---|---|
| Frontend typecheck | **runs** | `bunx tsc --noEmit` → exit 0 |
| Frontend unit (vitest) | **runs** | `bun run test` |
| Frontend build | **runs** | `bun run build` |
| `onecad-core` Rust | **runs** | `cargo check -p onecad-core --all-targets` → clean, 33 s |
| `onecad-worker-stub` | **runs** | `cargo check -p onecad-worker-stub` → clean |
| `onecad` app crate | **runs, after two fixes** | Needed GTK/WebKit system libraries (installed in-session: `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`) **and** a staged `externalBin` file. With both: `cargo check -p onecad --lib` → exit 0, 58 s |
| Bun | **runs** | 1.3.11 |
| `bun build --compile` | **runs** | see §3 |
| Real OCCT worker | **UNAVAILABLE** | No OCCT on the box; apt carries 7.6.3, the pin is 8.0.1 built from source (`scripts/build-pinned-occt.sh`). Not built, by decision. |
| Worker-backed Rust gates | **UNRUN — OWED** | `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` cannot be honest without the real worker |
| Playwright e2e | **runnable but not of record** | ~1.3 h for both projects, and the base is already red for unrelated reasons (see §5) |
| Packaged app (`tauri build`) | **UNRUN — OWED** | macOS is the shipping gate; this is Linux |

### The worker placeholder, and why it shouts

`src-tauri/binaries/onecad-worker-<triple>` is a staged **placeholder**, not a worker.
`tauri_build::build()` resolves every `externalBin` entry inside the build script, so
without a file there the app crate fails to compile before a line of Rust is checked —
measured: `resource path binaries/onecad-worker-x86_64-unknown-linux-gnu doesn't exist`.

`resolve_worker_path` checks *existence*, not executability, so a silent placeholder would
let a worker-backed test resolve a path and then fail in a way that reads like a kernel
bug. The staged file therefore prints two lines naming itself a placeholder and exits 66.
**Every worker-backed gate in this program is owed, not passed.**

---

## 2. AgentKit — inspected at `a450d6ff`, built in-session

| Claim from the specification package | Verified? | Evidence |
|---|---|---|
| The client accepts an injected `fetch` | **yes** | `packages/client/src/transport.ts`, `AgentKitClientOptions.fetch`; its doc comment names an Electron session-scoped fetch as the motivating case |
| `createRestHandler` needs no socket | **yes** | `packages/transport-http/src/handler.ts:119`; `serveRest` is literally `{ fetch: createRestHandler(deps) }` |
| `ProposalApplier.getOutcome` returns `null` for "never ran" | **yes** | `packages/host/src/ports/proposal-applier.ts` |
| The OpenAI-compatible provider flattens tool-role images | **yes, with a caveat** | `mapMessageContent` in `packages/core/src/providers/openai-compatible.ts` drops image parts on `system`/`tool` roles — but it reports them through a `flattened` set and warns. The gap is real; it is **not** silent, which the specification slightly overstates |
| **AgentKit is installable from a tag** | **NO — the claim does not hold** | `git ls-remote --tags` returns nothing; `packages/agentkit/` holds only `README.md` and `package.json`, no committed `dist/`. The documented `github:andrejvysny/AgentKit#v0.5.0` does not resolve, and `#<commit>` resolves to the private workspace root, which exports nothing |
| AgentKit builds from source at the pin | **yes, measured** | `scripts/bootstrap-agentkit.sh` → `bun install` (262 packages, 3.11 s), `bun run build` (12 packages), `build:umbrella` → "dist assembled from 12 packages (377 files)" |

The tag gap is the one blocking external dependency in this program. It is bridged by
`scripts/bootstrap-agentkit.sh` and is recorded in §5 as a known gap, not hidden.

---

## 3. `bun build --compile` — measured, not assumed

Two risks were probed before any production code was written, because both could have
invalidated ADR-0015.

**FTS5 and `bun:sqlite` inside a compiled binary — both work.** AgentKit's `SCHEMA_V8`
issues `CREATE VIRTUAL TABLE … USING fts5`, so a Bun build without FTS5 would fail at
*open* time on every boot. A compiled probe created an FTS5 table, matched a row, then
opened a real `SqliteAssistantStore` on a real file and round-tripped a chat record:

```
FTS5: ok
SqliteAssistantStore: ok (created c1, listed 1)
```

**Binary size is ~99 MB, and it is the runtime, not us:**

| Build | Bytes |
|---|---:|
| compiled `console.log("hi")` | 99,295,580 |
| the same, with AgentKit + the store | 99,399,737 |
| minified | 99,365,844 |
| **delta attributable to our code and all dependencies** | **104,157** |

So bundling choices cannot recover this. Dropping AgentKit's MCP packages is still correct
for attack surface and startup cost, but it is not a size lever. Embedding any JavaScript
runtime costs about this much; Node's single-executable output is comparable.

---

## 4. OneCAD — capability matrix for the surfaces this program touches

| Surface | Exists today | Notes |
|---|---|---|
| Module/panel contribution framework | yes | `src/platform/`; slots are a closed set, ids owner-namespaced, duplicates a hard failure |
| A free slot for a sidebar | **no** | `Slots.ShellRight` is occupied by `InspectorPanel`, which is absolutely positioned and owns the right edge. The sanctioned pattern for sharing a docked region is the tab strip (`src/stores/sidebarTabStore.ts`) |
| Published modeling services | **7 methods, 2 services** | `GeometryQueryService.classifyElement` plus six command-API methods. `CadClient` has ~90. Any real geometry read for an agent is a façade-widening work package of its own |
| Second supervised child process | **precedented** | `src-tauri/src/library.rs:839` (`EphemeralWorker`) and `library_ingest.rs:525` already spawn independent `WorkerManager`s never installed into `AppState` |
| Reusable generic framing | **no** | `onecad-protocol` is OCW1-specific: magic baked into `parse_header`, `OcwCodec` unconfigurable, `ProtocolClient::connect` requires an OCW1 `hello` frame |
| Anything AI-related | **greenfield** | No `agentkit`, `assistant`, work-order or grant code exists anywhere |
| Durable atomic write primitive | yes | `onecad_core::io::durable_write` — temp sibling → `sync_all` → rename → `fsync_dir` |

**Two names are already taken** and are not reused by this program: *sidecar* means the C++
OCCT worker throughout the codebase, and *candidate* means snap/selection candidates and
repair-rebind candidates.

---

## 5. Known gaps and owed gates

| Item | Status |
|---|---|
| AgentKit `v0.5.0` tag | **blocked on the AgentKit repository owner.** Until then `scripts/bootstrap-agentkit.sh` builds from a pinned commit and the dependency is not CI-reproducible |
| Real-kernel Rust gates | **owed** — needs pinned OCCT 8.0.1 and a real worker |
| `bun run e2e` of record | **owed, and not this program's to close.** `TODO.md` at HEAD records Gate A open: the `commitFillet` same-turn race, and a last run of record at 438 passed / 106 failed. Pre-existing and untouched here |
| Packaged app on macOS/Windows/Linux | **owed** — including the EMB-01 requirement that the assistant starts on a clean target with no globally installed Bun or Node |
| Offline run against real local weights | **owed** — the only honest test of ADR-0017's locality claim, since a loopback endpoint can proxy a cloud model |
