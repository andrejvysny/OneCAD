/*
 * Always-visible origin axis triad.
 *
 * Three colored legs from the world origin along +X/+Y/+Z, sized to the
 * grid's major step (see GridPlane.chooseGridStep) so the triad reads as
 * "this is the origin" at any zoom level without dominating the view. Unlike
 * GridPlane, the triad is NOT gated by gridVisible — the viewport should
 * never lose its sense of orientation, grid on or off.
 */
import * as THREE from "three";
import { chooseGridStep } from "./GridPlane";

interface TriadColors {
  x: THREE.Color;
  y: THREE.Color;
  z: THREE.Color;
}

export class OriginTriad {
  readonly object3D: THREE.Group;
  private readonly colors: TriadColors;
  private seg: THREE.LineSegments | null = null;
  private currentLength = 0;

  constructor(colors: TriadColors) {
    this.colors = colors;
    this.object3D = new THREE.Group();
    this.object3D.name = "originTriad";
    // Draw alongside the grid, behind solid geometry.
    this.object3D.renderOrder = -1;
  }

  /** Rescale the legs to the grid's major step for the current camera distance. */
  update(cameraDistance: number): void {
    const { major } = chooseGridStep(cameraDistance);
    if (major !== this.currentLength) {
      this.rebuild(major);
      this.currentLength = major;
    }
  }

  private rebuild(length: number): void {
    this.disposeSeg();

    // One LineSegments, three legs, vertex colors (simpler than three objects).
    const pos = [
      0, 0, 0, length, 0, 0, // +X
      0, 0, 0, 0, length, 0, // +Y
      0, 0, 0, 0, 0, length, // +Z
    ];
    const col: number[] = [];
    for (const c of [this.colors.x, this.colors.y, this.colors.z]) {
      col.push(c.r, c.g, c.b, c.r, c.g, c.b);
    }

    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    geo.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));

    this.seg = new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ vertexColors: true }));
    this.seg.renderOrder = -1;
    this.object3D.add(this.seg);
  }

  private disposeSeg(): void {
    if (!this.seg) return;
    this.object3D.remove(this.seg);
    this.seg.geometry.dispose();
    (this.seg.material as THREE.Material).dispose();
    this.seg = null;
  }

  dispose(): void {
    this.disposeSeg();
  }
}
