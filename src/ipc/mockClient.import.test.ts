/*
 * mockClient — STEP import (STEP-IMPORT WP-A, mock lane).
 *
 * There is no STEP reader here, so the mock FABRICATES what the real projection
 * would publish: one new body, an `ImportStep` history row, a body name, a bumped
 * revision, and a `document-changed` so the viewport ingests the mesh. These
 * tests pin that fabrication because the whole e2e import flow — and therefore
 * every UI assertion about import — rests on it.
 *
 * Both lanes share it: `insertStep()` appends to the OPEN document (the File-menu
 * lane), `importStep(path)` opens a new one from a file (the start-screen lane).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  emitMockDocumentChanged,
  mockClient,
  resetMockDocument,
  setMockLatency,
} from "./mockClient";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import type { DocumentChange } from "./types";

const doc = () => documentStore.getState();

describe("mockClient STEP import", () => {
  beforeEach(() => {
    setMockLatency(0);
    documentStore.setState(seedMockDocument());
    resetMockDocument();
  });
  afterEach(() => setMockLatency(0));

  it("insertStep appends one body, an ImportStep row, and bumps the revision", async () => {
    const before = doc();
    const bodiesBefore = Object.keys(before.bodies).length;
    const featuresBefore = before.features.length;
    const revisionBefore = before.revision;

    const res = await mockClient.insertStep();

    expect(res).not.toBeNull();
    expect(res!.changedBodies).toHaveLength(1);
    expect(res!.removedBodies).toEqual([]);
    expect(res!.opLabel).toBe("Import");
    expect(res!.revision).toBeGreaterThan(revisionBefore);

    const row = res!.features[res!.features.length - 1];
    expect(row.opType).toBe("ImportStep");
    expect(row.label).toBe("Import");
    // Interim bucket — the backend has no import `kind`, so it folds into boolean.
    expect(row.kind).toBe("boolean");
    expect(row.status).toBe("ok");

    // The projection store carries the SAME facts (the mock plays the backend's
    // part: there is no projection-updated event to hydrate from).
    const after = doc();
    expect(Object.keys(after.bodies)).toHaveLength(bodiesBefore + 1);
    expect(after.features).toHaveLength(featuresBefore + 1);
    expect(after.revision).toBe(res!.revision);
    expect(after.dirty).toBe(true);
  });

  it("names the fabricated body 'Imported N', counting up per import", async () => {
    const first = await mockClient.insertStep();
    const second = await mockClient.insertStep();

    expect(doc().bodies[first!.changedBodies[0].bodyId].name).toBe("Imported 1");
    expect(doc().bodies[second!.changedBodies[0].bodyId].name).toBe("Imported 2");
    // Distinct bodies, not one re-emitted.
    expect(first!.changedBodies[0].bodyId).not.toBe(second!.changedBodies[0].bodyId);
  });

  it("fires document-changed so the viewport ingests the imported mesh", async () => {
    const seen: DocumentChange[] = [];
    const off = mockClient.onDocumentChanged((c) => seen.push(c));
    try {
      const res = await mockClient.insertStep();
      expect(seen).toHaveLength(1);
      expect(seen[0].revision).toBe(res!.revision);
      expect(seen[0].changedBodies.map((b) => b.bodyId)).toEqual([res!.changedBodies[0].bodyId]);

      // The advertised body is fetchable as a real MESH1 blob (offset box).
      const mesh = await mockClient.getBodyMesh(res!.changedBodies[0].bodyId, "coarse");
      expect(mesh.byteLength).toBeGreaterThan(64);
    } finally {
      off();
    }
  });

  it("the start-screen lane runs the same fabrication and returns a snapshot", async () => {
    const featuresBefore = doc().features.length;

    const snap = await mockClient.importStep("/Users/andrej/CAD/Widget.step");

    expect(snap.title).toBe("Widget");
    expect(doc().features).toHaveLength(featuresBefore + 1);
    expect(doc().features[doc().features.length - 1].opType).toBe("ImportStep");
    expect(Object.values(doc().bodies).some((b) => b.name === "Imported 1")).toBe(true);
  });

  it("the project lane returns a snapshot with the appended import row", async () => {
    const featuresBefore = doc().features.length;

    const snap = await mockClient.importProject();

    // `null` is the user-cancelled-the-picker case on the real lane; the mock
    // always fabricates, so a null here is the mock breaking its own contract.
    if (!snap) throw new Error("mockClient.importProject() returned null");
    expect(snap.title).toBe("ImportedProject");
    // Same fabrication as an in-document STEP: the imported body + history row
    // land in the projection through the shared `importStepAndEmit` path.
    expect(doc().features).toHaveLength(featuresBefore + 1);
    expect(doc().features[doc().features.length - 1].opType).toBe("ImportStep");
    expect(Object.values(doc().bodies).some((b) => b.name === "Imported 1")).toBe(true);
  });

  it("importFileDialog returns a unified OneCAD/STEP pick", async () => {
    expect(await mockClient.importFileDialog()).toBe("/Users/andrej/CAD/Projects/Imported.onecad");
  });

  it("resetMockDocument restarts the import numbering", async () => {
    await mockClient.insertStep();
    documentStore.setState(seedMockDocument());
    resetMockDocument();

    const res = await mockClient.insertStep();
    expect(doc().bodies[res!.changedBodies[0].bodyId].name).toBe("Imported 1");
  });

  it("keeps the imported name across a later committed op's projection re-assert", async () => {
    const res = await mockClient.insertStep();
    const bodyId = res!.changedBodies[0].bodyId;

    // A committed op re-asserts the mock's owned metadata over the store — the
    // exact path that used to rewrite a locally-set name back to "Body N".
    emitMockDocumentChanged({ revision: res!.revision, changedBodies: [], removedBodies: [] });
    await mockClient.applyEditCommand({ cmd: "renameBody", body: "body1", name: "Base" });

    expect(doc().bodies[bodyId].name).toBe("Imported 1");
  });
});
