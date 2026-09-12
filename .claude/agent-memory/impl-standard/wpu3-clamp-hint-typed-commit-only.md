---
name: wpu3-clamp-hint-typed-commit-only
description: ModelToolController fillet/chamfer typed-value clamp disclosure site is onFilletChip, not the line numbers a brief may cite
metadata:
  type: project
---

`ModelToolController.ts`'s typed-commit path for the fillet/chamfer numeric chip is the
private method `onFilletChip(v)` (search for it, do not trust line numbers from a brief —
they drift as the file grows). It is wired as the `onValue` callback of both
`toolChipStore.getState().showFillet(...)` call sites (fresh arm and re-edit arm).

`guardEdgeOpValue(value)` is the single seam both the chip and the drag path call
(`src/tools/preview/filletRadius.ts` `clampToEdgeOpRange`). It returns the full
`EdgeOpClampResult` (`{value, clamped, reason}`); callers that only need the number use
`.value`. Drag deliberately shows no clamp hint — the live value already discloses it in
real time — so only `onFilletChip` calls the hint publisher.

The mock lane's `analyzeEdgeOpRange` (`src/ipc/mockClient.ts`) ALWAYS returns
`confidence: "none"` (no kernel to measure against) — so a clamp can never fire in
`e2e/filletChamfer.spec.ts` or any mock-lane Playwright spec. Any WP-U3-shaped clamp-disclosure
e2e case must be skipped/omitted with that fact stated, not authored against the mock.

Concurrent WP-U1 (undo/redo) work in this same file adds `completionVerb`/`removedBodyIds`
plumbing to `finishRevolve`/`finishExtrude` — unrelated to the fillet/chamfer chip region
(~3350-4170, ~7580); re-read the file before editing since multiple agents land in this one
file simultaneously.
