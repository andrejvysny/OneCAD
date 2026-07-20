import { describe, it, expect, beforeEach } from "vitest";
import { toolStore } from "./toolStore";
import { viewportStore } from "./viewportStore";
import { selectionStore } from "./selectionStore";
import { resetStores } from "@/test/resetStores";

describe("toolStore.setMode (sketch entry)", () => {
  beforeEach(() => resetStores());

  it("bare sketch entry leaves activeSketchId null and selection untouched", () => {
    const before = selectionStore.getState().selected;

    toolStore.getState().setMode("sketch");

    expect(toolStore.getState().mode).toBe("sketch");
    expect(toolStore.getState().sketchTool).toBe("line");
    // No target → the controller shows the plane picker; nothing is active yet.
    expect(viewportStore.getState().activeSketchId).toBeNull();
    // A bare entry must not run the select side effect.
    expect(selectionStore.getState().selected).toBe(before);
  });

  it("an explicit id targets + selects that existing sketch", () => {
    toolStore.getState().setMode("sketch", "sketch2");

    expect(viewportStore.getState().activeSketchId).toBe("sketch2");
    expect(selectionStore.getState().selected).toEqual([{ kind: "sketch", id: "sketch2" }]);
  });

  it("leaving sketch mode clears the active sketch", () => {
    toolStore.getState().setMode("sketch", "sketch2");
    toolStore.getState().setMode("model");

    expect(toolStore.getState().mode).toBe("model");
    expect(viewportStore.getState().activeSketchId).toBeNull();
  });
});
