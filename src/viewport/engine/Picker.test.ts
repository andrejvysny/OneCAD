/*
 * Picker pure helpers: screen→world line threshold, edge-vs-face preference,
 * and intersection → PickHit resolution through the registry (fake intersections).
 */
import { describe, it, expect } from "vitest";
import * as THREE from "three";
import {
  linePickThreshold,
  choosePreferredHit,
  secondaryHitWins,
  resolvePick,
  pickKey,
} from "./Picker";
import { buildBodyObjects, type MeshEntry } from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { makeBoxMesh, type FaceColor } from "@/ipc/mockMeshes";

function boxEntry(): MeshEntry {
  return buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1);
}

function fakeFaceHit(bodyId: string, faceIndex: number): THREE.Intersection {
  return {
    distance: 10,
    point: new THREE.Vector3(40, 0, 0),
    object: Object.assign(new THREE.Object3D(), { userData: { bodyId, kind: "face" } }),
    faceIndex,
    face: { normal: new THREE.Vector3(1, 0, 0) } as unknown as THREE.Face,
  } as unknown as THREE.Intersection;
}

function fakeEdgeHit(bodyId: string, vertexIndex: number, distance = 10): THREE.Intersection {
  return {
    distance,
    point: new THREE.Vector3(40, 30, 15),
    object: Object.assign(new THREE.Object3D(), { userData: { bodyId, kind: "edge" } }),
    index: vertexIndex,
  } as unknown as THREE.Intersection;
}

describe("linePickThreshold", () => {
  it("scales linearly with focus distance (perspective)", () => {
    const cam = new THREE.PerspectiveCamera(76, 1, 0.1, 1000);
    const near = linePickThreshold(cam, 800, 100, 6);
    const far = linePickThreshold(cam, 800, 200, 6);
    expect(near).toBeGreaterThan(0);
    expect(far).toBeCloseTo(near * 2, 5);
  });

  it("uses the frustum height for an orthographic camera", () => {
    const cam = new THREE.OrthographicCamera(-100, 100, 50, -50, 0.1, 1000); // height 100
    // 6px of 600px viewport over a 100-unit frustum = 1 world unit.
    expect(linePickThreshold(cam, 600, 260, 6)).toBeCloseTo(1, 5);
  });
});

describe("choosePreferredHit — edge wins within tolerance, loses when occluded", () => {
  const face = fakeFaceHit("body1", 0); // distance 10
  it("prefers an edge at (or within bias of) the face distance", () => {
    const edge = fakeEdgeHit("body1", 0, 10.1);
    expect(choosePreferredHit(face, edge, 0.5)?.kind).toBe("edge");
  });
  it("keeps the face when the edge is much farther (occluded)", () => {
    const edge = fakeEdgeHit("body1", 0, 40);
    expect(choosePreferredHit(face, edge, 0.5)?.kind).toBe("face");
  });
  it("edge-only and face-only cases", () => {
    expect(choosePreferredHit(null, fakeEdgeHit("body1", 0), 0.5)?.kind).toBe("edge");
    expect(choosePreferredHit(face, null, 0.5)?.kind).toBe("face");
    expect(choosePreferredHit(null, null, 0.5)).toBeNull();
  });
});

describe("secondaryHitWins — sketch/body depth arbitration", () => {
  it("lets a numerically coplanar profile win over its host face", () => {
    expect(secondaryHitWins(100, 100.0005)).toBe(true);
  });

  it("does not select a profile through an occluding body", () => {
    expect(secondaryHitWins(100, 100.01)).toBe(false);
  });

  it("lets a profile in front of a body win", () => {
    expect(secondaryHitWins(100, 99)).toBe(true);
  });
});

describe("resolvePick — intersection → PickHit via the registry", () => {
  const entry = boxEntry();
  const lookup = (id: string) => (id === "body1" ? entry : undefined);

  it("maps a face triangle index to its TopoKey + world anchor", () => {
    const hit = resolvePick(fakeFaceHit("body1", 0), "face", lookup);
    expect(hit).not.toBeNull();
    expect(hit!.kind).toBe("face");
    expect(hit!.topoKey).toBe("f:0");
    expect(hit!.elementId).toBeUndefined(); // pure TopoKeys (no IDS_HAVE_ELEMENTIDS)
    expect(hit!.distance).toBe(10);
    expect(hit!.worldPos.x).toBe(40);
    expect(hit!.surfaceHint?.normal).toEqual([1, 0, 0]);
  });

  it("maps triangle 11 (last) to face f:5", () => {
    expect(resolvePick(fakeFaceHit("body1", 11), "face", lookup)!.topoKey).toBe("f:5");
  });

  it("maps an edge segment (vertexIndex>>1) to its edge TopoKey", () => {
    expect(resolvePick(fakeEdgeHit("body1", 0), "edge", lookup)!.topoKey).toBe("e:0");
    expect(resolvePick(fakeEdgeHit("body1", 10), "edge", lookup)!.topoKey).toBe("e:5");
  });

  it("returns null for an unknown body or missing index", () => {
    expect(resolvePick(fakeFaceHit("ghost", 0), "face", lookup)).toBeNull();
  });
});

/*
 * The de-index guarantee, end to end through a REAL raycast.
 *
 * A colored body's geometry loses its index (faceColors.ts), and three's
 * `faceIndex` is computed differently for indexed and non-indexed geometry.
 * Both paths report the TRIANGLE ordinal — which is the only reason
 * `TopoIndex.ordinalOf`'s FACE_RANGES binary search keeps working — but that is
 * an assumption about three, so it gets raycast rather than reasoned about.
 */
describe("picking survives the FACE_COLORS de-index", () => {
  const RED: FaceColor = [214, 74, 62, 255];
  const build = (mesh: ArrayBuffer) => buildBodyObjects(parseMeshPayload(mesh), "body1", 1);

  /** Raycast the entry's real geometry and resolve the hit like the Picker does. */
  function pickAlong(entry: MeshEntry, from: THREE.Vector3, dir: THREE.Vector3): string | null {
    const mesh = new THREE.Mesh(
      entry.geometry,
      new THREE.MeshBasicMaterial({ side: THREE.DoubleSide }),
    );
    mesh.userData = { bodyId: "body1", kind: "face" };
    const hit = new THREE.Raycaster(from, dir.normalize()).intersectObject(mesh, false)[0];
    return hit ? (resolvePick(hit, "face", () => entry)?.topoKey ?? null) : null;
  }

  it("resolves the same face ids on de-indexed geometry as on indexed", () => {
    const plain = build(makeBoxMesh());
    const colored = build(makeBoxMesh(80, 60, 30, 0, [0, 0, 0], [RED, null, null, null, RED, null]));
    expect(colored.hasVertexColors).toBe(true);
    expect(colored.geometry.getIndex()).toBeNull();
    expect(plain.geometry.getIndex()).not.toBeNull();

    const rays: [THREE.Vector3, THREE.Vector3][] = [
      [new THREE.Vector3(200, 0, 0), new THREE.Vector3(-1, 0, 0)], // → +X face
      [new THREE.Vector3(-200, 0, 0), new THREE.Vector3(1, 0, 0)], // → −X face
      [new THREE.Vector3(0, 200, 0), new THREE.Vector3(0, -1, 0)], // → +Y face
      [new THREE.Vector3(0, 0, 200), new THREE.Vector3(0, 0, -1)], // → +Z face
      [new THREE.Vector3(0, 0, -200), new THREE.Vector3(0, 0, 1)], // → −Z face
    ];
    for (const [from, dir] of rays) {
      const expected = pickAlong(plain, from.clone(), dir.clone());
      expect(expected).not.toBeNull(); // guard: the ray really hits the box
      expect(pickAlong(colored, from.clone(), dir.clone())).toBe(expected);
    }
    // …and the ids are the ones the face table says, not just "equal to each other".
    expect(pickAlong(colored, new THREE.Vector3(200, 0, 0), new THREE.Vector3(-1, 0, 0))).toBe("f:0");
    expect(pickAlong(colored, new THREE.Vector3(0, 0, 200), new THREE.Vector3(0, 0, -1))).toBe("f:4");

    plain.dispose();
    colored.dispose();
  });
});

describe("pickKey", () => {
  it("is stable for the same element and null-safe", () => {
    const entry = boxEntry();
    const hit = resolvePick(fakeFaceHit("body1", 0), "face", () => entry)!;
    expect(pickKey(hit)).toBe("body1/face/f:0");
    expect(pickKey(null)).toBeNull();
  });
});
