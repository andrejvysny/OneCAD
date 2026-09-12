import { describe, it, expect, beforeEach } from "vitest";
import { toolStore } from "./toolStore";
import { viewportStore } from "./viewportStore";
import { selectionStore } from "./selectionStore";
import { resetStores } from "@/test/resetStores";
import { operationAttemptStore } from "./operationAttemptStore";

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

describe("toolStore.setMode opts.tool (AUTO-MODE preserve)", () => {
  beforeEach(() => resetStores());

  it("sketch entry keeps the caller-chosen tool instead of the line default", () => {
    toolStore.getState().setMode("sketch", undefined, { tool: "circle" });

    const s = toolStore.getState();
    expect(s.mode).toBe("sketch");
    expect(s.sketchTool).toBe("circle");
    expect(s.phase).toBe("armed");
  });

  it("model entry keeps the caller-chosen tool instead of the select default", () => {
    toolStore.getState().setMode("sketch", "sketch2");
    toolStore.getState().setMode("model", undefined, { tool: "extrude" });

    const s = toolStore.getState();
    expect(s.mode).toBe("model");
    expect(s.modelTool).toBe("extrude");
    expect(s.phase).toBe("armed");
    expect(viewportStore.getState().activeSketchId).toBeNull();
  });

  it("select as an explicit tool keeps the idle phase", () => {
    toolStore.getState().setMode("sketch", undefined, { tool: "select" });
    expect(toolStore.getState().phase).toBe("idle");
  });
});

describe("toolStore operation attempt fence", () => {
  beforeEach(() => resetStores());

  it("blocks direct tool and mode changes while submission is applying", () => {
    toolStore.getState().setTool("extrude");
    operationAttemptStore.getState().begin("mock-document", null);

    toolStore.getState().setTool("fillet");
    toolStore.getState().setMode("sketch");

    expect(toolStore.getState()).toMatchObject({ mode: "model", modelTool: "extrude" });
    expect(viewportStore.getState().statusHint?.message).toBe("Operation is still applying");
  });

  it("refuses a second applying attempt without replacing the first", () => {
    const first = operationAttemptStore.getState().begin(undefined, null);
    const second = operationAttemptStore.getState().begin(undefined, null);

    expect(first).not.toBeNull();
    expect(second).toBeNull();
    expect(operationAttemptStore.getState().attempt?.token).toBe(first);
  });

  it("allows a new document runtime to own its own attempt", () => {
    operationAttemptStore.getState().begin("doc-a", null);
    const next = operationAttemptStore.getState().begin("doc-b", null);

    expect(next).not.toBeNull();
    expect(operationAttemptStore.getState().attempt?.documentId).toBe("doc-b");
  });

  it("does not block a different document runtime", () => {
    operationAttemptStore.getState().begin("doc-a", null);
    toolStore.getState().setTool("extrude");
    expect(toolStore.getState().modelTool).toBe("extrude");
  });
});
