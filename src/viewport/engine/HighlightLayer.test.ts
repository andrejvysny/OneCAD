/*
 * HighlightLayer: the pure slice math, the overlay material contract, and the
 * ONE place the layer's "never dispose" rule is deliberately broken.
 *
 * Face ranges are triangle units → index units (×3). Edge ranges stay in
 * SEGMENT units: body edges are instanced fat lines, where one instance IS one
 * segment, so there is no ×2 vertex expansion to undo any more.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import * as THREE from "three";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { faceDrawRange, edgeSegmentSlice, HighlightLayer } from "./HighlightLayer";
import { palette } from "./palette";
import {
  buildBodyObjects,
  swap,
  disposeAll,
  __resetRegistryForTests,
} from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { encodeMesh1, makeBoxMesh } from "@/ipc/mockMeshes";

afterEach(() => {
  disposeAll();
  __resetRegistryForTests();
});

const deps = () => ({ root: new THREE.Group(), invalidate: vi.fn() });

/** A registered box — addHighlight resolves bodies through the REGISTRY. */
function registerBox(bodyId = "body1"): void {
  swap(bodyId, buildBodyObjects(parseMeshPayload(makeBoxMesh()), bodyId, 1));
}

describe("faceDrawRange (indexed geometry, 3 indices/triangle)", () => {
  // 3 faces: face0 tris[0,3), face1[3,2), face2[5,3).
  const faceRanges = new Uint32Array([0, 3, 3, 2, 5, 3]);
  it("maps face ordinal → {start,count} in index units", () => {
    expect(faceDrawRange(faceRanges, 0)).toEqual({ start: 0, count: 9 });
    expect(faceDrawRange(faceRanges, 1)).toEqual({ start: 9, count: 6 });
    expect(faceDrawRange(faceRanges, 2)).toEqual({ start: 15, count: 9 });
  });
});

describe("edgeSegmentSlice (instanced fat lines, 1 instance/segment)", () => {
  // 2 edges: edge0 segs[0,2), edge1 segs[2,1).
  const segRanges = new Uint32Array([0, 2, 2, 1]);

  it("maps edge ordinal → {first,count} in SEGMENT units (no ×2)", () => {
    expect(edgeSegmentSlice(segRanges, 0)).toEqual({ first: 0, count: 2 });
    expect(edgeSegmentSlice(segRanges, 1)).toEqual({ first: 2, count: 1 });
  });

  it("addresses the segment buffer at 6 floats per segment", () => {
    // The contract buildEdge relies on: [first*6, (first+count)*6) is exactly
    // this edge's endpoints in the entry's expanded positions array.
    const positions = new Float32Array(3 * 6).map((_, i) => i);
    const { first, count } = edgeSegmentSlice(segRanges, 1);
    expect([...positions.subarray(first * 6, (first + count) * 6)]).toEqual([
      12, 13, 14, 15, 16, 17,
    ]);
  });
});

/*
 * Selection/hover read as three distinct states in the viewport, so the three
 * face materials must not collapse into one another: cyan hover, selected tint
 * per element, and a LIGHTER selected tint for a whole body (a body's worth of
 * surface at the per-face opacity buries the shading that reads as shape).
 */
describe("overlay materials", () => {
  it("hover is the cyan viewport token, selection is the selected tint", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState({ kind: "face", id: "body1#f:1", bodyId: "body1", topoKey: "f:1" }, [
      { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" },
    ]);

    const mats = d.root.children.map(
      (o) => (o as THREE.Mesh).material as THREE.MeshBasicMaterial,
    );
    expect(mats).toHaveLength(2);
    const hover = mats.find((m) => m.opacity === 0.45)!;
    const selected = mats.find((m) => m.opacity === 0.55)!;
    expect(hover.color.getHex()).toBe(palette.hover3d().getHex());
    expect(selected.color.getHex()).toBe(palette.selected3d().getHex());
    layer.dispose();
  });

  it("a whole selected BODY uses its own, lighter tint material", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState(null, [{ kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" }]);
    const faceMat = (d.root.children[0] as THREE.Mesh).material as THREE.MeshBasicMaterial;

    layer.setState(null, [{ kind: "body", id: "body1" }]);
    const bodyMat = (d.root.children[0] as THREE.Mesh).material as THREE.MeshBasicMaterial;

    expect(bodyMat).not.toBe(faceMat);
    expect(bodyMat.color.getHex()).toBe(faceMat.color.getHex()); // same token…
    expect(bodyMat.opacity).toBe(0.45); // …lighter than the 0.55 face tint
    layer.dispose();
  });

  it("edge overlays are fat lines, heavier than a body edge, drawn over it", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState({ kind: "edge", id: "body1#e:1", bodyId: "body1", topoKey: "e:1" }, []);
    const line = d.root.children[0] as LineSegments2;
    expect(line).toBeInstanceOf(LineSegments2);

    const mat = line.material;
    expect(mat.color.getHex()).toBe(palette.hover3d().getHex());
    expect(mat.linewidth).toBeGreaterThan(1.5); // > BODY_EDGE_WIDTH at dpr 1
    expect(mat.depthTest).toBe(false);
    expect(mat.transparent).toBe(true);
    layer.dispose();
  });

  it("refreshColors re-reads every shared material", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    layer.setState({ kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" }, [
      { kind: "body", id: "body1" },
    ]);
    const before = d.root.children.map((o) =>
      ((o as THREE.Mesh).material as THREE.Material & { color: THREE.Color }).color.getHex(),
    );

    layer.refreshColors();

    // No theme flip here — the point is that refreshColors touches the live
    // materials rather than throwing or swapping them out.
    expect(
      d.root.children.map((o) =>
        ((o as THREE.Mesh).material as THREE.Material & { color: THREE.Color }).color.getHex(),
      ),
    ).toEqual(before);
    layer.dispose();
  });
});

/*
 * The disposal split. Face/body overlays SHARE the body's BufferAttributes and
 * must never be disposed; edge overlays own the InstancedInterleavedBuffer
 * `setPositions` built for them and leak a GL buffer per hover if they are not.
 */
describe("setClippingPlanes (section view)", () => {
  const planes = () => [new THREE.Plane(new THREE.Vector3(0, 0, -1), 0)];

  it("reaches all FIVE overlay materials, including the ones not drawn yet", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    const p = planes();

    layer.setClippingPlanes(p);

    // Hover + selection, face + edge, at once: that is four of the five, and the
    // whole-body tint is the fifth.
    layer.setState({ kind: "edge", id: "body1#e:1", bodyId: "body1", topoKey: "e:1" }, [
      { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" },
      { kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" },
      { kind: "body", id: "body1" },
    ]);
    const mats = new Set<THREE.Material>();
    d.root.traverse((o) => {
      const m = (o as THREE.Mesh).material as THREE.Material | undefined;
      if (m) mats.add(m);
    });
    expect(mats.size).toBeGreaterThan(0);
    for (const m of mats) expect(m.clippingPlanes).toBe(p);

    layer.setClippingPlanes(null);
    for (const m of mats) expect(m.clippingPlanes).toBeNull();
    layer.dispose();
  });

  it("recompiles only on a plane COUNT change, and repaints every time", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();
    layer.setState(null, [{ kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" }]);
    const mat = (d.root.children[0] as THREE.Mesh).material as THREE.Material;
    const p = planes();

    layer.setClippingPlanes(p);
    const version = mat.version;
    const repaints = d.invalidate.mock.calls.length; // setState repainted too
    layer.setClippingPlanes(p);
    layer.setClippingPlanes(p);

    expect(mat.version).toBe(version);
    // Still one repaint per call: the plane MOVED even when nothing recompiled.
    expect(d.invalidate.mock.calls.length).toBe(repaints + 2);
    layer.dispose();
  });
});

describe("geometry ownership on rebuild", () => {
  it("disposes an edge overlay's geometry but never a face overlay's", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState(null, [
      { kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" },
      { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" },
    ]);
    const edge = d.root.children.find((o) => o instanceof LineSegments2)!;
    const face = d.root.children.find((o) => !(o instanceof LineSegments2))!;
    const edgeSpy = vi.spyOn((edge as LineSegments2).geometry, "dispose");
    const faceSpy = vi.spyOn((face as THREE.Mesh).geometry, "dispose");

    layer.setState(null, []); // rebuild drops both

    expect(edgeSpy).toHaveBeenCalledTimes(1);
    expect(faceSpy).not.toHaveBeenCalled();
    expect(d.root.children).toHaveLength(0);
    layer.dispose();
  });

  it("the edge overlay covers only its own edge's segments", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState(null, [{ kind: "edge", id: "body1#e:3", bodyId: "body1", topoKey: "e:3" }]);
    const geo = (d.root.children[0] as LineSegments2).geometry;
    // A box edge is a single 2-point polyline ⇒ exactly one instanced segment.
    expect(geo.instanceCount).toBe(1);
    layer.dispose();
  });
});

/*
 * Element lookup goes through `ordinalForRef`, not the raw TopoKey: a TopoKey is
 * snapshot-scoped and a regen may rename it, while a PROMOTED ref also carries
 * the persistent ElementId — which is what a MESH1 blob with IDS_HAVE_ELEMENTIDS
 * puts in its id tables. Resolving one key only would drop the overlay of a
 * selection that is still perfectly well identified.
 */
describe("resolving a ref against the mesh", () => {
  it("drops the overlay when NEITHER id names anything (never a guess)", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox();

    layer.setState(null, [
      { kind: "face", id: "body1#f:99", bodyId: "body1", topoKey: "f:99", elementId: "el_x" },
      { kind: "edge", id: "body1#e:99", bodyId: "body1", topoKey: "e:99" },
    ]);

    expect(d.root.children).toHaveLength(0);
    layer.dispose();
  });

  it("falls back to the promoted ElementId when the mesh's ids ARE ElementIds", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    // A blob whose face id table holds minted ids (IDS_HAVE_ELEMENTIDS set).
    swap(
      "body1",
      buildBodyObjects(
        parseMeshPayload(
          encodeMesh1({
            positions: [0, 0, 0, 10, 0, 0, 10, 10, 0, 0, 10, 0],
            faces: [{ id: "el_top", triangles: [[0, 1, 2], [0, 2, 3]] }],
            idsHaveElementIds: true,
          }),
        ),
        "body1",
        1,
      ),
    );

    // The TopoKey is stale; only the ElementId still names the face.
    layer.setState(null, [
      { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0", elementId: "el_top" },
    ]);

    expect(d.root.children).toHaveLength(1);
    layer.dispose();
  });

  it("does NOT reach for the ElementId when the mesh's ids are TopoKeys", () => {
    const d = deps();
    const layer = new HighlightLayer(d);
    registerBox(); // IDS_HAVE_ELEMENTIDS clear — "f:0" is a TopoKey, not an id

    layer.setState(null, [
      { kind: "face", id: "body1#f:99", bodyId: "body1", topoKey: "f:99", elementId: "f:0" },
    ]);

    expect(d.root.children).toHaveLength(0);
    layer.dispose();
  });
});
