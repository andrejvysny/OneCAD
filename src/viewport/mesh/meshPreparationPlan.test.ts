/*
 * VP-HARDENING PR-03A — one immutable plan decides the layout and its price.
 *
 * The arithmetic pinned here is the review's: a body of 1,000,000 triangles
 * over 500,000 vertices with normals is 24,000,000 bytes of geometry when it is
 * prepared indexed and 108,000,000 bytes when a colour forces the de-indexed
 * layout — a factor of 4.5 that used to fall entirely outside admission,
 * because the payload carried no FACE_COLORS section and the body colour lived
 * in the document.
 */
import { describe, it, expect } from "vitest";
import { planMeshPreparation, needsVertexColors } from "./meshPreparationPlan";
import { validateMeshView, type ValidatedMesh } from "./validateMesh";
import { parseMeshPayload, FLAG, MESH1_VERSION, type BodyMeshView } from "./parseMeshPayload";
import { expandEdgeSegments } from "./meshRegistry";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import type { Rgba } from "@/ipc/types";

const BODY = "body_1";
const RED: Rgba = [200, 40, 40, 255];

/** The review's body: 1,000,000 triangles over 500,000 vertices, with normals. */
const V = 500_000;
const T = 1_000_000;

interface SyntheticInput {
  readonly normals: boolean;
  /** Per-face authored colours; alpha 0 means "unset — use the body material". */
  readonly faceColors?: Rgba;
}

/**
 * One MESH1-shaped view whose every section is a ZERO-COPY slice of a single
 * ArrayBuffer — the shape `parseMeshPayload` produces, and the reason
 * `sourceRetainedBytes` is the whole buffer rather than the sum of the slices.
 *
 * Typed `BodyMeshView` at the seam so a field drifting in `parseMeshPayload`
 * breaks this fixture at compile time rather than silently.
 */
function syntheticView({ normals, faceColors }: SyntheticInput): BodyMeshView {
  const positionBytes = V * 3 * 4;
  const indexBytes = T * 3 * 4;
  const faceRangeBytes = 2 * 4;
  const faceIdOffsetBytes = 2 * 4;
  const faceColorBytes = faceColors ? 4 : 0;
  const idChars = new TextEncoder().encode("f:0");

  let cursor = 0;
  const take = (bytes: number): number => {
    const at = cursor;
    cursor += bytes;
    return at;
  };
  const positionsAt = take(positionBytes);
  const normalsAt = normals ? take(positionBytes) : -1;
  const indicesAt = take(indexBytes);
  const faceRangesAt = take(faceRangeBytes);
  const faceIdOffsetsAt = take(faceIdOffsetBytes);
  const faceColorsAt = faceColors ? take(faceColorBytes) : -1;
  const idCharsAt = take(idChars.length);
  const buffer = new ArrayBuffer(cursor + ((4 - (cursor % 4)) % 4));

  const positions = new Float32Array(buffer, positionsAt, V * 3);
  const indices = new Uint32Array(buffer, indicesAt, T * 3);
  const faceRanges = new Uint32Array(buffer, faceRangesAt, 2);
  faceRanges[0] = 0;
  faceRanges[1] = T;
  const faceIdOffsets = new Uint32Array(buffer, faceIdOffsetsAt, 2);
  faceIdOffsets[0] = 0;
  faceIdOffsets[1] = idChars.length;
  const faceIdChars = new Uint8Array(buffer, idCharsAt, idChars.length);
  faceIdChars.set(idChars);

  let normalArray: Float32Array | null = null;
  if (normals) {
    normalArray = new Float32Array(buffer, normalsAt, V * 3);
    // Unit length is a validation requirement, never renormalised downstream.
    for (let v = 0; v < V; v++) normalArray[v * 3 + 2] = 1;
  }
  let colorArray: Uint8Array | null = null;
  if (faceColors) {
    colorArray = new Uint8Array(buffer, faceColorsAt, 4);
    colorArray.set(faceColors);
  }

  return {
    buffer,
    blobByteOffset: 0,
    version: MESH1_VERSION,
    flags: (normals ? FLAG.HAS_NORMALS : 0) | (faceColors ? FLAG.HAS_FACE_COLORS : 0),
    lod: 2,
    vertexCount: V,
    triangleCount: T,
    faceCount: 1,
    edgeCount: 0,
    edgePointCount: 0,
    bboxMin: [0, 0, 0],
    bboxMax: [0, 0, 0],
    hasNormals: normals,
    hasEdges: false,
    hasFaceBboxes: false,
    idsHaveElementIds: false,
    hasFaceColors: faceColors !== undefined,
    positions,
    indices,
    faceRanges,
    faceIdOffsets,
    faceIdChars,
    normals: normalArray,
    edgeRanges: null,
    edgePositions: null,
    edgeIdOffsets: null,
    edgeIdChars: null,
    faceBboxes: null,
    faceColors: colorArray,
  };
}

function validated(input: SyntheticInput): ValidatedMesh {
  const result = validateMeshView(syntheticView(input), BODY);
  if (!result.ok) throw new Error(`the synthetic mesh must validate: ${result.code} ${result.detail}`);
  return result.mesh;
}

function planned(mesh: ValidatedMesh, appearance = {}, budgets?: { singleMeshPayloadBytes: number }) {
  const result = planMeshPreparation(mesh, appearance, BODY, budgets);
  if (!result.ok) throw new Error(`unexpected refusal: ${result.code} ${result.detail}`);
  return result.plan;
}

describe("planMeshPreparation — the review's arithmetic", () => {
  it("prices an uncoloured body as 24,000,000 bytes of INDEXED geometry", () => {
    const mesh = validated({ normals: true });
    const plan = planned(mesh);

    expect(plan.layout).toBe("indexed");
    expect(plan.vertexCount).toBe(V);
    expect(plan.positionBytes).toBe(6_000_000);
    expect(plan.normalBytes).toBe(6_000_000);
    expect(plan.colorBytes).toBe(0);
    expect(plan.indexBytes).toBe(12_000_000);
    expect(plan.edgeExpansionBytes).toBe(0);
    expect(plan.gpuBytes).toBe(24_000_000);
    // Indexed preparation materialises nothing: the attributes ARE the payload,
    // so the only CPU cost is the buffer those zero-copy views keep alive.
    expect(plan.sourceRetainedBytes).toBe(mesh.view.buffer.byteLength);
    expect(plan.cpuBytes).toBe(plan.sourceRetainedBytes);
  });

  it("prices a METADATA-ONLY body colour as 108,000,000 bytes of DE-INDEXED geometry", () => {
    const mesh = validated({ normals: true });
    // The payload has no FACE_COLORS section whatsoever — this colour is
    // document metadata, which is exactly the case that used to be mispriced.
    expect(mesh.view.faceColors).toBeNull();
    const plan = planned(mesh, { bodyColor: RED });

    expect(plan.layout).toBe("deindexed");
    expect(plan.vertexCount).toBe(3 * T);
    expect(plan.positionBytes).toBe(36_000_000);
    expect(plan.normalBytes).toBe(36_000_000);
    expect(plan.colorBytes).toBe(36_000_000);
    expect(plan.indexBytes).toBe(0);
    expect(plan.gpuBytes).toBe(108_000_000);
    // De-indexing materialises all three attributes AND still reads from the
    // source, so the source stays charged alongside them.
    expect(plan.cpuBytes).toBe(plan.sourceRetainedBytes + 108_000_000);
    expect(plan.gpuBytes).toBe(4.5 * planned(mesh).gpuBytes);
  });

  it("prices an authored face-colour map with a single entry as de-indexed", () => {
    const mesh = validated({ normals: true });
    const plan = planned(mesh, { authoredFaceColors: new Map([["f:0", RED]]) });
    expect(plan.layout).toBe("deindexed");
    expect(plan.gpuBytes).toBe(108_000_000);
  });

  it("prices a payload FACE_COLORS section as de-indexed regardless of appearance", () => {
    const mesh = validated({ normals: true, faceColors: RED });
    expect(planned(mesh).layout).toBe("deindexed");
    expect(planned(mesh, { bodyColor: RED }).layout).toBe("deindexed");
  });

  it("leaves an EMPTY appearance indexed — an unset section and an empty map are not colours", () => {
    // Alpha 0 is "unset — use the body material" (mesh_format.md §4 type 12):
    // the section exists, no face is actually coloured, and the layout must not
    // pay for a colour stream nothing would write to.
    const unset = validated({ normals: true, faceColors: [9, 9, 9, 0] });
    expect(unset.view.faceColors).not.toBeNull();
    expect(planned(unset).layout).toBe("indexed");
    expect(planned(unset, { authoredFaceColors: new Map() }).layout).toBe("indexed");
  });

  it("never prices normals a payload does not carry", () => {
    const mesh = validated({ normals: false });
    expect(planned(mesh).normalBytes).toBe(0);
    expect(planned(mesh).gpuBytes).toBe(6_000_000 + 12_000_000);
    const coloured = planned(mesh, { bodyColor: RED });
    expect(coloured.normalBytes).toBe(0);
    expect(coloured.gpuBytes).toBe(36_000_000 + 36_000_000);
  });

  it("refuses a preparation that exceeds the single-mesh cap, instead of pricing it wrong", () => {
    const mesh = validated({ normals: true });
    // The payload itself fits; the de-indexed expansion is what does not.
    const cap = 30_000_000;
    expect(planMeshPreparation(mesh, {}, BODY, { singleMeshPayloadBytes: cap }).ok).toBe(true);
    const refused = planMeshPreparation(mesh, { bodyColor: RED }, BODY, { singleMeshPayloadBytes: cap });
    expect(refused.ok).toBe(false);
    if (refused.ok) return;
    expect(refused.code).toBe("payload-budget");
    expect(refused.bodyId).toBe(BODY);
    expect(refused.detail).toContain("deindexed");
  });
});

describe("planMeshPreparation — against a real MESH1 payload", () => {
  const box = () => {
    const result = validateMeshView(parseMeshPayload(makeBoxMesh()), BODY);
    if (!result.ok) throw new Error("the mock box must validate");
    return result.mesh;
  };

  it("prices the edge expansion as the exact buffer expandEdgeSegments allocates", () => {
    const mesh = box();
    const view = mesh.view;
    const expanded = expandEdgeSegments(view.edgePositions!, view.edgeRanges!, view.edgeCount);
    expect(planned(mesh).edgeExpansionBytes).toBe(expanded.positions.byteLength);
  });

  it("charges the WHOLE blob once, not the sum of its zero-copy slices", () => {
    const mesh = box();
    const view = mesh.view;
    const slices =
      view.positions.byteLength + view.indices.byteLength + (view.normals?.byteLength ?? 0);
    const plan = planned(mesh);
    expect(plan.sourceRetainedBytes).toBe(view.buffer.byteLength);
    expect(plan.sourceRetainedBytes).toBeGreaterThan(slices); // header, table, tables, padding
  });

  it("keeps the edge expansion in BOTH layouts — colour changes faces, not edges", () => {
    const mesh = box();
    const indexed = planned(mesh);
    const deindexed = planned(mesh, { bodyColor: RED });
    expect(deindexed.edgeExpansionBytes).toBe(indexed.edgeExpansionBytes);
    expect(deindexed.cpuBytes - indexed.cpuBytes).toBe(
      deindexed.positionBytes + deindexed.normalBytes + deindexed.colorBytes,
    );
  });
});

describe("needsVertexColors — the single layout predicate", () => {
  it("answers from any one of the three independent colour sources", () => {
    const plain = syntheticView({ normals: true });
    expect(needsVertexColors(plain)).toBe(false);
    expect(needsVertexColors(plain, RED)).toBe(true);
    expect(needsVertexColors(plain, undefined, new Map([["f:0", RED]]))).toBe(true);
    expect(needsVertexColors(syntheticView({ normals: true, faceColors: RED }))).toBe(true);
  });
});
