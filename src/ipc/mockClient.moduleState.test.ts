/*
 * Module-owned document state on the mock lane: undo/redo and per-document
 * scoping.
 *
 * `api::set_module_state` goes through the core `EditCommand::SetModuleState`
 * transaction, which records an inverse — so a module's write is undoable in the
 * app. The mock is where every frontend consumer of that fact is proved, so it
 * has to model it too; a mock that quietly made module state un-undoable would
 * make each of those tests vacuous.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  getMockLatency,
  mockClient,
  resetMockDocument,
  setMockLatency,
} from "./mockClient";
import type { DocumentChange } from "./types";

const FOO = "com.example.foo";
const BAR = "com.example.bar";

const realLatency = getMockLatency();

beforeEach(async () => {
  setMockLatency(0);
  resetMockDocument();
  await mockClient.closeDocument();
});

afterEach(() => setMockLatency(realLatency));

async function payloadOf(moduleId: string): Promise<unknown> {
  return (await mockClient.getModuleState(moduleId))?.payload ?? null;
}

describe("module state is undoable", () => {
  it("undo restores the prior slice, redo puts the new one back", async () => {
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: "first" } });
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: "second" } });

    await mockClient.undo();
    expect(await payloadOf(FOO)).toEqual({ v: "first" });

    await mockClient.redo();
    expect(await payloadOf(FOO)).toEqual({ v: "second" });
  });

  it("undoing the FIRST write removes the slice entirely", async () => {
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: 1 } });

    await mockClient.undo();

    expect(await mockClient.getModuleState(FOO)).toBeNull();
    expect(await mockClient.listDocumentModules()).toEqual([]);
  });

  it("undoing a CLEAR brings the slice back", async () => {
    await mockClient.setModuleState(FOO, { schemaVersion: 3, payload: { v: 1 } });
    await mockClient.setModuleState(FOO, null);
    expect(await mockClient.getModuleState(FOO)).toBeNull();

    await mockClient.undo();

    expect(await mockClient.getModuleState(FOO)).toEqual({
      moduleId: FOO,
      schemaVersion: 3,
      payload: { v: 1 },
    });
  });

  it("leaves another module's slice alone", async () => {
    await mockClient.setModuleState(BAR, { schemaVersion: 1, payload: "bar" });
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: "foo" });

    await mockClient.undo();

    expect(await payloadOf(FOO)).toBeNull();
    expect(await payloadOf(BAR)).toBe("bar");
  });

  it("emits document-changed on undo, which is what a module re-hydrates on", async () => {
    const seen: DocumentChange[] = [];
    const stop = mockClient.onDocumentChanged((c) => seen.push(c));
    try {
      await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: 1 } });
      // The WRITE emits nothing, mirroring the real backend: module state is not
      // part of the projection and no body moved.
      expect(seen).toEqual([]);

      await mockClient.undo();
      expect(seen).toHaveLength(1);
      expect(seen[0].changedBodies).toEqual([]);
    } finally {
      stop();
    }
  });

  it("a module write drops the redo stack, like any other edit", async () => {
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: 1 } });
    await mockClient.undo();
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: 2 } });

    await mockClient.redo(); // nothing to redo — the branch was abandoned
    expect(await payloadOf(FOO)).toEqual({ v: 2 });
  });
});

describe("module state is per-document", () => {
  it("a new document starts with none", async () => {
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: 1 } });
    await mockClient.newDocument();
    expect(await mockClient.getModuleState(FOO)).toBeNull();
  });

  it("opening another document starts with none", async () => {
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: 1 } });
    await mockClient.openDocument("/Users/andrej/CAD/Projects/Bracket.onecad");
    expect(await mockClient.getModuleState(FOO)).toBeNull();
  });

  it("closing drops every slice", async () => {
    await mockClient.setModuleState(FOO, { schemaVersion: 1, payload: { v: 1 } });
    await mockClient.setModuleState(BAR, { schemaVersion: 1, payload: { v: 2 } });
    await mockClient.closeDocument();
    expect(await mockClient.listDocumentModules()).toEqual([]);
  });
});
