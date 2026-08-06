import { describe, expect, it } from "vitest";
import {
  VISUAL_CHORD_ERROR,
  VISUAL_MAX_ANGLE_DEG,
  chordErrorForSweep,
  visualSegmentsForClosedCurve,
  visualSegmentsForSweep,
} from "./visualTessellation";

describe("visual tessellation policy", () => {
  it("keeps a small full circle below the angular visibility limit", () => {
    const segments = visualSegmentsForClosedCurve(5);
    expect(segments).toBe(Math.ceil(360 / VISUAL_MAX_ANGLE_DEG));
  });

  it("adds segments for large silhouettes until the chord error is bounded", () => {
    const segments = visualSegmentsForClosedCurve(100);
    expect(segments).toBeGreaterThan(visualSegmentsForClosedCurve(5));
    expect(chordErrorForSweep(360, 100, segments)).toBeLessThanOrEqual(VISUAL_CHORD_ERROR);
  });

  it("applies the same policy to partial sweeps and clamps malformed inputs", () => {
    expect(visualSegmentsForSweep(90, 100)).toBe(Math.ceil(visualSegmentsForClosedCurve(100) / 4));
    expect(visualSegmentsForSweep(0, 10)).toBe(2);
    expect(visualSegmentsForSweep(Number.NaN, Number.NaN)).toBe(2);
  });
});
