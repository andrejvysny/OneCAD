/*
 * entityNames — the derived display names the constraint rows and the
 * conflicting-constraint sentence read (WP-2, S10/S11).
 */
import { describe, it, expect } from "vitest";
import { constraintRowLabel, directionSkippedHint, entityDisplayName } from "./entityNames";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";

const entities: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
  { id: "e2", type: "Line", p0: [40, 0], p1: [40, 30] },
  { id: "e3", type: "Circle", center: [0, 0], radius: 5 },
  { id: "e4", type: "Arc", center: [0, 0], radius: 5, start: [5, 0], end: [0, 5] },
  { id: "e5", type: "Line", p0: [0, 30], p1: [0, 0] },
];

describe("entityDisplayName", () => {
  it("numbers each kind independently, in session order", () => {
    expect(entityDisplayName(entities, "e1")).toBe("Line 1");
    expect(entityDisplayName(entities, "e2")).toBe("Line 2");
    expect(entityDisplayName(entities, "e5")).toBe("Line 3");
    expect(entityDisplayName(entities, "e3")).toBe("Circle 1");
    expect(entityDisplayName(entities, "e4")).toBe("Arc 1");
  });

  it("answers null for an id this session does not hold", () => {
    expect(entityDisplayName(entities, "e9")).toBeNull();
    expect(entityDisplayName([], "e1")).toBeNull();
  });
});

describe("constraintRowLabel", () => {
  const label = (c: SketchConstraint, opts?: { value?: string | null }): string =>
    constraintRowLabel(c, entities, opts);

  it("names the single operand of an axis lock", () => {
    expect(label({ id: "c1", type: "Horizontal", entities: ["e2"] })).toBe("Horizontal · Line 2");
  });

  it("names both points of a positional pair", () => {
    expect(
      label({
        id: "c2",
        type: "Coincident",
        entities: ["e1", "e2"],
        positions: ["End", "Start"],
      }),
    ).toBe("Coincident · Line 1 end – Line 2 start");
  });

  it("uses the relation glyph for Perpendicular and Parallel", () => {
    expect(label({ id: "c3", type: "Perpendicular", entities: ["e2", "e5"] })).toBe(
      "Perpendicular · Line 2 ⟂ Line 3",
    );
    expect(label({ id: "c4", type: "Parallel", entities: ["e1", "e5"] })).toBe(
      "Parallel · Line 1 ∥ Line 3",
    );
  });

  it("carries a pre-formatted value when the caller has no separate value column", () => {
    const distance: SketchConstraint = { id: "c5", type: "Distance", entities: ["e1"], value: 30 };
    expect(label(distance, { value: "30 mm" })).toBe("Distance 30 mm · Line 1");
    expect(label(distance)).toBe("Distance · Line 1");
  });

  it("drops the whole operand clause rather than leaking an id it cannot name", () => {
    expect(label({ id: "c6", type: "Coincident", entities: ["e1", "gone"] })).toBe("Coincident");
    expect(label({ id: "c7", type: "Horizontal", entities: ["gone"] })).toBe("Horizontal");
  });

  it("uses the catalog label, not the raw wire type", () => {
    expect(label({ id: "c8", type: "OnCurve", entities: ["e1", "e3"], positions: ["Start"] })).toBe(
      "On curve · Line 1 start – Circle 1",
    );
  });
});

describe("directionSkippedHint", () => {
  it("states the witness as the fact that beat the candidate (B3 copy)", () => {
    expect(
      directionSkippedHint("Perpendicular", [{ id: "c1", type: "Vertical", entities: ["e2"] }], entities),
    ).toBe("Perpendicular skipped — Line 2 is already vertical");
  });

  it("joins a multi-constraint witness", () => {
    expect(
      directionSkippedHint(
        "Parallel",
        [
          { id: "c1", type: "Horizontal", entities: ["e1"] },
          { id: "c2", type: "Perpendicular", entities: ["e1", "e2"] },
        ],
        entities,
      ),
    ).toBe(
      "Parallel skipped — Line 1 is already horizontal and Line 1 is already perpendicular to Line 2",
    );
  });

  it("claims no reason it cannot show", () => {
    expect(
      directionSkippedHint("Perpendicular", [{ id: "c1", type: "Vertical", entities: ["gone"] }], entities),
    ).toBe("Perpendicular skipped — it contradicts the sketch's existing directions");
    expect(directionSkippedHint("Perpendicular", [], entities)).toBe(
      "Perpendicular skipped — it contradicts the sketch's existing directions",
    );
  });
});
