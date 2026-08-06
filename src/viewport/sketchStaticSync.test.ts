/*
 * SketchStaticSync store wiring: initial sweep fetches visible sketches (skips
 * invisible), visibility flips + removals reach the layer, entering/exiting sketch
 * mode toggles the editing hide + refetches, and a getSketchRegions rejection
 * degrades to empty regions. The engine + layer + client are fakes.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { SketchStaticSync } from "./sketchStaticSync";
import { documentStore, type SketchMeta } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { selectionStore, sketchRegionRef } from "@/stores/selectionStore";
import type { CadClient } from "@/ipc/client";
import type { SketchRegion, SketchSession } from "@/ipc/types";
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

function meta(id: string, visible: boolean, geometryToken = "geometry-v1"): SketchMeta {
  return { id, name: id, visible, dof: 0, status: "ok", geometryToken };
}

const REGION: SketchRegion = {
  regionId: "r-keep",
  outerLoop: [],
  holes: [],
  previewTriangles: {
    positions: [0, 0, 10, 0, 0, 10],
    indices: [0, 1, 2],
  },
};

/** Plain vi.fn bag — kept UNcast so the specs can assert/mockClear on each spy. */
function fakeLayer() {
  return {
    setSketch: vi.fn(),
    removeSketch: vi.fn(),
    setVisible: vi.fn(),
    setEditingSketch: vi.fn(),
    setSelected: vi.fn(),
    setHover: vi.fn(),
  };
}

function fakeEngine(layer: ReturnType<typeof fakeLayer>) {
  return {
    getSketchStaticLayer: () => layer as unknown as SketchStaticLayer,
  } as unknown as ViewportEngine;
}

function fakeClient(regions: SketchRegion[] = []) {
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
  const getSketchRegions = vi.fn(async () => ({ regions }));
  return { getSketch, getSketchRegions } as unknown as CadClient & {
    getSketch: typeof getSketch;
    getSketchRegions: typeof getSketchRegions;
  };
}

let sync: SketchStaticSync | null = null;

beforeEach(() => {
  documentStore.setState({ sketches: {}, revision: 0 });
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

  it("rebuilds a same-id visible sketch when its geometry token changes", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true, "geometry-v1") } });
    const layer = fakeLayer();
    const client = fakeClient([REGION]);
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();
    client.getSketch.mockClear();
    client.getSketchRegions.mockClear();
    layer.setSketch.mockClear();
    layer.removeSketch.mockClear();

    documentStore.setState({
      sketches: {
        s1: { ...meta("s1", true, "geometry-v2"), dof: 1 },
      },
    });
    await tick();

    expect(layer.removeSketch).toHaveBeenCalledWith("s1");
    expect(client.getSketch).toHaveBeenCalledOnce();
    expect(client.getSketchRegions).toHaveBeenCalledOnce();
    expect(layer.setSketch).toHaveBeenCalledOnce();
  });

  it("does not refetch when a same-id sketch keeps its geometry token", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true, "geometry-v1") } });
    const layer = fakeLayer();
    const client = fakeClient();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();
    client.getSketch.mockClear();
    client.getSketchRegions.mockClear();
    layer.setSketch.mockClear();

    documentStore.setState({
      sketches: {
        s1: {
          ...meta("s1", true, "geometry-v1"),
          name: "Renamed",
          dof: 4,
          status: "under",
        },
      },
    });
    await tick();

    expect(client.getSketch).not.toHaveBeenCalled();
    expect(client.getSketchRegions).not.toHaveBeenCalled();
    expect(layer.setSketch).not.toHaveBeenCalled();
  });

  it("retains valid region refs and clears stale refs after a token refresh", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true, "geometry-v1") } });
    const layer = fakeLayer();
    const client = fakeClient([REGION]);
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();

    const kept = sketchRegionRef("s1", "r-keep");
    const stale = sketchRegionRef("s1", "r-stale");
    selectionStore.getState().set([kept, stale, { kind: "body", id: "b1" }]);
    selectionStore.getState().setHover(stale);

    documentStore.setState({
      sketches: { s1: meta("s1", true, "geometry-v2") },
    });
    await tick();

    expect(selectionStore.getState().selected).toEqual([
      kept,
      { kind: "body", id: "b1" },
    ]);
    expect(selectionStore.getState().hover).toBeNull();
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

describe("SketchStaticSync editing (sketch→sketch switch)", () => {
  it("reloads the CLOSED sketch when its token bumps while another sketch is active", async () => {
    // The contract SketchController.switchTo relies on: A's layer copy is stale the
    // moment A closes, and the ONLY signal is its geometryToken. The mode never
    // changes across a switch, so the sketch→model refetch above never fires here.
    documentStore.setState({
      sketches: { A: meta("A", true, "wire:A:v1"), B: meta("B", true, "wire:B:v1") },
    });
    const layer = fakeLayer();
    const client = fakeClient();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();

    toolStore.getState().setMode("sketch", "A");
    viewportStore.setState({ activeSketchId: "A" });
    await tick();
    // The switch lands: B is the edited sketch, A is closed and back on the layer.
    viewportStore.setState({ activeSketchId: "B" });
    await tick();
    client.getSketch.mockClear();
    layer.setSketch.mockClear();
    layer.removeSketch.mockClear();

    documentStore.setState({
      sketches: { A: meta("A", true, "local:1"), B: meta("B", true, "wire:B:v1") },
    });
    await tick();

    expect(layer.removeSketch).toHaveBeenCalledWith("A"); // stale fills never stay pickable
    expect(client.getSketch).toHaveBeenCalledWith("A");
    expect(client.getSketch).not.toHaveBeenCalledWith("B");
    expect(layer.setSketch).toHaveBeenCalledOnce();
  });

  it("does NOT refetch a sketch whose token bumps while IT is the one being edited", async () => {
    documentStore.setState({ sketches: { A: meta("A", true, "wire:A:v1") } });
    const layer = fakeLayer();
    const client = fakeClient();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();

    toolStore.getState().setMode("sketch", "A");
    viewportStore.setState({ activeSketchId: "A" });
    await tick();
    client.getSketch.mockClear();

    documentStore.setState({ sketches: { A: meta("A", true, "local:2") } });
    await tick();

    // The live SketchObject owns A's geometry while it is open — refetching would
    // race the session, so the diff drops the copy and waits for the exit refetch.
    expect(layer.removeSketch).toHaveBeenCalledWith("A");
    expect(client.getSketch).not.toHaveBeenCalled();
  });
});

describe("SketchStaticSync fill degradation + selection mirror", () => {
  it("still builds the sketch with empty regions when getSketchRegions rejects", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    const client = fakeClient();
    client.getSketchRegions.mockRejectedValueOnce(new Error("no regions"));
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();

    expect(layer.setSketch).toHaveBeenCalledWith("s1", expect.objectContaining({ regions: [] }));
  });

  it("retries a failed region fetch when a later regen publishes", async () => {
    // The open-a-project shape: the region fetch races a not-yet-ready worker and
    // rejects, the sketch is built curves-only, and NOTHING would refetch it (its
    // geometryToken never changes) until the gated first regen publishes.
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    const client = fakeClient([REGION]);
    client.getSketchRegions.mockRejectedValueOnce(new Error("worker not connected"));
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();
    expect(layer.setSketch).toHaveBeenLastCalledWith("s1", expect.objectContaining({ regions: [] }));

    documentStore.setState({ revision: 1 });
    await tick();

    expect(client.getSketchRegions).toHaveBeenCalledTimes(2);
    expect(layer.setSketch).toHaveBeenLastCalledWith(
      "s1",
      expect.objectContaining({ regions: [REGION] }),
    );
  });

  it("does not retry a sketch whose regions arrived, and never spins", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    const client = fakeClient([REGION]);
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();
    expect(client.getSketchRegions).toHaveBeenCalledOnce();

    documentStore.setState({ revision: 1 });
    documentStore.setState({ revision: 2 });
    await tick();
    expect(client.getSketchRegions).toHaveBeenCalledOnce();
  });

  it("re-attempts a still-failing fetch once per publish, not in a loop", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    const client = fakeClient();
    client.getSketchRegions.mockRejectedValue(new Error("still not connected"));
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();
    expect(client.getSketchRegions).toHaveBeenCalledTimes(1);

    documentStore.setState({ revision: 1 });
    await tick();
    expect(client.getSketchRegions).toHaveBeenCalledTimes(2);

    // No further publish ⇒ no further attempt (the retry is edge-triggered).
    await tick();
    await tick();
    expect(client.getSketchRegions).toHaveBeenCalledTimes(2);
  });

  it("grants a publish that landed DURING the failing fetch (no stranded curves-only)", async () => {
    // The publish's retry sweep runs while the fetch is still in flight, so it sees
    // an empty retry set; without the missed-revision grant the sketch would then
    // wait for a publish that may never come.
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    const client = fakeClient([REGION]);
    let rejectFirst!: (e: Error) => void;
    client.getSketchRegions.mockImplementationOnce(
      () =>
        new Promise((_res, rej) => {
          rejectFirst = rej;
        }),
    );
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), client);
    await tick();

    documentStore.setState({ revision: 1 }); // publish lands mid-fetch
    await tick();
    rejectFirst(new Error("worker not connected")); // fetch fails AFTER the publish
    await tick();
    await tick(); // the microtask-scheduled immediate retry resolves

    expect(client.getSketchRegions).toHaveBeenCalledTimes(2);
    expect(layer.setSketch).toHaveBeenLastCalledWith(
      "s1",
      expect.objectContaining({ regions: [REGION] }),
    );
  });

  it("mirrors sketch selection + hover into the layer tint", async () => {
    documentStore.setState({ sketches: { s1: meta("s1", true) } });
    const layer = fakeLayer();
    sync = new SketchStaticSync();
    sync.attach(fakeEngine(layer), fakeClient());
    await tick();

    selectionStore.getState().set([{ kind: "sketch", id: "s1" }]);
    expect(layer.setSelected).toHaveBeenLastCalledWith([{ kind: "sketch", sketchId: "s1" }]);
    selectionStore.getState().setHover({ kind: "sketch", id: "s1" });
    expect(layer.setHover).toHaveBeenLastCalledWith({ kind: "sketch", sketchId: "s1" });

    selectionStore.getState().set([sketchRegionRef("s1", "r1")]);
    expect(layer.setSelected).toHaveBeenLastCalledWith([
      { kind: "sketchRegion", sketchId: "s1", regionId: "r1" },
    ]);
    selectionStore.getState().setHover(sketchRegionRef("s1", "r0"));
    expect(layer.setHover).toHaveBeenLastCalledWith({
      kind: "sketchRegion",
      sketchId: "s1",
      regionId: "r0",
    });

    selectionStore.getState().setHover({ kind: "face", id: "body#f:1" });
    expect(layer.setHover).toHaveBeenLastCalledWith(null);
  });
});
