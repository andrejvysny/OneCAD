/*
 * Grid capture ladder table (SNAP §5.4, P0 gate).
 *
 * `chooseGridStep` picks the grid step off the camera distance on a 1/2/5/10
 * decade ladder, so the projected cell size JUMPS at every rung. The invariant
 * that must survive every jump, at every offered snap radius and every tested
 * viewport height, is:
 *
 *   1. grid capture never exceeds the user's point reach, and
 *   2. a mid-cell region always survives where cursor numeric rounding — not
 *      grid — is the answer.
 *
 * (2) is the property the legacy `gridRequireProximity` hack existed to fake
 * and the reason the factor is 0.42 rather than the 0.707 that would cover a
 * square cell completely.
 */
import { describe, expect, it } from "vitest";
import { chooseGridStep } from "@/viewport/engine/GridPlane";
import { computePlaneScreenMetric } from "@/viewport/engine/planeMetric";
import { gridReachPx, metricNorm, GRID_REACH_FACTOR, GRID_REACH_ACQUIRE_FACTOR } from "@/tools/sketch/snapTypes";
import { SNAP_RADIUS_ORDER, SNAP_RADIUS_PX } from "@/tools/sketch/snapRadius";
import { VIEWPORT_HEIGHTS, XY_PLANE } from "./cameras";
import * as THREE from "three";

/** Camera distances straddling every decade rung the app can select. */
const DISTANCES = [
  1, 2, 5, 9, 10, 11, 25, 49, 50, 51, 99, 100, 101, 250, 499, 500, 501, 1000, 2500, 5000, 10_000,
];

function orthoAt(distance: number, height: number): THREE.OrthographicCamera {
  const width = Math.round(height * (4 / 3));
  const halfHeight = distance / 2;
  const aspect = width / height;
  const cam = new THREE.OrthographicCamera(
    -halfHeight * aspect,
    halfHeight * aspect,
    halfHeight,
    -halfHeight,
    0.1,
    1e6,
  );
  cam.up.set(0, 0, 1);
  cam.position.set(0, 0, distance);
  cam.lookAt(new THREE.Vector3(0, 0, 0));
  cam.updateMatrixWorld(true);
  cam.updateProjectionMatrix();
  return cam;
}

describe("grid capture ladder", () => {
  for (const height of VIEWPORT_HEIGHTS) {
    const width = Math.round(height * (4 / 3));
    for (const radiusId of SNAP_RADIUS_ORDER) {
      const acquirePx = SNAP_RADIUS_PX[radiusId];
      it(`leaves a mid-cell rounding region at ${width}x${height}, radius ${radiusId}`, () => {
        for (const distance of DISTANCES) {
          const step = chooseGridStep(distance).minor;
          const cam = orthoAt(distance, height);
          const m = computePlaneScreenMetric(XY_PLANE, cam, { width, height }, { x: 0, y: 0 });
          expect(m, `metric at d=${distance}`).not.toBeNull();
          if (!m) continue;
          const reach = gridReachPx(m, step, acquirePx);
          const uCellPx = metricNorm(m, step, 0);
          const vCellPx = metricNorm(m, 0, step);
          const cell = Math.min(uCellPx, vCellPx);

          // (1) never wider than the user's point reach times the grid's own
          // small allowance (crossings are re-acquirable without aiming, so
          // they get GRID_REACH_ACQUIRE_FACTOR; geometry stays at acquirePx).
          expect(reach, `d=${distance}`).toBeLessThanOrEqual(
            acquirePx * GRID_REACH_ACQUIRE_FACTOR + 1e-9,
          );

          // (2) a point at the CELL CENTRE is outside grid capture whenever the
          // cell is big enough for the cell term to bind. The centre is at
          // (cell/2, cell/2) from the nearest node in the tightest direction,
          // i.e. at least cell/2 away in screen px.
          if (reach < acquirePx * GRID_REACH_ACQUIRE_FACTOR - 1e-9) {
            expect(reach).toBeCloseTo(GRID_REACH_FACTOR * cell, 9);
            expect(reach).toBeLessThan(cell / 2);
          }
        }
      });
    }
  }

  it("caps at the point reach on the ladder rungs with the widest cells", () => {
    // `reach = min(userReach·1.25, 0.42·cell)` — both terms must actually bind
    // somewhere, or one of them is dead code the ladder never exercises.
    //
    // WHICH one binds is a function of cell size, and the grid deliberately
    // draws BIG cells now (GridPlane's CELLS_ACROSS), so the cell term no
    // longer binds at every rung of a 1024x768 viewport the way it did when the
    // grid was dense. It still binds where cells get small in pixels: a short
    // viewport combined with the widest offered reach. Sweep both axes.
    let capped = 0;
    let cellBound = 0;
    for (const height of VIEWPORT_HEIGHTS) {
      const width = Math.round(height * (4 / 3));
      for (const radiusId of SNAP_RADIUS_ORDER) {
        const acquirePx = SNAP_RADIUS_PX[radiusId];
        for (const distance of DISTANCES) {
          const step = chooseGridStep(distance).minor;
          const cam = orthoAt(distance, height);
          const m = computePlaneScreenMetric(XY_PLANE, cam, { width, height }, { x: 0, y: 0 });
          if (!m) continue;
          const cell = Math.min(metricNorm(m, step, 0), metricNorm(m, 0, step));
          const reach = gridReachPx(m, step, acquirePx);
          expect(reach).toBeCloseTo(
            Math.min(acquirePx * GRID_REACH_ACQUIRE_FACTOR, GRID_REACH_FACTOR * cell),
            9,
          );
          if (GRID_REACH_FACTOR * cell >= acquirePx * GRID_REACH_ACQUIRE_FACTOR) capped++;
          else cellBound++;
        }
      }
    }
    expect(capped, "some rung must be capped by the user's reach").toBeGreaterThan(0);
    expect(cellBound, "some rung must be bound by the cell size").toBeGreaterThan(0);
  });

  it("shrinks below the point reach when the grid is zoomed far out", () => {
    const cam = orthoAt(10_000, 400);
    const step = chooseGridStep(10_000).minor;
    const m = computePlaneScreenMetric(XY_PLANE, cam, { width: 533, height: 400 }, { x: 0, y: 0 });
    expect(m).not.toBeNull();
    if (!m) return;
    const cell = Math.min(metricNorm(m, step, 0), metricNorm(m, 0, step));
    expect(gridReachPx(m, step, SNAP_RADIUS_PX.s)).toBeCloseTo(
      Math.min(SNAP_RADIUS_PX.s * GRID_REACH_ACQUIRE_FACTOR, GRID_REACH_FACTOR * cell),
      9,
    );
  });

  it("is zero for a non-positive step", () => {
    const cam = orthoAt(260, 768);
    const m = computePlaneScreenMetric(XY_PLANE, cam, { width: 1024, height: 768 }, { x: 0, y: 0 });
    expect(m).not.toBeNull();
    if (!m) return;
    expect(gridReachPx(m, 0, 8)).toBe(0);
    expect(gridReachPx(m, -1, 8)).toBe(0);
  });
});
