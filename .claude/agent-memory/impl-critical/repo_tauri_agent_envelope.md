---
name: repo-tauri-agent-envelope
description: tools/tauri-agent ActionResult delivery contract (status is not a retry signal) and how to get an attributable tsc/bun-test run while other agents edit the package
metadata:
  type: project
---

Facts about `tools/tauri-agent`'s MCP envelope and verifying changes to it.

- **`ActionResult.status` is NOT the retry signal; `ActionResult.delivery` is.** `delivery.retrySafe`
  is true **only** at phase `not_started` — nothing else is ever safe to re-send, because any posted
  CGEvent may have partly landed. The four phases are `not_started` → `input_started` →
  `input_completed` → `postconditions_completed`, tracked across the stages of `finishAction`
  (`src/mcp/tools/actionPipeline.ts`); `deliveryFor()` in `src/mcp/envelope.ts` is the single place
  that derives `retrySafe` / `mayHaveSideEffects` / `inputCompleted` from a phase. Do not add a
  second derivation site.
- **An `error` field means "the action did not happen".** After a delivered input, a settle,
  effects or screenshot failure comes back `status:"warning"` with **no** `error`, the cause in
  `data.postconditionError` and `delivery.evidenceIncomplete:true`. Turning that back into an
  `error` is precisely what makes a caller (or `scripts/smoke-actions.ts`) double-apply a Save or a
  Delete. `defineTool`'s `isError` is derived from `status`, so a delivered failure is also not an
  MCP protocol error.
- `okResult` / `errorResult` default `delivery` to `not_started` — correct for every query tool
  (`ui_*`, `wait_for`, `observe_*`, `session_*`, `window_*`), which post nothing. `errorResult`
  takes an optional 4th `delivery` argument for the pipeline's failure branches.
- `recover()` must keep `releaseAll()` first and the `BRIDGE_WEDGED` → `reconnectOnce` hop in
  **every** branch; both run before the phase decides error-vs-warning.
- **Attributable verification while other agents edit the package.** `tools/tauri-agent` is
  self-contained apart from repo-root `node_modules` and one import of
  `../../../src/viewport/engine/navInput.ts` (`tests/wheelClass.test.ts`; `tests/config.test.ts`
  also walks to the repo root). So: `cp -R tools/tauri-agent <scratch>/tools/tauri-agent`, symlink
  `<scratch>/node_modules` and `<scratch>/src` at the real ones, `git show HEAD:<path> >` every file
  another package is mid-edit on, then run `bunx tsc --noEmit -p tools/tauri-agent` and
  `TAURI_AGENT_REQUIRE_HELPER=1 bun test` from the scratch copy. That separates your failures from
  theirs without touching their working tree.
