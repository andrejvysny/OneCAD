/*
 * DIRECTION DECIDES THE OPERATION (pure, framework-free).
 *
 * Shapr3D's push/pull rule, generalized: drag INTO material and the extrude cuts,
 * drag away and it joins. Before this, the rule only existed for a sketch carrying
 * a `hostFace` — a sketch on a datum plane slicing straight through a body opened
 * on `NewBody` and offered nothing but a one-shot text hint.
 *
 * WHAT THE CALLER SUPPLIES: one probe per direction, each answering "is there a
 * body that way, how far, and did the ray START inside it". The `inside` flag,
 * not a near-zero gap, is what makes this honest — a sketch coplanar with a body's
 * top face reads gap ≈ 0 on BOTH sides, and only the side that goes into the solid
 * exits through a back face.
 *
 * WHAT IT DELIBERATELY DOES NOT DO: guess. No material on the side being dragged,
 * and nothing reachable at this depth, means `NewBody` — the same answer the tool
 * gave before, and one the user can override on the chip. A wrong silent Cut is
 * far worse than a New Body the user re-targets.
 */
import type { BooleanMode } from "./modelToolMachine";

/** What a probe found on ONE side of the sketch plane. */
export interface SideProbe {
  bodyId: string;
  /** Distance from the profile to that body's first surface, in world units. */
  gap: number;
  /** True when the profile starts INSIDE that body (the ray exited a back face). */
  inside: boolean;
}

/** Probes for both directions along the sketch normal. */
export interface MaterialSides {
  /** Along +normal. */
  pos: SideProbe | null;
  /** Along −normal. */
  neg: SideProbe | null;
}

export const NO_MATERIAL: MaterialSides = { pos: null, neg: null };

/**
 * Hysteresis band around the CONTACT distance, in world units.
 *
 * Rule 4 flips to Add the moment the prism reaches a separate body. A pointer
 * hovering at exactly that distance would otherwise strobe Add↔NewBody every
 * frame, and each flip re-sends the previewed op. Entering costs `+HOLD`, leaving
 * costs `−HOLD`, so the two thresholds never coincide. Mirrors
 * `EDGE_OP_FLIP_HOLD`'s reasoning in `modelToolMachine`.
 */
export const CONTACT_FLIP_HOLD = 0.5;

export interface AutoMode {
  mode: BooleanMode;
  targetBodyId: string | null;
}

/**
 * The boolean mode a drag of `depth` resolves to, given what is on each side.
 *
 * `prev` is the currently resolved mode, read for two reasons: a depth of exactly
 * zero HOLDS it (there is no direction to read, and blinking through a third
 * state at every zero crossing would be noise), and rule 4's hysteresis needs to
 * know which side of the threshold it is coming from.
 */
export function autoModeFor(sides: MaterialSides, depth: number, prev: AutoMode): AutoMode {
  if (!Number.isFinite(depth) || depth === 0) return prev;

  const near = depth > 0 ? sides.pos : sides.neg;
  const far = depth > 0 ? sides.neg : sides.pos;

  // (2) The profile starts inside material and grows further in — a pocket.
  if (near?.inside) return { mode: "Cut", targetBodyId: near.bodyId };

  // (3) The profile sits flush ON a body and grows AWAY from it. The prism lands
  // on that face, so it fuses — this is the push half of push/pull, and the case
  // a face-hosted sketch has always taken.
  if (far?.inside) return { mode: "Add", targetBodyId: far.bodyId };

  // (4) A separate body in the path: joins once the prism actually reaches it.
  if (near) {
    const wasAdd = prev.mode === "Add" && prev.targetBodyId === near.bodyId;
    const threshold = near.gap + (wasAdd ? -CONTACT_FLIP_HOLD : CONTACT_FLIP_HOLD);
    if (Math.abs(depth) > threshold) return { mode: "Add", targetBodyId: near.bodyId };
  }

  // (5) Nothing to interact with in this direction.
  return { mode: "NewBody", targetBodyId: null };
}

/** Whether either side found material — i.e. whether the auto lane has anything to say. */
export function hasMaterial(sides: MaterialSides): boolean {
  return sides.pos !== null || sides.neg !== null;
}
