---
name: toolchip-confirm-gate-requires-context
description: chip-confirm gating in toolChipStore-driven tests requires setContext, not just showXxx — missingRequiredTargetMessage blocks confirm otherwise
metadata:
  type: feedback
---

Any `*ChipCluster.test.tsx` that asserts on `chip-confirm` enabled/disabled must pair the
`toolChipStore.getState().showXxx(...)` call with `setContext(tool, { kind: ..., affectedBodies/hostBodies/bodies: [...] })`
whose referenced body ids exist in `documentStore.getState().bodies`, mirroring what the real
ModelToolController does before showing the chip (e.g. `showHoleChip` in ModelToolController.ts).

**Why:** `activeToolPresentation.ts` `missingRequiredTargetMessage` (~line 192) returns "Tool
targets are unavailable — cancel and reopen the tool" whenever `context` is null or
`context.tool !== s.kind`, which folds into `blocked` and forces `canConfirm` false regardless of
`rawInputErrors`. A test that calls only `showHole(...)` without `setContext` will have every
confirm-related assertion silently pass/fail against this fallback message, not the real validation
gate under test.

**How to apply:** when fixing or writing tests for a toolChip cluster (hole, fillet, shell, pattern,
etc.) that check `chip-confirm` state, add a local helper mirroring the production `show<Tool>Chip`
pairing (see `showHoleWithContext` in `src/features/toolbar/HoleChipCluster.test.tsx`) rather than
calling `showXxx` bare. Seed `documentStore.setState({ bodies: {...} })` first so
`missingRequiredTargetMessage`'s body-existence check passes too.
