import { describe, it, expect } from "vitest";
import { snapToDecade, chooseGridStep } from "./GridPlane";

describe("snapToDecade (1/2/5/10 progression)", () => {
  it("snaps within a decade", () => {
    expect(snapToDecade(1)).toBe(1);
    expect(snapToDecade(1.9)).toBe(2);
    expect(snapToDecade(2)).toBe(2);
    expect(snapToDecade(4.9)).toBe(5);
    expect(snapToDecade(5)).toBe(5);
    expect(snapToDecade(9.9)).toBe(10);
  });
  it("never lets one rung be more than 2.5x the value it snapped", () => {
    // The cell-size swing between rungs is what CELLS_ACROSS is budgeted
    // against — see GridPlane.ts.
    for (let v = 0.01; v < 1000; v *= 1.07) {
      expect(snapToDecade(v) / v).toBeLessThanOrEqual(2.5 + 1e-9);
      expect(snapToDecade(v)).toBeGreaterThanOrEqual(v - 1e-9);
    }
  });
  it("scales across decades", () => {
    expect(snapToDecade(10)).toBe(10);
    expect(snapToDecade(30)).toBe(50);
    expect(snapToDecade(0.3)).toBeCloseTo(0.5, 9);
  });
});

describe("chooseGridStep", () => {
  it("major is 10× minor", () => {
    const s = chooseGridStep(250);
    expect(s.major).toBe(s.minor * 10);
  });
  it("step grows with camera distance", () => {
    const near = chooseGridStep(50);
    const far = chooseGridStep(5000);
    expect(far.minor).toBeGreaterThan(near.minor);
  });
  it("close distance yields a fine step", () => {
    expect(chooseGridStep(50).minor).toBe(2); // 50/25 = 2
  });
  it("keeps the visible cell count in the countable band", () => {
    // ~10..25 cells across the camera distance — the property the readout chip
    // describes and the reason the grid no longer reads as texture.
    for (let d = 1; d <= 100_000; d *= 1.11) {
      const cells = d / chooseGridStep(d).minor;
      expect(cells, `d=${d}`).toBeGreaterThanOrEqual(25 / 2.5 - 1e-9);
      expect(cells, `d=${d}`).toBeLessThanOrEqual(25 + 1e-9);
    }
  });
});
