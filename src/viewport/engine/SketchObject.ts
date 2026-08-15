/*
 * SketchObject — one sketch's presence in `sketchRoot` (F-WP6).
 *
 * Everything is authored in plane (u,v) coordinates inside a group carrying the
 * plane basis matrix (see sketchBasis), so local (u,v,0) → world (Z-up). Under
 * that group:
 *   - a low-alpha plane TINT quad (canvas-sketch token) + an adaptive plane grid
 *     (reused GridPlane, re-centered in plane-local coords),
 *   - committed entities as `Line2` (px line width, addons `lines/` — ships with
 *     three 0.185.1 for the WebGLRenderer), colored by constraint state,
 *   - endpoint/center markers as constant-size `THREE.Points`,
 *   - an in-progress PREVIEW channel (rubber-band) the tool machines drive.
 *
 * Line2 needs the material `resolution` in device pixels for correct width, so
 * the engine calls `update(...)` every frame with the viewport size (already
 * dpr-multiplied by the caller). `linewidth` is therefore also a device-px
 * value — see `cssLineWidth()` below, which the width constants are built
 * from so they read as their CSS-px name on any display.
 *
 * Line2 import note: `three/addons/lines/Line2.js` is WebGLRenderer-only (the
 * WebGPU build lives under `lines/webgpu/`). We are WebGL-default (F-WP4), so the
 * WebGL Line2 is correct here; a WebGPU sketch path is a later concern.
 */
import * as THREE from "three";
import { Line2 } from "three/examples/jsm/lines/Line2.js";
import { LineGeometry } from "three/examples/jsm/lines/LineGeometry.js";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import type { SketchEntity, SketchPlane, SketchSolveStatus } from "@/ipc/types";
import { GridPlane } from "./GridPlane";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import { planeBasisMatrix, worldToPlanePoint, type Point2 } from "./sketchBasis";
import type { DraftEntity } from "@/tools/sketch/toolMachine";
import { ellipseParams, sampleEllipse } from "@/tools/sketch/ellipseMath";
import { angleArcPoints } from "@/tools/sketch/angleArcPreview";
import { buildRingTexture, buildDotTexture } from "./markerTextures";
import { findClosedLoops, loopCentroid } from "@/tools/sketch/closedLoop";
import { layoutDimensionLines } from "@/tools/sketch/dimensionLineLayout";
import type { SketchConstraint } from "@/ipc/types";

const ARC_SEGMENTS = 64;

/**
 * Fat-line `linewidth` (LineMaterial) is measured in DEVICE px — the same
 * units `update()` feeds `resolution` (ViewportEngine calls it with
 * `width*dpr, height*dpr` every frame). A bare CSS-px number here would render
 * half as wide as intended on a 2x display. Widths below are authored in CSS
 * px and multiplied by this cap; the cap value (2) mirrors ViewportEngine's
 * own `MAX_DPR` (renderer.setPixelRatio cap) so a linewidth and the
 * `resolution` it's measured against stay in the same units even past 2x
 * displays. Not imported directly — SketchObject has no reason to depend on
 * ViewportEngine — so keep this value in step with it by hand.
 */
export const MAX_DPR = 2;

/** CSS px → device px for a fat-line `linewidth`, using the shared DPR cap. */
export function cssLineWidth(cssPx: number): number {
  const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
  return cssPx * Math.min(dpr, MAX_DPR);
}

const LINE_WIDTH = cssLineWidth(1.25);
const PREVIEW_WIDTH = cssLineWidth(1);
const TRIM_GHOST_WIDTH = cssLineWidth(2);
/** Selection HALO width — drawn BEHIND the entity's own semantic-color line
 *  (P1 audit fix), wide enough to read as an outline rather than a
 *  slightly-fatter version of the same stroke. Hover has no halo. */
const SELECTION_HALO_WIDTH = cssLineWidth(2.5);
/** The angle-preview arc is a lightweight annotation glyph, not geometry —
 *  thinner than every real entity line so it never competes with them. */
const ANGLE_ARC_WIDTH = cssLineWidth(0.75);
/**
 * Endpoint/midpoint/centroid AFFORDANCE opacity while idle — not permanent
 * document content (Sketcher UX cleanup, Track B1b). The three tiers are NOT
 * equal: an idle rectangle should read as faint corners with no midpoint/
 * centroid clutter at all, matching the progressive-disclosure gate — a flat
 * 0.35 for all three (the earlier value here) put midpoint/centroid dots on
 * screen at the same weight as endpoints even when nothing was selected.
 * `setPointAffordance` restores full opacity per-tier on hover/selection/a
 * point-relevant tool.
 */
const ENDPOINT_IDLE_OPACITY = 0.3;
const MIDPOINT_IDLE_OPACITY = 0;
const CENTROID_IDLE_OPACITY = 0;

/** Flat local xyz (z=0) polyline for an entity, in plane coords. */
export function entityPolyline(e: {
  type: SketchEntity["type"];
  p0?: [number, number];
  p1?: [number, number];
  center?: [number, number];
  radius?: number;
  start?: [number, number];
  end?: [number, number];
  majorR?: number;
  minorR?: number;
  rotation?: number;
}): number[] {
  if (e.type === "Line" && e.p0 && e.p1) return [e.p0[0], e.p0[1], 0, e.p1[0], e.p1[1], 0];
  if (e.type === "Ellipse") {
    // Sampled parametric polyline, CLOSED (the first sample is repeated last) so the
    // Line2 strip meets itself — same shape as the Circle branch above.
    const params = ellipseParams(e);
    if (!params) return [];
    const ring = sampleEllipse(params);
    const out: number[] = [];
    for (let i = 0; i <= ring.length; i++) {
      const p = ring[i % ring.length];
      out.push(p[0], p[1], 0);
    }
    return out;
  }
  if (e.type === "Circle" && e.center && e.radius !== undefined) {
    const out: number[] = [];
    for (let i = 0; i <= ARC_SEGMENTS; i++) {
      const a = (i / ARC_SEGMENTS) * Math.PI * 2;
      out.push(e.center[0] + e.radius * Math.cos(a), e.center[1] + e.radius * Math.sin(a), 0);
    }
    return out;
  }
  if (e.type === "Arc" && e.center && e.radius !== undefined && e.start && e.end) {
    const a0 = Math.atan2(e.start[1] - e.center[1], e.start[0] - e.center[0]);
    let a1 = Math.atan2(e.end[1] - e.center[1], e.end[0] - e.center[0]);
    while (a1 <= a0) a1 += Math.PI * 2; // CCW sweep
    const out: number[] = [];
    for (let i = 0; i <= ARC_SEGMENTS; i++) {
      const a = a0 + ((a1 - a0) * i) / ARC_SEGMENTS;
      out.push(e.center[0] + e.radius * Math.cos(a), e.center[1] + e.radius * Math.sin(a), 0);
    }
    return out;
  }
  return [];
}

/** Endpoint/center markers of an entity, flat local xyz. */
function entityMarkers(e: SketchEntity): number[] {
  const out: number[] = [];
  const add = (p?: [number, number]) => p && out.push(p[0], p[1], 0);
  if (e.type === "Line") {
    add(e.p0);
    add(e.p1);
  } else if (e.type === "Circle" || e.type === "Ellipse") {
    add(e.center);
  } else if (e.type === "Arc") {
    add(e.center);
    add(e.start);
    add(e.end);
  } else if (e.type === "Point") {
    add(e.p0);
  }
  return out;
}

const mid2 = (a: [number, number], b: [number, number]): [number, number] => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];

/** Midpoint marker of an entity, flat local xyz — Line: segment midpoint; Arc:
 *  midpoint along the sweep (matches the CCW-sweep convention `entityPolyline`
 *  tessellates with). Circle/Ellipse/Point have no single natural midpoint. */
function entityMidpoints(e: SketchEntity): number[] {
  if (e.type === "Line" && e.p0 && e.p1) {
    const [x, y] = mid2(e.p0, e.p1);
    return [x, y, 0];
  }
  if (e.type === "Arc" && e.center && e.radius !== undefined && e.start && e.end) {
    const a0 = Math.atan2(e.start[1] - e.center[1], e.start[0] - e.center[0]);
    let a1 = Math.atan2(e.end[1] - e.center[1], e.end[0] - e.center[0]);
    while (a1 <= a0) a1 += Math.PI * 2;
    const am = (a0 + a1) / 2;
    return [e.center[0] + e.radius * Math.cos(am), e.center[1] + e.radius * Math.sin(am), 0];
  }
  return [];
}

interface SketchObjectDeps {
  sketchRoot: THREE.Object3D;
  invalidate: () => void;
}

export class SketchObject {
  private readonly planeGroup = new THREE.Group();
  private readonly entityGroup = new THREE.Group();
  private readonly previewGroup = new THREE.Group();
  // Destructive doomed-piece overlay (Trim tool hover), above committed entities.
  private readonly trimGhostGroup = new THREE.Group();
  // Dashed angle-preview arc (live angle chip ↔ its reference segment).
  private readonly angleArcGroup = new THREE.Group();
  // Permanent dimension-line witnesses (ticks + baseline + arrowheads) for
  // constrained edges — its own group so a rebuild never touches committed
  // entity lines.
  private readonly dimLineGroup = new THREE.Group();
  private readonly grid: GridPlane;
  private readonly tint: THREE.Mesh;
  private readonly points: THREE.Points;
  private readonly pointsMat: THREE.PointsMaterial;
  private readonly midpoints: THREE.Points;
  private readonly midpointsMat: THREE.PointsMaterial;
  private readonly centroids: THREE.Points;
  private readonly centroidsMat: THREE.PointsMaterial;
  // Highlight for a specifically SELECTED point (an endpoint/center picked by
  // the Select tool, not just its owning entity) — separate from `points` so
  // selecting one endpoint never recolors the shared vertex-ring pass.
  private readonly selectedPoints: THREE.Points;
  private readonly selectedPointsMat: THREE.PointsMaterial;

  private plane: SketchPlane | null = null;
  private entities: SketchEntity[] = [];
  private status: SketchSolveStatus = "UnderConstrained";
  private constraints: SketchConstraint[] = [];
  private selected = new Set<string>();
  private hovered = new Set<string>();
  /** The chain segment a live angle chip is measured against, or null. */
  private angleRefId: string | null = null;

  // Shared line materials, by state.
  private readonly matUnder: LineMaterial;
  private readonly matFull: LineMaterial;
  private readonly matConflict: LineMaterial;
  private readonly matSelected: LineMaterial;
  private readonly matHover: LineMaterial;
  private readonly matConstruction: LineMaterial;
  private readonly matReference: LineMaterial;
  private readonly matAngleRef: LineMaterial;
  private readonly matPreview: LineMaterial;
  private readonly matTrimGhost: LineMaterial;
  private readonly matAngleArc: LineMaterial;
  private readonly matDimLine: LineMaterial;
  private readonly allMaterials: LineMaterial[];

  private readonly _basis = new THREE.Matrix4();
  private readonly _target = new THREE.Vector3();

  constructor(private readonly deps: SketchObjectDeps) {
    this.planeGroup.name = "sketchPlane";
    this.planeGroup.add(
      this.entityGroup,
      this.previewGroup,
      this.trimGhostGroup,
      this.angleArcGroup,
      this.dimLineGroup,
    );
    deps.sketchRoot.add(this.planeGroup);

    // Plane tint quad (large, low alpha) + adaptive grid, both plane-local.
    this.tint = new THREE.Mesh(
      new THREE.PlaneGeometry(4000, 4000),
      new THREE.MeshBasicMaterial({
        color: palette.sketchPlane(),
        transparent: true,
        opacity: 0.5,
        depthWrite: false,
        side: THREE.DoubleSide,
        toneMapped: false,
      }),
    );
    this.tint.renderOrder = RENDER_ORDER.SKETCH_TINT;
    this.planeGroup.add(this.tint);

    this.grid = new GridPlane({ minor: palette.gridMinor(), major: palette.gridMajor(), clear: palette.sketchPlane() });
    this.grid.setVisible(true);
    this.planeGroup.add(this.grid.object3D);

    // All in-plane content is coplanar: it renders in the TRANSPARENT pass with
    // depthWrite off, layered by RENDER_ORDER only (see renderOrder.ts). Depth
    // TEST stays on so solid bodies still occlude sketch content behind them.
    this.pointsMat = new THREE.PointsMaterial({
      color: palette.sketchVertex(),
      map: buildDotTexture(0.7) ?? undefined,
      size: 10,
      sizeAttenuation: false,
      transparent: true,
      opacity: ENDPOINT_IDLE_OPACITY,
      alphaTest: 0.05,
      depthWrite: false,
      toneMapped: false,
    });
    this.points = new THREE.Points(new THREE.BufferGeometry(), this.pointsMat);
    this.points.renderOrder = RENDER_ORDER.SKETCH_POINTS;
    this.entityGroup.add(this.points);

    this.midpointsMat = new THREE.PointsMaterial({
      color: palette.sketchMidpoint(),
      map: buildDotTexture() ?? undefined,
      size: 6,
      sizeAttenuation: false,
      transparent: true,
      opacity: MIDPOINT_IDLE_OPACITY,
      alphaTest: 0.05,
      depthWrite: false,
      toneMapped: false,
    });
    this.midpoints = new THREE.Points(new THREE.BufferGeometry(), this.midpointsMat);
    this.midpoints.renderOrder = RENDER_ORDER.SKETCH_POINTS;
    this.entityGroup.add(this.midpoints);

    this.centroidsMat = new THREE.PointsMaterial({
      color: palette.sketchCentroid(),
      map: buildDotTexture() ?? undefined,
      size: 7,
      sizeAttenuation: false,
      transparent: true,
      opacity: CENTROID_IDLE_OPACITY,
      alphaTest: 0.05,
      depthWrite: false,
      toneMapped: false,
    });
    this.centroids = new THREE.Points(new THREE.BufferGeometry(), this.centroidsMat);
    this.centroids.renderOrder = RENDER_ORDER.SKETCH_POINTS;
    this.entityGroup.add(this.centroids);

    this.selectedPointsMat = new THREE.PointsMaterial({
      color: palette.sketchSelected(),
      map: buildRingTexture() ?? undefined,
      size: 16,
      sizeAttenuation: false,
      transparent: true,
      alphaTest: 0.05,
      depthWrite: false,
      toneMapped: false,
    });
    this.selectedPoints = new THREE.Points(new THREE.BufferGeometry(), this.selectedPointsMat);
    // Above every other marker tier — a selected point must never read as
    // "hidden behind" the vertex ring/midpoint dot it highlights.
    this.selectedPoints.renderOrder = RENDER_ORDER.SKETCH_POINTS + 1;
    this.entityGroup.add(this.selectedPoints);

    const mk = (color: THREE.Color, opts: Partial<ConstructorParameters<typeof LineMaterial>[0]> = {}) =>
      new LineMaterial({
        color: color.getHex(),
        linewidth: LINE_WIDTH,
        transparent: true,
        depthWrite: false,
        toneMapped: false,
        ...opts,
      });
    this.matUnder = mk(palette.sketchUnder());
    this.matFull = mk(palette.sketchFull());
    this.matConflict = mk(palette.sketchConflict());
    // Selection is a HALO material (P1 audit fix) — see `rebuildEntities`:
    // wider than a stroke, so it reads as an outline behind whatever semantic
    // color the entity's own line is drawn in. Hover is a plain LINE_WIDTH
    // color swap, so nothing gets fatter under the cursor.
    this.matSelected = mk(palette.sketchSelected(), { linewidth: SELECTION_HALO_WIDTH });
    this.matHover = mk(palette.hover3d());
    this.matConstruction = mk(palette.sketchConstruction(), { dashed: true, dashSize: 3, gapSize: 2 });
    // SOLID, deliberately: construction is dashed because it is not real
    // geometry, whereas reference geometry IS real (it bounds regions) — it just
    // is not YOURS to move. Colour carries the difference, not the stroke.
    this.matReference = mk(palette.sketchReference());
    this.matAngleRef = mk(palette.sketchAngleRef());
    this.matPreview = mk(palette.sketchUnder(), { linewidth: PREVIEW_WIDTH, transparent: true, opacity: 0.9 });
    this.matTrimGhost = mk(palette.destructive(), { linewidth: TRIM_GHOST_WIDTH, transparent: true, opacity: 0.95 });
    this.matAngleArc = mk(palette.sketchAngleRef(), {
      linewidth: ANGLE_ARC_WIDTH,
      dashed: true,
      dashSize: 3,
      gapSize: 2,
      transparent: true,
      opacity: 0.9,
    });
    // Neutral overlay tone (reused, not a new token) — a dimension line is
    // annotation, not geometry, same rationale as `matAngleArc`'s violet.
    this.matDimLine = mk(palette.referenceNeutral(), { linewidth: ANGLE_ARC_WIDTH, transparent: true, opacity: 0.85 });
    this.allMaterials = [
      this.matUnder,
      this.matFull,
      this.matConflict,
      this.matSelected,
      this.matHover,
      this.matConstruction,
      this.matReference,
      this.matAngleRef,
      this.matPreview,
      this.matTrimGhost,
      this.matAngleArc,
      this.matDimLine,
    ];
  }

  /**
   * Theme change: re-read every color this object baked at construction.
   *
   * The nine LineMaterials are built from `color.getHex()` — a VALUE snapshot,
   * not a live reference — and entity state changes only swap which material an
   * entity uses, never recolor one. So without this every sketch drawn before a
   * theme flip keeps the old theme's colors until the sketch is re-entered.
   */
  refreshColors(): void {
    (this.tint.material as THREE.MeshBasicMaterial).color.copy(palette.sketchPlane());
    this.grid.setColors({
      minor: palette.gridMinor(),
      major: palette.gridMajor(),
      clear: palette.sketchPlane(),
    });
    this.pointsMat.color.copy(palette.sketchVertex());
    this.midpointsMat.color.copy(palette.sketchMidpoint());
    this.centroidsMat.color.copy(palette.sketchCentroid());
    this.selectedPointsMat.color.copy(palette.sketchSelected());
    this.matUnder.color.copy(palette.sketchUnder());
    this.matFull.color.copy(palette.sketchFull());
    this.matConflict.color.copy(palette.sketchConflict());
    this.matSelected.color.copy(palette.sketchSelected());
    this.matHover.color.copy(palette.hover3d());
    this.matConstruction.color.copy(palette.sketchConstruction());
    this.matReference.color.copy(palette.sketchReference());
    this.matAngleRef.color.copy(palette.sketchAngleRef());
    this.matPreview.color.copy(palette.sketchUnder());
    this.matTrimGhost.color.copy(palette.destructive());
    this.matAngleArc.color.copy(palette.sketchAngleRef());
    this.matDimLine.color.copy(palette.referenceNeutral());
  }

  setVisible(visible: boolean): void {
    this.planeGroup.visible = visible;
    this.deps.invalidate();
  }

  /** Rebuild committed geometry from a session. `constraints` is optional —
   *  omitted (drag-preview refreshes, which never edit constraints) keeps
   *  whatever dimension lines are already drawn. */
  setSession(
    plane: SketchPlane,
    entities: SketchEntity[],
    status: SketchSolveStatus,
    constraints?: SketchConstraint[],
  ): void {
    this.plane = plane;
    this.entities = entities;
    this.status = status;
    if (constraints) this.constraints = constraints;
    planeBasisMatrix(plane, this._basis);
    this.planeGroup.matrixAutoUpdate = false;
    this.planeGroup.matrix.copy(this._basis);
    this.planeGroup.matrixWorldNeedsUpdate = true;
    this.rebuildEntities();
    this.rebuildPoints();
    this.rebuildMidpoints();
    this.rebuildCentroids();
    this.rebuildDimensionLines();
    this.deps.invalidate();
  }

  /** Replace the rubber-band preview (drafts in the same plane). */
  setPreview(drafts: DraftEntity[]): void {
    for (const c of [...this.previewGroup.children]) this.disposeLine(c);
    this.previewGroup.clear();
    for (const d of drafts) {
      const positions = entityPolyline({
        type: d.type,
        p0: d.p0 ? [d.p0.x, d.p0.y] : undefined,
        p1: d.p1 ? [d.p1.x, d.p1.y] : undefined,
        center: d.center ? [d.center.x, d.center.y] : undefined,
        radius: d.radius,
        start: d.start ? [d.start.x, d.start.y] : undefined,
        end: d.end ? [d.end.x, d.end.y] : undefined,
        majorR: d.majorR,
        minorR: d.minorR,
        rotation: d.rotation,
      });
      if (positions.length < 6) continue;
      const line = this.buildLine(positions, d.construction ? this.matConstruction : this.matPreview);
      this.previewGroup.add(line);
    }
    this.deps.invalidate();
  }

  /** Replace the destructive trim-ghost overlay (the doomed piece), or clear it. */
  setTrimGhost(draft: DraftEntity | null): void {
    for (const c of [...this.trimGhostGroup.children]) this.disposeLine(c);
    this.trimGhostGroup.clear();
    if (draft) {
      const positions = entityPolyline({
        type: draft.type,
        p0: draft.p0 ? [draft.p0.x, draft.p0.y] : undefined,
        p1: draft.p1 ? [draft.p1.x, draft.p1.y] : undefined,
        center: draft.center ? [draft.center.x, draft.center.y] : undefined,
        radius: draft.radius,
        start: draft.start ? [draft.start.x, draft.start.y] : undefined,
        end: draft.end ? [draft.end.x, draft.end.y] : undefined,
        majorR: draft.majorR,
        minorR: draft.minorR,
        rotation: draft.rotation,
      });
      if (positions.length >= 6) {
        const line = this.buildLine(positions, this.matTrimGhost);
        line.renderOrder = RENDER_ORDER.TRIM_GHOST; // above committed entities + markers
        this.trimGhostGroup.add(line);
      }
    }
    this.deps.invalidate();
  }

  /**
   * Replace the dashed angle-preview arc (live angle chip ↔ its reference
   * segment), or clear it. `fromDeg`/`toDeg` are ABSOLUTE headings — see
   * `angleArcPreview.ts` for why a direct sweep between them is already the
   * shorter arc, with no re-folding needed here.
   */
  setAnglePreview(arc: { center: Point2; radius: number; fromDeg: number; toDeg: number } | null): void {
    for (const c of [...this.angleArcGroup.children]) this.disposeLine(c);
    this.angleArcGroup.clear();
    if (arc && arc.radius > 1e-9) {
      const positions: number[] = [];
      for (const p of angleArcPoints(arc.center, arc.radius, arc.fromDeg, arc.toDeg)) {
        positions.push(p.x, p.y, 0);
      }
      if (positions.length >= 6) {
        const line = this.buildLine(positions, this.matAngleArc);
        line.renderOrder = RENDER_ORDER.ANGLE_ARC_PREVIEW;
        this.angleArcGroup.add(line);
      }
    }
    this.deps.invalidate();
  }

  /** Recolor from the current selection (sketch entity ids). A newly selected,
   *  unconstrained edge's live-length label is `SelectionDimensionLabels`'
   *  job (DOM overlay, not 3D geometry) — selection no longer touches the
   *  dimension-line witnesses this class draws, which are authored-only. */
  setSelection(selectedIds: Iterable<string>): void {
    this.selected = new Set(selectedIds);
    this.rebuildEntities();
    this.deps.invalidate();
  }

  /** Recolor from the current hover (sketch entity ids). Selection wins over hover. */
  setHover(hoveredIds: Iterable<string>): void {
    this.hovered = new Set(hoveredIds);
    this.rebuildEntities();
    this.deps.invalidate();
  }

  /** Highlight glyph for specifically-selected POINTS (an endpoint/center the
   *  Select tool picked, not just its owning entity), in plane (u,v) coords. */
  setSelectedPoints(points: Point2[]): void {
    const flat: number[] = [];
    for (const p of points) flat.push(p.x, p.y, 0);
    const geo = this.selectedPoints.geometry;
    geo.setAttribute("position", new THREE.Float32BufferAttribute(flat, 3));
    geo.computeBoundingSphere();
    this.deps.invalidate();
  }

  /** Tint the chain segment a live angle chip is measured against, or clear
   *  it. Selection/hover still win — the entity stays selectable/hoverable
   *  like anything else while it happens to be the angle reference. */
  setAngleReference(id: string | null): void {
    if (this.angleRefId === id) return;
    this.angleRefId = id;
    this.rebuildEntities();
    this.deps.invalidate();
  }

  /**
   * Endpoint/midpoint/centroid AFFORDANCE (Sketcher UX cleanup, Track B1b/P1
   * hardening): each tier's own flag restores ITS material to full opacity,
   * otherwise it sits at its own idle constant (endpoints faint, midpoints/
   * centroids hidden). Still WHOLE-SKETCH per tier, not per-entity — a true
   * per-entity reveal needs per-vertex alpha (a shader material keyed off a
   * vertex attribute), not a per-tier material opacity; that's future work.
   * The caller (`ViewportRoot`) narrows WHICH tier lights up from what's
   * actually hovered/selected/tool-relevant, so hovering one line's BODY no
   * longer also lights up every midpoint/centroid in the sketch — only its
   * own endpoints do, matching what that hover can actually snap to. A
   * material-opacity write + one `invalidate()` per changed tier, no
   * `setAttribute`/`computeBoundingSphere` — cheap enough to call at
   * hover-change frequency (the caller already dedupes hover to actual
   * transitions; this does not add its own throttling).
   */
  setPointAffordance(state: { endpoints: boolean; midpoints: boolean; centroids: boolean }): void {
    this.pointsMat.opacity = state.endpoints ? 1 : ENDPOINT_IDLE_OPACITY;
    this.midpointsMat.opacity = state.midpoints ? 1 : MIDPOINT_IDLE_OPACITY;
    this.centroidsMat.opacity = state.centroids ? 1 : CENTROID_IDLE_OPACITY;
    this.deps.invalidate();
  }

  private statusMaterial(): LineMaterial {
    switch (this.status) {
      case "FullyConstrained":
        return this.matFull;
      case "OverConstrained":
      case "Conflicting":
        return this.matConflict;
      default:
        return this.matUnder;
    }
  }

  private rebuildEntities(): void {
    // Remove all lines (keep the Points objects — vertices/midpoints/centroids/selection).
    for (const c of [...this.entityGroup.children]) {
      if (c === this.points || c === this.midpoints || c === this.centroids || c === this.selectedPoints) continue;
      this.disposeLine(c);
      this.entityGroup.remove(c);
    }
    const statusMat = this.statusMaterial();
    for (const e of this.entities) {
      const positions = entityPolyline(e);
      if (positions.length < 6) continue;
      // SELECTION does not REPLACE the entity's own color (P1 audit fix): a
      // wider halo renders BEHIND it instead, so an under/full/conflict (or
      // reference/construction/angle-ref) color stays visible while selected
      // rather than being wiped by one flat selection tint.
      //
      // HOVER is a pure COLOR swap at the normal stroke weight — no halo. A
      // hover halo made every line under the cursor read as suddenly heavier,
      // which is a width change the user never asked for; the transient
      // feedback carries fine in color alone.
      const isSelected = this.selected.has(e.id);
      const isHovered = !isSelected && this.hovered.has(e.id);
      if (isSelected) {
        const halo = this.buildLine(positions, this.matSelected);
        halo.renderOrder = RENDER_ORDER.SKETCH_CURVES_HALO;
        halo.userData.selectionHalo = true;
        this.entityGroup.add(halo);
      }
      const mat = isHovered
        ? this.matHover
        : this.angleRefId === e.id
          ? this.matAngleRef
          : e.referenceLocked
            ? this.matReference
            : e.construction
              ? this.matConstruction
              : statusMat;
      this.entityGroup.add(this.buildLine(positions, mat));
    }
  }

  private rebuildPoints(): void {
    const flat: number[] = [];
    for (const e of this.entities) flat.push(...entityMarkers(e));
    const geo = this.points.geometry;
    geo.setAttribute("position", new THREE.Float32BufferAttribute(flat, 3));
    geo.computeBoundingSphere();
  }

  private rebuildMidpoints(): void {
    const flat: number[] = [];
    for (const e of this.entities) flat.push(...entityMidpoints(e));
    const geo = this.midpoints.geometry;
    geo.setAttribute("position", new THREE.Float32BufferAttribute(flat, 3));
    geo.computeBoundingSphere();
  }

  private rebuildCentroids(): void {
    const byId = new Map(this.entities.map((e) => [e.id, e]));
    const flat: number[] = [];
    for (const loop of findClosedLoops(this.entities)) {
      const c = loopCentroid(loop, byId);
      if (c) flat.push(c[0], c[1], 0);
    }
    const geo = this.centroids.geometry;
    geo.setAttribute("position", new THREE.Float32BufferAttribute(flat, 3));
    geo.computeBoundingSphere();
  }

  /** Rebuild the dimension-line witnesses — authored constraints only (Track
   *  B3); a merely-selected, unconstrained edge gets a lightweight DOM label
   *  instead (`SelectionDimensionLabels`), not this 3D geometry. */
  private rebuildDimensionLines(): void {
    for (const c of [...this.dimLineGroup.children]) this.disposeLine(c);
    this.dimLineGroup.clear();
    for (const dim of layoutDimensionLines(this.entities, this.constraints)) {
      const segments: [Point2, Point2][] = [dim.baseline, ...dim.ticks, ...dim.arrows];
      for (const [a, b] of segments) {
        const line = this.buildLine([a.x, a.y, 0, b.x, b.y, 0], this.matDimLine);
        line.renderOrder = RENDER_ORDER.DIM_LINE;
        // Tags a witness segment as NOT a committed entity — a scene-wide
        // `Line2` traversal (e.g. a test counting entity lines) needs a way
        // to tell the two apart without knowing this group exists.
        line.userData.dimLineWitness = true;
        this.dimLineGroup.add(line);
      }
    }
  }

  private buildLine(positions: number[], mat: LineMaterial): Line2 {
    const geo = new LineGeometry();
    geo.setPositions(positions);
    const line = new Line2(geo, mat);
    if (mat.dashed) line.computeLineDistances();
    line.renderOrder = RENDER_ORDER.SKETCH_CURVES;
    return line;
  }

  private disposeLine(obj: THREE.Object3D): void {
    const l = obj as Line2;
    l.geometry?.dispose();
    // Materials are shared — never disposed here.
  }

  /** Per-frame: Line2 resolution + adaptive grid re-center (plane-local). */
  update(width: number, height: number, cameraTarget: THREE.Vector3, cameraDistance: number): void {
    for (const m of this.allMaterials) m.resolution.set(width, height);
    if (this.plane) {
      const local = worldToPlanePoint(this.plane, this._target.copy(cameraTarget));
      this.grid.update(new THREE.Vector3(local.x, local.y, 0), cameraDistance);
    }
  }

  dispose(): void {
    for (const c of [...this.entityGroup.children]) {
      if (c === this.points || c === this.midpoints || c === this.centroids || c === this.selectedPoints) continue;
      this.disposeLine(c);
    }
    for (const c of [...this.previewGroup.children]) this.disposeLine(c);
    for (const c of [...this.trimGhostGroup.children]) this.disposeLine(c);
    for (const c of [...this.angleArcGroup.children]) this.disposeLine(c);
    for (const c of [...this.dimLineGroup.children]) this.disposeLine(c);
    this.points.geometry.dispose();
    this.pointsMat.map?.dispose();
    this.pointsMat.dispose();
    this.midpoints.geometry.dispose();
    this.midpointsMat.map?.dispose();
    this.midpointsMat.dispose();
    this.centroids.geometry.dispose();
    this.centroidsMat.map?.dispose();
    this.centroidsMat.dispose();
    this.selectedPoints.geometry.dispose();
    this.selectedPointsMat.map?.dispose();
    this.selectedPointsMat.dispose();
    (this.tint.geometry as THREE.BufferGeometry).dispose();
    (this.tint.material as THREE.Material).dispose();
    this.grid.dispose();
    for (const m of this.allMaterials) m.dispose();
    this.deps.sketchRoot.remove(this.planeGroup);
  }
}
