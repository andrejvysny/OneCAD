/*
 * sketchService.extendEntity — the Extend commit path (real mock lane) plus the
 * pure constraint cascade it runs (`remapExtendConstraints`).
 *
 * The load-bearing invariant: the grown entity REPLACES the target under a FRESH
 * id, because `marshalUpsert` diffs entities BY ID — an in-place endpoint move
 * emits no wire op and would silently never reach the backend.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { extendEntity, remapExtendConstraints } from "./sketchService";
import { mockClient, resetMockSketches } from "@/ipc/mockClient";
import { planeFor } from "@/ipc/mockSketch";
import { sketchStore } from "@/stores/sketchStore";
import { viewportStore } from "@/stores/viewportStore";
import type { CadClient } from "@/ipc/client";
import type {
  SketchConstraint,
  SketchConstraintType,
  SketchEntity,
  SketchUpsertResult,
} from "@/ipc/types";

/** Target line [0,0]→[10,0]; `wall` is the vertical it extends to at x=20. */
const TARGET: SketchEntity = { id: "b", type: "Line", p0: [0, 0], p1: [10, 0] };
const WALL: SketchEntity = { id: "w", type: "Line", p0: [20, -50], p1: [20, 50] };
/** A hover on the far half ⇒ the END grows. */
const AT_END: [number, number] = [9, 0];

function seed(entities: SketchEntity[], constraints: SketchConstraint[]): void {
  sketchStore.getState().setSession({
    sketchId: "sk-extend",
    plane: planeFor("XY"),
    entities,
    constraints,
    dof: 0,
    status: "UnderConstrained",
  });
}

describe("extendEntity — commit path", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
    viewportStore.getState().setStatusHint(null);
  });

  it("REPLACES the target with a fresh id in ONE upsert, and pushes ONE 'extend' undo", async () => {
    seed([TARGET, WALL], []);
    const spy = vi.spyOn(mockClient, "sketchUpsert");
    const outcome = await extendEntity(mockClient, "b", AT_END, { minSize: 0.1 });
    expect(outcome).toEqual({ ok: true });
    expect(spy).toHaveBeenCalledTimes(1);

    const s = sketchStore.getState().session!;
    const ids = s.entities.map((e) => e.id);
    expect(ids).not.toContain("b"); // the id MUST change or the wire sees nothing
    expect(ids).toContain("w");
    expect(s.entities).toHaveLength(2); // replaced in place, not appended
    const grown = s.entities.find((e) => e.id !== "w")!;
    expect(grown.type).toBe("Line");
    expect(grown.p0).toEqual([0, 0]); // untouched end
    expect(grown.p1![0]).toBeCloseTo(20, 9); // grown out to the wall
    expect(grown.p1![1]).toBeCloseTo(0, 9);
    // The replacement keeps the target's SLOT in the array (order is meaningful
    // downstream for region/profile ordering).
    expect(s.entities[0].id).toBe(grown.id);

    expect(sketchStore.getState().undoStack).toHaveLength(1);
    expect(sketchStore.getState().lastUndoPush?.kind).toBe("extend");
    spy.mockRestore();
  });

  it("grows the START when the hover is on the near half", async () => {
    const back: SketchEntity = { id: "k", type: "Line", p0: [-5, -50], p1: [-5, 50] };
    seed([TARGET, back], []);
    await extendEntity(mockClient, "b", [1, 0], { minSize: 0.1 });
    const grown = sketchStore.getState().session!.entities.find((e) => e.id !== "k")!;
    expect(grown.p0![0]).toBeCloseTo(-5, 9);
    expect(grown.p1).toEqual([10, 0]);
  });

  it("cascades the constraints onto the successor (kept ones re-minted, moved-end weld dropped)", async () => {
    seed(
      [TARGET, WALL],
      [
        { id: "k1", type: "Horizontal", entities: ["b"] }, // kept, re-minted
        { id: "k2", type: "Coincident", entities: ["b", "w"], positions: ["End", "Start"] }, // moved end
        { id: "k3", type: "Vertical", entities: ["w"] }, // untouched
      ],
    );
    await extendEntity(mockClient, "b", AT_END, { minSize: 0.1 });

    const s = sketchStore.getState().session!;
    const grownId = s.entities.find((e) => e.id !== "w")!.id;
    expect(s.constraints.map((c) => c.type).sort()).toEqual(["Horizontal", "Vertical"]);
    const kept = s.constraints.find((c) => c.type === "Horizontal")!;
    expect(kept.entities).toEqual([grownId]);
    expect(kept.id).not.toBe("k1"); // fresh id, or the wire emits no op
    expect(s.constraints.find((c) => c.type === "Vertical")!.id).toBe("k3");
  });

  it("REFUSES locked reference geometry — no upsert, and says why", async () => {
    seed([{ ...TARGET, referenceLocked: true }, WALL], []);
    const spy = vi.spyOn(mockClient, "sketchUpsert");
    const outcome = await extendEntity(mockClient, "b", AT_END, { minSize: 0.1 });
    expect(outcome).toEqual({ ok: false, reason: "refused" });
    expect(spy).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toMatch(
      /Reference geometry is locked.*cannot be extended/,
    );
    expect(sketchStore.getState().undoStack).toHaveLength(0);
    spy.mockRestore();
  });

  it("is inert when there is nothing to extend to (no upsert, no undo)", async () => {
    const parallel: SketchEntity = { id: "p", type: "Line", p0: [0, 5], p1: [30, 5] };
    seed([TARGET, parallel], []);
    const before = sketchStore.getState().session;
    const spy = vi.spyOn(mockClient, "sketchUpsert");
    const outcome = await extendEntity(mockClient, "b", AT_END, { minSize: 0.1 });
    expect(outcome).toEqual({ ok: false, reason: "noExtension" });
    expect(spy).not.toHaveBeenCalled();
    expect(sketchStore.getState().session).toBe(before); // untouched
    expect(sketchStore.getState().undoStack).toHaveLength(0);
    spy.mockRestore();
  });

  it("is a no-op for an unknown id / no session", async () => {
    seed([TARGET, WALL], []);
    const before = sketchStore.getState().session;
    expect(await extendEntity(mockClient, "nope", AT_END, { minSize: 0.1 })).toEqual({
      ok: false,
      reason: "refused",
    });
    expect(sketchStore.getState().session).toBe(before);
    sketchStore.getState().reset();
    expect(await extendEntity(mockClient, "b", AT_END, { minSize: 0.1 })).toEqual({
      ok: false,
      reason: "refused",
    });
    expect(sketchStore.getState().session).toBeNull();
  });

  it("publishes NOTHING when a newer session superseded it mid-flight", async () => {
    seed([TARGET, WALL], []);
    let resolve!: (r: SketchUpsertResult) => void;
    const client = {
      sketchUpsert: () => new Promise<SketchUpsertResult>((res) => (resolve = res)),
    } as unknown as CadClient;

    const p = extendEntity(client, "b", AT_END, { minSize: 0.1 });
    await new Promise((r) => setTimeout(r, 0)); // parked at the upsert await

    seed([TARGET], []); // a newer setSession bumps the generation
    const superseding = sketchStore.getState().session;
    resolve({
      sketchId: "sk-extend",
      sketchRevision: 1,
      dof: 0,
      status: "UnderConstrained",
      solvedPositions: {},
    });
    expect(await p).toEqual({ ok: false, reason: "refused" });
    expect(sketchStore.getState().session).toBe(superseding);
    expect(sketchStore.getState().undoStack).toHaveLength(0);
  });
});

// ── The cascade table (pure) ──────────────────────────────────────────────────
//
// Driven directly so every row is stated once, without a solver in the way. The
// rule: a LINE that grew is longer (every dimension + `Equal` over it is stale);
// an ARC that grew kept its centre and radius (so `Radius`/`Diameter`/`Equal`
// survive) but moved its endpoints (so the measures over them do not).

describe("remapExtendConstraints — cascade table", () => {
  let seq = 0;
  const nextId = (): string => `n${++seq}`;
  beforeEach(() => {
    seq = 0;
  });

  /** One constraint of `type` over the grown entity (+ a bystander where the
   *  kind needs a second ref), at the given position on the grown entity. */
  const one = (type: SketchConstraintType, positions?: SketchConstraint["positions"]): SketchConstraint => ({
    id: "c0",
    type,
    entities: ["old", "other"],
    ...(positions ? { positions } : {}),
  });

  const run = (
    c: SketchConstraint,
    targetType: SketchEntity["type"],
    movedEnd: "Start" | "End" = "End",
  ): SketchConstraint[] => remapExtendConstraints([c], { oldId: "old", newId: "new", movedEnd, targetType }, nextId);

  it("passes constraints that do not reference the grown entity through UNCHANGED", () => {
    const untouched: SketchConstraint = { id: "keep", type: "Horizontal", entities: ["other"] };
    const out = remapExtendConstraints(
      [untouched],
      { oldId: "old", newId: "new", movedEnd: "End", targetType: "Line" },
      nextId,
    );
    expect(out[0]).toBe(untouched); // same object: no wire op, no churn
  });

  it("DROPS anything anchored at the endpoint the extension moved", () => {
    expect(run(one("Coincident", ["End", "Start"]), "Line")).toEqual([]);
    expect(run(one("Coincident", ["Start", "Start"]), "Arc", "Start")).toEqual([]);
  });

  it("KEEPS a weld at the FAR end (this is what holds the profile together)", () => {
    const out = run(one("Coincident", ["Start", "End"]), "Line");
    expect(out).toHaveLength(1);
    expect(out[0].entities).toEqual(["new", "other"]);
    expect(out[0].id).toBe("n1"); // fresh id — a same-id rewrite emits no wire op
    expect(out[0].positions).toEqual(["Start", "End"]);
  });

  it.each([
    "Distance",
    "HorizontalDistance",
    "VerticalDistance",
    "Angle",
    "Radius",
    "Diameter",
    "Equal",
  ] as const)("DROPS %s on a grown LINE", (type) => {
    expect(run(one(type), "Line")).toEqual([]);
  });

  it.each(["Distance", "HorizontalDistance", "VerticalDistance", "Angle"] as const)(
    "DROPS %s on a grown ARC",
    (type) => {
      expect(run(one(type), "Arc")).toEqual([]);
    },
  );

  it.each(["Radius", "Diameter", "Equal"] as const)(
    "KEEPS %s on a grown ARC (its centre and radius did not move)",
    (type) => {
      const out = run(one(type), "Arc");
      expect(out).toHaveLength(1);
      expect(out[0].type).toBe(type);
      expect(out[0].entities).toEqual(["new", "other"]);
      expect(out[0].id).not.toBe("c0");
    },
  );

  it.each([
    "Horizontal",
    "Vertical",
    "Parallel",
    "Perpendicular",
    "Tangent",
    "Concentric",
    "Fixed",
    "OnCurve",
    "Midpoint",
    "Symmetric",
  ] as const)("KEEPS %s on both kinds (an extend changes length, not direction)", (type) => {
    for (const targetType of ["Line", "Arc"] as const) {
      const out = run(one(type), targetType);
      expect(out).toHaveLength(1);
      expect(out[0].entities).toEqual(["new", "other"]);
    }
  });
});
