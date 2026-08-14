/*
 * TransformGizmo — the placement tool's drag surface (WP-B W2): three axis
 * arrows (translate along one axis), three plane quads (translate in two — the
 * fit-check gesture people actually reach for), and three rings (rotate about an
 * axis through the pivot). A sibling of `DragHandle`, deliberately NOT a
 * generalisation of it: the extrude handle is a single one-axis depth grab on a
 * hot path with its own e2e coverage, and folding the two would put the extrude
 * lane at risk for no shared behaviour beyond "screen-scaled overlay mesh".
 *
 * WORLD-AXIS ONLY, NEVER ROTATED. The group's origin follows the placement
 * (`setPose`) so the gizmo rides the moved body, but its arms stay parallel to
 * the world axes because the FSM it drives stores a WORLD translation and a
 * WORLD rotation axis. Spinning the gizmo with the body would make the arrows
 * point at directions the record cannot express.
 *
 * Everything is authored in "px units" and the whole group is scaled by the
 * world size of one screen pixel each frame (`setScale`), so the gizmo holds a
 * constant on-screen size — which also keeps the GRAB TARGETS a fixed size, the
 * same reason `PlanePicker` scales its quads. The three pick tiers occupy
 * disjoint RADIUS bands so they cannot be confused from a normal viewing angle:
 * arrow tips end at 67px, quads sit 16–31px out along their in-plane axes (≥16px
 * clear of any arrow's 6px pick cylinder), and the rotation arcs occupy a compact
 * 64–76px band on the 45° bisector, where no arrow and no quad reaches.
 *
 * Arbitration between handles is plain NEAREST-HIT — the three.js house rule, and
 * the honest one for an overlay where nothing is depth-tested: whatever the ray
 * reaches first is what the user sees under the cursor. The edge-on case that
 * used to make this surprising is gone with the full rings (U5): a ±26° arc
 * cannot collapse into a line that sweeps the whole widget, so there is no longer
 * a handle that wins from across the gizmo.
 */
import * as THREE from "three";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";

export type GizmoAxis = "X" | "Y" | "Z";
export type GizmoPartKind = "axis" | "plane" | "ring";

/** Which handle a raycast landed on. Structurally the controller's `PatternAxis`. */
export interface GizmoHit {
  kind: GizmoPartKind;
  axis: GizmoAxis;
}

const AXES: GizmoAxis[] = ["X", "Y", "Z"];
const AXIS_DIR: Record<GizmoAxis, THREE.Vector3> = {
  X: new THREE.Vector3(1, 0, 0),
  Y: new THREE.Vector3(0, 1, 0),
  Z: new THREE.Vector3(0, 0, 1),
};

const ARM_PX = 54; // arrow shaft length
const SHAFT_R_PX = 1.4;
const CONE_PX = 13;
const CONE_R_PX = 4.5;
const ARM_HIT_R_PX = 6; // fat, invisible grab cylinder around each arrow
const QUAD_NEAR_PX = 16; // inner corner of a plane quad, along both in-plane axes
const QUAD_PX = 15;
/*
 * Rotation handles (U5). These used to be three FULL tori at r=82px, which is a
 * 164px-wide circle per axis: they crossed each other, they crossed both the
 * arrows and the pivot, and viewed edge-on a ring collapses to a line sweeping
 * the whole widget and wins the nearest-hit over any arm it crosses.
 *
 * They are now compact double-headed ARCS. Three properties do the work:
 *   - radius 70px sits just OUTSIDE the arrow tips (67px) and well outside the
 *     plane quads (whose far corner is r≈44), so the three tiers stay in
 *     disjoint bands and the pivot stays clear;
 *   - each arc is centred on the +45° bisector of its own plane, which is where
 *     no arrow lives (they are on the axes) and which faces the default camera;
 *   - a ±26° sweep is short enough that no arc can degenerate into a line across
 *     the widget, which is what removes the edge-on ambiguity entirely rather
 *     than arbitrating it.
 */
const RING_R_PX = 70;
const RING_SWEEP_RAD = (26 * Math.PI) / 180;
const RING_TUBE_PX = 1.1;
/** ≥8 CSS px of pick corridor (12px diameter), well clear of the 2.2px visual. */
const RING_HIT_TUBE_PX = 6;
/** The double-headed arrowheads that say "this drags around". */
const RING_HEAD_PX = 6;
const RING_HEAD_R_PX = 2.6;
const ARC_SEGMENTS = 24;

const ARM_OPACITY = 0.9;
const QUAD_OPACITY = 0.4;
const RING_OPACITY = 0.75;
const HOT_OPACITY = 1;

export interface TransformGizmoDeps {
  root: THREE.Object3D; // interactionRoot
  invalidate: () => void;
}

/** One grabbable handle: its visible material, its (invisible) pick mesh. */
interface Part {
  kind: GizmoPartKind;
  axis: GizmoAxis;
  mat: THREE.MeshBasicMaterial;
  hit: THREE.Mesh;
  baseOpacity: number;
}

const axisColor = (axis: GizmoAxis): THREE.Color =>
  axis === "X" ? palette.axisX() : axis === "Y" ? palette.axisY() : palette.axisZ();

const same = (a: GizmoHit | null, b: Part): boolean => a?.kind === b.kind && a.axis === b.axis;

export class TransformGizmo {
  private readonly group = new THREE.Group();
  private readonly parts: Part[] = [];
  private readonly hitMat: THREE.MeshBasicMaterial;
  private readonly geometries: THREE.BufferGeometry[] = [];
  private hovered: GizmoHit | null = null;
  private active: GizmoHit | null = null;

  constructor(private readonly deps: TransformGizmoDeps) {
    this.group.name = "transformGizmo";
    this.group.visible = false;
    this.group.renderOrder = RENDER_ORDER.TRANSFORM_GIZMO;
    this.hitMat = new THREE.MeshBasicMaterial({ visible: false, toneMapped: false });
    for (const axis of AXES) this.buildArrow(axis);
    for (const axis of AXES) this.buildQuad(axis);
    for (const axis of AXES) this.buildRing(axis);
    // Matrices flushed once here so a raycast BEFORE the first frame resolves
    // (the same reason PlanePicker flushes in its constructor).
    this.group.updateMatrixWorld(true);
    deps.root.add(this.group);
  }

  /** A visible overlay material for one handle (never depth-tested, never lit). */
  private newMat(axis: GizmoAxis, opacity: number): THREE.MeshBasicMaterial {
    return new THREE.MeshBasicMaterial({
      color: axisColor(axis),
      depthTest: false,
      transparent: true,
      opacity,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
  }

  /** Register a handle's visible meshes plus the fat pick mesh that grabs it. */
  private addPart(
    kind: GizmoPartKind,
    axis: GizmoAxis,
    mat: THREE.MeshBasicMaterial,
    visuals: THREE.Mesh[],
    hit: THREE.Mesh,
    baseOpacity: number,
  ): void {
    for (const m of visuals) {
      m.renderOrder = RENDER_ORDER.TRANSFORM_GIZMO;
      this.geometries.push(m.geometry);
      this.group.add(m);
    }
    hit.userData.gizmoKind = kind;
    hit.userData.gizmoAxis = axis;
    this.geometries.push(hit.geometry);
    this.group.add(hit);
    this.parts.push({ kind, axis, mat, hit, baseOpacity });
  }

  private buildArrow(axis: GizmoAxis): void {
    const mat = this.newMat(axis, ARM_OPACITY);
    const q = new THREE.Quaternion().setFromUnitVectors(AXIS_DIR.Y, AXIS_DIR[axis]);
    const shaft = new THREE.Mesh(new THREE.CylinderGeometry(SHAFT_R_PX, SHAFT_R_PX, ARM_PX, 10), mat);
    shaft.position.copy(AXIS_DIR[axis]).multiplyScalar(ARM_PX / 2);
    shaft.quaternion.copy(q);
    const cone = new THREE.Mesh(new THREE.ConeGeometry(CONE_R_PX, CONE_PX, 14), mat);
    cone.position.copy(AXIS_DIR[axis]).multiplyScalar(ARM_PX + CONE_PX / 2);
    cone.quaternion.copy(q);
    const hit = new THREE.Mesh(
      new THREE.CylinderGeometry(ARM_HIT_R_PX, ARM_HIT_R_PX, ARM_PX + CONE_PX, 8),
      this.hitMat,
    );
    hit.position.copy(AXIS_DIR[axis]).multiplyScalar((ARM_PX + CONE_PX) / 2);
    hit.quaternion.copy(q);
    this.addPart("axis", axis, mat, [shaft, cone], hit, ARM_OPACITY);
  }

  /**
   * The quad for `axis` is the one whose NORMAL is that axis — the Z quad
   * translates in XY. Naming it by the normal is what keeps every handle keyed
   * by a single axis letter, so one `GizmoHit` shape covers all three kinds.
   */
  private buildQuad(axis: GizmoAxis): void {
    const mat = this.newMat(axis, QUAD_OPACITY);
    const [a, b] = AXES.filter((x) => x !== axis);
    const centre = new THREE.Vector3()
      .addScaledVector(AXIS_DIR[a], QUAD_NEAR_PX + QUAD_PX / 2)
      .addScaledVector(AXIS_DIR[b], QUAD_NEAR_PX + QUAD_PX / 2);
    // PlaneGeometry is authored in local XY (normal +Z) — rotate +Z onto `axis`.
    const q = new THREE.Quaternion().setFromUnitVectors(AXIS_DIR.Z, AXIS_DIR[axis]);
    const quad = new THREE.Mesh(new THREE.PlaneGeometry(QUAD_PX, QUAD_PX), mat);
    quad.position.copy(centre);
    quad.quaternion.copy(q);
    const hit = new THREE.Mesh(new THREE.PlaneGeometry(QUAD_PX, QUAD_PX), this.hitMat);
    hit.position.copy(centre);
    hit.quaternion.copy(q);
    this.addPart("plane", axis, mat, [quad], hit, QUAD_OPACITY);
  }

  /**
   * The arc for `axis` spins ABOUT that axis, so it lives in the plane of the
   * other two — centred on their negative bisector (see the RING_* constants).
   *
   * Classification stays `{kind:"ring", axis}`: the gesture and everything
   * downstream of it are unchanged, only the geometry the user grabs is.
   */
  private buildRing(axis: GizmoAxis): void {
    const mat = this.newMat(axis, RING_OPACITY);
    const [a, b] = AXES.filter((x) => x !== axis);
    // Centre of the sweep: the +45° bisector of the two in-plane axes — the
    // direction that holds no arrow (those are ON the axes) and no quad (that
    // ends at r≈44 in this direction), and that faces the default camera.
    const centreAngle = Math.PI * 0.25;
    const pointAt = (t: number): THREE.Vector3 => {
      const angle = centreAngle + t;
      return new THREE.Vector3()
        .addScaledVector(AXIS_DIR[a], Math.cos(angle) * RING_R_PX)
        .addScaledVector(AXIS_DIR[b], Math.sin(angle) * RING_R_PX);
    };
    const path = new THREE.CatmullRomCurve3(
      Array.from({ length: ARC_SEGMENTS + 1 }, (_, i) =>
        pointAt(-RING_SWEEP_RAD + (2 * RING_SWEEP_RAD * i) / ARC_SEGMENTS),
      ),
    );
    const arc = new THREE.Mesh(
      new THREE.TubeGeometry(path, ARC_SEGMENTS, RING_TUBE_PX, 6, false),
      mat,
    );
    // A head at each end: the handle drags BOTH ways, and a bare arc reads as
    // decoration rather than something to grab.
    const visuals: THREE.Mesh[] = [arc];
    for (const end of [-1, 1] as const) {
      const tip = pointAt(end * RING_SWEEP_RAD);
      const inward = pointAt(end * (RING_SWEEP_RAD - 0.08));
      const head = new THREE.Mesh(
        new THREE.ConeGeometry(RING_HEAD_R_PX, RING_HEAD_PX, 10),
        mat,
      );
      head.position.copy(tip);
      head.quaternion.setFromUnitVectors(
        AXIS_DIR.Y,
        tip.clone().sub(inward).normalize(),
      );
      visuals.push(head);
    }
    // Separate, fatter pick corridor over the SAME curve — a trackpad user must
    // not have to land on a 2px stroke.
    const hit = new THREE.Mesh(
      new THREE.TubeGeometry(path, ARC_SEGMENTS, RING_HIT_TUBE_PX, 6, false),
      this.hitMat,
    );
    this.addPart("ring", axis, mat, visuals, hit, RING_OPACITY);
  }

  /** Move the gizmo to `origin`. Orientation is fixed to the WORLD axes. */
  setPose(origin: THREE.Vector3): void {
    this.group.position.copy(origin);
    this.group.updateMatrixWorld(true);
    this.deps.invalidate();
  }

  /** Where the widget sits, for a depth-accurate screen scale. */
  worldAnchor(): THREE.Vector3 {
    return this.group.position.clone();
  }

  /** Constant screen size: `worldPerPx` world units per CSS pixel. */
  setScale(worldPerPx: number): void {
    this.group.scale.setScalar(Math.max(worldPerPx, 1e-6));
    this.group.updateMatrixWorld(true);
    this.deps.invalidate();
  }

  setVisible(visible: boolean): void {
    this.group.visible = visible;
    if (!visible) {
      this.hovered = null;
      this.active = null;
      this.applyColors();
    }
    this.deps.invalidate();
  }

  get visible(): boolean {
    return this.group.visible;
  }

  /** Handle under the pointer (hover tint). */
  setHover(hit: GizmoHit | null): void {
    if (hit?.kind === this.hovered?.kind && hit?.axis === this.hovered?.axis) return;
    this.hovered = hit;
    this.applyColors();
  }

  /** Handle currently being DRAGGED — outranks hover, survives leaving the mesh. */
  setActive(hit: GizmoHit | null): void {
    this.active = hit;
    this.applyColors();
  }

  /**
   * Theme change: re-read every handle's axis color. `applyColors` is the single
   * writer of material color + opacity, so the highlight state is reproduced
   * here rather than being clobbered by the refresh.
   */
  refreshColors(): void {
    this.applyColors();
  }

  private applyColors(): void {
    const hot = palette.selectedEdge();
    for (const p of this.parts) {
      const lit = same(this.active, p) || (this.active === null && same(this.hovered, p));
      p.mat.color.copy(lit ? hot : axisColor(p.axis));
      p.mat.opacity = lit ? HOT_OPACITY : p.baseOpacity;
    }
    this.deps.invalidate();
  }

  /**
   * The gizmo's extent in WORLD units, as a radius about its pivot (U5).
   *
   * The widget is authored in px units and uniformly scaled, so its reach is a
   * single number: the farthest handle (an arrow tip at `ARM_PX + CONE_PX`, or a
   * rotation arc plus its pick corridor) times the current scale. Chip placement
   * turns that into a screen-space exclusion — see
   * `ViewportEngine.getInteractionOverlayBounds`.
   */
  worldRadius(): number {
    const reachPx = Math.max(ARM_PX + CONE_PX, RING_R_PX + RING_HIT_TUBE_PX + RING_HEAD_PX);
    return reachPx * this.group.scale.x;
  }

  /** Nearest handle under `raycaster`, or null (arbitration — see the header). */
  raycast(raycaster: THREE.Raycaster): GizmoHit | null {
    if (!this.group.visible) return null;
    this.group.updateMatrixWorld(true);
    const hits = raycaster.intersectObjects(
      this.parts.map((p) => p.hit),
      false,
    );
    if (hits.length === 0) return null;
    const { gizmoKind, gizmoAxis } = hits[0].object.userData;
    return { kind: gizmoKind as GizmoPartKind, axis: gizmoAxis as GizmoAxis };
  }

  dispose(): void {
    for (const g of this.geometries) g.dispose();
    for (const p of this.parts) p.mat.dispose();
    this.hitMat.dispose();
    this.deps.root.remove(this.group);
  }
}
