/*
 * Accepted intent → persisted constraints (SNAP P3).
 *
 * THE rule: only an ACCEPTED relation persists. Coordinate coincidence is not
 * evidence — the same two coordinates could be a deliberate weld or an accident,
 * and only the snap decision knows which.
 */
import { describe, expect, it } from "vitest";
import type { SketchConstraint } from "@/ipc/types";
import {
  canonicalKey,
  persistAcceptedIntents,
  type PlacedIntent,
} from "./snapPersistence";
import type { FrontendPointRef, SnapCandidate, SnapDecision, SnapRelationIntent } from "./snapTypes";

const ref = (entityId: string, position: FrontendPointRef["position"]): FrontendPointRef => ({
  entityId,
  position,
});

function decisionWith(intents: SnapRelationIntent[]): SnapDecision {
  const candidate: SnapCandidate = {
    id: "c",
    source: "geometryPoint",
    kind: "endpoint",
    projection: { kind: "point", point: { x: 0, y: 0 } },
    previewPoint: { x: 0, y: 0 },
    claims: ["point"],
    errorPx: 0,
    semanticBiasPx: 0,
    scorePx: 0,
    label: "",
    guides: [],
    refs: [],
    relationIntents: intents,
  };
  return {
    raw: { x: 0, y: 0 },
    point: { x: 0, y: 0 },
    accepted: [candidate],
    rejected: [],
    primaryId: "c",
    primaryKind: "endpoint",
    label: null,
    guides: [],
    snapped: true,
    traceId: 1,
  };
}

const placed = (
  intents: SnapRelationIntent[],
  target: FrontendPointRef | null,
  alt = false,
): PlacedIntent => ({
  decision: decisionWith(intents),
  suppressInference: alt,
  target,
});

let seq = 0;
const opts = (over: Partial<Parameters<typeof persistAcceptedIntents>[1]> = {}) => ({
  mode: "standard" as const,
  nextConstraintId: () => `k${++seq}`,
  existing: [] as SketchConstraint[],
  originAnchored: false,
  ...over,
});

const types = (cs: SketchConstraint[]): string[] => cs.map((c) => c.type);

describe("persistAcceptedIntents — the mode matrix", () => {
  const coincident: SnapRelationIntent = { kind: "Coincident", refs: [ref("l0", "End")] };
  const hPoints: SnapRelationIntent = { kind: "HorizontalPoints", refs: [ref("l0", "Start")] };
  const target = ref("l1", "Start");

  it("off persists NOTHING inferred", () => {
    const r = persistAcceptedIntents([placed([coincident, hPoints], target)], opts({ mode: "off" }));
    expect(r.constraints).toEqual([]);
    expect(r.dropped.map((d) => d.reason)).toEqual(["mode-off", "mode-off"]);
  });

  it("minimal keeps POINT relations and drops DIRECTION relations", () => {
    const r = persistAcceptedIntents(
      [placed([coincident, hPoints], target)],
      opts({ mode: "minimal" }),
    );
    expect(types(r.constraints)).toEqual(["Coincident"]);
    expect(r.dropped).toEqual([{ kind: "HorizontalPoints", reason: "mode-minimal" }]);
  });

  it("standard keeps both", () => {
    const r = persistAcceptedIntents([placed([coincident, hPoints], target)], opts());
    expect(types(r.constraints)).toEqual(["Coincident", "HorizontalPoints"]);
  });

  it("Alt suppresses inference under EVERY mode", () => {
    for (const mode of ["off", "minimal", "standard"] as const) {
      const r = persistAcceptedIntents(
        [placed([coincident, hPoints], target, true)],
        opts({ mode }),
      );
      expect(r.constraints, mode).toEqual([]);
      expect(r.dropped.every((d) => d.reason === "alt-suppressed"), mode).toBe(true);
    }
  });
});

describe("persistAcceptedIntents — per-kind shapes", () => {
  it("Coincident names both points and both roles", () => {
    const r = persistAcceptedIntents(
      [placed([{ kind: "Coincident", refs: [ref("a1", "End")] }], ref("l1", "Start"))],
      opts(),
    );
    expect(r.constraints[0]).toMatchObject({
      type: "Coincident",
      entities: ["l1", "a1"],
      positions: ["Start", "End"],
    });
  });

  it("Midpoint names the point and the LINE it is the midpoint of", () => {
    const r = persistAcceptedIntents(
      [placed([{ kind: "Midpoint", refs: [], lineIds: ["l0"] }], ref("p1", "Start"))],
      opts(),
    );
    expect(r.constraints[0]).toMatchObject({
      type: "Midpoint",
      entities: ["p1", "l0"],
      positions: ["Start"],
    });
  });

  it("OnCurve names the point and the curve", () => {
    const r = persistAcceptedIntents(
      [placed([{ kind: "OnCurve", refs: [], curveIds: ["c1"] }], ref("p1", "Start"))],
      opts(),
    );
    expect(r.constraints[0]).toMatchObject({ type: "OnCurve", entities: ["p1", "c1"] });
  });

  it("an x guide persists VerticalPoints and a y guide HorizontalPoints", () => {
    const v = persistAcceptedIntents(
      [placed([{ kind: "VerticalPoints", refs: [ref("l0", "Start")] }], ref("l1", "End"))],
      opts(),
    );
    expect(v.constraints[0]).toMatchObject({
      type: "VerticalPoints",
      entities: ["l1", "l0"],
      positions: ["End", "Start"],
    });
    const h = persistAcceptedIntents(
      [placed([{ kind: "HorizontalPoints", refs: [ref("l0", "Start")] }], ref("l1", "End"))],
      opts(),
    );
    expect(h.constraints[0].type).toBe("HorizontalPoints");
  });

  it("Parallel / Perpendicular name the authored entity and the reference line", () => {
    const r = persistAcceptedIntents(
      [placed([{ kind: "Parallel", refs: [], lineIds: ["l0"] }], ref("l1", "End"))],
      opts(),
    );
    expect(r.constraints[0]).toMatchObject({ type: "Parallel", entities: ["l1", "l0"] });
    expect(r.constraints[0].positions).toBeUndefined();
  });
});

describe("persistAcceptedIntents — the origin anchor", () => {
  const originIntent: SnapRelationIntent = { kind: "FixedOrigin", refs: [] };

  it("anchors ONE point per sketch", () => {
    const r = persistAcceptedIntents(
      [placed([originIntent], ref("l1", "Start")), placed([originIntent], ref("l2", "Start"))],
      opts(),
    );
    expect(types(r.constraints)).toEqual(["Fixed"]);
    expect(r.constraints[0].entities).toEqual(["l1"]); // the FIRST placement wins
    expect(r.dropped).toEqual([{ kind: "FixedOrigin", reason: "origin-already-anchored" }]);
  });

  it("does not anchor when the sketch is already anchored", () => {
    const r = persistAcceptedIntents(
      [placed([originIntent], ref("l1", "Start"))],
      opts({ originAnchored: true }),
    );
    expect(r.constraints).toEqual([]);
  });

  it("does not anchor a PLACEMENT-ONLY click", () => {
    const r = persistAcceptedIntents([placed([originIntent], null)], opts());
    expect(r.constraints).toEqual([]);
    expect(r.dropped).toEqual([{ kind: "FixedOrigin", reason: "placement-only" }]);
  });
});

describe("persistAcceptedIntents — dedupe", () => {
  it("never re-authors a relation the batch already has", () => {
    const existing: SketchConstraint[] = [
      { id: "x", type: "Coincident", entities: ["l0", "l1"], positions: ["End", "Start"] },
    ];
    const r = persistAcceptedIntents(
      [placed([{ kind: "Coincident", refs: [ref("l0", "End")] }], ref("l1", "Start"))],
      opts({ existing }),
    );
    expect(r.constraints).toEqual([]);
    expect(r.dropped).toEqual([{ kind: "Coincident", reason: "duplicate" }]);
  });

  it("treats a symmetric relation as the SAME relation either way round", () => {
    // Two identical equations read as over-constrained, which is a false
    // diagnosis the user cannot act on.
    const a = canonicalKey({
      type: "Coincident",
      entities: ["l0", "l1"],
      positions: ["End", "Start"],
    });
    const b = canonicalKey({
      type: "Coincident",
      entities: ["l1", "l0"],
      positions: ["Start", "End"],
    });
    expect(a).toBe(b);
  });

  it("keeps an ASYMMETRIC relation's operand order significant", () => {
    // `Midpoint(point, line)` is not `Midpoint(line, point)`.
    const a = canonicalKey({ type: "Midpoint", entities: ["p1", "l0"], positions: ["Start"] });
    const b = canonicalKey({ type: "Midpoint", entities: ["l0", "p1"], positions: ["Start"] });
    expect(a).not.toBe(b);
  });
});

describe("persistAcceptedIntents — placement-only clicks", () => {
  it("drops an intent whose click produced no addressable point", () => {
    const r = persistAcceptedIntents(
      [placed([{ kind: "Coincident", refs: [ref("l0", "End")] }], null)],
      opts(),
    );
    expect(r.constraints).toEqual([]);
    expect(r.dropped).toEqual([{ kind: "Coincident", reason: "placement-only" }]);
  });

  it("drops an intent that names no reference", () => {
    const r = persistAcceptedIntents(
      [placed([{ kind: "Coincident", refs: [] }], ref("l1", "Start"))],
      opts(),
    );
    expect(r.constraints).toEqual([]);
    expect(r.dropped).toEqual([{ kind: "Coincident", reason: "missing-ref" }]);
  });
});

// ── A7: quadrant snap authors OnCurve ──────────────────────────────────────
//
// `relationForCachedPoint` (snapCandidates.ts) now attaches this exact shape
// to a quadrant candidate — the same OnCurve intent an onCurve/intersection
// candidate would, targeting the curve the quadrant is an extremum of.
describe("persistAcceptedIntents — quadrant snap (A7)", () => {
  const quadrantIntent: SnapRelationIntent = { kind: "OnCurve", refs: [], curveIds: ["circle1"] };

  it("an accepted quadrant candidate persists OnCurve, bound to the entity and the clicked point role", () => {
    const r = persistAcceptedIntents(
      [placed([quadrantIntent], ref("line1", "Start"))],
      opts(),
    );
    expect(r.constraints).toHaveLength(1);
    expect(r.constraints[0]).toMatchObject({
      type: "OnCurve",
      entities: ["line1", "circle1"],
      positions: ["Start"],
    });
  });

  it("Alt still suppresses it, same as any other inferred relation", () => {
    const r = persistAcceptedIntents(
      [placed([quadrantIntent], ref("line1", "Start"), true)],
      opts(),
    );
    expect(r.constraints).toEqual([]);
    expect(r.dropped).toEqual([{ kind: "OnCurve", reason: "alt-suppressed" }]);
  });
});
