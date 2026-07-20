import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { OriginTriad } from "./OriginTriad";
import { chooseGridStep } from "./GridPlane";

function makeTriad(): OriginTriad {
  return new OriginTriad({
    x: new THREE.Color(1, 0, 0),
    y: new THREE.Color(0, 1, 0),
    z: new THREE.Color(0, 0, 1),
  });
}

describe("OriginTriad", () => {
  it("sizes each leg to chooseGridStep(distance).major", () => {
    const triad = makeTriad();
    triad.update(250);
    const expected = chooseGridStep(250).major;

    const seg = triad.object3D.children[0] as THREE.LineSegments;
    const pos = seg.geometry.getAttribute("position").array;
    // Endpoints: +X at index 3, +Y at index 10, +Z at index 17.
    expect(pos[3]).toBeCloseTo(expected, 9);
    expect(pos[10]).toBeCloseTo(expected, 9);
    expect(pos[17]).toBeCloseTo(expected, 9);

    triad.dispose();
  });

  it("grows the legs when the camera distance crosses a decade step", () => {
    const triad = makeTriad();
    triad.update(50);
    const near = chooseGridStep(50).major;
    triad.update(5000);
    const far = chooseGridStep(5000).major;
    expect(far).toBeGreaterThan(near);

    const seg = triad.object3D.children[0] as THREE.LineSegments;
    const pos = seg.geometry.getAttribute("position").array;
    expect(pos[3]).toBeCloseTo(far, 9);

    triad.dispose();
  });

  it("does not rebuild when the step is unchanged", () => {
    const triad = makeTriad();
    triad.update(250);
    const seg1 = triad.object3D.children[0];
    triad.update(260); // same 1/5/10 decade step as 250
    const seg2 = triad.object3D.children[0];
    expect(seg2).toBe(seg1);
    triad.dispose();
  });

  it("dispose releases geometry + material and empties the group", () => {
    const triad = makeTriad();
    triad.update(250);
    expect(triad.object3D.children.length).toBe(1);

    const seg = triad.object3D.children[0] as THREE.LineSegments;
    const geoSpy = vi.spyOn(seg.geometry, "dispose");
    const matSpy = vi.spyOn(seg.material as THREE.Material, "dispose");

    triad.dispose();

    expect(geoSpy).toHaveBeenCalledOnce();
    expect(matSpy).toHaveBeenCalledOnce();
    expect(triad.object3D.children.length).toBe(0);
  });
});
