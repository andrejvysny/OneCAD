/*
 * VP-HARDENING WP04 — MESH1 semantic validation, resource admission, and the
 * adversarial mutation sweep (lane U).
 *
 * TEST-MESH-01 structural mutations stay the PARSER's job and keep their typed
 *              kinds; an unknown optional section is skipped, not rejected.
 * TEST-MESH-02 non-finite values, bad bounds, invalid normals and out-of-range
 *              indices are refused BEFORE any derived array is allocated.
 * TEST-MESH-03 face/edge range and ID-table semantics: legal zero-length ranges
 *              and cross-namespace name reuse accepted, impossible tables refused.
 * TEST-MESH-04 checked arithmetic and peak resource admission refuse enormous
 *              but structurally plausible payloads before allocation.
 */
import { describe, it, expect } from "vitest";
import {
  MeshParseError,
  parseMeshPayload,
  type BodyMeshView,
  type MeshParseErrorKind,
} from "./parseMeshPayload";
import {
  MESH_BUDGETS,
  MeshAdmission,
  preparedCpuBytesOf,
} from "./meshAdmission";
import {
  validateMeshView,
  isValidatedMesh,
  requireValidatedMesh,
  MeshValidationError,
  type MeshAccounting,
  type MeshValidationCode,
} from "./validateMesh";
import { expandEdgeSegments } from "./meshRegistry";
import {
  MESH_MUTATORS,
  applyNamedMutation,
  mutateMesh,
  withUnknownSection,
  type MutationExpectation,
} from "./meshMutations";
import { encodeMesh1, makeBoxMesh, makeCylinderMesh } from "@/ipc/mockMeshes";
import { makeBodyMeshViewFixture } from "@/test/fixtures/bodyMeshView";

const BODY = "body_1";
const ZERO_BOUNDS = { min: [0, 0, 0], max: [0, 0, 0] } as const;

function validate(blob: ArrayBuffer, budgets?: { singleMeshPayloadBytes: number }) {
  return validateMeshView(parseMeshPayload(blob), BODY, budgets);
}

/** The outcome of running a blob through parse → validate, as one comparable token. */
function pipeline(blob: ArrayBuffer): MutationExpectation {
  let view: BodyMeshView;
  try {
    view = parseMeshPayload(blob);
  } catch (e) {
    if (e instanceof MeshParseError) return e.kind;
    throw e;
  }
  const result = validateMeshView(view, BODY);
  if (!result.ok) return result.code;
  const { headerBoundsExcursionMm, headerBoundsSlackMm } = result.mesh.accounting;
  return headerBoundsExcursionMm > headerBoundsSlackMm ? "accept-with-excursion" : "accept";
}

/** A two-triangle square with per-face bboxes, ElementId-style ids, and one edge. */
function faceBboxMesh(): ArrayBuffer {
  return encodeMesh1({
    positions: [0, 0, 0, 10, 0, 0, 10, 10, 0, 0, 10, 0],
    normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1],
    faces: [
      { triangles: [[0, 1, 2]], id: "f:0" },
      { triangles: [[0, 2, 3]], id: "f:1" },
    ],
    edges: [
      { points: [[0, 0, 0], [10, 0, 0]], id: "e:0" },
      { points: [[10, 0, 0], [10, 10, 0]], id: "e:1" },
    ],
    faceBboxes: true,
  });
}

/** Face ids long enough (39 bytes each) that an offset table can imply a >256-byte id. */
function longIdMesh(): ArrayBuffer {
  const id = (n: number) => `el_${String(n).padStart(2, "0")}0f4a1b2c3d4e5f60718293a4b5c6d7e`;
  return encodeMesh1({
    positions: [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 2, 0, 0, 2, 1, 0, 3, 0, 0, 3, 1, 0],
    faces: [
      { triangles: [[0, 1, 2]], id: id(1) },
      { triangles: [[0, 2, 3]], id: id(2) },
      { triangles: [[1, 4, 5]], id: id(3) },
      { triangles: [[1, 5, 2]], id: id(4) },
      { triangles: [[4, 6, 7]], id: id(5) },
      { triangles: [[4, 7, 5]], id: id(6) },
      { triangles: [[6, 7, 3]], id: id(7) },
      { triangles: [[0, 3, 7]], id: id(8) },
    ],
    idsHaveElementIds: true,
  });
}

// ── TEST-MESH-01 — structure stays the parser's job ──────────────────────────

describe("TEST-MESH-01 structural mutations are typed parser rejections", () => {
  const cases: ReadonlyArray<readonly [string, MeshParseErrorKind, (dv: DataView) => void]> = [
    ["magic", "bad-magic", (dv) => dv.setUint32(0x00, 0x4d455349, true)],
    ["version", "unsupported-version", (dv) => dv.setUint16(0x04, 2, true)],
    ["reserved flag bit", "reserved-nonzero", (dv) => dv.setUint16(0x06, dv.getUint16(0x06, true) | 0x0020, true)],
    ["reserved header word", "reserved-nonzero", (dv) => dv.setUint32(0x38, 1, true)],
    ["section pad", "reserved-nonzero", (dv) => dv.setUint32(64 + 0x04, 7, true)],
    ["section alignment", "misaligned", (dv) => dv.setUint32(64 + 0x08, dv.getUint32(64 + 0x08, true) + 2, true)],
    ["section overlap", "section-overlap", (dv) => dv.setUint32(64 + 16 + 0x08, dv.getUint32(64 + 0x08, true), true)],
    ["duplicate type", "duplicate-section", (dv) => dv.setUint32(64 + 16 + 0x00, dv.getUint32(64 + 0x00, true), true)],
    ["section length", "bad-length", (dv) => dv.setUint32(64 + 0x0c, dv.getUint32(64 + 0x0c, true) - 4, true)],
  ];

  for (const [name, kind, mutate] of cases) {
    it(`TEST-MESH-01 rejects a mutated ${name} with kind "${kind}"`, () => {
      const blob = makeBoxMesh();
      mutate(new DataView(blob));
      expect(() => parseMeshPayload(blob)).toThrow(MeshParseError);
      try {
        parseMeshPayload(blob);
      } catch (e) {
        expect((e as MeshParseError).kind).toBe(kind);
      }
    });
  }

  it("TEST-MESH-01 rejects a truncated blob before reading any section", () => {
    expect(() => parseMeshPayload(makeBoxMesh().slice(0, 32))).toThrow(
      expect.objectContaining({ kind: "truncated" }),
    );
  });

  it("TEST-MESH-01 rejects a blob whose last section runs past the end", () => {
    const blob = makeBoxMesh();
    expect(() => parseMeshPayload(blob.slice(0, blob.byteLength - 8))).toThrow(
      expect.objectContaining({ kind: "section-bounds" }),
    );
  });

  it("TEST-MESH-01 SKIPS an unknown optional section and still validates", () => {
    const blob = withUnknownSection(makeBoxMesh(), 200, new Uint8Array([9, 8, 7, 6]));
    const view = parseMeshPayload(blob);
    expect(view.faceCount).toBe(6);
    expect(view.edgeCount).toBe(12);
    const result = validateMeshView(view, BODY);
    expect(result.ok).toBe(true);
  });

  it("TEST-MESH-01 a pristine box and cylinder validate and price themselves", () => {
    for (const blob of [makeBoxMesh(), makeCylinderMesh(), faceBboxMesh(), longIdMesh()]) {
      const result = validate(blob);
      expect(result.ok).toBe(true);
      if (!result.ok) continue;
      const accounting: MeshAccounting = result.mesh.accounting;
      expect(accounting.payloadBytes).toBeGreaterThan(0);
      expect(accounting.estimatedGpuBytes).toBeGreaterThan(0);
      expect(isValidatedMesh(result.mesh)).toBe(true);
      expect(isValidatedMesh(result.mesh.view)).toBe(false);
    }
  });

  it("TEST-MESH-01 the box's segment accounting matches the real expansion", () => {
    const view = parseMeshPayload(makeBoxMesh());
    const result = validateMeshView(view, BODY);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    const expanded = expandEdgeSegments(view.edgePositions!, view.edgeRanges!, view.edgeCount);
    expect(result.mesh.accounting.segmentCount).toBe(expanded.segTotal);
    expect(result.mesh.accounting.edgeSegmentBytes).toBe(expanded.positions.byteLength);
  });
});

// ── TEST-MESH-02 — values, normals, indices, bounds ──────────────────────────

describe("TEST-MESH-02 non-finite values, bad normals, and out-of-range indices", () => {
  const nonFinite = [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY];

  for (const bad of nonFinite) {
    it(`TEST-MESH-02 rejects a ${bad} position component`, () => {
      const blob = makeBoxMesh();
      parseMeshPayload(blob).positions[7] = bad;
      const result = validate(blob);
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe<MeshValidationCode>("nonfinite-position");
        expect(result.bodyId).toBe(BODY);
        expect(result.detail).toContain("vertex 2 axis 1"); // float index 7
      }
    });

    it(`TEST-MESH-02 rejects a ${bad} normal component`, () => {
      const blob = makeBoxMesh();
      parseMeshPayload(blob).normals![4] = bad;
      const result = validate(blob);
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.code).toBe<MeshValidationCode>("nonfinite-normal");
    });
  }

  it("TEST-MESH-02 rejects a ZERO-length normal instead of normalising it", () => {
    const blob = makeBoxMesh();
    const view = parseMeshPayload(blob);
    view.normals![0] = 0;
    view.normals![1] = 0;
    view.normals![2] = 0;
    const result = validate(blob);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe<MeshValidationCode>("normal-length");
      expect(result.detail).toContain("length 0");
    }
  });

  it("TEST-MESH-02 rejects a normal of length 1.01 and accepts one inside the window", () => {
    const scaled = (factor: number) => {
      const blob = makeBoxMesh();
      const view = parseMeshPayload(blob);
      for (let a = 0; a < 3; a++) view.normals![a] *= factor;
      return validate(blob);
    };
    const rejected = scaled(1.01);
    expect(rejected.ok).toBe(false);
    if (!rejected.ok) expect(rejected.code).toBe<MeshValidationCode>("normal-length");
    expect(scaled(1.0005).ok).toBe(true);
  });

  it("TEST-MESH-02 rejects a triangle index at or past the vertex count", () => {
    for (const offset of [0, 1, 64]) {
      const blob = makeBoxMesh();
      const view = parseMeshPayload(blob);
      view.indices[3] = view.vertexCount + offset;
      const result = validate(blob);
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.code).toBe<MeshValidationCode>("index-out-of-range");
    }
  });

  it("TEST-MESH-02 MEASURES a header bbox that does not enclose the geometry instead of rejecting it", () => {
    // OCCT fills the MESH1 header box from the shape BEFORE re-tessellating, so
    // a stale coarse box is a producer defect, not a broken body: the excursion
    // is reported and the measured bounds win (numerics §9.3 / WP10 revisits).
    const blob = makeBoxMesh();
    new DataView(blob).setFloat32(0x2c, 0, true); // bboxMax.x, box spans ±40
    const result = validate(blob);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    const { accounting } = result.mesh;
    expect(accounting.headerBoundsExcursionMm).toBeCloseTo(40, 5);
    expect(accounting.headerBoundsExcursionMm).toBeGreaterThan(accounting.headerBoundsSlackMm);
    expect(accounting.actualBounds.max[0]).toBeCloseTo(40, 5);
    expect(accounting.actualBounds.min[0]).toBeCloseTo(-40, 5);
    expect(pipeline(blob)).toBe("accept-with-excursion");
  });

  it("TEST-MESH-02 reports zero excursion and exact measured bounds for an honest header", () => {
    const result = validate(makeBoxMesh());
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.mesh.accounting.headerBoundsExcursionMm).toBe(0);
    expect(result.mesh.accounting.actualBounds).toEqual({ min: [-40, -30, -15], max: [40, 30, 15] });
  });

  it("TEST-MESH-02 rejects a non-finite EDGE point, which a bounds test alone would miss", () => {
    // Every comparison against NaN is false, so an enclosure check silently
    // accepts one; the finiteness sweep is what actually catches it.
    const blob = makeBoxMesh();
    parseMeshPayload(blob).edgePositions![4] = Number.NaN;
    const result = validate(blob);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe<MeshValidationCode>("nonfinite-position");
      expect(result.detail).toContain("edge point");
    }
  });

  it("TEST-MESH-02 rejects a header lod outside the coarse/medium/fine tiers", () => {
    for (const lod of [3, 7, 0xffff]) {
      const blob = makeBoxMesh();
      new DataView(blob).setUint16(0x1c, lod, true);
      const result = validate(blob);
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.code).toBe<MeshValidationCode>("lod");
    }
    for (const lod of [0, 1, 2]) {
      const blob = makeBoxMesh();
      new DataView(blob).setUint16(0x1c, lod, true);
      expect(validate(blob).ok).toBe(true);
    }
  });

  it("TEST-MESH-02 rejects a non-finite and an inverted header bbox", () => {
    const nan = makeBoxMesh();
    new DataView(nan).setFloat32(0x20, Number.NaN, true);
    expect(pipeline(nan)).toBe("bbox");

    const inverted = makeBoxMesh();
    const dv = new DataView(inverted);
    const min = dv.getFloat32(0x24, true);
    dv.setFloat32(0x24, dv.getFloat32(0x30, true), true);
    dv.setFloat32(0x30, min, true);
    expect(pipeline(inverted)).toBe("bbox");
  });

  it("TEST-MESH-02 treats a sub-slack header excursion as noise, not a diagnostic", () => {
    const blob = makeBoxMesh();
    // A vertex outside by less than max(1e-4, 1e-6·extent) is measurement noise
    // and must not raise a producer diagnostic.
    new DataView(blob).setFloat32(0x2c, 40 - 5e-5, true);
    expect(pipeline(blob)).toBe("accept");
    const result = validate(blob);
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.mesh.accounting.headerBoundsExcursionMm).toBeLessThanOrEqual(
        result.mesh.accounting.headerBoundsSlackMm,
      );
    }
  });

  it("TEST-MESH-02 rejects an inverted per-face bbox", () => {
    const blob = faceBboxMesh();
    const view = parseMeshPayload(blob);
    const lo = view.faceBboxes![0];
    view.faceBboxes![0] = view.faceBboxes![3];
    view.faceBboxes![3] = lo;
    const result = validate(blob);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.code).toBe<MeshValidationCode>("face-bbox");
  });

  it("TEST-MESH-02 allocates NO typed arrays while proving a payload wrong", () => {
    const blob = makeBoxMesh();
    const view = parseMeshPayload(blob);
    view.positions[11] = Number.NaN;

    // Validation reads the zero-copy views and must not materialise anything —
    // in particular not the edge expansion, which `buildBodyObjects` allocates.
    const realFloat32 = globalThis.Float32Array;
    const realUint32 = globalThis.Uint32Array;
    let allocations = 0;
    class CountingFloat32 extends realFloat32 {
      constructor(...args: ConstructorParameters<typeof Float32Array>) {
        super(...args);
        allocations++;
      }
    }
    class CountingUint32 extends realUint32 {
      constructor(...args: ConstructorParameters<typeof Uint32Array>) {
        super(...args);
        allocations++;
      }
    }
    globalThis.Float32Array = CountingFloat32 as unknown as typeof Float32Array;
    globalThis.Uint32Array = CountingUint32 as unknown as typeof Uint32Array;
    let result: ReturnType<typeof validateMeshView>;
    try {
      result = validateMeshView(view, BODY);
    } finally {
      globalThis.Float32Array = realFloat32;
      globalThis.Uint32Array = realUint32;
    }
    expect(result.ok).toBe(false);
    expect(allocations).toBe(0);
  });

  it("TEST-MESH-02 buildBodyObjects refuses a raw view that fails validation", () => {
    const blob = makeBoxMesh();
    parseMeshPayload(blob).positions[2] = Number.POSITIVE_INFINITY;
    expect(() => requireValidatedMesh(parseMeshPayload(blob), BODY)).toThrow(MeshValidationError);
  });
});

// ── TEST-MESH-03 — ranges and ID tables ──────────────────────────────────────

describe("TEST-MESH-03 face/edge ranges and ID tables", () => {
  it("TEST-MESH-03 rejects a gap, an overlap, and a short final range", () => {
    const shift = (delta: number, index: number) => {
      const blob = makeBoxMesh();
      parseMeshPayload(blob).faceRanges[index] += delta;
      return pipeline(blob);
    };
    expect(shift(1, 2)).toBe("face-range"); // face 1 starts one triangle late
    expect(shift(-1, 4)).toBe("face-range"); // face 2 starts one triangle early
    expect(shift(-1, 11)).toBe("face-range"); // last face covers one triangle too few
  });

  it("TEST-MESH-03 ACCEPTS zero-length face and edge ranges at a shared boundary", () => {
    const blob = encodeMesh1({
      positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
      faces: [
        { triangles: [], id: "f:0" }, // a declared degenerate face owns no triangles
        { triangles: [[0, 1, 2]], id: "f:1" },
        { triangles: [], id: "f:2" },
      ],
      edges: [
        { points: [], id: "e:0" }, // a declared degenerate edge owns no points
        { points: [[0, 0, 0], [1, 0, 0]], id: "e:1" },
      ],
    });
    const view = parseMeshPayload(blob);
    expect(view.faceRanges[0]).toBe(0);
    expect(view.faceRanges[1]).toBe(0);
    expect(validateMeshView(view, BODY).ok).toBe(true);
  });

  it("TEST-MESH-03 rejects broken edge point ranges", () => {
    const bump = (index: number) => {
      const blob = makeBoxMesh();
      parseMeshPayload(blob).edgeRanges![index] += 1;
      return pipeline(blob);
    };
    expect(bump(0)).toBe("edge-range"); // edge 0 no longer starts at 0
    expect(bump(1)).toBe("edge-range"); // edge 0 claims a point edge 1 owns
    expect(bump(23)).toBe("edge-range"); // last edge runs past P
  });

  it("TEST-MESH-03 rejects edge counts declared without the edge sections", () => {
    const view: BodyMeshView = { ...makeBodyMeshViewFixture(), edgeCount: 2, edgePointCount: 4 };
    const result = validateMeshView(view, BODY);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.code).toBe<MeshValidationCode>("edge-range");
  });

  it("TEST-MESH-03 rejects non-monotonic, short, and long ID offset tables", () => {
    const nonMonotonic = makeBoxMesh();
    const nmView = parseMeshPayload(nonMonotonic);
    nmView.faceIdOffsets[2] = nmView.faceIdOffsets[3] + 1;
    expect(pipeline(nonMonotonic)).toBe("id-offsets");

    // A moved TERMINAL offset disagrees with FACE_ID_CHARS.byteLen, which the
    // parser owns; validation sees the same violation on a hand-built view.
    const long = makeBoxMesh();
    parseMeshPayload(long).faceIdOffsets[6] += 4;
    expect(pipeline(long)).toBe("bad-length");

    const base = makeBodyMeshViewFixture({
      faces: [{ id: "f:0", triangles: [[0, 1, 2]] }],
    });
    const shortEnd: BodyMeshView = { ...base, faceIdOffsets: Uint32Array.from([0, 2]) };
    const shortResult = validateMeshView(shortEnd, BODY);
    expect(shortResult.ok).toBe(false);
    if (!shortResult.ok) expect(shortResult.code).toBe<MeshValidationCode>("id-offsets");

    const nonZeroStart: BodyMeshView = { ...base, faceIdOffsets: Uint32Array.from([1, 3]) };
    const startResult = validateMeshView(nonZeroStart, BODY);
    expect(startResult.ok).toBe(false);
    if (!startResult.ok) expect(startResult.code).toBe<MeshValidationCode>("id-offsets");
  });

  it("TEST-MESH-03 rejects invalid UTF-8, an empty id, and a 257-byte id", () => {
    const badUtf8 = makeBoxMesh();
    parseMeshPayload(badUtf8).faceIdChars[1] = 0xff;
    expect(pipeline(badUtf8)).toBe("id-utf8");

    const empty = makeBoxMesh();
    const emptyView = parseMeshPayload(empty);
    emptyView.faceIdOffsets[1] = emptyView.faceIdOffsets[0];
    expect(pipeline(empty)).toBe("id-empty");

    const overlong = encodeMesh1({
      positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
      faces: [{ triangles: [[0, 1, 2]], id: "e".repeat(257) }],
    });
    expect(pipeline(overlong)).toBe("id-too-long");

    const atCap = encodeMesh1({
      positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
      faces: [{ triangles: [[0, 1, 2]], id: "e".repeat(256) }],
    });
    expect(pipeline(atCap)).toBe("accept");
  });

  it("TEST-MESH-03 rejects duplicate face ids and duplicate edge ids", () => {
    const dupFace = makeBoxMesh();
    const faceView = parseMeshPayload(dupFace);
    faceView.faceIdChars.set(faceView.faceIdChars.subarray(0, 3), 3);
    expect(pipeline(dupFace)).toBe("id-duplicate");

    const dupEdge = makeBoxMesh();
    const edgeView = parseMeshPayload(dupEdge);
    edgeView.edgeIdChars!.set(edgeView.edgeIdChars!.subarray(0, 3), 3);
    expect(pipeline(dupEdge)).toBe("id-duplicate");
  });

  it("TEST-MESH-03 ACCEPTS the same text in the face and edge namespaces", () => {
    const blob = makeBoxMesh();
    const view = parseMeshPayload(blob);
    // Edge 0's id becomes "f:0" — legal: uniqueness is per namespace (numerics §10).
    view.edgeIdChars!.set(view.faceIdChars.subarray(0, 3), 0);
    expect(pipeline(blob)).toBe("accept");
  });
});

// ── TEST-MESH-04 — checked arithmetic and admission ──────────────────────────

describe("TEST-MESH-04 overflow-safe counts and peak resource admission", () => {
  it("TEST-MESH-04 refuses counts whose products leave safe-integer range", () => {
    const base = makeBodyMeshViewFixture();
    for (const counts of [
      { vertexCount: 2 ** 52 },
      { triangleCount: 2 ** 52 },
      { faceCount: 2 ** 52 },
      { edgeCount: Number.NaN },
      { edgePointCount: -1 },
    ]) {
      const view: BodyMeshView = { ...base, ...counts };
      const result = validateMeshView(view, BODY);
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.code).toBe<MeshValidationCode>("count-overflow");
    }
  });

  it("TEST-MESH-04 refuses a payload near 2^32 bytes against the 256 MiB cap", () => {
    // 2^30 vertices is a legal u32 header count; 12 GiB of positions is not a
    // legal payload, and saying so must not require touching the array.
    const view: BodyMeshView = { ...makeBodyMeshViewFixture(), vertexCount: 2 ** 30 };
    const result = validateMeshView(view, BODY);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe<MeshValidationCode>("payload-budget");
      expect(result.detail).toContain(String(MESH_BUDGETS.singleMeshPayloadBytes));
    }
  });

  it("TEST-MESH-04 refuses a legal small blob whose edge ranges imply a huge expansion", () => {
    const blob = makeBoxMesh();
    const view = parseMeshPayload(blob);
    // Structurally perfect: 12 edges, 24 points, a 1 KB payload — and one point
    // count that expands to ~13 GB of segment endpoints.
    view.edgeRanges![1] = 0x2000_0000;
    expect(blob.byteLength).toBeLessThan(4096);

    const started = performance.now();
    const result = validateMeshView(parseMeshPayload(blob), BODY);
    const elapsedMs = performance.now() - started;

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe<MeshValidationCode>("expansion-budget");
      // 0x20000000 − 1 segments from the mutated edge plus 11 from the rest.
      expect(result.detail).toContain("536870922 segments");
    }
    // Refused by arithmetic: attempting the allocation could not be this cheap.
    expect(elapsedMs).toBeLessThan(50);
  });

  it("TEST-MESH-04 counts edge expansion against the single-payload cap", () => {
    const blob = makeBoxMesh();
    const generous = validate(blob, { singleMeshPayloadBytes: MESH_BUDGETS.singleMeshPayloadBytes });
    expect(generous.ok).toBe(true);
    const tight = validate(blob, { singleMeshPayloadBytes: 1200 });
    expect(tight.ok).toBe(false);
    if (!tight.ok) expect(["payload-budget", "expansion-budget"]).toContain(tight.code);
  });

  it("TEST-MESH-04 admission refuses a 600 MiB replacement of a 600 MiB body but admits 400 MiB", () => {
    const MIB = 1024 * 1024;
    const accounting = (mib: number): MeshAccounting => ({
      actualBounds: ZERO_BOUNDS,
      headerBoundsExcursionMm: 0,
      headerBoundsSlackMm: 1e-4,
      payloadBytes: mib * MIB,
      edgeSegmentBytes: 0,
      colorBytes: 0,
      estimatedGpuBytes: 1,
      triangleCount: 1,
      segmentCount: 0,
    });
    const admission = new MeshAdmission({ preparedCpuBytes: 1024 * MIB, estimatedGpuBytes: 768 * MIB });

    const installed = admission.reserve("body1", accounting(600));
    expect(installed.ok).toBe(true);
    expect(admission.snapshot().preparedCpuBytes).toBe(600 * MIB);

    // Peak rule: while the 600 MiB body is still installed, a 600 MiB
    // replacement would need 1200 MiB at once.
    const refused = admission.reserve("body1", accounting(600));
    expect(refused.ok).toBe(false);
    if (!refused.ok) {
      expect(refused.reason).toBe("cpu-budget");
      expect(refused.detail).toContain("body1");
    }

    const admitted = admission.reserve("body1", accounting(400));
    expect(admitted.ok).toBe(true);
    expect(admission.snapshot()).toEqual({
      preparedCpuBytes: 1000 * MIB,
      estimatedGpuBytes: 2,
      holdings: 2,
    });

    // Retiring the old entry frees its bytes, and `release` is idempotent.
    if (installed.ok) {
      installed.release();
      installed.release();
    }
    expect(admission.snapshot()).toEqual({
      preparedCpuBytes: 400 * MIB,
      estimatedGpuBytes: 1,
      holdings: 1,
    });
  });

  it("TEST-MESH-04 admission refuses on the GPU cap independently of the CPU cap", () => {
    const admission = new MeshAdmission({ preparedCpuBytes: 1_000_000_000, estimatedGpuBytes: 1000 });
    const refused = admission.reserve("body1", {
      actualBounds: ZERO_BOUNDS,
      headerBoundsExcursionMm: 0,
      headerBoundsSlackMm: 1e-4,
      payloadBytes: 10,
      edgeSegmentBytes: 0,
      colorBytes: 0,
      estimatedGpuBytes: 1001,
      triangleCount: 1,
      segmentCount: 0,
    });
    expect(refused.ok).toBe(false);
    if (!refused.ok) expect(refused.reason).toBe("gpu-budget");
    expect(admission.snapshot().holdings).toBe(0);
  });

  it("TEST-MESH-04 prices a real body's prepared CPU cost from its accounting", () => {
    const result = validate(makeCylinderMesh());
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    const { accounting } = result.mesh;
    expect(preparedCpuBytesOf(accounting)).toBe(
      accounting.payloadBytes + accounting.edgeSegmentBytes + accounting.colorBytes,
    );
    expect(new MeshAdmission().reserve(BODY, accounting).ok).toBe(true);
  });
});

// ── TEST-MESH-01…04 — seeded adversarial sweep ───────────────────────────────

/**
 * Counterexamples the sweep has found, replayed on every run as fixed
 * regressions. Each entry names the base blob, the mutator, and the
 * `(seed, index)` that produced it; `mutateMesh` is deterministic in that
 * triple, so an entry reproduces exactly without replaying the whole sweep.
 *
 * EMPTY is the expected steady state: the 2026-09-13 sweep of 10,000 cases over
 * seed 90213 found none. An entry appearing here means the sweep caught the
 * validator disagreeing with the specification, and it stays forever.
 */
const MESH_ADVERSARIAL_CORPUS: ReadonlyArray<{
  readonly base: "box" | "cylinder" | "faceBbox" | "longId";
  readonly mutator: string;
  readonly seed: number;
  readonly index: number;
  readonly expected: MutationExpectation;
}> = [];

const SWEEP_SEED = 90213;
const SWEEP_CASES = 10_000;

describe("TEST-MESH-01 TEST-MESH-02 TEST-MESH-03 TEST-MESH-04 seeded adversarial sweep", () => {
  const bases = {
    box: makeBoxMesh(),
    cylinder: makeCylinderMesh(32, 40, 12),
    faceBbox: faceBboxMesh(),
    longId: longIdMesh(),
  } as const;
  type BaseName = keyof typeof bases;
  const baseNames = Object.keys(bases) as BaseName[];

  it("TEST-MESH-01 TEST-MESH-02 TEST-MESH-03 TEST-MESH-04 every single-invariant mutation lands on its declared outcome", () => {
    const started = performance.now();
    const counterexamples: string[] = [];
    const exercised = new Map<string, number>();
    let evaluated = 0;

    for (let index = 0; index < SWEEP_CASES; index++) {
      const baseName = baseNames[index % baseNames.length];
      const generated = mutateMesh(bases[baseName], SWEEP_SEED, index);
      if (!generated) continue;
      evaluated++;
      exercised.set(generated.mutator, (exercised.get(generated.mutator) ?? 0) + 1);
      const actual = pipeline(generated.blob);
      if (actual !== generated.expected) {
        counterexamples.push(
          `base=${baseName} mutator=${generated.mutator} seed=${SWEEP_SEED} index=${index} expected=${generated.expected} actual=${actual}`,
        );
      }
    }
    const elapsedMs = performance.now() - started;

    if (counterexamples.length > 0) {
      // Printed so the failing (base, mutator, seed, index) can go straight into
      // MESH_ADVERSARIAL_CORPUS above.
      // eslint-disable-next-line no-console
      console.error(`mesh mutation counterexamples:\n${counterexamples.slice(0, 20).join("\n")}`);
    }
    expect(counterexamples).toEqual([]);
    expect(evaluated).toBeGreaterThan(SWEEP_CASES / 2);
    // Every mutator in the table must actually have run against some base.
    const never = MESH_MUTATORS.map((m) => m.name).filter((name) => !exercised.has(name));
    expect(never).toEqual([]);
    expect(elapsedMs).toBeLessThan(20_000);
  });

  for (const entry of MESH_ADVERSARIAL_CORPUS) {
    it(`TEST-MESH-01 corpus regression ${entry.base}/${entry.mutator}@${entry.index}`, () => {
      const generated = applyNamedMutation(bases[entry.base], entry.mutator, entry.seed, entry.index);
      expect(generated).not.toBeNull();
      expect(pipeline(generated!.blob)).toBe(entry.expected);
    });
  }
});
