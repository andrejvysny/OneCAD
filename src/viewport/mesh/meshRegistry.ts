/*
 * Mesh registry — a plain module Map<bodyId, MeshEntry> living OUTSIDE zustand.
 *
 * GPU geometry is heavy, imperative, and must be disposed deterministically, so
 * it never belongs in a projection store. `buildBodyObjects` turns a parsed
 * BodyMeshView into THREE geometry with ZERO-COPY attributes (positions/normals/
 * index alias the MESH1 blob directly); edges are expanded into fat-line segment
 * endpoints (the only non-trivial transform). `swap` double-buffers: the new
 * entry is published immediately and the old one is disposed on the NEXT frame
 * (via flushDisposals) so nothing that referenced it this frame reads freed
 * buffers. `disposeAll` + a dev leak tripwire guarantee the registry is empty
 * after a document closes.
 *
 * VP-HARDENING WP04 acceptance: TEST-MESH-05 — `MeshEntry.displayState` and
 * `isEntryPromotable` carry the stale-inspection-only policy of §9.
 *
 * VP-HARDENING WP03 (spec §8.2, requirement VP04): the registry is the UNIQUE
 * disposer of installed face/edge geometry. Every other consumer — the body
 * object, a section stencil pair, a whole-body highlight — BORROWS the exact
 * geometry object through a {@link MeshLease} and never builds a wrapper that
 * shares its BufferAttributes. Retirement is ordered against the frame loop: a
 * swapped-out entry becomes `retired`, and is disposed only at a LATER
 * `flushDisposals` than the one that ended the frame it was retired in, and
 * only once every lease on it is released.
 */
import * as THREE from "three";
import {
  countLeaseAcquired,
  countLeaseReleased,
  countMeshBuilt,
  countMeshRetired,
} from "../vph/instrumentation";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";
import { logError, logWarn } from "@/debug/log";
import type { BodyMeshView } from "./parseMeshPayload";
import { TopoIndex } from "./faceRangeIndex";
import { bakeFaceColors, deIndexTriangles, needsVertexColors } from "./faceColors";
import { requireValidatedMesh, type MeshAccounting, type ValidatedMesh } from "./validateMesh";
import { deriveIdentity, type RenderResourceIdentity } from "./renderResourceIdentity";
import type { Rgba } from "@/ipc/types";

/**
 * What the user is actually looking at for a body (spec §9). `current` is live
 * geometry. `stale-inspection-only` is the LAST VALID mesh kept on screen after
 * a replacement failed validation or admission — orbitable and measurable with
 * a historical tag, but never promotable to a persistent reference, because a
 * selected old face must not masquerade as current topology.
 * `failed-initial` is a body that never had valid geometry: an explicit
 * non-geometry state, not a fabricated proxy. `pending-replacement` is the
 * window between a fresh payload arriving and its replacement being installed.
 */
export type MeshDisplayState =
  | "current"
  | "stale-inspection-only"
  | "failed-initial"
  | "pending-replacement";

/**
 * Ownership state of one registry resource (spec §8.2).
 *
 * `installed` — the registry publishes it and consumers may lease it.
 * `retired`   — swapped out or removed; no NEW lease should be taken, and it is
 *               disposed once the open ones are released and a frame boundary
 *               has passed.
 * `disposed`  — its GPU buffers are gone; a lease on it is a bug.
 */
export type MeshResourceState = "installed" | "retired" | "disposed";

/**
 * A borrower's claim on an installed resource. Holding one guarantees the
 * registry will not dispose the geometry underneath you. `release()` is
 * idempotent — a consumer torn down twice must not double-decrement.
 */
export interface MeshLease {
  readonly entry: MeshEntry;
  /** Debug owner tag (`"body"`, `"section"`, `"highlight:body"`) — reported by the tripwire. */
  readonly owner: string;
  release(): void;
}

export interface MeshEntry {
  readonly bodyId: string;
  readonly meshRev: number;
  /** Authoritative publication this exact installed object was fetched from. */
  readonly provenance?: MeshProvenance;
  /** Full resource identity (spec §8.1) — the prefix of every owned-overlay cache key. */
  readonly identity: RenderResourceIdentity;
  /**
   * Ownership state. Mutable because the registry moves it; nothing outside
   * this module may write it. Read it after {@link acquireLease} to find out
   * whether the thing you just leased is still worth drawing.
   */
  resourceState: MeshResourceState;
  /**
   * The frame index this entry was retired at, or -1 while installed. Disposal
   * requires `retiredAtFrame < currentFrame`, which is what makes retirement
   * ordered against the frame loop rather than immediate.
   */
  retiredAtFrame: number;
  readonly view: BodyMeshView;
  /** Measured byte cost of this entry, proved by semantic validation. */
  readonly accounting: MeshAccounting;
  /**
   * Mutable because the entry OUTLIVES its own currency: a failed replacement
   * demotes the installed entry to `stale-inspection-only` in place rather than
   * removing geometry the user is still looking at. {@link isEntryPromotable}
   * is the single read of this fact.
   */
  displayState: MeshDisplayState;
  /**
   * Faces. Plain bodies: indexed geometry whose attributes alias the blob
   * (zero-copy). Bodies with authored FACE_COLORS: DE-INDEXED, with a baked
   * per-vertex `color` attribute (see faceColors.ts — triangle order, and
   * therefore picking and highlight drawRanges, is identical either way).
   */
  readonly geometry: THREE.BufferGeometry;
  /**
   * Edges: expanded segment endpoints as a FAT-line geometry (null when the
   * mesh has no edges). `setPositions` retains the array BY REFERENCE — no
   * CPU-side copy — and computes the bounds eagerly, which the LineSegments2
   * raycast needs before the first render.
   */
  readonly edgeGeometry: LineSegmentsGeometry | null;
  /**
   * The very array `edgeGeometry` was built from (xyzxyz per segment), kept so
   * HighlightLayer can slice one edge's segments out of it without re-expanding
   * the polylines. Null when the mesh has no edges.
   */
  readonly edgeSegmentPositions: Float32Array | null;
  /** Triangle index → face ordinal → lazy face id. */
  readonly faceIndex: TopoIndex;
  /** Segment ordinal → edge ordinal → lazy edge id (null when no edges). */
  readonly edgeIndex: TopoIndex | null;
  /** Packed {firstSeg, segCount} per edge, for the edge-highlight slice (null when no edges). */
  readonly edgeSegmentRanges: Uint32Array | null;
  /** True when `geometry` carries a baked per-vertex `color` attribute. */
  readonly hasVertexColors: boolean;
  /** The body color this entry was baked with (undefined = theme neutral). */
  readonly bodyColor?: Rgba;
  /** User-authored face colors this entry was baked with, keyed by the mesh id (ElementId or TopoKey). */
  readonly authoredFaceColors?: ReadonlyMap<string, Rgba>;
  /**
   * Theme change: re-bake the color attribute against the CURRENT body-fill
   * token. Only unset faces move — authored colors are data, not a token — so
   * this is an in-place rewrite of the existing array, never a new attribute.
   * No-op for a body with no colors (and after dispose).
   */
  rebakeFaceColors(): void;
  dispose(): void;
}

export interface MeshProvenance {
  readonly documentId: string;
  readonly runtimeSession: string;
  readonly snapshotId: number;
  readonly generation: number;
}

const registry = new Map<string, MeshEntry>();
let currentPublication: MeshProvenance | null = null;

export function setCurrentMeshPublication(provenance: MeshProvenance | null): void {
  currentPublication = provenance;
}

export function getCurrentMeshPublication(): MeshProvenance | null {
  return currentPublication;
}
/** Retired entries awaiting an ordered, lease-free frame boundary to be disposed at. */
const pendingDisposal: MeshEntry[] = [];
let liveGeometryCount = 0;
/**
 * Rendered-frame counter, advanced by {@link flushDisposals} (which the engine
 * calls exactly once per submitted frame, after `renderer.render`). Retirement
 * compares against it, so nothing is ever freed inside the frame that retired
 * it — a draw call already recorded for this frame may still name it.
 */
let currentFrame = 0;
/** Incremented whenever the leak tripwire catches a non-empty registry on close. */
export let leakTripwireCount = 0;

/** Per-entry lease bookkeeping. Off the entry so the interface stays readable. */
interface LeaseBook {
  /** Open lease count per owner tag — the tripwire reports these verbatim. */
  readonly owners: Map<string, number>;
  count: number;
  /** A lease attempt on a disposed entry has already been reported. */
  warned: boolean;
}

const leaseBooks = new WeakMap<MeshEntry, LeaseBook>();
let openLeaseTotal = 0;

function bookFor(entry: MeshEntry): LeaseBook {
  let book = leaseBooks.get(entry);
  if (!book) {
    book = { owners: new Map(), count: 0, warned: false };
    leaseBooks.set(entry, book);
  }
  return book;
}

/** Owner tags of every lease currently open on `entry` (one string per lease). */
export function openLeases(entry: MeshEntry): readonly string[] {
  const book = leaseBooks.get(entry);
  if (!book) return [];
  const out: string[] = [];
  for (const [owner, n] of book.owners) for (let i = 0; i < n; i++) out.push(owner);
  return out;
}

/**
 * Borrow `entry`'s geometry under `owner`. The lease is the ONLY thing that
 * keeps a retired resource alive, so every consumer that points a scene object
 * at `entry.geometry` (or `entry.edgeGeometry`) must hold one for exactly as
 * long as that object exists.
 *
 * Acquiring on an already-disposed entry is a consumer bug, not a fatal one:
 * it is reported once per entry and the returned lease hands back an entry
 * whose `resourceState` is `disposed`, so the caller can bail instead of
 * drawing freed buffers.
 */
export function acquireLease(entry: MeshEntry, owner: string): MeshLease {
  const book = bookFor(entry);
  if (entry.resourceState === "disposed") {
    if (!book.warned) {
      book.warned = true;
      logError("vp", "lease acquired on a DISPOSED mesh resource", {
        bodyId: entry.bodyId,
        meshRev: entry.meshRev,
        owner,
      });
    }
    return { entry, owner, release() {} };
  }
  book.count++;
  book.owners.set(owner, (book.owners.get(owner) ?? 0) + 1);
  openLeaseTotal++;
  countLeaseAcquired();
  let released = false;
  return {
    entry,
    owner,
    release() {
      if (released) return;
      released = true;
      // `dispose()` already voided every book entry (and reported the leak), so
      // a late release must not drive the counters negative.
      if (entry.resourceState === "disposed") return;
      book.count--;
      const left = (book.owners.get(owner) ?? 1) - 1;
      if (left > 0) book.owners.set(owner, left);
      else book.owners.delete(owner);
      openLeaseTotal--;
      countLeaseReleased();
      // The last release on a retired resource restarts its retirement clock:
      // it is freed one frame boundary later, never inside the frame that let
      // it go.
      if (book.count === 0 && entry.resourceState === "retired") {
        entry.retiredAtFrame = currentFrame;
      }
    },
  };
}

/** Listeners notified when a resource leaves the installed set (cache retirement). */
const retireListeners = new Set<(entry: MeshEntry) => void>();

/**
 * Subscribe to resource retirement. Owned overlays cut from a resource (the
 * highlight cache) MUST drop when their source retires — a compact face buffer
 * copied from a superseded mesh is stale geometry, not a cache hit.
 */
export function onEntryRetired(cb: (entry: MeshEntry) => void): () => void {
  retireListeners.add(cb);
  return () => retireListeners.delete(cb);
}

function retire(entry: MeshEntry): void {
  if (entry.resourceState !== "installed") return;
  entry.resourceState = "retired";
  entry.retiredAtFrame = currentFrame;
  pendingDisposal.push(entry);
  for (const cb of [...retireListeners]) cb(entry);
}

/**
 * Build THREE geometry from a SEMANTICALLY VALIDATED mesh. Does NOT insert into
 * the registry — call {@link swap} to publish it. Face attributes are zero-copy
 * views over the MESH1 blob; only the edge segment buffer is materialised.
 *
 * Only a {@link ValidatedMesh} may become GPU geometry (spec §9). The ingestion
 * boundary (`MeshIngest`) validates and admits first, then hands the validated
 * object in. The non-ingestion lanes — exact previews, library placement
 * ghosts, fixtures — pass a raw view and are validated HERE, throwing
 * `MeshValidationError`: they have no display state to fall back to, so the
 * only alternative would be building geometry from unproven numbers.
 */
export function buildBodyObjects(
  mesh: ValidatedMesh | BodyMeshView,
  bodyId: string,
  meshRev: number,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
  provenance?: MeshProvenance,
): MeshEntry {
  const validated = requireValidatedMesh(mesh, bodyId);
  const view = validated.view;
  const geometry = new THREE.BufferGeometry();
  // `drawRange` counts INDICES when the geometry is indexed and VERTICES when it
  // is not — and de-indexing produces exactly `indices.length` vertices, so the
  // same count is right in both arms (and so are HighlightLayer's face ranges).
  const colorAttr = buildFaceGeometry(geometry, view, bodyColor, authoredFaceColors);
  geometry.setDrawRange(0, view.indices.length);
  // MEASURED bounds, not the header box: the header box is the producer's
  // advisory statement and can be stale (validateMesh.ts explains how OCCT
  // produces one). Picking, fit, and frustum culling must agree with the
  // triangles that are actually there.
  const { min: bmin, max: bmax } = validated.accounting.actualBounds;
  reportHeaderBoundsExcursion(bodyId, validated.accounting);
  geometry.boundingBox = new THREE.Box3(
    new THREE.Vector3(bmin[0], bmin[1], bmin[2]),
    new THREE.Vector3(bmax[0], bmax[1], bmax[2]),
  );
  geometry.boundingSphere = geometry.boundingBox.getBoundingSphere(new THREE.Sphere());

  const faceIndex = new TopoIndex(view.faceRanges, view.faceCount, view.faceIdOffsets, view.faceIdChars);

  let edgeGeometry: LineSegmentsGeometry | null = null;
  let edgeSegmentPositions: Float32Array | null = null;
  let edgeIndex: TopoIndex | null = null;
  let edgeSegmentRanges: Uint32Array | null = null;

  if (view.hasEdges && view.edgeRanges && view.edgePositions && view.edgeIdOffsets && view.edgeIdChars) {
    const { positions, segRanges } = expandEdgeSegments(
      view.edgePositions,
      view.edgeRanges,
      view.edgeCount,
    );
    // Fat lines are instanced quads: one INSTANCE per segment, so `instanceCount`
    // (set by setPositions) is the whole draw extent — `setDrawRange` is
    // meaningless on this geometry and is deliberately not called.
    edgeGeometry = new LineSegmentsGeometry();
    edgeGeometry.setPositions(positions);
    edgeSegmentPositions = positions;
    edgeSegmentRanges = segRanges;
    edgeIndex = new TopoIndex(segRanges, view.edgeCount, view.edgeIdOffsets, view.edgeIdChars);
  }

  liveGeometryCount += edgeGeometry ? 2 : 1;
  let disposed = false;
  const entry: MeshEntry = {
    bodyId,
    meshRev,
    provenance,
    identity: deriveIdentity({ view, bodyId, provenance, meshRev }),
    resourceState: "installed",
    retiredAtFrame: -1,
    view,
    accounting: validated.accounting,
    displayState: "current",
    geometry,
    edgeGeometry,
    edgeSegmentPositions,
    faceIndex,
    edgeIndex,
    edgeSegmentRanges,
    hasVertexColors: colorAttr !== null,
    bodyColor,
    authoredFaceColors,
    rebakeFaceColors() {
      if (!colorAttr || disposed) return;
      bakeFaceColors(view, bodyColor, authoredFaceColors, colorAttr.array as Float32Array);
      colorAttr.needsUpdate = true;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      // Freeing under an open lease is the leak this package exists to make
      // impossible: `flushDisposals` never does it, so reaching here with
      // leases means a borrower outlived the document (or a test forced it).
      // Report the owners — the tag is the whole point of the tag.
      const book = leaseBooks.get(entry);
      if (book && book.count > 0) {
        leakTripwireCount++;
        logError("vp", "mesh resource disposed with OPEN leases", {
          bodyId: entry.bodyId,
          meshRev: entry.meshRev,
          owners: openLeases(entry),
        });
        openLeaseTotal -= book.count;
        for (let i = 0; i < book.count; i++) countLeaseReleased();
        book.count = 0;
        book.owners.clear();
      }
      entry.resourceState = "disposed";
      geometry.dispose();
      edgeGeometry?.dispose();
      liveGeometryCount -= edgeGeometry ? 2 : 1;
    },
  };
  return entry;
}

/**
 * Bodies whose header bbox has already been reported as not enclosing their
 * geometry. A producer defect is worth saying ONCE — repeating it every regen
 * or every preview frame would bury the log. Bounded so a long session's
 * preview ids cannot grow it without limit.
 */
const boundsExcursionReported = new Set<string>();
const BOUNDS_REPORT_CAP = 512;

function reportHeaderBoundsExcursion(bodyId: string, accounting: MeshAccounting): void {
  if (accounting.headerBoundsExcursionMm <= accounting.headerBoundsSlackMm) return;
  if (boundsExcursionReported.has(bodyId)) return;
  if (boundsExcursionReported.size >= BOUNDS_REPORT_CAP) boundsExcursionReported.clear();
  boundsExcursionReported.add(bodyId);
  logWarn("mesh", "MESH1 header bbox does not enclose geometry — using measured bounds", {
    bodyId,
    excursionMm: accounting.headerBoundsExcursionMm,
    slackMm: accounting.headerBoundsSlackMm,
    measured: accounting.actualBounds,
  });
}

/**
 * Populate the face geometry's attributes. Returns the baked `color` attribute
 * when the mesh carries authored FACE_COLORS, else null (and the plain indexed,
 * zero-copy layout).
 */
function buildFaceGeometry(
  geometry: THREE.BufferGeometry,
  view: BodyMeshView,
  bodyColor?: Rgba,
  authoredFaceColors?: ReadonlyMap<string, Rgba>,
): THREE.BufferAttribute | null {
  if (!needsVertexColors(view, bodyColor, authoredFaceColors)) {
    geometry.setAttribute("position", new THREE.BufferAttribute(view.positions, 3));
    if (view.normals) {
      geometry.setAttribute("normal", new THREE.BufferAttribute(view.normals, 3));
    }
    geometry.setIndex(new THREE.BufferAttribute(view.indices, 1));
    return null;
  }
  // Colored: vertices are duplicated per triangle so a crease between two
  // authored colors stays crisp instead of interpolating across the shared
  // vertex. Zero-copy is traded away here, and only here.
  const { positions, normals } = deIndexTriangles(view);
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  if (normals) geometry.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
  const colorAttr = new THREE.BufferAttribute(bakeFaceColors(view, bodyColor, authoredFaceColors), 3);
  geometry.setAttribute("color", colorAttr);
  return colorAttr;
}

/**
 * Expand per-edge polylines into GL_LINES segment endpoints. Edge `e` occupies
 * segments `[firstSeg, firstSeg+pointCount-1)`; segment ordinal → edge maps via
 * `segRanges` (packed {firstSeg, segCount}).
 */
export function expandEdgeSegments(
  edgePositions: Float32Array,
  edgeRanges: Uint32Array,
  edgeCount: number,
): { positions: Float32Array; segRanges: Uint32Array; segTotal: number } {
  let segTotal = 0;
  for (let e = 0; e < edgeCount; e++) {
    segTotal += Math.max(0, edgeRanges[e * 2 + 1] - 1);
  }
  const positions = new Float32Array(segTotal * 6); // 2 verts × 3 floats per segment
  const segRanges = new Uint32Array(edgeCount * 2);
  let segCursor = 0;
  let out = 0;
  for (let e = 0; e < edgeCount; e++) {
    const firstPoint = edgeRanges[e * 2];
    const pointCount = edgeRanges[e * 2 + 1];
    const segCount = Math.max(0, pointCount - 1);
    segRanges[e * 2] = segCursor;
    segRanges[e * 2 + 1] = segCount;
    for (let p = 0; p < segCount; p++) {
      const a = (firstPoint + p) * 3;
      const b = (firstPoint + p + 1) * 3;
      positions[out++] = edgePositions[a];
      positions[out++] = edgePositions[a + 1];
      positions[out++] = edgePositions[a + 2];
      positions[out++] = edgePositions[b];
      positions[out++] = edgePositions[b + 1];
      positions[out++] = edgePositions[b + 2];
    }
    segCursor += segCount;
  }
  return { positions, segRanges, segTotal };
}

/** Publish `next` for its body; RETIRE any previous entry (disposal is ordered). */
export function swap(bodyId: string, next: MeshEntry): void {
  const prev = registry.get(bodyId);
  registry.set(bodyId, next);
  // The upload volume was already priced by semantic validation, off the same
  // counts the geometry was built from — no need to re-walk the attributes.
  if (prev !== next) countMeshBuilt(next.accounting.estimatedGpuBytes);
  if (prev && prev !== next) retire(prev);
}

/** Remove a body's entry, retiring its geometry for a later frame boundary. */
export function remove(bodyId: string): void {
  const prev = registry.get(bodyId);
  if (prev) {
    registry.delete(bodyId);
    retire(prev);
  }
}

export function getEntry(bodyId: string): MeshEntry | undefined {
  return registry.get(bodyId);
}

/**
 * May this entry's topology author an operation or acquire a persistent
 * reference? Only `current` geometry may (spec §9): a stale body stays
 * orbitable, fittable, and inspectable with a historical tag, but a face picked
 * off it is NOT current topology and must never be promoted as though it were.
 *
 * Pure and side-effect free so the picker/promotion sites can consult it
 * without reaching into ingestion state.
 */
export function isEntryPromotable(entry: MeshEntry | undefined): boolean {
  return entry !== undefined && entry.displayState === "current";
}

export function registrySize(): number {
  return registry.size;
}

/**
 * Theme change: re-bake every registered body's face colors, because the UNSET
 * faces carry the body-fill TOKEN and that token just moved. Authored colors
 * are unaffected. A body with no colors is a no-op, so this stays cheap even
 * with the whole registry loaded. Driven from `MeshIngest.refreshColors()`
 * (see engine/README.md § Theming) — it does NOT repaint on its own.
 */
export function refreshFaceColors(): void {
  for (const e of registry.values()) e.rebakeFaceColors();
}

/**
 * End of a rendered frame: advance the frame counter, then free every retired
 * resource that has outlived the frame it was retired in AND has no lease left.
 * The engine calls this once per frame after `renderer.render`.
 *
 * A resource whose last lease was released this frame stays: `release()` reset
 * its `retiredAtFrame`, so it goes at the NEXT boundary. That one-frame lag is
 * the ordered retirement of spec §8.2 — the alternative would be blocking on
 * `gl.finish()`.
 */
export function flushDisposals(): void {
  currentFrame++;
  if (pendingDisposal.length === 0) return;
  let disposedCount = 0;
  let write = 0;
  for (let read = 0; read < pendingDisposal.length; read++) {
    const e = pendingDisposal[read];
    const book = leaseBooks.get(e);
    if ((book?.count ?? 0) === 0 && e.retiredAtFrame < currentFrame) {
      e.dispose();
      disposedCount++;
      continue;
    }
    pendingDisposal[write++] = e;
  }
  pendingDisposal.length = write;
  if (disposedCount > 0) countMeshRetired(disposedCount);
}

/**
 * Dispose everything and clear the registry (document close). Dev leak tripwire:
 * after this the registry MUST be empty and no live geometry may remain — a
 * violation logs an `err`-level `vp` event and bumps {@link leakTripwireCount}.
 *
 * Close releases the registry's OWNERSHIP; it cannot invalidate a live borrower
 * that was never detached (spec §8.2), so an entry still under lease here is
 * reported by owner tag from `dispose()` and freed anyway — the document is
 * gone and leaving the buffers alive would leak them outright.
 */
export function disposeAll(): void {
  // Empty the registry BEFORE notifying: a retirement listener rebuilds against
  // the live registry, and one that could still see the bodies being closed
  // would take fresh leases on them halfway through the close.
  const installed = [...registry.values()];
  registry.clear();
  currentPublication = null;
  for (const e of installed) retire(e);
  for (const e of pendingDisposal) e.dispose();
  countMeshRetired(pendingDisposal.length);
  pendingDisposal.length = 0;
  currentFrame++;
  if (registrySize() !== 0 || liveGeometryCount !== 0) {
    leakTripwireCount++;
    logError("vp", "meshRegistry leak tripwire after disposeAll", {
      size: registrySize(),
      liveGeometries: liveGeometryCount,
    });
  }
}

/** One entry as the ownership tests read it. */
export interface MeshEntryDebugInfo {
  readonly bodyId: string;
  readonly meshRev: number;
  readonly state: MeshResourceState;
  readonly retiredAtFrame: number;
  readonly leases: readonly string[];
}

/** Test/debug view of registry ownership — never read by production code. */
export function registryDebugSnapshot(): {
  frame: number;
  openLeases: number;
  installed: MeshEntryDebugInfo[];
  retired: MeshEntryDebugInfo[];
} {
  const info = (e: MeshEntry): MeshEntryDebugInfo => ({
    bodyId: e.bodyId,
    meshRev: e.meshRev,
    state: e.resourceState,
    retiredAtFrame: e.retiredAtFrame,
    leases: openLeases(e),
  });
  return {
    frame: currentFrame,
    openLeases: openLeaseTotal,
    installed: [...registry.values()].map(info),
    retired: pendingDisposal.map(info),
  };
}

/** Test-only: reset internal counters (does NOT dispose — call disposeAll first). */
export function __resetRegistryForTests(): void {
  registry.clear();
  boundsExcursionReported.clear();
  pendingDisposal.length = 0;
  liveGeometryCount = 0;
  leakTripwireCount = 0;
  openLeaseTotal = 0;
  currentFrame = 0;
}
