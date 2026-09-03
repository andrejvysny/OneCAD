---
name: frontend-gotchas
description: OneCAD frontend implementation gotchas — engine test doubles, frozen contracts vs new tools, status-hint severities, engine bridge vs deps.engine
metadata:
  type: project
---

Durable facts for anyone adding a frontend feature to OneCAD. Learned while
building the Project-edges tool (WP-P).

**Why:** each of these cost a full test cycle to discover; none is stated in
CLAUDE.md and none is derivable without running the suite.

**How to apply:** read before adding a viewport method, a tool, or a status hint.

- Adding a method to `ViewportEngine` breaks ~25 hand-rolled engine doubles in
  `src/**/*.test.ts` (they are cast `as unknown as ViewportEngine`, so `tsc` is
  blind to the gap). Grep the doubles by an existing sibling method name and add
  the new key to each; there is no shared factory.
- `viewportStore`'s `StatusSeverity` is only `"info" | "error"` — there is no
  `warning` rung. A partial refusal has to be reported as `error`.
- `sketchService.ts` reaches the viewport through the module singleton
  `getViewportEngine()` (engine BRIDGE), not through `SketchController`'s
  `deps.engine`. A controller test that asserts on an engine double must also
  register it via `setViewportEngine()` or the write goes nowhere.
- A new sketch/model tool must be added in five places or something breaks:
  the `SketchTool`/`ModelTool` union, `ModelingSketchTools`/`ModelingModelTools`
  in `modules/modeling/ids.ts` (`satisfies Record<Tool, string>` — exhaustive),
  `SKETCH_TOOL_DESCRIPTORS`/`MODEL_TOOL_DESCRIPTORS`, `MODELING_BINDINGS`, and
  `activateTool`'s `SKETCH_ONLY`/`MODEL_ONLY`.
- `toolbarHidden: true` on a descriptor is the supported "registered but no
  toolbar button" split — it keeps the frozen `toolbarContract.ts` intact while
  the tool still resolves its shortcut and mirrors into `platform.toolHost`.
  Skipping the descriptor entirely instead makes `register.test.ts`'s
  "every tool binding reaches its registered tool" probe fail.
- `useShortcuts.test.tsx`'s contributed-chord block needs a key no built-in
  binding owns; it currently uses `q` (it was `j` until Project-edges took it).
- Adding a registered PANEL requires amending the frozen
  `src/test/contracts/shellContract.ts`. To avoid that, mount new chrome as a
  plain child of an existing registered panel — the documented precedent is
  `ConstraintMenu`/`SketchErrorPulse` inside `SketchChromeBar`.
