---
name: fp-s13-setmode-default-blast-radius
description: toolStore.setMode("sketch") bare-entry default change (Line→Select) breaks ~10 files / 69 tests in src/tools/sketch/** that assume the old default
metadata:
  type: project
---

Changing `toolStore.ts`'s `setMode("sketch", id?, opts?)` bare-entry default from
`sketchTool: "line"` to `"select"` (FP-S13, docs/qa/UX_REVIEW_2026-09-14.md) is
correct for the two real UX call sites (`activateTool.ts`'s `tool === "sketch"`
branch, `useShortcuts.ts`'s `enterSketch` action) once they're updated to pass
`opts.tool: "line"` explicitly for the New-Sketch flow — both stay green.

But `src/tools/sketch/*.test.ts` (arc, construction, cursor, datum, draw,
liveDim, planePick, queue, snapClear, switch — 10 files, 69 tests as of
2026-09-15) call bare `setMode("sketch", "sketchId")` or `setMode("sketch")`
directly in their own `beforeEach`/helpers, several with an explicit comment
`// default tool = line`. Under the new default those tests silently arm
Select instead of Line, and a few actively crash: `SketchController`'s
`onPointerUp` checks `this.selectActive` BEFORE `this.planePicking`, and
`enter()`'s `await Promise.resolve()` at its top means the sync `toolStore`
subscriber's `selectMachine(s.sketchTool)` call runs (and can set
`selectActive = true`) *before* `enter()` reaches `beginPlanePick()` — so a
bare/default-select entry into the plane-pick path routes clicks through
`onSelectPointerDown`/`onSelectPointerUp` instead of resolving the plane pick.

**How to apply:** if you touch `toolStore.ts`'s `setMode` default again, grep
`src/tools/sketch/*.test.ts` for bare `setMode("sketch"` calls first — they
need `{ tool: "line" }` added to keep exercising what they claim to exercise.
Whoever owns `src/tools/sketch/**` still needs to do this; it was left
unfixed by design (out of scope, directory owned by a concurrent agent) when
FP-S13 landed. See [[wpu3-clamp-hint-typed-commit-only]] for a similar
cross-agent boundary case.
