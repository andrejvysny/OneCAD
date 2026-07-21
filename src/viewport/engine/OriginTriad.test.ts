import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { OriginTriad } from "./OriginTriad";

function makeTriad(): OriginTriad {
  return new OriginTriad({
    x: new THREE.Color(1, 0, 0),
    y: new THREE.Color(0, 1, 0),
    z: new THREE.Color(0, 0, 1),
  });
}

/** Perspective camera looking at the world origin from `distance` up the +Z axis. */
function perspAt(distance: number, fov = 76): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(fov, 4 / 3, 0.1, 10_000);
  cam.position.set(0, 0, distance);
  cam.lookAt(0, 0, 0);
  cam.updateMatrixWorld(true);
  return cam;
}

/** Apparent leg length in CSS px, projected through the camera. */
function screenLength(triad: OriginTriad, cam: THREE.Camera, height: number): number {
  const tip = new THREE.Vector3(0, triad.legLength, 0).project(cam);
  const origin = new THREE.Vector3(0, 0, 0).project(cam);
  // NDC spans 2 across the viewport, so half the height converts NDC → px.
  return Math.abs(tip.y - origin.y) * (height / 2);
}

describe("OriginTriad", () => {
  it("holds a constant on-screen leg length across zoom (perspective)", () => {
    const triad = makeTriad();
    const height = 800;

    triad.update(perspAt(50), 1000, height, 1);
    const near = screenLength(triad, perspAt(50), height);
    triad.update(perspAt(5000), 1000, height, 1);
    const far = screenLength(triad, perspAt(5000), height);

    expect(near).toBeCloseTo(far, 3);
    // …and it is the size we asked for, not merely stable.
    expect(near).toBeCloseTo(110, 3);

    triad.dispose();
  });

  it("scales world length linearly with camera depth — no decade jumps", () => {
    const triad = makeTriad();
    triad.update(perspAt(100), 1000, 800, 1);
    const at100 = triad.legLength;
    triad.update(perspAt(200), 1000, 800, 1);
    const at200 = triad.legLength;
    // A 1/5/10 step would snap both to the same value; a linear scale doubles.
    expect(at200 / at100).toBeCloseTo(2, 6);

    // Smooth in the small: a 1% dolly moves the length ~1%, never 0 or 10x.
    triad.update(perspAt(101), 1000, 800, 1);
    expect(triad.legLength / at100).toBeCloseTo(1.01, 6);

    triad.dispose();
  });

  it("uses view-axis depth, not orbit distance, when the camera is panned off-origin", () => {
    const triad = makeTriad();
    // Same depth from the origin plane, but shifted sideways: the triad is
    // farther from the camera in a straight-line sense, yet the same size.
    const straight = perspAt(300);
    const panned = new THREE.PerspectiveCamera(76, 4 / 3, 0.1, 10_000);
    panned.position.set(400, 0, 300);
    panned.lookAt(400, 0, 0);
    panned.updateMatrixWorld(true);

    triad.update(straight, 1000, 800, 1);
    const a = triad.legLength;
    triad.update(panned, 1000, 800, 1);
    expect(triad.legLength).toBeCloseTo(a, 6);

    triad.dispose();
  });

  it("sizes from the frustum height under an orthographic camera", () => {
    const triad = makeTriad();
    const ortho = new THREE.OrthographicCamera(-200, 200, 150, -150, 0.1, 10_000);
    ortho.position.set(0, 0, 500);
    ortho.lookAt(0, 0, 0);
    ortho.updateMatrixWorld(true);

    triad.update(ortho, 1000, 600, 1);
    // (top - bottom) / height = 300/600 = 0.5 world units per px.
    expect(triad.legLength).toBeCloseTo(0.5 * 110, 6);

    // Ortho has no perspective falloff — dollying must not resize the triad.
    ortho.position.set(0, 0, 5000);
    ortho.updateMatrixWorld(true);
    triad.update(ortho, 1000, 600, 1);
    expect(triad.legLength).toBeCloseTo(0.5 * 110, 6);

    triad.dispose();
  });

  it("tracks the fat-line resolution in device pixels", () => {
    const triad = makeTriad();
    triad.update(perspAt(250), 800, 600, 2);
    const seg = triad.object3D.children[0] as LineSegments2;
    const mat = seg.material as { resolution: THREE.Vector2; linewidth: number };
    expect(mat.resolution.x).toBe(1600);
    expect(mat.resolution.y).toBe(1200);
    expect(mat.linewidth).toBeGreaterThan(1);
    triad.dispose();
  });

  it("offsets in depth so the coplanar X/Y legs do not z-fight the grid", () => {
    const triad = makeTriad();
    const seg = triad.object3D.children[0] as LineSegments2;
    const mat = seg.material as unknown as THREE.Material;
    expect(mat.polygonOffset).toBe(true);
    expect(mat.polygonOffsetFactor).toBeLessThan(0);
    // Still depth-tested, so solid bodies occlude the triad.
    expect(mat.depthTest).toBe(true);
    triad.dispose();
  });

  it("reuses one geometry across updates instead of rebuilding per frame", () => {
    const triad = makeTriad();
    triad.update(perspAt(250), 1000, 800, 1);
    const seg1 = triad.object3D.children[0];
    triad.update(perspAt(900), 1000, 800, 1);
    expect(triad.object3D.children[0]).toBe(seg1);
    expect(triad.object3D.children.length).toBe(1);
    triad.dispose();
  });

  it("dispose releases geometry + material and empties the group", () => {
    const triad = makeTriad();
    const seg = triad.object3D.children[0] as LineSegments2;
    const geoSpy = vi.spyOn(seg.geometry, "dispose");
    const matSpy = vi.spyOn(seg.material as unknown as THREE.Material, "dispose");

    triad.dispose();

    expect(geoSpy).toHaveBeenCalledOnce();
    expect(matSpy).toHaveBeenCalledOnce();
    expect(triad.object3D.children.length).toBe(0);
  });
});
