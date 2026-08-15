/*
 * Constraint V2 — the specification, executable (SKETCH-V2 P0).
 *
 * These fixtures are the FROZEN BEHAVIOURAL CONTRACT for the constraint system
 * rebuild. They state what the system must eventually do; the runner
 * (`corpus.test.ts`) asserts only the fields whose owning phase has landed and
 * REPORTS the rest, so the gap between "what we want" and "what we have" is
 * visible at every gate instead of being discovered at the end.
 *
 * This is a sibling of `src/test/contracts/`, not a member of it: a contract
 * there pins CURRENT user-visible arrangement, whereas this corpus pins FUTURE
 * behaviour that does not exist yet. It is also deliberately NOT in `corpus/`,
 * which is reserved for characterization fixtures captured from OneCAD-CPP at
 * frozen commit `b4ddccc` where every number carries a provenance citation.
 * Nothing here is captured from the oracle — these are design requirements.
 *
 * RULES (mirroring `src/test/contracts/README.md`):
 *  - `target` values state the REQUIREMENT. Changing one is changing the
 *    product and needs a recorded decision in `TODO.md` — never to go green.
 *  - `current` values record what the code does TODAY. They are expected to
 *    move as phases land; each move must be deliberate and noted at its gate.
 *  - When `current` and `target` differ, that difference IS the work item.
 */
import type { SketchConstraint, SketchEntity } from "@/ipc/types";

/** Which phase must land before an expectation field can be asserted. */
export type Phase =
  | "P0" // available today: well-formedness + mock DOF
  | "P2" // atomic edit transactions + full diagnostics (accept/reject, redundant vs conflicting)
  | "P4" // canonical shared topology (vertex identity, degree, connectivity)
  | "P6" // solver mobility engine (per-vertex local DOF, free directions)
  | "P7" // solver-native endpoint dragging (drag outcomes)
  | "P10"; // solver-derived per-entity definition state (colors)

/** Per-entity definition state (spec §30). */
export type EntityDefinitionState =
  | "Unconstrained"
  | "PartiallyConstrained"
  | "FullyConstrained"
  | "Redundant"
  | "Conflicting";

/** Intrinsic DOF by geometry kind (spec §31). */
export const INTRINSIC_DOF: Record<string, number> = {
  Point: 2,
  Line: 4,
  Circle: 3,
  Arc: 5,
  Ellipse: 5,
};

export interface VertexExpectation {
  /** Frontend point keys expected to resolve to ONE shared vertex (spec §3). */
  members: string[];
  /** Incident curve count — spec §4 "branches become explicit". */
  degree: number;
  /** Remaining local DOF at this vertex: 0 immovable, 1 one direction, 2 free (spec §16). */
  localDof: 0 | 1 | 2;
}

export interface DragExpectation {
  /** Point key grabbed. */
  grab: string;
  /** Pointer displacement requested. */
  pointerDelta: [number, number];
  /** Displacement the geometry must actually take (spec §13/§17). */
  resultDelta: [number, number];
  /** True when the whole gesture must be rejected atomically (spec §10). */
  rejected?: boolean;
}

export interface EditExpectation {
  /** The constraint the user tries to add on top of `constraints`. */
  add: SketchConstraint;
  accepted: boolean;
  rejection?: "duplicate" | "redundant" | "conflicting" | "unsupported" | "invalidGeometry";
}

export interface ConstraintCase {
  name: string;
  /** Why this case exists — the behaviour it defends. */
  why: string;
  entities: SketchEntity[];
  constraints: SketchConstraint[];

  /** Global DOF the solver must report (spec §34). Phase P2. */
  targetDof: number;
  /** What `mockSketch.solveSketch` reports today. Phase P0. Divergence from
   *  `targetDof` is a known gap, not a bug in this file. */
  currentMockDof: number;

  /** Shared-vertex topology (spec §3-§5). Phase P4. */
  vertices?: Record<string, VertexExpectation>;
  /** Connected component by double-click, seeded from any member (spec §8/§9). Phase P4. */
  connectedComponent?: { seed: string; entityIds: string[] };
  /** Per-entity color state (spec §30-§32). Phase P10. */
  entityStates?: Record<string, EntityDefinitionState>;
  /** Direct-manipulation outcomes (spec §10/§13/§17). Phase P7. */
  drags?: DragExpectation[];
  /** Atomic edit accept/reject (spec §26-§28). Phase P2. */
  edits?: EditExpectation[];
}

// ── builders ─────────────────────────────────────────────────────────────────

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

const c = (
  id: string,
  type: SketchConstraint["type"],
  entities: string[],
  positions?: SketchConstraint["positions"],
  value?: number,
): SketchConstraint => ({
  id,
  type,
  entities,
  ...(positions ? { positions } : {}),
  ...(value !== undefined ? { value } : {}),
});

// ── the corpus ───────────────────────────────────────────────────────────────

export const CONSTRAINT_V2_CORPUS: ConstraintCase[] = [
  {
    name: "free line",
    why: "Baseline: an unconstrained line is 4 DOF and reads Blue (spec §32 golden test).",
    entities: [line("l1", [0, 0], [100, 0])],
    constraints: [],
    targetDof: 4,
    currentMockDof: 4,
    vertices: {
      v_start: { members: ["l1.Start"], degree: 1, localDof: 2 },
      v_end: { members: ["l1.End"], degree: 1, localDof: 2 },
    },
    entityStates: { l1: "Unconstrained" },
    drags: [{ grab: "l1.End", pointerDelta: [20, 15], resultDelta: [20, 15] }],
  },
  {
    name: "horizontal line",
    why: "Adding H removes exactly one DOF and flips the entity Orange. Dragging the free end diagonally must move ONLY along the free direction, with no frontend special-casing (spec §13).",
    entities: [line("l1", [0, 0], [100, 0])],
    constraints: [c("c1", "Horizontal", ["l1"])],
    targetDof: 3,
    currentMockDof: 3,
    entityStates: { l1: "PartiallyConstrained" },
    drags: [{ grab: "l1.End", pointerDelta: [20, 15], resultDelta: [20, 0] }],
    edits: [
      {
        add: c("c2", "Horizontal", ["l1"]),
        accepted: false,
        rejection: "duplicate",
      },
      {
        add: c("c3", "Vertical", ["l1"]),
        accepted: false,
        rejection: "conflicting",
      },
    ],
  },
  {
    name: "vertical line",
    why: "Mirror of the H case — the free direction is the other axis.",
    entities: [line("l1", [0, 0], [0, 100])],
    constraints: [c("c1", "Vertical", ["l1"])],
    targetDof: 3,
    currentMockDof: 3,
    entityStates: { l1: "PartiallyConstrained" },
    drags: [{ grab: "l1.End", pointerDelta: [20, 15], resultDelta: [0, 15] }],
  },
  {
    name: "fully constrained line",
    why: "H + Fixed start + Distance removes every DOF: Green, and neither endpoint moves under any drag (spec §32 golden test).",
    entities: [line("l1", [0, 0], [100, 0])],
    constraints: [
      c("c1", "Horizontal", ["l1"]),
      c("c2", "Fixed", ["l1"], ["Start"]),
      c("c3", "Distance", ["l1", "l1"], ["Start", "End"], 100),
    ],
    targetDof: 0,
    currentMockDof: 0,
    entityStates: { l1: "FullyConstrained" },
    drags: [
      { grab: "l1.End", pointerDelta: [20, 15], resultDelta: [0, 0] },
      { grab: "l1.Start", pointerDelta: [-5, -5], resultDelta: [0, 0] },
    ],
  },
  {
    name: "connected L shape",
    why: "THE case that distinguishes the two manipulation modes (spec golden test). Shared vertex B never splits; double-click selects both; group drag is rigid; endpoint drag may deform.",
    entities: [line("ab", [0, 0], [100, 0]), line("bc", [100, 0], [100, 100])],
    constraints: [
      c("weld", "Coincident", ["ab", "bc"], ["End", "Start"]),
      c("h", "Horizontal", ["ab"]),
      c("v", "Vertical", ["bc"]),
    ],
    targetDof: 4,
    currentMockDof: 4,
    vertices: {
      v_a: { members: ["ab.Start"], degree: 1, localDof: 2 },
      v_b: { members: ["ab.End", "bc.Start"], degree: 2, localDof: 2 },
      v_c: { members: ["bc.End"], degree: 1, localDof: 2 },
    },
    connectedComponent: { seed: "ab", entityIds: ["ab", "bc"] },
    entityStates: { ab: "PartiallyConstrained", bc: "PartiallyConstrained" },
  },
  {
    name: "connected L shape, anchored",
    why: "Fixing A must make the strict rigid group drag reject atomically — no partial deformation (spec §10).",
    entities: [line("ab", [0, 0], [100, 0]), line("bc", [100, 0], [100, 100])],
    constraints: [
      c("weld", "Coincident", ["ab", "bc"], ["End", "Start"]),
      c("h", "Horizontal", ["ab"]),
      c("v", "Vertical", ["bc"]),
      c("fix", "Fixed", ["ab"], ["Start"]),
    ],
    targetDof: 2,
    currentMockDof: 2,
    connectedComponent: { seed: "bc", entityIds: ["ab", "bc"] },
    drags: [{ grab: "GROUP", pointerDelta: [20, 15], resultDelta: [0, 0], rejected: true }],
  },
  {
    name: "branched polyline",
    why: "Vertex B has degree 3. Double-click from ANY segment selects all three — branches are included, and no proximity heuristic is involved (spec §8).",
    entities: [
      line("ab", [0, 0], [100, 0]),
      line("bc", [100, 0], [200, 0]),
      line("bd", [100, 0], [100, -100]),
    ],
    constraints: [
      c("w1", "Coincident", ["ab", "bc"], ["End", "Start"]),
      c("w2", "Coincident", ["ab", "bd"], ["End", "Start"]),
    ],
    targetDof: 8,
    currentMockDof: 8,
    vertices: {
      v_b: { members: ["ab.End", "bc.Start", "bd.Start"], degree: 3, localDof: 2 },
    },
    connectedComponent: { seed: "bc", entityIds: ["ab", "bc", "bd"] },
  },
  {
    name: "rectangle",
    why: "Four welded corners, every vertex degree 2, a closed profile. The canonical region/loop case.",
    entities: [
      line("l1", [0, 0], [100, 0]),
      line("l2", [100, 0], [100, 50]),
      line("l3", [100, 50], [0, 50]),
      line("l4", [0, 50], [0, 0]),
    ],
    constraints: [
      c("w1", "Coincident", ["l1", "l2"], ["End", "Start"]),
      c("w2", "Coincident", ["l2", "l3"], ["End", "Start"]),
      c("w3", "Coincident", ["l3", "l4"], ["End", "Start"]),
      c("w4", "Coincident", ["l4", "l1"], ["End", "Start"]),
    ],
    targetDof: 8,
    currentMockDof: 8,
    vertices: {
      v_bl: { members: ["l1.Start", "l4.End"], degree: 2, localDof: 2 },
      v_br: { members: ["l1.End", "l2.Start"], degree: 2, localDof: 2 },
      v_tr: { members: ["l2.End", "l3.Start"], degree: 2, localDof: 2 },
      v_tl: { members: ["l3.End", "l4.Start"], degree: 2, localDof: 2 },
    },
    connectedComponent: { seed: "l3", entityIds: ["l1", "l2", "l3", "l4"] },
  },
  {
    name: "origin anchored endpoint",
    why: "Snapping an endpoint to the origin anchors it: that vertex reaches localDof 0, but the LINE keeps orientation and length DOF, so it must read Orange, not Green (spec golden test).",
    entities: [line("l1", [0, 0], [100, 0])],
    constraints: [c("anchor", "Fixed", ["l1"], ["Start"])],
    targetDof: 2,
    currentMockDof: 2,
    vertices: {
      v_origin: { members: ["l1.Start"], degree: 1, localDof: 0 },
      v_free: { members: ["l1.End"], degree: 1, localDof: 2 },
    },
    entityStates: { l1: "PartiallyConstrained" },
  },
  {
    name: "circle",
    why: "Radius alone leaves the centre free — 3 intrinsic DOF, 2 remaining.",
    entities: [circle("c1", [0, 0], 25)],
    constraints: [c("r", "Radius", ["c1"], undefined, 25)],
    targetDof: 2,
    currentMockDof: 2,
    entityStates: { c1: "PartiallyConstrained" },
  },
  {
    name: "arc",
    why: "Arc endpoints must become ordinary vertices (spec §6). Until then they are addressable only under Coincident — this case is the regression guard for that seam.",
    entities: [arc("a1", [0, 0], 50, [50, 0], [0, 50])],
    constraints: [],
    targetDof: 5,
    currentMockDof: 5,
    vertices: {
      v_center: { members: ["a1.Center"], degree: 0, localDof: 2 },
      v_start: { members: ["a1.Start"], degree: 1, localDof: 2 },
      v_end: { members: ["a1.End"], degree: 1, localDof: 2 },
    },
    entityStates: { a1: "Unconstrained" },
  },
  {
    name: "redundant constraint",
    why: "A line already Horizontal must reject a second Horizontal with reason `duplicate`, leaving the document byte-identical and creating NO undo record (spec §28 golden test).",
    entities: [line("l1", [0, 0], [100, 0])],
    constraints: [c("c1", "Horizontal", ["l1"])],
    targetDof: 3,
    currentMockDof: 3,
    edits: [{ add: c("dup", "Horizontal", ["l1"]), accepted: false, rejection: "duplicate" }],
  },
  {
    name: "conflicting dimension",
    why: "Both endpoints pinned on a horizontal line: a VerticalDistance of 20 is unsatisfiable. Reject atomically, preserve the old value and the geometry, and name the constraints involved (spec §28 golden test).",
    entities: [line("l1", [0, 0], [100, 0])],
    constraints: [
      c("h", "Horizontal", ["l1"]),
      c("fa", "Fixed", ["l1"], ["Start"]),
      c("fb", "Fixed", ["l1"], ["End"]),
    ],
    targetDof: 0,
    currentMockDof: 0,
    edits: [
      {
        add: c("vd", "VerticalDistance", ["l1", "l1"], ["Start", "End"], 20),
        accepted: false,
        rejection: "conflicting",
      },
    ],
  },
];
