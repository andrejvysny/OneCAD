/*
 * sketchService.applyConstraint — the user-apply flow (S4b design items 2 + 3)
 * against the mock solver lane. Covers: applicability → apply round-trip (geometric
 * lands in the session), reject-on-conflict revert, and the dimensional chip route
 * (opens the seeded Dimension chip; its Enter authors through
 * commitDimensionConstraint). Kept in a SEPARATE file from sketchService.test.ts.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { applyConstraint } from "./sketchService";
import { toConstraintTarget, type SketchConstraintTarget } from "./constraintTarget";
import { evaluateApplicability } from "./constraintApplicability";
import { mockClient, resetMockSketches } from "@/ipc/mockClient";
import { planeFor } from "@/ipc/mockSketch";
import { sketchStore } from "@/stores/sketchStore";
import { toolChipStore } from "@/stores/toolChipStore";
import type { ConstraintPosition, SketchConstraint, SketchEntity, SketchSession } from "@/ipc/types";

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({ id, type: "Line", p0, p1 });
const circle = (id: string, center: [number, number], radius: number): SketchEntity => ({ id, type: "Circle", center, radius });

function seed(entities: SketchEntity[], constraints: SketchConstraint[]): SketchSession {
  const s: SketchSession = {
    sketchId: "sk-apply",
    plane: planeFor("XY"),
    entities,
    constraints,
    dof: 0,
    status: "UnderConstrained",
  };
  sketchStore.getState().setSession(s);
  return s;
}

const targetsFor = (
  entities: SketchEntity[],
  ...refs: { entityId: string; point?: ConstraintPosition }[]
): SketchConstraintTarget[] => refs.map((r) => toConstraintTarget(r, entities)!);

const flush = () => new Promise((r) => setTimeout(r, 5));

describe("applyConstraint — geometric author + reject-on-conflict", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
    toolChipStore.getState().clear();
  });

  it("applicability → apply: two lines ⇒ Parallel offered ⇒ applied ⇒ in session", async () => {
    const entities = [line("e1", [0, 0], [40, 0]), line("e2", [0, 10], [40, 12])];
    seed(entities, []);
    const parallel = evaluateApplicability(targetsFor(entities, { entityId: "e1" }, { entityId: "e2" }), entities).find(
      (a) => a.type === "Parallel",
    )!;
    expect(parallel).toBeDefined();

    const { rejected } = await applyConstraint(mockClient, parallel);
    expect(rejected).toBe(false);

    const s = sketchStore.getState().session!;
    expect(s.constraints).toHaveLength(1);
    expect(s.constraints[0]).toMatchObject({ type: "Parallel", entities: ["e1", "e2"] });
    expect(s.status).toBe("UnderConstrained");
  });

  it("rejects + reverts a geometric constraint that over-constrains the sketch", async () => {
    // One line (4 DOF) fully constrained by two Coincidents (2+2); +Horizontal ⇒ over.
    const entities = [line("e1", [0, 0], [40, 0])];
    seed(entities, [
      { id: "k1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] },
      { id: "k2", type: "Coincident", entities: ["e1", "e1"], positions: ["End", "Start"] },
    ]);
    const horiz = evaluateApplicability(targetsFor(entities, { entityId: "e1" }), entities).find(
      (a) => a.type === "Horizontal",
    )!;

    const { rejected } = await applyConstraint(mockClient, horiz);
    expect(rejected).toBe(true);

    const s = sketchStore.getState().session!;
    expect(s.constraints.some((c) => c.type === "Horizontal")).toBe(false);
    expect(s.constraints).toHaveLength(2);
    expect(s.status).toBe("FullyConstrained");
    expect(s.dof).toBe(0);
  });

  it("rejects + reverts a duplicate Tangent that over-constrains the sketch", async () => {
    // line (4 DOF) + circle (3 DOF) = 7 total, fully consumed by 4 pre-seeded
    // constraints (Fixed × 2 on the line's endpoints + Fixed on the circle's
    // center + Radius) — mirrors the real-world case this design targets:
    // autoConstrain.ts already infers a Tangent for an arc-start-on-line, so a
    // user re-applying Tangent on an already-fully-constrained pair is a
    // redundant duplicate the worker reports as OverConstrained.
    const entities = [line("e1", [0, 0], [40, 0]), circle("e2", [60, 0], 10)];
    seed(entities, [
      { id: "k1", type: "Fixed", entities: ["e1"], positions: ["Start"] },
      { id: "k2", type: "Fixed", entities: ["e1"], positions: ["End"] },
      { id: "k3", type: "Fixed", entities: ["e2"], positions: ["Center"] },
      { id: "k4", type: "Radius", entities: ["e2"], value: 10 },
    ]);
    const tangent = evaluateApplicability(targetsFor(entities, { entityId: "e1" }, { entityId: "e2" }), entities).find(
      (a) => a.type === "Tangent",
    )!;
    expect(tangent).toBeDefined();

    const { rejected } = await applyConstraint(mockClient, tangent);
    expect(rejected).toBe(true);

    const s = sketchStore.getState().session!;
    expect(s.constraints.some((c) => c.type === "Tangent")).toBe(false);
    expect(s.constraints).toHaveLength(4);
    expect(s.status).toBe("FullyConstrained");
    expect(s.dof).toBe(0);
  });

  it("is a no-op with no active session", async () => {
    const { rejected } = await applyConstraint(mockClient, {
      type: "Parallel",
      targets: [
        { kind: "line", entityId: "e1" },
        { kind: "line", entityId: "e2" },
      ],
      dimensional: false,
    });
    expect(rejected).toBe(false);
    expect(sketchStore.getState().session).toBeNull();
  });
});

describe("applyConstraint — dimensional route (chip flow)", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
    toolChipStore.getState().clear();
  });

  it("does NOT silent-commit: opens the seeded Dimension chip, then Enter authors it", async () => {
    const entities = [circle("e1", [0, 0], 5)];
    seed(entities, []);
    const radius = evaluateApplicability(targetsFor(entities, { entityId: "e1" }), entities).find(
      (a) => a.type === "Radius",
    )!;

    const { rejected } = await applyConstraint(mockClient, radius);
    expect(rejected).toBe(false);

    // The chip is armed + seeded from current geometry; nothing committed yet.
    const chip = toolChipStore.getState();
    expect(chip.kind).toBe("dimension");
    expect(chip.value).toBe(5);
    expect(chip.suffix).toBe("mm");
    expect(sketchStore.getState().session!.constraints).toHaveLength(0);

    // Chip Enter → author the Radius through commitDimensionConstraint.
    chip.onValue!(8);
    await flush();

    const s = sketchStore.getState().session!;
    expect(s.constraints.some((c) => c.type === "Radius" && c.value === 8)).toBe(true);
    expect(toolChipStore.getState().kind).toBe("none"); // chip cleared on commit
  });
});

describe("applyConstraint — serialization of rapid applies", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
    toolChipStore.getState().clear();
  });

  it("two applies fired before the first upsert resolves BOTH persist (queued rebase)", async () => {
    const entities = [line("e1", [0, 0], [40, 0]), line("e2", [0, 10], [40, 12])];
    seed(entities, []);
    const targets = targetsFor(entities, { entityId: "e1" }, { entityId: "e2" });
    const offered = evaluateApplicability(targets, entities);
    const parallel = offered.find((a) => a.type === "Parallel")!;
    // Equal (not Perpendicular): two lines being BOTH parallel and equal-length
    // is not a contradiction — Perpendicular would be (mockConflicts.ts R3), which
    // would make this fixture a genuine reject-on-conflict case rather than the
    // race-condition/queueing behavior this test actually targets.
    const equal = offered.find((a) => a.type === "Equal")!;
    expect(parallel).toBeDefined();
    expect(equal).toBeDefined();

    // Delay the FIRST sketchUpsert so the second apply fires while it is
    // in flight — without the mutation queue both captured the same stale
    // session and the second diff removed the first's constraint.
    const realUpsert = mockClient.sketchUpsert.bind(mockClient);
    let delayed = false;
    const delayedClient = {
      ...mockClient,
      sketchUpsert: (async (...args: Parameters<typeof mockClient.sketchUpsert>) => {
        if (!delayed) {
          delayed = true;
          await new Promise((r) => setTimeout(r, 10));
        }
        return realUpsert(...args);
      }) as typeof mockClient.sketchUpsert,
    };

    const p1 = applyConstraint(delayedClient, parallel);
    const p2 = applyConstraint(delayedClient, equal);
    await Promise.all([p1, p2]);
    await flush();

    const s = sketchStore.getState().session!;
    const kinds = s.constraints.map((c) => c.type).sort();
    expect(kinds).toEqual(["Equal", "Parallel"]);
  });
});
