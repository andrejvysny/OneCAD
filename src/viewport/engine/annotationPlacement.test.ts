import { describe, expect, it } from "vitest";
import { placeAnnotations } from "./annotationPlacement";

const viewport = { x: 0, y: 0, width: 300, height: 200 };
const size = { width: 60, height: 24 };

function rect(center: { x: number; y: number }, dimensions = size) {
  return { x: center.x - dimensions.width / 2, y: center.y - dimensions.height / 2, width: dimensions.width, height: dimensions.height };
}

function clearByPad(a: ReturnType<typeof rect>, b: ReturnType<typeof rect>, pad = 6): boolean {
  return a.x + a.width + pad <= b.x || b.x + b.width + pad <= a.x
    || a.y + a.height + pad <= b.y || b.y + b.height + pad <= a.y;
}

function inside(rectangle: ReturnType<typeof rect>, bounds: typeof viewport): boolean {
  return rectangle.x >= bounds.x && rectangle.y >= bounds.y
    && rectangle.x + rectangle.width <= bounds.x + bounds.width
    && rectangle.y + rectangle.height <= bounds.y + bounds.height;
}

describe("placeAnnotations", () => {
  it("keeps pair at its desired point and places lower priorities inside padded clear space", () => {
    const protectedBox = { x: 10, y: 10, width: 40, height: 30 };
    const placements = placeAnnotations([
      { id: "a", priority: 100, center: { x: 150, y: 100 }, size, pinned: false, visible: true },
      { id: "b", priority: 200, center: { x: 150, y: 100 }, size, pinned: false, visible: true },
      { id: "pair", priority: 300, center: { x: 150, y: 100 }, size, pinned: false, visible: true },
    ], viewport, viewport, [protectedBox]);
    const byId = new Map(placements.map((placement) => [placement.id, placement]));
    const pair = byId.get("pair")!;
    const b = byId.get("b")!;
    const a = byId.get("a")!;

    expect(pair).toMatchObject({ status: "visible", center: { x: 150, y: 100 } });
    expect([a, b, pair].every((placement) => placement.status === "visible")).toBe(true);
    const boxes = [a, b, pair].map((placement) => rect(placement.center!));
    expect(boxes.every((box) => inside(box, viewport))).toBe(true);
    expect(boxes.every((box) => clearByPad(box, protectedBox))).toBe(true);
    expect(clearByPad(boxes[0], boxes[1])).toBe(true);
    expect(clearByPad(boxes[0], boxes[2])).toBe(true);
    expect(clearByPad(boxes[1], boxes[2])).toBe(true);
  });

  it("is deterministic despite input order and gives contested capacity to pair", () => {
    const annotations = [
      { id: "a", priority: 100, center: { x: 35, y: 15 }, size, pinned: false, visible: true },
      { id: "b", priority: 200, center: { x: 35, y: 15 }, size, pinned: false, visible: true },
      { id: "pair", priority: 300, center: { x: 35, y: 15 }, size, pinned: false, visible: true },
    ];
    const expected = placeAnnotations(annotations, { x: 0, y: 0, width: 70, height: 30 }, { x: 0, y: 0, width: 70, height: 30 }, []);
    for (const order of [[0, 1, 2], [2, 1, 0], [1, 0, 2]]) {
      const actual = placeAnnotations(order.map((index) => annotations[index]), { x: 0, y: 0, width: 70, height: 30 }, { x: 0, y: 0, width: 70, height: 30 }, []);
      expect(new Map(actual.map((placement) => [placement.id, placement.status]))).toEqual(
        new Map(expected.map((placement) => [placement.id, placement.status])),
      );
    }
    expect(new Map(expected.map((placement) => [placement.id, placement.status]))).toEqual(new Map([
      ["a", "no-space"], ["b", "no-space"], ["pair", "visible"],
    ]));
  });

  it("keeps a pinned label fixed and hides it when the fixed position is protected", () => {
    const [placement] = placeAnnotations([
      { id: "pair", priority: 300, center: { x: 150, y: 100 }, size, pinned: true, visible: true },
    ], viewport, viewport, [{ x: 120, y: 80, width: 60, height: 40 }]);

    expect(placement).toEqual({ id: "pair", center: null, status: "no-space" });
  });

  it("keeps a valid pinned point exact instead of relocating it", () => {
    const [placement] = placeAnnotations([
      { id: "pair", priority: 300, center: { x: 100, y: 60 }, size, pinned: true, visible: true },
    ], viewport, viewport, []);

    expect(placement).toEqual({ id: "pair", center: { x: 100, y: 60 }, status: "visible" });
  });

  it("uses safe and protected box edges rather than an arbitrary displacement ring", () => {
    const [placement] = placeAnnotations([
      { id: "pair", priority: 300, center: { x: 80, y: 80 }, size, pinned: false, visible: true },
    ], viewport, viewport, [{ x: 50, y: 60, width: 60, height: 40 }]);

    expect(placement.status).toBe("visible");
    expect(placement.center).not.toEqual({ x: 80, y: 80 });
  });

  it("returns unknown for an unmeasured label and no-space for an invalid safe rectangle", () => {
    const unknown = placeAnnotations([
      { id: "a", priority: 100, center: { x: 10, y: 10 }, size: null, pinned: false, visible: true },
    ], viewport, viewport, []);
    const noSpace = placeAnnotations([
      { id: "a", priority: 100, center: { x: 10, y: 10 }, size, pinned: false, visible: true },
    ], viewport, { x: 0, y: 0, width: 0, height: 20 }, []);

    expect(unknown[0].status).toBe("unknown");
    expect(noSpace[0].status).toBe("no-space");
  });

  it("normalizes partial safe rectangles and rejects non-finite placement input", () => {
    const partialSize = { width: 40, height: 24 };
    const [partial] = placeAnnotations([
      { id: "a", priority: 100, center: { x: -10, y: 20 }, size: partialSize, pinned: false, visible: true },
    ], viewport, { x: -30, y: 0, width: 80, height: 80 }, []);
    const invalid = placeAnnotations([
      { id: "nan", priority: 100, center: { x: Number.NaN, y: 20 }, size, pinned: false, visible: true },
      { id: "infinity", priority: 100, center: { x: 20, y: 20 }, size: { width: Number.POSITIVE_INFINITY, height: 20 }, pinned: false, visible: true },
    ], viewport, viewport, []);

    expect(partial.status).toBe("visible");
    expect(inside(rect(partial.center!, partialSize), { x: 0, y: 0, width: 50, height: 80 })).toBe(true);
    expect(invalid.map((placement) => placement.status)).toEqual(["unknown", "unknown"]);
  });
});
