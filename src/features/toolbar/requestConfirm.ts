/*
 * The single confirm gate for an armed model operation (spec §4.2).
 *
 * Its own module rather than a member of the label or the strip: five unrelated
 * surfaces enter a confirm — the viewport label's field Enter, the operation
 * strip's Done, the inspector's edge-op and draft fields, the hole cluster and
 * the gear panel — and a gate that lived in any one of them would make the
 * others import a component module for a store read.
 */
import { toolChipStore } from "@/stores/toolChipStore";

/**
 * The one gate every confirm entry (field Enter, Done, ✓) passes, read FRESH
 * from the store at call time rather than from the render that built the handler.
 *
 * It refuses only what is SETTLED: `pending` validation passes because
 * `commitFillet` deliberately awaits the in-flight range check, and a stale
 * `previewLifecycle` of `invalid` is not consulted because a throttled status
 * would silently drop a corrected Enter — the controller refuses those with a
 * message instead.
 */
export function requestConfirm(): void {
  const s = toolChipStore.getState();
  if (s.validation.status === "invalid") return;
  if (s.retainedCommitFailure !== null) return;
  if (s.previewLifecycle.status === "applying") return;
  s.onConfirm?.();
}
