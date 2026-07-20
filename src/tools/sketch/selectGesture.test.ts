import { describe, it, expect } from "vitest";
import {
  isDraggableHandle,
  pointRef,
  dragIntent,
  clickSelection,
  shouldApplyDrag,
} from "./selectGesture";
import type { SketchEntity } from "@/ipc/types";
import type { SketchSel } from "@/stores/sketchSelectionStore";

// ── (c) draggable-handle predicate ────────────────────────────────────────────
describe("isDraggableHandle", () => {
  it("Line Start/End are handles", () => {
    expect(isDraggableHandle("Line", "Start")).toBe(true);
    expect(isDraggableHandle("Line", "End")).toBe(true);
  });

  it("Circle Center is a handle", () => {
    expect(isDraggableHandle("Circle", "Center")).toBe(true);
  });

  it("Arc Center is a handle but Arc Start/End are NOT (worker-derived)", () => {
    expect(isDraggableHandle("Arc", "Center")).toBe(true);
    expect(isDraggableHandle("Arc", "Start")).toBe(false);
    expect(isDraggableHandle("Arc", "End")).toBe(false);
  });

  it("free Point Start is a handle", () => {
    expect(isDraggableHandle("Point", "Start")).toBe(true);
  });

  it("a body pick (no position) is never a handle", () => {
    expect(isDraggableHandle("Line", undefined)).toBe(false);
    expect(isDraggableHandle("Circle", undefined)).toBe(false);
  });

  it("mismatched positions are not handles", () => {
    expect(isDraggableHandle("Line", "Center")).toBe(false);
    expect(isDraggableHandle("Circle", "Start")).toBe(false);
  });
});

describe("pointRef", () => {
  it("builds `<entityId>.<Position>` for a point pick", () => {
    expect(pointRef({ entityId: "e1", point: "Start" })).toBe("e1.Start");
    expect(pointRef({ entityId: "c3", point: "Center" })).toBe("c3.Center");
  });

  it("returns null for a body pick", () => {
    expect(pointRef({ entityId: "e1" })).toBeNull();
  });
});

// ── (a) click-vs-drag classification ──────────────────────────────────────────
describe("dragIntent", () => {
  const entities: SketchEntity[] = [
    { id: "l1", type: "Line", p0: [0, 0], p1: [10, 0] },
    { id: "c1", type: "Circle", center: [5, 5], radius: 3 },
    { id: "a1", type: "Arc", center: [0, 0], radius: 4, start: [4, 0], end: [0, 4] },
  ];

  it("a draggable handle pick → a DragIntent with the point ref", () => {
    expect(dragIntent({ entityId: "l1", point: "End" }, entities)).toEqual({
      sel: { entityId: "l1", point: "End" },
      pointRef: "l1.End",
    });
    expect(dragIntent({ entityId: "c1", point: "Center" }, entities)?.pointRef).toBe("c1.Center");
  });

  it("an arc endpoint pick → null (not draggable)", () => {
    expect(dragIntent({ entityId: "a1", point: "Start" }, entities)).toBeNull();
  });

  it("a body pick → null (falls back to click-select)", () => {
    expect(dragIntent({ entityId: "l1" }, entities)).toBeNull();
  });

  it("a miss → null", () => {
    expect(dragIntent(null, entities)).toBeNull();
  });

  it("a pick on an unknown entity → null", () => {
    expect(dragIntent({ entityId: "ghost", point: "Start" }, entities)).toBeNull();
  });
});

// ── (a) shift/meta selection semantics ────────────────────────────────────────
describe("clickSelection", () => {
  const a: SketchSel = { entityId: "l1", point: "Start" };
  const b: SketchSel = { entityId: "c1" };

  it("plain + hit → replace with [hit]", () => {
    expect(clickSelection([b], a, false)).toEqual([a]);
  });

  it("plain + miss → clear", () => {
    expect(clickSelection([a, b], null, false)).toEqual([]);
  });

  it("additive + new hit → append (toggle on)", () => {
    expect(clickSelection([b], a, true)).toEqual([b, a]);
  });

  it("additive + already-selected hit → remove (toggle off)", () => {
    expect(clickSelection([a, b], a, true)).toEqual([b]);
  });

  it("additive + miss → unchanged (never clears)", () => {
    const cur = [a, b];
    expect(clickSelection(cur, null, true)).toBe(cur);
  });

  it("identity is entityId + point: a body and its vertex are distinct", () => {
    const body: SketchSel = { entityId: "l1" };
    const vtx: SketchSel = { entityId: "l1", point: "Start" };
    // Toggling the vertex while the body is selected adds it (does not remove the body).
    expect(clickSelection([body], vtx, true)).toEqual([body, vtx]);
  });
});

// ── (b, gate) latest-wins seq reconcile ───────────────────────────────────────
describe("shouldApplyDrag", () => {
  it("applies a newer seq", () => {
    expect(shouldApplyDrag(2, { seq: 3 })).toBe(true);
  });

  it("drops an equal or older seq (out-of-order)", () => {
    expect(shouldApplyDrag(3, { seq: 3 })).toBe(false);
    expect(shouldApplyDrag(3, { seq: 1 })).toBe(false);
  });

  it("drops a null/undefined response (client already dropped it)", () => {
    expect(shouldApplyDrag(0, null)).toBe(false);
    expect(shouldApplyDrag(0, undefined)).toBe(false);
  });
});
