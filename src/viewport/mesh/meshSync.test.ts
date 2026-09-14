/*
 * MeshIngest store wiring: document-changed → fetch visible bodies → registry
 * swap + scene object; removal + visibility lazy-load; detach empties the
 * registry. THREE is real (jsdom-safe geometry), the engine + client are fakes.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as THREE from "three";
import { MeshIngest, meshGeneration } from "./meshSync";
import * as reg from "./meshRegistry";
import { parseMeshPayload } from "./parseMeshPayload";
import * as admissionModule from "./meshAdmission";
import { validateMeshView } from "./validateMesh";
import { planMeshPreparation } from "./meshPreparationPlan";
import { makeBoxMesh, makeCylinderMesh } from "@/ipc/mockMeshes";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import { __resetLogForTests, logSnapshot } from "@/debug/log";
import type { CadClient } from "@/ipc/client";
import { promoteViewportPick, STALE_PICK_HINT } from "@/ipc/promote";
import type { DocumentChange } from "@/ipc/types";
import type { FrameSubmission, ViewportEngine } from "../engine/ViewportEngine";
import { HighlightLayer } from "../engine/HighlightLayer";

const tick = () => new Promise((r) => setTimeout(r, 0));

function fakeEngine() {
  const bodiesRoot = new THREE.Group();
  const afterRender = new Set<(frame: FrameSubmission) => void>();
  let submission = 0;
  /**
   * Submit one frame. By default it carries every body in `bodiesRoot` with the
   * provenance of its registry entry — what the real engine derives from the
   * DISPLAYED geometry (PR-11). Tests pass an explicit id list to model a
   * hidden/isolated body, or an override to model a mismatched publication.
   */
  const completeRender = (
    bodyIds?: readonly string[],
    provenanceOverride?: Record<string, reg.MeshProvenance | null>,
  ) => {
    const ids = bodyIds ?? bodiesRoot.children.map((c) => c.userData.bodyId as string).filter(Boolean);
    const rows = ids.map((bodyId) => ({
      bodyId,
      provenance: provenanceOverride && bodyId in provenanceOverride
        ? provenanceOverride[bodyId]
        : reg.getEntry(bodyId)?.provenance ?? null,
    }));
    const frame: FrameSubmission = {
      submission: ++submission,
      requestedRevision: submission,
      publication: null,
      displayedBodyIds: ids,
      displayedProvenance: rows,
    };
    [...afterRender].forEach((cb) => cb(frame));
  };
  return {
    bodiesRoot,
    invalidate: vi.fn(),
    refreshHighlights: vi.fn(),
    setHighlightState: vi.fn(),
    onAfterRender: (cb: (frame: FrameSubmission) => void) => {
      afterRender.add(cb);
      return () => afterRender.delete(cb);
    },
    completeRender,
  } as unknown as ViewportEngine & {
    bodiesRoot: THREE.Group;
    invalidate: ReturnType<typeof vi.fn>;
    refreshHighlights: ReturnType<typeof vi.fn>;
    setHighlightState: ReturnType<typeof vi.fn>;
    completeRender: typeof completeRender;
  };
}

function fakeClient(
  getMesh: ReturnType<typeof vi.fn> = vi.fn(async () => makeBoxMesh()),
  retained: DocumentChange | null = null,
) {
  if (documentStore.getState().documentId === undefined) {
    documentStore.setState({ documentId: "test-document", runtimeSession: "test-runtime" });
  }
  const listeners = new Set<(c: DocumentChange) => void>();
  const client = {
    getBodyMesh: getMesh,
    getCurrentMeshPublication: () => retained,
    onDocumentChanged: (cb: (c: DocumentChange) => void) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  } as unknown as CadClient;
  return {
    client,
    getMesh,
    emit: (c: DocumentChange) => {
      const document = documentStore.getState();
      const change = {
        ...c,
        documentId: c.documentId ?? document.documentId,
        runtimeSession: c.runtimeSession ?? document.runtimeSession,
      };
      listeners.forEach((listener) => listener(change));
    },
  };
}

function setBodies(bodies: Record<string, boolean>) {
  const full: Record<string, { id: string; name: string; visible: boolean }> = {};
  for (const [id, visible] of Object.entries(bodies)) full[id] = { id, name: id, visible };
  documentStore.setState({ bodies: full });
}

const changed = (bodyId: string): DocumentChange => ({
  revision: 1,
  changedBodies: [{ bodyId, meshKey: `${bodyId}:fine:1` }],
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
  it("strictly parses normative mesh generations including split body ids", () => {
    expect(meshGeneration("body_uuid:split:4:fine:27")).toBe(27);
    expect(meshGeneration("body:fine:not-a-number")).toBeNull();
    expect(meshGeneration("body:fine:9007199254740992")).toBeNull();
    expect(meshGeneration("body:unknown:2")).toBeNull();
  });

  it("pins the fetch and installed entry to an authoritative publication generation", async () => {
    setBodies({ body1: true });
    documentStore.setState({ documentId: "doc-1", runtimeSession: "runtime-1", geometrySource: "live" });
    const engine = fakeEngine();
    const { client, getMesh, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    emit({ ...changed("body1"), documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 41 });
    await tick();

    expect(getMesh).toHaveBeenLastCalledWith("body1", "fine", 1, "runtime-1");
    expect(reg.getEntry("body1")?.provenance).toEqual({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      snapshotId: 41,
      generation: 1,
    });
  });

  it("fails closed to an unpinned install for a malformed publication meshKey", async () => {
    setBodies({ body1: true });
    documentStore.setState({ documentId: "doc-1", geometrySource: "live" });
    const engine = fakeEngine();
    const { client, getMesh, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit({
      ...changed("body1"),
      documentId: "doc-1",
      snapshotId: 42,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:bad" }],
    });
    await tick();
    expect(getMesh).toHaveBeenLastCalledWith("body1", "fine");
    expect(reg.getEntry("body1")?.provenance).toBeUndefined();
  });

  it("rejects a delayed event from a replaced same-id runtime even when snapshot and generation repeat", async () => {
    setBodies({ body1: true });
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-new",
      geometrySource: "live",
    });
    const engine = fakeEngine();
    const { client, getMesh, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    getMesh.mockClear();

    emit({
      ...changed("body1"),
      documentId: "doc-1",
      runtimeSession: "runtime-old",
      snapshotId: 1,
    });
    await tick();

    expect(getMesh).not.toHaveBeenCalled();
    expect(reg.getCurrentMeshPublication()).toBeNull();
  });

  it("fetches + swaps + adds a scene object for a changed, visible body", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, getMesh, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    emit(changed("body1"));
    await tick();

    expect(getMesh).toHaveBeenCalledWith("body1", "fine");
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
  it("pins bootstrap fetches from the retained matching native publication", async () => {
    setBodies({ body1: true });
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
    });
    const retained = {
      ...changed("body1"),
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      snapshotId: 9,
    };
    const getMesh = vi.fn(async () => makeBoxMesh());
    ingest = new MeshIngest();
    ingest.attach(fakeEngine(), fakeClient(getMesh, retained).client);
    await tick();

    expect(getMesh).toHaveBeenCalledWith("body1", "fine", 1, "runtime-1");
    expect(reg.getEntry("body1")?.provenance).toEqual({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      snapshotId: 9,
      generation: 1,
    });
  });

  it("keeps bootstrap/open mesh unpinned and therefore non-authoritative until a publication", async () => {
    setBodies({ body1: true });
    documentStore.setState({ documentId: "opened-doc", geometrySource: "cached" });
    const engine = fakeEngine();
    const { client, getMesh } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    expect(getMesh).toHaveBeenCalledWith("body1", "fine");
    expect(reg.getEntry("body1")?.provenance).toBeUndefined();
  });

  it("loads only the visible pre-seeded bodies on attach, before any event fires", async () => {
    setBodies({ body1: true, body2: false });
    const engine = fakeEngine();
    const { client, getMesh } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    await tick();

    expect(getMesh).toHaveBeenCalledTimes(1);
    expect(getMesh).toHaveBeenCalledWith("body1", "fine");
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
    expect(getMesh).toHaveBeenCalledWith("body1", "fine");
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

    expect(getMesh).toHaveBeenCalledWith("body2", "fine"); // fetched (kept fresh)
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

// ── SAVE/OPEN hardening: self-healing reconcile + surfaced failures ──────────
describe("MeshIngest reconcile (missed document-changed self-heal)", () => {
  it("the exact reopen wedge: first fetch hits the pre-publish window (empty), the " +
     "post-regen projection re-applies the SAME bodies, and the body still loads", async () => {
    setBodies({ body1: true });
    // First fetch = the open window (mesh not regenerated yet); later = published.
    let published = false;
    const getMesh = vi.fn(async () => (published ? makeBoxMesh() : new ArrayBuffer(0)));
    const engine = fakeEngine();
    const { client } = fakeClient(getMesh);
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();
    expect(bodyGroup(engine, "body1")).toBeUndefined(); // pre-publish miss, no scene object

    // The open-regen publishes; document-changed is MISSED (the webview race).
    // The post-regen projection re-hydrates the store with the SAME visible flag —
    // before the reconcile pass this produced no visibility diff and the body
    // stayed invisible forever.
    published = true;
    setBodies({ body1: true });
    await tick();
    expect(bodyGroup(engine, "body1")).toBeDefined();
    expect(bodyGroup(engine, "body1")!.visible).toBe(true);
  });

  it("reconcile drops a scene object whose body left the store (missed removal)", async () => {
    setBodies({ body1: true, body2: true });
    const engine = fakeEngine();
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();
    expect(bodyGroup(engine, "body2")).toBeDefined();

    // body2 vanishes from the projection with NO removedBodies event.
    setBodies({ body1: true });
    await tick();
    expect(bodyGroup(engine, "body2")).toBeUndefined();
  });

  it("reconcile does not re-fetch a body that is already loaded or in flight", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, getMesh } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();
    expect(getMesh).toHaveBeenCalledTimes(1);

    // A projection re-apply with identical content: loaded body ⇒ no new fetch.
    setBodies({ body1: true });
    await tick();
    expect(getMesh).toHaveBeenCalledTimes(1);
  });
});

describe("MeshIngest empty-mesh bounded retry", () => {
  it("retries a get_mesh miss on its own and renders once the mesh publishes, with NO further event", async () => {
    vi.useFakeTimers();
    try {
      setBodies({ body1: true });
      let published = false;
      const getMesh = vi.fn(async () => (published ? makeBoxMesh() : new ArrayBuffer(0)));
      const engine = fakeEngine();
      const { client } = fakeClient(getMesh);
      ingest = new MeshIngest();
      ingest.attach(engine, client);
      await vi.advanceTimersByTimeAsync(0);
      expect(bodyGroup(engine, "body1")).toBeUndefined();

      published = true; // the publish lands silently (no document-changed, no projection)
      await vi.advanceTimersByTimeAsync(1000); // > retry backoff
      expect(bodyGroup(engine, "body1")).toBeDefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it("gives up after the bounded retries without wedging or throwing", async () => {
    vi.useFakeTimers();
    try {
      setBodies({ body1: true });
      const getMesh = vi.fn(async () => new ArrayBuffer(0)); // never publishes
      const engine = fakeEngine();
      const { client } = fakeClient(getMesh);
      ingest = new MeshIngest();
      ingest.attach(engine, client);
      await vi.advanceTimersByTimeAsync(10_000);
      // initial + 3 bounded retries, then stop — no unbounded polling.
      expect(getMesh).toHaveBeenCalledTimes(4);
      expect(bodyGroup(engine, "body1")).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });
});

// ── geometryPending chip (viewportStore) ──────────────────────────────────
describe("MeshIngest geometryPending", () => {
  it("attach with visible bodies + no meshes yet -> pending true", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    // Never resolves within this test's synchronous window: pending must be
    // true the instant the store status is ready, before any fetch settles.
    const { client } = fakeClient(vi.fn(() => new Promise<ArrayBuffer>(() => {})));
    ingest = new MeshIngest();
    ingest.attach(engine, client);

    expect(viewportStore.getState().geometryPending).toBe(true);
  });

  it("all visible bodies' meshes land -> pending false", async () => {
    setBodies({ body1: true, body2: true });
    const engine = fakeEngine();
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    expect(viewportStore.getState().geometryPending).toBe(false);
  });

  it("2 visible bodies, only 1 mesh landed -> STILL pending (count-based)", async () => {
    setBodies({ body1: true, body2: true });
    const engine = fakeEngine();
    // body1 resolves immediately; body2 never resolves in this window.
    const getMesh = vi.fn((id: string) =>
      id === "body1" ? Promise.resolve(makeBoxMesh()) : new Promise<ArrayBuffer>(() => {}),
    );
    const { client } = fakeClient(getMesh);
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    expect(bodyGroup(engine, "body1")).toBeDefined();
    expect(bodyGroup(engine, "body2")).toBeUndefined();
    expect(viewportStore.getState().geometryPending).toBe(true);
  });

  it("detach forces pending false", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client } = fakeClient(vi.fn(() => new Promise<ArrayBuffer>(() => {})));
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    expect(viewportStore.getState().geometryPending).toBe(true);

    ingest.detach();
    ingest = null;
    expect(viewportStore.getState().geometryPending).toBe(false);
  });

  it("document not ready -> pending false even with unloaded visible bodies", async () => {
    documentStore.setState({ status: "loading" });
    try {
      setBodies({ body1: true });
      const engine = fakeEngine();
      const { client } = fakeClient(vi.fn(() => new Promise<ArrayBuffer>(() => {})));
      ingest = new MeshIngest();
      ingest.attach(engine, client);

      expect(viewportStore.getState().geometryPending).toBe(false);
    } finally {
      documentStore.setState({ status: "ready" }); // restore for other tests
    }
  });
});

describe("MeshIngest load-failure surfacing", () => {
  it("a rejected mesh fetch surfaces an error hint and does not block other bodies", async () => {
    setBodies({ bad: true, good: true });
    const getMesh = vi.fn(async (id: string) => {
      if (id === "bad") throw new Error("boom");
      return makeBoxMesh();
    });
    const engine = fakeEngine();
    const { client } = fakeClient(getMesh);
    viewportStore.getState().setStatusHint(null);
    // The failure lands on the structured log lane (DEV-OBSERVABILITY Wave F),
    // which vitest keeps closed by default — open it just for this assertion.
    __resetLogForTests();
    try {
      ingest = new MeshIngest();
      ingest.attach(engine, client);
      await tick();

      expect(bodyGroup(engine, "good")).toBeDefined(); // the failure is per-body
      expect(bodyGroup(engine, "bad")).toBeUndefined();
      const hint = viewportStore.getState().statusHint;
      expect(hint?.severity).toBe("error");
      expect(hint?.message).toContain("boom");
      const logged = logSnapshot().filter((e) => e.level === "error" && e.tag === "mesh");
      expect(logged).toHaveLength(1);
      expect(logged[0].msg).toContain("bad");
    } finally {
      __resetLogForTests({ enabled: false });
      viewportStore.getState().setStatusHint(null);
    }
  });
});

// ── selection integrity across the two kinds of reload (WP-U4 / D-5) ─────────

describe("MeshIngest selection reconcile", () => {
  const faceRef = (topoKey: string) => ({
    kind: "face" as const,
    id: `body1#${topoKey}`,
    bodyId: "body1",
    topoKey,
  });

  afterEach(() => {
    selectionStore.getState().set([]);
    selectionStore.getState().setHover(null);
  });

  it("a COLOUR reload keeps the selection; the next REGEN publish reconciles it", async () => {
    // `onColorChanged` re-publishes the same topology under the same snapshot.
    // Reconciling it would drop every unpromoted ref because a colour changed.
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit(changed("body1"));
    await tick();

    selectionStore.getState().set([faceRef("f:4")]);
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "body1", visible: true, color: [1, 2, 3, 255] } },
    });
    await tick();

    expect(selectionStore.getState().selected).toHaveLength(1);

    // The same body, published by a regen this time: an unpromoted ref has no
    // authority across one, so it goes.
    emit(changed("body1"));
    await tick();

    expect(selectionStore.getState().selected).toEqual([]);
  });

  it("dropping a removed body clears its face/edge refs", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit(changed("body1"));
    await tick();

    const other = { kind: "sketch" as const, id: "sketch1" };
    selectionStore.getState().set([faceRef("f:4"), other]);
    emit({ revision: 2, changedBodies: [], removedBodies: ["body1"] });
    await tick();

    expect(selectionStore.getState().selected).toEqual([other]);
  });

  it("coalesces a burst overtaking delayed authored colors and installs only newest mesh", async () => {
    // `resolveAuthoredFaceColors` awaits one `elementInfo` per authored colour,
    // which is long enough for a newer publish to overtake this one. Without a
    // re-check after that await the older mesh swaps in over the newer.
    documentStore.setState({
      bodies: {
        body1: {
          id: "body1",
          name: "body1",
          visible: true,
          faceColors: { el_a: [9, 9, 9, 255] },
        },
      },
    });
    const meshes = [makeBoxMesh(), makeCylinderMesh()];
    let nth = 0;
    const getMesh = vi.fn(async () => meshes[Math.min(nth++, 1)]);
    const gates: Array<(v: null) => void> = [];
    const { client, emit } = fakeClient(getMesh);
    (client as unknown as { elementInfo: () => Promise<null> }).elementInfo = () =>
      new Promise<null>((r) => gates.push(r));
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client); // reconcile() → load #1 (box)
    await tick();
    emit(changed("body1")); // → load #2 (cylinder)
    await tick();
    expect(gates).toHaveLength(1);

    gates[0](null); // current load completes; exactly one queued reload starts
    await tick();
    expect(gates).toHaveLength(2);
    expect(reg.getEntry("body1")).toBeUndefined(); // superseded box never installs
    gates[1](null);
    await tick();

    expect(reg.getEntry("body1")!.view.faceCount).toBe(3); // the cylinder, not the box
  });

  it("rejects the final swap when runtime authority changes during authored-color resolution", async () => {
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-old",
      geometrySource: "live",
      bodies: {
        body1: {
          id: "body1",
          name: "body1",
          visible: true,
          faceColors: { el_a: [9, 9, 9, 255] },
        },
      },
    });
    const retained = {
      ...changed("body1"),
      documentId: "doc-1",
      runtimeSession: "runtime-old",
      snapshotId: 1,
    };
    let resolveColor!: (value: null) => void;
    const { client } = fakeClient(vi.fn(async () => makeBoxMesh()), retained);
    (client as unknown as { elementInfo: () => Promise<null> }).elementInfo = () =>
      new Promise<null>((resolve) => { resolveColor = resolve; });
    ingest = new MeshIngest();
    ingest.attach(fakeEngine(), client);
    await tick();

    documentStore.setState({ runtimeSession: "runtime-new" });
    resolveColor(null);
    await tick();

    expect(reg.getEntry("body1")).toBeUndefined();
  });

  it("rejects a delayed fetch across removal and re-add without token ABA", async () => {
    setBodies({ body1: true });
    const gates: Array<(mesh: ArrayBuffer) => void> = [];
    const getMesh = vi.fn(() => new Promise<ArrayBuffer>((resolve) => gates.push(resolve)));
    const { client, emit } = fakeClient(getMesh);
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    expect(gates).toHaveLength(1);

    documentStore.setState({ bodies: {} });
    emit({ revision: 2, changedBodies: [], removedBodies: ["body1"] });
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "body1", visible: true } },
    });
    emit(changed("body1"));
    expect(gates).toHaveLength(1);

    gates[0](makeBoxMesh());
    await tick();
    expect(reg.getEntry("body1")).toBeUndefined();
    expect(gates).toHaveLength(2);
    gates[1](makeCylinderMesh());
    await tick();
    expect(reg.getEntry("body1")!.view.faceCount).toBe(3);
  });

  it("bounds repeated remove and re-add bursts to one active plus one queued fetch", async () => {
    setBodies({ body1: true });
    const gates: Array<(mesh: ArrayBuffer) => void> = [];
    const getMesh = vi.fn(() => new Promise<ArrayBuffer>((resolve) => gates.push(resolve)));
    const { client, emit } = fakeClient(getMesh);
    ingest = new MeshIngest();
    ingest.attach(fakeEngine(), client);

    for (let revision = 2; revision <= 5; revision++) {
      documentStore.setState({ bodies: {} });
      emit({ revision, changedBodies: [], removedBodies: ["body1"] });
      documentStore.setState({
        bodies: { body1: { id: "body1", name: "body1", visible: true } },
      });
      emit(changed("body1"));
    }
    expect(getMesh).toHaveBeenCalledTimes(1);

    gates[0](makeBoxMesh());
    await tick();
    expect(getMesh).toHaveBeenCalledTimes(2);
    gates[1](makeCylinderMesh());
    await tick();
    expect(reg.getEntry("body1")!.view.faceCount).toBe(3);
  });

  it("retains the last valid mesh when the latest fetch fails", async () => {
    setBodies({ body1: true });
    const getMesh = vi
      .fn<() => Promise<ArrayBuffer>>()
      .mockResolvedValueOnce(makeBoxMesh())
      .mockRejectedValueOnce(new Error("latest unavailable"));
    const { client, emit } = fakeClient(getMesh);
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();
    const lastValid = reg.getEntry("body1");

    emit(changed("body1"));
    await tick();

    expect(reg.getEntry("body1")).toBe(lastValid);
    expect(engine.bodiesRoot.children).toHaveLength(1);
    expect(viewportStore.getState().statusHint?.message).toContain("failed to load");
  });

  it("keeps the installed mesh and contains a rejected native render acknowledgment", async () => {
    setBodies({ body1: true });
    documentStore.setState({ documentId: "doc-a", runtimeSession: "runtime-a" });
    const { client, emit } = fakeClient();
    const acknowledge = vi.fn(async () => { throw new Error("runtime replaced"); });
    (client as unknown as { meshRenderCompleted: typeof acknowledge }).meshRenderCompleted = acknowledge;
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit({
      ...changed("body1"),
      snapshotId: 7,
      documentId: "doc-a",
      runtimeSession: "runtime-a",
      renderExpectationId: 11,
    });
    await tick();
    await tick();
    const installed = reg.getEntry("body1");

    engine.completeRender();
    await tick();

    expect(acknowledge).toHaveBeenCalledWith({
      expectationId: 11,
      documentId: "doc-a",
      revision: 1,
      snapshotId: 7,
      bodyId: "body1",
    });
    expect(reg.getEntry("body1")).toBe(installed);
  });

  it("PR-11 does not acknowledge a frame that did not submit the body, then acknowledges the first that does", async () => {
    setBodies({ body1: true });
    documentStore.setState({ documentId: "doc-a", runtimeSession: "runtime-a" });
    const { client, emit } = fakeClient();
    const acknowledge = vi.fn(async () => {});
    (client as unknown as { meshRenderCompleted: typeof acknowledge }).meshRenderCompleted = acknowledge;
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit({
      ...changed("body1"),
      snapshotId: 7,
      documentId: "doc-a",
      runtimeSession: "runtime-a",
      renderExpectationId: 11,
    });
    await tick();
    await tick();

    // A frame with OTHER bodies only (body1 hidden / isolated away) proves nothing.
    engine.completeRender(["someOtherBody"]);
    await tick();
    expect(acknowledge).not.toHaveBeenCalled();

    // A frame that submits body1 under a DIFFERENT publication is not it either.
    engine.completeRender(["body1"], { body1: { documentId: "doc-a", runtimeSession: "runtime-a", snapshotId: 6, generation: 0 } });
    await tick();
    expect(acknowledge).not.toHaveBeenCalled();

    // The first frame that carries body1 with the installed provenance is acknowledged once.
    engine.completeRender();
    engine.completeRender();
    await tick();
    expect(acknowledge).toHaveBeenCalledTimes(1);
    expect(acknowledge).toHaveBeenCalledWith({
      expectationId: 11,
      documentId: "doc-a",
      revision: 1,
      snapshotId: 7,
      bodyId: "body1",
    });
  });
});

// ── validated installation, stale display, and publication freshness (WP04) ──

/** A structurally perfect box whose vertex 3 carries a NaN component. */
function semanticallyBrokenBox(): ArrayBuffer {
  const blob = makeBoxMesh();
  parseMeshPayload(blob).positions[9] = Number.NaN;
  return blob;
}

describe("MeshIngest validated installation and display state", () => {
  afterEach(() => {
    viewportStore.getState().setStatusHint(null);
    __resetLogForTests({ enabled: false });
  });

  it("TEST-MESH-05 keeps the last valid body installed as stale-inspection-only when a replacement fails validation", async () => {
    setBodies({ body1: true });
    const getMesh = vi
      .fn<() => Promise<ArrayBuffer>>()
      .mockResolvedValueOnce(makeBoxMesh())
      .mockResolvedValueOnce(semanticallyBrokenBox())
      .mockResolvedValueOnce(makeCylinderMesh());
    const engine = fakeEngine();
    const { client, emit } = fakeClient(getMesh);
    __resetLogForTests();
    ingest = new MeshIngest();
    const seen: string[] = [];
    ingest.onDisplayStateChanged((id) => seen.push(id));
    ingest.attach(engine, client);
    await tick();

    const lastValid = reg.getEntry("body1");
    expect(lastValid).toBeDefined();
    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(reg.isEntryPromotable(lastValid)).toBe(true);
    const heldAfterFirst = ingest.admission.snapshot();
    expect(heldAfterFirst.holdings).toBe(1);

    // A structurally valid but SEMANTICALLY invalid replacement arrives.
    emit(changed("body1"));
    await tick();

    expect(reg.getEntry("body1")).toBe(lastValid); // still installed, still drawn
    expect(engine.bodiesRoot.children).toHaveLength(1);
    expect(ingest.getDisplayState("body1")).toBe("stale-inspection-only");
    expect(ingest.getDisplayDiagnostic("body1")?.code).toBe("nonfinite-position");
    expect(reg.isEntryPromotable(reg.getEntry("body1"))).toBe(false);
    expect(lastValid!.displayState).toBe("stale-inspection-only");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("warn");
    expect(hint?.message).toContain("body1");
    expect(hint?.message).toContain("nonfinite-position");
    const logged = logSnapshot().filter((e) => e.level === "error" && e.tag === "mesh");
    expect(logged).toHaveLength(1);
    // The refused payload gave its reservation back; only the installed body pays.
    expect(ingest.admission.snapshot()).toEqual(heldAfterFirst);
    expect(seen).toContain("body1");

    // A valid publication restores currency.
    emit({ revision: 3, changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:3" }], removedBodies: [] });
    await tick();

    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(ingest.getDisplayDiagnostic("body1")).toBeUndefined();
    expect(reg.isEntryPromotable(reg.getEntry("body1"))).toBe(true);
    expect(reg.getEntry("body1")!.view.faceCount).toBe(3); // the cylinder replaced the box
    // The replaced box is retired but not yet freed, so it is still charged: the
    // budget follows the resource, not the body id (PR-03B).
    expect(ingest.admission.snapshot().holdings).toBe(2);
    reg.flushDisposals();
    expect(ingest.admission.snapshot().holdings).toBe(1);
  });

  it("TEST-MESH-05 records failed-initial with no scene object when the FIRST payload is invalid", async () => {
    setBodies({ body1: true });
    const { client } = fakeClient(vi.fn(async () => semanticallyBrokenBox()));
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    expect(reg.getEntry("body1")).toBeUndefined();
    expect(engine.bodiesRoot.children).toHaveLength(0);
    expect(ingest.getDisplayState("body1")).toBe("failed-initial");
    expect(ingest.getDisplayDiagnostic("body1")?.code).toBe("nonfinite-position");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
    expect(ingest.admission.snapshot().holdings).toBe(0);
  });

  it("TEST-MESH-05 a structurally malformed payload is rejected with its parser kind, not a throw", async () => {
    setBodies({ body1: true });
    const torn = makeBoxMesh();
    new DataView(torn).setUint32(0x00, 0x4d455349, true); // bad magic
    const { client } = fakeClient(vi.fn(async () => torn));
    ingest = new MeshIngest();
    ingest.attach(fakeEngine(), client);
    await tick();

    expect(ingest.getDisplayState("body1")).toBe("failed-initial");
    expect(ingest.getDisplayDiagnostic("body1")?.code).toBe("bad-magic");
  });

  it("TEST-MESH-05 dropping a body clears its display state and gives back its budget", async () => {
    setBodies({ body1: true });
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(fakeEngine(), client);
    await tick();
    expect(ingest.admission.snapshot().holdings).toBe(1);

    emit({ revision: 2, changedBodies: [], removedBodies: ["body1"] });
    await tick();

    expect(ingest.getDisplayState("body1")).toBeUndefined();
    // Dropped is not freed: the entry is retired and its buffers survive until
    // the ordered flush, so it keeps paying until then (PR-03B).
    expect(ingest.admission.snapshot().holdings).toBe(1);
    reg.flushDisposals();
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: 0,
      estimatedGpuBytes: 0,
      holdings: 0,
    });
  });
});


describe("MeshIngest publication freshness", () => {
  afterEach(() => {
    selectionStore.getState().set([]);
    viewportStore.getState().setStatusHint(null);
  });

  /*
   * R1(a) BLOCKER 2 — a change that publishes NO bodies is not evidence that the
   * installed geometry stopped being current.
   *
   * Most `DocumentChange`s carry no geometry at all: `upsertVariable`,
   * `renameVariable`, `deleteRecord`, an `editOperationInput` that resolved to
   * the same shape. Dropping the publication for one of those makes
   * `installedEntryIsCurrent` false for EVERY body, so every pick refuses and
   * the sticky stale hint never clears for the rest of the session.
   */
  it("keeps the publication across a change that publishes no bodies", async () => {
    setBodies({ body1: true });
    documentStore.setState({
      documentId: "doc-1", runtimeSession: "runtime-1", geometrySource: "live",
    });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit({ ...changed("body1"), documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 41 });
    await tick();
    const published = reg.getCurrentMeshPublication();
    expect(published).toEqual({
      documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 41, generation: 1,
    });

    emit({
      revision: 2,
      changedBodies: [],
      removedBodies: [],
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      snapshotId: 42,
    });
    await tick();

    // The SAME publication object, snapshot 41 included: the installed entry's
    // provenance still describes what is on screen, and adopting 42 here would
    // orphan it just as surely as dropping the publication does.
    expect(reg.getCurrentMeshPublication()).toEqual(published);
  });

  it("a pick taken before a bodiless change still promotes afterwards", async () => {
    setBodies({ body1: true });
    documentStore.setState({
      documentId: "doc-1", runtimeSession: "runtime-1", geometrySource: "live",
    });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    emit({ ...changed("body1"), documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 41 });
    await tick();
    const entry = reg.getEntry("body1")!;
    const proof = { entry, kind: "face" as const, topoKey: "f:0" };

    emit({
      revision: 2, changedBodies: [], removedBodies: [],
      documentId: "doc-1", runtimeSession: "runtime-1", snapshotId: 42,
    });
    await tick();

    const promoteSelection = vi.fn(async () => [
      { topoKey: "f:0", elementId: "el_1", kind: "face", bodyId: "body1" },
    ]);
    const out = await promoteViewportPick(
      { promoteSelection } as unknown as CadClient, proof, { topoKey: "f:0", kind: "face" },
    );
    expect(out?.elementId).toBe("el_1");
    expect(viewportStore.getState().statusHint?.message).not.toBe(STALE_PICK_HINT);
  });

  it("TEST-PUB-01 an older generation completing after a newer one installs nothing and releases its reservation", async () => {
    // The authored-colour resolve is the long await inside a load, so it is
    // where a reordered completion lands AFTER a newer publication was accepted.
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: {
        body1: { id: "body1", name: "body1", visible: true, faceColors: { el_a: [9, 9, 9, 255] } },
      },
    });
    const meshes = [makeBoxMesh(), makeCylinderMesh()];
    let nth = 0;
    const getMesh = vi.fn(async () => meshes[Math.min(nth++, 1)]);
    const gates: Array<(v: null) => void> = [];
    const { client, emit } = fakeClient(getMesh);
    (client as unknown as { elementInfo: () => Promise<null> }).elementInfo = () =>
      new Promise<null>((r) => gates.push(r));
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    const face = { kind: "face" as const, id: "body1#f:4", bodyId: "body1", topoKey: "f:4" };
    selectionStore.getState().set([face]);

    // Generation 1's job is parked on its colour resolve. It holds NO budget:
    // the authored colours it is waiting for are an INPUT to the layout, so the
    // plan — and therefore the price — does not exist yet, and neither does any
    // derived array (PR-03A). Generation 2 publishes and coalesces behind it.
    expect(gates).toHaveLength(1);
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: 0,
      estimatedGpuBytes: 0,
      holdings: 0,
    });
    emit({
      ...changed("body1"),
      revision: 2,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:2" }],
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      snapshotId: 2,
    });
    await tick();
    expect(gates).toHaveLength(1); // latest-wins: one active request per body

    gates[0](null); // the OLDER generation finishes last
    await tick();

    expect(reg.getEntry("body1")).toBeUndefined(); // nothing installed by the stale job
    expect(engine.bodiesRoot.children).toHaveLength(0);
    // The stale job allocated nothing and therefore owes nothing; the newer job
    // is in turn parked on its own colour resolve, so the budget is still clear.
    expect(gates).toHaveLength(2);
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: 0,
      estimatedGpuBytes: 0,
      holdings: 0,
    });
    expect(selectionStore.getState().selected).toEqual([face]); // a discarded result touches nothing

    gates[1](null);
    await tick();
    const installed = reg.getEntry("body1")!;
    expect(installed.view.faceCount).toBe(3); // the newer mesh, installed exactly once
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: installed.plan.cpuBytes,
      estimatedGpuBytes: installed.plan.gpuBytes,
      holdings: 1,
    });
  });

  it("TEST-PUB-02 a late buffer from closed document A is rejected after B reuses the body id", async () => {
    documentStore.setState({
      documentId: "doc-a",
      runtimeSession: "runtime-a",
      geometrySource: "live",
      bodies: { body1: { id: "body1", name: "body1", visible: true } },
    });
    const gates: Array<(mesh: ArrayBuffer) => void> = [];
    const getMesh = vi.fn(() => new Promise<ArrayBuffer>((resolve) => gates.push(resolve)));
    const retained = {
      ...changed("body1"),
      documentId: "doc-a",
      runtimeSession: "runtime-a",
      snapshotId: 5,
    };
    const { client, emit } = fakeClient(getMesh, retained);
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    expect(gates).toHaveLength(1); // document A's fetch is in flight

    // Document A closes and B opens, reusing the SAME body-local id.
    documentStore.setState({ documentId: "doc-b", runtimeSession: "runtime-b", bodies: {} });
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "body1", visible: true } },
    });
    emit({
      revision: 1,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:1" }],
      removedBodies: [],
      documentId: "doc-b",
      runtimeSession: "runtime-b",
      snapshotId: 1,
    });
    await tick();

    gates[0](makeBoxMesh()); // document A's buffer finally arrives
    await tick();
    expect(reg.getEntry("body1")).toBeUndefined(); // A's result never installs
    expect(ingest.admission.snapshot().holdings).toBe(0);

    expect(gates).toHaveLength(2); // B's own fetch, released behind A's
    gates[1](makeCylinderMesh());
    await tick();

    const fromB = reg.getEntry("body1")!;
    expect(fromB.view.faceCount).toBe(3);
    expect(fromB.provenance?.documentId).toBe("doc-b");
    expect(reg.registrySize()).toBe(1); // no shared-cache identity collision
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: fromB.plan.cpuBytes,
      estimatedGpuBytes: fromB.plan.gpuBytes,
      holdings: 1,
    });
    expect(ingest.getDisplayState("body1")).toBe("current");
  });

  it("TEST-PUB-02 the document fence alone rejects a validated mesh whose document has been replaced", async () => {
    // No newer request for this body, so the per-body token is unchanged: the
    // ONLY thing that can reject this payload is the publication fence — and it
    // must give the admission reservation back when it does.
    documentStore.setState({
      documentId: "doc-a",
      runtimeSession: "runtime-a",
      geometrySource: "live",
      bodies: { body1: { id: "body1", name: "body1", visible: true } },
    });
    const gates: Array<(mesh: ArrayBuffer) => void> = [];
    const getMesh = vi.fn(() => new Promise<ArrayBuffer>((resolve) => gates.push(resolve)));
    const retained = {
      ...changed("body1"),
      documentId: "doc-a",
      runtimeSession: "runtime-a",
      snapshotId: 5,
    };
    const { client } = fakeClient(getMesh, retained);
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    expect(gates).toHaveLength(1);

    documentStore.setState({ documentId: "doc-b" });
    gates[0](makeBoxMesh());
    await tick();

    expect(reg.getEntry("body1")).toBeUndefined();
    expect(engine.bodiesRoot.children).toHaveLength(0);
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: 0,
      estimatedGpuBytes: 0,
      holdings: 0,
    });
    // A fence rejection is not a payload defect: no stale/failed state is stamped.
    expect(ingest.getDisplayState("body1")).toBeUndefined();
  });
});

describe("MeshIngest pending replacement", () => {
  afterEach(() => {
    viewportStore.getState().setStatusHint(null);
  });

  it("TEST-MESH-05 announces pending-replacement and returns to current when the replacement is fenced out", async () => {
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: {
        body1: { id: "body1", name: "body1", visible: true, faceColors: { el_a: [9, 9, 9, 255] } },
      },
    });
    const meshes = [makeBoxMesh(), makeCylinderMesh()];
    let nth = 0;
    const getMesh = vi.fn(async () => meshes[Math.min(nth++, 1)]);
    const gates: Array<(v: null) => void> = [];
    const retained = {
      ...changed("body1"),
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      snapshotId: 1,
    };
    const { client, emit } = fakeClient(getMesh, retained);
    (client as unknown as { elementInfo: () => Promise<null> }).elementInfo = () =>
      new Promise<null>((r) => gates.push(r));
    const engine = fakeEngine();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();
    gates[0](null);
    await tick();

    const installed = reg.getEntry("body1")!;
    expect(installed.view.faceCount).toBe(6); // the box is on screen
    expect(ingest.getDisplayState("body1")).toBe("current");
    const heldForBox = ingest.admission.snapshot();

    // A replacement is being prepared: the body still shows the box.
    emit({
      ...changed("body1"),
      revision: 2,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:2" }],
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      snapshotId: 2,
    });
    await tick();
    expect(gates).toHaveLength(2);
    expect(ingest.getDisplayState("body1")).toBe("pending-replacement");
    expect(reg.getEntry("body1")).toBe(installed);
    // Only the box is charged. The replacement is parked on the colour resolve
    // that DECIDES its layout, so its plan does not exist yet and neither does
    // any derived array; the peak is taken the moment the plan is (PR-03A), and
    // the box's own hold is what makes that a peak rather than a swap.
    expect(ingest.admission.snapshot()).toEqual(heldForBox);

    // The runtime is replaced mid-preparation, so the fence rejects the result.
    documentStore.setState({ runtimeSession: "runtime-new" });
    gates[1](null);
    await tick();

    expect(reg.getEntry("body1")).toBe(installed); // untouched
    expect(installed.displayState).toBe("current"); // not demoted: nothing was wrong with it
    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(reg.isEntryPromotable(installed)).toBe(true);
    expect(ingest.admission.snapshot()).toEqual(heldForBox);
  });
});

describe("MeshIngest post-install failure isolation", () => {
  afterEach(() => {
    viewportStore.getState().setStatusHint(null);
    __resetLogForTests({ enabled: false });
  });

  /*
   * R1(a) MAJOR 4 — every NOTIFY call-out is isolated, and the load-failure hint
   * belongs to the NOT-INSTALLED branch only.
   *
   * `refreshHighlights` allocates (it rebuilds owned face-set geometry), so it
   * is the realistic thrower. Before the fix it escaped the announcement block,
   * skipped `notifyBodyLoaded` — which every commit's completion waits on — and
   * published "Body failed to load" over a body that had just loaded perfectly.
   */
  it("isolates a throwing refreshHighlights: no hint, body-loaded still fires, state stays current", async () => {
    setBodies({ body1: true });
    const { client } = fakeClient();
    const engine = fakeEngine();
    engine.refreshHighlights.mockImplementation(() => {
      throw new Error("highlight rebuild exploded");
    });
    __resetLogForTests();
    const loaded: string[] = [];
    ingest = new MeshIngest();
    ingest.onBodyLoaded((bodyId) => loaded.push(bodyId));
    ingest.attach(engine, client);
    await tick();

    const installed = reg.getEntry("body1");
    expect(installed?.displayState).toBe("current");
    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(loaded).toEqual(["body1"]);
    expect(viewportStore.getState().statusHint?.message ?? "").not.toContain("failed to load");
    const logged = logSnapshot().filter((e) => e.level === "error" && e.tag === "mesh");
    expect(logged).toHaveLength(1);
    expect(logged[0].ctx).toMatchObject({ bodyId: "body1", step: "refreshHighlights" });
  });

  it("isolates a throwing selection reconcile and still announces the load", async () => {
    setBodies({ body1: true });
    const getMesh = vi
      .fn<() => Promise<ArrayBuffer>>()
      .mockResolvedValueOnce(makeBoxMesh())
      .mockResolvedValueOnce(makeCylinderMesh());
    const { client, emit } = fakeClient(getMesh);
    // `reconcileSelectionForBody` asks the backend about a promoted ref it
    // cannot find in the new table; a throwing client is the realistic fault.
    (client as unknown as { elementInfo: () => Promise<never> }).elementInfo = () => {
      throw new Error("elementInfo exploded");
    };
    const engine = fakeEngine();
    __resetLogForTests();
    const loaded: string[] = [];
    ingest = new MeshIngest();
    ingest.onBodyLoaded((bodyId) => loaded.push(bodyId));
    ingest.attach(engine, client);
    await tick();
    selectionStore.getState().set([
      { kind: "face", id: "body1#f:4", bodyId: "body1", topoKey: "f:4", elementId: "el_z" },
    ]);
    loaded.length = 0;

    emit(changed("body1"));
    await tick();

    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(loaded).toEqual(["body1"]);
    expect(viewportStore.getState().statusHint?.message ?? "").not.toContain("failed to load");
  });

  it("TEST-MESH-05 a throwing bodyLoaded subscriber does not demote the freshly installed mesh", async () => {
    // Everything after the atomic publish is best-effort: a selection reconcile,
    // a highlight rebuild, or — here — a `bodyLoaded` subscriber. The geometry on
    // screen IS the new validated mesh; calling it stale would both lie and block
    // every operation authored from it.
    setBodies({ body1: true });
    const { client } = fakeClient();
    const engine = fakeEngine();
    __resetLogForTests();
    ingest = new MeshIngest();
    ingest.onBodyLoaded(() => {
      throw new Error("subscriber exploded");
    });
    ingest.attach(engine, client);
    await tick();

    const installed = reg.getEntry("body1");
    expect(installed).toBeDefined();
    expect(installed!.displayState).toBe("current");
    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(reg.isEntryPromotable(installed)).toBe(true);
    expect(bodyGroup(engine, "body1")).toBeDefined(); // the scene object is published

    // The failure is reported once, as the listener fault it is. It gets no
    // status hint: the body DID load, and "Body failed to load" would be a lie
    // about the geometry the user is looking at (PR-02).
    const logged = logSnapshot().filter((e) => e.level === "error" && e.tag === "mesh");
    expect(logged).toHaveLength(1);
    expect(logged[0].msg).toBe("body-loaded listener threw");
    expect(logged[0].ctx).toMatchObject({ bodyId: "body1" });
    expect(viewportStore.getState().statusHint?.message ?? "").not.toContain("failed to load");

    // Exactly one reservation — the installed body's — and no leak.
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: installed!.plan.cpuBytes,
      estimatedGpuBytes: installed!.plan.gpuBytes,
      holdings: 1,
    });
  });

  it("TEST-MESH-05 a later valid publication still installs after a post-install failure", async () => {
    setBodies({ body1: true });
    const getMesh = vi
      .fn<() => Promise<ArrayBuffer>>()
      .mockResolvedValueOnce(makeBoxMesh())
      .mockResolvedValueOnce(makeCylinderMesh());
    const { client, emit } = fakeClient(getMesh);
    const engine = fakeEngine();
    ingest = new MeshIngest();
    let explode = true;
    ingest.onBodyLoaded(() => {
      if (explode) throw new Error("subscriber exploded");
    });
    ingest.attach(engine, client);
    await tick();
    explode = false;

    emit(changed("body1"));
    await tick();

    expect(reg.getEntry("body1")!.view.faceCount).toBe(3);
    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(ingest.admission.snapshot().holdings).toBe(2); // the retired box, until the flush
    reg.flushDisposals();
    expect(ingest.admission.snapshot().holdings).toBe(1);
  });
});

/*
 * WP03 ownership across the document lifecycle (spec §8.2, TEST-RES-05).
 *
 * A real HighlightLayer is wired to the fake engine here rather than a spy:
 * the thing under test is that leases and owned overlays return to the baseline
 * when a document closes, and a `vi.fn()` for `setHighlightState` would prove
 * nothing about either.
 */
describe("TEST-RES-05 — resource baseline across document cycles", () => {
  /** Point the fake engine's highlight hooks at a real layer. */
  function withHighlights(engine: ReturnType<typeof fakeEngine>) {
    const layer = new HighlightLayer({ root: new THREE.Group(), invalidate: () => {} });
    const spied = engine as unknown as {
      refreshHighlights: ReturnType<typeof vi.fn>;
      setHighlightState: ReturnType<typeof vi.fn>;
    };
    spied.refreshHighlights.mockImplementation(() => layer.refresh());
    spied.setHighlightState.mockImplementation((hover, selected) => layer.setState(hover, selected));
    return layer;
  }

  const faceRef = {
    kind: "face" as const,
    id: "body1#f:0",
    bodyId: "body1",
    topoKey: "f:0",
  };

  it("fifty open/close cycles end at zero entries, zero leases and zero owned bytes", async () => {
    for (let cycle = 0; cycle < 50; cycle++) {
      setBodies({ body1: true, body2: true });
      const engine = fakeEngine();
      const layer = withHighlights(engine);
      const { client } = fakeClient();
      ingest = new MeshIngest();
      ingest.attach(engine, client);
      await tick();

      // A document nobody interacted with would not exercise the borrowers.
      layer.setState(faceRef, [{ kind: "body", id: "body2" }]);
      expect(reg.registrySize()).toBe(2);
      expect(reg.registryDebugSnapshot().openLeases).toBeGreaterThan(0);
      expect(layer.resourceStats().bytes).toBeGreaterThan(0);

      ingest.detach();
      ingest = null;
      layer.dispose();

      expect(reg.registrySize()).toBe(0);
      expect(reg.registryDebugSnapshot().openLeases).toBe(0);
      expect(layer.resourceStats()).toEqual({ entries: 0, bytes: 0, displayed: 0 });
    }
    expect(reg.leakTripwireCount).toBe(0);
  });

  it("a replacement releases the outgoing body lease and frees it one flush later", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    const first = reg.getEntry("body1")!;
    expect(reg.openLeases(first)).toEqual(["body"]);

    emit(changed("body1"));
    await tick();

    const second = reg.getEntry("body1")!;
    expect(second).not.toBe(first);
    // The new lease was taken in PREPARE and the old one released only after
    // the scene moved (numerics §11.2) — so the outgoing resource is retired,
    // lease-free, and still intact.
    expect(reg.openLeases(second)).toEqual(["body"]);
    expect(reg.openLeases(first)).toEqual([]);
    expect(first.resourceState).toBe("retired");
    expect(first.geometry.getAttribute("position").array).toBe(first.view.positions);

    reg.flushDisposals();
    expect(first.resourceState).toBe("disposed");
    expect(second.resourceState).toBe("installed");
    expect(reg.leakTripwireCount).toBe(0);
  });

  it("dropping a body releases its lease before the resource retires", async () => {
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();
    const entry = reg.getEntry("body1")!;

    emit({ revision: 2, changedBodies: [], removedBodies: ["body1"] });
    await tick();

    expect(reg.openLeases(entry)).toEqual([]);
    expect(entry.resourceState).toBe("retired");
    reg.flushDisposals();
    expect(entry.resourceState).toBe("disposed");
    expect(reg.leakTripwireCount).toBe(0);
  });
});

/*
 * VP-HARDENING PR-03B — the budget follows the RESOURCE, not the body id.
 *
 * Releasing the outgoing reservation at the swap priced a body, not the memory:
 * the retired entry stays fully allocated until an ordered, lease-free frame
 * boundary frees it, so between the swap and that flush the machine is holding
 * BOTH meshes while the counter claims one.
 */
describe("TEST-MESH-04 — admission prices every live resource", () => {
  // The document budget is a module singleton (the preview lanes share it), so
  // each case starts from a known one and hands back the default.
  afterEach(() => {
    admissionModule.__resetDocumentAdmissionForTests();
  });

  it("charges all four entries when three replacements land before a frame boundary", async () => {
    admissionModule.__resetDocumentAdmissionForTests();
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    const first = reg.getEntry("body1")!;
    const unitCpu = first.plan.cpuBytes;
    expect(ingest.admission.snapshot().holdings).toBe(1);

    for (const revision of [2, 3, 4]) {
      emit({
        revision,
        changedBodies: [{ bodyId: "body1", meshKey: `body1:fine:${revision}` }],
        removedBodies: [],
      });
      await tick();
    }

    expect(ingest.admission.snapshot().holdings).toBe(4);
    expect(ingest.admission.snapshot().preparedCpuBytes).toBe(4 * unitCpu);

    reg.flushDisposals();
    const installed = reg.getEntry("body1")!;
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: unitCpu,
      estimatedGpuBytes: installed.plan.gpuBytes,
      holdings: 1,
    });
  });

  it("refuses a replacement that does not fit ALONGSIDE the resources still alive", async () => {
    const probe = validateMeshView(parseMeshPayload(makeBoxMesh()), "probe");
    if (!probe.ok) throw new Error("the mock box must validate");
    const probePlan = planMeshPreparation(probe.mesh, {}, "probe");
    if (!probePlan.ok) throw new Error("the mock box must be plannable");
    const unitGpu = probePlan.plan.gpuBytes;
    // Room for three of these meshes at once, not four.
    admissionModule.__resetDocumentAdmissionForTests({
      estimatedGpuBytes: Math.floor(unitGpu * 3.5),
    });
    setBodies({ body1: true });
    const engine = fakeEngine();
    const { client, emit } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(engine, client);
    await tick();

    for (const revision of [2, 3]) {
      emit({
        revision,
        changedBodies: [{ bodyId: "body1", meshKey: `body1:fine:${revision}` }],
        removedBodies: [],
      });
      await tick();
    }
    const third = reg.getEntry("body1")!;
    expect(ingest.admission.snapshot().holdings).toBe(3);

    emit({
      revision: 4,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:4" }],
      removedBodies: [],
    });
    await tick();

    // Refused: the two retired-but-alive meshes are part of the peak.
    expect(ingest.getDisplayDiagnostic("body1")?.code).toBe("gpu-budget");
    expect(reg.getEntry("body1")).toBe(third); // the drawn resource never changed
    expect(bodyGroup(engine, "body1")).toBeDefined();
    expect(ingest.admission.snapshot().holdings).toBe(3);

    // Freeing the retired meshes frees the budget, and the body recovers.
    reg.flushDisposals();
    expect(ingest.admission.snapshot().holdings).toBe(1);
    emit({
      revision: 5,
      changedBodies: [{ bodyId: "body1", meshKey: "body1:fine:5" }],
      removedBodies: [],
    });
    await tick();
    expect(ingest.getDisplayState("body1")).toBe("current");
    expect(reg.getEntry("body1")).not.toBe(third);
    expect(ingest.admission.snapshot().holdings).toBe(2);
  });
});

/*
 * VP-HARDENING PR-03A — the price is the layout that is actually built.
 *
 * A body colour is DOCUMENT METADATA, not a MESH1 section, so a mesh with no
 * FACE_COLORS payload can still be built de-indexed with a baked per-vertex
 * colour stream. Pricing that mesh from the payload alone charges the indexed
 * layout and then allocates the de-indexed one — for the review's 1,000,000
 * triangle body, 24,000,000 bytes charged against 108,000,000 bytes built.
 */
describe("TEST-MESH-04 — the reservation is the layout that gets built", () => {
  afterEach(() => {
    admissionModule.__resetDocumentAdmissionForTests();
  });

  /** The de-indexed cost of `blob`, computed from the payload alone. */
  function deIndexedCost(blob: ArrayBuffer) {
    const view = parseMeshPayload(blob);
    const verts = view.indices.length; // 3·T, one per triangle corner
    const positionBytes = verts * 3 * 4;
    const normalBytes = view.normals ? verts * 3 * 4 : 0;
    const colorBytes = verts * 3 * 4;
    let segments = 0;
    for (let e = 0; e < view.edgeCount; e++) {
      segments += Math.max(0, (view.edgeRanges?.[e * 2 + 1] ?? 0) - 1);
    }
    const edgeExpansionBytes = segments * 24;
    return {
      // Zero-copy views keep the whole payload alive alongside the expansion.
      preparedCpuBytes: view.buffer.byteLength + positionBytes + normalBytes + colorBytes + edgeExpansionBytes,
      estimatedGpuBytes: positionBytes + normalBytes + colorBytes + edgeExpansionBytes,
      holdings: 1,
    };
  }

  it("charges a metadata-coloured body the DE-INDEXED bytes it actually allocates", async () => {
    admissionModule.__resetDocumentAdmissionForTests();
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: {
        body1: { id: "body1", name: "body1", visible: true, color: [200, 40, 40, 255] },
      },
    });
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(fakeEngine(), client);
    await tick();

    const entry = reg.getEntry("body1")!;
    // The layout really is de-indexed: no index, one vertex per triangle corner.
    expect(entry.hasVertexColors).toBe(true);
    expect(entry.geometry.getIndex()).toBeNull();
    expect(ingest.admission.snapshot()).toEqual(deIndexedCost(makeBoxMesh()));
  });

  it("installs what it priced — the DEV off-plan tripwire stays quiet either way", async () => {
    for (const color of [undefined, [200, 40, 40, 255] as const]) {
      admissionModule.__resetDocumentAdmissionForTests();
      reg.disposeAll();
      reg.__resetRegistryForTests();
      __resetLogForTests();
      documentStore.setState({
        documentId: "doc-1",
        runtimeSession: "runtime-1",
        geometrySource: "live",
        bodies: { body1: { id: "body1", name: "body1", visible: true, color } },
      });
      const { client } = fakeClient(vi.fn(async () => makeCylinderMesh()));
      ingest = new MeshIngest();
      ingest.attach(fakeEngine(), client);
      await tick();

      const entry = reg.getEntry("body1")!;
      expect(entry.plan.layout).toBe(color ? "deindexed" : "indexed");
      expect(ingest.admission.snapshot()).toEqual({
        preparedCpuBytes: entry.plan.cpuBytes,
        estimatedGpuBytes: entry.plan.gpuBytes,
        holdings: 1,
      });
      expect(logSnapshot().filter((e) => e.level === "error")).toEqual([]);
      ingest.detach();
      __resetLogForTests({ enabled: false });
    }
  });

  it("charges an uncoloured body the INDEXED bytes it actually allocates", async () => {
    admissionModule.__resetDocumentAdmissionForTests();
    setBodies({ body1: true });
    const { client } = fakeClient();
    ingest = new MeshIngest();
    ingest.attach(fakeEngine(), client);
    await tick();

    const entry = reg.getEntry("body1")!;
    expect(entry.hasVertexColors).toBe(false);
    const view = parseMeshPayload(makeBoxMesh());
    const attributeBytes =
      view.positions.byteLength + (view.normals?.byteLength ?? 0) + view.indices.byteLength;
    const edgeBytes = entry.edgeSegmentPositions?.byteLength ?? 0;
    expect(ingest.admission.snapshot()).toEqual({
      preparedCpuBytes: view.buffer.byteLength + edgeBytes,
      estimatedGpuBytes: attributeBytes + edgeBytes,
      holdings: 1,
    });
  });
});
