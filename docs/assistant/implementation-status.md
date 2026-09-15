# Assistant program — implementation status

**Work package:** WP-AI1 — assistant host process, private bridge, sidebar transport.
**Branch:** `claude/loving-cori-rz1v4i`, base `c947636`.
**Last updated:** 2026-09-14.

This document separates **implemented**, **tested**, and **owed**. "Code written" is not
"verified", and a gate that did not run is recorded as not run.

---

## 1. What this work package is, and what it is not

It implements the specification package's **WP1** (wire contract and private bridge) and
**WP2** (bundle AgentKit, connect the sidebar transport), plus the read-only half of
**WP5** (local provider gateway) so the agent has a model to answer with.

**The assistant cannot change a CAD document, and that is structural.** See ADR-0018: no
CAD tool is registered in the agent's catalog, the OCAK1 verb table contains no
document/filesystem/network verb, and AgentKit's `ProposalApplier` — the one place a write
could land — is a `NoopProposalApplier` whose `apply()` throws.

Out of scope, deliberately and in this order: the isolated draft workspace, the
authorization grant, idempotent operation ids, durable commit receipts, native adoption,
the modeling query façade, deterministic recipes, the evaluator registry, model
qualification. That ordering is the specification's own — model-driven mutation comes only
after the candidate/adoption/recovery foundations pass.

---

## 2. Requirement coverage

| ID | Requirement | State |
|---|---|---|
| EMB-01 | AgentKit bundled; no globally installed Bun/Node | **implemented, partially tested.** Compiles to a standalone binary staged as an `externalBin`; the packaged-app launch on a clean target is **owed** |
| LOC-01 | Same-machine inference, no cloud fallback | **implemented and tested end to end** — the gateway validates at registration, refuses non-loopback and every encoded-host spelling, disables redirects and proxy inheritance, and is installed only through a trusted command. The offline run against real weights is still **owed**: a loopback endpoint can proxy a cloud model and no code check can tell |
| SEC-01 | Restricted IO, data is not authority | **implemented and tested.** Principal stamping, verb table with no wildcard, no filesystem/shell/network tool |
| PORT-01 | Registered through platform contracts | **implemented and tested.** `onecad.assistant` registers through the module registry; no forbidden imports |
| AUTH-01/02, CAND-01, REF-01, OBS-01, IMG-01, VER-01, TXN-01, BUD-01 | Approval, candidates, references, observations, verification, transactions, budgets | **not started** — these belong to the later CAD work packages |

---

## 3. Measured results

Every number below was produced by a command run on this branch. Nothing is estimated.

### Runs clean

| Gate | Result |
|---|---|
| `bunx tsc --noEmit` | clean |
| `bun run test` (vitest) | **352 files, 6040 passed, 0 failed, 78 skipped** (baseline before this work: 346 / 5992) |
| `bun run build` | clean |
| hex gate `grep -rna '#[0-9a-fA-F]\{6\}' src …` | empty |
| `node scripts/verify-modeling-coverage.mjs` | 34 rows, 9 corpus cases, 16 CI jobs, 20 registry operations |
| `node scripts/verify-modeling-contracts.mjs` | 41 rows, 19 operations, 15 tier-checked |
| `cd assistant-host && bun test` | **59 passed, 0 failed, 7 files** |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean (see §5 — two pre-existing lints had to be fixed to get here) |
| `cargo test -p onecad-assistant-protocol --all-features` | **68 passed, 0 failed** |
| `cargo test -p onecad-assistant-protocol` (no features) | **58 passed, 0 failed** |
| `cargo test -p onecad --lib` | **540 passed, 0 failed** |
| `cargo test --test assistant_provider_gateway` | **17 passed, 0 failed** |
| `ONECAD_REQUIRE_ASSISTANT_HOST=1 cargo test --test assistant_bridge` | **6 passed, 0 failed** |

### The end-to-end proof

`assistant_bridge` runs against the **real compiled Bun sidecar**, not a mock:

```
real_host_completes_the_handshake ....................... ok
a_real_agentkit_fetch_streams_its_body_and_ends ......... ok
cancelling_a_stream_leaves_the_bridge_healthy ........... ok
a_killed_child_is_restarted_and_serves_again ............ ok
an_incompatible_protocol_version_is_rejected_before_anything_else ... ok
a_sidecar_request_claiming_the_ui_principal_is_refused .. ok
```

Their passing also proves the child's stdout hygiene: a single non-frame byte on stdout
would be a fatal frame error.

### Anti-vacuous-green, verified

`ONECAD_REQUIRE_ASSISTANT_HOST=1` with the binary pointed at a nonexistent path **fails
loudly** — 4 failed, 2 passed (the two that use a scripted in-process peer and need no
binary). It does not skip green. Checked explicitly, not assumed.

### The compiled binary

`src-tauri/binaries/onecad-assistant-host-x86_64-unknown-linux-gnu` — **100,543,494 bytes**.
Per ADR-0015 that is the Bun runtime floor, not our code: a compiled `console.log("hi")`
measures 99,295,580 bytes, so everything we add is ~104 KB, and minifying changes nothing.

### Startup bundle, verified not regressed

`AssistantPanel` pulls `agentkit/client` and `agentkit/contracts` (TypeBox). The UI
contributions therefore live in `src/modules/assistant/ui.ts`, not `register.ts`, because
`bootstrap.ts` imports the latter at startup. Verified by inspecting the built chunks:
`dist/assets/index-*.js` contains **0** AgentKit/TypeBox markers; `EditorScreen-*.js`
contains them.

### CORRECTION (2026-09-15): the chat loop does not complete

An external review of commit `0a0de65` found two defects that this document previously
understated, and I confirmed both by reading the code rather than taking them on trust:

1. **The configured provider never reaches AgentKit.** `assistant_configure_provider`
   populates Rust's `ProviderRegistry`, but `buildApp` performs no provider seeding — it
   contains no `upsertProvider` and no `defaultProviderId` — and the panel submits only
   `{content}` with no `providerId`. AgentKit's `TurnRunner` resolves its provider from its
   own store, which is empty, so a fresh install reaches `no_provider` before the factory
   that would have used Rust's gateway is ever called.
2. **The panel cannot render a reply.** `AssistantPanel` holds only `{key, text}` user
   echoes, discards the submit result, and never calls `streamRun`, `listMessages`,
   `useChat` or `useRun`. No assistant message can appear, and a terminal run failure is
   invisible.

Together these mean the WP2 exit gate — *a local durable chat sidebar* — **is not met**.
The earlier text here said only that "a real local model answering" was unverified. That
was too generous: the path is not merely unverified, it is structurally incomplete, and
saying so is the point of this document.

The panel's empty-state copy compounds it by claiming the assistant "can read the model and
propose edits; every proposal is reviewed before it is applied". Neither is true — the sole
tool reports constants — and the second sentence also contradicts the product's
approve-before-start autonomy decision.

What *is* real and independently gated stands: the supervised child process, the OCAK1
bridge with its principal boundary, the loopback-only gateway, and the absence of mutation
authority. What does not work yet is the last mile between them.

---

## 4. Owed — not run, not passed

| Gate | Why |
|---|---|
| `ONECAD_REQUIRE_WORKER=1 cargo test --workspace` | No OCCT on this machine. `src-tauri/binaries/onecad-worker-<triple>` is a **placeholder that exits 66** and says so on stderr, staged only so `tauri_build::build()` lets the app crate compile |
| `bun run e2e` of record | ~1.3 h, and the base is already red for unrelated reasons — `TODO.md` records Gate A open at 438 passed / 106 failed. Not this program's to close. The new panel is off by default and the targeted suites pass |
| `bun run tauri build` + packaged launch (EMB-01) | macOS/Windows are the shipping targets; this is Linux |
| Offline run against real local weights (LOC-01) | Needs a local inference runtime and a network-restricted environment |
| CI reproducibility of the AgentKit dependency | Blocked on an AgentKit `v0.5.0` tag — see §6 |

---

## 5. Defects found in existing code (not caused by this work)

1. **The hex gate was unsound.** Three files use a literal NUL byte as a composite-key
   separator (`src/features/tree/ModelTreePanel.tsx:96`,
   `src/shortcuts/keymap.golden.test.ts:56`, `src/tools/sketch/projectTool.ts:80`). `grep`
   classifies them as binary and **skips them silently**, so the gate read 750 of 753 files
   and exited 0 looking exactly like a clean run. `CLAUDE.md`'s command now carries `-a`.
   Measured: the gate is genuinely empty with and without the flag — nothing was hiding, but
   the gate as written could not have told you.
2. **Two pre-existing clippy lints blocked the workspace gate**, both the newest-stable trap
   `CLAUDE.md` warns about (1.94.0 `nonminimal_bool`):
   `crates/onecad-core/src/regen/feature_pattern.rs:330` and
   `crates/onecad-core/src/regen/feature_pattern_tests.rs:217`. Both files are untouched at
   HEAD — verified with `git diff HEAD`. Fixed mechanically with clippy's own suggestion,
   solely so `cargo clippy --workspace -- -D warnings` could be run and reported honestly.
3. **A protocol asymmetry, found by cross-checking the two implementations.** The Rust
   `Res`/`End` types allowed `ok: false` with no `error`, which the TypeScript peer rejects
   on decode. The doc comment claimed the invariant; nothing enforced it. `Envelope::validate`
   now enforces it on every decode, with five tests pinning it.

---

## 6. Known gap: the AgentKit dependency

AgentKit publishes nothing installable today — no git tag, and `packages/agentkit/` has no
committed `dist/`, so the documented `github:andrejvysny/AgentKit#v0.5.0` does not resolve
and `#<commit>` resolves to the private workspace root.

`scripts/bootstrap-agentkit.sh` bridges this: it clones the pinned commit
`a450d6ff470fcfaacf3a37375d00d962ed2632e8`, runs AgentKit's own `bun run build &&
bun run build:umbrella`, and `assistant-host` depends on the result by path.

**This is not CI-reproducible in the normal sense** — CI must clone and build AgentKit on
every run. When `v0.5.0` is cut: point `assistant-host/package.json` at the tag, delete
the script, and drop its two call sites in `.github/workflows/ci.yml`. Nothing else
depends on it.

---

## 7. Next eligible work package

The candidate/draft workspace and durable adoption (the specification's WP3 + WP4). Its
entry condition is met: there is a supervised host, a bridge with a principal boundary, and
a tool loop — with no mutation authority to retrofit safety around.

That package replaces exactly one object, the `NoopProposalApplier`, behind a draft
workspace, an authorization grant, idempotent operation ids and durable receipts. Until
then, `apply()` throwing is correct behaviour and its test asserts it.
