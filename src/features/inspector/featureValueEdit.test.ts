import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { FeatureMeta } from "@/stores/documentStore";
import { canEditFeatureValue, commitFeatureValue, featureValueField } from "./featureValueEdit";
import * as clientModule from "@/ipc/client";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";

function feature(over: Partial<FeatureMeta> = {}): FeatureMeta {
  return {
    id: "f2",
    kind: "extrude",
    opType: "Extrude",
    label: "Extrude",
    valueText: "25.0 mm",
    primaryValue: 25,
    primaryValueKind: "length",
    status: "ok",
    ...over,
  };
}

describe("featureValueField — the opType → wire params key table", () => {
  it("maps every op whose row carries one editable dimension", () => {
    expect(featureValueField("Extrude")).toBe("distance");
    expect(featureValueField("Revolve")).toBe("angleDeg");
    expect(featureValueField("Fillet")).toBe("radius");
    // A chamfer edits d1 only — `distance2` rides the stored params untouched.
    expect(featureValueField("Chamfer")).toBe("radius");
    expect(featureValueField("Shell")).toBe("thickness");
    expect(featureValueField("Hole")).toBe("diameter");
  });

  it("has no field for an op with no single primary dimension", () => {
    // MIRRORS dto.rs `feature_value`: these arms mint no `primary`, so a row for
    // one of them must not offer an editor it could never commit.
    for (const t of [
      "Sketch",
      "Boolean",
      "TransformBody",
      "LinearPattern",
      "CircularPattern",
      "MirrorBody",
      "ImportStep",
      undefined,
      "",
    ]) {
      expect(featureValueField(t)).toBeNull();
    }
  });
});

describe("canEditFeatureValue — the three guards", () => {
  it("allows an applied, unsuppressed, dimensioned row while Select is active", () => {
    expect(canEditFeatureValue(feature(), 0, 3, "select")).toBe(true);
  });

  it("GUARD 1 — refuses while any model tool is armed", () => {
    for (const tool of ["extrude", "fillet", "hole", "measure", "transform"] as const) {
      expect(canEditFeatureValue(feature(), 0, 3, tool)).toBe(false);
    }
  });

  it("GUARD 2 — refuses a row at or beyond the rollback cursor", () => {
    // appliedOps = 2 ⇒ rows 0..1 are applied; row 2 is a draft past the bar.
    expect(canEditFeatureValue(feature(), 1, 2, "select")).toBe(true);
    expect(canEditFeatureValue(feature(), 2, 2, "select")).toBe(false);
    expect(canEditFeatureValue(feature(), 5, 2, "select")).toBe(false);
  });

  it("GUARD 3 — a suppressed row is read-only", () => {
    expect(canEditFeatureValue(feature({ suppressed: true }), 0, 3, "select")).toBe(false);
  });

  it("refuses a row the projection gave no primary value (Boolean/Sketch/pattern)", () => {
    expect(canEditFeatureValue(feature({ primaryValue: undefined }), 0, 3, "select")).toBe(false);
    // …and one whose opType this module cannot address, even WITH a value.
    expect(
      canEditFeatureValue(feature({ opType: "LinearPattern" }), 0, 3, "select"),
    ).toBe(false);
  });
});

describe("commitFeatureValue", () => {
  const getOperationParams = vi.fn();
  const applyEditCommand = vi.fn();
  let spy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    getOperationParams.mockReset();
    applyEditCommand.mockReset();
    spy = vi
      .spyOn(clientModule, "createClient")
      .mockReturnValue({ getOperationParams, applyEditCommand } as unknown as clientModule.CadClient);
  });
  afterEach(() => spy.mockRestore());

  it("PATCHES only the primary field, preserving every other stored param", async () => {
    // A revolve's picked axis + target body are NOT in the projection, so a
    // whole-params replace would silently drop them (the bug the merge exists for).
    getOperationParams.mockResolvedValue({
      angleDeg: { value: 90 },
      axis: { kind: "sketchLine", sketchId: "s1", lineId: "e2" },
      targetBodyId: "body1",
      profile: { sketchId: "s1", regionId: "r0" },
    });
    applyEditCommand.mockResolvedValue({ revision: 8, changedBodies: [], removedBodies: [], features: [] });

    expect(await commitFeatureValue("f7", "Revolve", 45)).toBe(true);
    expect(getOperationParams).toHaveBeenCalledWith("f7");
    expect(applyEditCommand).toHaveBeenCalledWith({
      cmd: "updateOperationParams",
      record: "f7",
      op: {
        opType: "Revolve",
        params: {
          angleDeg: { value: 45 },
          axis: { kind: "sketchLine", sketchId: "s1", lineId: "e2" },
          targetBodyId: "body1",
          profile: { sketchId: "s1", regionId: "r0" },
        },
      },
    });
  });

  it("hydrates the document store from the regen result", async () => {
    getOperationParams.mockResolvedValue({ distance: { value: 25 } });
    applyEditCommand.mockResolvedValue({
      revision: 11,
      changedBodies: [{ bodyId: "b9", lod: "med" }],
      removedBodies: [],
      features: [
        { id: "f2", kind: "extrude", opType: "Extrude", label: "Extrude", valueText: "40.0 mm", primaryValue: 40, primaryValueKind: "length", status: "ok" },
      ],
    });

    await commitFeatureValue("f2", "Extrude", 40);
    const doc = documentStore.getState();
    expect(doc.revision).toBe(11);
    expect(doc.features[0].primaryValue).toBe(40);
    expect(doc.features[0].valueText).toBe("40.0 mm");
    expect(doc.bodies.b9).toBeDefined();
  });

  it("refuses an op it has no field for, without touching the backend", async () => {
    expect(await commitFeatureValue("f1", "Sketch", 5)).toBe(false);
    expect(getOperationParams).not.toHaveBeenCalled();
    expect(applyEditCommand).not.toHaveBeenCalled();
  });

  it("surfaces a backend rejection as a sticky error hint and reports failure", async () => {
    getOperationParams.mockRejectedValue(new Error("no params for record f4"));
    expect(await commitFeatureValue("f4", "Extrude", 12)).toBe(false);
    expect(viewportStore.getState().statusHint).toMatchObject({
      message: expect.stringContaining("Edit failed: no params for record f4"),
      severity: "error",
      sticky: true,
    });
  });
});
