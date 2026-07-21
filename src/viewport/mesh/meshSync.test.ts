/*
 * MeshIngest store wiring: document-changed → fetch visible bodies → registry
 * swap + scene object; removal + visibility lazy-load; detach empties the
 * registry. THREE is real (jsdom-safe geometry), the engine + client are fakes.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as THREE from "three";
import { MeshIngest } from "./meshSync";
import * as reg from "./meshRegistry";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { documentStore } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import type { CadClient } from "@/ipc/client";
import type { DocumentChange } from "@/ipc/types";
import type { ViewportEngine } from "../engine/ViewportEngine";

const tick = () => new Promise((r) => setTimeout(r, 0));

function fakeEngine() {
  const bodiesRoot = new THREE.Group();
  return {
    bodiesRoot,
    invalidate: vi.fn(),
    refreshHighlights: vi.fn(),
    setHighlightState: vi.fn(),
  } as unknown as ViewportEngine & {
    bodiesRoot: THREE.Group;
    invalidate: ReturnType<typeof vi.fn>;
    refreshHighlights: ReturnType<typeof vi.fn>;
    setHighlightState: ReturnType<typeof vi.fn>;
  };
}

function fakeClient(getMesh = vi.fn(async () => makeBoxMesh())) {
  const listeners = new Set<(c: DocumentChange) => void>();
  const client = {
    getBodyMesh: getMesh,
    onDocumentChanged: (cb: (c: DocumentChange) => void) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  } as unknown as CadClient;
  return { client, getMesh, emit: (c: DocumentChange) => listeners.forEach((l) => l(c)) };
}

function setBodies(bodies: Record<string, boolean>) {
  const full: Record<string, { id: string; name: string; visible: boolean }> = {};
  for (const [id, visible] of Object.entries(bodies)) full[id] = { id, name: id, visible };
  documentStore.setState({ bodies: full });
}

const changed = (bodyId: string): DocumentChange => ({
  revision: 1,
  changedBodies: [{ bodyId, meshKey: `${bodyId}:coarse:1` }],
  removedBodies: [],
});

/** Pull the shared face material off the first loaded body's scene group. */
function faceMaterial(engine: ReturnType<typeof fakeEngine>): THREE.MeshStandardMaterial {
  const group = engine.bodiesRoot.children[0] as THREE.Group;
  const mesh = group.children.find((c) => c.userData.kind === "face") as THREE.Mesh;
  return mesh.material as THREE.MeshStandardMaterial;
}

let ingest: MeshIngest | null = null;

beforeEach(() => {
  reg.disposeAll();
  reg.__resetRegistryForTests();
});
afterEach(() => {
  ingest?.detach();
  ingest = null;
  toolStore.getState().setMode("model"); // dimming tests flip this; keep the file isolated
});

describe("MeshIngest onDocumentChanged", () => {
  it("fetches + swaps + adds a scene object for a changed, visible body", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, getMesh, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    emit(changed("body1"));
    await tick();

    expect(getMesh).toHaveBeenCalledWith("body1", "coarse");
    expect(reg.getEntry("body1")).toBeDefined();
    expect(engine.bodiesRoot.children.length).toBe(1);
    expect(engine.refreshHighlights).toHaveBeenCalled();
  });

  it("skips an invisible changed body (nothing fetched)", async () => {
    setBodies({ body1: false });
    const engine = fakeEngine();
    const { client, getMesh, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    emit(changed("body1"));
    await tick();

    expect(getMesh).not.toHaveBeenCalled();
    expect(reg.getEntry("body1")).toBeUndefined();
    expect(engine.bodiesRoot.children.length).toBe(0);
  });

  it("drops a removed body from registry + scene", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    emit(changed("body1"));
    await tick();
    expect(reg.getEntry("body1")).toBeDefined();

    emit({ revision: 2, changedBodies: [], removedBodies: ["body1"] });
    await tick();
    expect(reg.getEntry("body1")).toBeUndefined();
    expect(engine.bodiesRoot.children.length).toBe(0);
  });
});

describe("MeshIngest initial sweep (bodies already in the store at attach)", () => {
  it("loads only the visible pre-seeded bodies on attach, before any event fires", async () => {
    setBodies({ body1: true, body2: false });
    const engine = fakeEngine();
    const { client, getMesh } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    await tick();

    expect(getMesh).toHaveBeenCalledTimes(1);
    expect(getMesh).toHaveBeenCalledWith("body1", "coarse");
    expect(reg.getEntry("body1")).toBeDefined();
    expect(reg.getEntry("body2")).toBeUndefined();
    expect(engine.bodiesRoot.children.length).toBe(1);
  });
});

describe("MeshIngest empty mesh response (not yet regenerated)", () => {
  it("does not swap/mutate the scene when getBodyMesh resolves an empty buffer, and does not throw", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient(vi.fn(async () => new ArrayBuffer(0)));
    ingest = new MeshIngest();
    ingest.attach(engine, client); // initial sweep also resolves empty — same guard

    emit(changed("body1"));
    await tick();

    expect(reg.getEntry("body1")).toBeUndefined();
    expect(engine.bodiesRoot.children.length).toBe(0);
    expect(engine.refreshHighlights).not.toHaveBeenCalled();
  });
});

describe("MeshIngest visibility + detach", () => {
  it("lazy-loads a body the first time it becomes visible", async () => {
    setBodies({ body1: false });
    const engine = fakeEngine();
    const { client, getMesh, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    emit(changed("body1"));
    await tick();
    expect(getMesh).not.toHaveBeenCalled();

    setBodies({ body1: true }); // flip visible → lazy fetch
    await tick();
    expect(getMesh).toHaveBeenCalledWith("body1", "coarse");
    expect(reg.getEntry("body1")).toBeDefined();
  });

  it("detach clears the scene, disposes the registry, and clears highlights", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    emit(changed("body1"));
    await tick();
    expect(reg.registrySize()).toBe(1);

    ingest.detach();
    ingest = null;
    expect(reg.registrySize()).toBe(0);
    expect(engine.bodiesRoot.children.length).toBe(0);
    expect(engine.setHighlightState).toHaveBeenCalledWith(null, []);
  });
});

describe("MeshIngest sketch-mode dimming", () => {
  it("dims the shared face material on mode -> sketch (edge material untouched)", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit(changed("body1"));
    await tick();

    const mat = faceMaterial(engine);
    const versionBefore = mat.version;

    toolStore.getState().setMode("sketch");

    expect(mat.transparent).toBe(true);
    expect(mat.opacity).toBe(0.35);
    expect(mat.version).toBeGreaterThan(versionBefore); // needsUpdate was set
    expect(engine.invalidate).toHaveBeenCalled();
  });

  it("restores the EXACT prior face material state on mode -> model", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit(changed("body1"));
    await tick();

    // Unusual priors — deliberately far from MeshStandardMaterial defaults, so a
    // restore that hardcodes reset values (rather than replaying the save) fails.
    const mat = faceMaterial(engine);
    mat.transparent = true;
    mat.opacity = 0.62;
    mat.depthWrite = false;

    toolStore.getState().setMode("sketch");
    expect(mat.opacity).toBe(0.35); // dimmed

    toolStore.getState().setMode("model");
    expect(mat.transparent).toBe(true);
    expect(mat.opacity).toBe(0.62);
    expect(mat.depthWrite).toBe(false);
  });

  it("applies the dim immediately when attach() happens while already in sketch mode", async () => {
    toolStore.getState().setMode("sketch");
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit(changed("body1"));
    await tick();

    expect(faceMaterial(engine).transparent).toBe(true);
    expect(faceMaterial(engine).opacity).toBe(0.35);
  });

  it("detach unsubscribes from toolStore — a later mode flip is a no-op", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit(changed("body1"));
    await tick();

    const mat = faceMaterial(engine);
    const opacityBefore = mat.opacity;
    const invalidateCallsBeforeDetach = engine.invalidate.mock.calls.length;

    ingest.detach();
    ingest = null;

    expect(() => toolStore.getState().setMode("sketch")).not.toThrow();
    expect(mat.opacity).toBe(opacityBefore); // detached ingest no longer touches it
    expect(engine.invalidate.mock.calls.length).toBe(invalidateCallsBeforeDetach);
  });
});
