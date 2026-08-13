/*
 * Mesh-derived geometric metrics for the MOCK lane (WP-C1).
 *
 * The real lane answers "how big is this body / this face / this edge?" with the
 * kernel (`QueryMassProperties`, `QueryElement` — OCCT `GProp` over the BRep).
 * The mock lane has no kernel, but it DOES have the real MESH1 bytes
 * (`mockMeshes.ts` synthesizes them verbatim), and for a closed triangle mesh the
 * divergence theorem gives volume / centroid / inertia that are EXACT, not
 * approximate. So the mock answers from the geometry it is actually drawing
 * rather than from a hash.
 *
 * WHY THAT MATTERS (it is not polish): the previous mock synthesized every
 * `elementInfo` field from an FNV hash of the pick, which meant every face
 * reported the normal `(0,0,1)`. A tool that derives the ANGLE between two picked
 * faces would then read 0° for every pair in the mock lane, and its e2e coverage
 * would be vacuous. Mesh-derived normals make the mock lane's angle a real
 * measurement of a real box.
 *
 * EXACTNESS, precisely stated:
 *   - Volume / centroid / inertia are exact for the mesh AS A SOLID — i.e. exact
 *     for the box, and exact for the FACETED 24-gon prism the mock draws in place
 *     of a cylinder. They are not π·r²·h, and must not be presented as if the
 *     mock had a curved surface. Numeric truth for real curved geometry lives in
 *     `worker/tests/test_mass_properties.cpp` + `src-tauri/tests/wire_contract.rs`.
 *   - Float32 positions round-trip through the MESH1 blob, so a value derived
 *     here carries float32 precision even though the arithmetic is float64.
 */
import { parseMeshPayload, type BodyMeshView } from "@/viewport/mesh/parseMeshPayload";
import type { MassProperties } from "./types";

type Vec3 = [number, number, number];
type Mat3 = [Vec3, Vec3, Vec3];

const decoder = new TextDecoder();

/** The i-th vertex of the flat `positions` array. */
function vertexAt(positions: Float32Array, i: number): Vec3 {
  return [positions[3 * i], positions[3 * i + 1], positions[3 * i + 2]];
}

function sub(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

function cross(a: Vec3, b: Vec3): Vec3 {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

function length(v: Vec3): number {
  return Math.hypot(v[0], v[1], v[2]);
}

function normalize(v: Vec3): Vec3 | null {
  const n = length(v);
  if (!Number.isFinite(n) || n < 1e-12) return null;
  return [v[0] / n, v[1] / n, v[2] / n];
}

/** Flip `v` so its dominant component is positive — the worker's rule verbatim
 *  (`MassProperties.cpp sign_canonical`), so both lanes agree on axis signs. */
function signCanonical(v: Vec3): Vec3 {
  let dominant = 0;
  for (let i = 1; i < 3; i++) if (Math.abs(v[i]) > Math.abs(v[dominant])) dominant = i;
  return v[dominant] < 0 ? [-v[0], -v[1], -v[2]] : v;
}

/** The face ids of a parsed mesh, in emission order. */
export function faceIds(view: BodyMeshView): string[] {
  const out: string[] = [];
  for (let f = 0; f < view.faceCount; f++) {
    out.push(
      decoder.decode(view.faceIdChars.subarray(view.faceIdOffsets[f], view.faceIdOffsets[f + 1])),
    );
  }
  return out;
}

/** The edge ids of a parsed mesh, in emission order (empty when it carries none). */
export function edgeIds(view: BodyMeshView): string[] {
  const offs = view.edgeIdOffsets;
  const chars = view.edgeIdChars;
  if (!offs || !chars) return [];
  const out: string[] = [];
  for (let e = 0; e < view.edgeCount; e++) {
    out.push(decoder.decode(chars.subarray(offs[e], offs[e + 1])));
  }
  return out;
}

// ── Body mass properties (divergence theorem over the closed hull) ───────────

/**
 * Exact mass properties of a CLOSED triangle mesh.
 *
 * Each triangle `(a,b,c)` spans a tetrahedron with the origin; summing the signed
 * contributions of those tetrahedra collapses the surface integral to a volume
 * one (divergence theorem), so an outward-wound closed mesh sums to its exact
 * enclosed volume, centroid and second moments — no sampling anywhere.
 *
 * The mesh's WINDING is load-bearing: outward-CCW gives a positive volume.
 * `mockMeshes` emits outward-CCW everywhere, and `Math.abs` is deliberately NOT
 * applied — a negative result would mean the mock started emitting inward
 * triangles, which the caller should see rather than have papered over.
 *
 * `principalMoments` / `principalAxes` follow the same contract as the wire form:
 * volume integrals (mm⁵) about the CENTROID, unit right-handed axes paired in
 * order. Their ORDER is the eigensolver's, not a sort — for an axis-aligned body
 * the inertia tensor is already diagonal, so the axes come out as world X, Y, Z.
 */
export function massPropertiesFromMesh(bodyId: string, blob: ArrayBuffer): MassProperties {
  const view = parseMeshPayload(blob);
  const { positions, indices } = view;

  let volume = 0;
  let area = 0;
  const momentSum: Vec3 = [0, 0, 0];
  // ∫ x·xᵀ dV about the ORIGIN, accumulated per tetrahedron (Blow/Binstock).
  const covariance: Mat3 = [
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
  ];
  // The canonical unit tetrahedron's covariance, ∫ x·xᵀ over (0,e1,e2,e3).
  const CANON: Mat3 = [
    [2 / 120, 1 / 120, 1 / 120],
    [1 / 120, 2 / 120, 1 / 120],
    [1 / 120, 1 / 120, 2 / 120],
  ];

  for (let t = 0; t < view.triangleCount; t++) {
    const a = vertexAt(positions, indices[3 * t]);
    const b = vertexAt(positions, indices[3 * t + 1]);
    const c = vertexAt(positions, indices[3 * t + 2]);

    area += length(cross(sub(b, a), sub(c, a))) / 2;

    // det[a b c] — six times the signed tetrahedron volume.
    const det = dot(a, cross(b, c));
    const tetVolume = det / 6;
    volume += tetVolume;
    // A tetrahedron's centroid is the mean of its four vertices; one is the origin.
    for (let i = 0; i < 3; i++) momentSum[i] += (tetVolume * (a[i] + b[i] + c[i])) / 4;

    // C_tet = det · A · CANON · Aᵀ, with A = [a b c] as COLUMNS.
    const cols: Mat3 = [a, b, c];
    for (let i = 0; i < 3; i++) {
      for (let j = 0; j < 3; j++) {
        let sum = 0;
        for (let k = 0; k < 3; k++) {
          for (let l = 0; l < 3; l++) sum += CANON[k][l] * cols[k][i] * cols[l][j];
        }
        covariance[i][j] += det * sum;
      }
    }
  }

  const centroid: Vec3 =
    Math.abs(volume) < 1e-12 ? [0, 0, 0] : [momentSum[0] / volume, momentSum[1] / volume, momentSum[2] / volume];

  // Parallel-axis shift of the covariance to the centroid, then the inertia
  // tensor I = tr(C)·Id − C (the standard identity for a solid).
  const centred: Mat3 = [
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
  ];
  for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) centred[i][j] = covariance[i][j] - volume * centroid[i] * centroid[j];
  }
  const trace = centred[0][0] + centred[1][1] + centred[2][2];
  const inertia: Mat3 = [
    [trace - centred[0][0], -centred[0][1], -centred[0][2]],
    [-centred[1][0], trace - centred[1][1], -centred[1][2]],
    [-centred[2][0], -centred[2][1], trace - centred[2][2]],
  ];

  const { values, vectors } = jacobiEigen(inertia);
  const a1 = signCanonical(normalize(vectors[0]) ?? [1, 0, 0]);
  const a2 = signCanonical(normalize(vectors[1]) ?? [0, 1, 0]);
  // Third axis REBUILT as a1 × a2 — the worker's rule, and what guarantees a
  // right-handed frame (canonicalizing all three independently can flip handedness).
  const a3 = normalize(cross(a1, a2)) ?? [0, 0, 1];

  return {
    bodyId,
    volume,
    surfaceArea: area,
    centroid,
    principalMoments: values,
    principalAxes: [a1, a2, a3],
  };
}

/**
 * Eigen-decomposition of a SYMMETRIC 3×3 matrix by cyclic Jacobi rotations.
 *
 * Chosen over a closed-form cubic solve because Jacobi is unconditionally stable
 * for the near-degenerate cases that matter here — a cube has three equal
 * moments, and a characteristic-polynomial root finder loses most of its digits
 * exactly there. An already-diagonal matrix (every axis-aligned body) needs zero
 * rotations, so the returned axes come out as world X, Y, Z in order.
 *
 * `vectors[k]` is the eigenvector paired with `values[k]`.
 */
function jacobiEigen(input: Mat3): { values: Vec3; vectors: [Vec3, Vec3, Vec3] } {
  const a: number[][] = input.map((row) => [...row]);
  // Eigenvector accumulator, held COLUMN-wise: v[i][k] is component i of vector k.
  const v: number[][] = [
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
  ];

  for (let sweep = 0; sweep < 64; sweep++) {
    const off = Math.abs(a[0][1]) + Math.abs(a[0][2]) + Math.abs(a[1][2]);
    if (off < 1e-14 * (Math.abs(a[0][0]) + Math.abs(a[1][1]) + Math.abs(a[2][2]) + 1e-30)) break;
    for (const [p, q] of [
      [0, 1],
      [0, 2],
      [1, 2],
    ] as const) {
      if (Math.abs(a[p][q]) < 1e-300) continue;
      const theta = (a[q][q] - a[p][p]) / (2 * a[p][q]);
      const t = Math.sign(theta || 1) / (Math.abs(theta) + Math.sqrt(theta * theta + 1));
      const cos = 1 / Math.sqrt(t * t + 1);
      const sin = t * cos;
      for (let k = 0; k < 3; k++) {
        const akp = a[k][p];
        const akq = a[k][q];
        a[k][p] = cos * akp - sin * akq;
        a[k][q] = sin * akp + cos * akq;
      }
      for (let k = 0; k < 3; k++) {
        const apk = a[p][k];
        const aqk = a[q][k];
        a[p][k] = cos * apk - sin * aqk;
        a[q][k] = sin * apk + cos * aqk;
      }
      for (let k = 0; k < 3; k++) {
        const vkp = v[k][p];
        const vkq = v[k][q];
        v[k][p] = cos * vkp - sin * vkq;
        v[k][q] = sin * vkp + cos * vkq;
      }
    }
  }

  return {
    values: [a[0][0], a[1][1], a[2][2]],
    vectors: [
      [v[0][0], v[1][0], v[2][0]],
      [v[0][1], v[1][1], v[2][1]],
      [v[0][2], v[1][2], v[2][2]],
    ],
  };
}

// ── Per-element metrics (the `elementInfo` inputs) ────────────────────────────

/** One mesh face's geometry, mirroring the fields of a kernel face descriptor. */
export interface MockFaceMetrics {
  /** Summed triangle area (mm²) — exact for a planar face. */
  area: number;
  /** BOUNDING-BOX centre of the face's vertices, matching the kernel descriptor. */
  center: Vec3;
  /** Area-weighted unit normal. Meaningful as a PLANE normal only when `planar`. */
  normal: Vec3;
  /** Whether every triangle of this face shares one normal (within 1e-4). */
  planar: boolean;
  /** Bounding-box diagonal — the kernel's coarse `size` proxy. */
  size: number;
}

/** One mesh edge's geometry, mirroring the fields of a kernel edge descriptor. */
export interface MockEdgeMetrics {
  /** Summed polyline length (mm). */
  length: number;
  center: Vec3;
  /** A 2-point polyline is a straight segment; anything longer is a curve. */
  straight: boolean;
  size: number;
}

function bboxOf(points: Vec3[]): { center: Vec3; size: number } {
  const min: Vec3 = [Infinity, Infinity, Infinity];
  const max: Vec3 = [-Infinity, -Infinity, -Infinity];
  for (const p of points) {
    for (let i = 0; i < 3; i++) {
      if (p[i] < min[i]) min[i] = p[i];
      if (p[i] > max[i]) max[i] = p[i];
    }
  }
  if (!Number.isFinite(min[0])) return { center: [0, 0, 0], size: 0 };
  return {
    center: [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2],
    size: Math.hypot(max[0] - min[0], max[1] - min[1], max[2] - min[2]),
  };
}

/** Metrics for the face carrying `faceId`, or null when the mesh has no such face. */
export function faceMetricsFromMesh(blob: ArrayBuffer, faceId: string): MockFaceMetrics | null {
  const view = parseMeshPayload(blob);
  const index = faceIds(view).indexOf(faceId);
  if (index < 0) return null;

  const firstTri = view.faceRanges[2 * index];
  const triCount = view.faceRanges[2 * index + 1];
  const points: Vec3[] = [];
  const areaWeighted: Vec3 = [0, 0, 0];
  const unitNormals: Vec3[] = [];
  let area = 0;

  for (let t = firstTri; t < firstTri + triCount; t++) {
    const a = vertexAt(view.positions, view.indices[3 * t]);
    const b = vertexAt(view.positions, view.indices[3 * t + 1]);
    const c = vertexAt(view.positions, view.indices[3 * t + 2]);
    points.push(a, b, c);
    const n = cross(sub(b, a), sub(c, a));
    const twiceArea = length(n);
    area += twiceArea / 2;
    for (let i = 0; i < 3; i++) areaWeighted[i] += n[i];
    const unit = normalize(n);
    if (unit) unitNormals.push(unit);
  }

  const normal = normalize(areaWeighted) ?? ([0, 0, 1] as Vec3);
  // Planar iff every triangle agrees with the mean normal. The mock's cylinder
  // side face must fail this — the whole point of `surfaceType` is that a curved
  // face's "normal" is not a plane normal, and a mock that claimed otherwise
  // would let the angle tool measure between two things that are not planes.
  const planar = unitNormals.every((n) => dot(n, normal) > 1 - 1e-4);
  const { center, size } = bboxOf(points);
  return { area, center, normal, planar, size };
}

/** One mesh face's CYLINDRICAL fit — the inputs a concentric snap needs. */
export interface MockCylinderMetrics {
  /** Unit axis direction, sign-canonical (the worker's `sign_canonical` rule). */
  axis: Vec3;
  /** A point ON the axis — the face's vertex centroid, exact for a full ring. */
  origin: Vec3;
  /** The common distance from the axis (mm). */
  radius: number;
}

/**
 * Fit a cylinder to the face carrying `faceId`, or `null` when it is not one.
 *
 * MEASURED, not asserted: the mock's "cylinder" is a faceted prism, and what
 * makes it read as a cylinder is a property its triangles genuinely have —
 * every facet normal is perpendicular to one common direction, and every
 * vertex lies on one circle in the plane normal to it. So the axis is
 * recovered as the eigenvector of the facet-normal covariance with the
 * SMALLEST eigenvalue (a cylinder's normals span the plane ⊥ axis and put zero
 * weight along it), a circle is LEAST-SQUARES fitted to the vertices projected
 * into that plane, and both the perpendicularity and the fit residual are
 * CHECKED before an answer is returned. A box face, a cone, a sphere fail one
 * of them and get `null` — the same refusal discipline
 * `faceMetricsFromMesh`'s `planar` flag applies.
 *
 * The circle is fitted rather than averaged because the vertex list is not
 * symmetric: the seam vertex is duplicated and triangulation repeats corners,
 * so a plain centroid sits slightly OFF the axis and would fail its own
 * equidistance test. Tolerances are loose enough for float32 round-tripping
 * through MESH1 and far too tight for a non-cylinder to slip through.
 */
export function cylinderMetricsFromMesh(
  blob: ArrayBuffer,
  faceId: string,
): MockCylinderMetrics | null {
  const view = parseMeshPayload(blob);
  const index = faceIds(view).indexOf(faceId);
  if (index < 0) return null;

  const firstTri = view.faceRanges[2 * index];
  const triCount = view.faceRanges[2 * index + 1];
  if (triCount < 2) return null;

  const normals: Vec3[] = [];
  const points: Vec3[] = [];
  for (let t = firstTri; t < firstTri + triCount; t++) {
    const a = vertexAt(view.positions, view.indices[3 * t]);
    const b = vertexAt(view.positions, view.indices[3 * t + 1]);
    const c = vertexAt(view.positions, view.indices[3 * t + 2]);
    points.push(a, b, c);
    const n = normalize(cross(sub(b, a), sub(c, a)));
    if (n) normals.push(n);
  }
  if (normals.length < 2 || points.length === 0) return null;

  // Σ n·nᵀ over the facet normals; its least-significant eigenvector is the
  // one direction no normal leans along — the axis, if there is one.
  const covariance: Mat3 = [
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
  ];
  for (const n of normals) {
    for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) covariance[i][j] += n[i] * n[j];
  }
  const { values, vectors } = jacobiEigen(covariance);
  let least = 0;
  for (let k = 1; k < 3; k++) if (values[k] < values[least]) least = k;
  const axis = normalize(vectors[least]);
  if (!axis) return null;

  // Every facet normal must be perpendicular to it, or this is not a cylinder.
  for (const n of normals) if (Math.abs(dot(n, axis)) > 1e-3) return null;

  // Project into the plane ⊥ axis and fit a circle there.
  const e1 = normalize(
    Math.abs(axis[0]) < 0.9 ? cross(axis, [1, 0, 0]) : cross(axis, [0, 1, 0]),
  );
  if (!e1) return null;
  const e2 = cross(axis, e1);
  const uv: [number, number][] = [];
  let alongSum = 0;
  for (const p of points) {
    uv.push([dot(p, e1), dot(p, e2)]);
    alongSum += dot(p, axis);
  }
  const circle = fitCircle(uv);
  if (!circle) return null;
  const { u: cu, v: cv, radius } = circle;
  for (const [u, v] of uv) {
    if (Math.abs(Math.hypot(u - cu, v - cv) - radius) > 1e-3 * radius) return null;
  }

  const along = alongSum / points.length;
  const origin: Vec3 = [
    e1[0] * cu + e2[0] * cv + axis[0] * along,
    e1[1] * cu + e2[1] * cv + axis[1] * along,
    e1[2] * cu + e2[2] * cv + axis[2] * along,
  ];
  return { axis: signCanonical(axis), origin, radius };
}

/**
 * Least-squares circle through 2D points (Kåsa): the residual
 * `u² + v² − 2·a·u − 2·b·v − c` is LINEAR in `(a, b, c)`, so the fit is one
 * 3×3 normal-equation solve rather than an iteration. `null` for a degenerate
 * (collinear / coincident) set, where there is no circle to report.
 */
function fitCircle(
  uv: ReadonlyArray<readonly [number, number]>,
): { u: number; v: number; radius: number } | null {
  if (uv.length < 3) return null;
  // Centre the data first — a ring 80 mm off the origin otherwise loses digits
  // in the u² terms before the solve ever runs.
  let mu = 0;
  let mv = 0;
  for (const [u, v] of uv) {
    mu += u / uv.length;
    mv += v / uv.length;
  }
  const m: number[][] = [
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
  ];
  for (const [rawU, rawV] of uv) {
    const u = rawU - mu;
    const v = rawV - mv;
    const s = u * u + v * v;
    const row = [2 * u, 2 * v, 1];
    for (let i = 0; i < 3; i++) {
      for (let j = 0; j < 3; j++) m[i][j] += row[i] * row[j];
      m[i][3] += row[i] * s;
    }
  }
  // Gauss-Jordan with partial pivoting on the 3×4 augmented system.
  for (let col = 0; col < 3; col++) {
    let pivot = col;
    for (let r = col + 1; r < 3; r++) if (Math.abs(m[r][col]) > Math.abs(m[pivot][col])) pivot = r;
    if (Math.abs(m[pivot][col]) < 1e-12) return null;
    [m[col], m[pivot]] = [m[pivot], m[col]];
    const d = m[col][col];
    for (let j = col; j < 4; j++) m[col][j] /= d;
    for (let r = 0; r < 3; r++) {
      if (r === col) continue;
      const f = m[r][col];
      for (let j = col; j < 4; j++) m[r][j] -= f * m[col][j];
    }
  }
  const [a, b, c] = [m[0][3], m[1][3], m[2][3]];
  const rSquared = c + a * a + b * b;
  if (!(rSquared > 0)) return null;
  return { u: a + mu, v: b + mv, radius: Math.sqrt(rSquared) };
}

/** Metrics for the edge carrying `edgeId`, or null when the mesh has no such edge. */
export function edgeMetricsFromMesh(blob: ArrayBuffer, edgeId: string): MockEdgeMetrics | null {
  const view = parseMeshPayload(blob);
  const ranges = view.edgeRanges;
  const edgePositions = view.edgePositions;
  if (!ranges || !edgePositions) return null;
  const index = edgeIds(view).indexOf(edgeId);
  if (index < 0) return null;

  const firstPoint = ranges[2 * index];
  const pointCount = ranges[2 * index + 1];
  const points: Vec3[] = [];
  for (let p = firstPoint; p < firstPoint + pointCount; p++) {
    points.push(vertexAt(edgePositions, p));
  }
  let total = 0;
  for (let i = 1; i < points.length; i++) total += length(sub(points[i], points[i - 1]));
  const { center, size } = bboxOf(points);
  return { length: total, center, straight: points.length === 2, size };
}
