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
  pixelWorldSize,
  edgeDepthSlack,
  EDGE_DEPTH_SLACK_PX,
  choosePreferredHit,
  firstUnclippedHit,
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
import { encodeMesh1, makeBoxMesh, type FaceColor } from "@/ipc/mockMeshes";

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

  it("classifies ids individually when an ElementId-labelled mesh still contains TopoKeys", () => {
    const mixed = buildBodyObjects(
      parseMeshPayload(
        encodeMesh1({
          positions: [0, 0, 0, 1, 0, 0, 0, 1, 0, 2, 0, 0, 3, 0, 0, 2, 1, 0],
          faces: [
            { triangles: [[0, 1, 2]], id: "f:0" },
            { triangles: [[3, 4, 5]], id: "el_face" },
          ],
          idsHaveElementIds: true,
        }),
      ),
      "mixed",
      1,
    );
    const mixedLookup = (id: string) => (id === "mixed" ? mixed : undefined);

    expect(resolvePick(fakeFaceHit("mixed", 0), "face", mixedLookup)).toMatchObject({
      topoKey: "f:0",
      elementId: undefined,
    });
    expect(resolvePick(fakeFaceHit("mixed", 1), "face", mixedLookup)).toMatchObject({
      topoKey: "el_face",
      elementId: "el_face",
    });

    mixed.dispose();
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

  it("enumeration keeps the existing edge-preferred probe first", () => {
    const { picker, probe } = harness({ w: VIEW.w * DPR, h: VIEW.h * DPR });
    const preferred = probe();
    const candidates = picker.probeCandidates(VIEW.w / 2, VIEW.h / 2);
    expect(candidates[0]).toMatchObject({
      bodyId: preferred?.bodyId,
      kind: preferred?.kind,
      topoKey: preferred?.topoKey,
      meshRev: 1,
      entryIdentity: { bodyId: "body1", meshRev: 1 },
    });
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

describe("firstUnclippedHit — section view drops what the cut removed", () => {
  const near = { distance: 1, point: new THREE.Vector3(0, 0, 10) } as THREE.Intersection;
  const far = { distance: 2, point: new THREE.Vector3(0, 0, -10) } as THREE.Intersection;
  // Keeps z <= 0 (the unflipped XY cut), i.e. `near` above is cut away.
  const planes = [new THREE.Plane(new THREE.Vector3(0, 0, -1), 0)];

  it("returns [0] verbatim with no planes — the pre-section behavior", () => {
    expect(firstUnclippedHit([near, far], null)).toBe(near);
    expect(firstUnclippedHit([near, far], [])).toBe(near);
    expect(firstUnclippedHit([], planes)).toBeNull();
  });

  it("skips the cut-away hit and takes the first SURVIVOR", () => {
    expect(firstUnclippedHit([near, far], planes)).toBe(far);
  });

  it("is null when the cut removed everything under the pointer", () => {
    expect(firstUnclippedHit([near], planes)).toBeNull();
  });

  it("tests a fat line at its point ON the segment, not the one on the ray", () => {
    // pointOnLine is kept (z = -1); point is cut away (z = +1). Taking `point`
    // would drop a visible edge lying just under the plane.
    const edge = {
      distance: 1,
      point: new THREE.Vector3(0, 0, 1),
      pointOnLine: new THREE.Vector3(0, 0, -1),
    } as unknown as THREE.Intersection;
    expect(firstUnclippedHit([edge], planes)).toBe(edge);
  });
});

describe("Picker under a section cut — clicking through selects the interior", () => {
  const VIEW = { w: 800, h: 600 };

  afterEach(() => {
    disposeAll();
    __resetRegistryForTests();
  });

  /**
   * The mock box (80x60x30 at the origin) seen from above and slightly in front,
   * so the ray through the canvas centre crosses the TOP face (f:4, z=+15) and
   * then the BOTTOM one (f:5, z=-15). `up` stays world +Z (engine invariant) and
   * the camera is tilted off the pole for the same reason CameraRig clamps pitch.
   */
  function harness(planes: THREE.Plane[] | null) {
    const entry = boxEntry();
    swap("body1", entry);
    const mesh = new THREE.Mesh(
      entry.geometry,
      // DoubleSide, exactly like the real body face material: the bottom face is
      // hit from behind, and a FrontSide material would hide the survivor.
      new THREE.MeshStandardMaterial({ side: THREE.DoubleSide }),
    );
    mesh.userData = { bodyId: "body1", kind: "face" };
    mesh.updateMatrixWorld(true);
    const root = new THREE.Group();
    root.add(mesh);

    const camera = new THREE.PerspectiveCamera(50, VIEW.w / VIEW.h, 0.1, 5000);
    camera.up.set(0, 0, 1);
    camera.position.set(0, -20, 400);
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld(true);

    const canvas = document.createElement("canvas");
    canvas.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: VIEW.w, height: VIEW.h }) as DOMRect;

    const picker = new Picker({
      canvas,
      getCamera: () => camera,
      getRoot: () => root,
      getViewportHeight: () => VIEW.h,
      getFocusDistance: () => 400,
      getResolution: () => ({ w: VIEW.w, h: VIEW.h }),
      invalidate: vi.fn(),
      getClippingPlanes: () => planes,
      isActive: () => true,
      onHover: vi.fn(),
      onPick: vi.fn(),
    });
    return { picker, probe: () => picker.probe(VIEW.w / 2, VIEW.h / 2) };
  }

  it("picks the nearest face when there is no cut", () => {
    const { picker, probe } = harness(null);
    expect(probe()?.topoKey).toBe("f:4"); // the top face, +Z
    picker.dispose();
  });

  it("picks the face the cut EXPOSED, never the one it removed", () => {
    // The unflipped XY cut at 0 keeps z <= 0, so the top face is not on screen.
    const { picker, probe } = harness([new THREE.Plane(new THREE.Vector3(0, 0, -1), 0)]);
    const hit = probe();
    expect(hit?.topoKey).not.toBe("f:4");
    expect(hit?.topoKey).toBe("f:5"); // the bottom face, now visible through the cut
    expect(hit?.worldPos.z).toBeCloseTo(-15, 3);
    picker.dispose();
  });

  it("finds nothing when the cut removed the whole body under the pointer", () => {
    // Keeps z >= 100 — the box is entirely on the discarded side.
    const { picker, probe } = harness([new THREE.Plane(new THREE.Vector3(0, 0, 1), -100)]);
    expect(probe()).toBeNull();
    picker.dispose();
  });
});

describe("Picker overlap enumeration", () => {
  const VIEW = { w: 800, h: 600 };

  afterEach(() => {
    disposeAll();
    __resetRegistryForTests();
  });

  function harness(options: { hiddenFront?: boolean; tied?: boolean; clipped?: boolean } = {}) {
    const root = new THREE.Group();
    for (const [bodyId, meshRev, z] of [
      ["body-a", 11, 0],
      ["body-b", 12, options.tied ? 0 : -50],
    ] as const) {
      const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), bodyId, meshRev);
      swap(bodyId, entry);
      const mesh = new THREE.Mesh(
        entry.geometry,
        new THREE.MeshStandardMaterial({ side: THREE.DoubleSide }),
      );
      mesh.userData = { bodyId, kind: "face" };
      mesh.position.z = z;
      const parent = new THREE.Group();
      parent.visible = !(options.hiddenFront && bodyId === "body-a");
      parent.add(mesh);
      root.add(parent);
    }
    root.updateMatrixWorld(true);
    const camera = new THREE.PerspectiveCamera(50, VIEW.w / VIEW.h, 0.1, 5000);
    camera.position.set(0, -20, 400);
    camera.lookAt(0, 0, -20);
    camera.updateMatrixWorld(true);
    const canvas = document.createElement("canvas");
    canvas.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: VIEW.w, height: VIEW.h }) as DOMRect;
    const picker = new Picker({
      canvas,
      getCamera: () => camera,
      getRoot: () => root,
      getViewportHeight: () => VIEW.h,
      getFocusDistance: () => 400,
      getResolution: () => VIEW,
      getClippingPlanes: () => options.clipped
        ? [new THREE.Plane(new THREE.Vector3(0, 0, -1), -20)]
        : null,
      invalidate: vi.fn(),
      isActive: () => true,
      onHover: vi.fn(),
      onPick: vi.fn(),
    });
    return picker;
  }

  it("returns all stacked visible bodies with triangle hits deduplicated", () => {
    const picker = harness();
    const hits = picker.probeCandidates(VIEW.w / 2, VIEW.h / 2, { kinds: ["face"] });
    expect(new Set(hits.map((hit) => hit.bodyId))).toEqual(new Set(["body-a", "body-b"]));
    expect(new Set(hits.map((hit) => `${hit.bodyId}/${hit.topoKey}`)).size).toBe(hits.length);
    expect(hits[0]?.bodyId).toBe("body-a");
    picker.dispose();
  });

  it("honors clipping, hidden parents, inclusion, exclusion and body filtering", () => {
    const clipped = harness({ clipped: true });
    expect(new Set(clipped.probeCandidates(400, 300).map((hit) => hit.bodyId)))
      .toEqual(new Set(["body-b"]));
    clipped.dispose();
    const hidden = harness({ hiddenFront: true });
    expect(new Set(hidden.probeCandidates(400, 300).map((hit) => hit.bodyId)))
      .toEqual(new Set(["body-b"]));
    expect(hidden.probeCandidates(400, 300, { includeBodyIds: ["body-a"] })).toEqual([]);
    expect(hidden.probeCandidates(400, 300, { excludeBodyIds: ["body-b"] })).toEqual([]);
    const bodies = hidden.probeCandidates(400, 300, { kinds: ["body"] });
    expect(bodies).toHaveLength(1);
    expect(bodies[0]).toMatchObject({ kind: "body", bodyId: "body-b", topoKey: "body-b" });
    hidden.dispose();
  });

  it("sorts exact depth ties by stable identity and rejects unsupported kinds", () => {
    const picker = harness({ tied: true });
    const bodies = picker.probeCandidates(400, 300, { kinds: ["body"] });
    expect(bodies.slice(0, 2).map((hit) => hit.bodyId)).toEqual(["body-a", "body-b"]);
    expect(picker.probeCandidates(400, 300, {
      kinds: ["vertex" as unknown as "face"],
    })).toEqual([]);
    picker.dispose();
  });
});

/*
 * Edge-vs-face arbitration in SCREEN space (D9, finding T8).
 *
 * The two halves are tested apart. The DEPTH window is pure arithmetic over
 * fabricated hits, because the point of the change is which depth the window is
 * measured at — something no scene can demonstrate on its own. The SCREEN
 * distance is a real raycast against the real box, because it is three's Line2
 * radius that enforces it and only a raycast proves what that radius is.
 */
describe("edgeDepthSlack — the window follows the HIT depth, not the orbit target", () => {
  const H = 600;
  const cam = (fov: number) => {
    const c = new THREE.PerspectiveCamera(fov, 4 / 3, 0.1, 5000);
    c.updateProjectionMatrix();
    return c;
  };

  it("is exactly EDGE_DEPTH_SLACK_PX pixels of world size at the given depth", () => {
    const c = cam(35);
    expect(edgeDepthSlack(c, H, 400)).toBeCloseTo(EDGE_DEPTH_SLACK_PX * pixelWorldSize(c, H, 400), 9);
    // …and a pixel really does grow with depth and with the field of view.
    expect(pixelWorldSize(c, H, 800)).toBeCloseTo(pixelWorldSize(c, H, 400) * 2, 9);
    expect(pixelWorldSize(cam(76), H, 400)).toBeGreaterThan(pixelWorldSize(cam(35), H, 400));
  });

  it("is depth-independent for an orthographic camera", () => {
    const oc = new THREE.OrthographicCamera(-100, 100, 50, -50, 0.1, 1000); // height 100
    expect(pixelWorldSize(oc, H, 10)).toBeCloseTo(pixelWorldSize(oc, H, 9000), 9);
    expect(pixelWorldSize(oc, H, 10)).toBeCloseTo(100 / H, 9);
  });

  it("accepts a boundary edge a pixel behind its face and refuses one forty deep", () => {
    const c = cam(35);
    const faceDepth = 400;
    const px = pixelWorldSize(c, H, faceDepth);
    const face = { ...fakeFaceHit("body1", 0), distance: faceDepth } as THREE.Intersection;
    const justBehind = fakeEdgeHit("body1", 0, faceDepth + 1.5 * px);
    const farBehind = fakeEdgeHit("body1", 0, faceDepth + 40 * px);
    const slack = edgeDepthSlack(c, H, faceDepth);
    expect(choosePreferredHit(face, justBehind, slack)?.kind).toBe("edge");
    expect(choosePreferredHit(face, farBehind, slack)?.kind).toBe("face");

    /*
     * The retired rule read the ORBIT-TARGET distance instead of the face's own,
     * so the same two hits flipped with nothing but a pan or a zoom: a target
     * pulled in to 30 refused the boundary edge (T8), and one pushed out to 3000
     * accepted geometry forty pixels deep (the session-32 inverse).
     */
    expect(choosePreferredHit(face, justBehind, linePickThreshold(c, H, 30))?.kind).toBe("face");
    expect(choosePreferredHit(face, farBehind, linePickThreshold(c, H, 3000))?.kind).toBe("edge");
  });

  it("keeps the face when the edge is far behind it, whatever the camera", () => {
    for (const c of [cam(76), cam(35)]) {
      const face = { ...fakeFaceHit("body1", 0), distance: 100 } as THREE.Intersection;
      const occluded = fakeEdgeHit("body1", 0, 400);
      expect(choosePreferredHit(face, occluded, edgeDepthSlack(c, 600, 100))?.kind).toBe("face");
    }
  });
});

/*
 * The screen-distance half, end to end. The mock box (80×60×30, origin-centred)
 * carries both its face mesh and its fat edge lines, so every pick below goes
 * through the same `raycastAll` + `choosePreferredHit` path the pointer uses.
 */
describe("Picker screen-space edge arbitration — real faces and real edges", () => {
  const VIEW = { w: 800, h: 600 };
  /** The vertical edge at the (+x,+y) corner: [40,30,−15]..[40,30,15]. */
  const CORNER: [number, number, number] = [40, 30, 0];
  const CORNER_TOP: [number, number, number] = [40, 30, 15];

  afterEach(() => {
    disposeAll();
    __resetRegistryForTests();
  });

  function harness(camera: THREE.Camera) {
    const entry = boxEntry();
    swap("body1", entry);
    const root = new THREE.Group();
    const mesh = new THREE.Mesh(
      entry.geometry,
      new THREE.MeshStandardMaterial({ side: THREE.DoubleSide }),
    );
    mesh.userData = { bodyId: "body1", kind: "face" };
    root.add(mesh);
    const line = new LineSegments2(
      entry.edgeGeometry!,
      new LineMaterial({ linewidth: BODY_EDGE_WIDTH }),
    );
    line.userData = { bodyId: "body1", kind: "edge" };
    root.add(line);
    root.updateMatrixWorld(true);

    const canvas = document.createElement("canvas");
    canvas.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: VIEW.w, height: VIEW.h }) as DOMRect;

    const picker = new Picker({
      canvas,
      getCamera: () => camera,
      getRoot: () => root,
      getViewportHeight: () => VIEW.h,
      // Deliberately NOT the camera's distance: the orbit target is wherever the
      // user last panned to, and the arbitration must not read it.
      getFocusDistance: () => 30,
      // dpr 1, so one device pixel IS one CSS pixel and the offsets below are
      // the CSS px the EDGE_PICK_PX radius is expressed in.
      getResolution: () => ({ w: VIEW.w, h: VIEW.h }),
      invalidate: vi.fn(),
      isActive: () => true,
      onHover: vi.fn(),
      onPick: vi.fn(),
    });
    return picker;
  }

  /** World point → client px, the inverse of the Picker's own NDC math. */
  function toScreen(camera: THREE.Camera, p: readonly [number, number, number]) {
    const v = new THREE.Vector3(p[0], p[1], p[2]).project(camera);
    return { x: ((v.x + 1) / 2) * VIEW.w, y: ((1 - v.y) / 2) * VIEW.h };
  }

  /** Unit screen vector perpendicular to the corner edge, pointing INTO the box. */
  function inward(camera: THREE.Camera) {
    const a = toScreen(camera, CORNER);
    const b = toScreen(camera, CORNER_TOP);
    const along = { x: b.x - a.x, y: b.y - a.y };
    const len = Math.hypot(along.x, along.y);
    expect(len).toBeGreaterThan(1); // a head-on edge would make this meaningless
    const n = { x: -along.y / len, y: along.x / len };
    const centre = toScreen(camera, [0, 0, 0]);
    const sign = Math.sign(n.x * (centre.x - a.x) + n.y * (centre.y - a.y)) || 1;
    return { x: n.x * sign, y: n.y * sign };
  }

  /** Pick `px` CSS pixels off the corner edge (positive = into the body). */
  function pickOffEdge(camera: THREE.Camera, px: number) {
    const picker = harness(camera);
    const at = toScreen(camera, CORNER);
    const dir = inward(camera);
    const hit = picker.probe(at.x + dir.x * px, at.y + dir.y * px);
    picker.dispose();
    return hit;
  }

  function perspective(fov: number): THREE.Camera {
    const c = new THREE.PerspectiveCamera(fov, VIEW.w / VIEW.h, 0.1, 5000);
    c.up.set(0, 0, 1); // engine invariant: world is Z-up
    c.position
      .set(...CORNER)
      .addScaledVector(new THREE.Vector3(1, 1, 0.35).normalize(), 400);
    c.lookAt(...CORNER);
    c.updateMatrixWorld(true);
    return c;
  }

  function ortho(): THREE.Camera {
    const halfH = 60;
    const c = new THREE.OrthographicCamera(
      (-halfH * VIEW.w) / VIEW.h,
      (halfH * VIEW.w) / VIEW.h,
      halfH,
      -halfH,
      0.1,
      5000,
    );
    c.up.set(0, 0, 1);
    c.position
      .set(...CORNER)
      .addScaledVector(new THREE.Vector3(1, 1, 0.35).normalize(), 400);
    c.lookAt(...CORNER);
    c.updateMatrixWorld(true);
    return c;
  }

  describe.each([
    ["FOV 76", perspective(76)],
    ["FOV 35", perspective(35)],
    ["orthographic", ortho()],
  ])("%s", (_label, camera) => {
    it("takes the edge two pixels inside it, with the face directly behind", () => {
      expect(pickOffEdge(camera, 2)).toMatchObject({ kind: "edge", topoKey: "e:10" });
    });

    it("still takes the edge seven pixels in — the raised EDGE_PICK_PX", () => {
      expect(pickOffEdge(camera, 7)).toMatchObject({ kind: "edge", topoKey: "e:10" });
    });

    it("gives the face the pick twelve pixels in, past the radius", () => {
      expect(pickOffEdge(camera, 12)?.kind).toBe("face");
    });

    it("takes the silhouette edge from three pixels OUTSIDE the body", () => {
      // Nothing behind it at all, so the face can never be the answer here.
      expect(pickOffEdge(camera, -3)).toMatchObject({ kind: "edge", topoKey: "e:10" });
    });

    it("takes the face in the middle of it, far from any edge", () => {
      // The session-32 inverse: a left-click on a body face must select that
      // face, never an edge that happens to be near in world units.
      const picker = harness(camera);
      const at = toScreen(camera, [40, 0, 0]); // centre of the +X face
      const hit = picker.probe(at.x, at.y);
      picker.dispose();
      expect(hit).toMatchObject({ kind: "face", bodyId: "body1" });
    });
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
