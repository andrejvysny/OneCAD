/*
 * MeasureOverlay — the floating labels for the read-only Measure tool (W2-B).
 *
 * Same split as `ModelToolChips`: content is React, POSITIONING is imperative.
 * Each label owns a plain DOM host node that the engine appends to its HTML
 * overlay and transforms against a world anchor every frame, so panning/orbiting
 * moves the labels with the geometry at zero React cost. We `createPortal` into
 * that host so React never manages the moved node itself (the "removeChild: not
 * a child" reconciliation crash).
 *
 * Unlike the tool chips there may be THREE labels at once (pick A, pick B, and
 * the pair summary), so each takes a distinct `mountChip` id — the overlay
 * driver is an id-keyed registry, which is what makes multi-chip work.
 *
 * Nothing here is interactive: measuring writes nothing, so there is no ✓/✕ and
 * the labels stay `pointer-events-none` and out of the way of picking.
 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useMeasureStore } from "@/stores/measureStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import { AREA_SUFFIX, LENGTH_SUFFIX, formatArea, formatLength } from "@/units/format";
import { planePairLabel } from "./MeasurePanel";
import type { MeasurePick, MeasureSummary } from "@/tools/modelTools/measureTool";

type Vec3 = [number, number, number];

/**
 * The reading for one picked element. A FACE reports its area, an EDGE its arc
 * length — both straight from the kernel descriptor's `magnitude`. Anything else
 * (a vertex, a whole body) is labelled by kind with a bare value rather than
 * captioned with a unit the quantity may not have.
 */
export function pickLabel(pick: MeasurePick): string {
  if (pick.kind === "face") return `Area ${formatArea(pick.magnitude)}`;
  if (pick.kind === "edge") return `Length ${formatLength(pick.magnitude)} ${LENGTH_SUFFIX}`;
  return `${pick.kind} ${formatLength(pick.magnitude)}`;
}

/**
 * The pair reading.
 *
 * "Center ↔ center" is not decoration: `ElementInfo.center` is the kernel's
 * BOUNDING-BOX centre, not a centroid and not the closest point, so the label
 * has to say which distance this is. Calling it "Distance" would quietly imply
 * the minimum separation between the two elements — a different quantity that
 * this tool does not compute.
 */
export function summaryLabel(summary: MeasureSummary): string {
  return `Center ↔ center ${formatLength(summary.distance)} ${LENGTH_SUFFIX}`;
}

/** Per-axis separation, second pick minus first. */
export function deltaLabel(summary: MeasureSummary): string {
  const [dx, dy, dz] = summary.delta;
  return `ΔX ${formatLength(dx)}  ΔY ${formatLength(dy)}  ΔZ ${formatLength(dz)}`;
}

/** Midpoint of the two picked centres — where the pair label sits. */
function midpoint(a: Vec3, b: Vec3): Vec3 {
  return [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2];
}

/** One world-anchored label. `id` keys it in the engine's overlay registry. */
function MeasureChip({
  id,
  world,
  children,
  testid,
}: {
  id: string;
  world: Vec3;
  children: React.ReactNode;
  testid: string;
}) {
  const engine = useViewportEngine();
  // A plain DOM host, created once; the engine owns its DOM position.
  const [host] = useState(() => {
    const el = document.createElement("div");
    el.dataset.testid = testid;
    return el;
  });
  const anchorKey = world.join(",");

  useEffect(() => {
    if (!engine) return;
    engine.mountChip(id, host, world);
    return () => engine.unmountChip(id, host);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine, id, anchorKey, host]);

  return createPortal(
    <div className="pointer-events-none inline-flex flex-col items-start gap-0.5 rounded-md border border-border bg-surface px-2 py-1 font-mono text-[11.5px] text-ink-2 shadow-panel">
      {children}
    </div>,
    host,
  );
}

export function MeasureOverlay() {
  const picks = useMeasureStore((s) => s.picks);
  const summary = useMeasureStore((s) => s.summary);

  if (picks.length === 0) return null;

  return (
    <>
      {picks.map((pick, i) => (
        <MeasureChip
          // Index-keyed on purpose: the two slots are POSITIONAL ("first pick" /
          // "second pick"), and keeping the ids stable across a replacement lets
          // the overlay driver re-anchor an existing chip instead of tearing one
          // down and mounting another on every click.
          key={`measure-slot-${i}`}
          id={`measure:${i}`}
          testid={`measure-label-${i}`}
          world={pick.center}
        >
          <span>{pickLabel(pick)}</span>
        </MeasureChip>
      ))}
      {summary && picks.length === 2 && (
        <MeasureChip
          id="measure:sum"
          testid="measure-label-sum"
          world={midpoint(picks[0].center, picks[1].center)}
        >
          <span data-testid="measure-distance">{summaryLabel(summary)}</span>
          <span className="text-ink-5">{deltaLabel(summary)}</span>
          {/* The plane relationship, when both picks are planes (WP-C1). It sits
              on the pair chip as well as the panel because it belongs to the two
              picks the chip already straddles — reading it in mid-air between the
              faces is the whole point of a floating label. */}
          {planePairLabel(summary) && (
            <span data-testid="measure-angle">{planePairLabel(summary)}</span>
          )}
        </MeasureChip>
      )}
    </>
  );
}

/** Re-exported so a test can assert the unit without re-deriving the string. */
export { AREA_SUFFIX };
