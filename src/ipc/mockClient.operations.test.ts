import { describe, it, expect, beforeEach } from "vitest";
import { mockClient, resetMockDocument, resetMockSketches, setMockLatency } from "./mockClient";
import { updateScalarParamsCommand } from "./tauriCommandMap";
import type { FeatureRecord, OperationOp, SketchEntity } from "./types";

const CIRCLE: SketchEntity = { id: "e1", type: "Circle", center: [0, 0], radius: 10 };

/** Author + finish a one-circle sketch so an extrude has a region to consume. */
async function seedRegion(sketchId = "skA"): Promise<string> {
  await mockClient.enterSketch({ newOnPlane: "XY", sketchId });
  await mockClient.sketchUpsert(sketchId, [CIRCLE], []);
  const fin = await mockClient.finishSketch(sketchId);
  return fin.regions[0].regionId;
}

function extrudeOp(sketchId: string, regionId: string, distance: number): OperationOp {
  return { opType: "Extrude", sketchId, regionId, params: { distance } };
}

function revolveOp(sketchId: string, regionId: string, angleDeg: number, featureId?: string): OperationOp {
  return { opType: "Revolve", sketchId, regionId, featureId, params: { angleDeg, booleanMode: "NewBody" } };
}

describe("mockClient operations", () => {
  beforeEach(() => {
    setMockLatency(0);
    resetMockDocument();
    resetMockSketches();
  });

  it("applyOperation(Extrude) appends a feature + synthesizes a body", async () => {
    const regionId = await seedRegion();
    const res = await mockClient.applyOperation(extrudeOp("skA", regionId, 25));
    expect(res.changedBodies).toHaveLength(1);
    expect(res.features).toHaveLength(6); // 5 base + 1
    const feat = res.features.find((f) => f.kind === "extrude" && f.valueText === "25.0 mm");
    expect(feat).toBeTruthy();
    expect(res.opLabel).toBe("Extrude");

    // The synthesized body is fetchable as a real MESH1 blob.
    const bodyId = res.changedBodies[0].bodyId;
    const mesh = await mockClient.getBodyMesh(bodyId, "coarse");
    expect(mesh.byteLength).toBeGreaterThan(64);
  });

  it("repeated persisted-sketch region reads stay exact after plane caching", async () => {
    const first = await mockClient.getSketchRegions("sketch2");
    const second = await mockClient.getSketchRegions("sketch2");
    expect(first.regions.length).toBeGreaterThan(0);
    expect(second.regions.map((region) => region.regionId)).toEqual(
      first.regions.map((region) => region.regionId),
    );
    const session = await mockClient.beginPreview({
      opType: "Extrude",
      sketchId: "sketch2",
      regionId: second.regions[0].regionId,
      params: { distance: 5 },
    });
    await mockClient.endPreview(session.sessionId, false);
  });

  it("undo removes the body + feature; redo restores them", async () => {
    const regionId = await seedRegion();
    const applied = await mockClient.applyOperation(extrudeOp("skA", regionId, 12));
    const bodyId = applied.changedBodies[0].bodyId;

    const undone = await mockClient.undo();
    expect(undone.removedBodies).toContain(bodyId);
    expect(undone.features).toHaveLength(5);
    expect(undone.opLabel).toBe("Extrude");

    const redone = await mockClient.redo();
    expect(redone.changedBodies.map((b) => b.bodyId)).toContain(bodyId);
    expect(redone.features).toHaveLength(6);
  });

  it("undo on an empty stack is a no-op", async () => {
    const res = await mockClient.undo();
    expect(res.changedBodies).toHaveLength(0);
    expect(res.removedBodies).toHaveLength(0);
    expect(res.features).toHaveLength(5);
  });

  it("Extrude (Add) grows the target body's mesh, creates no new body, labels the feature", async () => {
    const regionId = await seedRegion();
    const target = (await mockClient.applyOperation(extrudeOp("skA", regionId, 20))).changedBodies[0].bodyId;
    const before = (await mockClient.getBodyMesh(target, "coarse")).byteLength;

    const res = await mockClient.applyOperation({
      opType: "Extrude",
      sketchId: "skA",
      regionId,
      params: { distance: 15, booleanMode: "Add", targetBodyId: target },
    });
    // Feature-mode Add: changed = [target], NO new body (the mock has no CSG).
    expect(res.changedBodies.map((b) => b.bodyId)).toEqual([target]);
    expect(res.features.some((f) => f.kind === "extrude" && f.label === "Extrude (Add)")).toBe(true);
    // Naive concat grows the target mesh.
    const after = (await mockClient.getBodyMesh(target, "coarse")).byteLength;
    expect(after).toBeGreaterThan(before);
  });

  it("Extrude (Cut) targets the body, geometry unchanged, feature labeled", async () => {
    const regionId = await seedRegion();
    const target = (await mockClient.applyOperation(extrudeOp("skA", regionId, 20))).changedBodies[0].bodyId;
    const before = (await mockClient.getBodyMesh(target, "coarse")).byteLength;

    const res = await mockClient.applyOperation({
      opType: "Extrude",
      sketchId: "skA",
      regionId,
      params: { distance: 15, booleanMode: "Cut", targetBodyId: target },
    });
    expect(res.changedBodies.map((b) => b.bodyId)).toEqual([target]);
    expect(res.features.some((f) => f.kind === "extrude" && f.label === "Extrude (Cut)")).toBe(true);
    // Cut is a geometry no-op in the mock (documented) — the target mesh is unchanged.
    const after = (await mockClient.getBodyMesh(target, "coarse")).byteLength;
    expect(after).toBe(before);
  });

  it("Revolve (Cut) targets the body, no new body, feature labeled", async () => {
    const regionId = await seedRegion();
    const target = (await mockClient.applyOperation(extrudeOp("skA", regionId, 20))).changedBodies[0].bodyId;
    const res = await mockClient.applyOperation({
      opType: "Revolve",
      sketchId: "skA",
      regionId,
      params: { angleDeg: 180, booleanMode: "Cut", targetBodyId: target },
    });
    expect(res.changedBodies.map((b) => b.bodyId)).toEqual([target]);
    expect(res.features.some((f) => f.kind === "revolve" && f.label === "Revolve (Cut)")).toBe(true);
  });

  it("Add against a missing/invisible target falls back to NewBody (no crash)", async () => {
    const regionId = await seedRegion();
    const res = await mockClient.applyOperation({
      opType: "Extrude",
      sketchId: "skA",
      regionId,
      params: { distance: 10, booleanMode: "Add", targetBodyId: "nope" },
    });
    // No such body → the mock treats it as a fresh NewBody extrude.
    expect(res.changedBodies).toHaveLength(1);
    expect(res.features.some((f) => f.kind === "extrude" && f.label === "Extrude")).toBe(true);
  });

  it("Boolean removes the tool body and keeps the target", async () => {
    const regionId = await seedRegion();
    const b1 = (await mockClient.applyOperation(extrudeOp("skA", regionId, 10))).changedBodies[0].bodyId;
    const b2 = (await mockClient.applyOperation(extrudeOp("skA", regionId, 10))).changedBodies[0].bodyId;

    const res = await mockClient.applyOperation({
      opType: "Boolean",
      inputs: [
        { primary: { bodyId: b1, kind: "body" } },
        { primary: { bodyId: b2, kind: "body" } },
      ],
      params: { operation: "Union", targetBodyId: b1, toolBodyId: b2 },
    });
    expect(res.removedBodies).toContain(b2);
    expect(res.changedBodies.map((b) => b.bodyId)).toContain(b1);
    expect(res.features.some((f) => f.kind === "boolean" && f.label === "Union")).toBe(true);
  });

  it("applyOperation(Revolve) appends a revolve feature + synthesizes a body", async () => {
    const regionId = await seedRegion();
    const res = await mockClient.applyOperation(revolveOp("skA", regionId, 270));
    expect(res.changedBodies).toHaveLength(1);
    const feat = res.features.find((f) => f.kind === "revolve");
    expect(feat).toBeTruthy();
    expect(feat!.valueText).toBe("270°");
    expect(res.opLabel).toBe("Revolve");
    const bodyId = res.changedBodies[0].bodyId;
    const mesh = await mockClient.getBodyMesh(bodyId, "coarse");
    expect(mesh.byteLength).toBeGreaterThan(64); // a real MESH1 revolve body
  });

  it("re-editing a Revolve (featureId) updates the angle + reuses the same body", async () => {
    const regionId = await seedRegion();
    const created = await mockClient.applyOperation(revolveOp("skA", regionId, 360));
    const featureId = created.features.find((f) => f.kind === "revolve")!.id;
    const bodyId = created.changedBodies[0].bodyId;

    const edited = await mockClient.applyOperation(revolveOp("skA", regionId, 90, featureId));
    expect(edited.changedBodies.map((b) => b.bodyId)).toContain(bodyId); // rebuilt in place
    const revFeatures = edited.features.filter((f) => f.kind === "revolve");
    expect(revFeatures).toHaveLength(1); // no new row — same feature updated
    expect(revFeatures[0].valueText).toBe("90°");
  });

  it("get_operation_params returns the stored wire params; a re-edit deep-merge keeps the axis (Findings 3+4)", async () => {
    const regionId = await seedRegion();
    // A revolve authored with a NON-DEFAULT axis (the user-picked sketch line).
    const created = await mockClient.applyOperation({
      opType: "Revolve",
      sketchId: "skA",
      regionId,
      params: {
        angleDeg: 360,
        axis: { kind: "sketchLine", sketchId: "skA", lineId: "line-7" },
        booleanMode: "NewBody",
      },
    });
    const featureId = created.features.find((f) => f.kind === "revolve")!.id;

    // On re-edit arm, the controller fetches the stored params (wire shape).
    const stored = await mockClient.getOperationParams(featureId);
    expect(stored.angleDeg).toEqual({ value: 360 });
    expect(stored.axis).toEqual({ kind: "sketchLine", sketchId: "skA", lineId: "line-7" });
    expect(stored.profile).toEqual({ sketchId: "skA", regionId });

    // The angle-only commit deep-merges: the axis + profile survive byte-for-byte.
    const cmd = updateScalarParamsCommand(featureId, "Revolve", stored, { angleDeg: { value: 90 } });
    if (cmd.cmd !== "updateOperationParams") throw new Error("unreachable");
    const p = cmd.op.params as unknown as Record<string, unknown>;
    expect(p.angleDeg).toEqual({ value: 90 }); // only the scalar changed
    expect(p.axis).toEqual({ kind: "sketchLine", sketchId: "skA", lineId: "line-7" }); // NOT clobbered
    expect(p.profile).toEqual({ sketchId: "skA", regionId });

    // Applying it updates the feature's value text without appending a row.
    const edited = await mockClient.applyEditCommand(cmd);
    const rev = edited.features.filter((f) => f.kind === "revolve");
    expect(rev).toHaveLength(1);
    expect(rev[0].valueText).toBe("90°");

    // get_operation_params on an unknown record rejects.
    await expect(mockClient.getOperationParams("no-such-record")).rejects.toThrow();
  });

  it("Fillet re-emits the target body + adds a radius feature (documented mock limit)", async () => {
    const res = await mockClient.applyOperation({
      opType: "Fillet",
      inputs: [{ primary: { bodyId: "body1", kind: "edge" } }],
      params: { mode: "Fillet", radius: 3, edgeIds: ["e:2"] },
    });
    expect(res.changedBodies.map((b) => b.bodyId)).toContain("body1");
    expect(res.features.some((f) => f.kind === "fillet" && f.valueText === "3.0 mm")).toBe(true);
  });

  it("Shell adds a thickness feature + re-emits the target body; a re-edit updates in place", async () => {
    const created = await mockClient.applyOperation({
      opType: "Shell",
      inputs: [{ primary: { bodyId: "body1", elementId: "el_f", kind: "face" } }],
      params: { thickness: 2, openFaces: ["el_f"], targetBodyId: "body1" },
    });
    expect(created.changedBodies.map((b) => b.bodyId)).toContain("body1");
    // Look up by `opType`, not `kind`: the mock mirrors the REAL projection, where
    // `dto.rs feature_kind` folds Shell into the `fillet` bucket.
    const shell = created.features.find((f) => f.opType === "Shell");
    expect(shell?.kind).toBe("fillet");
    expect(shell?.valueText).toBe("2.0 mm");
    expect(created.opLabel).toBe("Shell");

    const edited = await mockClient.applyOperation({
      opType: "Shell",
      featureId: shell!.id,
      inputs: [],
      params: { thickness: 4, openFaces: [], targetBodyId: "body1" },
    });
    const shells = edited.features.filter((f) => f.opType === "Shell");
    expect(shells).toHaveLength(1); // updated, not appended
    expect(shells[0].valueText).toBe("4.0 mm");
  });

  it("LinearPattern / CircularPattern add ×count features (documented mock limit)", async () => {
    const lin = await mockClient.applyOperation({
      opType: "LinearPattern",
      inputs: [{ primary: { bodyId: "body1", kind: "body" } }],
      params: { sourceBodyId: "body1", direction: [1, 0, 0], spacing: 20, count: 4, fuseResult: true },
    });
    expect(lin.changedBodies.map((b) => b.bodyId)).toContain("body1");
    // `opType` is the identity; `kind` is the folded projection bucket (boolean).
    expect(lin.features.some((f) => f.opType === "LinearPattern" && f.kind === "boolean" && f.valueText === "×4")).toBe(
      true,
    );
    expect(lin.opLabel).toBe("Linear Pattern");

    const circ = await mockClient.applyOperation({
      opType: "CircularPattern",
      inputs: [{ primary: { bodyId: "body1", kind: "body" } }],
      params: { sourceBodyId: "body1", axisOrigin: [0, 0, 0], axisDirection: [0, 0, 1], angleDeg: 360, count: 6 },
    });
    expect(
      circ.features.some((f) => f.opType === "CircularPattern" && f.kind === "boolean" && f.valueText === "×6"),
    ).toBe(true);
    expect(circ.opLabel).toBe("Circular Pattern");
  });

  it("MirrorBody adds a mirror feature labelled by the plane; undo removes it", async () => {
    const res = await mockClient.applyOperation({
      opType: "MirrorBody",
      inputs: [{ primary: { bodyId: "body1", kind: "body" } }],
      params: { sourceBodyId: "body1", planePoint: [0, 0, 0], planeNormal: [1, 0, 0], fuseWithOriginal: false },
    });
    const mirror = res.features.find((f) => f.opType === "MirrorBody");
    expect(mirror?.kind).toBe("boolean");
    expect(mirror?.valueText).toBe("YZ"); // normal +X → YZ plane
    expect(res.opLabel).toBe("Mirror");

    const undone = await mockClient.undo();
    expect(undone.features.some((f) => f.opType === "MirrorBody")).toBe(false);
  });

  it("preview session commits with the latest streamed params", async () => {
    const regionId = await seedRegion();
    const session = await mockClient.beginPreview({
      opType: "Extrude",
      sketchId: "skA",
      regionId,
      params: { distance: 10 },
    });
    mockClient.updatePreview(session.sessionId, { distance: 30 }, 1);
    const res = await mockClient.endPreview(session.sessionId, true);
    expect(res).not.toBeNull();
    expect(res!.features.some((f) => f.kind === "extrude" && f.valueText === "30.0 mm")).toBe(true);
  });

  it("updatePreview delivers an exact result carrying its epoch", async () => {
    const regionId = await seedRegion();
    const epochs: number[] = [];
    const unsub = mockClient.onPreviewResult((r) => epochs.push(r.epoch));
    const session = await mockClient.beginPreview({
      opType: "Extrude",
      sketchId: "skA",
      regionId,
      params: { distance: 10 },
    });
    mockClient.updatePreview(session.sessionId, { distance: 20 }, 7);
    await new Promise((r) => setTimeout(r, 5)); // let the (0ms) latency fire
    expect(epochs).toContain(7);
    unsub();
  });

  it("a preview result after the session ends is dropped", async () => {
    const regionId = await seedRegion();
    const seen: number[] = [];
    const unsub = mockClient.onPreviewResult((r) => seen.push(r.epoch));
    const session = await mockClient.beginPreview({
      opType: "Extrude",
      sketchId: "skA",
      regionId,
      params: { distance: 10 },
    });
    mockClient.updatePreview(session.sessionId, { distance: 20 }, 3);
    await mockClient.endPreview(session.sessionId, false); // cancel before the 0ms timer
    await new Promise((r) => setTimeout(r, 5));
    expect(seen).not.toContain(3);
    unsub();
  });
});

/*
 * Suppression + boolean op-swap through the RAW EditCommand surface. The frontend
 * has no suppression overlay any more (TRUST wave), so the mock's returned
 * `features[].suppressed` IS the dim state every spec asserts on.
 */
/** The newest timeline row of a result (the op just committed). */
function lastFeature(res: { features: FeatureRecord[] }): FeatureRecord {
  const last = res.features[res.features.length - 1];
  if (!last) throw new Error("result carries no features");
  return last;
}

describe("mockClient — suppression + boolean re-edit", () => {
  beforeEach(() => {
    setMockLatency(0);
    resetMockDocument();
    resetMockSketches();
  });

  it("flips `suppressed` on the named feature and round-trips through undo/redo", async () => {
    const regionId = await seedRegion();
    const created = await mockClient.applyOperation(extrudeOp("skA", regionId, 10));
    const featureId = lastFeature(created).id;

    const suppressed = await mockClient.applyEditCommand({
      cmd: "setOperationSuppression",
      record: featureId,
      suppressed: true,
      cascade: true,
    });
    expect(suppressed.features.find((f) => f.id === featureId)?.suppressed).toBe(true);
    expect(suppressed.opLabel).toBe("Suppress");

    // Every EditCommand is undoable on the real backend (core mints an inverse for
    // SetOperationSuppression too), so ⌘Z must restore the un-suppressed timeline.
    const undone = await mockClient.undo();
    expect(undone.features.find((f) => f.id === featureId)?.suppressed).toBeFalsy();
    const redone = await mockClient.redo();
    expect(redone.features.find((f) => f.id === featureId)?.suppressed).toBe(true);
  });

  it("cascades to a DERIVABLE downstream consumer of the suppressed feature's body", async () => {
    const regionId = await seedRegion();
    const created = await mockClient.applyOperation(extrudeOp("skA", regionId, 10));
    const extrudeId = lastFeature(created).id;
    const bodyId = created.changedBodies[0].bodyId;
    // A Shell whose stored params NAME that body — the one dependency edge the
    // mock can actually prove (it has no dependency graph).
    const shelled = await mockClient.applyOperation({
      opType: "Shell",
      inputs: [{ primary: { bodyId, elementId: "el_f", kind: "face" } }],
      params: { thickness: 2, openFaces: ["el_f"], targetBodyId: bodyId },
    });
    const shellId = lastFeature(shelled).id;

    const res = await mockClient.applyEditCommand({
      cmd: "setOperationSuppression",
      record: extrudeId,
      suppressed: true,
      cascade: true,
    });
    expect(res.features.find((f) => f.id === extrudeId)?.suppressed).toBe(true);
    expect(res.features.find((f) => f.id === shellId)?.suppressed).toBe(true);
    // An UNRELATED earlier feature is never dragged in (cascade is downstream-only).
    expect(res.features.find((f) => f.id === "f2")?.suppressed).toBeFalsy();
  });

  it("does NOT cascade on un-suppress (mirrors the one-directional backend policy)", async () => {
    const regionId = await seedRegion();
    const created = await mockClient.applyOperation(extrudeOp("skA", regionId, 10));
    const extrudeId = lastFeature(created).id;
    const bodyId = created.changedBodies[0].bodyId;
    const shelled = await mockClient.applyOperation({
      opType: "Shell",
      inputs: [{ primary: { bodyId, elementId: "el_f", kind: "face" } }],
      params: { thickness: 2, openFaces: ["el_f"], targetBodyId: bodyId },
    });
    const shellId = lastFeature(shelled).id;
    await mockClient.applyEditCommand({ cmd: "setOperationSuppression", record: extrudeId, suppressed: true, cascade: true });

    const res = await mockClient.applyEditCommand({
      cmd: "setOperationSuppression",
      record: extrudeId,
      suppressed: false,
      cascade: false,
    });
    expect(res.features.find((f) => f.id === extrudeId)?.suppressed).toBe(false);
    // The dependent stays suppressed — un-suppress must never resurrect it.
    expect(res.features.find((f) => f.id === shellId)?.suppressed).toBe(true);
  });

  it("a boolean operation swap edits the row in place, preserving the stored bodies", async () => {
    const created = await mockClient.applyOperation({
      opType: "Boolean",
      inputs: [
        { primary: { bodyId: "body1", kind: "body" } },
        { primary: { bodyId: "body9", kind: "body" } },
      ],
      params: { operation: "Union", targetBodyId: "body1", toolBodyId: "body9" },
    });
    const featureId = lastFeature(created).id;
    expect(lastFeature(created).label).toBe("Union");

    const stored = await mockClient.getOperationParams(featureId);
    const cmd = updateScalarParamsCommand(featureId, "Boolean", stored, { operation: "Cut" });
    const edited = await mockClient.applyEditCommand(cmd);

    // One row still (edit, not append) and the mock's boolean label follows the swap.
    expect(edited.features.filter((f) => f.opType === "Boolean")).toHaveLength(1);
    expect(lastFeature(edited).label).toBe("Cut");
    // The consumed bodies survived the merge verbatim.
    expect(await mockClient.getOperationParams(featureId)).toEqual({
      operation: "Cut",
      targetBodyId: "body1",
      toolBodyId: "body9",
    });
  });
});
