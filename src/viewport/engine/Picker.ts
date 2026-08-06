/*
 * Picker — rAF-coalesced raycast over body face meshes + fat edge LineSegments2.
 *
 * Hover follows the pointer (coalesced to one raycast per frame) and fires only
 * when the hit CHANGES, so an idle pointer schedules no frames (render-on-demand
 * preserved). Edges win over faces within a screen-space tolerance: an edge hit
 * is preferred when it is no farther than the face hit (+bias) — so a boundary
 * edge lying on a face surface is selectable, while an occluded edge is not.
 *
 * Body edges are LineSegments2 (P3), whose raycast is SCREEN-SPACE: the hit
 * radius is `(material.linewidth + params.Line2.threshold) / 2` in the DEVICE px
 * `material.resolution` is expressed in. Two consequences this class owns:
 *   - the threshold is derived from the drawn edge width so the effective radius
 *     lands on EDGE_PICK_PX CSS px regardless of the line's weight or the dpr
 *     (see {@link line2PickThreshold}), and
 *   - `material.resolution` is flushed before every raycast, because a raycast
 *     that happens before the first render would otherwise see (0,0) and return
 *     silently — no hits, no error.
 * `params.Line.threshold` (world units) is still driven: LineSegments2 reports a
 * world-space `distance`, so it remains the right unit for the face-vs-edge bias.
 *
 * A hit resolves through the mesh registry: triangle index → face id, or segment
 * ordinal → edge id (both lazy TopoKey/ElementId decode). Click captures the
 * world anchor for a future AcquireElementIds promotion. The engine owns store
 * wiring via the onHover/onPick callbacks (this class stays store-agnostic).
 */
import * as THREE from "three";
import type { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import type { MeshEntry } from "../mesh/meshRegistry";
import { getEntry } from "../mesh/meshRegistry";
import { BODY_EDGE_WIDTH } from "./bodyMaterials";
import { MAX_DPR } from "./SketchObject";

export interface PickHit {
  bodyId: string;
  kind: "face" | "edge";
  topoKey: string;
  elementId?: string;
  /** Camera-ray distance used to arbitrate against coplanar sketch profiles. */
  distance: number;
  worldPos: THREE.Vector3;
  /** Local hint for previews/promotion (face normal in world space). */
  surfaceHint?: { normal?: [number, number, number] };
}

export interface PickModifiers {
  shift: boolean;
  meta: boolean;
  /** Pick-through: body face/edge wins over a coplanar sketch fill lying on it. */
  alt: boolean;
}

const EDGE_PICK_PX = 6;
const DRAG_PX = 4; // pointer travel over this ⇒ a drag, not a click

// ── Pure helpers (unit-tested) ──────────────────────────────────────────────

/** World units spanning `px` screen pixels at `focusDistance`, for both cameras. */
export function linePickThreshold(
  camera: THREE.Camera,
  viewportHeight: number,
  focusDistance: number,
  px = EDGE_PICK_PX,
): number {
  const h = Math.max(viewportHeight, 1);
  if ((camera as THREE.PerspectiveCamera).isPerspectiveCamera) {
    const fov = (camera as THREE.PerspectiveCamera).fov;
    const worldPerPx = (2 * focusDistance * Math.tan((fov * Math.PI) / 360)) / h;
    return px * worldPerPx;
  }
  const oc = camera as THREE.OrthographicCamera;
  const worldPerPx = (oc.top - oc.bottom) / h;
  return px * worldPerPx;
}

/**
 * `raycaster.params.Line2.threshold` that makes a LineSegments2 pick radius
 * exactly `px` CSS pixels.
 *
 * LineSegments2 tests `distance < (material.linewidth + threshold) / 2` in
 * device px, so the drawn width has to be SUBTRACTED out — otherwise a fatter
 * edge would silently pick wider than a thin one. Clamped at 0: a line drawn
 * wider than the tolerance already picks at its own half-width, and a negative
 * threshold would be read as `0` by three anyway (`threshold || 0`).
 */
export function line2PickThreshold(
  dpr: number,
  edgeWidthDevice: number,
  px = EDGE_PICK_PX,
): number {
  return Math.max(0, 2 * px * dpr - edgeWidthDevice);
}

/** The renderer's capped device-pixel ratio — the units fat-line widths use. */
function cappedDpr(): number {
  return Math.min(typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1, MAX_DPR);
}

/**
 * Prefer the edge hit when it is no farther than the face hit plus `bias`
 * (edges lie on face boundaries, so distances tie; bias covers float wobble and
 * lets a coincident boundary edge win). An occluded edge (much farther) loses.
 */
export function choosePreferredHit(
  faceHit: THREE.Intersection | null,
  edgeHit: THREE.Intersection | null,
  bias: number,
): { hit: THREE.Intersection; kind: "face" | "edge" } | null {
  if (edgeHit && (!faceHit || edgeHit.distance <= faceHit.distance + bias)) {
    return { hit: edgeHit, kind: "edge" };
  }
  if (faceHit) return { hit: faceHit, kind: "face" };
  return null;
}

/**
 * Whether a secondary sketch hit is in front of, or numerically coplanar with,
 * a body hit. A farther sketch never selects through an occluding body.
 */
export function secondaryHitWins(
  bodyDistance: number,
  sketchDistance: number,
  relativeEpsilon = 1e-5,
): boolean {
  if (!Number.isFinite(sketchDistance)) return false;
  if (!Number.isFinite(bodyDistance)) return true;
  const tolerance =
    Math.max(1, Math.abs(bodyDistance), Math.abs(sketchDistance)) * relativeEpsilon;
  return sketchDistance <= bodyDistance + tolerance;
}

/** Resolve a chosen intersection into a PickHit via the mesh registry. */
export function resolvePick(
  hit: THREE.Intersection,
  kind: "face" | "edge",
  lookup: (bodyId: string) => MeshEntry | undefined = getEntry,
): PickHit | null {
  const bodyId = hit.object.userData.bodyId as string | undefined;
  if (!bodyId) return null;
  const entry = lookup(bodyId);
  if (!entry) return null;

  let id: string | null = null;
  let surfaceHint: PickHit["surfaceHint"];
  // LineSegments2 reports `point` on the RAY and `pointOnLine` on the segment;
  // the anchor we promote must be the one that lies on the geometry.
  let worldPos = hit.point;
  if (kind === "face") {
    if (hit.faceIndex == null) return null;
    id = entry.faceIndex.idAt(hit.faceIndex);
    if (hit.face) {
      const n = hit.face.normal
        .clone()
        .applyNormalMatrix(new THREE.Matrix3().getNormalMatrix(hit.object.matrixWorld))
        .normalize();
      surfaceHint = { normal: [n.x, n.y, n.z] };
    }
  } else {
    // LineSegments2 is INSTANCED — one instance per segment — so its
    // intersection carries `faceIndex` = the segment ordinal directly, and no
    // `index` at all. (A plain LineSegments reported a VERTEX index, which is
    // why this used to shift by one.)
    if (hit.faceIndex == null || !entry.edgeIndex) return null;
    id = entry.edgeIndex.idAt(hit.faceIndex);
    worldPos = hit.pointOnLine ?? hit.point;
  }
  if (id == null) return null;

  const isElementId = entry.view.idsHaveElementIds;
  return {
    bodyId,
    kind,
    topoKey: id,
    elementId: isElementId ? id : undefined,
    distance: hit.distance,
    worldPos: worldPos.clone(),
    surfaceHint,
  };
}

/** Stable key for hover-change detection (internal; separator avoids id chars). */
export function pickKey(hit: PickHit | null): string | null {
  return hit ? `${hit.bodyId}/${hit.kind}/${hit.topoKey}` : null;
}

// ── Controller ──────────────────────────────────────────────────────────────

export interface PickerDeps {
  canvas: HTMLCanvasElement;
  getCamera: () => THREE.Camera;
  getRoot: () => THREE.Object3D; // bodiesRoot
  getViewportHeight: () => number;
  getFocusDistance: () => number;
  /**
   * Drawing-buffer size in DEVICE px (canvas CSS size × capped dpr) — the units
   * a fat line's `material.resolution` is expressed in. Injected rather than
   * read off the renderer so the resolution self-flush below is testable in
   * jsdom, where nothing ever renders.
   */
  getResolution: () => { w: number; h: number };
  invalidate: () => void;
  /** Picking is only live in this mode (model + select); returns false in sketch mode. */
  isActive: () => boolean;
  /**
   * Hover changed (carries the pointer's client coords for a secondary hit-test,
   * plus the live Alt state so the hover tint can match what a click would select).
   */
  onHover: (hit: PickHit | null, clientX: number, clientY: number, alt: boolean) => void;
  /** Click (carries the pointer's client coords for a secondary hit-test). */
  onPick: (hit: PickHit | null, mods: PickModifiers, clientX: number, clientY: number) => void;
  /**
   * Secondary hover token (e.g. a sketch id under the pointer), consulted ONLY when
   * there is no body hit. Folded into the hover key so hover fires when moving
   * between secondary targets in empty space — the body pick path is unchanged.
   */
  secondaryHoverKey?: (clientX: number, clientY: number) => string | null;
}

export class Picker {
  private readonly raycaster = new THREE.Raycaster();
  private lastHoverKey: string | null = null;

  private pendingMove: PointerEvent | null = null;
  private moveScheduled = false;

  private downX = 0;
  private downY = 0;
  private downButton = -1;
  private moved = false;

  constructor(private readonly deps: PickerDeps) {
    const c = deps.canvas;
    c.addEventListener("pointermove", this.onPointerMove);
    c.addEventListener("pointerdown", this.onPointerDown);
    c.addEventListener("pointerup", this.onPointerUp);
    c.addEventListener("pointerleave", this.onPointerLeave);
  }

  /** Orbit hit-test seam: is there pickable geometry under this client point? */
  hasHitAt(clientX: number, clientY: number): boolean {
    if (!this.deps.isActive()) return false;
    return this.raycast(clientX, clientY) != null;
  }

  /**
   * One-shot pick regardless of `isActive` — for tools that own picking directly
   * (boolean tool-body pick). Returns the resolved hit or null.
   */
  probe(clientX: number, clientY: number): PickHit | null {
    return this.pickAt(clientX, clientY);
  }

  // ── pointer handlers ──

  private onPointerMove = (e: PointerEvent): void => {
    if (!this.deps.isActive()) return;
    if (e.buttons !== 0) {
      this.moved = true;
      return; // no hover while dragging
    }
    this.pendingMove = e;
    if (this.moveScheduled) return;
    this.moveScheduled = true;
    requestAnimationFrame(() => {
      this.moveScheduled = false;
      const ev = this.pendingMove;
      this.pendingMove = null;
      if (!ev || !this.deps.isActive()) return;
      this.updateHover(ev.clientX, ev.clientY, ev.altKey);
    });
  };

  private onPointerDown = (e: PointerEvent): void => {
    this.downX = e.clientX;
    this.downY = e.clientY;
    this.downButton = e.button;
    this.moved = false;
  };

  private onPointerUp = (e: PointerEvent): void => {
    const wasClick =
      this.downButton === 0 &&
      e.button === 0 &&
      !this.moved &&
      Math.abs(e.clientX - this.downX) <= DRAG_PX &&
      Math.abs(e.clientY - this.downY) <= DRAG_PX;
    this.downButton = -1;
    if (!wasClick || !this.deps.isActive()) return;
    const hit = this.pickAt(e.clientX, e.clientY);
    this.deps.onPick(
      hit,
      { shift: e.shiftKey, meta: e.metaKey || e.ctrlKey, alt: e.altKey },
      e.clientX,
      e.clientY,
    );
  };

  private onPointerLeave = (e: PointerEvent): void => {
    if (this.lastHoverKey !== null) {
      this.lastHoverKey = null;
      // Pointer left the canvas: coords are off-canvas so the secondary hit-test
      // clears too (the handler resolves nothing under an out-of-bounds point).
      this.deps.onHover(null, e.clientX, e.clientY, e.altKey);
      this.deps.invalidate();
    }
  };

  // ── raycasting ──

  private updateHover(clientX: number, clientY: number, alt: boolean): void {
    const hit = this.pickAt(clientX, clientY);
    // No body hit ⇒ fold the secondary (sketch) token into the key so hover fires
    // when moving between sketches in empty space, but stays quiet over one sketch.
    const key = hit
      ? pickKey(hit)
      : this.secondaryKey(clientX, clientY);
    if (key === this.lastHoverKey) return; // unchanged ⇒ no repaint (idle stays quiet)
    this.lastHoverKey = key;
    this.deps.onHover(hit, clientX, clientY, alt);
    this.deps.invalidate();
  }

  private secondaryKey(clientX: number, clientY: number): string | null {
    const token = this.deps.secondaryHoverKey?.(clientX, clientY);
    return token ? `sk:${token}` : null;
  }

  private pickAt(clientX: number, clientY: number): PickHit | null {
    const chosen = this.raycast(clientX, clientY);
    return chosen ? resolvePick(chosen.hit, chosen.kind) : null;
  }

  /** Raycast faces + edges and apply the edge-preference rule. */
  private raycast(
    clientX: number,
    clientY: number,
  ): { hit: THREE.Intersection; kind: "face" | "edge" } | null {
    const rect = this.deps.canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;
    const ndc = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -(((clientY - rect.top) / rect.height) * 2 - 1),
    );
    const camera = this.deps.getCamera();
    this.raycaster.setFromCamera(ndc, camera);
    const threshold = linePickThreshold(
      camera,
      this.deps.getViewportHeight(),
      this.deps.getFocusDistance(),
    );
    this.raycaster.params.Line = { threshold };
    // Screen-space tolerance for the fat edge lines (see line2PickThreshold).
    this.raycaster.params.Line2 = {
      threshold: line2PickThreshold(cappedDpr(), BODY_EDGE_WIDTH),
    };

    const faceObjects: THREE.Object3D[] = [];
    const edgeObjects: THREE.Object3D[] = [];
    // traverseVisible: an own-flag check misses meshes inside a HIDDEN body
    // group (children keep visible=true) — hidden bodies must not be pickable.
    this.deps.getRoot().traverseVisible((o) => {
      if (o.userData.kind === "face") faceObjects.push(o);
      else if (o.userData.kind === "edge") edgeObjects.push(o);
    });
    this.flushEdgeResolution(edgeObjects);

    const faceHit = this.raycaster.intersectObjects(faceObjects, false)[0] ?? null;
    const edgeHit = this.raycaster.intersectObjects(edgeObjects, false)[0] ?? null;
    return choosePreferredHit(faceHit, edgeHit, threshold);
  }

  /**
   * Push the current drawing-buffer size into every gathered edge material.
   *
   * `LineSegments2.onBeforeRender` keeps `resolution` current for DRAWING, but
   * its raycast bails out silently when `resolution` is still (0,0) — which is
   * exactly the state of a body whose first frame has not rendered yet (an
   * orbit-gating `hasHitAt` on pointerdown can arrive first). Mirrors
   * `SketchStaticLayer.hitTest`'s matrixWorld self-flush: a hit-test must not
   * depend on a render having happened.
   *
   * Materials are SHARED per material kind, so one write per unique material is
   * enough — the Set keeps N bodies from costing N writes.
   */
  private flushEdgeResolution(edgeObjects: readonly THREE.Object3D[]): void {
    if (edgeObjects.length === 0) return;
    const { w, h } = this.deps.getResolution();
    const seen = new Set<LineMaterial>();
    for (const o of edgeObjects) {
      const mat = (o as THREE.Mesh).material as LineMaterial | undefined;
      if (!mat?.resolution || seen.has(mat)) continue;
      mat.resolution.set(w, h);
      seen.add(mat);
    }
  }

  dispose(): void {
    const c = this.deps.canvas;
    c.removeEventListener("pointermove", this.onPointerMove);
    c.removeEventListener("pointerdown", this.onPointerDown);
    c.removeEventListener("pointerup", this.onPointerUp);
    c.removeEventListener("pointerleave", this.onPointerLeave);
    this.lastHoverKey = null;
    this.pendingMove = null;
  }
}
