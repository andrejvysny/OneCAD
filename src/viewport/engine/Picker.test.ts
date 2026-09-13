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
  choosePreferredHit,
  firstUnclippedHit,
  secondaryHitWins,
  resolvePick,
  pickKey,
} from "./Picker";
import { BODY_EDGE_WIDTH_CSS } from "./bodyMaterials";
import { line2PickThresholdCss } from "./screenLineStyle";
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

  function harness(resolutionCss: { w: number; h: number }) {
    const entry = boxEntry();
    swap("body1", entry);
    const material = new LineMaterial({ linewidth: BODY_EDGE_WIDTH_CSS });
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
      getResolutionCss: () => resolutionCss,
      invalidate: vi.fn(),
      isActive: () => true,
      onHover: vi.fn(),
      onPick: vi.fn(),
    });
    // Pointer at the canvas center ⇒ NDC (0,0) ⇒ the ray through `target`.
    // `probeOffset(dx)` walks the pointer dx CSS px to the right of the edge:
    // the camera's right vector is horizontal (world up is +Z), so the edge —
    // which runs along world Z — projects EXACTLY vertical and dx is the
    // perpendicular screen distance.
    return {
      picker,
      material,
      probe: () => picker.probe(VIEW.w / 2, VIEW.h / 2),
      probeOffset: (dx: number) => picker.probe(VIEW.w / 2 + dx, VIEW.h / 2),
    };
  }

  it("resolves the edge under the pointer to its own TopoKey", () => {
    const { picker, probe } = harness(VIEW);
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
    const { picker, probe } = harness(VIEW);
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

  it("flushes the LOGICAL (CSS) viewport resolution into the shared edge material", () => {
    // Not the drawing buffer: `LineSegments2.onBeforeRender` overwrites this
    // from `renderer.getViewport()`, which `WebGLRenderer.setSize` stores
    // UNSCALED. A device-px flush would disagree with every rendered frame.
    const { picker, material, probe } = harness(VIEW);
    expect(material.resolution.x).toBe(0); // nothing has rendered
    probe();
    expect([material.resolution.x, material.resolution.y]).toEqual([VIEW.w, VIEW.h]);
    picker.dispose();
  });

  it("would find NOTHING at resolution (0,0) — the reason the flush exists", () => {
    // Non-vacuity guard for the test above: neuter the flush (report zeros) and
    // the identical pick goes silently null.
    const { picker, probe } = harness({ w: 0, h: 0 });
    expect(probe()).toBeNull();
    picker.dispose();
  });

  /*
   * TEST-LINE-03 — the edge acquisition radius is 6 CSS px, at every DPR, and
   * before as well as after the first rendered frame.
   *
   * The hit radius three actually uses is `(linewidth + threshold) / 2` in the
   * units `material.resolution` carries. With CSS px on both sides that is
   * `(1.25 + line2PickThresholdCss(6, 1.25)) / 2` = 6 CSS px exactly, and the
   * device pixel ratio appears nowhere in the expression. The two DPR cases
   * below therefore have to produce the SAME boundary — that is the assertion.
   */
  describe("TEST-LINE-03 acquisition radius", () => {
    /** The largest offset that still acquires, bisected to 1/64 CSS px. */
    function acquisitionBoundary(probeOffset: (dx: number) => unknown): number {
      let hit = 0;
      let miss = 24;
      while (miss - hit > 1 / 64) {
        const mid = (hit + miss) / 2;
        if (probeOffset(mid)) hit = mid;
        else miss = mid;
      }
      return (hit + miss) / 2;
    }

    it("is 6 CSS px whatever window.devicePixelRatio says", () => {
      // Sanity: the threshold the Picker installs really does make the radius 6.
      expect((BODY_EDGE_WIDTH_CSS + line2PickThresholdCss(6, BODY_EDGE_WIDTH_CSS)) / 2).toBe(6);

      const boundaries: number[] = [];
      for (const dpr of [1, 2]) {
        vi.stubGlobal("devicePixelRatio", dpr);
        const { picker, material, probeOffset } = harness(VIEW);

        // BEFORE any frame: resolution is still (0,0) and the Picker's own
        // flush is the only thing that makes the raycast resolve at all.
        expect(material.resolution.x).toBe(0);
        const before = acquisitionBoundary(probeOffset);

        // AFTER a frame: `onBeforeRender` has written the logical viewport,
        // which is the same pair of numbers the flush writes.
        material.resolution.set(VIEW.w, VIEW.h);
        const after = acquisitionBoundary(probeOffset);

        expect(after).toBeCloseTo(before, 6);
        expect(before).toBeCloseTo(6, 1);
        boundaries.push(before);
        picker.dispose();
        disposeAll();
        __resetRegistryForTests();
        vi.unstubAllGlobals();
      }
      expect(boundaries[1]).toBeCloseTo(boundaries[0], 6);
    });

    it("moves when a DEVICE-px resolution is flushed — the R01 non-vacuity guard", () => {
      // Feed the old (drawing-buffer) numbers. The raycast maps the pointer's
      // NDC offset into RESOLUTION pixels, so a 2x resolution halves the
      // acquisition radius measured in the CSS pixels the pointer moves in:
      // 6 / DPR = 3, not 6. Proof the CSS units above are load bearing and not
      // an accident of the fixture.
      const { picker, probeOffset } = harness({ w: VIEW.w * DPR, h: VIEW.h * DPR });
      expect(acquisitionBoundary(probeOffset)).toBeCloseTo(6 / DPR, 1);
      picker.dispose();
    });
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
      getResolutionCss: () => ({ w: VIEW.w, h: VIEW.h }),
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
      getResolutionCss: () => VIEW,
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

describe("pickKey", () => {
  it("is stable for the same element and null-safe", () => {
    const entry = boxEntry();
    const hit = resolvePick(fakeFaceHit("body1", 0), "face", () => entry)!;
    expect(pickKey(hit)).toBe("body1/face/f:0");
    expect(pickKey(null)).toBeNull();
  });
});
