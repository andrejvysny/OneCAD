/*
 * MESH1 SEMANTIC validation — VP-HARDENING WP04 (spec §9 VP05, numerics §10).
 *
 * `parseMeshPayload` proves BYTE LAYOUT: magic, version, reserved bits, table
 * alignment/overlap/bounds, and that each section's byte length agrees with the
 * header counts. It proves nothing about what the numbers MEAN. A blob can be
 * perfectly framed and still carry NaN positions, indices past the vertex
 * count, face ranges that claim the same triangle twice, ID offsets that walk
 * backwards, or an edge table whose point counts imply a 25 GB segment buffer.
 *
 * This module is the semantic gate. It is PURE and SYNCHRONOUS, it allocates no
 * typed arrays (the zero-copy views survive untouched), and — the load-bearing
 * ordering rule from numerics §10 — it does every count/product/byte-size check
 * with CHECKED ARITHMETIC BEFORE anything derived is allocated. `buildBodyObjects`
 * is the only consumer that matters: it will not build GPU geometry from
 * anything but a {@link ValidatedMesh}, and a `ValidatedMesh` can only come out
 * of {@link validateMeshView}.
 *
 * The result is a value, never an exception: numerics §10 forbids throwing
 * across a UI store action, so the ingestion boundary turns a failure into an
 * explicit display state (`stale-inspection-only` / `failed-initial`) instead.
 * {@link MeshValidationError} exists only for the non-ingestion lanes (preview
 * and ghost meshes) that hand `buildBodyObjects` a raw view.
 *
 * Acceptance: TEST-MESH-01, TEST-MESH-02, TEST-MESH-03, TEST-MESH-04.
 */
import type { BodyMeshView } from "./parseMeshPayload";
import { MESH_BUDGETS } from "./meshAdmission";

/**
 * Runtime AND compile-time brand. Private to this module, so the only way to
 * hold a `ValidatedMesh` is to have obtained one from `validateMeshView`.
 */
const VALIDATED: unique symbol = Symbol("onecad.mesh.validated");

/** An axis-aligned box measured from the mesh's own points. */
export interface MeshBounds {
  readonly min: readonly [number, number, number];
  readonly max: readonly [number, number, number];
}

/** Byte cost of a mesh once every derived array is materialised, plus its measured extent. */
export interface MeshAccounting {
  /**
   * Bounds MEASURED from the positions and edge points — the authority the
   * renderer uses, because the header box is the producer's advisory statement
   * and can be stale (see the enclosure note in `validateMeshView`). Falls back
   * to the header box for a mesh with no points at all.
   */
  readonly actualBounds: MeshBounds;
  /** How far the furthest point lies OUTSIDE the header box, per axis; 0 when enclosed. */
  readonly headerBoundsExcursionMm: number;
  /** The excursion above which the header box is worth reporting: `max(1e-4, 1e-6·extent)`. */
  readonly headerBoundsSlackMm: number;
  /** The MESH1 sections themselves (positions, normals, indices, tables, ids). */
  readonly payloadBytes: number;
  /** The expanded fat-line segment endpoint buffer (`expandEdgeSegments`). */
  readonly edgeSegmentBytes: number;
  /** The de-indexed + baked colour representation, when the mesh carries FACE_COLORS. */
  readonly colorBytes: number;
  /** Typed-array bytes that will actually be handed to GPU geometry. */
  readonly estimatedGpuBytes: number;
  readonly triangleCount: number;
  /** Fat-line segments the edge tables expand to: Σ max(0, pointCount − 1). */
  readonly segmentCount: number;
}

/** A view whose SEMANTICS have been proved. Obtainable only from {@link validateMeshView}. */
export interface ValidatedMesh {
  readonly [VALIDATED]: true;
  readonly view: BodyMeshView;
  readonly accounting: MeshAccounting;
}

export type MeshValidationCode =
  | "nonfinite-position"
  | "nonfinite-normal"
  | "normal-length"
  | "index-out-of-range"
  | "face-range"
  | "edge-range"
  | "id-offsets"
  | "id-utf8"
  | "id-empty"
  | "id-too-long"
  | "id-duplicate"
  | "bbox"
  | "face-bbox"
  | "lod"
  | "count-overflow"
  | "payload-budget"
  | "expansion-budget";

export type ValidationResult =
  | { readonly ok: true; readonly mesh: ValidatedMesh }
  | {
      readonly ok: false;
      readonly code: MeshValidationCode;
      readonly detail: string;
      readonly bodyId: string;
    };

export interface MeshValidationBudgets {
  /** One mesh payload including its derived expansion (spec §9: 256 MiB). */
  readonly singleMeshPayloadBytes: number;
}

/** Typed failure for the raw-view lanes. The ingestion boundary never sees this. */
export class MeshValidationError extends Error {
  constructor(
    readonly code: MeshValidationCode,
    readonly detail: string,
    readonly bodyId: string,
  ) {
    super(`mesh ${bodyId}: ${code} — ${detail}`);
    this.name = "MeshValidationError";
  }
}

/** IDs are capped per numerics §10; real ElementIds and TopoKeys are far shorter. */
const MAX_ID_BYTES = 256;
/** A normal must already be unit length. Never renormalise — a bad normal is a producer defect. */
const NORMAL_LENGTH_MIN = 0.999;
const NORMAL_LENGTH_MAX = 1.001;
/** Header-bbox enclosure slack: absolute floor, plus a relative term for large parts. */
const BBOX_ABS_SLACK = 1e-4;
const BBOX_REL_SLACK = 1e-6;
/** Bytes per expanded fat-line segment: 2 endpoints × 3 floats × 4 bytes. */
const BYTES_PER_SEGMENT = 24;

export function isValidatedMesh(mesh: ValidatedMesh | BodyMeshView): mesh is ValidatedMesh {
  return VALIDATED in mesh;
}

/** `a·b` when the exact product is representable, else null (numerics §10). */
function checkedMul(a: number, b: number): number | null {
  const product = a * b;
  return Number.isSafeInteger(product) ? product : null;
}

function checkedAdd(a: number, b: number): number | null {
  const sum = a + b;
  return Number.isSafeInteger(sum) ? sum : null;
}

interface IdFailure {
  readonly code: MeshValidationCode;
  readonly detail: string;
}

/**
 * Validate one ID table (numerics §10): offsets start at 0, are monotonic
 * non-decreasing, end exactly at the character-section length; every id is
 * nonempty, at most 256 UTF-8 bytes, decodes under FATAL UTF-8, and is unique
 * WITHIN its own namespace. The same text appearing in both the face and edge
 * namespaces is legal and must not be rejected.
 */
function validateIdTable(
  offsets: Uint32Array,
  chars: Uint8Array,
  count: number,
  namespace: "face" | "edge",
): IdFailure | null {
  if (offsets[0] !== 0) {
    return { code: "id-offsets", detail: `${namespace} id offsets start at ${offsets[0]}, not 0` };
  }
  for (let i = 0; i < count; i++) {
    const start = offsets[i];
    const end = offsets[i + 1];
    if (!Number.isSafeInteger(end) || !(end >= start)) {
      return {
        code: "id-offsets",
        detail: `${namespace} id offsets not monotonic at ${i}: ${start} → ${end}`,
      };
    }
  }
  if (offsets[count] !== chars.length) {
    return {
      code: "id-offsets",
      detail: `${namespace} id offsets end at ${offsets[count]}, chars length is ${chars.length}`,
    };
  }

  const decoder = new TextDecoder("utf-8", { fatal: true });
  const seen = new Set<string>();
  for (let i = 0; i < count; i++) {
    const start = offsets[i];
    const byteLen = offsets[i + 1] - start;
    if (byteLen === 0) {
      return { code: "id-empty", detail: `${namespace} id ${i} is empty` };
    }
    if (byteLen > MAX_ID_BYTES) {
      return {
        code: "id-too-long",
        detail: `${namespace} id ${i} is ${byteLen} bytes, cap is ${MAX_ID_BYTES}`,
      };
    }
    let text: string;
    try {
      text = decoder.decode(chars.subarray(start, start + byteLen));
    } catch {
      return { code: "id-utf8", detail: `${namespace} id ${i} is not valid UTF-8` };
    }
    if (seen.has(text)) {
      return { code: "id-duplicate", detail: `${namespace} id ${i} repeats "${text}"` };
    }
    seen.add(text);
  }
  return null;
}

/**
 * Prove a parsed view's SEMANTICS and price it. Checks run in the order the
 * specification lists them, so a payload violating several invariants is
 * reported against the first — deterministically, which is what makes the
 * adversarial corpus reproducible.
 *
 * Every loop walks the DECLARED extent (3·V, 3·T, F, E, 3·P), not the array's
 * own length. `parseMeshPayload` already guarantees the two agree for a real
 * blob; for a hand-built view that disagrees, the out-of-range read yields
 * `undefined`, which every check below rejects. There is no path that reads
 * past a buffer or trusts a count an array cannot back.
 */
export function validateMeshView(
  view: BodyMeshView,
  bodyId: string,
  budgets: Partial<MeshValidationBudgets> = {},
): ValidationResult {
  const payloadCap = budgets.singleMeshPayloadBytes ?? MESH_BUDGETS.singleMeshPayloadBytes;
  const fail = (code: MeshValidationCode, detail: string): ValidationResult => ({
    ok: false,
    code,
    detail,
    bodyId,
  });

  const vertexCount = view.vertexCount;
  const triangleCount = view.triangleCount;
  const faceCount = view.faceCount;
  const edgeCount = view.edgeCount;
  const edgePointCount = view.edgePointCount;

  // ── (a) header quality tier, counts, products, and byte budgets ──
  // All of this runs BEFORE any derived allocation (numerics §10).
  if (view.lod !== 0 && view.lod !== 1 && view.lod !== 2) {
    return fail("lod", `lod ${view.lod} is not coarse (0), medium (1), or fine (2)`);
  }
  const declared: ReadonlyArray<readonly [string, number]> = [
    ["vertexCount", vertexCount],
    ["triangleCount", triangleCount],
    ["faceCount", faceCount],
    ["edgeCount", edgeCount],
    ["edgePointCount", edgePointCount],
  ];
  for (const [name, n] of declared) {
    if (!Number.isSafeInteger(n) || n < 0) {
      return fail("count-overflow", `${name} ${n} is not a non-negative safe integer`);
    }
  }

  const positionFloats = checkedMul(3, vertexCount);
  const indexWords = checkedMul(3, triangleCount);
  const faceRangeWords = checkedMul(2, faceCount);
  const edgeRangeWords = checkedMul(2, edgeCount);
  const edgePositionFloats = checkedMul(3, edgePointCount);
  const faceBboxFloats = checkedMul(6, faceCount);
  const faceColorBytes = checkedMul(4, faceCount);
  if (
    positionFloats === null ||
    indexWords === null ||
    faceRangeWords === null ||
    edgeRangeWords === null ||
    edgePositionFloats === null ||
    faceBboxFloats === null ||
    faceColorBytes === null
  ) {
    return fail(
      "count-overflow",
      `declared counts overflow safe integer arithmetic (V=${vertexCount} T=${triangleCount} F=${faceCount} E=${edgeCount} P=${edgePointCount})`,
    );
  }

  // Section byte sizes as {elements, bytesPerElement}, summed with checked
  // arithmetic. Derived from the DECLARED counts, never from array lengths —
  // the point is to price the payload before trusting anything about it.
  const payloadTerms: ReadonlyArray<readonly [number, number]> = [
    [positionFloats, 4],
    [view.normals ? positionFloats : 0, 4],
    [indexWords, 4],
    [faceRangeWords, 4],
    [faceCount + 1, 4],
    [view.faceIdChars.length, 1],
    [view.hasEdges ? edgeRangeWords : 0, 4],
    [view.hasEdges ? edgePositionFloats : 0, 4],
    [view.hasEdges ? edgeCount + 1 : 0, 4],
    [view.hasEdges ? (view.edgeIdChars?.length ?? 0) : 0, 1],
    [view.faceBboxes ? faceBboxFloats : 0, 4],
    [view.faceColors ? faceColorBytes : 0, 1],
  ];
  let payloadBytes = 0;
  for (const [elements, bytesPerElement] of payloadTerms) {
    const bytes = checkedMul(elements, bytesPerElement);
    const total = bytes === null ? null : checkedAdd(payloadBytes, bytes);
    if (total === null) {
      return fail("count-overflow", "payload byte size overflows safe integer arithmetic");
    }
    payloadBytes = total;
  }
  if (payloadBytes > payloadCap) {
    return fail("payload-budget", `payload ${payloadBytes} bytes exceeds the ${payloadCap} byte cap`);
  }

  // Authored FACE_COLORS force a de-indexed prepared representation (faceColors.ts):
  // 3·T vertices carrying position, colour, and — when present — normal.
  let colorBytes = 0;
  if (view.faceColors) {
    const deIndexedFloats = checkedMul(indexWords, 3);
    const streams = view.normals ? 3 : 2;
    const bytes = deIndexedFloats === null ? null : checkedMul(deIndexedFloats, 4 * streams);
    if (bytes === null) {
      return fail("count-overflow", "de-indexed colour representation overflows safe integer arithmetic");
    }
    colorBytes = bytes;
    const payloadWithColor = checkedAdd(payloadBytes, colorBytes);
    if (payloadWithColor === null || payloadWithColor > payloadCap) {
      return fail(
        "payload-budget",
        `payload ${payloadBytes} + colour expansion ${colorBytes} bytes exceeds the ${payloadCap} byte cap`,
      );
    }
  }

  // Edge expansion cost, summed with checked arithmetic from the RAW ranges —
  // this is precisely the allocation `expandEdgeSegments` would make, and it is
  // priced before the ranges themselves are proved contiguous below.
  let segmentCount = 0;
  if (view.hasEdges && view.edgeRanges) {
    const ranges = view.edgeRanges;
    for (let e = 0; e < edgeCount; e++) {
      const pointCount = ranges[e * 2 + 1];
      if (!Number.isSafeInteger(pointCount) || pointCount < 0) {
        return fail("count-overflow", `edge ${e} pointCount ${pointCount} is not a non-negative safe integer`);
      }
      const next = checkedAdd(segmentCount, pointCount > 0 ? pointCount - 1 : 0);
      if (next === null) {
        return fail("count-overflow", `edge segment total overflows safe integer arithmetic at edge ${e}`);
      }
      segmentCount = next;
    }
  }
  const edgeSegmentBytes = checkedMul(segmentCount, BYTES_PER_SEGMENT);
  if (edgeSegmentBytes === null) {
    return fail("count-overflow", `edge expansion of ${segmentCount} segments overflows safe integer arithmetic`);
  }
  const withColor = checkedAdd(payloadBytes, colorBytes);
  const preparedBytes = withColor === null ? null : checkedAdd(withColor, edgeSegmentBytes);
  if (preparedBytes === null || preparedBytes > payloadCap) {
    return fail(
      "expansion-budget",
      `edge expansion of ${segmentCount} segments (${edgeSegmentBytes} bytes) exceeds the ${payloadCap} byte cap`,
    );
  }

  // ── (b) every position finite, and the ACTUAL bounds measured in the same pass ──
  //
  // Edge points are checked here too: a NaN edge point would slip through a
  // bounds comparison silently, because every comparison against NaN is false.
  const positions = view.positions;
  const measuredMin: [number, number, number] = [Infinity, Infinity, Infinity];
  const measuredMax: [number, number, number] = [-Infinity, -Infinity, -Infinity];
  let pointsSeen = 0;
  const sweep = (points: Float32Array, count: number, what: string): string | null => {
    for (let p = 0; p < count; p++) {
      for (let a = 0; a < 3; a++) {
        const value = points[p * 3 + a];
        if (!Number.isFinite(value)) return `${what} ${p} axis ${a} is ${value}`;
        if (value < measuredMin[a]) measuredMin[a] = value;
        if (value > measuredMax[a]) measuredMax[a] = value;
      }
    }
    pointsSeen += count;
    return null;
  };
  const badPosition = sweep(positions, vertexCount, "vertex");
  if (badPosition) return fail("nonfinite-position", badPosition);
  if (view.edgePositions) {
    const badEdgePoint = sweep(view.edgePositions, edgePointCount, "edge point");
    if (badEdgePoint) return fail("nonfinite-position", badEdgePoint);
  }

  // ── (c) normals finite and already unit length (never renormalised) ──
  const normals = view.normals;
  if (normals) {
    for (let v = 0; v < vertexCount; v++) {
      const x = normals[v * 3];
      const y = normals[v * 3 + 1];
      const z = normals[v * 3 + 2];
      if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) {
        return fail("nonfinite-normal", `normal ${v} is (${x}, ${y}, ${z})`);
      }
      const length = Math.sqrt(x * x + y * y + z * z);
      if (!(length >= NORMAL_LENGTH_MIN && length <= NORMAL_LENGTH_MAX)) {
        return fail("normal-length", `normal ${v} has length ${length}`);
      }
    }
  }

  // ── (d) every index inside the vertex range ──
  const indices = view.indices;
  for (let i = 0; i < indexWords; i++) {
    const index = indices[i];
    if (!Number.isSafeInteger(index) || index < 0 || index >= vertexCount) {
      return fail("index-out-of-range", `index ${i} is ${index}, vertex count is ${vertexCount}`);
    }
  }

  // ── (e) face ranges: ordinal order, contiguous, exactly covering T ──
  const faceRanges = view.faceRanges;
  let triCursor = 0;
  for (let f = 0; f < faceCount; f++) {
    const firstTri = faceRanges[f * 2];
    const triCount = faceRanges[f * 2 + 1];
    if (!Number.isSafeInteger(firstTri) || !Number.isSafeInteger(triCount) || firstTri < 0 || triCount < 0) {
      return fail("face-range", `face ${f} range {${firstTri}, ${triCount}} is not a non-negative integer pair`);
    }
    if (firstTri !== triCursor) {
      return fail("face-range", `face ${f} starts at ${firstTri}, contiguous coverage requires ${triCursor}`);
    }
    const end = checkedAdd(firstTri, triCount);
    if (end === null) {
      return fail("count-overflow", `face ${f} range end overflows safe integer arithmetic`);
    }
    if (end > triangleCount) {
      return fail("face-range", `face ${f} ends at ${end}, beyond triangle count ${triangleCount}`);
    }
    triCursor = end;
  }
  if (triCursor !== triangleCount) {
    return fail("face-range", `face ranges cover ${triCursor} of ${triangleCount} triangles`);
  }

  // ── (f) edge point ranges: the same discipline over P ──
  if (view.hasEdges) {
    const edgeRanges = view.edgeRanges;
    if (!edgeRanges || !view.edgePositions || !view.edgeIdOffsets || !view.edgeIdChars) {
      return fail("edge-range", "HAS_EDGES is set but the edge sections are incomplete");
    }
    let pointCursor = 0;
    for (let e = 0; e < edgeCount; e++) {
      const firstPoint = edgeRanges[e * 2];
      const pointCount = edgeRanges[e * 2 + 1];
      if (!Number.isSafeInteger(firstPoint) || firstPoint < 0) {
        return fail("edge-range", `edge ${e} firstPoint ${firstPoint} is not a non-negative integer`);
      }
      if (firstPoint !== pointCursor) {
        return fail("edge-range", `edge ${e} starts at ${firstPoint}, contiguous coverage requires ${pointCursor}`);
      }
      const end = checkedAdd(firstPoint, pointCount);
      if (end === null) {
        return fail("count-overflow", `edge ${e} range end overflows safe integer arithmetic`);
      }
      if (end > edgePointCount) {
        return fail("edge-range", `edge ${e} ends at ${end}, beyond edge point count ${edgePointCount}`);
      }
      pointCursor = end;
    }
    if (pointCursor !== edgePointCount) {
      return fail("edge-range", `edge ranges cover ${pointCursor} of ${edgePointCount} points`);
    }
  } else if (edgeCount !== 0 || edgePointCount !== 0) {
    return fail(
      "edge-range",
      `edgeCount ${edgeCount} / edgePointCount ${edgePointCount} declared without HAS_EDGES`,
    );
  }

  // ── (g) ID tables, per namespace ──
  const faceIds = validateIdTable(view.faceIdOffsets, view.faceIdChars, faceCount, "face");
  if (faceIds) return fail(faceIds.code, faceIds.detail);
  if (view.hasEdges && view.edgeIdOffsets && view.edgeIdChars) {
    const edgeIds = validateIdTable(view.edgeIdOffsets, view.edgeIdChars, edgeCount, "edge");
    if (edgeIds) return fail(edgeIds.code, edgeIds.detail);
  }

  // ── (h) header bbox: finite and ordered (fatal); enclosure MEASURED, not fatal ──
  //
  // Strict enclosure would false-reject real worker output. `Tessellate.cpp`
  // fills the header box from `BRepBndLib::Add(shape, box)` BEFORE
  // `BRepMesh_IncrementalMesh` runs, so on a RE-tessellation OCCT can derive
  // that box from a coarser triangulation still attached to the shape; the fine
  // nodes then sit outside it by up to the coarse deflection — millimetres,
  // against a 1e-4 slack. A body is not wrong because its producer's advisory
  // box is stale, so the excursion is measured and reported, and the renderer
  // uses the bounds we measured ourselves.
  //
  // MESH1 v2 restores strict enclosure with a DECLARED quantisation allowance
  // (numerics §9.3, WP10); until that field exists there is nothing to check
  // the header box against except the geometry it is supposed to describe.
  const bboxMin = view.bboxMin;
  const bboxMax = view.bboxMax;
  let extent = 0;
  for (let a = 0; a < 3; a++) {
    if (!Number.isFinite(bboxMin[a]) || !Number.isFinite(bboxMax[a])) {
      return fail("bbox", `bbox axis ${a} is [${bboxMin[a]}, ${bboxMax[a]}]`);
    }
    if (bboxMin[a] > bboxMax[a]) {
      return fail("bbox", `bbox axis ${a} is inverted: [${bboxMin[a]}, ${bboxMax[a]}]`);
    }
    extent = Math.max(extent, bboxMax[a] - bboxMin[a]);
  }
  // A mesh with no points has nothing to measure: the header box is the only
  // statement of where the body is, so it stands unchallenged.
  const actualBounds =
    pointsSeen === 0
      ? { min: [bboxMin[0], bboxMin[1], bboxMin[2]] as const, max: [bboxMax[0], bboxMax[1], bboxMax[2]] as const }
      : { min: [measuredMin[0], measuredMin[1], measuredMin[2]] as const, max: [measuredMax[0], measuredMax[1], measuredMax[2]] as const };
  let headerBoundsExcursionMm = 0;
  for (let a = 0; a < 3; a++) {
    headerBoundsExcursionMm = Math.max(
      headerBoundsExcursionMm,
      bboxMin[a] - actualBounds.min[a],
      actualBounds.max[a] - bboxMax[a],
    );
  }

  // ── (i) per-face bboxes, when present ──
  const faceBboxes = view.faceBboxes;
  if (faceBboxes) {
    for (let f = 0; f < faceCount; f++) {
      for (let a = 0; a < 3; a++) {
        const lo = faceBboxes[f * 6 + a];
        const hi = faceBboxes[f * 6 + 3 + a];
        if (!Number.isFinite(lo) || !Number.isFinite(hi)) {
          return fail("face-bbox", `face ${f} bbox axis ${a} is [${lo}, ${hi}]`);
        }
        if (lo > hi) {
          return fail("face-bbox", `face ${f} bbox axis ${a} is inverted: [${lo}, ${hi}]`);
        }
      }
    }
  }

  const geometryBytes = view.faceColors
    ? colorBytes
    : (positionFloats + (view.normals ? positionFloats : 0)) * 4 + indexWords * 4;
  const accounting: MeshAccounting = {
    actualBounds,
    headerBoundsExcursionMm,
    headerBoundsSlackMm: Math.max(BBOX_ABS_SLACK, BBOX_REL_SLACK * extent),
    payloadBytes,
    edgeSegmentBytes,
    colorBytes,
    estimatedGpuBytes: geometryBytes + edgeSegmentBytes,
    triangleCount,
    segmentCount,
  };
  return { ok: true, mesh: { [VALIDATED]: true, view, accounting } };
}

/**
 * Validate a raw view or throw. Only for the lanes that do not go through the
 * ingestion boundary (exact previews, library placement ghosts, fixtures);
 * `MeshIngest` calls {@link validateMeshView} and handles the failure as state.
 */
export function requireValidatedMesh(
  mesh: ValidatedMesh | BodyMeshView,
  bodyId: string,
): ValidatedMesh {
  if (isValidatedMesh(mesh)) return mesh;
  const result = validateMeshView(mesh, bodyId);
  if (!result.ok) throw new MeshValidationError(result.code, result.detail, result.bodyId);
  return result.mesh;
}
