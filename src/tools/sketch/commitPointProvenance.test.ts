/*
 * Commit point provenance — the per-tool click → committed-point mapping
 * (SNAP §9.3).
 *
 * The table is normative: get one row wrong and an accepted relation is
 * authored against the wrong point, which is worse than not authoring it.
 */
import { describe, expect, it } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import { commitPointRefs, orderSlotCaps } from "./commitPointProvenance";

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({
  id,
  type: "Line",
  p0,
  p1,
});
const arcAt = (id: string, center: [number, number]): SketchEntity => ({
  id,
  type: "Arc",
  center,
  radius: 5,
  start: [center[0] + 5, center[1]],
  end: [center[0], center[1] + 5],
});

describe("commitPointRefs", () => {
  it("line: prior anchor → Start, committing click → End", () => {
    const refs = commitPointRefs({
      toolId: "line",
      entities: [line("l1", [0, 0], [10, 0])],
      priorAnchorCount: 1,
    });
    expect(refs).toEqual([
      { entityId: "l1", position: "Start" },
      { entityId: "l1", position: "End" },
    ]);
  });

  it("line in ARC mode maps the same two roles onto the committed Arc", () => {
    const refs = commitPointRefs({
      toolId: "line",
      entities: [arcAt("a1", [0, 0])],
      priorAnchorCount: 1,
      arcMode: true,
    });
    expect(refs).toEqual([
      { entityId: "a1", position: "Start" },
      { entityId: "a1", position: "End" },
    ]);
  });

  it("rect: first diagonal corner → line 0 Start, second → line 1 End", () => {
    const entities = [
      line("l0", [0, 0], [10, 0]),
      line("l1", [10, 0], [10, 5]),
      line("l2", [10, 5], [0, 5]),
      line("l3", [0, 5], [0, 0]),
    ];
    expect(commitPointRefs({ toolId: "rect", entities, priorAnchorCount: 1 })).toEqual([
      { entityId: "l0", position: "Start" },
      { entityId: "l1", position: "End" },
    ]);
  });

  it("centerRect: the CENTRE is placement-only (no centre entity exists)", () => {
    const entities = [
      line("l0", [0, 0], [10, 0]),
      line("l1", [10, 0], [10, 5]),
      line("l2", [10, 5], [0, 5]),
      line("l3", [0, 5], [0, 0]),
    ];
    expect(commitPointRefs({ toolId: "centerRect", entities, priorAnchorCount: 1 })).toEqual([
      null,
      { entityId: "l1", position: "End" },
    ]);
  });

  it("circle: centre maps, the radius click does not", () => {
    const entities: SketchEntity[] = [{ id: "c1", type: "Circle", center: [0, 0], radius: 5 }];
    expect(commitPointRefs({ toolId: "circle", entities, priorAnchorCount: 1 })).toEqual([
      { entityId: "c1", position: "Center" },
      null,
    ]);
  });

  it("arc: centre → Center, second anchor → Start, final click → End", () => {
    expect(
      commitPointRefs({ toolId: "arc", entities: [arcAt("a1", [0, 0])], priorAnchorCount: 2 }),
    ).toEqual([
      { entityId: "a1", position: "Center" },
      { entityId: "a1", position: "Start" },
      { entityId: "a1", position: "End" },
    ]);
  });

  it("arc3p: the two endpoints map, the through-point is placement-only", () => {
    expect(
      commitPointRefs({ toolId: "arc3p", entities: [arcAt("a1", [0, 0])], priorAnchorCount: 2 }),
    ).toEqual([
      { entityId: "a1", position: "Start" },
      { entityId: "a1", position: "End" },
      null,
    ]);
  });

  it("ellipse: the centre maps, both axis clicks are placement-only", () => {
    const entities: SketchEntity[] = [
      { id: "e1", type: "Ellipse", center: [0, 0], majorR: 10, minorR: 5, rotation: 0 },
    ];
    expect(commitPointRefs({ toolId: "ellipse", entities, priorAnchorCount: 2 })).toEqual([
      { entityId: "e1", position: "Center" },
      null,
      null,
    ]);
  });

  it("polygon: centre → construction circumcircle Center, vertex → line 0 Start", () => {
    const entities: SketchEntity[] = [
      line("l0", [10, 0], [5, 8]),
      line("l1", [5, 8], [-5, 8]),
      { id: "cc", type: "Circle", center: [0, 0], radius: 10, construction: true },
    ];
    expect(commitPointRefs({ toolId: "polygon", entities, priorAnchorCount: 1, sides: 3 })).toEqual([
      { entityId: "cc", position: "Center" },
      { entityId: "l0", position: "Start" },
    ]);
  });

  it("slot: the two cap CENTRES map, the width click is placement-only", () => {
    const entities: SketchEntity[] = [
      line("w0", [-50, 20], [50, 20]),
      line("w1", [-50, -20], [50, -20]),
      arcAt("capA", [-50, 0]),
      arcAt("capB", [50, 0]),
    ];
    expect(commitPointRefs({ toolId: "slot", entities, priorAnchorCount: 2 })).toEqual([
      { entityId: "capA", position: "Center" },
      { entityId: "capB", position: "Center" },
      null,
    ]);
  });

  it("point: the click maps to Point.Start", () => {
    const entities: SketchEntity[] = [{ id: "p1", type: "Point", p0: [3, 4] }];
    expect(commitPointRefs({ toolId: "point", entities, priorAnchorCount: 0 })).toEqual([
      { entityId: "p1", position: "Start" },
    ]);
  });

  it("an unknown tool maps nothing rather than guessing", () => {
    expect(commitPointRefs({ toolId: "nope", entities: [], priorAnchorCount: 2 })).toEqual([
      null,
      null,
      null,
    ]);
  });

  it("always returns one slot per placement", () => {
    for (const n of [0, 1, 2, 3]) {
      expect(
        commitPointRefs({ toolId: "line", entities: [line("l1", [0, 0], [1, 1])], priorAnchorCount: n }),
      ).toHaveLength(n + 1);
    }
  });
});

describe("orderSlotCaps", () => {
  const entities: SketchEntity[] = [arcAt("capA", [-50, 0]), arcAt("capB", [50, 0])];

  it("keeps the order when it already matches the click order", () => {
    const refs = [
      { entityId: "capA", position: "Center" as const },
      { entityId: "capB", position: "Center" as const },
      null,
    ];
    expect(orderSlotCaps(refs, entities, [{ x: -50, y: 0 }, { x: 50, y: 0 }])).toEqual(refs);
  });

  it("swaps when the machine emitted the caps the other way round", () => {
    const refs = [
      { entityId: "capB", position: "Center" as const },
      { entityId: "capA", position: "Center" as const },
      null,
    ];
    expect(orderSlotCaps(refs, entities, [{ x: -50, y: 0 }, { x: 50, y: 0 }])).toEqual([
      { entityId: "capA", position: "Center" },
      { entityId: "capB", position: "Center" },
      null,
    ]);
  });

  it("is inert without two caps or two anchors", () => {
    const refs = [{ entityId: "capA", position: "Center" as const }];
    expect(orderSlotCaps(refs, entities, [{ x: 0, y: 0 }])).toEqual(refs);
  });
});
