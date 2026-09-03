/*
 * Projected-geometry banner (WP-P) — the sketch chrome's second row.
 *
 * A plain CHILD of `SketchChromeBar`, not its own contribution: the editor mount
 * order is a frozen contract (`src/test/contracts/shellContract.ts`) and this
 * row's whole lifetime is "a sketch with projections is open", which is exactly
 * the lifetime `SketchChromeBar` already supplies. It is the same relationship
 * `ConstraintMenu` and `SketchErrorPulse` have with that row.
 *
 * Shown whenever the OPEN session holds projections — not only when they are
 * stale. Detach and Update are the only way to unpick a projection, and gating
 * them behind a staleness verdict would leave a user who projected the wrong
 * edge with no way back. Staleness re-tints the row and adds the reason.
 */
import { useMemo, useRef } from "react";
import { createClient } from "@/ipc/client";
import { useDocumentStore } from "@/stores/documentStore";
import { useSketchStore } from "@/stores/sketchStore";
import { detachProjection, updateProjection } from "@/tools/sketch/sketchService";
import { cn } from "@/ui/cn";
import { projectionStaleVerdict } from "./projectionStale";

export function ProjectionBanner() {
  const session = useSketchStore((s) => s.session);
  const features = useDocumentStore((s) => s.features);
  const stale = useMemo(
    () => projectionStaleVerdict(features, session?.sketchId),
    [features, session?.sketchId],
  );

  // Same lazy-ref shape as `ConstraintContextChips`: one client per mount, and a
  // construction failure (no backend at all) leaves the actions inert rather
  // than taking the sketch chrome down with it.
  const clientRef = useRef<ReturnType<typeof createClient> | null>(null);
  if (!clientRef.current) {
    try {
      clientRef.current = createClient();
    } catch {
      clientRef.current = null;
    }
  }

  const sketchId = session?.sketchId;
  const projected = session?.projections ? Object.keys(session.projections).length : 0;
  if (!sketchId || projected === 0) return null;

  const client = clientRef.current;
  const label = stale
    ? `Projected geometry is out of date${stale.count > 0 ? ` (${stale.count} entities)` : ""}`
    : `${projected} projected ${projected === 1 ? "entity" : "entities"}`;

  return (
    <div
      data-testid="projection-banner"
      data-stale={stale ? "true" : "false"}
      className={cn(
        "flex h-[30px] items-center gap-2 rounded-b-md border border-t-0 px-3 text-[12px]",
        stale
          ? "border-[color:var(--banner-warn-line)] bg-[var(--banner-warn-bg)] text-warn"
          : "border-sketch-chrome-border bg-sketch-chrome text-ink-3",
      )}
    >
      <span
        aria-hidden="true"
        className={cn("h-[7px] w-[7px] shrink-0 rounded-full", stale ? "bg-warn" : "bg-sketch-projected")}
      />
      <span className="whitespace-nowrap font-medium" title={stale?.message}>
        {label}
      </span>
      <button
        type="button"
        data-testid="projection-update"
        title="Re-cut every projected source against the current model"
        disabled={!client}
        onClick={() => client && void updateProjection(client, sketchId)}
        className="rounded-full px-2 py-0.5 font-medium text-accent hover:bg-hover-3"
      >
        Update
      </button>
      <button
        type="button"
        data-testid="projection-detach"
        title="Unlock the projected geometry and stop tracking its source. The shape stays."
        disabled={!client}
        onClick={() => client && void detachProjection(client, sketchId)}
        className="rounded-full px-2 py-0.5 text-ink-5 hover:bg-hover-3"
      >
        Detach
      </button>
    </div>
  );
}
