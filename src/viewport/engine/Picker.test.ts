/*
 * Picker pure helpers: screen→world line threshold, the fat-line (Line2) pick
 * radius, edge-vs-face preference, and intersection → PickHit resolution through
 * the registry (fake intersections) — plus one end-to-end raycast against a real
 * LineSegments2, which is the only way to prove what `faceIndex` actually means.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import * as THREE from "three";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import {
  Picker,
  linePickThreshold,
  line2PickThreshold,
  choosePreferredHit,
  secondaryHitWins,
  resolvePick,
  pickKey,
} from "./Picker";
import { BODY_EDGE_WIDTH } from "./bodyMaterials";
import {
  buildBodyObjects,
  swap,
  disposeAll,
  __resetRegistryForTests,
  type MeshEntry,
} from "../mesh/meshRegistry";
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

/**
 * A LineSegments2 intersection: `faceIndex` IS the segment ordinal (one instance
 * per segment), there is no `index`, and `point` lies on the RAY while
 * `pointOnLine` lies on the segment.
 */
function fakeEdgeHit(bodyId: string, segmentOrdinal: number, distance = 10): THREE.Intersection {
  return {
    distance,
    point: new THREE.Vector3(41, 31, 15), // on the ray, off the geometry
    pointOnLine: new THREE.Vector3(40, 30, 15), // on the segment
    object: Object.assign(new THREE.Object3D(), { userData: { bodyId, kind: "edge" } }),
    faceIndex: segmentOrdinal,
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

/*
 * LineSegments2 tests `dist < (linewidth + threshold) / 2` in DEVICE px, so the
 * threshold has to cancel the drawn width out — otherwise the pick tolerance
 * would silently change whenever the edge weight or the display's dpr did.
 */
describe("line2PickThreshold", () => {
  /** What three will actually use as the hit radius, in device px. */
  const radius = (dpr: number, width: number, px?: number) =>
    (width + line2PickThreshold(dpr, width, px)) / 2;

  it("yields a hit radius of exactly `px` CSS pixels, whatever the line width", () => {
    expect(radius(1, 1.5, 6)).toBe(6); // 6 CSS px at dpr 1 = 6 device px
    expect(radius(1, 4, 6)).toBe(6); // a fatter line does not pick wider
    expect(radius(2, 3, 6)).toBe(12); // 6 CSS px at dpr 2 = 12 device px
  });

  it("clamps at zero — a line drawn wider than the tolerance picks at its own width", () => {
    expect(line2PickThreshold(1, 40, 6)).toBe(0);
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

  /*
   * NO shift. A plain LineSegments reported a VERTEX index (2 per segment); an
   * instanced LineSegments2 reports the segment ordinal directly as `faceIndex`.
   * Shifting it here would bind every edge pick to the wrong edge.
   */
  it("maps an edge segment ordinal straight to its edge TopoKey", () => {
    expect(resolvePick(fakeEdgeHit("body1", 0), "edge", lookup)!.topoKey).toBe("e:0");
    expect(resolvePick(fakeEdgeHit("body1", 5), "edge", lookup)!.topoKey).toBe("e:5");
    expect(resolvePick(fakeEdgeHit("body1", 11), "edge", lookup)!.topoKey).toBe("e:11");
  });

  it("anchors an edge pick on the SEGMENT, not on the ray", () => {
    // `point` is where the ray came closest; only `pointOnLine` is on the edge,
    // and the anchor is what a later AcquireElementIds promotion resolves with.
    const hit = resolvePick(fakeEdgeHit("body1", 0), "edge", lookup)!;
    expect([hit.worldPos.x, hit.worldPos.y, hit.worldPos.z]).toEqual([40, 30, 15]);
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

/*
 * The fat-line edge path, end to end through a REAL raycast.
 *
 * Two things can only be proven here. (1) `faceIndex` really is the segment
 * ordinal — everything above takes that on faith from a fabricated hit.
 * (2) LineSegments2's screen-space raycast returns NOTHING when
 * `material.resolution` is still (0,0), silently and without error. That is the
 * state of every body before its first rendered frame, which is why the Picker
 * flushes the resolution itself instead of trusting `onBeforeRender`.
 */
describe("Picker → real LineSegments2 raycast", () => {
  const VIEW = { w: 800, h: 600 };
  const DPR = 2;

  afterEach(() => {
    disposeAll();
    __resetRegistryForTests();
  });

  /** Camera whose center of view is `target`, `distance` away along `dir`. */
  function cameraAt(target: THREE.Vector3, dir: THREE.Vector3, distance: number): THREE.Camera {
    const cam = new THREE.PerspectiveCamera(50, VIEW.w / VIEW.h, 0.1, 5000);
    cam.up.set(0, 0, 1); // engine invariant: world is Z-up
    cam.position.copy(target).addScaledVector(dir.normalize(), distance);
    cam.lookAt(target);
    cam.updateMatrixWorld(true); // also refreshes matrixWorldInverse
    return cam;
  }

  function harness(resolution: { w: number; h: number }) {
    const entry = boxEntry();
    swap("body1", entry);
    const material = new LineMaterial({ linewidth: BODY_EDGE_WIDTH });
    const line = new LineSegments2(entry.edgeGeometry!, material);
    line.userData = { bodyId: "body1", kind: "edge" };
    line.updateMatrixWorld(true);
    const root = new THREE.Group();
    root.add(line);

    const canvas = document.createElement("canvas");
    canvas.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: VIEW.w, height: VIEW.h }) as DOMRect;

    // e:10 is the vertical edge at the (+x,+y) corner — [40,30,-15]..[40,30,15].
    const target = new THREE.Vector3(40, 30, 0);
    const camera = cameraAt(target, new THREE.Vector3(1, 1, 0.35), 400);

    const picker = new Picker({
      canvas,
      getCamera: () => camera,
      getRoot: () => root,
      getViewportHeight: () => VIEW.h,
      getFocusDistance: () => 400,
      getResolution: () => resolution,
      invalidate: vi.fn(),
      isActive: () => true,
      onHover: vi.fn(),
      onPick: vi.fn(),
    });
    // Pointer at the canvas center ⇒ NDC (0,0) ⇒ the ray through `target`.
    return { picker, material, probe: () => picker.probe(VIEW.w / 2, VIEW.h / 2) };
  }

  it("resolves the edge under the pointer to its own TopoKey", () => {
    const { picker, probe } = harness({ w: VIEW.w * DPR, h: VIEW.h * DPR });
    const hit = probe();
    expect(hit).not.toBeNull();
    expect(hit!.kind).toBe("edge");
    expect(hit!.topoKey).toBe("e:10");
    expect(hit!.bodyId).toBe("body1");
    // The anchor sits ON the edge: x/y pinned to the corner it runs along.
    expect(hit!.worldPos.x).toBeCloseTo(40, 3);
    expect(hit!.worldPos.y).toBeCloseTo(30, 3);
    picker.dispose();
  });

  it("flushes the drawing-buffer resolution into the shared edge material", () => {
    const { picker, material, probe } = harness({ w: VIEW.w * DPR, h: VIEW.h * DPR });
    expect(material.resolution.x).toBe(0); // nothing has rendered
    probe();
    expect([material.resolution.x, material.resolution.y]).toEqual([1600, 1200]);
    picker.dispose();
  });

  it("would find NOTHING at resolution (0,0) — the reason the flush exists", () => {
    // Non-vacuity guard for the test above: neuter the flush (report zeros) and
    // the identical pick goes silently null.
    const { picker, probe } = harness({ w: 0, h: 0 });
    expect(probe()).toBeNull();
    picker.dispose();
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
