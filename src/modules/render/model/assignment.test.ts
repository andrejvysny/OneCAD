import { describe, it, expect } from "vitest";
import {
  faceOverrideCount,
  partitionDanglingAssignments,
  resolveAssignment,
  type RenderAssignments,
} from "./assignment";

const MAT_A = "mat_a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1";
const MAT_B = "mat_b2b2b2b2-b2b2-4b2b-8b2b-b2b2b2b2b2b2";

function assignments(over: Partial<RenderAssignments> = {}): RenderAssignments {
  return { bodies: {}, faces: {}, ...over };
}

describe("resolveAssignment", () => {
  it("returns null when neither body nor face is assigned", () => {
    expect(resolveAssignment(assignments(), "body-1")).toBeNull();
    expect(resolveAssignment(assignments(), "body-1", "el_face-1")).toBeNull();
  });

  it("returns the body assignment when there is no face override", () => {
    const a = assignments({ bodies: { "body-1": MAT_A } });
    expect(resolveAssignment(a, "body-1")).toBe(MAT_A);
    expect(resolveAssignment(a, "body-1", "el_face-1")).toBe(MAT_A);
  });

  it("face override wins over the body assignment", () => {
    const a = assignments({
      bodies: { "body-1": MAT_A },
      faces: { "el_face-1": MAT_B },
    });
    expect(resolveAssignment(a, "body-1", "el_face-1")).toBe(MAT_B);
    // A different, unassigned face on the same body still falls back to the body.
    expect(resolveAssignment(a, "body-1", "el_face-2")).toBe(MAT_A);
  });

  it("an unassigned body with an assigned face still resolves the face", () => {
    const a = assignments({ faces: { "el_face-1": MAT_B } });
    expect(resolveAssignment(a, "body-1", "el_face-1")).toBe(MAT_B);
  });
});

describe("faceOverrideCount", () => {
  it("counts only the faces present in the assignment map", () => {
    const a = assignments({
      faces: { "el_1": MAT_A, "el_2": MAT_B, "el_unrelated": MAT_A },
    });
    expect(faceOverrideCount(a, ["el_1", "el_2", "el_3"])).toBe(2);
  });

  it("is zero for an empty face id list or no overrides", () => {
    expect(faceOverrideCount(assignments(), [])).toBe(0);
    expect(faceOverrideCount(assignments(), ["el_1", "el_2"])).toBe(0);
  });
});

describe("partitionDanglingAssignments", () => {
  it("classifies known ids as live and unknown ids as dangling, without mutating the input", () => {
    const input = assignments({
      bodies: { "body-live": MAT_A, "body-gone": MAT_B },
      faces: { "el_live": MAT_A, "el_gone": MAT_B },
    });
    const inputSnapshot = JSON.parse(JSON.stringify(input));

    const result = partitionDanglingAssignments(
      input,
      new Set(["body-live"]),
      new Set(["el_live"]),
    );

    expect(result.live).toEqual(assignments({ bodies: { "body-live": MAT_A }, faces: { "el_live": MAT_A } }));
    expect(result.dangling).toEqual({ bodies: ["body-gone"], faces: ["el_gone"] });
    expect(input).toEqual(inputSnapshot);
  });

  it("is a no-op partition when everything is known", () => {
    const input = assignments({ bodies: { b1: MAT_A }, faces: { f1: MAT_B } });
    const result = partitionDanglingAssignments(input, new Set(["b1"]), new Set(["f1"]));
    expect(result.live).toEqual(input);
    expect(result.dangling).toEqual({ bodies: [], faces: [] });
  });

  it("reports everything as dangling rather than silently dropping it when nothing is known", () => {
    const input = assignments({ bodies: { b1: MAT_A }, faces: { f1: MAT_B } });
    const result = partitionDanglingAssignments(input, new Set(), new Set());
    expect(result.live).toEqual(assignments());
    expect(result.dangling).toEqual({ bodies: ["b1"], faces: ["f1"] });
  });
});
