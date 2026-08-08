/*
 * Typed `BodyMeshView` fixtures for tests that need a mesh view WITHOUT going
 * through a real MESH1 blob.
 *
 * Why this exists: `BodyMeshView` is a 24-field interface (parseMeshPayload.ts).
 * Hand-writing an object literal for it in a test compiles under vitest — which
 * does not typecheck — and only fails later in `bun run build`, where `tsc` runs.
 * That is exactly how the WP P0 clean-build break got in. Building the view here,
 * `BodyMeshView`-typed at the seam, makes any field drift a compile error in the
 * fixture instead of a silent structural mock in N test files.
 *
 * PREFER `parseMeshPayload(makeBoxMesh(...))` when the test cares about MESH1
 * framing. Use this builder only when the test needs a shape MESH1 fixtures do
 * not produce — e.g. ElementId-bearing face ids (`idsHaveElementIds`), which
 * `makeBoxMesh` always emits as TopoKeys.
 */
import { FLAG, MESH1_VERSION, type BodyMeshView } from "@/viewport/mesh/parseMeshPayload";
import type { Rgba } from "@/ipc/types";

/** One face of a fixture mesh: its id and the triangles it owns. */
export interface FixtureFace {
  /** Face id — TopoKey (`"f:0"`) or a minted ElementId (`"el_top"`). */
  id: string;
  /** Triangles as index triples into `positions`. */
  triangles: ReadonlyArray<readonly [number, number, number]>;
  /** Authored sRGB color; any face carrying one turns on FACE_COLORS. */
  color?: Rgba;
}

export interface BodyMeshViewFixtureInput {
  /** Flat xyz vertex positions (length 3·V). Defaults to one unit triangle. */
  positions?: readonly number[];
  /** Flat xyz per-vertex normals (length 3·V). Omit for a mesh with no normals. */
  normals?: readonly number[];
  /** Faces in emission order; their triangles concatenate into INDICES. */
  faces?: readonly FixtureFace[];
  /** True when `faces[].id` are persistent ElementIds rather than TopoKeys. */
  idsHaveElementIds?: boolean;
  lod?: number;
}

/** A single unit triangle in the XY plane, +Z normals, one face `f:0`. */
const DEFAULT_POSITIONS: readonly number[] = [0, 0, 0, 1, 0, 0, 0, 1, 0];
const DEFAULT_NORMALS: readonly number[] = [0, 0, 1, 0, 0, 1, 0, 0, 1];
const DEFAULT_FACES: readonly FixtureFace[] = [{ id: "f:0", triangles: [[0, 1, 2]] }];

const UNSET: Rgba = [0, 0, 0, 0];

function bbox(
  positions: readonly number[],
): { min: readonly [number, number, number]; max: readonly [number, number, number] } {
  if (positions.length === 0) {
    return { min: [0, 0, 0], max: [0, 0, 0] };
  }
  const min: [number, number, number] = [Infinity, Infinity, Infinity];
  const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < positions.length; i += 3) {
    for (let k = 0; k < 3; k++) {
      min[k] = Math.min(min[k], positions[i + k]);
      max[k] = Math.max(max[k], positions[i + k]);
    }
  }
  return { min, max };
}

/**
 * Build a fully-populated {@link BodyMeshView} from a small face description.
 *
 * Every derived field (`faceRanges`, `faceIdOffsets`, `faceIdChars`, counts,
 * flags, bbox) is computed here, so a fixture can never disagree with itself the
 * way a hand-written literal can. `buffer`/`blobByteOffset` are present and
 * consistent but carry no MESH1 bytes — nothing downstream of parsing reads them.
 */
export function makeBodyMeshViewFixture(input: BodyMeshViewFixtureInput = {}): BodyMeshView {
  const positions = input.positions ?? DEFAULT_POSITIONS;
  const normals = input.normals ?? (input.positions ? undefined : DEFAULT_NORMALS);
  const faces = input.faces ?? DEFAULT_FACES;

  const indices: number[] = [];
  const faceRanges = new Uint32Array(faces.length * 2);
  const faceIdOffsets = new Uint32Array(faces.length + 1);
  const encoder = new TextEncoder();
  const idBytes: Uint8Array[] = [];
  let idLen = 0;

  faces.forEach((face, f) => {
    faceRanges[f * 2] = indices.length / 3; // firstTri
    faceRanges[f * 2 + 1] = face.triangles.length; // triCount
    for (const t of face.triangles) indices.push(t[0], t[1], t[2]);
    const bytes = encoder.encode(face.id);
    idBytes.push(bytes);
    idLen += bytes.length;
    faceIdOffsets[f + 1] = idLen;
  });

  const faceIdChars = new Uint8Array(idLen);
  let at = 0;
  for (const bytes of idBytes) {
    faceIdChars.set(bytes, at);
    at += bytes.length;
  }

  const hasFaceColors = faces.some((f) => f.color !== undefined);
  let faceColors: Uint8Array | null = null;
  if (hasFaceColors) {
    faceColors = new Uint8Array(faces.length * 4);
    faces.forEach((f, i) => faceColors!.set(f.color ?? UNSET, i * 4));
  }

  const hasNormals = normals !== undefined;
  const { min, max } = bbox(positions);
  const flags =
    (hasNormals ? FLAG.HAS_NORMALS : 0) |
    (input.idsHaveElementIds ? FLAG.IDS_HAVE_ELEMENTIDS : 0) |
    (hasFaceColors ? FLAG.HAS_FACE_COLORS : 0);

  return {
    buffer: new ArrayBuffer(0),
    blobByteOffset: 0,
    version: MESH1_VERSION,
    flags,
    lod: input.lod ?? 0,

    vertexCount: positions.length / 3,
    triangleCount: indices.length / 3,
    faceCount: faces.length,
    edgeCount: 0,
    edgePointCount: 0,

    bboxMin: min,
    bboxMax: max,

    hasNormals,
    hasEdges: false,
    hasFaceBboxes: false,
    idsHaveElementIds: input.idsHaveElementIds ?? false,
    hasFaceColors,

    positions: Float32Array.from(positions),
    indices: Uint32Array.from(indices),
    faceRanges,
    faceIdOffsets,
    faceIdChars,

    normals: hasNormals ? Float32Array.from(normals) : null,
    edgeRanges: null,
    edgePositions: null,
    edgeIdOffsets: null,
    edgeIdChars: null,
    faceBboxes: null,
    faceColors,
  };
}
