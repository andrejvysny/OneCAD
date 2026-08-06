import { describe, it, expect } from "vitest";
import {
  DEFAULT_OFFSET_DISTANCE,
  distanceFromValueText,
  ghostOffsets,
  offsetAnchorFor,
  offsetAxisFor,
  offsetFaceValue,
  OFFSET_AXIS_MAX_DIVERGENCE_RAD,
  type FaceFrame,
} from "./faceOffset";

const frame = (center: [number, number, number], normal: [number, number, number]): FaceFrame => ({
  center,
  normal,
});

describe("offsetAxisFor", () => {
  it("returns the unit mean normal for one face", () => {
    expect(offsetAxisFor([frame([0, 0, 10], [0, 0, 3])])).toEqual([0, 0, 1]);
  });

  it("averages agreeing normals and re-normalizes", () => {
    // Two faces 60° apart — inside the 45°-from-the-MEAN tolerance (each deviates
    // 30°), so the pair still has one honest arrow between them.
    const axis = offsetAxisFor([
      frame([0, 0, 0], [1, 0, 0]),
      frame([0, 0, 0], [Math.cos(Math.PI / 3), Math.sin(Math.PI / 3), 0]),
    ]);
    expect(axis).not.toBeNull();
    expect(Math.hypot(...(axis as [number, number, number]))).toBeCloseTo(1, 12);
    // The bisector of the two: 30° off each.
    expect(axis?.[0]).toBeCloseTo(Math.cos(Math.PI / 6), 10);
    expect(axis?.[1]).toBeCloseTo(Math.sin(Math.PI / 6), 10);
  });

  it("REFUSES opposing normals rather than averaging them to nothing", () => {
    // The pair that motivates the whole refusal: their sum is the zero vector, so
    // a naive mean would normalize noise into a fabricated direction.
    expect(offsetAxisFor([frame([0, 0, 0], [0, 0, 1]), frame([0, 0, 0], [0, 0, -1])])).toBeNull();
  });

  it("REFUSES a set whose worst normal exceeds the divergence tolerance", () => {
    // Perpendicular faces: each is 45° from the mean, which is exactly AT the
    // tolerance — nudge one past it and the axis is refused.
    const past = OFFSET_AXIS_MAX_DIVERGENCE_RAD + 0.05;
    expect(
      offsetAxisFor([frame([0, 0, 0], [1, 0, 0]), frame([0, 0, 0], [Math.cos(2 * past), Math.sin(2 * past), 0])]),
    ).toBeNull();
  });

  it("refuses an empty set and a degenerate normal", () => {
    expect(offsetAxisFor([])).toBeNull();
    expect(offsetAxisFor([frame([0, 0, 0], [0, 0, 0])])).toBeNull();
  });
});

describe("offsetAnchorFor", () => {
  it("is the plain mean of the face centres", () => {
    expect(
      offsetAnchorFor([frame([0, 0, 0], [0, 0, 1]), frame([2, 4, 6], [0, 0, 1])]),
    ).toEqual([1, 2, 3]);
  });

  it("refuses an empty set and a non-finite centre (never substitutes the origin)", () => {
    expect(offsetAnchorFor([])).toBeNull();
    expect(offsetAnchorFor([frame([0, Number.NaN, 0], [0, 0, 1])])).toBeNull();
  });
});

describe("ghostOffsets", () => {
  it("moves EACH face along its OWN normal, not along the mean", () => {
    const offsets = ghostOffsets(
      [frame([0, 0, 0], [1, 0, 0]), frame([0, 0, 0], [0, 1, 0])],
      2.5,
    );
    expect(offsets).toEqual([
      [2.5, 0, 0],
      [0, 2.5, 0],
    ]);
  });

  it("normalizes the frame normal before scaling", () => {
    expect(ghostOffsets([frame([0, 0, 0], [0, 0, 4])], 3)).toEqual([[0, 0, 3]]);
  });

  it("carries the SIGN — a negative offset pulls the face back", () => {
    const [offset] = ghostOffsets([frame([0, 0, 0], [0, 0, 1])], -2);
    expect(offset[2]).toBe(-2);
    expect(offset[0]).toBeCloseTo(0, 12); // ±0 either way; the magnitude is what matters
    expect(offset[1]).toBeCloseTo(0, 12);
  });

  it("drops a degenerate frame rather than translating it by zero", () => {
    expect(ghostOffsets([frame([0, 0, 0], [0, 0, 0]), frame([0, 0, 0], [0, 0, 1])], 1)).toEqual([
      [0, 0, 1],
    ]);
  });

  it("yields nothing for a non-finite distance", () => {
    expect(ghostOffsets([frame([0, 0, 0], [0, 0, 1])], Number.NaN)).toEqual([]);
  });
});

describe("offsetFaceValue (mirrors dto.rs feature_value)", () => {
  it("prefixes the absolute cylindrical forms and keeps mm for the rest", () => {
    expect(offsetFaceValue(2.5, "Offset")).toEqual({
      valueText: "2.5 mm",
      primaryValue: 2.5,
      primaryValueKind: "length",
    });
    expect(offsetFaceValue(12, "Total")).toEqual({
      valueText: "12.0 mm",
      primaryValue: 12,
      primaryValueKind: "length",
    });
    expect(offsetFaceValue(6, "Radius")).toEqual({
      valueText: "R6.0",
      primaryValue: 6,
      primaryValueKind: "length",
    });
    expect(offsetFaceValue(8, "Diameter")).toEqual({
      valueText: "Ø8.0",
      primaryValue: 8,
      primaryValueKind: "diameter",
    });
  });

  it("keeps a NEGATIVE offset's sign in the row text", () => {
    expect(offsetFaceValue(-2.5, "Offset").valueText).toBe("-2.5 mm");
  });
});

describe("distanceFromValueText", () => {
  it("round-trips every shape offsetFaceValue emits", () => {
    for (const [d, t] of [
      [2.5, "Offset"],
      [12, "Total"],
      [6, "Radius"],
      [8, "Diameter"],
    ] as const) {
      expect(distanceFromValueText(offsetFaceValue(d, t).valueText)).toBe(d);
    }
  });

  it("keeps a negative offset NEGATIVE (the sign is meaningful here)", () => {
    // The fillet/shell parsers fall back on a non-positive value; doing that here
    // would silently turn a shrinking offset into a growing one.
    expect(distanceFromValueText("-2.5 mm")).toBe(-2.5);
  });

  it("falls back only on text with no number at all", () => {
    expect(distanceFromValueText("—")).toBe(DEFAULT_OFFSET_DISTANCE);
    expect(distanceFromValueText("", 7)).toBe(7);
  });
});
