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
import type { SketchEntity, SketchPlane } from "@/ipc/types";

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
 *  length constraint grows (Track B3: selection alone no longer does) and
 *  are not what these tests are counting. */
function entityLines(root: THREE.Object3D): Line2[] {
  const lines: Line2[] = [];
  root.traverse((o) => {
    if (o instanceof Line2 && !o.userData.dimLineWitness) lines.push(o);
  });
  return lines;
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
    expect(colorsOf(root, ["locked"]).get("locked")).toBe(palette.hover3d().getHex());
    obj.setSelection(["locked"]);
    expect(colorsOf(root, ["locked"]).get("locked")).toBe(palette.sketchSelected().getHex());
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

  it("selection and hover still win over the angle reference", () => {
    const { obj, root } = build([seg("ref")]);
    obj.setAngleReference("ref");
    obj.setHover(["ref"]);
    expect(colorsOf(root, ["ref"]).get("ref")).toBe(palette.hover3d().getHex());
    obj.setHover([]);
    obj.setSelection(["ref"]);
    expect(colorsOf(root, ["ref"]).get("ref")).toBe(palette.sketchSelected().getHex());
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

  it("setPointAffordance(true) brings all three marker materials to full opacity", () => {
    const { obj, root } = build([seg("a")]);
    const dim = markerMaterials(root).map((m) => m.opacity);
    obj.setPointAffordance(true);
    for (const mat of markerMaterials(root)) expect(mat.opacity).toBe(1);

    obj.setPointAffordance(false);
    markerMaterials(root).forEach((mat, i) => expect(mat.opacity).toBe(dim[i]));
    obj.dispose();
  });
});
