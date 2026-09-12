---
name: sketch-sticky-toggle-hint-pattern
description: Where to put shared toggle+statusHint logic for sketch draw modifiers, and where the per-sketch reset for constructionMode landed (WP-U8/D-2)
metadata:
  type: project
---

`src/tools/sketch/sketchService.ts` is the right home for a helper shared between the `X` shortcut (`src/shortcuts/useShortcuts.ts`) and a toolbar button (`src/features/toolbar/FloatingToolbar.tsx`) — it already imports both `sketchStore` and `viewportStore` and is React-free, so both call sites (a plain function and a JSX onClick) can import from it without a cycle. Example: `toggleConstructionModeWithHint()`.

`sketchStore.constructionMode` is now PER-SKETCH (D-2, 2026-09-11): `SketchController.openSession` calls `sketchStore.getState().resetConstructionMode()` right after `setSession`, so it resets on every sketch ENTRY (not on exit) — this matters for the reset test, which must reopen a session to observe it, not just close one.

There is no generic chip/badge primitive in `src/ui/` — only `SourceBadge.tsx`, which is domain-specific to the component library (`SourceBadgeKind`). For an ad-hoc persistent indicator chip, `src/features/tasks/TasksChip.tsx`'s inline class pattern (`rounded-[11px] border border-border-strong bg-surface ... text-accent`) is the closest existing reusable shape to copy — not a component to import.
