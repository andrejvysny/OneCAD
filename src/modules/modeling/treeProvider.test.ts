import { beforeEach, describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import { bootTestPlatform } from "@/test/renderWithPlatform";
import { resetStores } from "@/test/resetStores";
import { setViewportEngine } from "@/viewport/engineBridge";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { subscribeTreeReveal } from "@/features/tree/treeReveal";
import { ModelingTreeCommands } from "./ids";
import { contributeModelingTree, modelingTreeSections } from "./treeProvider";

function command(id: string) {
  const platform = bootTestPlatform(contributeModelingTree);
  const entry = platform.commands.get(id as never);
  if (!entry) throw new Error(`missing command ${id}`);
  return entry;
}

function fakeEngine(bounds: THREE.Box3 | null = new THREE.Box3().setFromCenterAndSize(
  new THREE.Vector3(),
  new THREE.Vector3(1, 1, 1),
)) {
  return {
    getBoundsForBodies: vi.fn(() => bounds),
    fitToBodies: vi.fn(),
    fitView: vi.fn(),
  };
}

describe("modeling tree commands", () => {
  beforeEach(() => {
    resetStores();
    setViewportEngine(null);
  });

  it("maps canvas subelements and sketch regions to their tree owners without changing selection", () => {
    const listener = vi.fn(() => true);
    const unsubscribe = subscribeTreeReveal(listener);
    const reveal = command(ModelingTreeCommands.revealSelection);
    const cases = [
      [{ kind: "face", id: "body1#f:2", bodyId: "body1" }, "body1"],
      [{ kind: "edge", id: "body1#e:3", bodyId: "body1" }, "body1"],
      [{ kind: "vertex", id: "body1#v:1", bodyId: "body1" }, "body1"],
      [{ kind: "sketchRegion", id: "region", sketchId: "sketch2", regionId: "r1" }, "sketch2"],
    ] as const;

    try {
      for (const [ref, nodeId] of cases) {
        selectionStore.getState().set([ref]);
        expect(reveal.execute({ selection: [], scopes: [] })).toEqual({ status: "done" });
        expect(listener).toHaveBeenLastCalledWith({
          providerId: expect.any(String),
          nodeId,
        });
        expect(selectionStore.getState().selected).toEqual([ref]);
      }
    } finally {
      unsubscribe();
    }
  });

  it("cancels reveal truthfully while no tree host is mounted", () => {
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);

    expect(command(ModelingTreeCommands.revealSelection).execute({ selection: [], scopes: [] })).toEqual({
      status: "cancelled",
    });
  });

  it("frames only a current visible body with an installed bound", () => {
    const engine = fakeEngine();
    setViewportEngine(engine as never);
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    const before = documentStore.getState();

    const result = command(ModelingTreeCommands.frameBody).execute({ selection: [], scopes: [] });

    expect(result).toEqual({ status: "done" });
    expect(engine.getBoundsForBodies).toHaveBeenCalledWith(["body1"]);
    expect(engine.fitToBodies).toHaveBeenCalledWith(["body1"]);
    expect(engine.fitView).not.toHaveBeenCalled();
    expect(documentStore.getState().revision).toBe(before.revision);
    expect(documentStore.getState().dirty).toBe(before.dirty);
  });

  it("refuses frame when the target is hidden, stale, pending, or absent", () => {
    const engine = fakeEngine(null);
    setViewportEngine(engine as never);
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    const frame = command(ModelingTreeCommands.frameBody);

    documentStore.setState({ bodies: { body1: { ...documentStore.getState().bodies.body1, visible: false } } });
    expect(frame.execute({ selection: [], scopes: [] })).toEqual({ status: "cancelled" });

    documentStore.setState({
      bodies: { body1: { ...documentStore.getState().bodies.body1, visible: true } },
      geometrySource: "cached",
    });
    expect(frame.execute({ selection: [], scopes: [] })).toEqual({ status: "cancelled" });

    documentStore.setState({ geometrySource: "live" });
    viewportStore.setState({ geometryPending: true });
    expect(frame.execute({ selection: [], scopes: [] })).toEqual({ status: "cancelled" });

    viewportStore.setState({ geometryPending: false });
    expect(frame.execute({ selection: [], scopes: [] })).toEqual({ status: "cancelled" });

    expect(engine.fitToBodies).not.toHaveBeenCalled();
    expect(engine.fitView).not.toHaveBeenCalled();
  });

  it("does not offer a dead frame action without a current installed bound", () => {
    expect(modelingTreeSections()[0].nodes[0].actions).toBeUndefined();

    setViewportEngine(fakeEngine() as never);
    expect(modelingTreeSections()[0].nodes[0].actions).toHaveLength(1);

    setViewportEngine(fakeEngine(null) as never);
    expect(modelingTreeSections()[0].nodes[0].actions).toBeUndefined();

    documentStore.setState({ geometrySource: "cached" });
    expect(modelingTreeSections()[0].nodes[0].actions).toBeUndefined();
  });
});
