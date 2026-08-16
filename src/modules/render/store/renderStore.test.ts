/*
 * The render store against the REAL mock client, not a hand-written fake: the
 * point of these tests is that the module's persistence actually goes through
 * `platform/documentState.ts` onto the same lane the app uses, so every
 * assertion below reads the slice back with `mockClient.getModuleState`.
 *
 * `setRenderDocumentStateService` is used only where a test needs to observe or
 * FAIL a write, never to replace the lane wholesale.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mockClient } from "@/ipc/mockClient";
import { createDocumentStateService, type DocumentStateService } from "@/platform";
import { RENDER_MODULE_ID } from "../manifest";
import { createMaterial } from "../model/material";
import { CURRENT_RENDER_SCHEMA_VERSION, createEmptyRenderState } from "../model/state";
import {
  renderStore,
  setRenderBodyLister,
  setRenderDocumentStateService,
} from "./renderStore";

const STEEL = createMaterial("Steel", { base: { base_metalness: 1 } });
const BRASS = createMaterial("Brass");

/** The slice as the backend holds it (never through the store). */
async function slice(): Promise<{ schemaVersion: number; payload: unknown } | null> {
  const state = await mockClient.getModuleState(RENDER_MODULE_ID);
  return state === null ? null : { schemaVersion: state.schemaVersion, payload: state.payload };
}

async function payload(): Promise<Record<string, unknown>> {
  return ((await slice())?.payload ?? {}) as Record<string, unknown>;
}

beforeEach(async () => {
  setRenderDocumentStateService(null);
  setRenderBodyLister(null);
  renderStore.getState().reset();
  await mockClient.closeDocument(); // drops every module slice
});

afterEach(() => {
  setRenderDocumentStateService(null);
  setRenderBodyLister(null);
  vi.restoreAllMocks();
});

describe("hydrate", () => {
  it("reads an absent slice as an empty state", async () => {
    await renderStore.getState().hydrate();

    const s = renderStore.getState();
    expect(s.hydrated).toBe(true);
    expect(s.readOnly).toBe(false);
    expect(s.warnings).toEqual([]);
    expect(s.state).toEqual(createEmptyRenderState());
  });

  it("surfaces tolerant-parse warnings without losing the rest of the state", async () => {
    await createDocumentStateService(mockClient, RENDER_MODULE_ID).write({
      schemaVersion: CURRENT_RENDER_SCHEMA_VERSION,
      payload: {
        schemaVersion: 1,
        openPbrRevision: "1.1.1",
        library: { [STEEL.id]: STEEL, broken: { id: "not-a-material-id", name: "x", base: {} } },
        assignments: { bodies: { body1: STEEL.id }, faces: {} },
      },
    });

    await renderStore.getState().hydrate();

    const s = renderStore.getState();
    expect(s.readOnly).toBe(false);
    expect(Object.keys(s.state.library)).toEqual([STEEL.id]);
    expect(s.state.assignments.bodies).toEqual({ body1: STEEL.id });
    expect(s.warnings).toHaveLength(1);
    expect(s.warnings[0]).toContain("broken");
  });

  it("holds a future-schema slice READ-ONLY and refuses every write", async () => {
    const foreign = { schemaVersion: 2, library: { future: "bytes" } };
    await createDocumentStateService(mockClient, RENDER_MODULE_ID).write({
      schemaVersion: 2,
      payload: foreign,
    });
    await renderStore.getState().hydrate();
    expect(renderStore.getState().readOnly).toBe(true);
    expect(renderStore.getState().warnings[0]).toContain("schemaVersion");

    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const setModuleState = vi.spyOn(mockClient, "setModuleState");

    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().assignBody("body1", STEEL.id);
    await renderStore.getState().assignFace("el_1", STEEL.id);
    await renderStore.getState().removeMaterial(STEEL.id);
    await renderStore.getState().clearFaceOverrides(["el_1"]);

    // The whole point: nothing reached the backend, so the foreign bytes are
    // exactly as they were.
    expect(setModuleState).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalled();
    expect(await payload()).toEqual(foreign);
    expect(renderStore.getState().state).toEqual(createEmptyRenderState());
  });
});

describe("mutating actions persist through the document-state seam", () => {
  it("upsertMaterial writes the library at the current schema version", async () => {
    await renderStore.getState().hydrate();
    await renderStore.getState().upsertMaterial(STEEL);

    expect((await slice())?.schemaVersion).toBe(CURRENT_RENDER_SCHEMA_VERSION);
    expect((await payload()).library).toEqual({ [STEEL.id]: STEEL });
    expect(renderStore.getState().state.library[STEEL.id]).toEqual(STEEL);
  });

  it("assignBody and assignFace persist, and null unassigns", async () => {
    await renderStore.getState().hydrate();
    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().assignBody("body1", STEEL.id);
    await renderStore.getState().assignFace("el_7", STEEL.id);

    expect((await payload()).assignments).toEqual({
      bodies: { body1: STEEL.id },
      faces: { el_7: STEEL.id },
    });

    await renderStore.getState().assignBody("body1", null);
    await renderStore.getState().assignFace("el_7", null);

    expect((await payload()).assignments).toEqual({ bodies: {}, faces: {} });
  });

  it("clearFaceOverrides drops several overrides in one write", async () => {
    await renderStore.getState().hydrate();
    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().assignFace("el_1", STEEL.id);
    await renderStore.getState().assignFace("el_2", STEEL.id);
    await renderStore.getState().assignFace("el_3", STEEL.id);

    const setModuleState = vi.spyOn(mockClient, "setModuleState");
    await renderStore.getState().clearFaceOverrides(["el_1", "el_3"]);

    expect(setModuleState).toHaveBeenCalledTimes(1);
    expect(renderStore.getState().state.assignments.faces).toEqual({ el_2: STEEL.id });
  });

  it("removeMaterial cascades its assignments in ONE write", async () => {
    await renderStore.getState().hydrate();
    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().upsertMaterial(BRASS);
    await renderStore.getState().assignBody("body1", STEEL.id);
    await renderStore.getState().assignBody("body2", BRASS.id);
    await renderStore.getState().assignFace("el_1", STEEL.id);

    const setModuleState = vi.spyOn(mockClient, "setModuleState");
    await renderStore.getState().removeMaterial(STEEL.id);

    // One write, or the document briefly (and after a crash, permanently)
    // references a material that no longer exists.
    expect(setModuleState).toHaveBeenCalledTimes(1);
    expect((await payload()).library).toEqual({ [BRASS.id]: BRASS });
    expect((await payload()).assignments).toEqual({ bodies: { body2: BRASS.id }, faces: {} });
  });

  it("an emptied state CLEARS the slice instead of writing an empty one", async () => {
    await renderStore.getState().hydrate();
    await renderStore.getState().upsertMaterial(STEEL);
    expect(await slice()).not.toBeNull();

    await renderStore.getState().removeMaterial(STEEL.id);

    // ADR-0004 skip-if-empty: no slice at all, not a slice holding nothing.
    expect(await slice()).toBeNull();
    expect(await mockClient.listDocumentModules()).toEqual([]);
  });

  it("reportUnboundFaceOverrides is session-only and never written", async () => {
    await renderStore.getState().hydrate();
    renderStore.getState().reportUnboundFaceOverrides("body1", ["el_9"]);

    expect(renderStore.getState().unboundFaceOverrides).toEqual({ body1: ["el_9"] });
    expect(await slice()).toBeNull();

    renderStore.getState().reportUnboundFaceOverrides("body1", []);
    expect(renderStore.getState().unboundFaceOverrides).toEqual({});
  });
});

describe("write failure", () => {
  it("re-hydrates from the backend rather than keeping the optimistic value", async () => {
    const real = createDocumentStateService(mockClient, RENDER_MODULE_ID);
    await real.write({
      schemaVersion: CURRENT_RENDER_SCHEMA_VERSION,
      payload: {
        schemaVersion: 1,
        openPbrRevision: "1.1.1",
        library: { [BRASS.id]: BRASS },
        assignments: { bodies: {}, faces: {} },
      },
    });
    await renderStore.getState().hydrate();

    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const failing: DocumentStateService = {
      read: real.read,
      write: () => Promise.reject(new Error("backend refused")),
      clear: () => Promise.reject(new Error("backend refused")),
    };
    setRenderDocumentStateService(failing);

    await renderStore.getState().upsertMaterial(STEEL);

    expect(warn).toHaveBeenCalled();
    // The optimistic Steel is gone: the backend never took it, and the store
    // asked rather than assumed.
    expect(Object.keys(renderStore.getState().state.library)).toEqual([BRASS.id]);
  });
});

describe("dangling bodies", () => {
  it("classifies assignments whose body the document no longer has", async () => {
    setRenderBodyLister(() => ["body1"]);
    await renderStore.getState().hydrate();
    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().assignBody("body1", STEEL.id);
    await renderStore.getState().assignBody("bodyGone", STEEL.id);

    expect(renderStore.getState().danglingBodies).toEqual(["bodyGone"]);
    // NEVER deleted — a dangling assignment is a report, not a cleanup (H5-B).
    expect((await payload()).assignments).toMatchObject({
      bodies: { body1: STEEL.id, bodyGone: STEEL.id },
    });

    setRenderBodyLister(() => ["body1", "bodyGone"]);
    renderStore.getState().recomputeDangling();
    expect(renderStore.getState().danglingBodies).toEqual([]);
  });

  it("reports nothing when the body set is unknown", async () => {
    setRenderBodyLister(() => null);
    await renderStore.getState().hydrate();
    await renderStore.getState().assignBody("bodyGone", STEEL.id);

    expect(renderStore.getState().danglingBodies).toEqual([]);
  });
});

describe("backend undo", () => {
  it("an assignment written by the store is undone by the backend's undo", async () => {
    await renderStore.getState().hydrate();
    await renderStore.getState().upsertMaterial(STEEL);
    await renderStore.getState().assignBody("body1", STEEL.id);

    const seen: unknown[] = [];
    const stop = mockClient.onDocumentChanged((c) => seen.push(c));
    try {
      await mockClient.undo();
    } finally {
      stop();
    }

    // The write went through the transaction path, so undo reverted it in the
    // document — the store is now the STALE copy, which is exactly why the
    // module re-hydrates on the document-changed this emitted.
    expect((await payload()).assignments).toEqual({ bodies: {}, faces: {} });
    expect(seen).toHaveLength(1);

    await renderStore.getState().hydrate();
    expect(renderStore.getState().state.assignments.bodies).toEqual({});
    expect(renderStore.getState().state.library[STEEL.id]).toEqual(STEEL);
  });
});

describe("reset", () => {
  it("forgets everything in memory and writes NOTHING", async () => {
    await renderStore.getState().hydrate();
    await renderStore.getState().upsertMaterial(STEEL);

    const setModuleState = vi.spyOn(mockClient, "setModuleState");
    renderStore.getState().reset();

    expect(setModuleState).not.toHaveBeenCalled();
    expect(renderStore.getState().hydrated).toBe(false);
    expect(renderStore.getState().state).toEqual(createEmptyRenderState());
    // The document still has its materials — closing a file must not delete them.
    expect((await payload()).library).toEqual({ [STEEL.id]: STEEL });
  });

  it("discards a hydrate that was in flight for the outgoing document", async () => {
    await createDocumentStateService(mockClient, RENDER_MODULE_ID).write({
      schemaVersion: CURRENT_RENDER_SCHEMA_VERSION,
      payload: {
        schemaVersion: 1,
        openPbrRevision: "1.1.1",
        library: { [STEEL.id]: STEEL },
        assignments: { bodies: {}, faces: {} },
      },
    });

    const pending = renderStore.getState().hydrate();
    renderStore.getState().reset();
    await pending;

    expect(renderStore.getState().state).toEqual(createEmptyRenderState());
    expect(renderStore.getState().hydrated).toBe(false);
  });
});
