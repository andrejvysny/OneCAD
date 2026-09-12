import { describe, expect, it } from "vitest";
import type { SketchEntity, SketchPlane } from "@/ipc/types";
import { sketchFrameBounds } from "./sketchFrameBounds";

const plane: SketchPlane = {
  kind: "custom",
  origin: [10, 20, 30],
  xAxis: [1, 0, 0],
  yAxis: [0, 0, 1],
  normal: [0, -1, 0],
};

describe("sketchFrameBounds", () => {
  it("maps finite non-construction geometry through the sketch plane", () => {
    const entities: SketchEntity[] = [
      { id: "line", type: "Line", p0: [-2, -3], p1: [4, 5] },
      { id: "far-construction", type: "Line", construction: true, p0: [100, 100], p1: [200, 200] },
    ];
    const bounds = sketchFrameBounds(plane, entities)!;
    expect(bounds.min.toArray()).toEqual([8, 20, 27]);
    expect(bounds.max.toArray()).toEqual([14, 20, 35]);
  });

  it("includes live drafts but refuses empty, point-only, and non-finite geometry", () => {
    expect(sketchFrameBounds(plane, [], [{ type: "Line", p0: { x: 1, y: 2 }, p1: { x: 5, y: 6 } }])).not.toBeNull();
    expect(sketchFrameBounds(plane, [])).toBeNull();
    expect(sketchFrameBounds(plane, [{ id: "p", type: "Point", p0: [1, 2] }])).toBeNull();
    expect(sketchFrameBounds(plane, [{ id: "bad", type: "Line", p0: [0, 0], p1: [Infinity, 1] }])).toBeNull();
  });
});
