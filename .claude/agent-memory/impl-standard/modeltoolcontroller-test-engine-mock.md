---
name: modeltoolcontroller-test-engine-mock
description: ModelToolController.*.test.ts needs a full engine stub — a bare `{} as ViewportEngine` throws inside the cross-tool cancel sweep
metadata:
  type: project
---

`new ModelToolController({ engine: {} as ViewportEngine, ... })` throws as soon as
`toolStore.setState({ modelTool: ... })` or `.dispose()` runs: `onToolChange` runs a full
cancel sweep across EVERY tool FSM (`cancelPreview`, `cancelFillet`, `cancelBoolean`,
`cancelRevolve`, `cancelShell`, ..., plus `dispose()`'s `endAlign`), each calling several
engine methods unconditionally, even when the test only exercises one tool (e.g. Measure).
This is true on current `master` (confirmed via `git stash` on an unmodified
`ModelToolController.measureCurrentness.test.ts`), not something a specific diff introduced.

Fix: a local `makeEngineMock()` returning a `Proxy` that lazily caches a `vi.fn()` per
accessed property, cast `as unknown as ViewportEngine` — sibling files
(`ModelToolController.align.test.ts`, `.commit.test.ts`, etc.) instead hand-enumerate every
method they need; the Proxy form avoids re-enumerating the sweep's engine surface and stays
correct as the sweep grows. Safe here because none of these guard tests assert on engine
calls; a suite that does should keep enumerated `vi.fn()`s per method like the sibling files.
