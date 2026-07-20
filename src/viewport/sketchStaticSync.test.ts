/*
 * SketchStaticSync store wiring: initial sweep fetches visible sketches (skips
 * invisible), visibility flips + removals reach the layer, entering/exiting sketch
 * mode toggles the editing hide + refetches, and a finishSketch rejection degrades
 * to empty regions. The engine + layer + client are fakes.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchStaticSync } from "./sketchStaticSync";
import { documentStore, type SketchMeta } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { selectionStore } from "@/stores/selectionStore";
import type { CadClient } from "@/ipc/client";
import type { SketchSession } from "@/ipc/types";
import type { ViewportEngine } from "./engine/ViewportEngine";
import type { SketchStaticLayer } from "./engine/SketchStaticLayer";

const tick = () => new Promise((r) => setTimeout(r, 0));

const PLANE: SketchSession["plane"] = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

function meta(id: string, visible: boolean): SketchMeta {
  return { id, name: id, visible, dof: 0, status: "ok" };
}

function fakeLayer() {
  return {
    setSketch: vi.fn(),
    removeSketch: vi.fn(),
    setVisible: vi.fn(),
    setEditingSketch: vi.fn(),
    setSelected: vi.fn(),
    setHover: vi.fn(),
  } as unknown as SketchStaticLayer & Record<string, ReturnType<typeof vi.fn>>;
}

function fakeEngine(layer: SketchStaticLayer) {
  return { getSketchStaticLayer: () => layer } as unknown as ViewportEngine;
}

function fakeClient() {
  const getSketch = vi.fn(
    async (sketchId: string): Promise<SketchSession> => ({
      sketchId,
      plane: PLANE,
      entities: [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }],
      constraints: [],
      dof: 0,
      status: "FullyConstrained",
    }),
  );
  const finishSketch = vi.fn(async () => ({ regions: [] }));
  return { getSketch, finishSketch } as unknown as CadClient & {
    getSketch: typeof getSketch;
    finishSketch: typeof finishSketch;
  };
}

let sync: SketchStaticSync | null = null;

beforeEach(() => {
  documentStore.setState({ sketches: {} });
  toolStore.setState({ mode: "model" });
  viewportStore.setState({ activeSketchId: null });
  selectionStore.getState().set([]);
  selectionStore.getState().setHover(null);
});

afterEach(() => {
  sync?.detach();
  sync = null;
});

describe("SketchStaticSync initial sweep", () => {
  it("fetches visible pre-seeded sketches and skips invisible ones", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true), s2: meta("s2", false) } });
    const layer = fakeLayer();
    const client = fakeClient();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();

    expect(client.getSketch).toHaveBeenCalledWith("s1");
    expect(client.getSketch).not.toHaveBeenCalledWith("s2");
    expect(layer.setSketch).toHaveBeenCalledWith(
      "s1",
      expect.objectContaining({ plane: PLANE, regions: [] }),
    );
  });
});

describe("SketchStaticSync document diff", () => {
  it("flips visibility on a loaded sketch via setVisible", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), fakeClient());
    await tick();

    documentStore.getState().setVisibility("s1", false);
    await tick();
    expect(layer.setVisible).toHaveBeenCalledWith("s1", false);
  });

  it("removes a sketch dropped from the document", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), fakeClient());
    await tick();

    documentStore.getState().removeSketch("s1");
    await tick();
    expect(layer.removeSketch).toHaveBeenCalledWith("s1");
  });

  it("lazy-loads a sketch the first time it becomes visible", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", false) } });
    const layer = fakeLayer();
    const client = fakeClient();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();
    expect(client.getSketch).not.toHaveBeenCalled();

    documentStore.getState().setVisibility("s1", true);
    await tick();
    expect(client.getSketch).toHaveBeenCalledWith("s1");
  });
});

describe("SketchStaticSync editing (mode enter/exit)", () => {
  it("hides the edited sketch on enter and refetches it on exit", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    const client = fakeClient();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();
    client.getSketch.mockClear();

    toolStore.getState().setMode("sketch", "s1");
    expect(layer.setEditingSketch).toHaveBeenLastCalledWith("s1");

    toolStore.getState().setMode("model");
    await tick();
    expect(layer.setEditingSketch).toHaveBeenLastCalledWith(null);
    expect(client.getSketch).toHaveBeenCalledWith("s1"); // geometry refetched on exit
  });
});

describe("SketchStaticSync fill degradation + selection mirror", () => {
  it("still builds the sketch with empty regions when finishSketch rejects", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    const client = fakeClient();
    client.finishSketch.mockRejectedValueOnce(new Error("no regions"));
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();

    expect(layer.setSketch).toHaveBeenCalledWith("s1", expect.objectContaining({ regions: [] }));
  });

  it("mirrors sketch selection + hover into the layer tint", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), fakeClient());
    await tick();

    selectionStore.getState().set([{ kind: "sketch", id: "s1" }]);
    expect(layer.setSelected).toHaveBeenLastCalledWith(["s1"]);
    selectionStore.getState().setHover({ kind: "sketch", id: "s1" });
    expect(layer.setHover).toHaveBeenLastCalledWith("s1");
    selectionStore.getState().setHover({ kind: "face", id: "body#f:1" });
    expect(layer.setHover).toHaveBeenLastCalledWith(null);
  });
});
