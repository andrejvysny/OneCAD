/*
 * Synthesize REAL MESH1 binary blobs byte-by-byte (protocol/mesh_format.md).
 *
 * These are the exact bytes a C++ worker would emit, so the mock drives the full
 * parse → registry → scene → pick pipeline with no backend. `encodeMesh1` is the
 * shared DataView writer; `makeBoxMesh` / `makeCylinderMesh` build the two demo
 * bodies. The writer lays present sections in ascending `type` order (== ascending
 * offset), each 4-byte aligned — matching the worked example in §5 verbatim.
 */
import { FLAG, SEC, parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { prismLocal, type PrismProfile } from "@/tools/preview/prismPreview";
import { latheLocal, type LatheAxis } from "@/tools/preview/lathePreview";
import {
  applyPlacementToNormal,
  applyPlacementToPoint,
  type Mat4Rows,
} from "@/tools/preview/patternPreview";
import type { Vec3 } from "@/tools/preview/depthProjection";
import type { SketchPlane } from "./types";

const HEADER_BYTES = 64;
const TABLE_ENTRY_BYTES = 16;

const align4 = (n: number): number => (n + 3) & ~3;

/** Authored sRGB `{r,g,b,a}`, 0–255 — appearance DATA (mesh_format.md §4 type 12). */
export type FaceColor = readonly [number, number, number, number];

export interface FaceSource {
  /** Triangles as triples of indices into `positions` (grouped under this face). */
  triangles: ReadonlyArray<readonly [number, number, number]>;
  /** Face id — TopoKey (`"f:0"`) or minted ElementId. */
  id: string;
  /**
   * Optional authored color. Any face carrying one turns on HAS_FACE_COLORS and
   * emits FACE_COLORS for the WHOLE mesh; faces without one encode `{0,0,0,0}`
   * (unset ⇒ the renderer falls back to the body material).
   */
  color?: FaceColor;
}

export interface EdgeSource {
  /** Polyline points (world xyz); consecutive points form segments. */
  points: ReadonlyArray<readonly [number, number, number]>;
  id: string;
}

export interface Mesh1Source {
  /** Vertex positions, flat xyz (length 3·V). */
  positions: number[];
  /** Optional per-vertex smoothed normals, flat xyz (length 3·V). */
  normals?: number[];
  /** Faces in emission order; their triangles are concatenated into INDICES. */
  faces: FaceSource[];
  edges?: EdgeSource[];
  lod?: number;
  /** Override bbox; otherwise computed from `positions`. */
  bbox?: { min: [number, number, number]; max: [number, number, number] };
  /** Set IDS_HAVE_ELEMENTIDS (ids are minted ElementIds, not pure TopoKeys). */
  idsHaveElementIds?: boolean;
  /** Emit the optional FACE_BBOXES section (computed per face). */
  faceBboxes?: boolean;
}

interface SectionBytes {
  type: number;
  bytes: Uint8Array;
}

const utf8 = new TextEncoder();

/** Encode a Mesh1Source into an exact MESH1 ArrayBuffer. */
export function encodeMesh1(src: Mesh1Source): ArrayBuffer {
  const V = src.positions.length / 3;
  if (!Number.isInteger(V)) throw new Error("positions length not a multiple of 3");

  // ── INDICES (grouped by face) + FACE_RANGES ──
  const indices: number[] = [];
  const faceRanges: number[] = []; // firstTri, triCount per face
  for (const face of src.faces) {
    const firstTri = indices.length / 3;
    for (const [a, b, c] of face.triangles) indices.push(a, b, c);
    faceRanges.push(firstTri, face.triangles.length);
  }
  const T = indices.length / 3;
  const F = src.faces.length;

  // ── FACE_ID_OFFS / FACE_ID_CHARS ──
  const faceIdBytes = src.faces.map((f) => utf8.encode(f.id));
  const faceIdOffs: number[] = [0];
  for (const b of faceIdBytes) faceIdOffs.push(faceIdOffs[faceIdOffs.length - 1] + b.length);
  const faceChars = concatBytes(faceIdBytes, faceIdOffs[F]);

  // ── EDGES ──
  const edges = src.edges ?? [];
  const E = edges.length;
  const edgePositions: number[] = [];
  const edgeRanges: number[] = []; // firstPoint, pointCount per edge
  for (const edge of edges) {
    const firstPoint = edgePositions.length / 3;
    for (const [x, y, z] of edge.points) edgePositions.push(x, y, z);
    edgeRanges.push(firstPoint, edge.points.length);
  }
  const P = edgePositions.length / 3;
  const edgeIdBytes = edges.map((e) => utf8.encode(e.id));
  const edgeIdOffs: number[] = [0];
  for (const b of edgeIdBytes) edgeIdOffs.push(edgeIdOffs[edgeIdOffs.length - 1] + b.length);
  const edgeChars = concatBytes(edgeIdBytes, edgeIdOffs[E]);

  const hasNormals = src.normals !== undefined;
  const hasEdges = E > 0;
  const hasFaceBboxes = src.faceBboxes === true;
  const hasFaceColors = src.faces.some((f) => f.color !== undefined);

  let flags = 0;
  if (hasNormals) flags |= FLAG.HAS_NORMALS;
  if (hasEdges) flags |= FLAG.HAS_EDGES;
  if (hasFaceBboxes) flags |= FLAG.HAS_FACE_BBOXES;
  if (src.idsHaveElementIds) flags |= FLAG.IDS_HAVE_ELEMENTIDS;
  if (hasFaceColors) flags |= FLAG.HAS_FACE_COLORS;

  // ── Section byte payloads, in ascending type order ──
  const sections: SectionBytes[] = [];
  sections.push({ type: SEC.POSITIONS, bytes: f32Bytes(src.positions) });
  if (hasNormals) sections.push({ type: SEC.NORMALS, bytes: f32Bytes(src.normals!) });
  sections.push({ type: SEC.INDICES, bytes: u32Bytes(indices) });
  sections.push({ type: SEC.FACE_RANGES, bytes: u32Bytes(faceRanges) });
  sections.push({ type: SEC.FACE_ID_OFFS, bytes: u32Bytes(faceIdOffs) });
  sections.push({ type: SEC.FACE_ID_CHARS, bytes: faceChars });
  if (hasEdges) {
    sections.push({ type: SEC.EDGE_RANGES, bytes: u32Bytes(edgeRanges) });
    sections.push({ type: SEC.EDGE_POSITIONS, bytes: f32Bytes(edgePositions) });
    sections.push({ type: SEC.EDGE_ID_OFFS, bytes: u32Bytes(edgeIdOffs) });
    sections.push({ type: SEC.EDGE_ID_CHARS, bytes: edgeChars });
  }
  if (hasFaceBboxes) {
    sections.push({ type: SEC.FACE_BBOXES, bytes: f32Bytes(computeFaceBboxes(src.positions, src.faces)) });
  }
  if (hasFaceColors) {
    sections.push({ type: SEC.FACE_COLORS, bytes: faceColorBytes(src.faces) });
  }

  // ── Lay sections out with 4-byte alignment ──
  const tableEnd = HEADER_BYTES + sections.length * TABLE_ENTRY_BYTES;
  let cursor = tableEnd;
  const placed = sections.map((s) => {
    const offset = align4(cursor);
    cursor = offset + s.bytes.length;
    return { ...s, offset };
  });
  const totalLen = align4(cursor);

  const buffer = new ArrayBuffer(totalLen);
  const dv = new DataView(buffer);
  const u8 = new Uint8Array(buffer);

  // Header (64 B).
  dv.setUint32(0x00, 0x4d455348, true); // magic
  dv.setUint16(0x04, 1, true); // version
  dv.setUint16(0x06, flags, true);
  dv.setUint32(0x08, V, true);
  dv.setUint32(0x0c, T, true);
  dv.setUint32(0x10, F, true);
  dv.setUint32(0x14, E, true);
  dv.setUint32(0x18, P, true);
  dv.setUint16(0x1c, src.lod ?? 0, true);
  dv.setUint16(0x1e, sections.length, true);
  const bbox = src.bbox ?? computeBbox(src.positions);
  dv.setFloat32(0x20, bbox.min[0], true);
  dv.setFloat32(0x24, bbox.min[1], true);
  dv.setFloat32(0x28, bbox.min[2], true);
  dv.setFloat32(0x2c, bbox.max[0], true);
  dv.setFloat32(0x30, bbox.max[1], true);
  dv.setFloat32(0x34, bbox.max[2], true);
  // reserved0/1 already zero.

  // Section table (16 B each, sorted by offset == type order here).
  placed.forEach((s, i) => {
    const base = HEADER_BYTES + i * TABLE_ENTRY_BYTES;
    dv.setUint32(base + 0x00, s.type, true);
    dv.setUint32(base + 0x04, 0, true); // pad
    dv.setUint32(base + 0x08, s.offset, true);
    dv.setUint32(base + 0x0c, s.bytes.length, true);
  });

  // Section data.
  for (const s of placed) u8.set(s.bytes, s.offset);

  return buffer;
}

// ── byte helpers ────────────────────────────────────────────────────────────

function f32Bytes(values: number[]): Uint8Array {
  return new Uint8Array(Float32Array.from(values).buffer);
}
function u32Bytes(values: number[]): Uint8Array {
  return new Uint8Array(Uint32Array.from(values).buffer);
}
function concatBytes(parts: Uint8Array[], total: number): Uint8Array {
  const out = new Uint8Array(total);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}
function computeBbox(positions: number[]): { min: [number, number, number]; max: [number, number, number] } {
  const min: [number, number, number] = [Infinity, Infinity, Infinity];
  const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < positions.length; i += 3) {
    for (let a = 0; a < 3; a++) {
      const v = positions[i + a];
      if (v < min[a]) min[a] = v;
      if (v > max[a]) max[a] = v;
    }
  }
  return { min, max };
}
/** 4·F u8 sRGB RGBA in face order; a face with no authored color stays `{0,0,0,0}`. */
function faceColorBytes(faces: FaceSource[]): Uint8Array {
  const out = new Uint8Array(faces.length * 4);
  faces.forEach((f, i) => {
    if (f.color) out.set(f.color, i * 4);
  });
  return out;
}
function computeFaceBboxes(positions: number[], faces: FaceSource[]): number[] {
  const out: number[] = [];
  for (const face of faces) {
    const min = [Infinity, Infinity, Infinity];
    const max = [-Infinity, -Infinity, -Infinity];
    for (const tri of face.triangles) {
      for (const vi of tri) {
        for (let a = 0; a < 3; a++) {
          const v = positions[vi * 3 + a];
          if (v < min[a]) min[a] = v;
          if (v > max[a]) max[a] = v;
        }
      }
    }
    out.push(min[0], min[1], min[2], max[0], max[1], max[2]);
  }
  return out;
}

// ── Box (80×60×30, centred at origin — matches the retired demo box) ─────────
//
// The corner table + face table below are the SINGLE source of truth for the mock
// box's topology. `mockFaceGeometry.ts` derives each face's sketch plane and
// projected boundary from the SAME tables, so the analytic face data and the
// rendered triangles can never drift apart (exporting the corners alone would
// still have left two hand-written corner lists to keep in step).

/** The eight box corners, keyed `"xyz"` with `1` = the POSITIVE half-space. */
export type BoxCornerKey = "000" | "100" | "110" | "010" | "001" | "101" | "111" | "011";

/** Default mock box dimensions (mm) — the seed body every mock lane renders. */
export const BOX_SIZE: readonly [number, number, number] = [80, 60, 30];

/** Corner coordinates for a box of `size`, centred at the origin. */
export function boxCorners(
  size: readonly [number, number, number] = BOX_SIZE,
): Record<BoxCornerKey, [number, number, number]> {
  const [hx, hy, hz] = [size[0] / 2, size[1] / 2, size[2] / 2];
  return {
    "000": [-hx, -hy, -hz], "100": [hx, -hy, -hz], "110": [hx, hy, -hz], "010": [-hx, hy, -hz],
    "001": [-hx, -hy, hz], "101": [hx, -hy, hz], "111": [hx, hy, hz], "011": [-hx, hy, hz],
  };
}

/** The six box faces `f:0..f:5`: outward normal + corner ring (CCW seen from
 *  outside — the winding the crease-split triangles are emitted in). */
export const BOX_FACES: readonly {
  id: string;
  normal: [number, number, number];
  corners: readonly [BoxCornerKey, BoxCornerKey, BoxCornerKey, BoxCornerKey];
}[] = [
  { id: "f:0", normal: [1, 0, 0], corners: ["100", "110", "111", "101"] },
  { id: "f:1", normal: [-1, 0, 0], corners: ["010", "000", "001", "011"] },
  { id: "f:2", normal: [0, 1, 0], corners: ["110", "010", "011", "111"] },
  { id: "f:3", normal: [0, -1, 0], corners: ["000", "100", "101", "001"] },
  { id: "f:4", normal: [0, 0, 1], corners: ["001", "101", "111", "011"] },
  { id: "f:5", normal: [0, 0, -1], corners: ["100", "000", "010", "110"] },
];

/** The twelve box edges `e:0..e:11` as corner pairs (bottom, top, verticals). */
export const BOX_EDGE_PAIRS: readonly [BoxCornerKey, BoxCornerKey][] = [
  ["000", "100"], ["100", "110"], ["110", "010"], ["010", "000"], // bottom
  ["001", "101"], ["101", "111"], ["111", "011"], ["011", "001"], // top
  ["000", "001"], ["100", "101"], ["110", "111"], ["010", "011"], // verticals
];

/**
 * Six crease-split faces `f:0..f:5`, twelve edges `e:0..e:11`.
 *
 * `origin` translates the whole box in world space (default: centred at the
 * origin, the seed body every mock lane renders). It exists so a fabricated body
 * — the STEP-import stand-in — can sit visibly clear of the seed box instead of
 * being buried inside it. `boxCorners`/`mockFaceGeometry` keep the un-translated
 * table, so the analytic face data still describes the DEFAULT box only.
 *
 * `faceColors` (indexed by {@link BOX_FACES}) stands in for a STEP file's
 * authored appearance: a `null`/absent entry leaves that face unset, so one box
 * can exercise both the colored and the fallback path.
 */
export function makeBoxMesh(
  sizeX = BOX_SIZE[0],
  sizeY = BOX_SIZE[1],
  sizeZ = BOX_SIZE[2],
  lod = 0,
  origin: readonly [number, number, number] = [0, 0, 0],
  faceColors?: ReadonlyArray<FaceColor | null>,
): ArrayBuffer {
  const centred = boxCorners([sizeX, sizeY, sizeZ]);
  const c = Object.fromEntries(
    Object.entries(centred).map(([key, p]) => [
      key,
      [p[0] + origin[0], p[1] + origin[1], p[2] + origin[2]] as [number, number, number],
    ]),
  ) as Record<BoxCornerKey, [number, number, number]>;

  const positions: number[] = [];
  const normals: number[] = [];
  const faces: FaceSource[] = [];

  // Each face: 4 own vertices (crease split) + 2 triangles, normal = face dir.
  BOX_FACES.forEach((face, i) => {
    const base = positions.length / 3;
    for (const key of face.corners) {
      positions.push(c[key][0], c[key][1], c[key][2]);
      normals.push(face.normal[0], face.normal[1], face.normal[2]);
    }
    faces.push({
      triangles: [
        [base, base + 1, base + 2],
        [base, base + 2, base + 3],
      ],
      id: face.id,
      color: faceColors?.[i] ?? undefined,
    });
  });

  const edges: EdgeSource[] = BOX_EDGE_PAIRS.map(([a, b], i) => ({
    points: [c[a], c[b]],
    id: `e:${i}`,
  }));

  return encodeMesh1({ positions, normals, faces, edges, lod });
}

// ── Cylinder (radial segments; side face + 2 caps) ───────────────────────────

/** Side face `f:0` + top cap `f:1` + bottom cap `f:2`; top/bottom circle + seam edges. */
export function makeCylinderMesh(radius = 25, height = 60, segments = 24, lod = 0): ArrayBuffer {
  const zTop = height / 2;
  const zBot = -height / 2;
  const positions: number[] = [];
  const normals: number[] = [];
  const faces: FaceSource[] = [];

  const push = (x: number, y: number, z: number, nx: number, ny: number, nz: number): number => {
    const idx = positions.length / 3;
    positions.push(x, y, z);
    normals.push(nx, ny, nz);
    return idx;
  };
  const ang = (i: number) => (i / segments) * Math.PI * 2;

  // Side face: (segments+1) bottom + top verts (seam duplicated) with radial normals.
  const sideBot: number[] = [];
  const sideTop: number[] = [];
  for (let i = 0; i <= segments; i++) {
    const a = ang(i);
    const cx = Math.cos(a);
    const cy = Math.sin(a);
    sideBot.push(push(radius * cx, radius * cy, zBot, cx, cy, 0));
    sideTop.push(push(radius * cx, radius * cy, zTop, cx, cy, 0));
  }
  const sideTris: [number, number, number][] = [];
  for (let i = 0; i < segments; i++) {
    sideTris.push([sideBot[i], sideBot[i + 1], sideTop[i + 1]]);
    sideTris.push([sideBot[i], sideTop[i + 1], sideTop[i]]);
  }
  faces.push({ triangles: sideTris, id: "f:0" });

  // Top cap (normal +Z): fan around a centre vertex.
  const topCenter = push(0, 0, zTop, 0, 0, 1);
  const topRing: number[] = [];
  for (let i = 0; i < segments; i++) {
    const a = ang(i);
    topRing.push(push(radius * Math.cos(a), radius * Math.sin(a), zTop, 0, 0, 1));
  }
  const topTris: [number, number, number][] = [];
  for (let i = 0; i < segments; i++) {
    topTris.push([topCenter, topRing[i], topRing[(i + 1) % segments]]);
  }
  faces.push({ triangles: topTris, id: "f:1" });

  // Bottom cap (normal -Z): fan wound the other way.
  const botCenter = push(0, 0, zBot, 0, 0, -1);
  const botRing: number[] = [];
  for (let i = 0; i < segments; i++) {
    const a = ang(i);
    botRing.push(push(radius * Math.cos(a), radius * Math.sin(a), zBot, 0, 0, -1));
  }
  const botTris: [number, number, number][] = [];
  for (let i = 0; i < segments; i++) {
    botTris.push([botCenter, botRing[(i + 1) % segments], botRing[i]]);
  }
  faces.push({ triangles: botTris, id: "f:2" });

  // Edges: top circle, bottom circle (closed polylines), one vertical seam.
  const circle = (z: number): [number, number, number][] => {
    const pts: [number, number, number][] = [];
    for (let i = 0; i <= segments; i++) {
      const a = ang(i);
      pts.push([radius * Math.cos(a), radius * Math.sin(a), z]);
    }
    return pts;
  };
  const edges: EdgeSource[] = [
    { points: circle(zTop), id: "e:0" },
    { points: circle(zBot), id: "e:1" },
    { points: [ [radius, 0, zBot], [radius, 0, zTop] ], id: "e:2" },
  ];

  return encodeMesh1({ positions, normals, faces, edges, lod });
}

// ── Extrude body (prism from a sketch region × depth) — the mock L2 body ──────

/**
 * Synthesize the exact extrude body: lift a region profile into a prism (shared
 * plane-local topology from prismPreview), transform to WORLD via the sketch
 * plane basis, and encode as MESH1. Faces `f:0` (bottom) / `f:1` (top) / `f:2`
 * (sides); edges are the top/bottom boundary loops + verticals. Sized by the
 * region (u,v) bbox × `depth`, so the emitted mesh's bbox scales with the drag —
 * exactly what the 60fps gate asserts against the final params.
 *
 * MOCK LIMIT: booleanMode (Add/Cut/Intersect) does not actually fuse/subtract
 * against the target body — the mock always emits the fresh prism as its own body
 * (the real fusion is OCCT's job in F-WP8).
 */
export function makeExtrudeBodyMesh(
  profile: PrismProfile,
  plane: SketchPlane,
  depth: number,
  lod = 0,
): ArrayBuffer {
  const d = Math.abs(depth) < 1e-4 ? (depth < 0 ? -1e-4 : 1e-4) : depth;
  const local = prismLocal(profile, d);
  const [ox, oy, oz] = plane.origin;
  const [xx, xy, xz] = plane.xAxis;
  const [yx, yy, yz] = plane.yAxis;
  const [nx, ny, nz] = plane.normal;

  const P = local.positions.length / 3;
  const worldPositions: number[] = new Array(P * 3);
  const worldNormals: number[] = new Array(P * 3);
  for (let i = 0; i < P; i++) {
    const u = local.positions[i * 3];
    const v = local.positions[i * 3 + 1];
    const w = local.positions[i * 3 + 2];
    worldPositions[i * 3] = ox + u * xx + v * yx + w * nx;
    worldPositions[i * 3 + 1] = oy + u * xy + v * yy + w * ny;
    worldPositions[i * 3 + 2] = oz + u * xz + v * yz + w * nz;
    const lx = local.normals[i * 3];
    const ly = local.normals[i * 3 + 1];
    const lz = local.normals[i * 3 + 2];
    worldNormals[i * 3] = lx * xx + ly * yx + lz * nx;
    worldNormals[i * 3 + 1] = lx * xy + ly * yy + lz * ny;
    worldNormals[i * 3 + 2] = lx * xz + ly * yz + lz * nz;
  }

  const faces: FaceSource[] = local.faces.map((f, i) => ({ triangles: f.triangles, id: `f:${i}` }));
  const edges: EdgeSource[] = local.edges.map((loop, i) => ({
    points: loop.map(
      (vi) => [worldPositions[vi * 3], worldPositions[vi * 3 + 1], worldPositions[vi * 3 + 2]] as [number, number, number],
    ),
    id: `e:${i}`,
  }));

  return encodeMesh1({ positions: worldPositions, normals: worldNormals, faces, edges, lod });
}

// ── Naive mesh concat (mock boolean Add) ──────────────────────────────────────

/**
 * Merge two MESH1 blobs into one (Wave 2 mock boolean Add). This is a NAIVE
 * visual concat — the two solids' vertices/faces/edges are placed side by side
 * with no CSG (no shared-boundary removal). It exists only so the mock's "Add"
 * body visibly grows; the e2e specs assert history rows / body-diff, never CSG.
 * Face/edge ids are namespaced (`a:`/`b:`) so they stay unique in the merged mesh.
 */
export function concatMesh1(a: ArrayBuffer, b: ArrayBuffer): ArrayBuffer {
  const va = parseMeshPayload(a);
  const vb = parseMeshPayload(b);

  const positions = [...va.positions, ...vb.positions];
  const bothHaveNormals = va.normals !== null && vb.normals !== null;
  const normals = bothHaveNormals ? [...va.normals!, ...vb.normals!] : undefined;

  const faces: FaceSource[] = [];
  const pushFaces = (
    v: ReturnType<typeof parseMeshPayload>,
    base: number,
    prefix: string,
  ): void => {
    for (let f = 0; f < v.faceCount; f++) {
      const firstTri = v.faceRanges[f * 2];
      const triCount = v.faceRanges[f * 2 + 1];
      const triangles: [number, number, number][] = [];
      for (let t = 0; t < triCount; t++) {
        const i = (firstTri + t) * 3;
        triangles.push([v.indices[i] + base, v.indices[i + 1] + base, v.indices[i + 2] + base]);
      }
      faces.push({ triangles, id: `${prefix}f:${f}` });
    }
  };
  pushFaces(va, 0, "a:");
  pushFaces(vb, va.vertexCount, "b:");

  const edges: EdgeSource[] = [];
  const pushEdges = (v: ReturnType<typeof parseMeshPayload>, prefix: string): void => {
    if (!v.edgeRanges || !v.edgePositions) return;
    for (let e = 0; e < v.edgeCount; e++) {
      const firstPoint = v.edgeRanges[e * 2];
      const pointCount = v.edgeRanges[e * 2 + 1];
      const points: [number, number, number][] = [];
      for (let p = 0; p < pointCount; p++) {
        const pi = (firstPoint + p) * 3;
        points.push([v.edgePositions[pi], v.edgePositions[pi + 1], v.edgePositions[pi + 2]]);
      }
      edges.push({ points, id: `${prefix}e:${e}` });
    }
  };
  pushEdges(va, "a:");
  pushEdges(vb, "b:");

  return encodeMesh1({ positions, normals, faces, edges });
}

// ── Rigid placement (mock TransformBody) ─────────────────────────────────────

/**
 * Rewrite a MESH1 blob under a rigid placement — the mock lane's `TransformBody`.
 *
 * Unlike the mock's other body ops this is NOT a stand-in: a rigid transform of a
 * tessellation IS the exact answer (the kernel moves the same points), so the
 * mock's geometry and the worker's agree up to float rounding. The matrix comes
 * from the SAME `placementMatrix` the ghost preview uses, which is what makes the
 * preview and the committed body identical rather than merely similar.
 *
 * Positions and EDGE polyline points are world points and take the full `T ∘ R`;
 * normals take the rotation only. Everything else is re-derived by `encodeMesh1`
 * from the moved data — header bbox, the optional FACE_BBOXES section, and the
 * section table — so no stale extent can survive (a stale header bbox silently
 * breaks zoom-to-fit and the picker's broad phase).
 */
export function transformMesh1(blob: ArrayBuffer, m: Mat4Rows): ArrayBuffer {
  const v = parseMeshPayload(blob);
  const positions = mapTriples(v.positions, (p) => applyPlacementToPoint(m, p));
  const normals = v.normals ? mapTriples(v.normals, (n) => applyPlacementToNormal(m, n)) : undefined;

  const faceIds = decodeIds(v.faceIdChars, v.faceIdOffsets, v.faceCount);
  const faces: FaceSource[] = [];
  for (let f = 0; f < v.faceCount; f++) {
    const firstTri = v.faceRanges[f * 2];
    const triCount = v.faceRanges[f * 2 + 1];
    const triangles: [number, number, number][] = [];
    for (let t = 0; t < triCount; t++) {
      const i = (firstTri + t) * 3;
      triangles.push([v.indices[i], v.indices[i + 1], v.indices[i + 2]]);
    }
    const color = faceColorAt(v.faceColors, f);
    faces.push(color ? { triangles, id: faceIds[f], color } : { triangles, id: faceIds[f] });
  }

  const edges: EdgeSource[] = [];
  if (v.edgeRanges && v.edgePositions && v.edgeIdChars && v.edgeIdOffsets) {
    const edgeIds = decodeIds(v.edgeIdChars, v.edgeIdOffsets, v.edgeCount);
    for (let e = 0; e < v.edgeCount; e++) {
      const firstPoint = v.edgeRanges[e * 2];
      const pointCount = v.edgeRanges[e * 2 + 1];
      const points: [number, number, number][] = [];
      for (let p = 0; p < pointCount; p++) {
        const i = (firstPoint + p) * 3;
        points.push(applyPlacementToPoint(m, [v.edgePositions[i], v.edgePositions[i + 1], v.edgePositions[i + 2]]));
      }
      edges.push({ points, id: edgeIds[e] });
    }
  }

  return encodeMesh1({
    positions,
    normals,
    faces,
    edges,
    lod: v.lod,
    idsHaveElementIds: v.idsHaveElementIds,
    faceBboxes: v.hasFaceBboxes,
  });
}

/** Apply `fn` to each xyz triple of a flat buffer, returning a flat number[]. */
function mapTriples(src: Float32Array, fn: (p: Vec3) => Vec3): number[] {
  const out = new Array<number>(src.length);
  for (let i = 0; i < src.length; i += 3) {
    const [x, y, z] = fn([src[i], src[i + 1], src[i + 2]]);
    out[i] = x;
    out[i + 1] = y;
    out[i + 2] = z;
  }
  return out;
}

/** Decode `count` UTF-8 ids out of a chars blob + its prefix-sum offsets. */
function decodeIds(chars: Uint8Array, offsets: Uint32Array, count: number): string[] {
  const dec = new TextDecoder();
  const out: string[] = [];
  for (let i = 0; i < count; i++) out.push(dec.decode(chars.subarray(offsets[i], offsets[i + 1])));
  return out;
}

/** Face `i`'s authored color, or undefined when the section is absent/unset (alpha 0). */
function faceColorAt(colors: Uint8Array | null, i: number): FaceColor | undefined {
  if (!colors || colors.length < (i + 1) * 4) return undefined;
  const a = colors[i * 4 + 3];
  return a === 0 ? undefined : [colors[i * 4], colors[i * 4 + 1], colors[i * 4 + 2], a];
}

// ── Revolve body (profile ring swept around an in-plane axis) — the mock L2 body ─

/**
 * Synthesize a coarse revolve body: sweep the profile ring around the sketch-line
 * axis (shared plane-local lathe topology from lathePreview), transform to WORLD
 * via the sketch-plane basis, smooth per-vertex normals from the triangles, and
 * encode as MESH1 under a single face `f:0` + the two profile-boundary edges.
 *
 * MOCK LIMIT: this is a deterministic stand-in — no OCCT solid, no booleanMode
 * fusion; the real revolve (BRepPrimAPI_MakeRevol) is the worker's job. A
 * degenerate/near-zero angle is floored so a body still forms.
 */
export function makeRevolveBodyMesh(
  ring: [number, number][],
  axis: LatheAxis,
  plane: SketchPlane,
  angleDeg: number,
  lod = 0,
): ArrayBuffer {
  const a = Math.abs(angleDeg) < 1 ? 1 : Math.min(360, angleDeg);
  const local = latheLocal(ring, axis, a);
  const [ox, oy, oz] = plane.origin;
  const [xx, xy, xz] = plane.xAxis;
  const [yx, yy, yz] = plane.yAxis;
  const [nx, ny, nz] = plane.normal;

  const P = local.positions.length / 3;
  const worldPositions: number[] = new Array(P * 3);
  for (let i = 0; i < P; i++) {
    const u = local.positions[i * 3];
    const v = local.positions[i * 3 + 1];
    const w = local.positions[i * 3 + 2];
    worldPositions[i * 3] = ox + u * xx + v * yx + w * nx;
    worldPositions[i * 3 + 1] = oy + u * xy + v * yy + w * ny;
    worldPositions[i * 3 + 2] = oz + u * xz + v * yz + w * nz;
  }

  // Smooth normals: accumulate each triangle's face normal onto its vertices.
  const normals = new Array<number>(P * 3).fill(0);
  const idx = local.indices;
  for (let t = 0; t + 2 < idx.length; t += 3) {
    const ia = idx[t];
    const ib = idx[t + 1];
    const ic = idx[t + 2];
    const ax0 = worldPositions[ia * 3], ay0 = worldPositions[ia * 3 + 1], az0 = worldPositions[ia * 3 + 2];
    const e1x = worldPositions[ib * 3] - ax0, e1y = worldPositions[ib * 3 + 1] - ay0, e1z = worldPositions[ib * 3 + 2] - az0;
    const e2x = worldPositions[ic * 3] - ax0, e2y = worldPositions[ic * 3 + 1] - ay0, e2z = worldPositions[ic * 3 + 2] - az0;
    const nxT = e1y * e2z - e1z * e2y;
    const nyT = e1z * e2x - e1x * e2z;
    const nzT = e1x * e2y - e1y * e2x;
    for (const vi of [ia, ib, ic]) {
      normals[vi * 3] += nxT;
      normals[vi * 3 + 1] += nyT;
      normals[vi * 3 + 2] += nzT;
    }
  }
  for (let i = 0; i < P; i++) {
    const len = Math.hypot(normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]) || 1;
    normals[i * 3] /= len;
    normals[i * 3 + 1] /= len;
    normals[i * 3 + 2] /= len;
  }

  const triangles: [number, number, number][] = [];
  for (let t = 0; t + 2 < idx.length; t += 3) triangles.push([idx[t], idx[t + 1], idx[t + 2]]);
  const faces: FaceSource[] = [{ triangles, id: "f:0" }];

  // Edges: the two profile-boundary rings (start θ=0, end θ=angle).
  const ringN = local.ringCount;
  const endBase = local.segments * ringN;
  const loopPoints = (base: number): [number, number, number][] => {
    const pts: [number, number, number][] = [];
    for (let i = 0; i <= ringN; i++) {
      const vi = base + (i % ringN);
      pts.push([worldPositions[vi * 3], worldPositions[vi * 3 + 1], worldPositions[vi * 3 + 2]]);
    }
    return pts;
  };
  const edges: EdgeSource[] = [
    { points: loopPoints(0), id: "e:0" },
    { points: loopPoints(endBase), id: "e:1" },
  ];

  return encodeMesh1({ positions: worldPositions, normals, faces, edges, lod });
}
