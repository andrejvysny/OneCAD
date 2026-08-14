import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { FeatureMeta } from "@/stores/documentStore";
import {
  canBindFeatureValue,
  canEditFeatureValue,
  commitFeatureValue,
  featureValueField,
} from "./featureValueEdit";
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
    applyEditCommand.mockResolvedValue({
      revision: 8,
      changedBodies: [],
      removedBodies: [],
      features: [],
      terminal: "noop",
    });

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

// ── WP-VE.2: binding a dimension to a document variable ──────────────────────

describe("canBindFeatureValue", () => {
  it("offers binding on every inline-editable op EXCEPT Hole", () => {
    for (const t of ["Extrude", "Revolve", "Fillet", "Chamfer", "Shell", "OffsetFace"]) {
      expect(canBindFeatureValue(t)).toBe(true);
    }
  });

  /*
   * A hole's own re-edit does NOT use the merge-patch lane: `commitHole`
   * rebuilds every `HoleParams` scalar from its FSM and wholesale-replaces the
   * op, so editing a hole's DEPTH would silently drop a binding on its
   * DIAMETER — a field the user never touched. Offering an affordance that an
   * ordinary follow-up gesture throws away is worse than not offering it.
   */
  it("does NOT offer binding on Hole, whose tool re-edit would discard it", () => {
    expect(featureValueField("Hole")).toBe("diameter");
    expect(canBindFeatureValue("Hole")).toBe(false);
  });

  it("offers nothing for an op with no editable dimension", () => {
    for (const t of ["Sketch", "Boolean", "TransformBody", undefined]) {
      expect(canBindFeatureValue(t)).toBe(false);
    }
  });
});

describe("commitFeatureValue — variable binding (WP-VE.2)", () => {
  const getOperationParams = vi.fn();
  const applyEditCommand = vi.fn();
  let spy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    getOperationParams.mockReset();
    applyEditCommand.mockReset();
    // `terminal` is what the verdict reads (`types.ts`: no production result may
    // omit it). A binding edit whose regen published no body is a `noop`, NOT a
    // failure — without the stamp the body-count fallback would call it one.
    applyEditCommand.mockResolvedValue({
      revision: 2,
      changedBodies: [],
      removedBodies: [],
      features: [],
      terminal: "noop",
    });
    spy = vi
      .spyOn(clientModule, "createClient")
      .mockReturnValue({ getOperationParams, applyEditCommand } as unknown as clientModule.CadClient);
  });
  afterEach(() => spy.mockRestore());

  /** The patched scalar carries BOTH: `expr` binds it, `value` is the cache. */
  it("sends {value, expr} when binding, leaving sibling params untouched", async () => {
    getOperationParams.mockResolvedValue({
      distance: { value: 25 },
      draftAngleDeg: { value: 0 },
      profile: { sketchId: "s1", regionId: "r0" },
    });

    expect(await commitFeatureValue("f2", "Extrude", 25, "height")).toBe(true);
    expect(applyEditCommand.mock.calls[0][0].op.params).toEqual({
      distance: { value: 25, expr: "height" },
      draftAngleDeg: { value: 0 },
      profile: { sketchId: "s1", regionId: "r0" },
    });
  });

  /** Unbinding is the ABSENCE of `expr` — the backend replaces the whole op. */
  it("omits expr entirely when clearing a binding", async () => {
    getOperationParams.mockResolvedValue({ distance: { value: 25, expr: "height" } });

    expect(await commitFeatureValue("f2", "Extrude", 40, null)).toBe(true);
    expect(applyEditCommand.mock.calls[0][0].op.params.distance).toEqual({ value: 40 });
  });

  /*
   * The HONESTY GATE. A plain numeric edit must not silently carry a stale
   * binding forward: it would leave the field showing `=height` while the number
   * the user typed is what regen overwrites on the next pass.
   */
  it("a plain numeric edit over a bound field clears the binding", async () => {
    getOperationParams.mockResolvedValue({ distance: { value: 25, expr: "height" } });

    expect(await commitFeatureValue("f2", "Extrude", 40)).toBe(true);
    expect(applyEditCommand.mock.calls[0][0].op.params.distance).toEqual({ value: 40 });
  });

  it("refuses to bind an op whose re-edit lane cannot keep the binding", async () => {
    getOperationParams.mockResolvedValue({ diameter: { value: 6 } });

    expect(await commitFeatureValue("f9", "Hole", 6, "boltD")).toBe(false);
    expect(applyEditCommand).not.toHaveBeenCalled();
  });
});
