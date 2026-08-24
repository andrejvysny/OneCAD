import { describe, it, expect } from "vitest";
import * as THREE from "three";
import {
  layoutBadges,
  entityAnchor,
  entityPointCoord,
  GLYPH_STANDOFF_PX,
  DIMENSION_STANDOFF_PX,
  type ConstraintBadge,
} from "./badgeLayout";
import { planeFor } from "@/ipc/mockSketch";
import { HtmlOverlayDriver } from "@/viewport/engine/HtmlOverlayDriver";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import type { SketchPlane, SketchSession } from "@/ipc/types";

const session: SketchSession = {
  sketchId: "sk",
  plane: planeFor("XY"),
  entities: [
    { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
    { id: "e2", type: "Line", p0: [40, 0], p1: [40, 20] },
    { id: "c1", type: "Circle", center: [10, 10], radius: 5 },
  ],
  constraints: [
    { id: "k1", type: "Horizontal", entities: ["e1"] },
    { id: "k2", type: "Coincident", entities: ["e2", "e1"], positions: ["Start", "End"] },
    { id: "k3", type: "Distance", entities: ["e1"], value: 40 },
    { id: "k4", type: "Radius", entities: ["c1"], value: 5 },
  ],
  dof: 2,
  status: "UnderConstrained",
};

describe("entityAnchor / entityPointCoord", () => {
  it("line anchor is the midpoint", () => {
    expect(entityAnchor(session.entities[0])).toEqual({ x: 20, y: 0 });
  });
  it("circle anchor is the center", () => {
    expect(entityAnchor(session.entities[2])).toEqual({ x: 10, y: 10 });
  });
  it("named point coords resolve", () => {
    expect(entityPointCoord(session.entities[0], "End")).toEqual({ x: 40, y: 0 });
    expect(entityPointCoord(session.entities[1], "Start")).toEqual({ x: 40, y: 0 });
  });
});

describe("layoutBadges", () => {
  const badges = layoutBadges(session);

  // GLYPH STANDOFF — CHANGED DELIBERATELY (SKETCH_UX_AUDIT #5 residual): the
  // standoff used to be baked into `at` as 10mm of PLANE space, so these pins
  // read a nudged anchor ({x:20,y:-10} here). It is now a SCREEN-px offset the
  // overlay driver applies per frame (`standoffPx`), so `at` is the raw entity
  // anchor again and the on-screen clearance no longer scales with zoom. See
  // badgeLayout.ts's `GLYPH_STANDOFF_PX` and the projection describe below.
  it("anchors a Horizontal glyph AT the line midpoint, standing it off in screen px", () => {
    const h = badges.find((b) => b.id === "k1")!;
    expect(h.glyph).toBe("H");
    expect(h.at).toEqual({ x: 20, y: 0 });
    expect(h.standoffPx).toBe(-GLYPH_STANDOFF_PX);
    expect(h.editable).toBe(false);
  });

  it("anchors a Coincident dot AT the shared point, standing it off in screen px", () => {
    const c = badges.find((b) => b.id === "k2")!;
    expect(c.glyph).toBe("•");
    // k2 is anchored at e2's Start=[40,0]; axisFrom is e2's OTHER endpoint
    // [40,20] (see the dedicated test below) — the direction the driver takes
    // its perpendicular from.
    expect(c.at).toEqual({ x: 40, y: 0 });
    expect(c.standoffPx).toBe(-GLYPH_STANDOFF_PX);
  });

  it("marks dimensional constraints editable with a value glyph", () => {
    const d = badges.find((b) => b.id === "k3")!;
    expect(d.editable).toBe(true);
    expect(d.value).toBe(40);
    expect(d.glyph).toBe("40");
    // Positive standoff = the driver's default side, i.e. the OTHER side of the
    // entity from the glyph badges (which carry a negative one).
    expect(d.standoffPx).toBe(DIMENSION_STANDOFF_PX);
    const r = badges.find((b) => b.id === "k4")!;
    expect(r.editable).toBe(true);
    expect(r.at).toEqual({ x: 10, y: 10 });
  });

  it("returns [] for a null session", () => {
    expect(layoutBadges(null)).toEqual([]);
  });

  it("gives a line-anchored badge its endpoint as the leader axisFrom", () => {
    const h = badges.find((b) => b.id === "k1")!;
    // e1 is p0=[0,0], p1=[40,0] — the axis runs from p0 to the midpoint, and
    // the driver offsets perpendicular to its PROJECTION. Raw p0, not a nudged
    // copy: with the standoff now living in screen px there is no plane-space
    // delta to carry (it used to read {x:0,y:-10}).
    expect(h.axisFrom).toEqual({ x: 0, y: 0 });
  });

  it("gives a circle-anchored badge a point on its boundary as axisFrom", () => {
    const r = badges.find((b) => b.id === "k4")!;
    // c1 centre=[10,10], radius=5 — boundary point along +U from the centre.
    expect(r.axisFrom).toEqual({ x: 15, y: 10 });
  });

  it("a Coincident badge's axisFrom runs along the entity's own axis, away from the shared point", () => {
    const c = badges.find((b) => b.id === "k2")!;
    // k2 is anchored at e2's Start = [40,0] (the point e1 and e2 share). e2 is
    // p0=[40,0], p1=[40,20] — the axis runs away from the shared point toward
    // e2's OTHER endpoint, so the leader (and the standoff direction) reads
    // off THAT axis, not a zero-length one degenerating at the vertex itself.
    expect(c.axisFrom).toEqual({ x: 40, y: 20 });
    expect(c.at).toEqual({ x: 40, y: 0 });
  });

  // CHANGED DELIBERATELY with the screen-px standoff: k1 (Horizontal, a GLYPH)
  // and k3 (Distance, DIMENSIONAL) are both anchored at e1's midpoint again —
  // the plane-space bake used to separate their anchors as a side effect. They
  // no longer collide on screen because their standoffs point to OPPOSITE
  // sides (negative vs positive), and the shared anchor restores the U9
  // stagger/cluster grouping the anchor key exists for.
  it("a glyph (k1) and a co-located dimensional chip (k3) share the entity's anchor and stagger", () => {
    const h = badges.find((b) => b.id === "k1")!;
    const d = badges.find((b) => b.id === "k3")!;
    expect(h.at).toEqual(d.at);
    expect(d.at).toEqual({ x: 20, y: 0 });
    expect(Math.sign(h.standoffPx)).toBe(-Math.sign(d.standoffPx));
    expect(h.offsetIndex).toBe(0);
    expect(d.offsetIndex).toBe(1);
  });

  it("a badge with no co-anchored sibling gets offsetIndex 0", () => {
    const c = badges.find((b) => b.id === "k2")!;
    const r = badges.find((b) => b.id === "k4")!;
    expect(c.offsetIndex).toBe(0);
    expect(r.offsetIndex).toBe(0);
  });
});

describe("layoutBadges — offsetIndex stagger (U9)", () => {
  it("two constraints anchored at the same point get offsetIndex 0 and 1, sorted by constraint id", () => {
    const s: SketchSession = {
      sketchId: "sk",
      plane: planeFor("XY"),
      entities: [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
      constraints: [
        // Registered in reverse-id order to prove the sort is by id, not insertion order.
        { id: "zz", type: "Horizontal", entities: ["e1"] },
        { id: "aa", type: "Fixed", entities: ["e1"] },
      ],
      dof: 0,
      status: "FullyConstrained",
    };
    const badges = layoutBadges(s);
    const aa = badges.find((b) => b.id === "aa")!;
    const zz = badges.find((b) => b.id === "zz")!;
    expect(aa.at).toEqual(zz.at);
    expect(aa.offsetIndex).toBe(0);
    expect(zz.offsetIndex).toBe(1);
  });

  it("a lone badge (no co-anchored sibling) gets offsetIndex 0", () => {
    const s: SketchSession = {
      sketchId: "sk",
      plane: planeFor("XY"),
      entities: [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
      constraints: [{ id: "k1", type: "Horizontal", entities: ["e1"] }],
      dof: 0,
      status: "FullyConstrained",
    };
    const badges = layoutBadges(s);
    expect(badges[0].offsetIndex).toBe(0);
  });

  it("two distinct anchors each get offsetIndex 0 — grouping is per-anchor, not global", () => {
    const s: SketchSession = {
      sketchId: "sk",
      plane: planeFor("XY"),
      entities: [
        { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
        { id: "e2", type: "Line", p0: [0, 100], p1: [40, 100] },
      ],
      constraints: [
        { id: "k1", type: "Horizontal", entities: ["e1"] },
        { id: "k2", type: "Horizontal", entities: ["e2"] },
      ],
      dof: 0,
      status: "FullyConstrained",
    };
    const badges = layoutBadges(s);
    expect(badges.find((b) => b.id === "k1")!.offsetIndex).toBe(0);
    expect(badges.find((b) => b.id === "k2")!.offsetIndex).toBe(0);
  });
});

describe("layoutBadges — M6c constraint glyph coverage", () => {
  const s: SketchSession = {
    sketchId: "sk",
    plane: planeFor("XY"),
    entities: [
      { id: "e1", type: "Line", p0: [0, 0], p1: [40, 40] },
      { id: "e2", type: "Line", p0: [40, 40], p1: [0, 80] },
      { id: "a1", type: "Arc", center: [10, 10], radius: 10, start: [10, 0], end: [0, 10] },
    ],
    constraints: [
      { id: "p1", type: "Perpendicular", entities: ["e2", "e1"] },
      { id: "p2", type: "Parallel", entities: ["e2", "e1"] },
      { id: "t1", type: "Tangent", entities: ["a1", "e1"] },
      { id: "an1", type: "Angle", entities: ["e1", "e2"], value: 90 },
    ],
    dof: 0,
    status: "FullyConstrained",
  };
  const badges = layoutBadges(s);

  it("renders a Perpendicular glyph", () => {
    expect(badges.find((b) => b.id === "p1")!.glyph).toBe("⟂");
  });
  it("renders a Parallel glyph", () => {
    expect(badges.find((b) => b.id === "p2")!.glyph).toBe("∥");
  });
  it("renders a Tangent glyph", () => {
    expect(badges.find((b) => b.id === "t1")!.glyph).toBe("T");
  });
  it("renders an editable Angle badge with a ° value", () => {
    const an = badges.find((b) => b.id === "an1")!;
    expect(an.editable).toBe(true);
    expect(an.value).toBe(90);
    expect(an.glyph).toBe("90°");
  });
});

/*
 * GLYPH STANDOFF, THROUGH THE PROJECTION (SKETCH_UX_AUDIT #5 residual).
 *
 * Origin of the standoff (adversarial-review M3): a `Fixed` glyph badge sat
 * exactly at a line's midpoint, out-ranked the line itself in
 * `elementsFromPoint`, and ate a select-tool click meant for the curve
 * (e2e/acceptance.spec.ts:161).
 *
 * The first fix baked 10mm of PLANE space into `at`, which could only be sized
 * for ONE zoom. Measured in the mock e2e lane (chromium, 1280x800, viewport
 * 722px tall, sketch-entry camera distance 97.51 at fov 76 ⇒ 4.74 px/mm):
 * 10mm projected to 47.39px, and the badge's actual on-screen standoff went
 * from 24.4px at sketch entry to 93.6px after one wheel-zoom in (distance
 * 39.64). The standoff is now a screen constant the driver applies per frame,
 * so these tests run `layoutBadges` output through the REAL
 * `HtmlOverlayDriver` at two zoom levels and compare pixels.
 *
 * The camera below is the sketch view `CadOrbitControls.viewAlongNormal`
 * establishes for an XY sketch: on the plane's +normal side (world +Z), the
 * plane's own xAxis (world +Y) to screen-right and its yAxis (world -X) up.
 */
describe("glyph standoff is a SCREEN constant (projected through HtmlOverlayDriver)", () => {
  const VIEW_PX = 800;
  const SKETCH_PICK_PX = 8; // SketchController.pickTol's select corridor

  function sketchCam(distance: number): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(76, 1, 0.1, 10_000);
    cam.position.set(0, 0, distance);
    cam.up.set(-1, 0, 0);
    cam.lookAt(0, 0, 0);
    cam.updateProjectionMatrix();
    cam.updateMatrixWorld();
    return cam;
  }

  const pxOf = (el: HTMLElement): { x: number; y: number } => {
    const m = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(el.style.transform);
    if (!m) throw new Error(`no pixel translate in ${el.style.transform}`);
    return { x: Number(m[1]), y: Number(m[2]) };
  };

  /** Screen position of `badge` and of its own (un-offset) anchor. */
  function place(badge: ConstraintBadge, plane: SketchPlane, cam: THREE.Camera) {
    const driver = new HtmlOverlayDriver();
    const el = document.createElement("div");
    const anchorEl = document.createElement("div");
    driver.register("badge", el, planePointToWorld(plane, badge.at), {
      axisFrom: badge.axisFrom ? planePointToWorld(plane, badge.axisFrom) : undefined,
      offsetPx: badge.standoffPx,
    });
    driver.register("anchor", anchorEl, planePointToWorld(plane, badge.at));
    driver.update(cam, VIEW_PX, VIEW_PX);
    return { badge: pxOf(el), anchor: pxOf(anchorEl) };
  }

  const lineSession = (constraints: SketchSession["constraints"]): SketchSession => ({
    sketchId: "sk",
    plane: planeFor("XY"),
    entities: [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
    constraints,
    dof: 0,
    status: "FullyConstrained",
  });

  // The two zooms this suite compares. Anything that scales with the camera
  // reads ~2.5x apart between them.
  const NEAR = 40;
  const FAR = 100;

  it("a glyph badge stands off by the SAME pixels at two zoom levels", () => {
    const s = lineSession([{ id: "fx1", type: "Fixed", entities: ["e1"] }]);
    const badge = layoutBadges(s)[0];

    const far = place(badge, s.plane, sketchCam(FAR));
    const near = place(badge, s.plane, sketchCam(NEAR));

    const offFar = Math.hypot(far.badge.x - far.anchor.x, far.badge.y - far.anchor.y);
    const offNear = Math.hypot(near.badge.x - near.anchor.x, near.badge.y - near.anchor.y);
    expect(offFar).toBeCloseTo(GLYPH_STANDOFF_PX, 4);
    expect(offNear).toBeCloseTo(GLYPH_STANDOFF_PX, 4);

    // …and the two cameras really do differ: 20mm of the line projects 2.5x
    // wider at NEAR, which is exactly what the mm-baked standoff followed.
    const widthAt = (cam: THREE.Camera) => {
      const driver = new HtmlOverlayDriver();
      const a = document.createElement("div");
      const b = document.createElement("div");
      driver.register("a", a, planePointToWorld(s.plane, { x: 0, y: 0 }));
      driver.register("b", b, planePointToWorld(s.plane, { x: 20, y: 0 }));
      driver.update(cam, VIEW_PX, VIEW_PX);
      return Math.abs(pxOf(b).x - pxOf(a).x);
    };
    expect(widthAt(sketchCam(NEAR)) / widthAt(sketchCam(FAR))).toBeCloseTo(FAR / NEAR, 3);
  });

  it("the standoff clears the curve's own click corridor at BOTH zooms", () => {
    const s = lineSession([{ id: "fx1", type: "Fixed", entities: ["e1"] }]);
    const badge = layoutBadges(s)[0];
    // `offsetPx` is the clearance to the badge's NEAR EDGE (the driver adds
    // half the element's own box on top), so this is the whole guarantee —
    // no part of the badge reaches within it, whatever the badge measures.
    expect(GLYPH_STANDOFF_PX).toBeGreaterThanOrEqual(2 * SKETCH_PICK_PX);
    for (const d of [NEAR, FAR]) {
      const { badge: b, anchor } = place(badge, s.plane, sketchCam(d));
      expect(Math.hypot(b.x - anchor.x, b.y - anchor.y)).toBeGreaterThanOrEqual(SKETCH_PICK_PX);
    }
  });

  it("the standoff is PERPENDICULAR to the entity, and a glyph takes the opposite side from its dimension", () => {
    const s = lineSession([
      { id: "fx1", type: "Fixed", entities: ["e1"] },
      { id: "d1", type: "Distance", entities: ["e1"], value: 40 },
    ]);
    const badges = layoutBadges(s);
    const cam = sketchCam(FAR);
    const glyph = place(badges.find((b) => b.id === "fx1")!, s.plane, cam);
    const dim = place(badges.find((b) => b.id === "d1")!, s.plane, cam);

    // e1 runs along plane +x, which this camera puts on screen-right: the
    // offset is purely vertical for both.
    expect(glyph.badge.x).toBeCloseTo(glyph.anchor.x, 4);
    expect(dim.badge.x).toBeCloseTo(dim.anchor.x, 4);
    // Opposite sides of the line — the glyph below (the side the old
    // plane-space bake projected to), the dimension chip above.
    expect(glyph.badge.y - glyph.anchor.y).toBeCloseTo(GLYPH_STANDOFF_PX, 4);
    expect(dim.badge.y - dim.anchor.y).toBeCloseTo(-DIMENSION_STANDOFF_PX, 4);
  });

  it("does not move any badge's plane anchor off its entity", () => {
    const s = lineSession([
      { id: "fx1", type: "Fixed", entities: ["e1"] },
      { id: "d1", type: "Distance", entities: ["e1"], value: 40 },
    ]);
    for (const badge of layoutBadges(s)) expect(badge.at).toEqual(entityAnchor(s.entities[0]));
  });

  // A bare `Point` entity has no axis (`axisFromFor` returns undefined), and
  // the driver only offsets an item that HAS one — so this badge sits on its
  // point, exactly as it did under the plane-space bake.
  it("leaves a Point-anchored glyph badge on its point (no axis to take a direction from)", () => {
    const s: SketchSession = {
      sketchId: "sk",
      plane: planeFor("XY"),
      entities: [{ id: "p1", type: "Point", p0: [5, 5] }],
      constraints: [{ id: "fx1", type: "Fixed", entities: ["p1"] }],
      dof: 0,
      status: "FullyConstrained",
    };
    const badge = layoutBadges(s)[0];
    expect(badge.axisFrom).toBeUndefined();
    expect(badge.at).toEqual({ x: 5, y: 5 });
    const { badge: b, anchor } = place(badge, s.plane, sketchCam(FAR));
    expect(b).toEqual(anchor);
  });
});
