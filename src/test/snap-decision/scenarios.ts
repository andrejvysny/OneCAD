/*
 * The shared snap-decision scenario table (SNAP P0).
 *
 * ONE geometry + pointer table, consumed by the P0 determinism tests, the P2
 * arbitration/composition tests and the P5 intersection tests. Keeping the
 * fixtures here rather than inline per test file is what makes "endpoint beats
 * midpoint at 3px" and "the same case is order-independent" provably the SAME
 * case.
 *
 * Coordinates are plane (u,v) millimetres. The default metric is isotropic at
 * 1 px per unit, so a plane distance of 3 IS 3 screen pixels — every expected
 * pixel figure in the table can be read straight off the coordinates.
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";

export interface SnapScenario {
  name: string;
  entities: SketchEntity[];
  /** Raw cursor in plane coords. */
  raw: Point2;
  /** Gesture anchor, when the scenario is mid-gesture (polar needs one). */
  anchor?: Point2;
  /** Direction the chain is travelling (adds parallel/perpendicular rays). */
  refDir?: Point2;
  /** Free-text note on what the case is ABOUT — the reason it is in the table. */
  about: string;
}

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({
  id,
  type: "Line",
  p0,
  p1,
});

const circle = (id: string, center: [number, number], radius: number): SketchEntity => ({
  id,
  type: "Circle",
  center,
  radius,
});

const arc = (
  id: string,
  center: [number, number],
  radius: number,
  start: [number, number],
  end: [number, number],
): SketchEntity => ({ id, type: "Arc", center, radius, start, end });

const ellipse = (
  id: string,
  center: [number, number],
  majorR: number,
  minorR: number,
  rotation = 0,
): SketchEntity => ({ id, type: "Ellipse", center, majorR, minorR, rotation });

export const SCENARIO_BUILDERS = { line, circle, arc, ellipse };

/**
 * The table. Every entry is a real situation the audit named, not a synthetic
 * probe — the names are quoted in the phase gates.
 */
export const SNAP_SCENARIOS: SnapScenario[] = [
  {
    name: "endpoint-vs-midpoint-near",
    entities: [line("l1", [0, 0], [20, 0]), line("l2", [10, 6], [10, 20])],
    raw: { x: 10.5, y: 1.5 },
    about:
      "l2's endpoint (10,6) is 4.5 away; l1's midpoint (10,0) is 1.58 away. " +
      "Distance-plus-bias must pick the midpoint the cursor is visibly on; the " +
      "legacy absolute tier ladder would have picked the far endpoint.",
  },
  {
    name: "endpoint-vs-midpoint-equal",
    entities: [line("l1", [0, 0], [20, 0]), line("l2", [13, 0], [13, 20])],
    raw: { x: 11.5, y: 0 },
    about:
      "midpoint (10,0) and endpoint (13,0) are both 1.5 away — the endpoint's " +
      "stronger bias (-1.0 vs -0.75) breaks the tie, deterministically.",
  },
  {
    name: "origin-core-capture",
    entities: [line("l1", [-5, 1], [5, 1])],
    raw: { x: 0.4, y: 0.4 },
    about:
      "The origin is inside its 2px capture core, so it outranks the on-curve " +
      "point 0.6 away. Outside the core the origin is an ordinary -0.5 bias.",
  },
  {
    name: "origin-outside-core-loses-to-endpoint",
    entities: [line("l1", [3, 0], [20, 0])],
    raw: { x: 2.6, y: 0 },
    about:
      "3.6px from origin (bias -0.5 ⇒ 3.1) vs 0.4px from l1's endpoint " +
      "(bias -1.0 ⇒ -0.6): the endpoint the cursor is visibly on wins.",
  },
  {
    name: "two-guides-compose",
    entities: [line("l1", [0, 12], [8, 12]), line("l2", [25, 0], [25, 9])],
    raw: { x: 25.4, y: 12.4 },
    about:
      "x = 25 from l2's endpoints and y = 12 from l1's — two INDEPENDENT " +
      "claims that must both be accepted, not one suppressing the other.",
  },
  {
    name: "polar-plus-rounded-length",
    entities: [],
    anchor: { x: 0, y: 0 },
    raw: { x: 17.4, y: 17.9 },
    about:
      "45° polar ray plus a rounded length — the composition the legacy ladder " +
      "could not express, because polar and grid/dimension-round were one slot.",
  },
  {
    name: "polar-vs-guide-tie",
    entities: [line("l1", [0, 0], [0, 20])],
    anchor: { x: 0, y: 0 },
    raw: { x: 0.3, y: 14 },
    about:
      "The 90° polar ray and the vertical guide through l1's start describe the " +
      "SAME line. One of them must win by a stable rule, never by array order.",
  },
  {
    name: "grid-node-vs-mid-cell",
    entities: [],
    raw: { x: 0.4, y: 0.4 },
    about:
      "Near a grid node with step 1: inside grid reach. The mid-cell probe " +
      "(0.5,0.5) is outside it, which is where numeric rounding must answer.",
  },
  {
    name: "grid-mid-cell",
    entities: [],
    raw: { x: 0.5, y: 0.5 },
    about: "Exactly mid-cell — grid must NOT capture, leaving the field to numeric.",
  },
  {
    name: "intersection-line-line",
    entities: [line("l1", [-10, 0], [10, 0]), line("l2", [0, -10], [0, 10])],
    raw: { x: 0.6, y: 0.6 },
    about: "A crossing with no endpoint at it — intersection is the only named point.",
  },
  {
    name: "intersection-line-circle",
    entities: [line("l1", [-10, 0], [10, 0]), circle("c1", [0, 0], 5)],
    raw: { x: 5.3, y: 0.3 },
    about: "Two roots; the near one wins and the far one must still be generated.",
  },
  {
    name: "quadrant-on-arc",
    entities: [arc("a1", [0, 0], 10, [10, 0], [0, 10])],
    raw: { x: 0.4, y: 10.3 },
    about: "The 90° quadrant lies INSIDE the sweep; the 180°/270° ones do not.",
  },
  {
    name: "on-curve-loses-to-equally-close-named-point",
    entities: [line("l1", [0, 0], [20, 0]), circle("c1", [0, 3], 3)],
    raw: { x: 0.2, y: 0.2 },
    about:
      "The on-curve point on c1 and l1's endpoint are both ~0.2 away; the " +
      "+0.75 on-curve penalty is what makes the named point win.",
  },
  {
    name: "ellipse-extrema",
    entities: [ellipse("e1", [0, 0], 20, 10, 0)],
    raw: { x: 19.6, y: 0.3 },
    about: "P5: the ellipse's four axis extrema become quadrant candidates.",
  },
  {
    name: "ellipse-line-secant",
    entities: [ellipse("e1", [0, 0], 20, 10, 0), line("l1", [-30, 5], [30, 5])],
    raw: { x: 17.3, y: 5.2 },
    about: "P5: a line crossing an ellipse at two points, both snappable.",
  },
  {
    name: "ellipse-line-tangent",
    entities: [ellipse("e1", [0, 0], 20, 10, 0), line("l1", [-30, 10], [30, 10])],
    raw: { x: 0.4, y: 10.2 },
    about: "P5: a tangent line — one DOUBLE root, which must not be reported twice.",
  },
  {
    name: "ellipse-circle-four-roots",
    entities: [ellipse("e1", [0, 0], 20, 10, 0), circle("c1", [0, 0], 14)],
    raw: { x: 14.2, y: 0.5 },
    about: "P5: an ellipse and a concentric circle between its radii — four roots.",
  },
  {
    name: "disjoint-pair-broad-phase",
    entities: [line("l1", [0, 0], [1, 1]), circle("c1", [500, 500], 2)],
    raw: { x: 0.5, y: 0.5 },
    about: "P5: bounding boxes do not overlap — exact intersection must be SKIPPED.",
  },
  {
    name: "dense-junction-determinism",
    entities: [
      line("l1", [0, 0], [10, 0]),
      line("l2", [10, 0], [10, 10]),
      line("l3", [10, 10], [0, 10]),
      line("l4", [0, 10], [0, 0]),
    ],
    raw: { x: 10.2, y: 0.2 },
    about:
      "Two entities' endpoints sit at exactly (10,0). Which one a candidate " +
      "binds to must be a function of the geometry, never of array order.",
  },
];

/** Look one up by name; throws rather than silently testing nothing. */
export function scenario(name: string): SnapScenario {
  const s = SNAP_SCENARIOS.find((x) => x.name === name);
  if (!s) throw new Error(`unknown snap scenario: ${name}`);
  return s;
}

/** Every permutation-stable ordering of a scenario's entities (bounded). */
export function permutations<T>(items: readonly T[], cap = 24): T[][] {
  if (items.length <= 1) return [items.slice()];
  const out: T[][] = [];
  const walk = (rest: T[], acc: T[]): void => {
    if (out.length >= cap) return;
    if (rest.length === 0) {
      out.push(acc.slice());
      return;
    }
    for (let i = 0; i < rest.length; i++) {
      const next = rest.slice();
      const [pick] = next.splice(i, 1);
      acc.push(pick);
      walk(next, acc);
      acc.pop();
      if (out.length >= cap) return;
    }
  };
  walk(items.slice(), []);
  return out;
}
