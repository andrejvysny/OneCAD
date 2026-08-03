/*
 * The mock lane's `TransformBody` (WP-B W1).
 *
 * This is the one mock body op that is EXACT rather than a stand-in — a rigid
 * transform of a tessellation is what the kernel produces too — so it is held to
 * a numeric standard the other mock ops are not: every position, every normal,
 * every edge point, the header bbox and the optional FACE_BBOXES section are
 * asserted against closed-form expectations. A wrong header bbox does not look
 * wrong on screen; it silently breaks zoom-to-fit and the picker's broad phase,
 * which is exactly the failure a "the body moved, ship it" test would miss.
 *
 * The second half pins the FOLD rule the mock mirrors from core
 * `document::transform::can_fold_transform`.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { mockClient, resetMockDocument, resetMockSketches, setMockLatency } from "./mockClient";
import { BOX_SIZE, encodeMesh1, makeBoxMesh, transformMesh1 } from "./mockMeshes";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { placementMatrix, WORLD_AXIS } from "@/tools/preview/patternPreview";
import type { FeatureRecord, OperationOp, SketchEntity, TransformBodyParams } from "./types";

const IDENTITY_ROTATION = { center: [0, 0, 0] as [number, number, number], axis: WORLD_AXIS.Z, angleDeg: 0 };

function moveOp(targets: string[], translate: [number, number, number], featureId?: string): OperationOp {
  return {
    opType: "TransformBody",
    featureId,
    inputs: targets.map((bodyId) => ({ primary: { bodyId, kind: "body" as const } })),
    params: { targets, translate, rotate: { ...IDENTITY_ROTATION }, copy: false },
  };
}

function rotateOp(targets: string[], angleDeg: number, center: [number, number, number] = [0, 0, 0]): OperationOp {
  return {
    opType: "TransformBody",
    inputs: targets.map((bodyId) => ({ primary: { bodyId, kind: "body" as const } })),
    params: {
      targets,
      translate: [0, 0, 0],
      rotate: { center, axis: WORLD_AXIS.Z, angleDeg },
      copy: false,
    },
  };
}

/** The last timeline row (the project targets ES2020 — no `Array.prototype.at`). */
const lastRow = (rows: FeatureRecord[]): FeatureRecord => rows[rows.length - 1];

const matrixOf = (p: TransformBodyParams) =>
  placementMatrix(p.translate, p.rotate.center, p.rotate.axis, p.rotate.angleDeg);

// ── transformMesh1 exactness ────────────────────────────────────────────────

describe("transformMesh1 — exact rigid rewrite of a MESH1 blob", () => {
  // The mock box is 80×60×30 centred at the origin, so its bbox is exactly
  // ±[40, 30, 15] — every expectation below is closed-form off that.
  const HALF: [number, number, number] = [BOX_SIZE[0] / 2, BOX_SIZE[1] / 2, BOX_SIZE[2] / 2];

  it("translates every position by exactly the offset", () => {
    const before = parseMeshPayload(makeBoxMesh());
    const after = parseMeshPayload(transformMesh1(makeBoxMesh(), matrixOf(moveOp(["b"], [30, 0, 0]).params as TransformBodyParams)));
    expect(after.vertexCount).toBe(before.vertexCount);
    for (let i = 0; i < before.positions.length; i += 3) {
      expect(after.positions[i]).toBeCloseTo(before.positions[i] + 30, 4);
      expect(after.positions[i + 1]).toBeCloseTo(before.positions[i + 1], 4);
      expect(after.positions[i + 2]).toBeCloseTo(before.positions[i + 2], 4);
    }
  });

  it("recomputes the HEADER bbox — a translation shifts it by the same offset", () => {
    const m = matrixOf(moveOp(["b"], [30, -12, 4]).params as TransformBodyParams);
    const after = parseMeshPayload(transformMesh1(makeBoxMesh(), m));
    expect([...after.bboxMin]).toEqual([-HALF[0] + 30, -HALF[1] - 12, -HALF[2] + 4]);
    expect([...after.bboxMax]).toEqual([HALF[0] + 30, HALF[1] - 12, HALF[2] + 4]);
  });

  it("recomputes the header bbox under ROTATION — 90° about Z swaps the X/Y extents", () => {
    const m = matrixOf(rotateOp(["b"], 90).params as TransformBodyParams);
    const after = parseMeshPayload(transformMesh1(makeBoxMesh(), m));
    // The 80-long X extent becomes the Y extent and vice versa; Z is untouched.
    expect(after.bboxMin[0]).toBeCloseTo(-HALF[1], 3);
    expect(after.bboxMax[0]).toBeCloseTo(HALF[1], 3);
    expect(after.bboxMin[1]).toBeCloseTo(-HALF[0], 3);
    expect(after.bboxMax[1]).toBeCloseTo(HALF[0], 3);
    expect(after.bboxMin[2]).toBeCloseTo(-HALF[2], 3);
  });

  it("rotates NORMALS by the rotation only — a pure translation leaves them identical", () => {
    const before = parseMeshPayload(makeBoxMesh());
    const moved = parseMeshPayload(transformMesh1(makeBoxMesh(), matrixOf(moveOp(["b"], [30, -12, 4]).params as TransformBodyParams)));
    expect([...moved.normals!]).toEqual([...before.normals!]);

    const spun = parseMeshPayload(transformMesh1(makeBoxMesh(), matrixOf(rotateOp(["b"], 90).params as TransformBodyParams)));
    // The box's first face is +X (BOX_FACES[0]); 90° about Z sends it to +Y.
    expect(spun.normals![0]).toBeCloseTo(0, 5);
    expect(spun.normals![1]).toBeCloseTo(1, 5);
    expect(spun.normals![2]).toBeCloseTo(0, 5);
    // …and every normal stays unit length (the linear part is a pure rotation).
    for (let i = 0; i < spun.normals!.length; i += 3) {
      expect(Math.hypot(spun.normals![i], spun.normals![i + 1], spun.normals![i + 2])).toBeCloseTo(1, 5);
    }
  });

  it("moves EDGE polyline points too — they are world points, not local ones", () => {
    const before = parseMeshPayload(makeBoxMesh());
    const after = parseMeshPayload(transformMesh1(makeBoxMesh(), matrixOf(moveOp(["b"], [30, 0, 0]).params as TransformBodyParams)));
    expect(after.edgeCount).toBe(before.edgeCount);
    expect(after.edgePointCount).toBe(before.edgePointCount);
    for (let i = 0; i < before.edgePositions!.length; i += 3) {
      expect(after.edgePositions![i]).toBeCloseTo(before.edgePositions![i] + 30, 4);
      expect(after.edgePositions![i + 1]).toBeCloseTo(before.edgePositions![i + 1], 4);
    }
  });

  it("recomputes FACE_BBOXES when the source carried them", () => {
    // The box writer emits no FACE_BBOXES, so build a source that does.
    const withFaceBboxes = encodeTwoFaceQuad();
    const before = parseMeshPayload(withFaceBboxes);
    expect(before.hasFaceBboxes).toBe(true);

    const after = parseMeshPayload(
      transformMesh1(withFaceBboxes, matrixOf(moveOp(["b"], [0, 0, 7]).params as TransformBodyParams)),
    );
    expect(after.hasFaceBboxes).toBe(true);
    expect(after.faceBboxes).not.toBeNull();
    expect(after.faceBboxes!.length).toBe(before.faceBboxes!.length);
    for (let i = 0; i < before.faceBboxes!.length; i++) {
      // Only the Z components (indices 2 and 5 of each 6-tuple) move.
      const dz = i % 6 === 2 || i % 6 === 5 ? 7 : 0;
      expect(after.faceBboxes![i]).toBeCloseTo(before.faceBboxes![i] + dz, 4);
    }
  });

  it("preserves topology: face/edge ids, triangle grouping and the id flags", () => {
    const src = makeBoxMesh();
    const before = parseMeshPayload(src);
    const after = parseMeshPayload(transformMesh1(src, matrixOf(rotateOp(["b"], 33).params as TransformBodyParams)));
    expect(after.faceCount).toBe(before.faceCount);
    expect(after.triangleCount).toBe(before.triangleCount);
    expect([...after.indices]).toEqual([...before.indices]);
    expect([...after.faceRanges]).toEqual([...before.faceRanges]);
    expect([...after.faceIdChars]).toEqual([...before.faceIdChars]);
    expect([...after.edgeIdChars!]).toEqual([...before.edgeIdChars!]);
    expect(after.idsHaveElementIds).toBe(before.idsHaveElementIds);
    expect(after.lod).toBe(before.lod);
  });

  it("carries authored FACE_COLORS through verbatim (appearance is not geometry)", () => {
    const colored = encodeMesh1({
      positions: [0, 0, 0, 10, 0, 0, 10, 5, 0, 0, 5, 0],
      faces: [
        { triangles: [[0, 1, 2]], id: "f:0", color: [200, 30, 40, 255] },
        // A face with NO authored color stays `{0,0,0,0}` — "unset", not black.
        { triangles: [[0, 2, 3]], id: "f:1" },
      ],
    });
    const before = parseMeshPayload(colored);
    const after = parseMeshPayload(
      transformMesh1(colored, matrixOf(rotateOp(["b"], 90).params as TransformBodyParams)),
    );
    expect(after.hasFaceColors).toBe(true);
    expect([...after.faceColors!]).toEqual([...before.faceColors!]);
  });

  it("an identity placement is byte-stable in geometry (a legal no-op)", () => {
    const before = parseMeshPayload(makeBoxMesh());
    const after = parseMeshPayload(transformMesh1(makeBoxMesh(), matrixOf(moveOp(["b"], [0, 0, 0]).params as TransformBodyParams)));
    expect([...after.positions]).toEqual([...before.positions]);
    expect([...after.bboxMin]).toEqual([...before.bboxMin]);
  });
});

/** A two-face shape that DOES emit FACE_BBOXES (the box writer deliberately does not). */
function encodeTwoFaceQuad(): ArrayBuffer {
  return encodeMesh1({
    positions: [0, 0, 0, 10, 0, 0, 10, 5, 0, 0, 5, 0, 0, 0, 2, 10, 0, 2],
    faces: [
      { triangles: [[0, 1, 2], [0, 2, 3]], id: "f:0" },
      { triangles: [[0, 1, 4], [1, 5, 4]], id: "f:1" },
    ],
    faceBboxes: true,
  });
}

// ── The client lane: fold, re-edit, copies, undo ────────────────────────────

const CIRCLE: SketchEntity = { id: "e1", type: "Circle", center: [0, 0], radius: 10 };

async function seedExtrudedBody(sketchId = "skT"): Promise<string> {
  await mockClient.enterSketch({ newOnPlane: "XY", sketchId });
  await mockClient.sketchUpsert(sketchId, [CIRCLE], []);
  const fin = await mockClient.finishSketch(sketchId);
  const res = await mockClient.applyOperation({
    opType: "Extrude",
    sketchId,
    regionId: fin.regions[0].regionId,
    params: { distance: 10 },
  });
  return res.changedBodies[0].bodyId;
}

async function bboxOf(bodyId: string): Promise<{ min: readonly number[]; max: readonly number[] }> {
  const view = parseMeshPayload(await mockClient.getBodyMesh(bodyId, "coarse"));
  return { min: view.bboxMin, max: view.bboxMax };
}

describe("mock TransformBody — the client lane", () => {
  beforeEach(() => {
    setMockLatency(0);
    resetMockDocument();
    resetMockSketches();
  });

  it("appends one Move row and actually moves the body's mesh", async () => {
    const before = await bboxOf("body1");
    const res = await mockClient.applyOperation(moveOp(["body1"], [30, 0, 0]));

    const row = lastRow(res.features);
    expect(row.opType).toBe("TransformBody");
    expect(row.label).toBe("Move");
    expect(row.kind).toBe("boolean"); // the real projection bucket (dto.rs feature_kind)
    expect(row.valueText).toBe("Δ(30.0, 0.0, 0.0)");
    expect(res.changedBodies.map((b) => b.bodyId)).toEqual(["body1"]);

    const after = await bboxOf("body1");
    expect(after.min[0]).toBeCloseTo(before.min[0] + 30, 4);
    expect(after.max[0]).toBeCloseTo(before.max[0] + 30, 4);
  });

  it("a rotation row reads as degrees", async () => {
    const res = await mockClient.applyOperation(rotateOp(["body1"], 90));
    expect(lastRow(res.features).valueText).toBe("90.0°");
  });

  it("an identity placement still reads as an editable value, not a blank row", async () => {
    const res = await mockClient.applyOperation(moveOp(["body1"], [0, 0, 0]));
    expect(lastRow(res.features).valueText).toBe("0.0 mm");
  });

  it("a RE-EDIT is absolute, not cumulative — 30 then 50 lands at 50", async () => {
    const origin = await bboxOf("body1");
    const first = await mockClient.applyOperation(moveOp(["body1"], [30, 0, 0]));
    const recordId = lastRow(first.features).id;

    const second = await mockClient.applyOperation(moveOp(["body1"], [50, 0, 0], recordId));
    // Still ONE row for the placement (5 base features + 1).
    expect(second.features).toHaveLength(6);
    expect(lastRow(second.features).valueText).toBe("Δ(50.0, 0.0, 0.0)");

    const after = await bboxOf("body1");
    expect(after.min[0]).toBeCloseTo(origin.min[0] + 50, 4); // NOT +80
  });

  it("moves a MULTI-body selection with one record", async () => {
    const extruded = await seedExtrudedBody();
    const beforeA = await bboxOf("body1");
    const beforeB = await bboxOf(extruded);

    const res = await mockClient.applyOperation(moveOp(["body1", extruded], [0, 25, 0]));
    expect(res.features.filter((f) => f.opType === "TransformBody")).toHaveLength(1);
    expect(res.changedBodies.map((b) => b.bodyId).sort()).toEqual(["body1", extruded].sort());

    expect((await bboxOf("body1")).min[1]).toBeCloseTo(beforeA.min[1] + 25, 4);
    expect((await bboxOf(extruded)).min[1]).toBeCloseTo(beforeB.min[1] + 25, 4);
  });

  it("copy: true preserves the source and mints a new body, id-stable across re-edits", async () => {
    const before = await bboxOf("body1");
    const copyOp = (translate: [number, number, number], featureId?: string): OperationOp => ({
      opType: "TransformBody",
      featureId,
      params: { targets: ["body1"], translate, rotate: { ...IDENTITY_ROTATION }, copy: true },
    });

    const first = await mockClient.applyOperation(copyOp([100, 0, 0]));
    const copyId = first.changedBodies[0].bodyId;
    expect(copyId).not.toBe("body1");
    // The SOURCE is untouched…
    expect((await bboxOf("body1")).min[0]).toBeCloseTo(before.min[0], 4);
    // …and the copy sits at the offset.
    expect((await bboxOf(copyId)).min[0]).toBeCloseTo(before.min[0] + 100, 4);

    const recordId = lastRow(first.features).id;
    const second = await mockClient.applyOperation(copyOp([200, 0, 0], recordId));
    expect(second.changedBodies.map((b) => b.bodyId)).toEqual([copyId]); // same copy, rewritten
    expect((await bboxOf(copyId)).min[0]).toBeCloseTo(before.min[0] + 200, 4);
  });

  it("undo restores the body's original position without deleting it", async () => {
    const before = await bboxOf("body1");
    await mockClient.applyOperation(moveOp(["body1"], [30, 0, 0]));

    const undone = await mockClient.undo();
    // The seed body is CHANGED back, never reported as removed (a removal would
    // drop it from the scene instead of restoring it).
    expect(undone.removedBodies).toEqual([]);
    expect(undone.changedBodies.map((b) => b.bodyId)).toEqual(["body1"]);
    expect(undone.features.some((f) => f.opType === "TransformBody")).toBe(false);
    expect((await bboxOf("body1")).min[0]).toBeCloseTo(before.min[0], 4);
  });

  it("getOperationParams serves the stored placement back verbatim for a re-edit", async () => {
    const res = await mockClient.applyOperation(rotateOp(["body1"], 90, [5, 6, 7]));
    const stored = await mockClient.getOperationParams(lastRow(res.features).id);
    expect(stored.targets).toEqual(["body1"]);
    expect(stored.rotate).toEqual({ center: [5, 6, 7], axis: [0, 0, 1], angleDeg: { value: 90 } });
    expect(stored.copy).toBe(false);
  });
});

describe("mock canFoldTransform — the lineage rule", () => {
  beforeEach(() => {
    setMockLatency(0);
    resetMockDocument();
    resetMockSketches();
  });

  it("is null for a body no record has placed", async () => {
    expect(await mockClient.canFoldTransform("body1")).toBeNull();
  });

  it("names the record after a copy:false placement of that body", async () => {
    const res = await mockClient.applyOperation(moveOp(["body1"], [30, 0, 0]));
    const recordId = lastRow(res.features).id;
    expect(await mockClient.canFoldTransform("body1")).toBe(recordId);
  });

  it("still names the SAME record after a re-edit (one record per intent)", async () => {
    const first = await mockClient.applyOperation(moveOp(["body1"], [30, 0, 0]));
    const recordId = lastRow(first.features).id;
    await mockClient.applyOperation(moveOp(["body1"], [50, 0, 0], recordId));
    expect(await mockClient.canFoldTransform("body1")).toBe(recordId);
  });

  it("is null once a LATER op consumed the body — folding would move that op's input", async () => {
    await mockClient.applyOperation(moveOp(["body1"], [30, 0, 0]));
    await mockClient.applyOperation({
      opType: "Fillet",
      inputs: [{ primary: { bodyId: "body1", kind: "edge" }, }],
      params: { mode: "Fillet", radius: 2, edgeIds: ["e:0"] },
    });
    expect(await mockClient.canFoldTransform("body1")).toBeNull();
  });

  it("is null for a copy:true placement — folding would relocate the copies, not the source", async () => {
    await mockClient.applyOperation({
      opType: "TransformBody",
      params: { targets: ["body1"], translate: [100, 0, 0], rotate: { ...IDENTITY_ROTATION }, copy: true },
    });
    expect(await mockClient.canFoldTransform("body1")).toBeNull();
  });

  it("names a multi-body record for EACH of its targets", async () => {
    const extruded = await seedExtrudedBody();
    const res = await mockClient.applyOperation(moveOp(["body1", extruded], [0, 25, 0]));
    const recordId = lastRow(res.features).id;
    expect(await mockClient.canFoldTransform("body1")).toBe(recordId);
    expect(await mockClient.canFoldTransform(extruded)).toBe(recordId);
  });

  it("is null again after the placement is undone", async () => {
    await mockClient.applyOperation(moveOp(["body1"], [30, 0, 0]));
    await mockClient.undo();
    expect(await mockClient.canFoldTransform("body1")).toBeNull();
  });
});
