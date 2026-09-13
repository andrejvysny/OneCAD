/*
 * Adversarial MESH1 mutation generator — VP-HARDENING WP04.
 *
 * Acceptance: TEST-MESH-01, TEST-MESH-02, TEST-MESH-03, TEST-MESH-04.
 *
 * A hand-written negative test proves the check you thought of. This generator
 * exists for the ones you did not: it takes a VALID blob, applies exactly ONE
 * named mutation from a table with an entry per invariant, and declares what
 * the pipeline must say about the result — a specific `MeshParseErrorKind`, a
 * specific `MeshValidationCode`, or `"accept"` for the mutations the format
 * legitimately permits (an unknown optional section, a different LOD, a bbox
 * with slack, the same text used as both a face id and an edge id).
 *
 * Every case is addressed by `(base, seed, index)` and its RNG is derived from
 * that triple alone, so a counterexample found in a 10,000-case sweep replays
 * standalone from three numbers — which is what makes a checked-in adversarial
 * corpus possible.
 *
 * Two invariants are NOT reachable by mutating one of these small blobs in
 * place and are covered by explicit cases instead: `id-too-long` (needs an ID
 * character section of at least 257 bytes) and `count-overflow`/`payload-budget`
 * (need declared counts that no section length could satisfy, so the parser
 * rejects them for length before validation is reached).
 */
import { parseMeshPayload, type BodyMeshView, type MeshParseErrorKind } from "./parseMeshPayload";
import type { MeshValidationCode } from "./validateMesh";

const HEADER_BYTES = 64;
const TABLE_ENTRY_BYTES = 16;

const align4 = (n: number): number => (n + 3) & ~3;

/**
 * What the parse+validate pipeline must report for a mutated blob.
 * `accept-with-excursion` is a payload that validates but whose HEADER bbox no
 * longer encloses its own geometry — reported as a producer diagnostic and
 * overridden with measured bounds, never a rejection (see `validateMesh.ts`).
 */
export type MutationExpectation =
  | MeshParseErrorKind
  | MeshValidationCode
  | "accept"
  | "accept-with-excursion";

export interface MeshMutationCase {
  readonly mutator: string;
  readonly seed: number;
  readonly index: number;
  readonly blob: ArrayBuffer;
  readonly expected: MutationExpectation;
}

interface SectionEntry {
  readonly type: number;
  /** Byte offset of this section's 16-byte table entry within the blob. */
  readonly entryOffset: number;
  readonly offset: number;
  readonly byteLen: number;
}

interface MutationContext {
  readonly buffer: ArrayBuffer;
  readonly dv: DataView;
  /** Parsed from the WORKING COPY, so its typed arrays write straight through. */
  readonly view: BodyMeshView;
  /** Section table in blob order (ascending offset). */
  readonly sections: readonly SectionEntry[];
  readonly rng: () => number;
  /** Uniform integer in `[0, n)`; 0 when `n <= 0`. */
  pick(n: number): number;
}

interface MutationOutcome {
  readonly expected: MutationExpectation;
  /** A wholesale replacement (truncation, appended section); omit to use the in-place buffer. */
  readonly blob?: ArrayBuffer;
}

interface MeshMutator {
  readonly name: string;
  /** Mutate `ctx` and declare the expectation, or return null when inapplicable. */
  apply(ctx: MutationContext): MutationOutcome | null;
}

/** xorshift32. Deterministic, dependency-free, and good enough to pick mutations. */
export function makeRng(seed: number): () => number {
  let state = seed >>> 0 || 0x9e3779b9;
  return () => {
    state ^= state << 13;
    state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state / 0x1_0000_0000;
  };
}

/** Decorrelate `(seed, index)` into a per-case RNG seed. */
function caseSeed(seed: number, index: number): number {
  let h = (seed ^ Math.imul(index + 1, 0x9e3779b1)) >>> 0;
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b) >>> 0;
  h ^= h >>> 13;
  return h >>> 0;
}

function readSections(buffer: ArrayBuffer): SectionEntry[] {
  const dv = new DataView(buffer);
  const count = dv.getUint16(0x1e, true);
  const out: SectionEntry[] = [];
  for (let i = 0; i < count; i++) {
    const entryOffset = HEADER_BYTES + i * TABLE_ENTRY_BYTES;
    out.push({
      type: dv.getUint32(entryOffset, true),
      entryOffset,
      offset: dv.getUint32(entryOffset + 8, true),
      byteLen: dv.getUint32(entryOffset + 12, true),
    });
  }
  return out;
}

/**
 * Re-emit `blob` with one extra section of an UNKNOWN type appended. The
 * section table grows by one entry, so every existing section slides 16 bytes
 * — offsets stay 4-aligned and the table stays sorted by offset, which is what
 * makes this a structurally valid blob the parser must skip rather than reject.
 */
export function withUnknownSection(
  blob: ArrayBuffer,
  type: number,
  payload: Uint8Array,
): ArrayBuffer {
  const src = new Uint8Array(blob);
  const srcDv = new DataView(blob);
  const count = srcDv.getUint16(0x1e, true);
  const oldTableEnd = HEADER_BYTES + count * TABLE_ENTRY_BYTES;
  const shift = TABLE_ENTRY_BYTES;
  const unknownOffset = align4(blob.byteLength + shift);
  const total = align4(unknownOffset + payload.length);

  const out = new ArrayBuffer(total);
  const dst = new Uint8Array(out);
  const dv = new DataView(out);
  dst.set(src.subarray(0, HEADER_BYTES), 0);
  dv.setUint16(0x1e, count + 1, true);
  for (let i = 0; i < count; i++) {
    const from = HEADER_BYTES + i * TABLE_ENTRY_BYTES;
    dv.setUint32(from + 0x00, srcDv.getUint32(from + 0x00, true), true);
    dv.setUint32(from + 0x04, 0, true);
    dv.setUint32(from + 0x08, srcDv.getUint32(from + 0x08, true) + shift, true);
    dv.setUint32(from + 0x0c, srcDv.getUint32(from + 0x0c, true), true);
  }
  const newEntry = HEADER_BYTES + count * TABLE_ENTRY_BYTES;
  dv.setUint32(newEntry + 0x00, type, true);
  dv.setUint32(newEntry + 0x04, 0, true);
  dv.setUint32(newEntry + 0x08, unknownOffset, true);
  dv.setUint32(newEntry + 0x0c, payload.length, true);
  dst.set(src.subarray(oldTableEnd), oldTableEnd + shift);
  dst.set(payload, unknownOffset);
  return out;
}

/** A face id and an edge id of the same byte length, for byte-for-byte overwrites. */
function equalLengthIdPair(
  offsets: Uint32Array,
  count: number,
  ctx: MutationContext,
): [number, number] | null {
  if (count < 2) return null;
  const first = ctx.pick(count);
  for (let step = 1; step < count; step++) {
    const other = (first + step) % count;
    if (offsets[first + 1] - offsets[first] === offsets[other + 1] - offsets[other]) {
      return [first, other];
    }
  }
  return null;
}

const NON_FINITE: readonly number[] = [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY];

export const MESH_MUTATORS: readonly MeshMutator[] = [
  // ── mutations the format legitimately permits ──
  { name: "identity", apply: () => ({ expected: "accept" }) },
  {
    name: "lod-valid",
    apply: (ctx) => {
      ctx.dv.setUint16(0x1c, ctx.pick(3), true); // 0 coarse, 1 medium, 2 fine
      return { expected: "accept" };
    },
  },
  {
    name: "lod-invalid",
    apply: (ctx) => {
      ctx.dv.setUint16(0x1c, 3 + ctx.pick(0xfffc), true);
      return { expected: "lod" };
    },
  },
  {
    name: "bbox-inflate",
    apply: (ctx) => {
      const axis = ctx.pick(3);
      ctx.dv.setFloat32(0x20 + axis * 4, ctx.view.bboxMin[axis] - 5, true);
      ctx.dv.setFloat32(0x2c + axis * 4, ctx.view.bboxMax[axis] + 5, true);
      return { expected: "accept" };
    },
  },
  {
    name: "edge-id-shadows-face-id",
    apply: (ctx) => {
      // The SAME text in both namespaces is legal — uniqueness is per namespace.
      const { faceIdOffsets, faceIdChars, edgeIdOffsets, edgeIdChars, faceCount, edgeCount } = ctx.view;
      if (!edgeIdOffsets || !edgeIdChars || faceCount === 0 || edgeCount === 0) return null;
      const face = ctx.pick(faceCount);
      const faceLen = faceIdOffsets[face + 1] - faceIdOffsets[face];
      for (let e = 0; e < edgeCount; e++) {
        if (edgeIdOffsets[e + 1] - edgeIdOffsets[e] !== faceLen) continue;
        edgeIdChars.set(faceIdChars.subarray(faceIdOffsets[face], faceIdOffsets[face + 1]), edgeIdOffsets[e]);
        return { expected: "accept" };
      }
      return null;
    },
  },
  {
    name: "unknown-section",
    apply: (ctx) => ({
      expected: "accept",
      blob: withUnknownSection(ctx.buffer, 200, new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8])),
    }),
  },

  // ── byte-layout violations (parser) ──
  {
    name: "bad-magic",
    apply: (ctx) => {
      ctx.dv.setUint32(0x00, 0x4d455349, true);
      return { expected: "bad-magic" };
    },
  },
  {
    name: "bad-version",
    apply: (ctx) => {
      ctx.dv.setUint16(0x04, 2 + ctx.pick(64), true);
      return { expected: "unsupported-version" };
    },
  },
  {
    name: "reserved-flag-bit",
    apply: (ctx) => {
      ctx.dv.setUint16(0x06, ctx.view.flags | (1 << (5 + ctx.pick(11))), true);
      return { expected: "reserved-nonzero" };
    },
  },
  {
    name: "header-reserved-word",
    apply: (ctx) => {
      ctx.dv.setUint32(0x38 + ctx.pick(2) * 4, 1 + ctx.pick(0xffff), true);
      return { expected: "reserved-nonzero" };
    },
  },
  {
    name: "section-pad-nonzero",
    apply: (ctx) => {
      const s = ctx.sections[ctx.pick(ctx.sections.length)];
      ctx.dv.setUint32(s.entryOffset + 0x04, 1 + ctx.pick(0xffff), true);
      return { expected: "reserved-nonzero" };
    },
  },
  {
    name: "section-misaligned",
    apply: (ctx) => {
      const s = ctx.sections[ctx.pick(ctx.sections.length)];
      ctx.dv.setUint32(s.entryOffset + 0x08, s.offset + 1 + ctx.pick(3), true);
      return { expected: "misaligned" };
    },
  },
  {
    name: "section-overlap",
    apply: (ctx) => {
      if (ctx.sections.length < 2) return null;
      const i = 1 + ctx.pick(ctx.sections.length - 1);
      // A zero-length predecessor ends where it starts, so reusing its offset
      // would be a legal abutment rather than an overlap.
      if (ctx.sections[i - 1].byteLen === 0) return null;
      ctx.dv.setUint32(ctx.sections[i].entryOffset + 0x08, ctx.sections[i - 1].offset, true);
      return { expected: "section-overlap" };
    },
  },
  {
    name: "duplicate-section-type",
    apply: (ctx) => {
      if (ctx.sections.length < 2) return null;
      const i = 1 + ctx.pick(ctx.sections.length - 1);
      ctx.dv.setUint32(ctx.sections[i].entryOffset + 0x00, ctx.sections[i - 1].type, true);
      return { expected: "duplicate-section" };
    },
  },
  {
    name: "truncate-header",
    apply: (ctx) => ({ expected: "truncated", blob: ctx.buffer.slice(0, 4 + ctx.pick(60)) }),
  },
  {
    name: "truncate-tail",
    apply: (ctx) => {
      // encodeMesh1 pads the blob by at most 3 bytes, so dropping 8 always cuts
      // into the last section's declared extent — as long as the header and the
      // section table still survive the cut.
      const cut = ctx.buffer.byteLength - 8;
      if (cut <= HEADER_BYTES + ctx.sections.length * TABLE_ENTRY_BYTES) return null;
      return { expected: "section-bounds", blob: ctx.buffer.slice(0, align4(cut)) };
    },
  },
  {
    name: "section-short-length",
    apply: (ctx) => {
      // FACE_COLORS (type 12) is the one TOLERATED section — a wrong length there
      // costs the colours, not the body — so it is excluded from this mutation.
      const candidates = ctx.sections.filter((s) => s.type !== 12 && s.byteLen >= 4);
      if (candidates.length === 0) return null;
      const s = candidates[ctx.pick(candidates.length)];
      ctx.dv.setUint32(s.entryOffset + 0x0c, s.byteLen - 4, true);
      return { expected: "bad-length" };
    },
  },
  {
    name: "face-id-offset-end",
    apply: (ctx) => {
      // The parser cross-checks FACE_ID_CHARS.byteLen against offs[F], so a moved
      // terminal offset is caught as a LENGTH violation before validation runs.
      const { faceIdOffsets, faceCount } = ctx.view;
      faceIdOffsets[faceCount] = faceIdOffsets[faceCount] + 4;
      return { expected: "bad-length" };
    },
  },

  // ── semantic violations (validation) ──
  {
    name: "nonfinite-position",
    apply: (ctx) => {
      const { positions } = ctx.view;
      if (positions.length === 0) return null;
      positions[ctx.pick(positions.length)] = NON_FINITE[ctx.pick(NON_FINITE.length)];
      return { expected: "nonfinite-position" };
    },
  },
  {
    name: "nonfinite-edge-point",
    apply: (ctx) => {
      const { edgePositions } = ctx.view;
      if (!edgePositions || edgePositions.length === 0) return null;
      edgePositions[ctx.pick(edgePositions.length)] = NON_FINITE[ctx.pick(NON_FINITE.length)];
      return { expected: "nonfinite-position" };
    },
  },
  {
    name: "nonfinite-normal",
    apply: (ctx) => {
      const { normals } = ctx.view;
      if (!normals || normals.length === 0) return null;
      normals[ctx.pick(normals.length)] = NON_FINITE[ctx.pick(NON_FINITE.length)];
      return { expected: "nonfinite-normal" };
    },
  },
  {
    name: "normal-scaled",
    apply: (ctx) => {
      const { normals, vertexCount } = ctx.view;
      if (!normals || vertexCount === 0) return null;
      const v = ctx.pick(vertexCount);
      const scale = ctx.rng() < 0.5 ? 1.01 : 0.5;
      for (let a = 0; a < 3; a++) normals[v * 3 + a] *= scale;
      return { expected: "normal-length" };
    },
  },
  {
    name: "normal-zeroed",
    apply: (ctx) => {
      const { normals, vertexCount } = ctx.view;
      if (!normals || vertexCount === 0) return null;
      const v = ctx.pick(vertexCount);
      for (let a = 0; a < 3; a++) normals[v * 3 + a] = 0;
      return { expected: "normal-length" };
    },
  },
  {
    name: "index-out-of-range",
    apply: (ctx) => {
      const { indices, vertexCount } = ctx.view;
      if (indices.length === 0) return null;
      indices[ctx.pick(indices.length)] = vertexCount + ctx.pick(16);
      return { expected: "index-out-of-range" };
    },
  },
  {
    name: "face-range-shift",
    apply: (ctx) => {
      const { faceRanges, faceCount } = ctx.view;
      if (faceCount === 0) return null;
      const f = ctx.pick(faceCount);
      faceRanges[f * 2] = faceRanges[f * 2] + 1;
      return { expected: "face-range" };
    },
  },
  {
    name: "face-range-count",
    apply: (ctx) => {
      const { faceRanges, faceCount } = ctx.view;
      if (faceCount === 0) return null;
      const f = ctx.pick(faceCount);
      faceRanges[f * 2 + 1] = faceRanges[f * 2 + 1] + 1;
      return { expected: "face-range" };
    },
  },
  {
    name: "face-range-emptied",
    apply: (ctx) => {
      const { faceRanges, faceCount } = ctx.view;
      if (faceCount === 0) return null;
      const f = ctx.pick(faceCount);
      if (faceRanges[f * 2 + 1] === 0) return null; // already zero: no coverage change
      faceRanges[f * 2 + 1] = 0;
      return { expected: "face-range" };
    },
  },
  {
    name: "edge-range-shift",
    apply: (ctx) => {
      const { edgeRanges, edgeCount } = ctx.view;
      if (!edgeRanges || edgeCount === 0) return null;
      const e = ctx.pick(edgeCount);
      edgeRanges[e * 2] = edgeRanges[e * 2] + 1;
      return { expected: "edge-range" };
    },
  },
  {
    name: "edge-range-count",
    apply: (ctx) => {
      const { edgeRanges, edgeCount } = ctx.view;
      if (!edgeRanges || edgeCount === 0) return null;
      const e = ctx.pick(edgeCount);
      edgeRanges[e * 2 + 1] = edgeRanges[e * 2 + 1] + 1;
      return { expected: "edge-range" };
    },
  },
  {
    name: "edge-expansion-blowup",
    apply: (ctx) => {
      // Legal small blob, structurally valid table, ~13 GB of implied segment
      // endpoints. Must be refused by ARITHMETIC, never by attempting it.
      const { edgeRanges, edgeCount } = ctx.view;
      if (!edgeRanges || edgeCount === 0) return null;
      edgeRanges[ctx.pick(edgeCount) * 2 + 1] = 0x2000_0000;
      return { expected: "expansion-budget" };
    },
  },
  {
    name: "face-id-offset-nonmonotonic",
    apply: (ctx) => {
      const { faceIdOffsets, faceCount } = ctx.view;
      if (faceCount < 2) return null;
      const i = 1 + ctx.pick(faceCount - 1);
      faceIdOffsets[i] = faceIdOffsets[i + 1] + 1;
      return { expected: "id-offsets" };
    },
  },
  {
    name: "face-id-emptied",
    apply: (ctx) => {
      const { faceIdOffsets, faceCount } = ctx.view;
      if (faceCount < 2) return null;
      const i = 1 + ctx.pick(faceCount - 1);
      if (faceIdOffsets[i] === faceIdOffsets[i - 1]) return null;
      faceIdOffsets[i] = faceIdOffsets[i - 1];
      return { expected: "id-empty" };
    },
  },
  {
    name: "face-id-invalid-utf8",
    apply: (ctx) => {
      const { faceIdChars } = ctx.view;
      if (faceIdChars.length === 0) return null;
      faceIdChars[ctx.pick(faceIdChars.length)] = 0xff;
      return { expected: "id-utf8" };
    },
  },
  {
    name: "face-id-duplicate",
    apply: (ctx) => {
      const { faceIdOffsets, faceIdChars, faceCount } = ctx.view;
      const pair = equalLengthIdPair(faceIdOffsets, faceCount, ctx);
      if (!pair) return null;
      const [a, b] = pair;
      faceIdChars.set(faceIdChars.subarray(faceIdOffsets[b], faceIdOffsets[b + 1]), faceIdOffsets[a]);
      return { expected: "id-duplicate" };
    },
  },
  {
    name: "edge-id-invalid-utf8",
    apply: (ctx) => {
      const { edgeIdChars } = ctx.view;
      if (!edgeIdChars || edgeIdChars.length === 0) return null;
      edgeIdChars[ctx.pick(edgeIdChars.length)] = 0xff;
      return { expected: "id-utf8" };
    },
  },
  {
    name: "edge-id-duplicate",
    apply: (ctx) => {
      const { edgeIdOffsets, edgeIdChars, edgeCount } = ctx.view;
      if (!edgeIdOffsets || !edgeIdChars) return null;
      const pair = equalLengthIdPair(edgeIdOffsets, edgeCount, ctx);
      if (!pair) return null;
      const [a, b] = pair;
      edgeIdChars.set(edgeIdChars.subarray(edgeIdOffsets[b], edgeIdOffsets[b + 1]), edgeIdOffsets[a]);
      return { expected: "id-duplicate" };
    },
  },
  {
    name: "id-too-long",
    apply: (ctx) => {
      // Reachable only when the character section can hold a >256-byte id.
      const { faceIdOffsets, faceIdChars, faceCount } = ctx.view;
      if (faceCount < 2 || faceIdChars.length <= 256) return null;
      for (let i = 1; i < faceCount; i++) faceIdOffsets[i] = Math.min(257, faceIdChars.length);
      return { expected: "id-too-long" };
    },
  },
  {
    name: "bbox-nonfinite",
    apply: (ctx) => {
      const axis = ctx.pick(3);
      const base = ctx.rng() < 0.5 ? 0x20 : 0x2c;
      ctx.dv.setFloat32(base + axis * 4, NON_FINITE[ctx.pick(NON_FINITE.length)], true);
      return { expected: "bbox" };
    },
  },
  {
    name: "bbox-inverted",
    apply: (ctx) => {
      const axis = ctx.pick(3);
      const min = ctx.view.bboxMin[axis];
      const max = ctx.view.bboxMax[axis];
      if (!(max > min)) return null; // a degenerate axis cannot be inverted
      ctx.dv.setFloat32(0x20 + axis * 4, max, true);
      ctx.dv.setFloat32(0x2c + axis * 4, min, true);
      return { expected: "bbox" };
    },
  },
  {
    name: "bbox-shrunk",
    apply: (ctx) => {
      const axis = ctx.pick(3);
      const min = ctx.view.bboxMin[axis];
      const max = ctx.view.bboxMax[axis];
      if (!(max - min > 1)) return null; // no room to exclude a real vertex
      ctx.dv.setFloat32(0x2c + axis * 4, min + (max - min) / 2, true);
      // Not a rejection: an under-sized header box is a producer defect the
      // consumer measures around (MESH1 v1 policy, numerics §9.3).
      return { expected: "accept-with-excursion" };
    },
  },
  {
    name: "face-bbox-nonfinite",
    apply: (ctx) => {
      const { faceBboxes } = ctx.view;
      if (!faceBboxes || faceBboxes.length === 0) return null;
      faceBboxes[ctx.pick(faceBboxes.length)] = NON_FINITE[ctx.pick(NON_FINITE.length)];
      return { expected: "face-bbox" };
    },
  },
  {
    name: "face-bbox-inverted",
    apply: (ctx) => {
      const { faceBboxes, faceCount } = ctx.view;
      if (!faceBboxes || faceCount === 0) return null;
      const f = ctx.pick(faceCount);
      const axis = ctx.pick(3);
      const lo = faceBboxes[f * 6 + axis];
      const hi = faceBboxes[f * 6 + 3 + axis];
      if (!(hi > lo)) return null;
      faceBboxes[f * 6 + axis] = hi;
      faceBboxes[f * 6 + 3 + axis] = lo;
      return { expected: "face-bbox" };
    },
  },
];

const MUTATORS_BY_NAME = new Map(MESH_MUTATORS.map((m) => [m.name, m]));

function contextFor(base: ArrayBuffer, rng: () => number): MutationContext {
  const buffer = base.slice(0);
  return {
    buffer,
    dv: new DataView(buffer),
    view: parseMeshPayload(buffer),
    sections: readSections(buffer),
    rng,
    pick: (n: number) => (n <= 0 ? 0 : Math.min(n - 1, Math.floor(rng() * n))),
  };
}

/**
 * Produce case `index` of the `seed` sweep over `base`. Returns null when the
 * chosen mutator does not apply to this blob (the caller simply skips it).
 * Deterministic in `(base, seed, index)` alone.
 */
export function mutateMesh(base: ArrayBuffer, seed: number, index: number): MeshMutationCase | null {
  const rng = makeRng(caseSeed(seed, index));
  const mutator = MESH_MUTATORS[Math.min(MESH_MUTATORS.length - 1, Math.floor(rng() * MESH_MUTATORS.length))];
  const ctx = contextFor(base, rng);
  const outcome = mutator.apply(ctx);
  if (!outcome) return null;
  return {
    mutator: mutator.name,
    seed,
    index,
    blob: outcome.blob ?? ctx.buffer,
    expected: outcome.expected,
  };
}

/** Replay one named mutation against a base blob — the corpus regression entry point. */
export function applyNamedMutation(
  base: ArrayBuffer,
  name: string,
  seed: number,
  index: number,
): MeshMutationCase | null {
  const mutator = MUTATORS_BY_NAME.get(name);
  if (!mutator) throw new Error(`unknown mesh mutator "${name}"`);
  const rng = makeRng(caseSeed(seed, index));
  rng(); // consume the draw `mutateMesh` spends selecting the mutator
  const ctx = contextFor(base, rng);
  const outcome = mutator.apply(ctx);
  if (!outcome) return null;
  return { mutator: name, seed, index, blob: outcome.blob ?? ctx.buffer, expected: outcome.expected };
}
