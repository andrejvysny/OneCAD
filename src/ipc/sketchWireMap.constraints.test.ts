/*
 * sketchWireMap — S4b wire extension: the 5 user-applicable geometric constraint
 * kinds (Fixed / OnCurve / Tangent / Concentric / Symmetric) that `toWireConstraint`
 * previously dropped at the null seam. Exact Rust-typed-doc JSON pinned (kind-tagged
 * camelCase; verified field-for-field against
 * `src-tauri/crates/onecad-core/src/sketch/constraint.rs`).
 */
import { beforeEach, describe, expect, it } from "vitest";
import { createIdMap, marshalUpsert } from "./sketchWireMap";
import type { SketchConstraint, SketchEntity } from "./types";

let seq = 0;
const mint = () => `uuid-${++seq}`;
beforeEach(() => {
  seq = 0;
});

const point = (id: string, p0: [number, number]): SketchEntity => ({ id, type: "Point", p0 });
const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({ id, type: "Line", p0, p1 });
const circle = (id: string, center: [number, number], radius: number): SketchEntity => ({ id, type: "Circle", center, radius });
const arc = (id: string, center: [number, number], radius: number, start: [number, number], end: [number, number]): SketchEntity => ({ id, type: "Arc", center, radius, start, end });

const addConstraintOp = (ops: ReturnType<typeof marshalUpsert>) => ops.find((o) => o.op === "addConstraint");

describe("toWireConstraint — S4b geometric kinds", () => {
  it("Fixed → {kind:'fixed', point, at:<current coords>}", () => {
    const map = createIdMap("sk", "XY");
    const constraints: SketchConstraint[] = [
      { id: "c1", type: "Fixed", entities: ["p1"], positions: ["Start"] },
    ];
    const ops = marshalUpsert(map, { entities: [point("p1", [3, 4])], constraints }, mint);
    // p1 → uuid-1 (point + entity); constraint → uuid-2.
    expect(addConstraintOp(ops)).toEqual({
      op: "addConstraint",
      constraint: { kind: "fixed", id: "uuid-2", point: "uuid-1", at: [3, 4] },
    });
  });

  it("Fixed on a LINE endpoint carries that endpoint's coords + synthesized point id", () => {
    const map = createIdMap("sk", "XY");
    const constraints: SketchConstraint[] = [
      { id: "c1", type: "Fixed", entities: ["l1"], positions: ["End"] },
    ];
    const ops = marshalUpsert(map, { entities: [line("l1", [0, 0], [40, 0])], constraints }, mint);
    // l1: Start uuid-1, End uuid-2, line uuid-3; constraint uuid-4.
    expect(addConstraintOp(ops)).toEqual({
      op: "addConstraint",
      constraint: { kind: "fixed", id: "uuid-4", point: "uuid-2", at: [40, 0] },
    });
  });

  it("OnCurve → {kind:'onCurve', point, curve} (Arbitrary ⇒ no position field)", () => {
    const map = createIdMap("sk", "XY");
    const constraints: SketchConstraint[] = [
      { id: "c1", type: "OnCurve", entities: ["p1", "l1"], positions: ["Start"] },
    ];
    const ops = marshalUpsert(
      map,
      { entities: [point("p1", [1, 1]), line("l1", [0, 0], [10, 0])], constraints },
      mint,
    );
    // p1 → uuid-1; l1: Start uuid-2, End uuid-3, line uuid-4; constraint uuid-5.
    const c = addConstraintOp(ops);
    expect(c).toEqual({
      op: "addConstraint",
      constraint: { kind: "onCurve", id: "uuid-5", point: "uuid-1", curve: "uuid-4" },
    });
    // No `position` key (defaults to Arbitrary on the Rust side).
    expect("position" in (c as { constraint: object }).constraint).toBe(false);
  });

  it("Tangent → {kind:'tangent', entity1, entity2} (BUG-1: autoConstrain's arc↔line reaches the solver)", () => {
    const map = createIdMap("sk", "XY");
    const constraints: SketchConstraint[] = [
      { id: "c1", type: "Tangent", entities: ["a1", "l1"] },
    ];
    const ops = marshalUpsert(
      map,
      { entities: [arc("a1", [0, 0], 5, [5, 0], [0, 5]), line("l1", [5, 0], [5, 40])], constraints },
      mint,
    );
    // a1: center uuid-1, arc uuid-2; l1: Start uuid-3, End uuid-4, line uuid-5; constraint uuid-6.
    expect(addConstraintOp(ops)).toEqual({
      op: "addConstraint",
      constraint: { kind: "tangent", id: "uuid-6", entity1: "uuid-2", entity2: "uuid-5" },
    });
  });

  it("Concentric → {kind:'concentric', entity1, entity2}", () => {
    const map = createIdMap("sk", "XY");
    const constraints: SketchConstraint[] = [
      { id: "c1", type: "Concentric", entities: ["c1e", "c2e"] },
    ];
    const ops = marshalUpsert(
      map,
      { entities: [circle("c1e", [0, 0], 3), circle("c2e", [0, 0], 6)], constraints },
      mint,
    );
    // c1e: center uuid-1, circle uuid-2; c2e: center uuid-3, circle uuid-4; constraint uuid-5.
    expect(addConstraintOp(ops)).toEqual({
      op: "addConstraint",
      constraint: { kind: "concentric", id: "uuid-5", entity1: "uuid-2", entity2: "uuid-4" },
    });
  });

  it("Symmetric → {kind:'symmetric', point1, point2, axis}", () => {
    const map = createIdMap("sk", "XY");
    const constraints: SketchConstraint[] = [
      { id: "c1", type: "Symmetric", entities: ["p1", "p2", "l1"], positions: ["Start", "Start"] },
    ];
    const ops = marshalUpsert(
      map,
      {
        entities: [point("p1", [-5, 0]), point("p2", [5, 0]), line("l1", [0, -10], [0, 10])],
        constraints,
      },
      mint,
    );
    // p1 → uuid-1; p2 → uuid-2; l1: Start uuid-3, End uuid-4, line uuid-5; constraint uuid-6.
    expect(addConstraintOp(ops)).toEqual({
      op: "addConstraint",
      constraint: { kind: "symmetric", id: "uuid-6", point1: "uuid-1", point2: "uuid-2", axis: "uuid-5" },
    });
  });

  it("no longer drops the 5 kinds (the constraint id IS recorded in the map)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(
      map,
      { entities: [point("p1", [3, 4])], constraints: [{ id: "cF", type: "Fixed", entities: ["p1"], positions: ["Start"] }] },
      mint,
    );
    expect(map.constraint.has("cF")).toBe(true);
  });
});
