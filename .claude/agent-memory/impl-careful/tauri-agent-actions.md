---
name: tauri-agent-actions
description: Acting-tool gotchas in tools/tauri-agent — ref format @s<gen>e<n> is schema-enforced, releaseAll only on the error path, a plain object works as a stub McpServer, and the screenshot capability gate (skip vs refuse)
metadata:
  type: project
---

The pointer/keyboard/wait/observe/debug tools all run through
`src/mcp/tools/actionPipeline.ts` (`beginAction` → `resolveTarget` → `finishAction`).

- **Snapshot refs are `@s<gen>e<n>` and `TargetSchema` enforces `/^@s\d+e\d+$/`.** A fixture or
  hand-written target using the older `@e1` form is rejected by zod before the handler runs, and the
  envelope reports INTERNAL (a zod error), not INVALID_TARGET — easy to misread as a tool bug.
  **Why:** refs are generation-scoped so two snapshots can never issue the same ref.
  **How to apply:** write `@s1e1` in tests; never hand a user a ref from an older snapshot.
- **`input.releaseAll()` runs only on the error path, never after a successful action.** A `finally`
  that always released would make `pointer_down` / `keyboard_down` useless — the hold would end
  before the paired `up`. **How to apply:** keep new verbs inside `finishAction`'s try/catch rather
  than adding their own cleanup.
- **A plain object with a `registerTool` method works as a stub `McpServer`** because `defineTool`
  binds `server.registerTool`. `tests/fixtures/fakeEnv.ts` and `scripts/smoke-actions.ts` both drive
  the real registered handlers that way, so tests exercise shipped code, not internals.
- `NativeCapture.window(windowId, outPath, boundsPt?)` and `.screen(outPath, boundsPt?)` take the
  window bounds as a third argument; omitting it is legal but loses the crop the callers rely on.
- **A refusal thrown INSIDE `finishAction` still records one `releaseAll` input call**, because
  `recover` releases before building the envelope. Only a refusal in `beginAction` (frontmost,
  calibration) leaves `input.calls` empty. Assert on the verb list, not on `toEqual([])`.
- **Screenshots are asymmetric by design**: a policy-driven shot on a session without the Screen
  Recording grant is SKIPPED with a mandatory warning, while an explicit `screenshot:true` or
  `ui_screenshot` is REFUSED with SCREEN_CAPTURE_PERMISSION_DENIED before any input is posted.
  The gate is `session.status().capture` (`CaptureCapability`), set once in `#preflight`.
- `measure()` is exported from `platform/macos/capture.ts` purely as a test seam, and takes an
  injectable grant probe — that is how the denied/degraded paths are exercised on a machine that
  HAS both TCC grants. Never revoke a grant to test; `MacCapture` takes the probe in its ctor.
- The launched app writes its log to `<artifacts>/<session>/app-logs/dev.jsonl` (ONECAD_LOG_DIR);
  an ATTACHED app writes to `<root>/logs/dev.jsonl`. `devJsonlPath()` picks by `status().launched`.
  dev.jsonl is truncated at every app start, so a cursor past EOF means a NEW file, not "no news".
- `scripts/smoke-actions.ts` is the live gate; it moves the real mouse and keyboard, needs ports
  1420/4445 free, and verified 2026-09-13: 14 actions ok, tooltip visible in the hover PNG, view cube
  rotated by the right+Shift drag, `pgrep` empty afterwards.
