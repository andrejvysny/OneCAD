/*
 * `pickLabel` (WP-U11) — the per-pick chip text, exercised as a pure function.
 *
 * A rendered `<MeasureOverlay>` needs a live `ViewportEngine` bridge to mount
 * its chip host, so the three label shapes are pinned directly against the
 * function instead: the ordinary face/edge readings, unchanged, plus the new
 * Ø / R lead-ins a circle edge or cylindrical face's `classifyElement` radius
 * adds ahead of them.
 */
import { createElement } from "react";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, it, expect, vi } from "vitest";
import { MeasureOverlay, pickLabel } from "./MeasureOverlay";
import type { MeasurePick } from "@/tools/modelTools/measureTool";
import { measureStore } from "@/stores/measureStore";
import { settingsStore } from "@/stores/settingsStore";
import { measurementAnnotationStore } from "@/stores/measurementAnnotationStore";
import type { ChipPlacement } from "@/viewport/engine/ViewportEngine";

type MountChip = (id: string, host: HTMLElement, world: [number, number, number], placement?: ChipPlacement) => void;
const overlayEngine = vi.hoisted(() => ({
  mountChip: vi.fn<MountChip>((_id, host) => document.body.append(host)),
  unmountChip: vi.fn((_id: string, host: HTMLElement) => host.remove()),
}));
vi.mock("@/viewport/engineBridge", () => ({
  useViewportEngine: () => overlayEngine,
}));

function pick(over: Partial<MeasurePick>): MeasurePick {
  return {
    bodyId: "body1",
    elementId: "el_1",
    kind: "face",
    magnitude: 800,
    center: [0, 0, 0],
    curveType: -1,
    surfaceType: 0,
    normal: [0, 0, 1],
    hasNormal: true,
    radius: null,
    ...over,
  };
}

describe("pickLabel", () => {
  it("a plain face reports its area", () => {
    expect(pickLabel(pick({ kind: "face", magnitude: 800 }))).toBe("Area 800 mm²");
  });

  it("a plain edge reports its arc length", () => {
    expect(pickLabel(pick({ kind: "edge", magnitude: 20.735 }))).toMatch(/^Length 20\.7\d* mm$/);
  });

  it("a circle edge leads with the diameter, then the arc length", () => {
    expect(pickLabel(pick({ kind: "edge", magnitude: 20.735, radius: 3.3 }))).toBe(
      "Ø 6.6 mm · Length 20.735 mm",
    );
  });

  it("a cylindrical face leads with the radius, then the area", () => {
    expect(pickLabel(pick({ kind: "face", magnitude: 2010.6, radius: 20 }))).toBe(
      "R 20 mm · Area 2010.6 mm²",
    );
  });

  it("a non-face/edge kind falls back to the bare value", () => {
    expect(pickLabel(pick({ kind: "vertex", magnitude: 12 }))).toBe("vertex 12");
  });
});

describe("MeasureOverlay unit preference", () => {
  afterEach(() => {
    cleanup();
    measureStore.getState().clear();
    measurementAnnotationStore.getState().reset();
    settingsStore.setState({ displayUnit: "mm" });
    overlayEngine.mountChip.mockClear();
    overlayEngine.unmountChip.mockClear();
  });

  it("updates an armed label when display units change", () => {
    settingsStore.setState({ displayUnit: "mm" });
    measureStore.getState().set([pick({ kind: "edge", magnitude: 25.4 })], null);
    render(createElement(MeasureOverlay));
    expect(document.body.textContent).toContain("25.4 mm");

    act(() => settingsStore.setState({ displayUnit: "in" }));
    expect(document.body.textContent).toContain("1 in");
  });

  it("resets only replaced annotation identities and rejects a late old-placement callback", () => {
    const first = pick({ kind: "face", elementId: "face-a" });
    measureStore.getState().set([first], null);
    render(createElement(MeasureOverlay));
    const oldPlacement = overlayEngine.mountChip.mock.calls[0][3]?.annotation?.onPlacementStatus;
    if (!oldPlacement) throw new Error("expected measurement annotation placement");
    expect(measurementAnnotationStore.getState().slots.a.identity).toBe("face:body1:face-a");

    act(() => measureStore.getState().set([pick({ kind: "edge", elementId: "edge-b" })], null));
    oldPlacement("no-space");

    expect(measurementAnnotationStore.getState().slots.a).toMatchObject({
      identity: "edge:body1:edge-b",
      placement: "unknown",
    });
  });

  it("clears annotation state when the measurement session clears", () => {
    measureStore.getState().set([pick({ elementId: "face-a" })], null);
    render(createElement(MeasureOverlay));
    expect(measurementAnnotationStore.getState().slots.a.identity).toBe("face:body1:face-a");

    act(() => measureStore.getState().clear());
    expect(measurementAnnotationStore.getState().slots.a.identity).toBeNull();
  });

  it("does not retain a prior session pin across overlay unmount and remount", () => {
    measureStore.getState().set([pick({ elementId: "face-a" })], null);
    const first = render(createElement(MeasureOverlay));
    act(() => {
      measurementAnnotationStore.getState().setLivePosition("a", { x: 20, y: 30 });
      measurementAnnotationStore.getState().pin("a");
    });
    expect(measurementAnnotationStore.getState().slots.a.pinnedScreenPosition).toEqual({ x: 20, y: 30 });

    first.unmount();
    render(createElement(MeasureOverlay));
    expect(measurementAnnotationStore.getState().slots.a).toMatchObject({
      identity: "face:body1:face-a",
      pinnedScreenPosition: null,
    });
  });
});
