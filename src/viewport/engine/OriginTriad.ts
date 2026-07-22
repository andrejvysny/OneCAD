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
import { worldPerPixel } from "./screenScale";
import { RENDER_ORDER } from "./renderOrder";

interface TriadColors {
  x: THREE.Color;
  y: THREE.Color;
  z: THREE.Color;
}

/** Leg width in CSS px. Same weight as sketch lines. */
const TRIAD_WIDTH = 2;

/** Leg length in CSS px, held constant across zoom. */
const TRIAD_PX = 110;

export class OriginTriad {
  readonly object3D: THREE.Group;
  private readonly material: LineMaterial;
  private readonly seg: LineSegments2;

  constructor(colors: TriadColors) {
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
    });

    // One LineSegments2, three unit-length legs, per-vertex colors. Built once:
    // zoom only changes object3D.scale, so there is no per-frame reallocation.
    const geo = new LineSegmentsGeometry();
    geo.setPositions([
      0, 0, 0, 1, 0, 0, // +X
      0, 0, 0, 0, 1, 0, // +Y
      0, 0, 0, 0, 0, 1, // +Z
    ]);
    const col: number[] = [];
    for (const c of [colors.x, colors.y, colors.z]) {
      col.push(c.r, c.g, c.b, c.r, c.g, c.b);
    }
    geo.setColors(col);

    this.seg = new LineSegments2(geo, this.material);
    this.seg.renderOrder = RENDER_ORDER.TRIAD;
    // Tiny, always at the origin, and the bounding sphere ignores the group
    // scale — culling it is never a win and can wrongly drop it.
    this.seg.frustumCulled = false;
    this.object3D.add(this.seg);
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
  }

  /** Current leg length in world units. */
  get legLength(): number {
    return this.object3D.scale.x;
  }

  dispose(): void {
    this.object3D.remove(this.seg);
    this.seg.geometry.dispose();
    this.material.dispose();
  }
}
