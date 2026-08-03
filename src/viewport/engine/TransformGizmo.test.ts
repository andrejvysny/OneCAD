/*
 * TransformGizmo — handle CLASSIFICATION (WP-B W2).
 *
 * A gizmo that draws correctly but mis-classifies a grab is the worst outcome
 * available here: the user reaches for the Z ring, gets the Z arrow, and the
 * body slides instead of turning. So the tests below fire REAL raycasts at the
 * built scene graph (no mocked raycaster — the arbitration under test IS
 * three.js's nearest-hit ordering) and assert the kind AND the axis.
 *
 * Every classification case fires from the ISO direction — the viewport's own
 * default camera, and a deliberately GENERIC one. Axis-aligned test rays are
 * misleading here: they view one ring edge-on (where it legitimately sweeps
 * across the whole gizmo and wins) and they aim straight at the six points where
 * two rings genuinely cross, so an axis-aligned "failure" would be the test's
 * geometry, not the gizmo's. The edge-on case is pinned separately, as
 * characterisation rather than as an invariant it does not have.
 */
import { describe, it, expect, vi } from "vitest";
import * as THREE from "three";
import { TransformGizmo, type GizmoAxis, type GizmoHit } from "./TransformGizmo";
import { RENDER_ORDER } from "./renderOrder";

function makeGizmo(): { gizmo: TransformGizmo; root: THREE.Object3D; invalidate: () => void } {
  const root = new THREE.Object3D();
  const invalidate = vi.fn();
  const gizmo = new TransformGizmo({ root, invalidate });
  gizmo.setVisible(true);
  return { gizmo, root, invalidate };
}

/** The gizmo's group (its only child of `root`). */
const groupOf = (root: THREE.Object3D): THREE.Group => root.children[0] as THREE.Group;

/** The viewport's default camera direction — generic w.r.t. every handle. */
const ISO = new THREE.Vector3(1, 1, 1).normalize();

/**
 * Fire a ray at a point in the gizmo's LOCAL px-space (scale 1 by default), from
 * far enough back along `from` that nothing is behind the camera.
 */
function castAt(target: THREE.Vector3, from: THREE.Vector3): THREE.Raycaster {
  const dir = target.clone().sub(from).normalize();
  return new THREE.Raycaster(from, dir);
}

/** Fire at `target` from 600px away along the ISO view direction. */
const castIso = (target: THREE.Vector3): THREE.Raycaster =>
  castAt(target, target.clone().addScaledVector(ISO, 600));

const AXES: GizmoAxis[] = ["X", "Y", "Z"];
const DIR: Record<GizmoAxis, THREE.Vector3> = {
  X: new THREE.Vector3(1, 0, 0),
  Y: new THREE.Vector3(0, 1, 0),
  Z: new THREE.Vector3(0, 0, 1),
};

describe("TransformGizmo — scene graph", () => {
  it("builds nine grabbable handles: three arrows, three quads, three rings", () => {
    const { root } = makeGizmo();
    const tagged = groupOf(root).children.filter((c) => c.userData.gizmoKind !== undefined);
    const byKind = (kind: string) => tagged.filter((c) => c.userData.gizmoKind === kind);
    expect(byKind("axis").map((c) => c.userData.gizmoAxis).sort()).toEqual(AXES);
    expect(byKind("plane").map((c) => c.userData.gizmoAxis).sort()).toEqual(AXES);
    expect(byKind("ring").map((c) => c.userData.gizmoAxis).sort()).toEqual(AXES);
  });

  it("starts hidden and never raycasts while hidden", () => {
    const root = new THREE.Object3D();
    const gizmo = new TransformGizmo({ root, invalidate: vi.fn() });
    expect(gizmo.visible).toBe(false);
    expect(gizmo.raycast(castAt(new THREE.Vector3(30, 0, 0), new THREE.Vector3(30, 0, 400)))).toBeNull();
  });

  it("renders as a depth-test-free, unlit overlay on the drag-handle tier", () => {
    const { root } = makeGizmo();
    const visuals = groupOf(root).children.filter(
      (c): c is THREE.Mesh => (c as THREE.Mesh).isMesh && c.userData.gizmoKind === undefined,
    );
    expect(visuals.length).toBeGreaterThan(0);
    for (const m of visuals) {
      const mat = m.material as THREE.MeshBasicMaterial;
      expect(mat.depthTest).toBe(false);
      expect(mat.toneMapped).toBe(false); // overlay rule: never tone-mapped
      expect(m.renderOrder).toBe(RENDER_ORDER.TRANSFORM_GIZMO);
    }
  });
});

describe("TransformGizmo — raycast classification", () => {
  it("classifies each arrow by its own axis", () => {
    const { gizmo } = makeGizmo();
    for (const axis of AXES) {
      // 40px out along the arm — inside the shaft, well short of the rings.
      const target = DIR[axis].clone().multiplyScalar(40);
      expect(gizmo.raycast(castIso(target))).toEqual<GizmoHit>({ kind: "axis", axis });
    }
  });

  it("classifies each plane quad by its NORMAL axis (the Z quad translates in XY)", () => {
    const { gizmo } = makeGizmo();
    for (const axis of AXES) {
      const [a, b] = AXES.filter((x) => x !== axis);
      // The quad's centre: 23.5px out along BOTH of its in-plane axes.
      const target = DIR[a].clone().multiplyScalar(23.5).addScaledVector(DIR[b], 23.5);
      expect(gizmo.raycast(castIso(target))).toEqual<GizmoHit>({ kind: "plane", axis });
    }
  });

  it("classifies each ring by its rotation axis", () => {
    const { gizmo } = makeGizmo();
    for (const axis of AXES) {
      const [a, b] = AXES.filter((x) => x !== axis);
      // 45° round the ring (82·cos45 ≈ 58) — deliberately AWAY from the six
      // points where two rings genuinely cross on the axes.
      const target = DIR[a].clone().multiplyScalar(58).addScaledVector(DIR[b], 58);
      expect(gizmo.raycast(castIso(target))).toEqual<GizmoHit>({ kind: "ring", axis });
    }
  });

  it("misses cleanly in empty space beyond the outermost ring", () => {
    const { gizmo } = makeGizmo();
    expect(gizmo.raycast(castIso(new THREE.Vector3(300, 300, 0)))).toBeNull();
  });

  it("lets an EDGE-ON ring win over the arm it crosses (characterisation)", () => {
    const { gizmo } = makeGizmo();
    // Viewed straight down −Y, the Z ring collapses to a line through the X arm.
    // Nearest-hit hands the grab to the ring, which IS what is under the cursor.
    const target = new THREE.Vector3(40, 0, 0);
    const edgeOn = gizmo.raycast(castAt(target, new THREE.Vector3(40, 400, 0)));
    expect(edgeOn).toEqual<GizmoHit>({ kind: "ring", axis: "Z" });
    // Off that degenerate angle the same point is the arrow again.
    expect(gizmo.raycast(castIso(target))).toEqual<GizmoHit>({ kind: "axis", axis: "X" });
  });

  it("follows setPose: the same world ray hits nothing once the gizmo moves away", () => {
    const { gizmo } = makeGizmo();
    const target = new THREE.Vector3(40, 0, 0);
    expect(gizmo.raycast(castIso(target))?.kind).toBe("axis");
    gizmo.setPose(new THREE.Vector3(0, 0, 5000));
    expect(gizmo.raycast(castIso(target))).toBeNull();
  });

  it("scales the GRAB TARGETS with the gizmo, not just its look", () => {
    const { gizmo } = makeGizmo();
    // 400px out along X misses at scale 1 (the arm ends at 67)…
    const target = new THREE.Vector3(400, 0, 0);
    expect(gizmo.raycast(castIso(target))).toBeNull();
    // …and is inside the shaft once one pixel is worth 10 world units.
    gizmo.setScale(10);
    expect(gizmo.raycast(castIso(target))).toEqual<GizmoHit>({ kind: "axis", axis: "X" });
  });
});

describe("TransformGizmo — highlight + lifetime", () => {
  /** The visible material for one handle, found through its (invisible) hit mesh. */
  function matFor(root: THREE.Object3D, hit: GizmoHit): THREE.MeshBasicMaterial {
    const kids = groupOf(root).children as THREE.Mesh[];
    const idx = kids.findIndex(
      (c) => c.userData.gizmoKind === hit.kind && c.userData.gizmoAxis === hit.axis,
    );
    // Each handle registers its visuals immediately before its hit mesh.
    return kids[idx - 1].material as THREE.MeshBasicMaterial;
  }

  it("tints only the hovered handle, and restores it on unhover", () => {
    const { gizmo, root } = makeGizmo();
    const hovered: GizmoHit = { kind: "ring", axis: "Y" };
    const other: GizmoHit = { kind: "ring", axis: "X" };
    const base = matFor(root, hovered).color.getHex();
    gizmo.setHover(hovered);
    expect(matFor(root, hovered).color.getHex()).not.toBe(base);
    expect(matFor(root, other).color.getHex()).not.toBe(matFor(root, hovered).color.getHex());
    gizmo.setHover(null);
    expect(matFor(root, hovered).color.getHex()).toBe(base);
  });

  it("lets the ACTIVE handle outrank hover, so a drag that leaves the mesh stays lit", () => {
    const { gizmo, root } = makeGizmo();
    const active: GizmoHit = { kind: "axis", axis: "X" };
    const hovered: GizmoHit = { kind: "axis", axis: "Z" };
    gizmo.setActive(active);
    gizmo.setHover(hovered);
    const lit = matFor(root, active).color.getHex();
    expect(matFor(root, hovered).color.getHex()).not.toBe(lit);
    gizmo.setActive(null);
    expect(matFor(root, hovered).color.getHex()).toBe(lit); // hover takes over
  });

  it("drops every highlight when hidden, so a re-show never opens pre-lit", () => {
    const { gizmo, root } = makeGizmo();
    const part: GizmoHit = { kind: "plane", axis: "Z" };
    const base = matFor(root, part).color.getHex();
    gizmo.setHover(part);
    gizmo.setActive(part);
    gizmo.setVisible(false);
    expect(matFor(root, part).color.getHex()).toBe(base);
  });

  it("repaints in place on a theme change (materials are reused, never swapped)", () => {
    const { gizmo, root } = makeGizmo();
    const part: GizmoHit = { kind: "axis", axis: "Y" };
    const mat = matFor(root, part);
    gizmo.refreshColors();
    expect(matFor(root, part)).toBe(mat);
  });

  it("dispose detaches the group and frees every geometry exactly once", () => {
    const { gizmo, root } = makeGizmo();
    const geos = (groupOf(root).children as THREE.Mesh[]).map((m) => m.geometry);
    const spies = geos.map((g) => vi.spyOn(g, "dispose"));
    gizmo.dispose();
    expect(root.children).toHaveLength(0);
    for (const s of spies) expect(s).toHaveBeenCalledTimes(1);
  });
});
