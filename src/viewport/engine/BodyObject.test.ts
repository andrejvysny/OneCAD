/*
 * BodyObject render-mode matrix (W3).
 *
 * A mode is expressed as CHILD visibility (face Mesh / edge LineSegments2) plus
 * the kind of shared material set the children point at, and — for edges — which
 * of that set's two edge materials the mode's `edgeStyle` names. Never a
 * `material.wireframe` flip, which would leak onto every other body sharing that
 * set.
 */
import { describe, it, expect, afterEach } from "vitest";
import * as THREE from "three";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { buildBodyObject } from "./BodyObject";
import { BodyMaterialLibrary } from "./bodyMaterials";
import { DEFAULT_RENDER_MODE, RENDER_MODES, vertexColorKind } from "./renderModes";
import {
  buildBodyObjects,
  flushDisposals,
  openLeases,
  remove,
  swap,
  __resetRegistryForTests,
  disposeAll,
} from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { makeBoxMesh, type FaceColor } from "@/ipc/mockMeshes";

afterEach(() => {
  disposeAll();
  __resetRegistryForTests();
});

/** Stand-in authored STEP color on f:0 only (the rest fall back to the body material). */
const RED: FaceColor = [214, 74, 62, 255];
const coloredBoxMesh = () => makeBoxMesh(40, 40, 40, 0, [0, 0, 0], [RED, null, null, null, null, null]);

function handleFor(mesh: ArrayBuffer = makeBoxMesh()) {
  const entry = buildBodyObjects(parseMeshPayload(mesh), "body1", 1);
  const library = new BodyMaterialLibrary();
  const handle = buildBodyObject(entry, library);
  const face = handle.group.children.find((c) => c.userData.kind === "face") as THREE.Mesh;
  const edges = handle.group.children.find((c) => c.userData.kind === "edge") as LineSegments2;
  return { handle, face, edges, entry, library };
}

/*
 * D6 — the section stencils must lease the resource they actually DRAW, so the
 * face mesh carries the entry object itself, not just a body id that a
 * registry/scene divergence could resolve to a different resource (PR-06).
 */
describe("buildBodyObject resource identity", () => {
  it("stamps the leased entry on the face mesh it drew with", () => {
    const { handle, face, entry, library } = handleFor();
    expect(face.userData.meshEntry).toBe(entry);
    expect(face.geometry).toBe(entry.geometry);
    handle.dispose();
    entry.dispose();
    library.dispose();
  });
});

describe("BodyObjectHandle.applyMode", () => {
  it("shaded = faces only · shadedEdges = both · wireframe = edges only", () => {
    const { handle, face, edges, entry, library } = handleFor();
    expect(face).toBeDefined();
    expect(edges).toBeDefined();

    handle.applyMode(RENDER_MODES.shaded);
    expect([face.visible, edges.visible]).toEqual([true, false]);

    handle.applyMode(RENDER_MODES.shadedEdges);
    expect([face.visible, edges.visible]).toEqual([true, true]);

    handle.applyMode(RENDER_MODES.wireframe);
    expect([face.visible, edges.visible]).toEqual([false, true]);

    entry.dispose();
    library.dispose();
  });

  it("never touches the SHARED materials (no material.wireframe flip)", () => {
    const { handle, entry, library } = handleFor();
    const materials = library.get(RENDER_MODES.wireframe.materialKind);
    handle.applyMode(RENDER_MODES.wireframe);
    expect(materials.face.wireframe).toBe(false);
    expect(materials.face.visible).toBe(true);
    entry.dispose();
    library.dispose();
  });

  it("points both children at the mode's material set", () => {
    const { handle, face, edges, entry, library } = handleFor();
    const set = library.get(RENDER_MODES.wireframe.materialKind);
    handle.applyMode(RENDER_MODES.wireframe);
    expect(face.material).toBe(set.face);
    expect(edges.material).toBe(set.edgeWire);
    entry.dispose();
    library.dispose();
  });

  /*
   * The edge material follows `edgeStyle`, not the mode id: edges over a shaded
   * face are a near-black OUTLINE, edges drawn alone must invert with the theme
   * or they vanish on a dark canvas. Both live in the same shared set, so the
   * mode swaps the pointer rather than recoloring one material.
   */
  it("edgeStyle selects the edge material: onFaces → edge, standalone → edgeWire", () => {
    const { handle, edges, entry, library } = handleFor();
    const set = library.get(RENDER_MODES.shadedEdges.materialKind);

    handle.applyMode(RENDER_MODES.shadedEdges);
    expect(RENDER_MODES.shadedEdges.edgeStyle).toBe("onFaces");
    expect(edges.material).toBe(set.edge);

    handle.applyMode(RENDER_MODES.wireframe);
    expect(RENDER_MODES.wireframe.edgeStyle).toBe("standalone");
    expect(edges.material).toBe(set.edgeWire);

    // …and back, so the swap is not one-way.
    handle.applyMode(RENDER_MODES.shadedEdges);
    expect(edges.material).toBe(set.edge);
    entry.dispose();
    library.dispose();
  });

  it("is independent of setVisible — the group flag survives a mode change", () => {
    const { handle, face, entry, library } = handleFor();
    handle.setVisible(false);
    handle.applyMode(RENDER_MODES.shaded);
    expect(handle.group.visible).toBe(false);
    expect(face.visible).toBe(true); // the mode says faces; the group still hides them

    handle.setVisible(true);
    expect(handle.group.visible).toBe(true);
    entry.dispose();
    library.dispose();
  });
});

describe("buildBodyObject", () => {
  it("materials the children at BUILD time — previews never get an applyMode call", () => {
    const { face, edges, entry, library } = handleFor();
    const def = RENDER_MODES[DEFAULT_RENDER_MODE];
    const set = library.get(def.materialKind);
    expect(face.material).toBe(set.face);
    // The default mode's edgeStyle, not just "whatever `edge` is".
    expect(edges.material).toBe(def.edgeStyle === "standalone" ? set.edgeWire : set.edge);
    entry.dispose();
    library.dispose();
  });

  /*
   * Fat lines, not THREE.LineSegments: WebGL clamps LineBasicMaterial.linewidth
   * to 1 device px, so a body's edges would render as sub-CSS-pixel hairlines on
   * any HiDPI display. The type also decides how the Picker reads a hit
   * (faceIndex = segment ordinal, vs a plain line's vertex index).
   */
  it("draws edges as a LineSegments2 over the entry's instanced geometry", () => {
    const { edges, entry, library } = handleFor();
    expect(edges).toBeInstanceOf(LineSegments2);
    expect(edges.geometry).toBe(entry.edgeGeometry);
    entry.dispose();
    library.dispose();
  });

  /*
   * A colored body substitutes the VERTEX variant of whatever kind the mode
   * dictates. It is a per-body swap, not a mode: face/edge visibility, and thus
   * wireframe/edges behavior, must be byte-identical to a plain body's.
   */
  it("picks the vertex-color material kind for a body with baked FACE_COLORS", () => {
    const { handle, face, entry, library } = handleFor(coloredBoxMesh());
    expect(entry.hasVertexColors).toBe(true);

    for (const def of Object.values(RENDER_MODES)) {
      handle.applyMode(def);
      const vertexSet = library.get(vertexColorKind(def.materialKind));
      expect(face.material).toBe(vertexSet.face);
      expect((face.material as THREE.MeshStandardMaterial).vertexColors).toBe(true);
      // …and NOT the plain set the mode names.
      expect(face.material).not.toBe(library.get(def.materialKind).face);
    }
    entry.dispose();
    library.dispose();
  });

  it("leaves a color-less body on the plain kind, with identical visibility", () => {
    const plain = handleFor();
    const colored = handleFor(coloredBoxMesh());

    for (const def of Object.values(RENDER_MODES)) {
      plain.handle.applyMode(def);
      colored.handle.applyMode(def);
      expect([colored.face.visible, colored.edges.visible]).toEqual([
        plain.face.visible,
        plain.edges.visible,
      ]);
    }
    expect((plain.face.material as THREE.MeshStandardMaterial).vertexColors).toBe(false);

    plain.entry.dispose();
    plain.library.dispose();
    colored.entry.dispose();
    colored.library.dispose();
  });

  it("materials a colored body at BUILD time too (previews never applyMode)", () => {
    const { face, entry, library } = handleFor(coloredBoxMesh());
    const set = library.get(vertexColorKind(RENDER_MODES[DEFAULT_RENDER_MODE].materialKind));
    expect(face.material).toBe(set.face);
    entry.dispose();
    library.dispose();
  });

  it("keeps the group name + the userData the Picker resolves through", () => {
    const { handle, face, edges, entry, library } = handleFor();
    expect(handle.group.name).toBe("body:body1");
    expect(handle.group.userData.bodyId).toBe("body1");
    expect([face.userData.bodyId, face.userData.kind]).toEqual(["body1", "face"]);
    expect([edges.userData.bodyId, edges.userData.kind]).toEqual(["body1", "edge"]);
    entry.dispose();
    library.dispose();
  });
});

/*
 * WP03 ownership (spec §8.2): a body object BORROWS the registry's exact
 * geometry objects, so it holds a lease for as long as its group is in the
 * scene — and something must give that borrow back even when the owner drops
 * the handle without disposing it (the exact-preview path in ViewportEngine,
 * whose file belongs to another package).
 */
describe("TEST-RES-02 — body objects borrow the registry resource", () => {
  it("draws the registry's exact geometry objects and holds ONE body lease", () => {
    const { handle, face, edges, entry, library } = handleFor();

    expect(face.geometry).toBe(entry.geometry);
    expect(edges.geometry).toBe(entry.edgeGeometry);
    expect(openLeases(entry)).toEqual(["body"]);

    handle.dispose();
    handle.dispose(); // idempotent
    expect(openLeases(entry)).toEqual([]);
    library.dispose();
  });

  it("holds a retired resource alive until the handle is disposed", () => {
    const root = new THREE.Group();
    const { handle, entry, library } = handleFor();
    swap("body1", entry);
    root.add(handle.group); // on screen, so only an explicit dispose lets it go

    swap("body1", buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 2));
    flushDisposals();
    flushDisposals();

    expect(entry.resourceState).toBe("retired"); // the borrow outranks the clock
    handle.dispose();
    flushDisposals();
    expect(entry.resourceState).toBe("disposed");
    library.dispose();
  });

  /*
   * D6: a handle nobody disposes KEEPS its lease. The detached-borrower sweep
   * that used to release it is gone — it ran only on a retirement event, so it
   * was never equivalent to ownership, and a cleanup heuristic standing in for
   * an owner is exactly finding PR-06. The tripwire catches the leak instead of
   * a sweep hiding it.
   */
  it("keeps the lease of a detached handle nobody disposed (no sweep)", () => {
    const root = new THREE.Group();
    const { handle, entry, library } = handleFor();
    swap("preview", entry);
    root.add(handle.group);

    remove("preview");
    root.remove(handle.group);
    swap("other", buildBodyObjects(parseMeshPayload(makeBoxMesh()), "other", 1));
    remove("other");
    flushDisposals();
    flushDisposals();

    expect(openLeases(entry)).toEqual(["body"]);
    expect(entry.resourceState).toBe("retired");

    handle.dispose();
    flushDisposals();
    expect(entry.resourceState).toBe("disposed");
    library.dispose();
  });

  it("a retirement elsewhere never touches this handle's lease (the PREPARE window)", () => {
    const root = new THREE.Group();
    const { handle, entry, library } = handleFor();
    swap("body1", entry); // this handle's resource is the CURRENT one…

    // …and a retirement lands while the handle is still unparented, exactly as
    // meshSync's atomic install does between PREPARE and PUBLISH.
    swap("other", buildBodyObjects(parseMeshPayload(makeBoxMesh()), "other", 1));
    remove("other");

    expect(openLeases(entry)).toEqual(["body"]);
    root.add(handle.group);
    handle.dispose();
    library.dispose();
  });
});
