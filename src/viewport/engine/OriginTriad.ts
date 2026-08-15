/*
 * Always-visible origin axis triad.
 *
 * Three colored legs from the world origin along +X/+Y/+Z, held at a CONSTANT
 * on-screen length (TRIAD_PX) at every zoom level: the geometry is built once
 * at unit length and the group is scaled by the world-size of one pixel at the
 * origin. An earlier version sized the legs to the grid's major step, which
 * snaps on a 1/5/10 decade progression — so the triad visibly jumped between
 * sizes and shrank as you zoomed in. Unlike GridPlane, the triad is NOT gated
 * by gridVisible — the viewport should never lose its sense of orientation,
 * grid on or off.
 *
 * Drawn with LineSegments2 (addons `lines/`, same as SketchObject) for two
 * reasons: WebGL ignores LineBasicMaterial.linewidth, so plain lines are stuck
 * at 1px; and the +X/+Y legs are exactly coplanar with the grid's x=0/y=0
 * major lines, which z-fights into a dashed/half-missing axis. Fat lines are
 * expanded into triangle quads, so polygonOffset (POLYGON_OFFSET_FILL, the
 * only offset WebGL exposes) applies and pulls the triad in front of the grid
 * while it still depth-tests normally against solid bodies.
 */
import * as THREE from "three";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import type { HtmlOverlayDriver } from "./HtmlOverlayDriver";
import { worldPerPixel } from "./screenScale";
import { RENDER_ORDER } from "./renderOrder";

interface TriadColors {
  x: THREE.Color;
  y: THREE.Color;
  z: THREE.Color;
}

interface OriginTriadDeps {
  overlay: HtmlOverlayDriver;
  overlayEl: HTMLElement;
}

/** Leg width in CSS px. Same weight as sketch lines. */
const TRIAD_WIDTH = 2;

/** Leg length in CSS px, held constant across zoom. */
const TRIAD_PX = 110;

/**
 * Recessive weight, relative to real geometry — without this the triad reads
 * as ambiguous "two colored lines" rather than a coordinate frame (Sketcher
 * UX cleanup, Track B5).
 */
const TRIAD_OPACITY = 0.65;

/** How far past the leg's own tip an axis label sits, as a fraction of leg
 *  length — clear of the line's end cap without floating detached from it. */
const LABEL_REACH = 1.12;

/**
 * Below this on-screen distance (CSS px) between the origin and an axis's
 * label, that label hides (P2 hardening). A plane-normal orthographic view —
 * an XY sketch dead-on, say — projects the near-parallel-to-camera axis's
 * whole leg to almost no screen length, so its label lands back on top of
 * the origin: the busiest snap target in the scene, and the one place a
 * "Z" glyph helps least (there is no orientation ambiguity left to resolve
 * for an axis you're looking straight down).
 */
const NEAR_ORIGIN_HIDE_PX = 8;

const AXES = ["x", "y", "z"] as const;
type Axis = (typeof AXES)[number];

/** World origin — this group never translates, so it's always here. */
const ORIGIN = new THREE.Vector3(0, 0, 0);

/** Flat rgb pairs (both ends of each leg) in +X/+Y/+Z order. */
function legColors(colors: TriadColors): number[] {
  const col: number[] = [];
  for (const c of [colors.x, colors.y, colors.z]) {
    col.push(c.r, c.g, c.b, c.r, c.g, c.b);
  }
  return col;
}

/** Small "X"/"Y"/"Z" DOM label, colored from the SAME live CSS var the leg's
 *  baked-in-geometry color was read from at construction — a DOM element can
 *  read a custom property live, so this needs no `refreshColors()`
 *  bookkeeping on theme flip, unlike the geometry-side per-vertex colors. */
function makeLabel(axis: Axis): HTMLElement {
  const el = document.createElement("div");
  el.textContent = axis.toUpperCase();
  el.style.position = "absolute";
  el.style.left = "0";
  el.style.top = "0";
  el.style.font = "600 10px var(--font-ui)";
  el.style.color = `var(--color-axis-${axis})`;
  el.style.pointerEvents = "none";
  el.style.willChange = "transform";
  return el;
}

export class OriginTriad {
  readonly object3D: THREE.Group;
  private readonly material: LineMaterial;
  private readonly seg: LineSegments2;
  private readonly labels: Record<Axis, HTMLElement>;
  private readonly labelIds: Record<Axis, string>;

  constructor(
    colors: TriadColors,
    private readonly deps: OriginTriadDeps,
  ) {
    this.object3D = new THREE.Group();
    this.object3D.name = "originTriad";
    // Draw after the grid (renderOrder -1) but still behind solid geometry.
    this.object3D.renderOrder = RENDER_ORDER.TRIAD;

    this.material = new LineMaterial({
      linewidth: TRIAD_WIDTH,
      vertexColors: true,
      // Pull the coplanar +X/+Y legs off the grid plane in depth-buffer space.
      polygonOffset: true,
      polygonOffsetFactor: -4,
      polygonOffsetUnits: -4,
      transparent: true,
      opacity: TRIAD_OPACITY,
      // Coplanar transparent content convention (SketchObject precedent): the
      // polygonOffset above already keeps this off the grid in depth-buffer
      // space, so this content doesn't need to WRITE depth for anything to
      // layer correctly against it — only against solid bodies, which
      // `depthTest` (unset here, defaults true) still handles.
      depthWrite: false,
      toneMapped: false,
    });

    // One LineSegments2, three unit-length legs, per-vertex colors. Built once:
    // zoom only changes object3D.scale, so there is no per-frame reallocation.
    const geo = new LineSegmentsGeometry();
    geo.setPositions([
      0, 0, 0, 1, 0, 0, // +X
      0, 0, 0, 0, 1, 0, // +Y
      0, 0, 0, 0, 0, 1, // +Z
    ]);
    geo.setColors(legColors(colors));

    this.seg = new LineSegments2(geo, this.material);
    this.seg.renderOrder = RENDER_ORDER.TRIAD;
    // Tiny, always at the origin, and the bounding sphere ignores the group
    // scale — culling it is never a win and can wrongly drop it.
    this.seg.frustumCulled = false;
    this.object3D.add(this.seg);

    this.labels = { x: makeLabel("x"), y: makeLabel("y"), z: makeLabel("z") };
    this.labelIds = { x: "__originTriad_x", y: "__originTriad_y", z: "__originTriad_z" };
    for (const axis of AXES) {
      this.deps.overlayEl.appendChild(this.labels[axis]);
      // Registered once, at construction, for the object's whole lifetime —
      // unlike a snap hint these are permanent, so there is no lazy-register
      // branch to maintain. `update()` corrects the position every frame;
      // this initial (0,0,0) placement is never actually rendered from.
      this.deps.overlay.register(this.labelIds[axis], this.labels[axis], new THREE.Vector3(0, 0, 0));
    }
  }

  /**
   * Per-frame: rescale so each leg spans TRIAD_PX on screen, and refresh the
   * fat-line resolution. `width`/`height` are CSS px; `dpr` converts them to
   * the device pixels LineMaterial expects.
   */
  update(camera: THREE.Camera, width: number, height: number, dpr: number): void {
    const h = Math.max(height, 1);
    // The group never moves, so its local position IS its world position.
    this.object3D.scale.setScalar(worldPerPixel(camera, this.object3D.position, h) * TRIAD_PX);
    this.material.resolution.set(Math.max(width, 1) * dpr, h * dpr);

    // Label world positions ride the SAME constant-on-screen-size scale the
    // legs just got — a fixed reach just past each tip, no separate sizing.
    const reach = this.legLength * LABEL_REACH;
    const tips: Record<Axis, THREE.Vector3> = {
      x: new THREE.Vector3(reach, 0, 0),
      y: new THREE.Vector3(0, reach, 0),
      z: new THREE.Vector3(0, 0, reach),
    };
    // Origin is always world (0,0,0) — this group never translates.
    const originPx = this.projectToScreenPx(ORIGIN, camera, width, height);
    for (const axis of AXES) {
      this.deps.overlay.setWorldPos(this.labelIds[axis], tips[axis]);
      const tipPx = this.projectToScreenPx(tips[axis], camera, width, height);
      this.labels[axis].style.display =
        originPx.distanceTo(tipPx) < NEAR_ORIGIN_HIDE_PX ? "none" : "";
    }
  }

  /** NDC → CSS-px screen position, for the near-origin label hide check. */
  private projectToScreenPx(
    worldPos: THREE.Vector3,
    camera: THREE.Camera,
    width: number,
    height: number,
  ): THREE.Vector2 {
    const ndc = worldPos.clone().project(camera);
    return new THREE.Vector2((ndc.x * 0.5 + 0.5) * width, (1 - (ndc.y * 0.5 + 0.5)) * height);
  }

  /**
   * Theme change: re-bake the axis colors.
   *
   * The legs carry per-vertex colors rather than a material color (one
   * LineSegments2 draws all three), so this rewrites the geometry's color
   * attribute — there is no material color to copy into.
   */
  setColors(colors: TriadColors): void {
    (this.seg.geometry as LineSegmentsGeometry).setColors(legColors(colors));
  }

  /** Current leg length in world units. */
  get legLength(): number {
    return this.object3D.scale.x;
  }

  dispose(): void {
    this.object3D.remove(this.seg);
    this.seg.geometry.dispose();
    this.material.dispose();
    for (const axis of AXES) {
      this.deps.overlay.unregister(this.labelIds[axis]);
      this.labels[axis].remove();
    }
  }
}
