import { afterEach, describe, expect, it } from "vitest";
import * as THREE from "three";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { buildBodyObjects, disposeAll } from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import { captureFaceBounds, isFaceBoundsCurrent } from "./faceFrameBounds";

afterEach(() => disposeAll());

describe("captured face bounds", () => {
  it("bounds only the addressed indexed face in world coordinates", () => {
    const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body", 1);
    const body = new THREE.Group();
    const local = captureFaceBounds(entry, body, { topoKey: "f:0" })!;
    body.position.set(10, 20, 30);
    const capture = captureFaceBounds(entry, body, { topoKey: "f:0" });
    expect(capture).not.toBeNull();
    const size = capture!.bounds.getSize(new THREE.Vector3());
    expect([size.x, size.y, size.z].filter((value) => value === 0)).toHaveLength(1);
    expect(capture!.bounds.getCenter(new THREE.Vector3()).sub(
      local.bounds.getCenter(new THREE.Vector3()),
    ).toArray()).toEqual([10, 20, 30]);
    entry.dispose();
  });

  it("refuses missing identity and invalidates a replaced or hidden body", () => {
    const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body", 1);
    const replacement = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body", 2);
    const body = new THREE.Group();
    const capture = captureFaceBounds(entry, body, { topoKey: "f:0" })!;
    expect(captureFaceBounds(entry, body, { topoKey: "f:404" })).toBeNull();
    expect(isFaceBoundsCurrent(capture, entry, body)).toBe(true);
    expect(isFaceBoundsCurrent(capture, replacement, body)).toBe(false);
    body.position.x = 1;
    expect(isFaceBoundsCurrent(capture, entry, body)).toBe(false);
    body.position.x = 0;
    body.visible = false;
    expect(isFaceBoundsCurrent(capture, entry, body)).toBe(false);
    entry.dispose();
    replacement.dispose();
  });
});
