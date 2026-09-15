/*
 * FP-N5 (docs/qa/UX_REVIEW_2026-09-14.md N5): the measure readout must show the
 * body's display NAME, not its raw UUID — the same fact `ActiveToolTargets`
 * already reads off `documentStore.bodies`. The UUID stays only as a fallback
 * for a body the projection no longer has.
 */
import { describe, it, expect } from "vitest";
import type { BodyMeta } from "@/stores/documentStore";
import type { MeasurePick } from "@/tools/modelTools/measureTool";
import { bodyLabel, pickReadout } from "./measurePresentation";

const BODIES: Record<string, BodyMeta> = {
  "body_a1b2c3d4-uuid": { id: "body_a1b2c3d4-uuid", name: "Body 1", visible: true },
};

function facePick(bodyId: string): MeasurePick {
  return {
    bodyId,
    elementId: "f1",
    kind: "face",
    magnitude: 4800,
    center: [0, 0, 0],
    curveType: -1,
    surfaceType: 0,
    normal: [0, 0, 1],
    hasNormal: true,
    radius: null,
  };
}

describe("bodyLabel", () => {
  it("resolves the body's display name from the registry", () => {
    expect(bodyLabel("body_a1b2c3d4-uuid", BODIES)).toBe("Body 1");
  });

  it("falls back to the raw id when the body is gone from the projection", () => {
    expect(bodyLabel("body_ghost-uuid", BODIES)).toBe("body_ghost-uuid");
  });

  it("falls back to the raw id when no registry is supplied at all", () => {
    expect(bodyLabel("body_a1b2c3d4-uuid")).toBe("body_a1b2c3d4-uuid");
  });
});

describe("pickReadout body naming", () => {
  it("shows the resolved body name, not the UUID", () => {
    const readout = pickReadout(facePick("body_a1b2c3d4-uuid"), BODIES);
    expect(readout).toBe("face · Body Body 1 · Area 4800 mm²");
    expect(readout).not.toContain("body_a1b2c3d4-uuid");
  });
});
