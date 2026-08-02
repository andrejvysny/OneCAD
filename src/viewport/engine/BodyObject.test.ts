/*
 * BodyObject render-mode matrix (W3).
 *
 * A mode is expressed as CHILD visibility (face Mesh / edge LineSegments) plus
 * the kind of shared material set the children point at — never as a
 * `material.wireframe` flip, which would leak onto every other body sharing that
 * set.
 */
import { describe, it, expect, afterEach } from "vitest";
import * as THREE from "three";
import { buildBodyObject } from "./BodyObject";
import { BodyMaterialLibrary } from "./bodyMaterials";
import { DEFAULT_RENDER_MODE, RENDER_MODES } from "./renderModes";
import { buildBodyObjects, __resetRegistryForTests, disposeAll } from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";

afterEach(() => {
  disposeAll();
  __resetRegistryForTests();
});

function handleFor() {
  const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1);
  const library = new BodyMaterialLibrary();
  const handle = buildBodyObject(entry, library);
  const face = handle.group.children.find((c) => c.userData.kind === "face") as THREE.Mesh;
  const edges = handle.group.children.find(
    (c) => c.userData.kind === "edge",
  ) as THREE.LineSegments;
  return { handle, face, edges, entry, library };
}

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
    const set = library.get(RENDER_MODES[DEFAULT_RENDER_MODE].materialKind);
    expect(face.material).toBe(set.face);
    expect(edges.material).toBe(set.edge);
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
