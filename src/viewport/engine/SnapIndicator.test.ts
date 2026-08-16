/*
 * Snap indicator rendering (SNAP §10.1 / §10.2).
 *
 * Two things the old renderer got wrong and this pins:
 *   1. every snap KIND had the same dot, so the marker — the only feedback when
 *      hints are off — could not tell endpoint from midpoint;
 *   2. the guide dashes were WORLD-sized constants, so the cadence was a solid
 *      line at one zoom and a few enormous strokes at another.
 */
import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import type { SketchPlane } from "@/ipc/types";
import type { SnapDecision, SnapKind } from "@/tools/sketch/snapTypes";
import { GUIDE_DASH_CSS, GUIDE_GAP_CSS, glyphFor, SnapIndicator } from "./SnapIndicator";
import { RENDER_ORDER } from "./renderOrder";

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

function makeIndicator() {
  const interactionRoot = new THREE.Group();
  const overlayEl = document.createElement("div");
  document.body.appendChild(overlayEl);
  const overlay = {
    register: vi.fn(),
    unregister: vi.fn(),
    setWorldPos: vi.fn(),
  };
  const invalidate = vi.fn();
  const ind = new SnapIndicator({
    interactionRoot,
    overlay: overlay as never,
    overlayEl,
    invalidate,
  });
  ind.setPlane(PLANE);
  return { ind, interactionRoot, overlay, invalidate, overlayEl };
}

function decision(over: Partial<SnapDecision> = {}): SnapDecision {
  return {
    raw: { x: 5, y: 5 },
    point: { x: 5, y: 5 },
    accepted: [],
    rejected: [],
    primaryId: "x",
    primaryKind: "endpoint",
    label: "Endpoint",
    guides: [],
    snapped: true,
    traceId: 1,
    ...over,
  };
}

describe("marker glyphs", () => {
  it("gives every distinguishable snap kind its OWN silhouette", () => {
    const kinds: SnapKind[] = [
      "endpoint",
      "midpoint",
      "center",
      "origin",
      "quadrant",
      "intersection",
      "onCurve",
      "grid",
    ];
    const glyphs = kinds.map(glyphFor);
    expect(new Set(glyphs).size, `duplicate glyph in ${glyphs.join(",")}`).toBe(kinds.length);
  });

  it("collapses the GUIDE kinds onto one dot — the guide line is the feedback there", () => {
    expect(glyphFor("alignH")).toBe("dot");
    expect(glyphFor("alignV")).toBe("dot");
    expect(glyphFor("alignHV")).toBe("dot");
    expect(glyphFor("polar")).toBe("dot");
  });
});

describe("guide rendering", () => {
  it("draws ONE fat line per guide, at the snap-guide render tier", () => {
    const { ind, interactionRoot } = makeIndicator();
    ind.show(
      decision({
        primaryKind: "alignHV",
        guides: [
          { orientation: "vertical", value: 5, ref: { x: 5, y: 40 } },
          { orientation: "horizontal", value: 5, ref: { x: 40, y: 5 } },
        ],
      }),
      false,
    );
    const guideGroup = interactionRoot
      .getObjectByName("snapIndicator")
      ?.getObjectByName("snapGuides");
    expect(guideGroup?.children).toHaveLength(2);
    for (const c of guideGroup?.children ?? []) {
      expect(c.renderOrder).toBe(RENDER_ORDER.SNAP_GUIDES);
    }
    ind.dispose();
  });

  it("keeps the dash cadence CONSTANT in CSS px across zoom", () => {
    const { ind } = makeIndicator();
    const guides = [{ orientation: "vertical" as const, value: 5, ref: { x: 5, y: 40 } }];

    // Zoomed IN: 20 CSS px per plane unit.
    ind.update(1000, 800, 1, 20);
    ind.show(decision({ guides }), false);
    const near = ind.guideDashSizes()[0];
    // A 6px dash at 20px/unit is 0.3 world units.
    expect(near.dash).toBeCloseTo(GUIDE_DASH_CSS / 20, 9);
    expect(near.gap).toBeCloseTo(GUIDE_GAP_CSS / 20, 9);

    // Zoomed OUT: 0.5 CSS px per plane unit — 40x coarser in world terms…
    ind.update(1000, 800, 1, 0.5);
    ind.show(decision({ guides }), false);
    const far = ind.guideDashSizes()[0];
    expect(far.dash).toBeCloseTo(GUIDE_DASH_CSS / 0.5, 9);
    // …which is the SAME cadence on screen, which is the point.
    expect(far.dash * 0.5).toBeCloseTo(near.dash * 20, 9);
    ind.dispose();
  });

  it("re-sizes LIVE guides when the scale changes, without a rebuild", () => {
    // A zoom between two snap decisions must not leave the visible guide at
    // its old world dash: update() alone carries the new metric onto existing
    // materials. Before this, dash sizes were written only in setGuides(), so
    // whatever scale the LAST rebuild saw stayed frozen on screen — the
    // measured cause of the CI-only sketch-snap-rendering:118 failure, and a
    // transient §10.1 cadence violation during any zoom.
    const { ind } = makeIndicator();
    const guides = [{ orientation: "vertical" as const, value: 5, ref: { x: 5, y: 40 } }];
    ind.update(1000, 800, 1, 20);
    ind.show(decision({ guides }), false);
    expect(ind.guideDashSizes()[0].dash).toBeCloseTo(GUIDE_DASH_CSS / 20, 9);

    // No show() here — only the per-frame update with a new metric.
    ind.update(1000, 800, 1, 0.5);
    const live = ind.guideDashSizes()[0];
    expect(live.dash).toBeCloseTo(GUIDE_DASH_CSS / 0.5, 9);
    expect(live.gap).toBeCloseTo(GUIDE_GAP_CSS / 0.5, 9);
    ind.dispose();
  });

  it("converts the guide width for the CURRENT device pixel ratio", () => {
    const { ind } = makeIndicator();
    const guides = [{ orientation: "vertical" as const, value: 5, ref: { x: 5, y: 40 } }];
    ind.update(1000, 800, 1, 10);
    ind.show(decision({ guides }), false);
    const at1x = ind.guideDashSizes()[0].width;
    ind.update(2000, 1600, 2, 10);
    const at2x = ind.guideDashSizes()[0].width;
    // Device px, so 2x DPR is twice the number for the same CSS width.
    expect(at2x).toBeCloseTo(at1x * 2, 9);
    ind.dispose();
  });

  it("releases guide objects when fewer guides are shown", () => {
    const { ind, interactionRoot } = makeIndicator();
    ind.show(
      decision({
        guides: [
          { orientation: "vertical", value: 5, ref: { x: 5, y: 40 } },
          { orientation: "horizontal", value: 5, ref: { x: 40, y: 5 } },
        ],
      }),
      false,
    );
    ind.show(decision({ guides: [{ orientation: "vertical", value: 5, ref: { x: 5, y: 40 } }] }), false);
    const guideGroup = interactionRoot
      .getObjectByName("snapIndicator")
      ?.getObjectByName("snapGuides");
    expect(guideGroup?.children).toHaveLength(1);
    ind.dispose();
  });

  it("skips a DEGENERATE guide rather than drawing a zero-length dash", () => {
    const { ind, interactionRoot } = makeIndicator();
    // The cursor sits exactly ON the reference: the guide has no length.
    ind.show(
      decision({ point: { x: 5, y: 40 }, guides: [{ orientation: "vertical", value: 5, ref: { x: 5, y: 40 } }] }),
      false,
    );
    const guideGroup = interactionRoot
      .getObjectByName("snapIndicator")
      ?.getObjectByName("snapGuides");
    expect(guideGroup?.children).toHaveLength(0);
    ind.dispose();
  });
});

describe("hint chip", () => {
  it("shows the composed label when hints are on", () => {
    const { ind, overlayEl } = makeIndicator();
    ind.show(decision({ label: "Vertical · Horizontal" }), true);
    const chip = overlayEl.querySelector("[data-sketch-snap-hint]") as HTMLElement;
    expect(chip.textContent).toBe("Vertical · Horizontal");
    expect(chip.style.display).not.toBe("none");
    ind.dispose();
  });

  it("stays hidden when hints are off, while the marker still shows", () => {
    const { ind, overlayEl, interactionRoot } = makeIndicator();
    ind.show(decision(), false);
    const chip = overlayEl.querySelector("[data-sketch-snap-hint]") as HTMLElement;
    expect(chip.style.display).toBe("none");
    expect(interactionRoot.getObjectByName("snapIndicator")?.visible).toBe(true);
    ind.dispose();
  });

  it("hides everything for an unsnapped decision", () => {
    const { ind, interactionRoot } = makeIndicator();
    ind.show(decision(), false);
    ind.show(decision({ snapped: false }), false);
    expect(interactionRoot.getObjectByName("snapIndicator")?.visible).toBe(false);
    ind.dispose();
  });
});
