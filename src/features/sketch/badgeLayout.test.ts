import { describe, it, expect } from "vitest";
import { layoutBadges, entityAnchor, entityPointCoord } from "./badgeLayout";
import { planeFor } from "@/ipc/mockSketch";
import type { SketchSession } from "@/ipc/types";

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

  // GLYPH STANDOFF (adversarial-review M3 follow-up): a glyph's `at` is now
  // nudged GLYPH_STANDOFF_MM (10mm) perpendicular off the raw entity point —
  // e1 is p0=[0,0],p1=[40,0] (horizontal), so the perpendicular is straight
  // -y, off the midpoint {x:20,y:0}. See badgeLayout.ts's file header + the
  // dedicated standoff describe block below for the full derivation.
  it("places a Horizontal glyph off the line midpoint (standoff)", () => {
    const h = badges.find((b) => b.id === "k1")!;
    expect(h.glyph).toBe("H");
    expect(h.at).toEqual({ x: 20, y: -10 });
    expect(h.editable).toBe(false);
  });

  it("places a Coincident dot off the shared point (standoff)", () => {
    const c = badges.find((b) => b.id === "k2")!;
    expect(c.glyph).toBe("•");
    // k2 is anchored at e2's Start=[40,0]; axisFrom is e2's OTHER endpoint
    // [40,20] (see the dedicated test below), so the standoff runs -x from it.
    expect(c.at).toEqual({ x: 30, y: 0 });
  });

  it("marks dimensional constraints editable with a value glyph", () => {
    const d = badges.find((b) => b.id === "k3")!;
    expect(d.editable).toBe(true);
    expect(d.value).toBe(40);
    expect(d.glyph).toBe("40");
    const r = badges.find((b) => b.id === "k4")!;
    expect(r.editable).toBe(true);
    expect(r.at).toEqual({ x: 10, y: 10 });
  });

  it("returns [] for a null session", () => {
    expect(layoutBadges(null)).toEqual([]);
  });

  it("gives a line-anchored badge its endpoint as the leader axisFrom", () => {
    const h = badges.find((b) => b.id === "k1")!;
    // e1 is p0=[0,0], p1=[40,0] — the leader runs from p0 to the midpoint.
    // GLYPH STANDOFF: axisFrom is nudged by the SAME delta as `at` (see
    // badgeLayout.ts's glyphStandoff), so it reads p0 + that delta (0,-10),
    // not raw p0 — the leader's own (at − axisFrom) vector is preserved
    // exactly, which is the whole point (no drift into the driver's own
    // separate screen-space nudge).
    expect(h.axisFrom).toEqual({ x: 0, y: -10 });
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
    // Both `at` and `axisFrom` carry the SAME standoff delta (-10,0 here), so
    // the leader's own (at − axisFrom) vector — and therefore the direction
    // the overlay driver's own nudge rotates — is unchanged from before.
    expect(c.axisFrom).toEqual({ x: 30, y: 20 });
    expect(c.at).toEqual({ x: 30, y: 0 });
  });

  // Standoff follow-up: k1 (Horizontal, a GLYPH) and k3 (Distance, DIMENSIONAL)
  // used to co-anchor exactly at e1's midpoint and stagger via offsetIndex —
  // now only the glyph is nudged off that point, so they no longer share an
  // anchor at all and each is a standalone offsetIndex-0 group of one. Two
  // GLYPHS on the same entity still co-anchor (identical standoff applies to
  // both) and still stagger — see "layoutBadges — offsetIndex stagger" below,
  // whose zz/aa pair (Horizontal + Fixed, same entity) is exactly that case.
  it("a glyph (k1) and a co-located dimensional chip (k3) no longer share an anchor", () => {
    const h = badges.find((b) => b.id === "k1")!;
    const d = badges.find((b) => b.id === "k3")!;
    expect(h.at).not.toEqual(d.at);
    expect(d.at).toEqual({ x: 20, y: 0 }); // dimensional stays AT the raw midpoint
    expect(h.offsetIndex).toBe(0);
    expect(d.offsetIndex).toBe(0);
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
 * Adversarial-review M3 follow-up: e2e/acceptance.spec.ts:161 failed because a
 * `Fixed` glyph badge sat exactly at a line's midpoint and out-ranked the line
 * itself in `elementsFromPoint`, eating a select-tool click meant for the
 * curve. `badgeLayout.ts` is camera-agnostic (mm only), so the ≥16px CSS claim
 * is checked against a PINNED conversion rather than a live projection: a
 * one-off probe against `e2e/acceptance.spec.ts`'s own default-zoom camera
 * (`enterSketchViaPlanePicker`'s framed rectangle, `planePointToClient`
 * before/after a 1mm plane-space nudge) measured ~4.74 px/mm. Re-probe and
 * update this ratio if that framing ever changes.
 *
 * The bar this bake alone must clear is NOT the bare 16px target — it must
 * clear 16px PLUS `HtmlOverlayDriver`'s own worst-case counter-nudge (see
 * badgeLayout.ts's `GLYPH_STANDOFF_MM` doc for the measured case where that
 * nudge ran almost exactly opposite this bake and cancelled nearly all of
 * it). That nudge is a fixed CSS magnitude regardless of camera view:
 * `BADGE_OFFSET_PX` (10, ConstraintBadgeLayer.tsx) + half the 20px badge box
 * (10) = 20px.
 */
describe("layoutBadges — glyph standoff clears the entity's own click corridor (M3 follow-up)", () => {
  const PX_PER_MM_AT_DEFAULT_ZOOM = 4.74;
  const SELECT_CLICK_MIN_PX = 16;
  const DRIVER_OWN_WORST_CASE_NUDGE_PX = 20;

  it("a line's glyph badge center clears the click corridor even against the driver's own worst-case counter-nudge", () => {
    const s: SketchSession = {
      sketchId: "sk",
      plane: planeFor("XY"),
      entities: [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
      constraints: [{ id: "fx1", type: "Fixed", entities: ["e1"] }],
      dof: 0,
      status: "FullyConstrained",
    };
    const badge = layoutBadges(s)[0];
    const mid = entityAnchor(s.entities[0])!;
    const mmOff = Math.hypot(badge.at.x - mid.x, badge.at.y - mid.y);
    expect(mmOff * PX_PER_MM_AT_DEFAULT_ZOOM).toBeGreaterThanOrEqual(
      SELECT_CLICK_MIN_PX + DRIVER_OWN_WORST_CASE_NUDGE_PX,
    );
  });

  it("does not move a dimensional (editable) badge off its raw anchor", () => {
    const s: SketchSession = {
      sketchId: "sk",
      plane: planeFor("XY"),
      entities: [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
      constraints: [{ id: "d1", type: "Distance", entities: ["e1"], value: 40 }],
      dof: 0,
      status: "FullyConstrained",
    };
    const badge = layoutBadges(s)[0];
    expect(badge.editable).toBe(true);
    expect(badge.at).toEqual(entityAnchor(s.entities[0]));
  });

  // A bare `Point` entity's axisFromFor is undefined (no line/center to take a
  // direction from) — glyphStandoff leaves `at` unmoved rather than guessing;
  // the driver's own screen-space fallback still applies on top of this case.
  it("leaves a Point-anchored glyph badge unmoved (no axis to take a direction from)", () => {
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
  });
});
