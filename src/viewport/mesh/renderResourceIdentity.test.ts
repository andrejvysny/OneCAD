/*
 * Resource identity (VP-HARDENING spec §8.1). The signature answers "is this
 * the same NAMED topology" and nothing else: moving every vertex must not
 * change it, and renaming one face must.
 */
import { describe, it, expect } from "vitest";
import { deriveIdentity, identityKey, topologySignature } from "./renderResourceIdentity";
import { makeBodyMeshViewFixture, type FixtureFace } from "@/test/fixtures/bodyMeshView";

/** Two triangles per face, sharing a 4-vertex quad, so faces can be renamed freely. */
const QUAD: readonly number[] = [0, 0, 0, 10, 0, 0, 10, 10, 0, 0, 10, 0];
const FACES: readonly FixtureFace[] = [
  { id: "f:0", triangles: [[0, 1, 2]] },
  { id: "f:1", triangles: [[0, 2, 3]] },
];

describe("topologySignature", () => {
  it("ignores POSITIONS — a re-tessellation of the same named topology matches", () => {
    const a = makeBodyMeshViewFixture({ positions: QUAD, faces: FACES });
    const b = makeBodyMeshViewFixture({
      positions: [0, 0, 0, 10.5, 0, 0, 10.5, 10.5, 0, 0, 10.5, 0],
      faces: FACES,
    });

    expect(b.positions).not.toEqual(a.positions);
    expect(topologySignature(b)).toBe(topologySignature(a));
  });

  it("changes when a face id is RENAMED", () => {
    const a = makeBodyMeshViewFixture({ positions: QUAD, faces: FACES });
    const b = makeBodyMeshViewFixture({
      positions: QUAD,
      faces: [FACES[0], { id: "f:7", triangles: [[0, 2, 3]] }],
    });

    expect(b.faceRanges).toEqual(a.faceRanges); // same incidence, different name
    expect(topologySignature(b)).not.toBe(topologySignature(a));
  });

  it("changes when a ZERO-COUNT face is added (a face that owns no triangles)", () => {
    const a = makeBodyMeshViewFixture({ positions: QUAD, faces: FACES });
    const b = makeBodyMeshViewFixture({
      positions: QUAD,
      faces: [...FACES, { id: "f:2", triangles: [] }],
    });

    expect(b.triangleCount).toBe(a.triangleCount); // the geometry is identical
    expect(topologySignature(b)).not.toBe(topologySignature(a));
  });

  it("is a 16-hex-digit 64-bit value", () => {
    expect(topologySignature(makeBodyMeshViewFixture())).toMatch(/^[0-9a-f]{16}$/);
  });

  it("changes when the ids stop being TopoKeys and become ElementIds", () => {
    const a = makeBodyMeshViewFixture({ positions: QUAD, faces: FACES });
    const b = makeBodyMeshViewFixture({ positions: QUAD, faces: FACES, idsHaveElementIds: true });
    expect(topologySignature(b)).not.toBe(topologySignature(a));
  });
});

describe("deriveIdentity", () => {
  const view = makeBodyMeshViewFixture({ positions: QUAD, faces: FACES, lod: 2 });
  const provenance = {
    documentId: "doc-1",
    runtimeSession: "run-1",
    snapshotId: 7,
    generation: 3,
  };

  it("carries the publication fence, the revision, and the quality tier", () => {
    const id = deriveIdentity({ view, bodyId: "body1", provenance, meshRev: 5 });
    expect(id).toMatchObject({
      documentId: "doc-1",
      runtimeSession: "run-1",
      snapshotId: 7,
      generation: 3,
      bodyId: "body1",
      geometryRevision: 5,
      qualityKey: "2",
      meshFormatVersion: 1,
    });
  });

  it("falls back to \"\"/0 for an entry with no publication (bootstrap, preview)", () => {
    const id = deriveIdentity({ view, bodyId: "body1", meshRev: 1 });
    expect(id.documentId).toBe("");
    expect(id.runtimeSession).toBe("");
    expect(id.snapshotId).toBe(0);
    expect(id.generation).toBe(0);
  });

  it("keys two snapshots of an IDENTICAL topology apart", () => {
    const first = deriveIdentity({ view, bodyId: "body1", provenance, meshRev: 1 });
    const second = deriveIdentity({
      view,
      bodyId: "body1",
      provenance: { ...provenance, snapshotId: 8 },
      meshRev: 2,
    });
    expect(second.topologySignature).toBe(first.topologySignature);
    expect(identityKey(second)).not.toBe(identityKey(first));
  });

  it("identityKey is the canonical |-joined serialization", () => {
    const id = deriveIdentity({ view, bodyId: "body1", provenance, meshRev: 5 });
    expect(identityKey(id)).toBe(
      `doc-1|run-1|7|3|body1|${id.topologySignature}|5|2|1`,
    );
  });
});
