/*
 * Picker — rAF-coalesced raycast over body face meshes + fat edge LineSegments2.
 *
 * Hover follows the pointer (coalesced to one raycast per frame) and fires only
 * when the hit CHANGES, so an idle pointer schedules no frames (render-on-demand
 * preserved). Edges win over faces within a screen-space tolerance: an edge hit
 * is preferred when it is no farther than the face hit (+slack) — so a boundary
 * edge lying on a face surface is selectable, while an occluded edge is not.
 *
 * BOTH halves of that rule are expressed in SCREEN pixels (D9, finding T8). The
 * first half is three's own Line2 screen-space raycast, which only reports an
 * edge within {@link EDGE_PICK_PX} CSS px of the cursor. The second is the depth
 * window: {@link EDGE_DEPTH_SLACK_PX} CSS px of world size AT THE FACE HIT DEPTH
 * ({@link pixelWorldSize}). The slack used to be `linePickThreshold` at the
 * camera's ORBIT-TARGET distance, which is not the depth anything was hit at —
 * so a panned or zoomed camera made the same click land on the edge or on the
 * face depending on where the orbit target happened to sit. That produced both
 * reported symptoms: edges unpickable at one camera (T8) and faces unpickable at
 * another (the session-32 inverse).
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
 * `params.Line.threshold` (world units) is still driven for any PLAIN THREE.Line
 * that may be gathered; the fat-line path ignores it.
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
import { isInteractiveBoundary } from "@/ui/interactiveBoundary";

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

export type ProbeCandidateKind = "body" | "face" | "edge";
export interface ProbeCandidateFilter {
  kinds?: readonly ProbeCandidateKind[];
  includeBodyIds?: readonly string[];
  excludeBodyIds?: readonly string[];
}
export interface ProbeCandidate extends Omit<PickHit, "kind"> {
  kind: ProbeCandidateKind;
  meshRev?: number;
  entryIdentity?: { bodyId: string; meshRev: number };
  /** Exact installed object captured at probe time; local proof, never IPC. */
  entry?: MeshEntry;
}

export interface PickModifiers {
  shift: boolean;
  meta: boolean;
  /** Pick-through: body face/edge wins over a coplanar sketch fill lying on it. */
  alt: boolean;
}

/**
 * How close, in CSS px, the cursor must be to a body edge for that edge to be a
 * pick candidate at all. Raised 6 → 8 (T8): three's Line2 raycast enforces this
 * exactly — `line2PickThreshold` cancels the drawn width out — so it IS the
 * screen-distance half of the edge-vs-face rule, and five real-user clicks aimed
 * at 1–3 px accuracy still missing means the aiming target was too small.
 */
export const EDGE_PICK_PX = 8;

/**
 * `k` in D9: the edge-vs-face depth window, in CSS px of world size AT THE FACE
 * HIT DEPTH. An edge that is within {@link EDGE_PICK_PX} of the cursor and no
 * more than this far BEHIND the face under the cursor wins; anything deeper is
 * occluded geometry and the face keeps the pick.
 *
 * Two pixels, not one: a boundary edge and its own face tie in depth only in
 * exact arithmetic, and the two hits come from different tessellations (edge
 * polylines are sampled off the curve, faces are chordal triangles), so a
 * grazing view puts a genuine boundary edge up to about a pixel behind its face.
 */
export const EDGE_DEPTH_SLACK_PX = 2;

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
 * The world size of ONE CSS pixel at ray depth `depth`, for both camera kinds.
 *
 * Perspective: `2·depth·tan(fov/2) / viewportHeight`. Orthographic: the frustum
 * height over the viewport height (depth-independent, which is the point — an
 * ortho camera has one pixel size everywhere).
 *
 * Deliberately `linePickThreshold(…, 1)` rather than a second copy of the same
 * trigonometry: one screen→world conversion in this file, two callers.
 */
export function pixelWorldSize(
  camera: THREE.Camera,
  viewportHeight: number,
  depth: number,
): number {
  return linePickThreshold(camera, viewportHeight, depth, 1);
}

/**
 * D9's depth window: how far BEHIND the face hit an edge may sit and still win.
 *
 * Measured at the FACE hit's own depth, which is what makes the answer
 * camera-pose invariant — the same click on the same pixel of the same body
 * arbitrates identically however the orbit target has been panned or zoomed.
 * `faceDepth` is the caller's fallback (the focus distance) when nothing was
 * hit, where the value is unused anyway.
 */
export function edgeDepthSlack(
  camera: THREE.Camera,
  viewportHeight: number,
  faceDepth: number,
): number {
  return EDGE_DEPTH_SLACK_PX * pixelWorldSize(camera, viewportHeight, faceDepth);
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
 * Prefer the edge hit when it is no farther than the face hit plus `slack`
 * (edges lie on face boundaries, so distances tie; the slack covers float wobble
 * and lets a coincident boundary edge win). An occluded edge (much farther)
 * loses, and so does a face-interior click, where three reports no edge at all.
 *
 * A SILHOUETTE edge — one with no face behind it inside the depth window, and in
 * particular none at all — always wins, because `faceHit` is null there.
 *
 * `slack` comes from {@link edgeDepthSlack}; this function stays a pure
 * comparison so the rule and the screen→world conversion are tested apart.
 */
export function choosePreferredHit(
  faceHit: THREE.Intersection | null,
  edgeHit: THREE.Intersection | null,
  slack: number,
): { hit: THREE.Intersection; kind: "face" | "edge" } | null {
  if (edgeHit && (!faceHit || edgeHit.distance <= faceHit.distance + slack)) {
    return { hit: edgeHit, kind: "edge" };
  }
  if (faceHit) return { hit: faceHit, kind: "face" };
  return null;
}

/**
 * The first intersection that the section view has NOT cut away.
 *
 * three discards a fragment whose signed distance to a clipping plane is
 * negative, so a hit on the far side of the cut is geometry the user cannot
 * see. Taking `[0]` regardless is what would make an invisible face pickable —
 * and worse, would keep the newly exposed INTERIOR face unselectable, which is
 * the whole point of cutting. The anchor tested is the one that lies on the
 * geometry (`pointOnLine` for a fat line, `point` for a triangle), the same
 * choice `resolvePick` makes.
 *
 * `planes` null/empty ⇒ plain `[0]`, i.e. exactly the previous behavior.
 */
export function firstUnclippedHit(
  hits: readonly THREE.Intersection[],
  planes: readonly THREE.Plane[] | null,
): THREE.Intersection | null {
  if (!planes || planes.length === 0) return hits[0] ?? null;
  for (const hit of hits) {
    const at = hit.pointOnLine ?? hit.point;
    if (planes.every((p) => p.distanceToPoint(at) >= 0)) return hit;
  }
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

  // IDS_HAVE_ELEMENTIDS is body-wide: one tracked element sets it even while
  // untracked labels remain snapshot-scoped `f:N` / `e:N` TopoKeys. Classify the
  // chosen label itself using Rust's `el_` ElementId contract.
  const isElementId = id.startsWith("el_");
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
  /**
   * Live section-view clipping planes, or null when the section is off. Injected
   * as a GETTER (the `getCamera`/`getRoot` pattern) because the engine mutates
   * the plane in place as the offset slider moves.
   */
  getClippingPlanes?: () => THREE.Plane[] | null;
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

  /**
   * One-shot pick regardless of `isActive` — for tools that own picking directly
   * (boolean tool-body pick). Returns the resolved hit or null.
   */
  probe(clientX: number, clientY: number): PickHit | null {
    return this.pickAt(clientX, clientY);
  }

  /** Ordered, read-only overlap enumeration. Does not affect hover or selection. */
  probeCandidates(
    clientX: number,
    clientY: number,
    filter: ProbeCandidateFilter = {},
  ): ProbeCandidate[] {
    const allowed = new Set<ProbeCandidateKind>(filter.kinds ?? ["face", "edge"]);
    if ([...allowed].some((kind) => kind !== "body" && kind !== "face" && kind !== "edge")) {
      return [];
    }
    const include = filter.includeBodyIds ? new Set(filter.includeBodyIds) : null;
    const exclude = new Set(filter.excludeBodyIds ?? []);
    const raw = this.raycastAll(clientX, clientY, allowed.has("edge"));
    if (!raw) return [];
    const candidates: ProbeCandidate[] = [];
    if (allowed.has("face") || allowed.has("body")) {
      for (const hit of raw.faceHits) {
        const face = resolvePick(hit, "face");
        if (!face || (include && !include.has(face.bodyId)) || exclude.has(face.bodyId)) continue;
        const entry = getEntry(face.bodyId);
        const candidate = {
          ...face,
          meshRev: entry?.meshRev,
          entryIdentity: entry ? { bodyId: face.bodyId, meshRev: entry.meshRev } : undefined,
          entry,
        };
        if (allowed.has("face")) candidates.push(candidate);
        if (allowed.has("body")) {
          candidates.push({ ...candidate, kind: "body", topoKey: face.bodyId, elementId: undefined });
        }
      }
    }
    if (allowed.has("edge")) {
      for (const hit of raw.edgeHits) {
        const edge = resolvePick(hit, "edge");
        if (!edge || (include && !include.has(edge.bodyId)) || exclude.has(edge.bodyId)) continue;
        const entry = getEntry(edge.bodyId);
        candidates.push({
          ...edge,
          meshRev: entry?.meshRev,
          entryIdentity: entry ? { bodyId: edge.bodyId, meshRev: entry.meshRev } : undefined,
          entry,
        });
      }
    }
    const nearest = new Map<string, ProbeCandidate>();
    for (const candidate of candidates) {
      const key = `${candidate.bodyId}/${candidate.kind}/${candidate.topoKey}`;
      const prior = nearest.get(key);
      if (!prior || candidate.distance < prior.distance) nearest.set(key, candidate);
    }
    return [...nearest.values()].sort((a, b) => {
      const ad = a.distance - (a.kind === "edge" ? raw.edgeDepthSlack : 0);
      const bd = b.distance - (b.kind === "edge" ? raw.edgeDepthSlack : 0);
      const rank = (kind: ProbeCandidateKind) => kind === "edge" ? 0 : kind === "face" ? 1 : 2;
      return ad - bd || rank(a.kind) - rank(b.kind) ||
        a.bodyId.localeCompare(b.bodyId) || a.topoKey.localeCompare(b.topoKey);
    });
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
    if (isInteractiveBoundary(e)) {
      this.downButton = -1;
      return;
    }
    this.downX = e.clientX;
    this.downY = e.clientY;
    this.downButton = e.button;
    this.moved = false;
  };

  private onPointerUp = (e: PointerEvent): void => {
    if (isInteractiveBoundary(e)) {
      this.downButton = -1;
      return;
    }
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
    const all = this.raycastAll(clientX, clientY, true);
    if (!all) return null;
    const faceHit = all.faceHits[0] ?? null;
    const edgeHit = all.edgeHits[0] ?? null;
    return choosePreferredHit(faceHit, edgeHit, all.edgeDepthSlack);
  }

  private raycastAll(clientX: number, clientY: number, includeEdges: boolean): {
    faceHits: THREE.Intersection[];
    edgeHits: THREE.Intersection[];
    /** D9 depth window for the edge-vs-face rule, world units. */
    edgeDepthSlack: number;
  } | null {
    const rect = this.deps.canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;
    const ndc = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -(((clientY - rect.top) / rect.height) * 2 - 1),
    );
    const camera = this.deps.getCamera();
    this.raycaster.setFromCamera(ndc, camera);
    // Plain THREE.Line only (nothing under bodiesRoot is one today); the fat-line
    // path ignores `params.Line` and the edge-vs-face rule no longer reads it.
    this.raycaster.params.Line = {
      threshold: linePickThreshold(
        camera,
        this.deps.getViewportHeight(),
        this.deps.getFocusDistance(),
        EDGE_PICK_PX,
      ),
    };
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
      else if (includeEdges && o.userData.kind === "edge") edgeObjects.push(o);
    });
    this.flushEdgeResolution(edgeObjects);

    // The WHOLE sorted array, not `[0]`: under a section cut the nearest hit is
    // routinely on the half that was clipped away.
    const planes = this.deps.getClippingPlanes?.() ?? null;
    const visible = (hits: THREE.Intersection[]) => !planes || planes.length === 0
      ? hits
      : hits.filter((hit) => planes.every((p) =>
          p.distanceToPoint(hit.pointOnLine ?? hit.point) >= 0));
    const faceHits = visible(this.raycaster.intersectObjects(faceObjects, false));
    const edgeHits = visible(this.raycaster.intersectObjects(edgeObjects, false));
    return {
      faceHits,
      edgeHits,
      // At the FACE's depth, not the camera's orbit distance (D9). The focus
      // distance survives only as the no-face fallback, where the slack is unused.
      edgeDepthSlack: edgeDepthSlack(
        camera,
        this.deps.getViewportHeight(),
        faceHits[0]?.distance ?? this.deps.getFocusDistance(),
      ),
    };
  }

  /**
   * Push the current drawing-buffer size into every gathered edge material.
   *
   * `LineSegments2.onBeforeRender` keeps `resolution` current for DRAWING, but
   * its raycast bails out silently when `resolution` is still (0,0) — which is
   * exactly the state of a body whose first frame has not rendered yet (a
   * pick on pointerdown can arrive first). Mirrors
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
