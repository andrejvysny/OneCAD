/*
 * Live closure fills — EVEN-ODD nesting (audit item #2 residual: the donut hole).
 *
 * The v1 fill painted every closed loop independently, so a circle inside a
 * rectangle covered its own hole and the preview claimed material the kernel
 * would never build. These tests are about AREA, not mesh counts: a fill that
 * paints the hole and a fill that subtracts it produce the same object graph and
 * differ only in how much of the plane the triangles cover.
 *
 * THREE is real (jsdom-safe: nothing here needs a GL context); no renderer.
 */
import { describe, it, expect } from "vitest";
import * as THREE from "three";
import { buildNestedFillGeometries } from "./sketchFillGeometry";

type Ring = [number, number][];

/** A rectangle ring, CCW, with no repeated closing point (what `chainRing`
 *  hands over). */
const rect = (x0: number, y0: number, x1: number, y1: number): Ring => [
  [x0, y0],
  [x1, y0],
  [x1, y1],
  [x0, y1],
];

/** A circle sampled the way `entityPolyline` samples one. 128 segments puts the
 *  inscribed polygon within 0.02% of the true area, well inside the 1% the
 *  annulus assertions allow. */
const circle = (cx: number, cy: number, r: number, n = 128): Ring =>
  Array.from({ length: n }, (_, i): [number, number] => {
    const a = (2 * Math.PI * i) / n;
    return [cx + r * Math.cos(a), cy + r * Math.sin(a)];
  });

/** Total (unsigned) triangle area of one built geometry. */
function area(geo: THREE.BufferGeometry): number {
  const pos = geo.getAttribute("position");
  const idx = geo.getIndex()!;
  let sum = 0;
  for (let i = 0; i < idx.count; i += 3) {
    const [a, b, c] = [idx.getX(i), idx.getX(i + 1), idx.getX(i + 2)];
    sum +=
      Math.abs(
        (pos.getX(b) - pos.getX(a)) * (pos.getY(c) - pos.getY(a)) -
          (pos.getX(c) - pos.getX(a)) * (pos.getY(b) - pos.getY(a)),
      ) / 2;
  }
  return sum;
}

const totalArea = (geos: THREE.BufferGeometry[]): number => geos.reduce((n, g) => n + area(g), 0);

/** Every triangle centroid in a geometry — used to prove a hole is EMPTY, not
 *  merely that the totals happen to add up. */
function centroids(geo: THREE.BufferGeometry): [number, number][] {
  const pos = geo.getAttribute("position");
  const idx = geo.getIndex()!;
  const out: [number, number][] = [];
  for (let i = 0; i < idx.count; i += 3) {
    const [a, b, c] = [idx.getX(i), idx.getX(i + 1), idx.getX(i + 2)];
    out.push([
      (pos.getX(a) + pos.getX(b) + pos.getX(c)) / 3,
      (pos.getY(a) + pos.getY(b) + pos.getY(c)) / 3,
    ]);
  }
  return out;
}

describe("buildNestedFillGeometries — even-odd nesting", () => {
  it("subtracts a circle inside a circle: the donut paints an ANNULUS, not a disk", () => {
    const geos = buildNestedFillGeometries([circle(0, 0, 10), circle(0, 0, 4)]);
    expect(geos).toHaveLength(1);
    const annulus = Math.PI * (10 * 10 - 4 * 4);
    // The v1 behavior — two independent fills — totals π(100+16) = 364.4 here;
    // the disk alone is 314.2. Only 263.9 is the hole actually being subtracted.
    expect(totalArea(geos)).toBeGreaterThan(annulus * 0.99);
    expect(totalArea(geos)).toBeLessThan(annulus * 1.01);
  });

  it("puts NO triangle inside the donut's hole", () => {
    const [geo] = buildNestedFillGeometries([circle(0, 0, 10), circle(0, 0, 4)]);
    const inHole = centroids(geo).filter(([x, y]) => Math.hypot(x, y) < 4 * 0.999);
    expect(inHole).toEqual([]);
  });

  it("subtracts a rectangle inside a rectangle", () => {
    const geos = buildNestedFillGeometries([rect(-10, -10, 10, 10), rect(-5, -5, 5, 5)]);
    expect(geos).toHaveLength(1);
    expect(totalArea(geos)).toBeCloseTo(400 - 100, 6);
  });

  it("subtracts a circle from an enclosing RECTANGLE loop (the audited case)", () => {
    const geos = buildNestedFillGeometries([rect(-10, -10, 10, 10), circle(0, 0, 3)]);
    expect(geos).toHaveLength(1);
    const expected = 400 - Math.PI * 9;
    expect(totalArea(geos)).toBeGreaterThan(expected * 0.99);
    expect(totalArea(geos)).toBeLessThan(expected * 1.01);
  });

  it("subtracts a rectangle from an enclosing CIRCLE too — either shape can be the hole", () => {
    const geos = buildNestedFillGeometries([circle(0, 0, 10), rect(-2, -2, 2, 2)]);
    expect(geos).toHaveLength(1);
    const expected = Math.PI * 100 - 16;
    expect(totalArea(geos)).toBeGreaterThan(expected * 0.99);
    expect(totalArea(geos)).toBeLessThan(expected * 1.01);
  });

  it("PAINTS an island drawn inside a hole — depth 2 is material again", () => {
    const geos = buildNestedFillGeometries([
      rect(-20, -20, 20, 20), // depth 0 — material
      rect(-10, -10, 10, 10), // depth 1 — hole in the above
      rect(-5, -5, 5, 5), //     depth 2 — island, paints on its own
    ]);
    expect(geos).toHaveLength(2);
    // 1600 − 400 (hole) + 100 (island). The island's own area is NOT subtracted
    // from the outer ring twice.
    expect(totalArea(geos)).toBeCloseTo(1600 - 400 + 100, 6);
  });

  it("keeps going: a hole inside the island is subtracted from the island", () => {
    const geos = buildNestedFillGeometries([
      rect(-20, -20, 20, 20),
      rect(-10, -10, 10, 10),
      rect(-5, -5, 5, 5),
      rect(-2, -2, 2, 2),
    ]);
    expect(geos).toHaveLength(2);
    expect(totalArea(geos)).toBeCloseTo(1600 - 400 + 100 - 16, 6);
  });

  it("leaves DISJOINT loops alone — two fills, full area each", () => {
    const geos = buildNestedFillGeometries([rect(0, 0, 10, 10), rect(20, 0, 30, 10)]);
    expect(geos).toHaveLength(2);
    expect(area(geos[0])).toBeCloseTo(100, 6);
    expect(area(geos[1])).toBeCloseTo(100, 6);
  });

  it("does not misclassify two loops that SHARE AN EDGE", () => {
    // The v1-vertex-sampling trap: every vertex of the right rectangle's left
    // edge lies exactly ON the left rectangle's boundary, where a crossing test
    // is a coin flip — one flip would make a neighbour a hole.
    const geos = buildNestedFillGeometries([rect(0, 0, 10, 10), rect(10, 0, 20, 10)]);
    expect(geos).toHaveLength(2);
    expect(totalArea(geos)).toBeCloseTo(200, 6);
  });

  it("does not misclassify two loops that share a single CORNER", () => {
    const geos = buildNestedFillGeometries([rect(0, 0, 10, 10), rect(10, 10, 20, 20)]);
    expect(geos).toHaveLength(2);
    expect(totalArea(geos)).toBeCloseTo(200, 6);
  });

  it("subtracts a hole that TOUCHES the outer wall", () => {
    // A pocket drawn flush against an edge: shares the segment x = 0 with the
    // outer ring, and is still strictly enclosed by it.
    const geos = buildNestedFillGeometries([rect(0, 0, 20, 20), rect(0, 5, 5, 10)]);
    expect(geos).toHaveLength(1);
    expect(totalArea(geos)).toBeCloseTo(400 - 25, 6);
  });

  it("drops degenerate rings instead of emitting empty meshes", () => {
    const collinear: Ring = [
      [0, 0],
      [1, 0],
      [2, 0],
    ];
    expect(buildNestedFillGeometries([collinear])).toEqual([]);
    expect(
      buildNestedFillGeometries([
        [
          [0, 0],
          [1, 1],
        ],
      ]),
    ).toEqual([]);
    expect(buildNestedFillGeometries([])).toEqual([]);
  });

  it("still fills a lone ring with no holes", () => {
    const geos = buildNestedFillGeometries([rect(0, 0, 10, 10)]);
    expect(geos).toHaveLength(1);
    expect(area(geos[0])).toBeCloseTo(100, 6);
  });

  it("is orientation-agnostic — a CW-authored hole still subtracts", () => {
    const cw = [...rect(-5, -5, 5, 5)].reverse();
    const geos = buildNestedFillGeometries([rect(-10, -10, 10, 10), cw]);
    expect(geos).toHaveLength(1);
    expect(totalArea(geos)).toBeCloseTo(300, 6);
  });
});
