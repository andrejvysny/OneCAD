/*
 * SketchObject material selection — specifically the W0b `referenceLocked`
 * channel (SCHEMA §7.3).
 *
 * Host-face projected geometry gets its OWN material: solid (it is real
 * geometry that bounds regions, unlike dashed construction) and recessive (it
 * is not yours to move). Selection/hover still win, because locked geometry is
 * selectable and snappable.
 *
 * THREE is real (jsdom-safe: Line2/LineMaterial construct without a GL context);
 * no renderer.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { Line2 } from "three/examples/jsm/lines/Line2.js";
import type { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { SketchObject } from "./SketchObject";
import { palette } from "./palette";
import { RENDER_ORDER } from "./renderOrder";
import type { SketchConstraint, SketchEntity, SketchPlane } from "@/ipc/types";

const IDENTITY_PLANE: SketchPlane = {
  kind: "custom",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const seg = (id: string, referenceLocked?: boolean, construction?: boolean): SketchEntity => ({
  id,
  type: "Line",
  p0: [0, 0],
  p1: [10, 0],
  referenceLocked,
  construction,
});

/** Every committed-entity Line2 under `root` — excludes dimension-line
 *  witness segments (tagged `userData.dimLineWitness`), which an AUTHORED
 *  length constraint grows (Track B3: selection alone no longer does), and
 *  selection/hover HALO lines (tagged `userData.selectionHalo`, see
 *  `haloColor`), which are a separate underlay, not the entity's own color. */
function entityLines(root: THREE.Object3D): Line2[] {
  const lines: Line2[] = [];
  root.traverse((o) => {
    if (o instanceof Line2 && !o.userData.dimLineWitness && !o.userData.selectionHalo) lines.push(o);
  });
  return lines;
}

/** The selection/hover halo's color, or undefined when none is drawn. These
 *  tests never select/hover more than one entity at a time, so "the" halo is
 *  unambiguous. */
function haloColor(root: THREE.Object3D): number | undefined {
  let color: number | undefined;
  root.traverse((o) => {
    if (o instanceof Line2 && o.userData.selectionHalo) color = (o.material as LineMaterial).color.getHex();
  });
  return color;
}

/** How many halo lines are currently drawn. */
function haloCount(root: THREE.Object3D): number {
  let n = 0;
  root.traverse((o) => {
    if (o instanceof Line2 && o.userData.selectionHalo) n++;
  });
  return n;
}

/** Entity id → the color its Line2 was drawn with, in scene order. */
function colorsOf(root: THREE.Object3D, ids: string[]): Map<string, number> {
  const lines = entityLines(root);
  expect(lines.length).toBe(ids.length);
  return new Map(lines.map((l, i) => [ids[i], (l.material as LineMaterial).color.getHex()]));
}

function build(entities: SketchEntity[]): { obj: SketchObject; root: THREE.Object3D } {
  const root = new THREE.Object3D();
  const obj = new SketchObject({ sketchRoot: root, invalidate: vi.fn() });
  obj.setSession(IDENTITY_PLANE, entities, "UnderConstrained");
  return { obj, root };
}

describe("SketchObject — referenceLocked material", () => {
  it("draws locked geometry SOLID in its own recessive color", () => {
    const { obj, root } = build([seg("free"), seg("locked", true)]);
    const colors = colorsOf(root, ["free", "locked"]);
    expect(colors.get("locked")).toBe(palette.sketchReference().getHex());
    expect(colors.get("free")).toBe(palette.sketchUnder().getHex());
    expect(colors.get("locked")).not.toBe(colors.get("free"));

    // SOLID, not dashed — the visual contrast with construction is color only.
    expect((entityLines(root)[1].material as LineMaterial).dashed).toBeFalsy();
    obj.dispose();
  });

  it("is distinct from construction, and wins when an entity carries both flags", () => {
    const { obj, root } = build([seg("c", false, true), seg("both", true, true)]);
    const colors = colorsOf(root, ["c", "both"]);
    expect(colors.get("c")).toBe(palette.sketchConstruction().getHex());
    expect(colors.get("both")).toBe(palette.sketchReference().getHex());
    obj.dispose();
  });

  it("still tints on hover and selection (locked geometry is selectable)", () => {
    const { obj, root } = build([seg("locked", true)]);
    obj.setHover(["locked"]);
    // Hover is a COLOR swap at the normal stroke weight, no halo underneath.
    expect(colorsOf(root, ["locked"]).get("locked")).toBe(palette.hover3d().getHex());
    expect(haloCount(root)).toBe(0);
    obj.setHover([]);
    obj.setSelection(["locked"]);
    // Selection keeps the entity's own color and adds a halo underneath it
    // (P1 audit fix: selection used to wipe the semantic color).
    expect(colorsOf(root, ["locked"]).get("locked")).toBe(palette.sketchReference().getHex());
    expect(haloColor(root)).toBe(palette.sketchSelected().getHex());
    obj.dispose();
  });

  it("selection wins over hover, and only draws one halo", () => {
    const { obj, root } = build([seg("locked", true)]);
    obj.setHover(["locked"]);
    obj.setSelection(["locked"]);
    expect(haloCount(root)).toBe(1);
    expect(haloColor(root)).toBe(palette.sketchSelected().getHex());
    expect(colorsOf(root, ["locked"]).get("locked")).toBe(palette.sketchReference().getHex());
    obj.dispose();
  });

  it("draws no halo when neither selected nor hovered", () => {
    const { obj, root } = build([seg("locked", true)]);
    expect(haloCount(root)).toBe(0);
    obj.dispose();
  });
});

/*
 * PER-ENTITY constrained state (SCHEMA §7.4 `entityStates`). The whole-sketch
 * `status` used to color every entity identically; now each entity the solver
 * diagnosed gets its own color, and an entity the solver said NOTHING about
 * falls back to the whole-sketch tint. The fallback is the load-bearing half:
 * an ellipse-bearing sketch reports no map at all, and defaulting an absent key
 * to under-constrained would paint a diagnosis nobody made.
 */
describe("SketchObject — per-entity constrained state (SCHEMA §7.4)", () => {
  it("colors each entity from ITS OWN state, inside one under-constrained sketch", () => {
    const { obj, root } = build([seg("a"), seg("b"), seg("c")]);
    obj.setEntityStates({ a: "fullyConstrained", b: "conflicting", c: "underConstrained" });
    const colors = colorsOf(root, ["a", "b", "c"]);
    expect(colors.get("a")).toBe(palette.sketchFull().getHex());
    expect(colors.get("b")).toBe(palette.sketchConflict().getHex());
    expect(colors.get("c")).toBe(palette.sketchUnder().getHex());
    obj.dispose();
  });

  it("an entity ABSENT from the map falls back to the whole-sketch status color", () => {
    const { obj, root } = build([seg("known"), seg("unknown")]);
    obj.setEntityStates({ known: "fullyConstrained" }); // `unknown` is not a key
    expect(colorsOf(root, ["known", "unknown"]).get("unknown")).toBe(palette.sketchUnder().getHex());

    // …and it TRACKS the whole-sketch status, which is what "fallback" means.
    obj.setSession(IDENTITY_PLANE, [seg("known"), seg("unknown")], "Conflicting");
    expect(colorsOf(root, ["known", "unknown"]).get("unknown")).toBe(palette.sketchConflict().getHex());
    // The diagnosed one is unmoved by the sketch-wide status.
    expect(colorsOf(root, ["known", "unknown"]).get("known")).toBe(palette.sketchFull().getHex());
    obj.dispose();
  });

  it("an EMPTY map is unknown-for-all — every entity keeps the whole-sketch tint", () => {
    const { obj, root } = build([seg("a"), seg("b")]);
    obj.setEntityStates({});
    const colors = colorsOf(root, ["a", "b"]);
    expect(colors.get("a")).toBe(palette.sketchUnder().getHex());
    expect(colors.get("b")).toBe(palette.sketchUnder().getHex());
    obj.dispose();
  });

  it("REPLACES rather than merges — a key the newest map dropped goes back to unknown", () => {
    const { obj, root } = build([seg("a")]);
    obj.setEntityStates({ a: "fullyConstrained" });
    expect(colorsOf(root, ["a"]).get("a")).toBe(palette.sketchFull().getHex());
    obj.setEntityStates({});
    expect(colorsOf(root, ["a"]).get("a")).toBe(palette.sketchUnder().getHex());
    obj.dispose();
  });

  it("keeps the precedence HEAD: hover > angleRef > referenceLocked > construction > state", () => {
    const { obj, root } = build([seg("hov"), seg("ang"), seg("lock", true), seg("con", false, true)]);
    // Every one of them claims fullyConstrained — none may show that color.
    obj.setEntityStates({
      hov: "fullyConstrained",
      ang: "fullyConstrained",
      lock: "fullyConstrained",
      con: "fullyConstrained",
    });
    obj.setHover(["hov"]);
    obj.setAngleReference("ang");
    const colors = colorsOf(root, ["hov", "ang", "lock", "con"]);
    expect(colors.get("hov")).toBe(palette.hover3d().getHex());
    expect(colors.get("ang")).toBe(palette.sketchAngleRef().getHex());
    expect(colors.get("lock")).toBe(palette.sketchReference().getHex());
    expect(colors.get("con")).toBe(palette.sketchConstruction().getHex());
    for (const [, hex] of colors) expect(hex).not.toBe(palette.sketchFull().getHex());
    obj.dispose();
  });

  it("PROJECTED geometry outranks referenceLocked and the solver state (WP-P)", () => {
    // Projected geometry IS `referenceLocked`, so the two colors would be
    // indistinguishable without its own rung — and the whole point of the marker
    // is telling a host-face boundary apart from geometry cut off a body that
    // can still move underneath it.
    const { obj, root } = build([seg("host", true), seg("proj", true)]);
    obj.setEntityStates({ host: "fullyConstrained", proj: "fullyConstrained" });
    obj.setProjectedIds(["proj"]);
    const colors = colorsOf(root, ["host", "proj"]);
    expect(colors.get("proj")).toBe(palette.sketchProjected().getHex());
    expect(colors.get("host")).toBe(palette.sketchReference().getHex());
    expect(colors.get("proj")).not.toBe(colors.get("host"));
    obj.dispose();
  });

  it("hover and the angle reference still outrank PROJECTED", () => {
    const { obj, root } = build([seg("hov", true), seg("ang", true), seg("proj", true)]);
    obj.setProjectedIds(["hov", "ang", "proj"]);
    obj.setHover(["hov"]);
    obj.setAngleReference("ang");
    const colors = colorsOf(root, ["hov", "ang", "proj"]);
    expect(colors.get("hov")).toBe(palette.hover3d().getHex());
    expect(colors.get("ang")).toBe(palette.sketchAngleRef().getHex());
    expect(colors.get("proj")).toBe(palette.sketchProjected().getHex());
    obj.dispose();
  });

  it("REPLACES the projected set, and an equal set rebuilds nothing", () => {
    const root = new THREE.Object3D();
    const invalidate = vi.fn();
    const obj = new SketchObject({ sketchRoot: root, invalidate });
    obj.setSession(IDENTITY_PLANE, [seg("a", true)], "UnderConstrained");
    obj.setProjectedIds(["a"]);
    const built = entityLines(root)[0];
    invalidate.mockClear();

    obj.setProjectedIds(["a"]); // a session republish hands back the same ids
    expect(entityLines(root)[0]).toBe(built);
    expect(invalidate).not.toHaveBeenCalled();

    obj.setProjectedIds([]); // detached — back to plain locked reference geometry
    expect(colorsOf(root, ["a"]).get("a")).toBe(palette.sketchReference().getHex());
    expect(invalidate).toHaveBeenCalled();
    obj.dispose();
  });

  it("selection still HALOS rather than replacing a per-entity color", () => {
    const { obj, root } = build([seg("a")]);
    obj.setEntityStates({ a: "conflicting" });
    obj.setSelection(["a"]);
    expect(colorsOf(root, ["a"]).get("a")).toBe(palette.sketchConflict().getHex());
    expect(haloColor(root)).toBe(palette.sketchSelected().getHex());
    obj.dispose();
  });

  it("rebuilds ONLY when the map actually changed", () => {
    const root = new THREE.Object3D();
    const invalidate = vi.fn();
    const obj = new SketchObject({ sketchRoot: root, invalidate });
    obj.setSession(IDENTITY_PLANE, [seg("a")], "UnderConstrained");

    obj.setEntityStates({ a: "conflicting" });
    const built = entityLines(root)[0];
    invalidate.mockClear();

    // A DIFFERENT object with equal content — an echoed gesture map, or an
    // identity solve. Rebuilding here would tear down every Line2 in the sketch
    // once per write-back for an answer that did not move.
    obj.setEntityStates({ a: "conflicting" });
    expect(entityLines(root)[0]).toBe(built);
    expect(invalidate).not.toHaveBeenCalled();

    obj.setEntityStates({ a: "underConstrained" });
    expect(entityLines(root)[0]).not.toBe(built);
    expect(invalidate).toHaveBeenCalled();
    obj.dispose();
  });

  it("an unrecognized token is treated as UNKNOWN, not as an error (§7.4 reader rule)", () => {
    const { obj, root } = build([seg("a")]);
    // A worker one version ahead may emit a token this build has never seen.
    obj.setEntityStates({ a: "somethingNewer" } as unknown as Record<string, "conflicting">);
    expect(colorsOf(root, ["a"]).get("a")).toBe(palette.sketchUnder().getHex());
    obj.dispose();
  });
});

describe("SketchObject — angle reference highlight + arc preview", () => {
  it("tints the referenced entity in its own color, distinct from every other state", () => {
    const { obj, root } = build([seg("a"), seg("ref", true)]); // "ref" is also referenceLocked
    obj.setAngleReference("ref");
    const colors = colorsOf(root, ["a", "ref"]);
    expect(colors.get("ref")).toBe(palette.sketchAngleRef().getHex());
    expect(colors.get("ref")).not.toBe(palette.sketchReference().getHex());
    obj.dispose();
  });

  it("hover recolors the angle reference; selection halos it without replacing its color", () => {
    const { obj, root } = build([seg("ref")]);
    obj.setAngleReference("ref");
    obj.setHover(["ref"]);
    expect(colorsOf(root, ["ref"]).get("ref")).toBe(palette.hover3d().getHex());
    expect(haloCount(root)).toBe(0);
    obj.setHover([]);
    obj.setSelection(["ref"]);
    expect(colorsOf(root, ["ref"]).get("ref")).toBe(palette.sketchAngleRef().getHex());
    expect(haloColor(root)).toBe(palette.sketchSelected().getHex());
    obj.dispose();
  });

  it("clearing with null drops the tint back to its status color", () => {
    const { obj, root } = build([seg("ref")]);
    obj.setAngleReference("ref");
    obj.setAngleReference(null);
    expect(colorsOf(root, ["ref"]).get("ref")).toBe(palette.sketchUnder().getHex());
    obj.dispose();
  });

  it("draws a dashed arc line for a non-degenerate preview, and none below the length floor", () => {
    const { obj, root } = build([]);
    obj.setAnglePreview({ center: { x: 0, y: 0 }, radius: 10, fromDeg: 0, toDeg: 90 });
    const lines = entityLines(root);
    expect(lines.length).toBe(1);
    expect((lines[0].material as LineMaterial).dashed).toBe(true);
    expect((lines[0].material as LineMaterial).color.getHex()).toBe(palette.sketchAngleRef().getHex());

    obj.setAnglePreview({ center: { x: 0, y: 0 }, radius: 0, fromDeg: 0, toDeg: 90 });
    expect(entityLines(root).length).toBe(0); // a zero-radius preview draws nothing
    obj.dispose();
  });

  it("null clears a previously drawn preview", () => {
    const { obj, root } = build([]);
    obj.setAnglePreview({ center: { x: 0, y: 0 }, radius: 10, fromDeg: 0, toDeg: 45 });
    obj.setAnglePreview(null);
    const lines: Line2[] = [];
    root.traverse((o) => {
      if (o instanceof Line2) lines.push(o);
    });
    expect(lines.length).toBe(0);
    obj.dispose();
  });
});

/** The three marker materials (endpoints/midpoints/centroids), in the order
 *  they're added to `entityGroup` — `selectedPoints` (a 4th `Points`
 *  instance) is excluded, it isn't part of the affordance tier. */
function markerMaterials(root: THREE.Object3D): THREE.PointsMaterial[] {
  const mats: THREE.PointsMaterial[] = [];
  root.traverse((o) => {
    if (o instanceof THREE.Points) mats.push(o.material as THREE.PointsMaterial);
  });
  return mats.slice(0, 3);
}

describe("SketchObject — point affordance (Sketcher UX cleanup, Track B1b)", () => {
  it("starts dim (not full opacity) on a freshly-built sketch", () => {
    const { obj, root } = build([seg("a")]);
    for (const mat of markerMaterials(root)) expect(mat.opacity).toBeLessThan(1);
    obj.dispose();
  });

  it("setPointAffordance({...all true}) brings all three marker materials to full opacity", () => {
    const { obj, root } = build([seg("a")]);
    const dim = markerMaterials(root).map((m) => m.opacity);
    obj.setPointAffordance({ endpoints: true, midpoints: true, centroids: true });
    for (const mat of markerMaterials(root)) expect(mat.opacity).toBe(1);

    obj.setPointAffordance({ endpoints: false, midpoints: false, centroids: false });
    markerMaterials(root).forEach((mat, i) => expect(mat.opacity).toBe(dim[i]));
    obj.dispose();
  });

  it("each tier's flag controls only its own marker material", () => {
    const { obj, root } = build([seg("a")]);
    obj.setPointAffordance({ endpoints: true, midpoints: false, centroids: false });
    const [endpointsMat, midpointsMat, centroidsMat] = markerMaterials(root);
    expect(endpointsMat.opacity).toBe(1);
    expect(midpointsMat.opacity).toBeLessThan(1);
    expect(centroidsMat.opacity).toBeLessThan(1);
    obj.dispose();
  });
});

/*
 * Audit item #1 (A1, S1) — the active sketch must draw OVER a coplanar body
 * face. In sketch mode the camera looks down the plane normal, so the body's
 * volume sits between the eye and the plane and its depth-written opaque faces
 * hid the user's very first stroke completely. Every active-session material is
 * therefore depthTest:false; the ordering that used to come from depth comes
 * entirely from the RENDER_ORDER tiers, which is why this also pins them.
 */

const RECT: SketchEntity[] = [
  { id: "r1", type: "Line", p0: [-10, -10], p1: [10, -10] },
  { id: "r2", type: "Line", p0: [10, -10], p1: [10, 10] },
  { id: "r3", type: "Line", p0: [10, 10], p1: [-10, 10] },
  { id: "r4", type: "Line", p0: [-10, 10], p1: [-10, -10] },
];

/** A fully-populated session: fills, entities, halo, markers, dim lines,
 *  trim ghost and angle arc all present at once. */
function buildRich(): { obj: SketchObject; root: THREE.Object3D } {
  const root = new THREE.Object3D();
  const obj = new SketchObject({ sketchRoot: root, invalidate: vi.fn() });
  const dim: SketchConstraint = { id: "c1", type: "Distance", entities: ["r1"], value: 20 };
  obj.setSession(IDENTITY_PLANE, RECT, "UnderConstrained", [dim]);
  obj.setSelection(["r1"]);
  obj.setSelectedPoints([{ x: -10, y: -10 }]);
  obj.setPreview([{ type: "Line", p0: { x: 0, y: 0 }, p1: { x: 5, y: 5 } }]);
  obj.setTrimGhost({ type: "Line", p0: { x: 0, y: 0 }, p1: { x: 0, y: 5 } });
  obj.setAnglePreview({ center: { x: 0, y: 0 }, radius: 8, fromDeg: 0, toDeg: 90 });
  return { obj, root };
}

/** Every renderable under `root`, paired with the material it draws with. */
function drawables(root: THREE.Object3D): { obj: THREE.Object3D; mat: THREE.Material }[] {
  const out: { obj: THREE.Object3D; mat: THREE.Material }[] = [];
  root.traverse((o) => {
    const m = (o as THREE.Mesh).material;
    if (!m) return;
    for (const one of Array.isArray(m) ? m : [m]) out.push({ obj: o, mat: one });
  });
  return out;
}

/** The tiers the ACTIVE session draws its own content at. The plane tint and
 *  the plane grid are deliberately NOT here — they keep depth testing. */
const ACTIVE_TIERS = new Set<number>([
  RENDER_ORDER.ACTIVE_FILL,
  RENDER_ORDER.SKETCH_CURVES_HALO,
  RENDER_ORDER.SKETCH_CURVES,
  RENDER_ORDER.SKETCH_POINTS,
  RENDER_ORDER.SKETCH_POINTS + 1, // selected-point ring
  RENDER_ORDER.DIM_LINE, // == TRIM_GHOST == ANGLE_ARC_PREVIEW
]);

describe("SketchObject — active-session draw priority (audit item #1)", () => {
  it("draws EVERY active-session material with depthTest off", () => {
    const { obj, root } = buildRich();
    const active = drawables(root).filter((d) => ACTIVE_TIERS.has(d.obj.renderOrder));
    // Guard the guard: if the scene stopped producing these objects the loop
    // below would pass vacuously.
    expect(active.length).toBeGreaterThanOrEqual(8);
    for (const { obj: o, mat } of active) {
      expect(mat.depthTest, `${o.type}@${o.renderOrder} must not depth-test`).toBe(false);
    }
    obj.dispose();
  });

  it("covers every channel the session can draw, one material each", () => {
    const { obj, root } = buildRich();
    const at = (order: number, pred?: (d: { obj: THREE.Object3D }) => boolean) =>
      drawables(root).filter((d) => d.obj.renderOrder === order && (pred?.(d) ?? true));

    expect(at(RENDER_ORDER.ACTIVE_FILL, (d) => d.obj.name === "sketchActiveFill").length).toBe(1);
    expect(at(RENDER_ORDER.SKETCH_CURVES_HALO, (d) => d.obj.userData.selectionHalo === true).length).toBe(1);
    // 4 committed entity lines + 1 rubber-band preview line.
    expect(at(RENDER_ORDER.SKETCH_CURVES).length).toBe(5);
    // endpoints + midpoints + centroids.
    expect(at(RENDER_ORDER.SKETCH_POINTS).length).toBe(3);
    // The selected-point ring shares tier 5 with the annotation channels below,
    // so it is identified by kind rather than by number.
    expect(at(RENDER_ORDER.SKETCH_POINTS + 1, (d) => d.obj instanceof THREE.Points).length).toBe(1);
    expect(at(RENDER_ORDER.DIM_LINE, (d) => d.obj.userData.dimLineWitness === true).length).toBeGreaterThan(0);
    expect(at(RENDER_ORDER.TRIM_GHOST).length).toBeGreaterThan(0);
    obj.dispose();
  });

  it("leaves the plane TINT depth-testing — the surface is not the sketch", () => {
    const { obj, root } = buildRich();
    const tint = drawables(root).filter((d) => d.obj.renderOrder === RENDER_ORDER.SKETCH_TINT);
    expect(tint).toHaveLength(1);
    expect(tint[0].mat.depthTest).toBe(true);
    obj.dispose();
  });
});

/*
 * Audit item #2 (A5) — live closure fills. Loops come from `findClosedLoops`,
 * the SAME detector the centroid markers use, plus every self-closed curve:
 * NOT from the mock kernel, which finds at most one loop and is evicted from
 * production builds. The worker stays the region authority at finish time; this
 * is a preview aid and mints no ids.
 */
describe("SketchObject — live closure fills (audit item #2)", () => {
  const fills = (root: THREE.Object3D): THREE.Object3D[] => {
    const out: THREE.Object3D[] = [];
    root.traverse((o) => {
      if (o.name === "sketchActiveFill") out.push(o);
    });
    return out;
  };
  const fillsVisible = (root: THREE.Object3D): boolean | undefined =>
    root.getObjectByName("sketchFills")?.visible;

  /** Total triangle area of one fill mesh. A ring spliced without re-orienting
   *  its parts folds into a bowtie, whose triangulation cannot come out at the
   *  polygon's true area — so this is what tells a correct splice from a
   *  plausible-looking one. */
  const fillArea = (mesh: THREE.Object3D): number => {
    const geo = (mesh as THREE.Mesh).geometry as THREE.BufferGeometry;
    const pos = geo.getAttribute("position");
    const idx = geo.getIndex()!;
    let area = 0;
    for (let i = 0; i < idx.count; i += 3) {
      const [a, b, c] = [idx.getX(i), idx.getX(i + 1), idx.getX(i + 2)];
      area +=
        Math.abs(
          (pos.getX(b) - pos.getX(a)) * (pos.getY(c) - pos.getY(a)) -
            (pos.getX(c) - pos.getX(a)) * (pos.getY(b) - pos.getY(a)),
        ) / 2;
    }
    return area;
  };

  /** A second rectangle, disjoint from RECT (x ∈ [20, 40]). */
  const RECT2: SketchEntity[] = [
    { id: "q1", type: "Line", p0: [20, -10], p1: [40, -10] },
    { id: "q2", type: "Line", p0: [40, -10], p1: [40, 10] },
    { id: "q3", type: "Line", p0: [40, 10], p1: [20, 10] },
    { id: "q4", type: "Line", p0: [20, 10], p1: [20, -10] },
  ];

  it("fills a closed rectangle", () => {
    const { obj, root } = build(RECT);
    expect(fills(root)).toHaveLength(1);
    expect(fills(root)[0].renderOrder).toBe(RENDER_ORDER.ACTIVE_FILL);
    obj.dispose();
  });

  it("draws NO fill for an open polyline — the closure signal must be honest", () => {
    const { obj, root } = build(RECT.slice(0, 3));
    expect(fills(root)).toHaveLength(0);
    obj.dispose();
  });

  it("appears the moment the closing entity arrives, and goes away again", () => {
    const { obj, root } = build(RECT.slice(0, 3));
    expect(fills(root)).toHaveLength(0);
    obj.setSession(IDENTITY_PLANE, RECT, "UnderConstrained");
    expect(fills(root)).toHaveLength(1);
    obj.setSession(IDENTITY_PLANE, RECT.slice(0, 3), "UnderConstrained");
    expect(fills(root)).toHaveLength(0);
    obj.dispose();
  });

  it("fills a loop whose edges were drawn OUTWARD from shared corners", () => {
    // Head-to-head parts: `findClosedLoops` still walks the ring, but the two
    // reversed edges make a blind concatenation self-intersect. 100 (=10×10)
    // is the only area a correctly-spliced ring can produce.
    const { obj, root } = build([
      { id: "a", type: "Line", p0: [0, 0], p1: [10, 0] },
      { id: "b", type: "Line", p0: [0, 0], p1: [0, 10] },
      { id: "c", type: "Line", p0: [10, 10], p1: [10, 0] },
      { id: "d", type: "Line", p0: [10, 10], p1: [0, 10] },
    ]);
    expect(fills(root)).toHaveLength(1);
    expect(fillArea(fills(root)[0])).toBeCloseTo(100, 2);
    obj.dispose();
  });

  it("fills BOTH of two disjoint rectangles", () => {
    // The regression this pins: a single-loop detector fills neither, so a
    // profile the worker extrudes happily reads as "not closed" on screen.
    const { obj, root } = build([...RECT, ...RECT2]);
    expect(fills(root)).toHaveLength(2);
    obj.dispose();
  });

  it("fills the closed rectangle even with a stray open line in the sketch", () => {
    const stray: SketchEntity = { id: "s1", type: "Line", p0: [30, 30], p1: [40, 30] };
    const { obj, root } = build([...RECT, stray]);
    expect(fills(root)).toHaveLength(1);
    obj.dispose();
  });

  it("fills a lone circle — a self-closed curve has no endpoints to chain", () => {
    const { obj, root } = build([{ id: "c", type: "Circle", center: [0, 0], radius: 5 }]);
    expect(fills(root)).toHaveLength(1);
    obj.dispose();
  });

  it("fills a lone ellipse too", () => {
    const { obj, root } = build([
      { id: "e", type: "Ellipse", center: [0, 0], majorR: 8, minorR: 3, rotation: 0.4 },
    ]);
    expect(fills(root)).toHaveLength(1);
    obj.dispose();
  });

  it("skips CONSTRUCTION loops — they bound no extrudable profile", () => {
    const { obj, root } = build(RECT.map((e) => ({ ...e, construction: true })));
    expect(fills(root)).toHaveLength(0);
    obj.dispose();
  });

  it("SUBTRACTS a nested circle from its enclosing loop — the hole is a hole", () => {
    // Even-odd nesting: the circle is enclosed by one ring, so it is a hole in
    // it, not a second fill. Area is what tells the two apart — v1 painted both
    // rings and covered the hole (400 + 28.3); this paints 400 − 28.3.
    const circle: SketchEntity = { id: "c", type: "Circle", center: [0, 0], radius: 3 };
    const { obj, root } = build([...RECT, circle]);
    expect(fills(root)).toHaveLength(1);
    const expected = 400 - Math.PI * 9;
    expect(fillArea(fills(root)[0])).toBeGreaterThan(expected * 0.99);
    expect(fillArea(fills(root)[0])).toBeLessThan(expected * 1.01);
    obj.dispose();
  });

  it("paints an ISLAND drawn inside that hole — even-odd keeps going", () => {
    // Straight-edged all the way down, so the area is exact rather than the
    // stroke tessellation's inscribed approximation.
    const square = (tag: string, h: number): SketchEntity[] => [
      { id: `${tag}1`, type: "Line", p0: [-h, -h], p1: [h, -h] },
      { id: `${tag}2`, type: "Line", p0: [h, -h], p1: [h, h] },
      { id: `${tag}3`, type: "Line", p0: [h, h], p1: [-h, h] },
      { id: `${tag}4`, type: "Line", p0: [-h, h], p1: [-h, -h] },
    ];
    const { obj, root } = build([...RECT, ...square("hole", 5), ...square("island", 2)]);
    expect(fills(root)).toHaveLength(2);
    const total = fills(root).reduce((n, f) => n + fillArea(f), 0);
    expect(total).toBeCloseTo(400 - 100 + 16, 6);
    obj.dispose();
  });

  it("does not re-triangulate when handed the SAME entity array again", () => {
    const { obj, root } = build(RECT);
    const before = fills(root)[0];
    obj.setSession(IDENTITY_PLANE, RECT, "FullyConstrained");
    // Same object identity ⇒ the loop walk + ear-clip did not run. A solve that
    // moved nothing hands back its input.
    expect(fills(root)[0]).toBe(before);
    obj.dispose();
  });

  it("HIDES the fills on a transient (drag-frequency) update, rebuilding at gesture end", () => {
    const { obj, root } = build(RECT);
    const before = fills(root)[0];
    expect(fillsVisible(root)).toBe(true);

    // A drag frame: geometry moved, so a new array arrives every rAF. The fills
    // must neither re-triangulate nor sit stale under the moved strokes.
    const moved = RECT.map((e) => ({ ...e, p0: [e.p0![0] + 1, e.p0![1]], p1: [e.p1![0] + 1, e.p1![1]] }) as SketchEntity);
    obj.setSession(IDENTITY_PLANE, moved, "UnderConstrained", undefined, { transient: true });
    expect(fills(root)).toHaveLength(1);
    expect(fills(root)[0]).toBe(before);
    expect(fillsVisible(root)).toBe(false);

    // Gesture end (finishDrag) — non-transient, so they come back rebuilt.
    const committed = moved.map((e) => ({ ...e }));
    obj.setSession(IDENTITY_PLANE, committed, "UnderConstrained");
    expect(fills(root)).toHaveLength(1);
    expect(fills(root)[0]).not.toBe(before);
    expect(fillsVisible(root)).toBe(true);
    obj.dispose();
  });
});
