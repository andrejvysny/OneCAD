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
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
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
  viewportStore.setState({ isolatedBodyIds: null });
  settingsStore.setState({ displayMode: "shadedEdges" });
});

/** The scene group for a body (undefined when it has no scene object). */
function bodyGroup(
  engine: ReturnType<typeof fakeEngine>,
  bodyId: string,
): THREE.Group | undefined {
  return engine.bodiesRoot.children.find((c) => c.userData.bodyId === bodyId) as
    | THREE.Group
    | undefined;
}

/** [faceVisible, edgeVisible] for a loaded body. */
function childVisibility(engine: ReturnType<typeof fakeEngine>, bodyId: string): boolean[] {
  const group = bodyGroup(engine, bodyId)!;
  return ["face", "edge"].map(
    (kind) => group.children.find((c) => c.userData.kind === kind)!.visible,
  );
}

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

/*
 * W3 display mode — MeshIngest owns applying it, because it owns the
 * bodyId→handle map. Persisted (settingsStore), never a document write.
 */
describe("MeshIngest display mode", () => {
  it("applies the CURRENT mode to a body loaded later", async () => {
    settingsStore.setState({ displayMode: "wireframe" });
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    emit(changed("body1"));
    await tick();

    expect(childVisibility(engine, "body1")).toEqual([false, true]);
  });

  it("re-applies to every live body when the store flips", async () => {
    setBodies({ body1: true, body2: true });
    const engine = fakeEngine();
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    expect(childVisibility(engine, "body1")).toEqual([true, true]); // shadedEdges

    settingsStore.setState({ displayMode: "wireframe" });
    expect(childVisibility(engine, "body1")).toEqual([false, true]);
    expect(childVisibility(engine, "body2")).toEqual([false, true]);

    settingsStore.setState({ displayMode: "shaded" });
    expect(childVisibility(engine, "body1")).toEqual([true, false]);
    expect(engine.invalidate).toHaveBeenCalled();
  });

  it("detach unsubscribes — a later mode flip does not touch the scene", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();
    const group = bodyGroup(engine, "body1")!;

    ingest.detach();
    ingest = null;

    settingsStore.setState({ displayMode: "wireframe" });
    expect(group.children.every((c) => c.visible)).toBe(true);
  });
});

/*
 * W3 isolate — a TRANSIENT mask ANDed with the document's own `visible` fact.
 * The document is never written, so the tree eye keeps its meaning throughout.
 */
describe("MeshIngest isolation", () => {
  it("hides the bodies outside the isolate set and restores them on exit", async () => {
    setBodies({ body1: true, body2: true });
    const engine = fakeEngine();
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    viewportStore.setState({ isolatedBodyIds: ["body1"] });
    expect(bodyGroup(engine, "body1")!.visible).toBe(true);
    expect(bodyGroup(engine, "body2")!.visible).toBe(false);

    viewportStore.setState({ isolatedBodyIds: null });
    expect(bodyGroup(engine, "body2")!.visible).toBe(true);
  });

  it("a DOC-hidden body stays hidden inside the isolate set — and after exit", async () => {
    setBodies({ body1: true, body2: false });
    const engine = fakeEngine();
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    // body2 was never loaded (doc-hidden), so isolating it must not resurrect it.
    viewportStore.setState({ isolatedBodyIds: ["body1", "body2"] });
    await tick();
    expect(bodyGroup(engine, "body2")).toBeUndefined();

    viewportStore.setState({ isolatedBodyIds: null });
    await tick();
    expect(bodyGroup(engine, "body2")).toBeUndefined();
  });

  it("a tree-eye SHOW inside an isolate set loads the body but keeps it masked", async () => {
    setBodies({ body1: true, body2: false });
    const engine = fakeEngine();
    const { client, getMesh } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    viewportStore.setState({ isolatedBodyIds: ["body1"] });
    setBodies({ body1: true, body2: true }); // eye flips body2 on, still isolated away
    await tick();

    expect(getMesh).toHaveBeenCalledWith("body2", "coarse"); // fetched (kept fresh)
    expect(bodyGroup(engine, "body2")!.visible).toBe(false); // but masked

    viewportStore.setState({ isolatedBodyIds: null });
    expect(bodyGroup(engine, "body2")!.visible).toBe(true);
  });

  it("lazy-loads a body that becomes effectively visible only on exit", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    // A body the document gains while isolation masks it (no load: not effective).
    viewportStore.setState({ isolatedBodyIds: ["body1"] });
    setBodies({ body1: true, body2: false });
    await tick();
    expect(bodyGroup(engine, "body2")).toBeUndefined();

    // It becomes doc-visible while still masked, then isolation ends.
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "body1", visible: true },
        body2: { id: "body2", name: "body2", visible: true },
      },
    });
    await tick();
    viewportStore.setState({ isolatedBodyIds: null });
    await tick();
    expect(bodyGroup(engine, "body2")!.visible).toBe(true);
  });

  it("a body loaded WHILE isolated arrives masked", async () => {
    setBodies({ body1: true, body2: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    viewportStore.setState({ isolatedBodyIds: ["body1"] });
    emit(changed("body2")); // a regen republishes body2's mesh
    await tick();

    expect(bodyGroup(engine, "body2")!.visible).toBe(false);
  });

  it("detach unsubscribes — a later isolate flip does not touch the scene", async () => {
    setBodies({ body1: true, body2: true });
    const engine = fakeEngine();
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();
    const group = bodyGroup(engine, "body2")!;

    ingest.detach();
    ingest = null;

    viewportStore.setState({ isolatedBodyIds: ["body1"] });
    expect(group.visible).toBe(true);
  });
});
