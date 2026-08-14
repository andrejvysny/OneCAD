/*
 * Sketch ORIGIN relation probe (U0 red evidence).
 *
 * The UX audit's trust break: a rectangle drawn from the sketch origin, with 60
 * and 40 dimensions applied, still reports DOF 2. D14 fixed the contradictory
 * LABEL ("Under-constrained · DOF 0"), but the underlying cause was never
 * touched — the origin is not a snap target and no relation is ever persisted:
 *
 *   - `snapEngine.ts` ranks endpoint / midpoint / center / quadrant /
 *     intersection / onCurve / grid / align / polar. There is no origin tier, so
 *     landing on (0,0) is indistinguishable from landing on any grid node.
 *   - `autoConstrain.ts` infers H/V, perpendicular, parallel, tangent,
 *     concentric, equal-radius and endpoint coincidence — never an anchor to the
 *     sketch origin.
 *
 * Two free translation DOF therefore survive every dimension the user adds, and
 * "start at the origin" silently means nothing. RED until U7 makes the origin a
 * real snap target that persists a visible relation ON ACCEPT (and only then —
 * geometry drawn NEAR the origin must never be auto-fixed).
 */
import { describe, it, expect } from "vitest";

import type { SketchEntity } from "@/ipc/types";
import { inferConstraints } from "./autoConstrain";
import { computeSnap, type SnapOptions } from "./snapEngine";

const OPTS: SnapOptions = {
  gridStep: 5,
  pixelWorld: 0.1,
  // Grid off: otherwise a grid node at (0,0) masks the missing origin tier and
  // this probe would pass for the wrong reason.
  enableGrid: false,
  enableGuideLines: true,
  enableGuidePoints: true,
  suppress: false,
};

let n = 0;
const nextConstraintId = (): string => `c${++n}`;
/** The origin was offered as a snap and the user landed on it — see `InferOptions`. */
const ACCEPTED = { nextConstraintId, originAccepted: true };

/** A deliberately oblique line so `inferHV` cannot fire — anything this probe
 *  sees in the output is an origin relation, not an orientation one. */
const obliqueFromOrigin: SketchEntity = {
  id: "l1",
  type: "Line",
  p0: [0, 0],
  p1: [60, 40],
} as SketchEntity;

describe("the sketch origin is a real, persistable reference", () => {
  it("snaps to the origin when the cursor is within reach of it", () => {
    const snap = computeSnap({ x: 0.3, y: -0.2 }, [], OPTS);

    expect(snap.snapped).toBe(true);
    expect(snap.point).toEqual({ x: 0, y: 0 });
    expect(snap.label).toBe("Origin");
  });

  it("control: an oblique line away from the origin infers nothing", () => {
    const away = { ...obliqueFromOrigin, p0: [100, 100], p1: [160, 140] } as SketchEntity;
    expect(inferConstraints([away], [], ACCEPTED)).toEqual([]);
  });

  it("anchors an endpoint accepted on the origin with a persisted relation", () => {
    const out = inferConstraints([obliqueFromOrigin], [], ACCEPTED);

    const anchor = out.find((c) => c.type === "Coincident" || c.type === "Fixed");
    expect(anchor, `origin endpoint must persist a relation, got ${JSON.stringify(out)}`).toBeDefined();
    expect(anchor?.entities).toContain("l1");
  });

  it("never auto-fixes geometry when the origin was not offered as a snap", () => {
    // Point snapping off ⇒ the origin is not in the snap ladder, so a coordinate
    // that happens to be (0,0) is a coincidence, not an accepted relation.
    const out = inferConstraints([obliqueFromOrigin], [], { nextConstraintId });
    expect(out.find((c) => c.type === "Fixed")).toBeUndefined();
  });
});
