/*
 * HighlightLayer — hover + selected highlighting, with EXPLICIT GPU resource
 * ownership (VP-HARDENING WP03, spec §8.2/§8.3, requirement VP04, finding R02).
 *
 * Three overlay strategies, one per shape of the thing being highlighted, and
 * the difference between them is entirely a difference of OWNERSHIP:
 *
 *  - WHOLE BODY: a `THREE.Mesh` over the registry's EXACT `entry.geometry`
 *    object, held under a `"highlight:body"` lease. It owns nothing, copies
 *    nothing, and disposes nothing; the lease is what stops the registry
 *    freeing the buffers underneath it.
 *  - FACE (one, or a body's whole selected set): compact OWNED geometry cut by
 *    `faceSliceGeometry` — the face triangles copied into a dedicated buffer
 *    under a local vertex remap. It is disposed like any other owned resource.
 *    A body's multi-face selection is ONE combined buffer, reused across
 *    selection changes with geometric capacity growth.
 *  - EDGE: owned `LineSegmentsGeometry` over a `subarray` VIEW of the entry's
 *    expanded segment positions (no CPU copy, its own GL buffer), because a fat
 *    line is instanced per segment and `drawRange` has no meaning on it.
 *
 * What is GONE is the old shared-attribute wrapper: a `BufferGeometry` that
 * borrowed the body's position/normal/index with a narrowed `drawRange`. It
 * owned no buffers, so it could never be disposed — and three's `WebGLGeometries`
 * releases a geometry's binding state only on that geometry's own `dispose`
 * event, so every hover left renderer bookkeeping behind forever (R02).
 *
 * Owned overlays live in a bounded LRU (`HighlightCache`, 64 MiB / 256 entries),
 * pinned while displayed and retired with their source resource. If the pinned
 * set alone exhausts the budget the SEMANTIC selection is kept and that body is
 * drawn with the whole-body leased overlay instead, with one diagnostic per
 * change (spec §8.3 degraded mode).
 *
 * `setState` (selection change) and `refresh` (after a mesh swap) rebuild
 * against the CURRENT registry by set difference: unchanged overlays keep their
 * objects and their pins, and only the difference is built or released.
 */
import * as THREE from "three";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";
import type { EntityRef } from "@/stores/selectionStore";
import {
  acquireLease,
  getEntry,
  onEntryRetired,
  type MeshEntry,
  type MeshLease,
} from "../mesh/meshRegistry";
import { identityKey } from "../mesh/renderResourceIdentity";
import {
  buildFaceSetGeometry,
  estimateFaceSetBytes,
  type OwnedFaceGeometry,
} from "../mesh/faceSliceGeometry";
import { HighlightCache, highlightCacheKey } from "../mesh/highlightCache";
import { ordinalForRef } from "../mesh/rebindPick";
import { palette } from "./palette";
import { createScreenLineMaterial, LINE_WIDTHS_CSS } from "./screenLineStyle";
import { RENDER_ORDER } from "./renderOrder";

// ── Pure slice math (unit-tested) ───────────────────────────────────────────

/** Face ordinal → indexed-geometry drawRange (index units: 3 per triangle). */
export function faceDrawRange(
  faceRanges: Uint32Array,
  faceOrdinal: number,
): { start: number; count: number } {
  return { start: faceRanges[faceOrdinal * 2] * 3, count: faceRanges[faceOrdinal * 2 + 1] * 3 };
}

/**
 * Edge ordinal → its span of the entry's segment buffer, in SEGMENT units.
 *
 * Segments, not vertices: a fat-line geometry is instanced one-per-segment, and
 * the slice is taken as `positions.subarray(first * 6, (first + count) * 6)`
 * (6 floats = two xyz endpoints).
 */
export function edgeSegmentSlice(
  segRanges: Uint32Array,
  edgeOrdinal: number,
): { first: number; count: number } {
  return { first: segRanges[edgeOrdinal * 2], count: segRanges[edgeOrdinal * 2 + 1] };
}

export interface HighlightDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

const HOVER_OPACITY = 0.45;
const SELECT_OPACITY = 0.55;
/** A whole selected body is a large area — the same tint as one face would shout. */
const SELECT_BODY_OPACITY = 0.45;
/**
 * Highlight edges are drawn heavier than a body edge so they read THROUGH it:
 * the spec §7.3 selection halo, in CSS px, fed to `linewidth` unscaled
 * (`screenLineStyle.ts`).
 */
const HIGHLIGHT_EDGE_WIDTH_CSS = LINE_WIDTHS_CSS.selectionHalo;

type Role = "hover" | "selected";

/** One overlay the current state asks for, resolved against the live registry. */
interface Desired {
  readonly displayKey: string;
  readonly role: Role;
  readonly kind: "faceSet" | "edge" | "body";
  readonly entry: MeshEntry;
  /** Face ordinals (faceSet), the single edge ordinal (edge), or empty (body). */
  readonly ordinals: readonly number[];
  /** Cache slot key for the owned kinds; null for the leased whole-body kind. */
  readonly cacheKey: string | null;
  /** `role|bodyId` — the identity of the reusable per-body selection buffer. */
  readonly slotKey: string | null;
}

/** One overlay currently attached to the interaction root. */
interface Overlay {
  readonly object: THREE.Object3D;
  readonly entry: MeshEntry;
  /** Pinned cache slot, released on detach. */
  readonly cacheKey: string | null;
  /** Registry borrow, released on detach. */
  readonly lease: MeshLease | null;
  readonly slotKey: string | null;
}

export class HighlightLayer {
  private hover: EntityRef | null = null;
  private selected: EntityRef[] = [];
  /** Attached overlays by display key (role + kind + resource identity + ordinals). */
  private displayed = new Map<string, Overlay>();
  /** `role|bodyId` → the cache key of that body's reusable selection buffer. */
  private faceSetSlots = new Map<string, string>();
  private readonly cache = new HighlightCache();
  private readonly unsubscribeRetire: () => void;

  // Shared materials (layer-owned — the only thing disposed on teardown).
  private readonly hoverFaceMat: THREE.MeshBasicMaterial;
  private readonly selFaceMat: THREE.MeshBasicMaterial;
  private readonly selBodyMat: THREE.MeshBasicMaterial;
  private readonly hoverEdgeMat: LineMaterial;
  private readonly selEdgeMat: LineMaterial;

  constructor(private readonly deps: HighlightDeps) {
    const faceOverlay = {
      transparent: true,
      depthWrite: false,
      polygonOffset: true,
      polygonOffsetFactor: -1,
      polygonOffsetUnits: -1,
      side: THREE.DoubleSide,
      toneMapped: false,
    } as const;
    // Hover is the CYAN viewport token, not the UI accent: the accent is also
    // the preview/handle color, so hovering a face used to read as a preview.
    this.hoverFaceMat = new THREE.MeshBasicMaterial({
      color: palette.hover3d(),
      opacity: HOVER_OPACITY,
      ...faceOverlay,
    });
    this.selFaceMat = new THREE.MeshBasicMaterial({
      color: palette.selected3d(),
      opacity: SELECT_OPACITY,
      ...faceOverlay,
    });
    // Same token as selFaceMat, lighter — a whole body's worth of tint at the
    // per-face opacity buries the shading that tells you what shape it is.
    this.selBodyMat = new THREE.MeshBasicMaterial({
      color: palette.selected3d(),
      opacity: SELECT_BODY_OPACITY,
      ...faceOverlay,
    });
    const edgeOverlay = {
      // depthTest off: a highlighted edge lies exactly on the body edge it
      // replaces and must win outright. transparent keeps it in the same
      // (transparent) list as the face overlays, ordered by RENDER_ORDER.
      depthTest: false,
      transparent: true,
      toneMapped: false,
    } as const;
    const haloStyle = { widthCss: HIGHLIGHT_EDGE_WIDTH_CSS } as const;
    this.hoverEdgeMat = createScreenLineMaterial(haloStyle, {
      color: palette.hover3d().getHex(),
      ...edgeOverlay,
    });
    this.selEdgeMat = createScreenLineMaterial(haloStyle, {
      color: palette.selectedEdge().getHex(),
      ...edgeOverlay,
    });

    // An overlay cut from a resource that just retired is stale geometry, so
    // the cache drops it and — if it was on screen — the layer rebuilds against
    // whatever replaced it. The registry has already published the replacement
    // by the time this fires, so the rebuild sees the new resource.
    this.unsubscribeRetire = onEntryRetired((entry) => {
      const wasDisplayed = this.usesEntry(entry);
      this.cache.retireSource(entry);
      if (wasDisplayed) this.rebuild();
    });
  }

  /**
   * Section view: clip every overlay material (`null` = unclipped).
   *
   * These five materials are OUTSIDE both `BodyMaterialLibrary` instances, so
   * the body fan-out cannot reach them — and an unclipped highlight is worse
   * than a merely inconsistent one: the edge overlays are `depthTest: false`,
   * so a face selected BEFORE the cut paints its tint in mid-air, in the empty
   * space where the removed half used to be.
   */
  setClippingPlanes(planes: THREE.Plane[] | null): void {
    for (const mat of [
      this.hoverFaceMat,
      this.selFaceMat,
      this.selBodyMat,
      this.hoverEdgeMat,
      this.selEdgeMat,
    ] as THREE.Material[]) {
      // Only a COUNT change recompiles (the plane count is baked into the
      // shader); moving a plane is picked up per frame with no material write.
      const before = mat.clippingPlanes?.length ?? 0;
      mat.clippingPlanes = planes;
      if (before !== (planes?.length ?? 0)) mat.needsUpdate = true;
    }
    this.deps.invalidate();
  }

  setState(hover: EntityRef | null, selected: EntityRef[]): void {
    this.hover = hover;
    this.selected = selected;
    this.rebuild();
  }

  /** Rebuild against the current registry (call after a mesh swap). */
  refresh(): void {
    this.rebuild();
  }

  /**
   * Theme change: re-read the palette into the five shared materials.
   * `rebuild()` only recreates OBJECTS — it reuses these materials, so it can
   * never pick up a new theme on its own.
   */
  refreshColors(): void {
    this.hoverFaceMat.color.copy(palette.hover3d());
    this.selFaceMat.color.copy(palette.selected3d());
    this.selBodyMat.color.copy(palette.selected3d());
    this.hoverEdgeMat.color.copy(palette.hover3d());
    this.selEdgeMat.color.copy(palette.selectedEdge());
  }

  /** Owned-overlay accounting, for the resource tests (ACCEPTANCE §3.3). */
  resourceStats(): { entries: number; bytes: number; displayed: number } {
    const stats = this.cache.stats();
    return { entries: stats.entries, bytes: stats.bytes, displayed: this.displayed.size };
  }

  /** True when the last selection change had to fall back to a degraded overlay. */
  get degraded(): boolean {
    return this.cache.degraded;
  }

  dispose(): void {
    this.unsubscribeRetire();
    this.clearObjects();
    this.cache.clear();
    this.faceSetSlots.clear();
    this.hoverFaceMat.dispose();
    this.selFaceMat.dispose();
    this.selBodyMat.dispose();
    this.hoverEdgeMat.dispose();
    this.selEdgeMat.dispose();
  }

  // ── rebuild ────────────────────────────────────────────────────────────────

  /**
   * Set difference against what is already attached: detach what left (which
   * unpins and releases), keep what stayed, build only what is new.
   */
  private rebuild(): void {
    this.cache.beginChange();
    const desired = this.collectDesired();
    const prevSlots = this.faceSetSlots;
    this.faceSetSlots = new Map();

    for (const [key, overlay] of [...this.displayed]) {
      if (desired.has(key)) continue;
      this.detach(overlay);
      this.displayed.delete(key);
    }
    // A survivor keeps its buffer, so its slot must carry forward or the next
    // change would allocate instead of growing this one.
    for (const overlay of this.displayed.values()) {
      if (overlay.slotKey && overlay.cacheKey) this.faceSetSlots.set(overlay.slotKey, overlay.cacheKey);
    }
    for (const [key, want] of desired) {
      if (this.displayed.has(key)) continue;
      const overlay = this.create(want, prevSlots);
      if (overlay) this.displayed.set(key, overlay);
    }
    this.deps.invalidate();
  }

  /**
   * Resolve (hover, selected) into the overlay set the scene should hold.
   *
   * Face refs of one body and one role collapse into a SINGLE descriptor, which
   * is what makes a multi-face selection one owned buffer rather than N.
   */
  private collectDesired(): Map<string, Desired> {
    const out = new Map<string, Desired>();
    const faceSets = new Map<string, { role: Role; entry: MeshEntry; ordinals: number[] }>();

    const addFace = (role: Role, entry: MeshEntry, ordinal: number): void => {
      const key = `${role}|${entry.bodyId}`;
      const set = faceSets.get(key);
      if (set) {
        if (!set.ordinals.includes(ordinal)) set.ordinals.push(ordinal);
      } else {
        faceSets.set(key, { role, entry, ordinals: [ordinal] });
      }
    };

    const addRef = (ref: EntityRef, role: Role): void => {
      if (ref.kind === "face" || ref.kind === "edge") {
        if (!ref.bodyId || !ref.topoKey) return;
        const entry = getEntry(ref.bodyId);
        if (!entry) return;
        if (ref.kind === "face") {
          // Through `ordinalForRef`, not the raw TopoKey: a ref that survived a
          // regen may only be nameable by its promoted ElementId (rebindPick).
          const ord = ordinalForRef(entry.faceIndex, entry.view, ref);
          if (ord >= 0) addFace(role, entry, ord);
          return;
        }
        if (!entry.edgeSegmentPositions || !entry.edgeIndex || !entry.edgeSegmentRanges) return;
        const ord = ordinalForRef(entry.edgeIndex, entry.view, ref);
        if (ord < 0) return;
        if (edgeSegmentSlice(entry.edgeSegmentRanges, ord).count === 0) return;
        const cacheKey = highlightCacheKey(identityKey(entry.identity), "edge", [ord]);
        const displayKey = `${role}|${cacheKey}`;
        out.set(displayKey, {
          displayKey,
          role,
          kind: "edge",
          entry,
          ordinals: [ord],
          cacheKey,
          slotKey: null,
        });
        return;
      }
      if (ref.kind === "body") {
        const entry = getEntry(ref.id);
        if (!entry) return;
        const displayKey = `${role}|body|${identityKey(entry.identity)}`;
        out.set(displayKey, {
          displayKey,
          role,
          kind: "body",
          entry,
          ordinals: [],
          cacheKey: null,
          slotKey: null,
        });
      }
      // sketch / feature refs have no viewport geometry.
    };

    for (const ref of this.selected) addRef(ref, "selected");
    if (
      this.hover &&
      !this.selected.some((r) => r.id === this.hover!.id && r.kind === this.hover!.kind)
    ) {
      addRef(this.hover, "hover");
    }

    for (const [slotKey, set] of faceSets) {
      const ordinals = [...set.ordinals].sort((a, b) => a - b);
      const cacheKey = highlightCacheKey(identityKey(set.entry.identity), "faceSet", ordinals);
      const displayKey = `${set.role}|${cacheKey}`;
      out.set(displayKey, {
        displayKey,
        role: set.role,
        kind: "faceSet",
        entry: set.entry,
        ordinals,
        cacheKey,
        slotKey,
      });
    }
    return out;
  }

  private create(want: Desired, prevSlots: Map<string, string>): Overlay | null {
    if (want.kind === "body") return this.attachBody(want);
    const cacheKey = want.cacheKey!;
    let value = this.cache.get(cacheKey);
    if (!value) {
      value = want.kind === "faceSet" ? this.buildFaceSet(want, prevSlots) : this.buildEdge(want);
      // Budget exhausted by the pinned set: keep the semantic selection and
      // draw this body whole instead of dropping what the user selected.
      if (!value) return this.attachBody(want);
    }
    this.cache.pin(cacheKey);
    const object =
      want.kind === "faceSet"
        ? this.faceMesh(value.geometry, want)
        : this.edgeLine(value.geometry, want);
    this.deps.root.add(object);
    if (want.slotKey) this.faceSetSlots.set(want.slotKey, cacheKey);
    return {
      object,
      entry: want.entry,
      cacheKey,
      lease: null,
      slotKey: want.slotKey,
    };
  }

  /** Compact owned face buffer, reusing this (body, role)'s previous one. */
  private buildFaceSet(want: Desired, prevSlots: Map<string, string>) {
    const previousKey = want.slotKey ? prevSlots.get(want.slotKey) : undefined;
    const taken = previousKey ? this.cache.take(previousKey) : undefined;
    const reuse: OwnedFaceGeometry | undefined = taken?.owned ?? undefined;
    if (!this.cache.reserve(estimateFaceSetBytes(want.entry, want.ordinals), "faceSet")) {
      taken?.owned?.dispose();
      return undefined;
    }
    const owned = buildFaceSetGeometry(want.entry, want.ordinals, reuse);
    return this.cache.put(want.cacheKey!, {
      kind: "faceSet",
      geometry: owned.geometry,
      owned,
      bytes: owned.bytes,
      entry: want.entry,
      pinned: 0,
    });
  }

  /**
   * Owned segment geometry for ONE edge. The positions are a `subarray` VIEW of
   * the entry's expanded segment buffer — zero CPU copy — but `setPositions`
   * builds this object its own InstancedInterleavedBuffer, and therefore its
   * own GL buffer, which is what makes it owned and disposable.
   */
  private buildEdge(want: Desired) {
    const entry = want.entry;
    const { first, count } = edgeSegmentSlice(entry.edgeSegmentRanges!, want.ordinals[0]);
    const slice = entry.edgeSegmentPositions!.subarray(first * 6, (first + count) * 6);
    if (!this.cache.reserve(slice.byteLength, "edge")) return undefined;
    const geometry = new LineSegmentsGeometry();
    geometry.setPositions(slice);
    return this.cache.put(want.cacheKey!, {
      kind: "edge",
      geometry,
      owned: null,
      bytes: slice.byteLength,
      entry,
      pinned: 0,
    });
  }

  /**
   * The whole-body overlay: the EXACT registry geometry object under a lease.
   * No wrapper, no copy — and therefore nothing here to dispose.
   */
  private attachBody(want: Desired): Overlay | null {
    const lease = acquireLease(want.entry, "highlight:body");
    if (lease.entry.resourceState === "disposed") {
      lease.release();
      return null;
    }
    const mesh = new THREE.Mesh(
      want.entry.geometry,
      want.role === "hover" ? this.hoverFaceMat : this.selBodyMat,
    );
    mesh.renderOrder = RENDER_ORDER.HIGHLIGHT_FACE;
    mesh.userData.bodyId = want.entry.bodyId;
    this.deps.root.add(mesh);
    return { object: mesh, entry: want.entry, cacheKey: null, lease, slotKey: null };
  }

  private faceMesh(geometry: THREE.BufferGeometry, want: Desired): THREE.Mesh {
    const mesh = new THREE.Mesh(
      geometry,
      want.role === "hover" ? this.hoverFaceMat : this.selFaceMat,
    );
    mesh.renderOrder = RENDER_ORDER.HIGHLIGHT_FACE;
    mesh.userData.bodyId = want.entry.bodyId;
    // The topology ordinals stay EXTERNAL to the compact vertex numbering.
    mesh.userData.faceOrdinals = want.ordinals;
    return mesh;
  }

  private edgeLine(geometry: THREE.BufferGeometry, want: Desired): LineSegments2 {
    const line = new LineSegments2(
      geometry as LineSegmentsGeometry,
      want.role === "hover" ? this.hoverEdgeMat : this.selEdgeMat,
    );
    line.renderOrder = RENDER_ORDER.HIGHLIGHT_EDGE;
    line.userData.bodyId = want.entry.bodyId;
    line.userData.edgeOrdinals = want.ordinals;
    return line;
  }

  private usesEntry(entry: MeshEntry): boolean {
    for (const overlay of this.displayed.values()) if (overlay.entry === entry) return true;
    return false;
  }

  /**
   * Detach one overlay: off the root, unpin its cache slot, release its lease.
   * It never disposes: an owned overlay's lifetime belongs to the cache (which
   * may serve it again on the next hover), and a leased one owns nothing.
   */
  private detach(overlay: Overlay): void {
    this.deps.root.remove(overlay.object);
    if (overlay.cacheKey) this.cache.unpin(overlay.cacheKey);
    overlay.lease?.release();
  }

  private clearObjects(): void {
    for (const overlay of this.displayed.values()) this.detach(overlay);
    this.displayed.clear();
  }
}
