/*
 * AUTO-MODE dispatcher tests (tools/activateTool.ts): the mode follows the
 * picked tool + context. Matrix:
 *   model + sketch-only tool  ⇒ enter sketch WITH that tool (plane-pick intent)
 *   sketch + model-only tool  ⇒ drained finish ⇒ model WITH that tool armed
 *                               (+ pendingExtrude handoff for Extrude)
 *   context-local ids (select/mirror) ⇒ plain in-mode setTool
 *   "sketch" id               ⇒ explicit new-sketch intent (model mode only)
 */
import { describe, it, expect, beforeEach } from "vitest";
import { activateTool, isModelOnlyTool, isSketchOnlyTool } from "./activateTool";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";

describe("activateTool — tool classification", () => {
  it("classifies sketch-only vs model-only vs context-local tools", () => {
    for (const t of ["line", "rect", "circle", "arc", "arc3p", "dimension", "trim", "extend"] as const) {
      expect(isSketchOnlyTool(t)).toBe(true);
      expect(isModelOnlyTool(t)).toBe(false);
    }
    for (const t of ["datum", "extrude", "revolve", "fillet", "boolean", "shell", "linearPattern", "circularPattern"] as const) {
      expect(isModelOnlyTool(t)).toBe(true);
      expect(isSketchOnlyTool(t)).toBe(false);
    }
    // Context-local: resolved inside the current mode, never a switch trigger.
    expect(isSketchOnlyTool("select")).toBe(false);
    expect(isModelOnlyTool("select")).toBe(false);
    expect(isSketchOnlyTool("mirror")).toBe(false);
    expect(isModelOnlyTool("mirror")).toBe(false);
  });
});

describe("activateTool — model mode", () => {
  beforeEach(() => resetStores());

  it("a sketch-only tool enters sketch mode WITH that tool armed (not the default line)", async () => {
    await activateTool("circle");
    const s = toolStore.getState();
    expect(s.mode).toBe("sketch");
    expect(s.sketchTool).toBe("circle");
    expect(s.phase).toBe("armed");
    // No id ⇒ new-sketch intent: the controller shows the plane picker.
    expect(viewportStore.getState().activeSketchId).toBeNull();
  });

  it.each(["line", "rect", "arc", "arc3p", "dimension", "trim", "extend"] as const)(
    "enters sketch mode preserving %s",
    async (tool) => {
      await activateTool(tool);
      expect(toolStore.getState().mode).toBe("sketch");
      expect(toolStore.getState().sketchTool).toBe(tool);
    },
  );

  it("a model-only tool just arms in place", async () => {
    await activateTool("extrude");
    expect(toolStore.getState().mode).toBe("model");
    expect(toolStore.getState().modelTool).toBe("extrude");
  });

  it("the 'sketch' id enters sketch mode with the default tool", async () => {
    await activateTool("sketch");
    expect(toolStore.getState().mode).toBe("sketch");
    expect(toolStore.getState().sketchTool).toBe("line");
  });

  it("mirror stays context-local (model mirror)", async () => {
    await activateTool("mirror");
    expect(toolStore.getState().mode).toBe("model");
    expect(toolStore.getState().modelTool).toBe("mirror");
  });
});

describe("activateTool — sketch mode", () => {
  beforeEach(() => {
    resetStores();
    // Active edit session on the seeded sketch (mirrors an opened session).
    toolStore.getState().setMode("sketch", "sketch2");
  });

  it("a model-only tool finishes to model mode WITH the tool armed", async () => {
    await activateTool("fillet");
    const s = toolStore.getState();
    expect(s.mode).toBe("model");
    expect(s.modelTool).toBe("fillet");
    expect(s.phase).toBe("armed");
    expect(viewportStore.getState().activeSketchId).toBeNull();
    expect(viewportStore.getState().pendingExtrudeSketch).toBeNull();
  });

  it("extrude additionally rides the pendingExtrude handoff for the region pick", async () => {
    await activateTool("extrude");
    expect(toolStore.getState().mode).toBe("model");
    expect(viewportStore.getState().pendingExtrudeSketch).toBe("sketch2");
  });

  it("datum (DATUM W1) finishes the sketch FIRST, then arms in model mode", async () => {
    await activateTool("datum");
    const s = toolStore.getState();
    // A datum is authored against world planes, which only exist in model mode —
    // pressing D while drawing must not leave a half-finished sketch behind.
    expect(s.mode).toBe("model");
    expect(s.modelTool).toBe("datum");
    expect(s.phase).toBe("armed");
    expect(viewportStore.getState().activeSketchId).toBeNull();
    expect(viewportStore.getState().pendingExtrudeSketch).toBeNull();
  });

  it.each(["revolve", "boolean", "shell", "linearPattern", "circularPattern"] as const)(
    "finishes + arms %s without the extrude handoff",
    async (tool) => {
      await activateTool(tool);
      expect(toolStore.getState().mode).toBe("model");
      expect(toolStore.getState().modelTool).toBe(tool);
      expect(viewportStore.getState().pendingExtrudeSketch).toBeNull();
    },
  );

  it("a sketch-only tool just switches in place (no finish, no mode flip)", async () => {
    await activateTool("circle");
    expect(toolStore.getState().mode).toBe("sketch");
    expect(toolStore.getState().sketchTool).toBe("circle");
    expect(viewportStore.getState().activeSketchId).toBe("sketch2");
  });

  it("mirror stays context-local (sketch mirror)", async () => {
    await activateTool("mirror");
    expect(toolStore.getState().mode).toBe("sketch");
    expect(toolStore.getState().sketchTool).toBe("mirror");
  });

  it("select stays in sketch mode (no accidental finish)", async () => {
    await activateTool("select");
    expect(toolStore.getState().mode).toBe("sketch");
    expect(toolStore.getState().sketchTool).toBe("select");
  });
});
