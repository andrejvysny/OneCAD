/*
 * The plane→screen metric accuracy table (SNAP §5.3, P0 gate).
 *
 * The metric is a LOCAL linearization: it stands in for exact projection while
 * scoring candidates, and the whole spatial model rests on it being accurate
 * over the reach it is used at. The reach is the largest release radius (17px),
 * so this asserts agreement to 0.25px over displacements up to 12 plane-units'
 * worth of screen distance in every camera case, including the ones where a
 * perspective projection is least linear.
 */
import { describe, expect, it } from "vitest";
import {
  computePlaneScreenMetric,
  newPlaneMetricScratch,
  projectPlanePoint,
} from "@/viewport/engine/planeMetric";
import { metricApply, metricDet, metricUsable } from "@/tools/sketch/snapTypes";
import { cameraCases, XY_PLANE } from "./cameras";
import * as THREE from "three";

/** Displacements (CSS px) the local metric is asked to predict. */
const REACH_PX = [1, 2, 4, 8, 12] as const;
const DIRECTIONS = 16;
const MAX_ERROR_PX = 0.25;

describe("planeScreenMetric", () => {
  for (const c of cameraCases()) {
    it(`predicts exact projection within ${MAX_ERROR_PX}px — ${c.name}`, () => {
      const scratch = newPlaneMetricScratch();
      const m = computePlaneScreenMetric(c.plane, c.camera, c.viewport, c.at, scratch);
      expect(metricUsable(m)).toBe(true);
      if (!m) return;
      // Copy: `scratch.out` is reused by the projection calls below.
      const metric = { ...m };
      const origin = projectPlanePoint(c.plane, c.camera, c.viewport, c.at);
      expect(origin).not.toBeNull();
      if (!origin) return;
      const o = { ...origin };

      // Screen px per plane unit along each axis — used to convert a target
      // SCREEN displacement into the plane delta that produces it.
      const uScale = Math.hypot(metric.m00, metric.m10);
      const vScale = Math.hypot(metric.m01, metric.m11);
      const scale = Math.max(uScale, vScale);
      expect(scale).toBeGreaterThan(0);

      let worst = 0;
      for (const px of REACH_PX) {
        for (let k = 0; k < DIRECTIONS; k++) {
          const th = (k / DIRECTIONS) * Math.PI * 2;
          // A plane delta whose screen length is at most `px`.
          const du = (px / scale) * Math.cos(th);
          const dv = (px / scale) * Math.sin(th);
          const predicted = metricApply(metric, du, dv);
          const exact = projectPlanePoint(c.plane, c.camera, c.viewport, {
            x: c.at.x + du,
            y: c.at.y + dv,
          });
          expect(exact).not.toBeNull();
          if (!exact) return;
          worst = Math.max(
            worst,
            Math.hypot(o.x + predicted.x - exact.x, o.y + predicted.y - exact.y),
          );
        }
      }
      expect(worst).toBeLessThanOrEqual(MAX_ERROR_PX);
    });
  }

  it("is anisotropic under an oblique camera (the reason a scalar is wrong)", () => {
    const c = cameraCases().find((x) => x.name === "perspective steep oblique");
    expect(c).toBeDefined();
    if (!c) return;
    const m = computePlaneScreenMetric(c.plane, c.camera, c.viewport, c.at);
    expect(m).not.toBeNull();
    if (!m) return;
    const uScale = Math.hypot(m.m00, m.m10);
    const vScale = Math.hypot(m.m01, m.m11);
    // A scalar `pixelWorld` would report ONE of these for both directions.
    expect(Math.max(uScale, vScale) / Math.min(uScale, vScale)).toBeGreaterThan(1.5);
  });

  it("rejects a point behind the camera", () => {
    const cam = new THREE.PerspectiveCamera(50, 2, 0.1, 1000);
    cam.up.set(0, 0, 1);
    cam.position.set(0, 0, 100);
    cam.lookAt(new THREE.Vector3(0, 0, 0));
    cam.updateMatrixWorld(true);
    // A plane far behind the eye: the projection has w <= 0.
    const behind = { ...XY_PLANE, origin: [0, 0, 500] as [number, number, number] };
    expect(
      computePlaneScreenMetric(behind, cam, { width: 800, height: 400 }, { x: 0, y: 0 }),
    ).toBeNull();
  });

  it("rejects an edge-on (degenerate) view", () => {
    const cam = new THREE.OrthographicCamera(-100, 100, 50, -50, 0.1, 1000);
    cam.up.set(0, 0, 1);
    // Eye IN the sketch plane looking along it: u/v project onto one screen line.
    cam.position.set(0, -200, 0);
    cam.lookAt(new THREE.Vector3(0, 0, 0));
    cam.updateMatrixWorld(true);
    const m = computePlaneScreenMetric(XY_PLANE, cam, { width: 800, height: 400 }, { x: 0, y: 0 });
    expect(m).toBeNull();
  });

  it("rejects a zero-sized viewport", () => {
    const c = cameraCases()[0];
    expect(computePlaneScreenMetric(c.plane, c.camera, { width: 0, height: 0 }, c.at)).toBeNull();
  });

  it("allocates nothing per call once scratch exists", () => {
    const c = cameraCases()[0];
    const scratch = newPlaneMetricScratch();
    const a = computePlaneScreenMetric(c.plane, c.camera, c.viewport, c.at, scratch);
    const b = computePlaneScreenMetric(c.plane, c.camera, c.viewport, c.at, scratch);
    expect(a).toBe(b); // same scratch object, not a fresh allocation
    expect(a).toBe(scratch.out);
  });

  it("has a right-handed determinant sign for a face-on view", () => {
    const c = cameraCases()[0];
    const m = computePlaneScreenMetric(c.plane, c.camera, c.viewport, c.at);
    expect(m).not.toBeNull();
    if (!m) return;
    // Screen y grows DOWNWARD, so a CCW plane basis projects to a negative det.
    expect(metricDet(m)).toBeLessThan(0);
  });
});
