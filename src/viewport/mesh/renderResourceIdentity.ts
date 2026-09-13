/*
 * RenderResourceIdentity — what makes one installed GPU mesh resource THAT
 * resource and not a look-alike (VP-HARDENING spec §8.1, requirement VP04).
 *
 * Every field answers a different question, and collapsing them would let a
 * cached overlay outlive the thing it was cut from:
 *
 *  - `documentId` / `runtimeSession` / `snapshotId` / `generation` fence the
 *    resource to the publication it came from. A signature that happens to
 *    match across snapshots is still a different resource.
 *  - `topologySignature` is derived from the AUTHORITATIVE face/edge identity
 *    tables and their incidence (ranges), never from positions or triangle
 *    order: re-tessellating the same solid at a finer LOD moves every vertex
 *    but must not read as new topology, while renaming one face id must.
 *  - `geometryRevision` and `qualityKey` move without a semantic document edit
 *    (a re-tessellation, an LOD change), which is exactly why they are separate
 *    from the signature.
 *  - `meshFormatVersion` keeps a v1 and a v2 blob of the same body apart.
 *
 * {@link identityKey} is the canonical serialization used as a cache key. It is
 * a `|`-joined string because every field is already a scalar and the highlight
 * cache needs a cheap, stable, order-independent map key.
 */
import type { BodyMeshView } from "./parseMeshPayload";

export interface RenderResourceIdentity {
  readonly documentId: string;
  readonly runtimeSession: string;
  readonly snapshotId: number;
  readonly generation: number;
  readonly bodyId: string;
  /** 64-bit FNV-1a hex over the identity tables and their incidence (§8.1). */
  readonly topologySignature: string;
  readonly geometryRevision: number;
  readonly qualityKey: string;
  readonly meshFormatVersion: 1 | 2;
}

/** The publication fields an entry carries when it came from a live publish. */
export interface IdentityProvenance {
  readonly documentId: string;
  readonly runtimeSession: string;
  readonly snapshotId: number;
  readonly generation: number;
}

export interface DeriveIdentityInput {
  readonly view: BodyMeshView;
  readonly bodyId: string;
  /** Absent for a bootstrap/cached-open entry or a preview: fields fall back to ""/0. */
  readonly provenance?: IdentityProvenance;
  /** The registry's monotonic build counter for this body — the geometry revision. */
  readonly meshRev: number;
}

// ── 64-bit FNV-1a, carried as two 32-bit halves ─────────────────────────────
//
// JavaScript has no 64-bit integer arithmetic outside BigInt (which allocates
// per operation and is far too slow for a per-install hash), so the state is a
// {hi, lo} pair of unsigned 32-bit words and the multiply by the 64-bit prime
// 0x100000001b3 is decomposed as `h·0x1b3 + (h << 40)`.
//
// GRANULARITY: the byte tables (`faceIdChars`, `edgeIdChars`) are hashed one
// BYTE per round, the fixed-width u32 tables one WORD per round. A word round
// mixes the same bits with a quarter of the loop iterations, which matters:
// a 60 000-face body carries ~720 KB of u32 tables and this runs on every mesh
// install. Both granularities are length-prefixed and tagged, so no two tables
// can alias each other by concatenation.

const FNV_PRIME_LOW = 0x1b3;

interface Fnv64 {
  hi: number;
  lo: number;
}

function fnvInit(): Fnv64 {
  return { hi: 0xcbf29ce4, lo: 0x84222325 };
}

/** `h = h * 0x100000001b3` (mod 2^64), split as `h·0x1b3 + (h << 40)`. */
function fnvMultiply(h: Fnv64): void {
  const lowProduct = h.lo * FNV_PRIME_LOW;
  const carry = Math.floor(lowProduct / 0x100000000);
  const shifted = (h.lo << 8) >>> 0; // (h << 40) keeps only lo's low 24 bits
  h.lo = lowProduct >>> 0;
  h.hi = (h.hi * FNV_PRIME_LOW + carry + shifted) >>> 0;
}

function fnvByte(h: Fnv64, byte: number): void {
  h.lo = (h.lo ^ byte) >>> 0;
  fnvMultiply(h);
}

function fnvWord(h: Fnv64, word: number): void {
  h.lo = (h.lo ^ word) >>> 0;
  fnvMultiply(h);
}

function fnvTag(h: Fnv64, tag: string, length: number): void {
  for (let i = 0; i < tag.length; i++) fnvByte(h, tag.charCodeAt(i) & 0xff);
  fnvWord(h, length >>> 0);
}

function fnvWords(h: Fnv64, tag: string, words: Uint32Array | null | undefined): void {
  fnvTag(h, tag, words ? words.length : 0);
  if (!words) return;
  for (let i = 0; i < words.length; i++) fnvWord(h, words[i]);
}

function fnvBytes(h: Fnv64, tag: string, bytes: Uint8Array | null | undefined): void {
  fnvTag(h, tag, bytes ? bytes.length : 0);
  if (!bytes) return;
  for (let i = 0; i < bytes.length; i++) fnvByte(h, bytes[i]);
}

function fnvHex(h: Fnv64): string {
  return h.hi.toString(16).padStart(8, "0") + h.lo.toString(16).padStart(8, "0");
}

/**
 * Hash the face/edge IDENTITY tables and their incidence. Positions, normals,
 * colors and the triangle buffer are deliberately absent: the signature answers
 * "is this the same named topology", not "is this the same triangulation".
 */
export function topologySignature(view: BodyMeshView): string {
  const h = fnvInit();
  fnvWord(h, view.faceCount >>> 0);
  fnvWords(h, "faceRanges", view.faceRanges);
  fnvWords(h, "faceIdOffsets", view.faceIdOffsets);
  fnvBytes(h, "faceIdChars", view.faceIdChars);
  fnvWord(h, view.edgeCount >>> 0);
  fnvWords(h, "edgeRanges", view.edgeRanges);
  fnvWords(h, "edgeIdOffsets", view.edgeIdOffsets);
  fnvBytes(h, "edgeIdChars", view.edgeIdChars);
  // Whether the ids ARE ElementIds changes what every id string means.
  fnvWord(h, view.idsHaveElementIds ? 1 : 0);
  return fnvHex(h);
}

/** Build the identity of one installed resource. Pure — no registry state. */
export function deriveIdentity(input: DeriveIdentityInput): RenderResourceIdentity {
  const { view, bodyId, provenance, meshRev } = input;
  return {
    documentId: provenance?.documentId ?? "",
    runtimeSession: provenance?.runtimeSession ?? "",
    snapshotId: provenance?.snapshotId ?? 0,
    generation: provenance?.generation ?? 0,
    bodyId,
    topologySignature: topologySignature(view),
    geometryRevision: meshRev,
    qualityKey: String(view.lod),
    meshFormatVersion: view.version === 2 ? 2 : 1,
  };
}

/** Canonical serialization — the prefix of every owned-overlay cache key. */
export function identityKey(id: RenderResourceIdentity): string {
  return [
    id.documentId,
    id.runtimeSession,
    id.snapshotId,
    id.generation,
    id.bodyId,
    id.topologySignature,
    id.geometryRevision,
    id.qualityKey,
    id.meshFormatVersion,
  ].join("|");
}
