/*
 * `deriveAttachment` (WP-F1.2) — the pure half of attachment picking: what a
 * classified pick becomes. The frame is the load-bearing part, because the
 * placement solver seats a component BY it; an attachment whose `accepts` did
 * not match the snap kind its own geometry produces would simply never snap.
 */
import { describe, it, expect } from "vitest";
import type { ClassifyResult } from "@/ipc/types";
import { classifySnapKind, attachmentAccepts } from "./placementSolver";
import { deriveAttachment, perpendicularTo } from "./attachmentPicker";

const CIRCLE_EDGE: ClassifyResult = {
  kind: "edge",
  surfaceType: "",
  curveType: "circle",
  frame: { origin: [2, 2, 8], normal: null, axis: [0, 0, 1], radius: 3 },
};

describe("deriveAttachment", () => {
  it("authors a rim attachment at a circular edge's own centre", () => {
    // A circle has ONE meaningful centre, so the click position is not used —
    // unlike a face, where the clicked point picks out where the seat sits.
    const picked = deriveAttachment(CIRCLE_EDGE, [5, 5, 8]);
    expect(picked).toEqual({
      kind: "circularEdge",
      accepts: ["cylinder", "hole", "circularEdge"],
      frame: { origin: [2, 2, 8], z: [0, 0, 1], x: [1, 0, 0] },
    });
  });

  it("refuses geometry with no seatable frame rather than inventing one", () => {
    expect(
      deriveAttachment({ kind: "face", surfaceType: "plane", curveType: "", frame: null }, [0, 0, 0]),
    ).toBeNull();
    expect(
      deriveAttachment(
        {
          kind: "edge",
          surfaceType: "",
          curveType: "line",
          frame: { origin: [0, 0, 0], normal: null, axis: [1, 0, 0], radius: null },
        },
        [0, 0, 0],
      ),
    ).toBeNull();
  });

  it("falls back to world Y for the roll reference when z IS world X", () => {
    expect(perpendicularTo([1, 0, 0])).toEqual([0, 1, 0]);
    const picked = deriveAttachment(
      {
        kind: "face",
        surfaceType: "cylinder",
        curveType: "",
        frame: { origin: [0, 0, 0], normal: null, axis: [1, 0, 0], radius: 2 },
      },
      [5, 1, 1],
    );
    expect(picked?.frame).toEqual({ origin: [5, 0, 0], z: [1, 0, 0], x: [0, 1, 0] });
  });

  it("produces `accepts` the snap solver actually admits, for every picked kind", () => {
    const cases: ClassifyResult[] = [
      { kind: "face", surfaceType: "plane", curveType: "", frame: { origin: [0, 0, 0], normal: [0, 0, 1], axis: null, radius: null } },
      { kind: "face", surfaceType: "cylinder", curveType: "", frame: { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius: 3 } },
      CIRCLE_EDGE,
    ];
    for (const classify of cases) {
      const picked = deriveAttachment(classify, [0, 0, 0]);
      const snapKind = classifySnapKind(classify);
      expect(picked).not.toBeNull();
      expect(snapKind).not.toBeNull();
      expect(attachmentAccepts(picked!.accepts, snapKind!)).toBe(true);
    }
  });
});
