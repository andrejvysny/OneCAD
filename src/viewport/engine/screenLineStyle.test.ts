/*
 * The screen-space line unit contract (VP-HARDENING VP03, spec §7).
 *
 * TEST-LINE-01 (lane U): a CSS-authored width reaches `LineMaterial.linewidth`
 * unscaled at every DPR, and the pick threshold that cancels it out is CSS on
 * both sides. The G/N lanes of TEST-LINE-01 (measured widths on a real display)
 * are owed separately.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import {
  LINE_WIDTHS_CSS,
  applyScreenLineStyle,
  createScreenLineMaterial,
  line2PickThresholdCss,
  setLineResolutionCss,
} from "./screenLineStyle";

afterEach(() => vi.unstubAllGlobals());

describe("TEST-LINE-01 — CSS widths pass through unscaled", () => {
  it("hands LineMaterial the authored width at DPR 1, 1.5 and 2", () => {
    for (const dpr of [1, 1.5, 2]) {
      vi.stubGlobal("devicePixelRatio", dpr);
      const mat = createScreenLineMaterial({ widthCss: LINE_WIDTHS_CSS.bodyFeatureEdge });
      expect(mat.linewidth).toBe(1.25);
      mat.dispose();
      vi.unstubAllGlobals();
    }
  });

  it("carries every baseline width from spec §7.3 verbatim", () => {
    expect(LINE_WIDTHS_CSS.bodyFeatureEdge).toBe(1.25);
    expect(LINE_WIDTHS_CSS.tangentEdge).toBe(0.75);
    expect(LINE_WIDTHS_CSS.activeSketchInk).toBe(1.25);
    expect(LINE_WIDTHS_CSS.staticSketchInk).toBe(1.0);
    expect(LINE_WIDTHS_CSS.draft).toBe(1.25);
    expect(LINE_WIDTHS_CSS.selectionHalo).toBe(3.0);
    expect(LINE_WIDTHS_CSS.dimensionOrSnapGuide).toBe(1.0);
    expect(LINE_WIDTHS_CSS.hiddenSelectedEdge).toBe(1.0);
  });

  it("passes dash flags through and lets `params` win over the seeded values", () => {
    const mat = createScreenLineMaterial(
      { widthCss: 1, dashed: true, dashSizeCss: 6, gapSizeCss: 4 },
      { dashSize: 0.3, color: 0x112233 },
    );
    expect(mat.dashed).toBe(true);
    expect(mat.dashSize).toBe(0.3); // caller already converted to world units
    expect(mat.gapSize).toBe(4);
    expect(mat.color.getHex()).toBe(0x112233);
    mat.dispose();
  });

  it("applyScreenLineStyle re-applies a width to a live material", () => {
    const mat = new LineMaterial({ linewidth: 99 });
    applyScreenLineStyle(mat, { widthCss: LINE_WIDTHS_CSS.selectionHalo });
    expect(mat.linewidth).toBe(3.0);
    mat.dispose();
  });

  it("setLineResolutionCss writes the logical size verbatim", () => {
    vi.stubGlobal("devicePixelRatio", 2);
    const mat = new LineMaterial();
    setLineResolutionCss(mat, 800, 600);
    expect([mat.resolution.x, mat.resolution.y]).toEqual([800, 600]);
    mat.dispose();
  });
});

describe("TEST-LINE-01 — line2PickThresholdCss", () => {
  it("cancels the drawn width so the radius is the authored CSS radius", () => {
    expect(line2PickThresholdCss(6, 1.25)).toBe(10.75);
    // (linewidth + threshold) / 2 is the radius three actually applies.
    expect((1.25 + line2PickThresholdCss(6, 1.25)) / 2).toBe(6);
    expect((4 + line2PickThresholdCss(6, 4)) / 2).toBe(6); // a fatter line does not pick wider
  });

  it("clamps at zero — a line wider than the tolerance picks at its own width", () => {
    expect(line2PickThresholdCss(6, 12)).toBe(0);
    expect(line2PickThresholdCss(6, 40)).toBe(0);
  });

  it("has no DPR term at all: the same two CSS numbers give the same answer", () => {
    for (const dpr of [1, 1.5, 2, 3]) {
      vi.stubGlobal("devicePixelRatio", dpr);
      expect(line2PickThresholdCss(6, 1.25)).toBe(10.75);
      vi.unstubAllGlobals();
    }
  });
});
