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

/** Entity id → the color its Line2 was drawn with, in scene order. */
function colorsOf(root: THREE.Object3D, ids: string[]): Map<string, number> {
  const lines: Line2[] = [];
  root.traverse((o) => {
    if (o instanceof Line2) lines.push(o);
  });
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
    const lines: Line2[] = [];
    root.traverse((o) => {
      if (o instanceof Line2) lines.push(o);
    });
    expect((lines[1].material as LineMaterial).dashed).toBeFalsy();
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
