/*
 * RepairMarkerOverlay — the world-anchored crosshair for the repair candidate the
 * user is hovering in the repair panel (H9).
 *
 * `repairStore.hoveredWorldPos` was authored in M4b as a clean DATA seam with the
 * comment "a future engine marker may subscribe to this"; it had ZERO consumers
 * until now, so hovering a candidate told the user nothing about WHERE on the
 * model it sits — the single most useful thing when choosing between two edges
 * that score 0.91 and 0.89.
 *
 * Same split as MeasureOverlay: content is React, POSITIONING is imperative. The
 * chip owns a plain DOM host the engine appends to its overlay layer and
 * transforms against the world anchor every frame, and we `createPortal` into
 * that host so React never manages the moved node (the "removeChild: not a child"
 * reconciliation crash). Nothing here is interactive — a marker that could eat a
 * pick would fight the panel it belongs to.
 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useRepairStore } from "@/stores/repairStore";
import { useViewportEngine } from "@/viewport/engineBridge";

/** Engine overlay-registry id. One marker at a time, so one fixed key. */
const MARKER_ID = "repair-marker";

export function RepairMarkerOverlay() {
  const world = useRepairStore((s) => s.hoveredWorldPos);
  const engine = useViewportEngine();
  // A plain DOM host, created once; the engine owns its DOM position.
  const [host] = useState(() => {
    const el = document.createElement("div");
    el.dataset.testid = "repair-marker";
    return el;
  });
  // Re-register whenever the ANCHOR moves — the engine stores the world point at
  // mount time, so a new position is a new registration (MeasureChip's rule).
  const anchorKey = world ? world.join(",") : "";

  useEffect(() => {
    if (!engine || !world) return;
    engine.mountChip(MARKER_ID, host, world);
    return () => engine.unmountChip(MARKER_ID, host);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine, anchorKey, host]);

  if (!world) return null;

  return createPortal(
    <div
      aria-hidden="true"
      className="pointer-events-none relative h-4 w-4 -translate-x-1/2 -translate-y-1/2"
    >
      {/* Crosshair: two hairlines + a ring, all on the warn ramp the repair
          panel/banner already use. No new tokens, no raw hex. */}
      <span className="absolute left-1/2 top-0 h-full w-px -translate-x-1/2 bg-warn" />
      <span className="absolute left-0 top-1/2 h-px w-full -translate-y-1/2 bg-warn" />
      <span className="absolute inset-[3px] rounded-full border border-warn bg-warn-surface" />
    </div>,
    host,
  );
}
