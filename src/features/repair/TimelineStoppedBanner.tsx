/*
 * "Timeline stopped" banner (HISTORY-HARDEN H7b).
 *
 * A halted rebuild used to be visible ONLY as a tinted row inside a panel the user
 * has to open a body to reach: the viewport just quietly stops changing. This says
 * where it stopped, how much is not built because of it, and offers the two things
 * that actually unblock it.
 *
 * DERIVED, never carried on the wire: the halt point is the first `error` /
 * `needsRepair` feature and the blocked count is the dirty tail behind it
 * (`timelineHalt`). No projection field, no protocol change — the same rule
 * `HistoryList` tints its rows with (`haltIndexOf`), so the row the banner names is
 * always the row that is red.
 */
import { createClient } from "@/ipc/client";
import { useDocumentStore, type FeatureMeta } from "@/stores/documentStore";
import { useRepairStore } from "@/stores/repairStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { suppressFeature } from "@/features/inspector/historyActions";
import { haltIndexOf } from "@/features/inspector/HistoryList";
import { cn } from "@/ui/cn";

/** Where a timeline stopped, and how much is not built because of it. */
export interface TimelineHalt {
  index: number;
  feature: FeatureMeta;
  /** Steps AFTER the halt that never rebuilt (the dirty tail). */
  notRebuilt: number;
}

/**
 * The halt state of a feature timeline, or `null` when nothing stopped it.
 *
 * `notRebuilt` counts only the `dirty` rows behind the halt: a row that is `ok`
 * after it was rebuilt (it did not depend on the failed op), and counting it would
 * overstate the damage on exactly the documents where the damage is smallest.
 */
export function timelineHalt(features: FeatureMeta[]): TimelineHalt | null {
  const index = haltIndexOf(features);
  if (index < 0) return null;
  const notRebuilt = features.slice(index + 1).filter((f) => f.status === "dirty").length;
  return { index, feature: features[index], notRebuilt };
}

/** Select the halting row and bring it into view (the panel may be scrolled). */
function revealHaltRow(id: string): void {
  selectionStore.getState().set([{ kind: "feature", id }]);
  // The row only exists once the inspector has re-rendered into its FEATURE state,
  // so the scroll waits a frame. `scrollIntoView` is absent in jsdom — optional-call.
  requestAnimationFrame(() => {
    document
      .querySelector(`[data-testid="history-row-${id}"]`)
      ?.scrollIntoView?.({ block: "nearest" });
  });
}

/** Forget the worker's crash-circuit poison keys (H2), reporting the count. */
async function resetFailureBreaker(): Promise<void> {
  const status = viewportStore.getState().setStatusHint;
  try {
    const cleared = await createClient().clearWorkerCircuit();
    status(
      cleared === 0
        ? "Failure breaker was already clear"
        : `Failure breaker reset (${cleared} blocked ${cleared === 1 ? "step" : "steps"})`,
    );
  } catch (e) {
    status(`Reset failed: ${e instanceof Error ? e.message : String(e)}`, {
      severity: "error",
      sticky: true,
    });
  }
}

/**
 * Red pill naming the halt point, mirroring `RepairBanner`'s chrome (dot + text,
 * same rounded-full/shadow treatment) so the two read as one family of attention
 * affordances — in the error tone rather than the warn one, because a halted
 * timeline is a hard stop rather than something to get to eventually.
 */
export function TimelineStoppedBanner() {
  const features = useDocumentStore((s) => s.features);
  // Stacks BELOW the repair banner when both are up (they share the top-centre
  // slot); alone, it takes the same top slot the repair pill would have.
  const repairShowing = useRepairStore((s) => s.items.length > 0);
  const halt = timelineHalt(features);
  if (!halt) return null;

  const blocked =
    halt.notRebuilt === 0
      ? ""
      : ` — ${halt.notRebuilt} feature${halt.notRebuilt === 1 ? "" : "s"} not rebuilt`;

  return (
    <div
      data-testid="timeline-stopped-banner"
      className={cn(
        "absolute left-1/2 z-30 flex -translate-x-1/2 items-center gap-2 rounded-full border border-border bg-surface py-1.5 pl-3 pr-1.5 text-[12.5px] shadow-panel",
        repairShowing ? "top-[52px]" : "top-3",
      )}
    >
      <span aria-hidden="true" className="h-[7px] w-[7px] rounded-full bg-traffic-close" />
      <button
        type="button"
        data-testid="timeline-stopped-goto"
        onClick={() => revealHaltRow(halt.feature.id)}
        className="font-medium text-traffic-close hover:underline"
      >
        {`Timeline stopped at ${halt.feature.label}${blocked}`}
      </button>
      <span aria-hidden="true" className="h-[14px] w-px bg-border" />
      <button
        type="button"
        data-testid="timeline-stopped-suppress"
        title="Skip this feature so the rest of the timeline can rebuild"
        onClick={() => void suppressFeature(halt.feature.id, true)}
        className="rounded-full px-2 py-0.5 font-medium text-accent hover:bg-hover-3"
      >
        Suppress feature
      </button>
      <button
        type="button"
        data-testid="timeline-stopped-reset-breaker"
        title="Forget the worker's blocked-step list. Use after fixing something the document cannot see — a repaired input file, or a rebuilt worker."
        onClick={() => void resetFailureBreaker()}
        className="rounded-full px-2 py-0.5 text-[11.5px] text-ink-5 hover:bg-hover-3"
      >
        Reset failure breaker
      </button>
    </div>
  );
}
