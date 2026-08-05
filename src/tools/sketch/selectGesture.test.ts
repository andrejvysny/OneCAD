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

  // SP-2 flipped Start/End from false to true: they ARE real solver points, and the
  // `curves` channel is what finally carries the reshape back into the document.
  it("every arc handle is draggable — centre translates, endpoints reshape", () => {
    expect(isDraggableHandle("Arc", "Center")).toBe(true);
    expect(isDraggableHandle("Arc", "Start")).toBe(true);
    expect(isDraggableHandle("Arc", "End")).toBe(true);
    expect(isDraggableHandle("Arc", "Midpoint")).toBe(false);
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

  const grab: [number, number] = [7, 1];

  it("a plain point handle → a `point` intent carrying the point ref", () => {
    expect(dragIntent({ entityId: "l1", point: "End" }, entities, grab)).toEqual({
      sel: { entityId: "l1", point: "End" },
      kind: "point",
      pointRef: "l1.End",
      role: "End",
      grab,
    });
    expect(dragIntent({ entityId: "c1", point: "Center" }, entities, grab)?.pointRef).toBe(
      "c1.Center",
    );
  });

  // SP-2: these three were ALL null before the gesture kinds existed.
  it("an arc endpoint → an `arcEnd` intent addressing the ENTITY", () => {
    expect(dragIntent({ entityId: "a1", point: "Start" }, entities, grab)).toEqual({
      sel: { entityId: "a1", point: "Start" },
      kind: "arcEnd",
      pointRef: "a1",
      role: "Start",
      grab,
    });
  });

  it("an arc CENTRE → `entityBody` (a point drag would pin its own endpoints)", () => {
    expect(dragIntent({ entityId: "a1", point: "Center" }, entities, grab)?.kind).toBe(
      "entityBody",
    );
  });

  it("a body pick → translate a line, resize a round curve", () => {
    expect(dragIntent({ entityId: "l1" }, entities, grab)?.kind).toBe("entityBody");
    expect(dragIntent({ entityId: "c1" }, entities, grab)?.kind).toBe("radius");
    expect(dragIntent({ entityId: "a1" }, entities, grab)?.kind).toBe("radius");
  });

  it("an ellipse body → null (not registered with the solver, so nothing can drag it)", () => {
    const withEllipse: SketchEntity[] = [
      { id: "el1", type: "Ellipse", center: [0, 0], majorR: 5, minorR: 2, rotation: 0 },
    ];
    expect(dragIntent({ entityId: "el1" }, withEllipse, grab)).toBeNull();
    expect(dragIntent({ entityId: "el1", point: "Center" }, withEllipse, grab)?.kind).toBe("point");
  });

  it("a miss → null", () => {
    expect(dragIntent(null, entities, grab)).toBeNull();
  });

  it("a pick on an unknown entity → null", () => {
    expect(dragIntent({ entityId: "ghost", point: "Start" }, entities, grab)).toBeNull();
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

// ── W3 P3: an Ellipse's CENTRE is a real drag handle ─────────────────────────

describe("isDraggableHandle / dragIntent — Ellipse", () => {
  const ell: SketchEntity = {
    id: "el1",
    type: "Ellipse",
    center: [5, 5],
    majorR: 10,
    minorR: 3,
    rotation: 0,
  };

  it("Center is draggable; nothing else is", () => {
    expect(isDraggableHandle("Ellipse", "Center")).toBe(true);
    expect(isDraggableHandle("Ellipse", "Start")).toBe(false);
    expect(isDraggableHandle("Ellipse", "End")).toBe(false);
    expect(isDraggableHandle("Ellipse", "Midpoint")).toBe(false);
    expect(isDraggableHandle("Ellipse", undefined)).toBe(false);
  });

  it("a Center hit arms a drag with the `<id>.Center` point ref", () => {
    expect(dragIntent({ entityId: "el1", point: "Center" }, [ell], [0, 0])).toEqual({
      sel: { entityId: "el1", point: "Center" },
      kind: "point",
      pointRef: "el1.Center",
      role: "Center",
      grab: [0, 0],
    });
  });

  // Unchanged by SP-2: an ellipse curve is the one body pick with no gesture kind,
  // because the ellipse is never registered with the solver.
  it("a BODY hit is click-select, never a drag", () => {
    expect(dragIntent({ entityId: "el1" }, [ell], [0, 0])).toBeNull();
  });
});
