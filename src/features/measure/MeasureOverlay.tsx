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
import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useMeasureStore } from "@/stores/measureStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import { formatArea, formatLengthWithUnit } from "@/units/format";
import { planePairLabel } from "./measurePresentation";
import type { MeasurePick, MeasureSummary } from "@/tools/modelTools/measureTool";
import {
  measurementAnnotationStore,
  useMeasurementAnnotationStore,
  type MeasurementAnnotationRecord,
  type MeasurementAnnotationSlot,
} from "@/stores/measurementAnnotationStore";

type Vec3 = [number, number, number];
type AnnotationIdentities = Record<MeasurementAnnotationSlot, string | null>;

/**
 * The reading for one picked element. A FACE reports its area, an EDGE its arc
 * length — both straight from the kernel descriptor's `magnitude`. Anything else
 * (a vertex, a whole body) is labelled by kind with a bare value rather than
 * captioned with a unit the quantity may not have.
 *
 * A circle edge or cylindrical face additionally carries `radius` (WP-U11, from
 * the companion `classifyElement` read) and leads with the reading a CAD user
 * actually wants there — Ø for a circle (diameter, the callout convention),
 * R for a cylinder — ahead of the arc length / area on the same line.
 */
export function pickLabel(pick: MeasurePick): string {
  if (pick.kind === "edge" && pick.radius !== null) {
    return `Ø ${formatLengthWithUnit(pick.radius * 2)} · Length ${formatLengthWithUnit(pick.magnitude)}`;
  }
  if (pick.kind === "face" && pick.radius !== null) {
    return `R ${formatLengthWithUnit(pick.radius)} · Area ${formatArea(pick.magnitude)}`;
  }
  if (pick.kind === "face") return `Area ${formatArea(pick.magnitude)}`;
  if (pick.kind === "edge") return `Length ${formatLengthWithUnit(pick.magnitude)}`;
  return `${pick.kind} ${pick.magnitude}`;
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
  return `Center ↔ center ${formatLengthWithUnit(summary.distance)}`;
}

/** Per-axis separation, second pick minus first. */
export function deltaLabel(summary: MeasureSummary): string {
  const [dx, dy, dz] = summary.delta;
  return `ΔX ${formatLengthWithUnit(dx)}  ΔY ${formatLengthWithUnit(dy)}  ΔZ ${formatLengthWithUnit(dz)}`;
}

/** Midpoint of the two picked centres — where the pair label sits. */
function midpoint(a: Vec3, b: Vec3): Vec3 {
  return [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2];
}

function pickIdentity(pick: MeasurePick): string {
  return `${pick.kind}:${pick.bodyId}:${pick.elementId}`;
}

function pairedIdentity(picks: readonly MeasurePick[], summary: MeasureSummary | null): string | null {
  return summary && picks.length === 2 ? `${pickIdentity(picks[0])}|${pickIdentity(picks[1])}` : null;
}

function currentRecord(record: MeasurementAnnotationRecord, identity: string | null): MeasurementAnnotationRecord {
  return record.identity === identity
    ? record
    : { identity, manuallyHidden: false, pinnedScreenPosition: null, placement: "unknown", hasLivePosition: false };
}

/** One world-anchored label. `id` keys it in the engine's overlay registry. */
function MeasureChip({
  id,
  world,
  children,
  testid,
  slot,
  identity,
  annotation,
  priority,
}: {
  id: string;
  world: Vec3;
  children: React.ReactNode;
  testid: string;
  slot: MeasurementAnnotationSlot;
  identity: string;
  annotation: MeasurementAnnotationRecord;
  priority: number;
}) {
  const engine = useViewportEngine();
  // A plain DOM host, created once; the engine owns its DOM position.
  const [host] = useState(() => {
    const el = document.createElement("div");
    el.dataset.testid = testid;
    el.dataset.annotationId = id;
    return el;
  });
  const anchorKey = world.join(",");
  const screenPosition = annotation.pinnedScreenPosition;
  const onPlacementStatus = useCallback(
    (status: "visible" | "no-space" | "unknown") => {
      if (measurementAnnotationStore.getState().slots[slot].identity === identity) {
        measurementAnnotationStore.getState().setPlacement(slot, status);
      }
    },
    [identity, slot],
  );
  const onScreenPosition = useCallback(
    (position: { x: number; y: number } | null) => {
      if (measurementAnnotationStore.getState().slots[slot].identity === identity) {
        measurementAnnotationStore.getState().setLivePosition(slot, position);
      }
    },
    [identity, slot],
  );

  useEffect(() => {
    if (!engine) return;
    engine.mountChip(id, host, world, {
      screenPosition: screenPosition ?? undefined,
      constrainToSafeRect: true,
      annotation: {
        priority,
        pinned: screenPosition !== null,
        onPlacementStatus,
        onScreenPosition,
      },
    });
    return () => {
      engine.unmountChip(id, host);
      if (measurementAnnotationStore.getState().slots[slot].identity === identity) {
        measurementAnnotationStore.getState().setLivePosition(slot, null);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine, id, anchorKey, host, priority, screenPosition, onPlacementStatus, onScreenPosition]);

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
  const annotations = useMeasurementAnnotationStore((s) => s.slots);
  // The labels are mounted imperatively, but their text remains React-owned;
  // subscribe so a unit preference change updates an armed measurement.
  useSettingsStore((s) => s.displayUnit);
  const identities = useMemo<AnnotationIdentities>(() => ({
    a: picks[0] ? pickIdentity(picks[0]) : null,
    b: picks[1] ? pickIdentity(picks[1]) : null,
    pair: pairedIdentity(picks, summary),
  }), [picks, summary]);

  useEffect(() => {
    measurementAnnotationStore.getState().reconcile(identities);
  }, [identities]);
  useEffect(() => () => measurementAnnotationStore.getState().reset(), []);

  if (picks.length === 0) return null;

  return (
    <>
      {picks.map((pick, i) => (
        !currentRecord(annotations[i === 0 ? "a" : "b"], identities[i === 0 ? "a" : "b"]).manuallyHidden && <MeasureChip
          // Index-keyed on purpose: the two slots are POSITIONAL ("first pick" /
          // "second pick"), and keeping the ids stable across a replacement lets
          // the overlay driver re-anchor an existing chip instead of tearing one
          // down and mounting another on every click.
          key={`measure-slot-${i}`}
          id={`measure:${i}`}
          testid={`measure-label-${i}`}
          world={pick.center}
          slot={i === 0 ? "a" : "b"}
          identity={identities[i === 0 ? "a" : "b"]!}
          annotation={currentRecord(annotations[i === 0 ? "a" : "b"], identities[i === 0 ? "a" : "b"])}
          priority={i === 0 ? 100 : 200}
        >
          <span>{pickLabel(pick)}</span>
        </MeasureChip>
      ))}
      {summary && picks.length === 2 && !currentRecord(annotations.pair, identities.pair).manuallyHidden && (
        <MeasureChip
          id="measure:sum"
          testid="measure-label-sum"
          world={midpoint(picks[0].center, picks[1].center)}
          slot="pair"
          identity={identities.pair!}
          annotation={currentRecord(annotations.pair, identities.pair)}
          priority={300}
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
