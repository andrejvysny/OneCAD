/*
 * MeasurePanel — the docked measure reading (WP-C1).
 *
 * `createClient()` returns the shared `mockClient` under vitest (no Tauri
 * bridge), so `vi.spyOn(mockClient, "massProperties")` controls the fetch the
 * panel's own effect performs — the same seam `fileActions.test.ts` uses.
 *
 * The label formatters are exercised as PURE functions where possible; only the
 * async body-fetch behaviour needs a render.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { mockClient } from "@/ipc/mockClient";
import { measureStore } from "@/stores/measureStore";
import { measureAdd, measureInit, measureSummary } from "@/tools/modelTools/measureTool";
import type { MeasurePick } from "@/tools/modelTools/measureTool";
import type { MassProperties } from "@/ipc/types";
import {
  MeasurePanel,
  formatPoint,
  formatVolume,
  measuredBodyId,
  planePairLabel,
} from "./MeasurePanel";

function facePick(
  id: string,
  center: [number, number, number],
  normal: [number, number, number],
  bodyId = "body1",
): MeasurePick {
  return {
    bodyId,
    elementId: id,
    kind: "face",
    magnitude: 4800,
    center,
    curveType: -1,
    surfaceType: 0,
    normal,
    hasNormal: true,
  };
}

function summaryFor(a: MeasurePick, b: MeasurePick) {
  return measureSummary(measureAdd(measureAdd(measureInit(), a), b))!;
}

const MASS: MassProperties = {
  bodyId: "body1",
  volume: 144000,
  surfaceArea: 18000,
  centroid: [0, 0, 0],
  principalMoments: [54e6, 87.6e6, 120e6],
  principalAxes: [
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
  ],
};

beforeEach(() => measureStore.getState().clear());
afterEach(() => vi.restoreAllMocks());

describe("measuredBodyId", () => {
  it("is null with no picks and the NEWEST pick's body otherwise", () => {
    expect(measuredBodyId([])).toBeNull();
    expect(measuredBodyId([{ bodyId: "a" }, { bodyId: "b" }])).toBe("b");
  });
});

describe("formatVolume", () => {
  it("renders mm³ unchanged", () => {
    expect(formatVolume(144000, "mm")).toBe("144000 mm³");
  });

  it("converts by mmPer CUBED, not once", () => {
    // 1 cm³ = 1000 mm³. Dividing by 10 once would read "100 cm³" — the exact bug
    // class `formatArea` exists to prevent, one power up.
    expect(formatVolume(1000, "cm")).toBe("1 cm³");
    expect(formatVolume(1_000_000, "m")).toBe("0.001 m³");
    // 1 in³ = 25.4³ = 16387.064 mm³.
    expect(formatVolume(25.4 ** 3, "in")).toBe("1 in³");
  });

  it("passes a non-finite value through rather than rendering NaN arithmetic", () => {
    expect(formatVolume(Number.NaN, "mm")).toBe("NaN");
  });
});

describe("formatPoint", () => {
  it("formats each ordinate through the shared length formatter", () => {
    expect(formatPoint([1.5, -3, 0])).toBe("1.5, -3, 0");
  });

  it("strips trailing zeros without eating the integer part", () => {
    // The `100` → `1` regression `units/format` guards, reached through here.
    expect(formatPoint([100, 12.3456, 0.5])).toBe("100, 12.346, 0.5");
  });
});

describe("planePairLabel", () => {
  it("shows BOTH supplementary forms for an oblique pair", () => {
    const n: [number, number, number] = [Math.cos(Math.PI / 3), 0, Math.sin(Math.PI / 3)];
    const s = summaryFor(facePick("a", [0, 0, 0], [1, 0, 0]), facePick("b", [1, 0, 0], n));
    expect(planePairLabel(s)).toBe("Angle: 60° / 120°");
  });

  it("collapses to one form at exactly 90°", () => {
    const s = summaryFor(facePick("a", [0, 0, 0], [0, 0, 1]), facePick("b", [5, 0, 0], [1, 0, 0]));
    expect(planePairLabel(s)).toBe("Angle: 90°");
  });

  it("reports a PARALLEL pair as an offset, never as a 0° angle", () => {
    const s = summaryFor(
      facePick("a", [0, 0, 0], [0, 0, 1]),
      facePick("b", [0, 0, 30], [0, 0, 1]),
    );
    expect(planePairLabel(s)).toBe("Parallel planes, 30 mm apart");
  });

  it("is null when the two picks are not both planes", () => {
    const curved = { ...facePick("cyl", [5, 0, 0], [1, 0, 0]), surfaceType: 1 };
    expect(planePairLabel(summaryFor(facePick("a", [0, 0, 0], [0, 0, 1]), curved))).toBeNull();
  });
});

describe("MeasurePanel rendering", () => {
  it("renders nothing until something is picked", () => {
    render(<MeasurePanel />);
    expect(screen.queryByTestId("measure-panel")).toBeNull();
  });

  it("fetches and shows the picked body's mass properties", async () => {
    const spy = vi.spyOn(mockClient, "massProperties").mockResolvedValue(MASS);
    measureStore.getState().set([facePick("f4", [0, 0, 15], [0, 0, 1])], null);

    render(<MeasurePanel />);

    expect(await screen.findByTestId("measure-volume")).toHaveTextContent("144000 mm³");
    expect(screen.getByTestId("measure-surface-area")).toHaveTextContent("18000 mm²");
    expect(screen.getByTestId("measure-centroid")).toHaveTextContent("0, 0, 0");
    expect(spy).toHaveBeenCalledWith("body1");
  });

  it("shows the angle line once two planes are picked", async () => {
    vi.spyOn(mockClient, "massProperties").mockResolvedValue(MASS);
    const a = facePick("f4", [0, 0, 15], [0, 0, 1]);
    const b = facePick("f0", [40, 0, 0], [1, 0, 0]);
    measureStore.getState().set([a, b], summaryFor(a, b));

    render(<MeasurePanel />);

    expect(await screen.findByTestId("measure-plane-pair")).toHaveTextContent("Angle: 90°");
    expect(screen.getByTestId("measure-panel-distance")).toHaveTextContent("mm");
  });

  it("does NOT caption a stale reading onto a different body", async () => {
    // A response that arrives for body1 must not be shown while body2 is measured.
    vi.spyOn(mockClient, "massProperties").mockRejectedValue(new Error("no worker"));
    measureStore.getState().set([facePick("f4", [0, 0, 15], [0, 0, 1], "body2")], null);
    measureStore.getState().setMass(MASS); // MASS.bodyId === "body1"

    render(<MeasurePanel />);

    expect(screen.getByTestId("measure-panel")).toBeInTheDocument();
    expect(screen.queryByTestId("measure-volume")).toBeNull();
  });

  it("a failed fetch leaves the card without a mass section, never a fake zero", async () => {
    vi.spyOn(mockClient, "massProperties").mockRejectedValue(new Error("REF_UNRESOLVED"));
    measureStore.getState().set([facePick("f4", [0, 0, 15], [0, 0, 1])], null);

    render(<MeasurePanel />);

    await Promise.resolve();
    expect(screen.queryByTestId("measure-volume")).toBeNull();
    expect(measureStore.getState().mass).toBeNull();
  });
});
