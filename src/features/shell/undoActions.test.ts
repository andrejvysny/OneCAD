/*
 * The undo/redo router's ONE job: decide who owns the chord, and never let two
 * of them answer it.
 *
 * The failure this guards against is the one the 2026-09-11 review hit — ⌘Z
 * inside a dimension field reverting the MODEL instead of the typo, with the
 * focus still sitting in the field. Reaching native text undo is therefore
 * terminal: there is no fall-through to the document history, not even when the
 * webview reports the text stack is empty.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { toolStore } from "@/stores/toolStore";
import { resetStores } from "@/test/resetStores";
import { redoSketch, undoSketch } from "@/tools/sketch/sketchService";
import { setModelToolController } from "@/tools/modelTools/modelToolBridge";
import type { ModelToolController } from "@/tools/modelTools/ModelToolController";
import { runRedo, runUndo } from "./undoActions";

vi.mock("@/tools/sketch/sketchService", async (importActual) => {
  const actual = await importActual<typeof import("@/tools/sketch/sketchService")>();
  return {
    ...actual,
    undoSketch: vi.fn(() => Promise.resolve()),
    redoSketch: vi.fn(() => Promise.resolve()),
  };
});

let ctrl: { undo: ReturnType<typeof vi.fn>; redo: ReturnType<typeof vi.fn> };
let exec: ReturnType<typeof vi.fn>;
let field: HTMLInputElement | null = null;

/** jsdom implements no `execCommand`, so the router's one native seam is stubbed. */
function stubExecCommand(result = true): void {
  exec = vi.fn(() => result);
  (document as unknown as { execCommand: unknown }).execCommand = exec;
}

function focusTextField(): void {
  field = document.createElement("input");
  document.body.appendChild(field);
  field.focus();
}

beforeEach(() => {
  resetStores();
  vi.mocked(undoSketch).mockClear();
  vi.mocked(redoSketch).mockClear();
  ctrl = { undo: vi.fn(() => Promise.resolve()), redo: vi.fn(() => Promise.resolve()) };
  setModelToolController(ctrl as unknown as ModelToolController);
  stubExecCommand();
});

afterEach(() => {
  field?.remove();
  field = null;
});

describe("undo router — a focused text field keeps native text undo", () => {
  it("runs the native undo and NEVER the document history", async () => {
    focusTextField();
    await runUndo();
    expect(exec).toHaveBeenCalledWith("undo");
    expect(ctrl.undo).not.toHaveBeenCalled();
    expect(undoSketch).not.toHaveBeenCalled();
  });

  it("runs the native redo and NEVER the document history", async () => {
    focusTextField();
    await runRedo();
    expect(exec).toHaveBeenCalledWith("redo");
    expect(ctrl.redo).not.toHaveBeenCalled();
    expect(redoSketch).not.toHaveBeenCalled();
  });

  it("does not fall through when the native stack reports empty", async () => {
    // WebKit answers `false` for an empty text stack. Treating that as "nothing
    // happened, try the document" is exactly how a typo-undo becomes a model
    // revert — so it stays consumed.
    stubExecCommand(false);
    focusTextField();
    await runUndo();
    expect(ctrl.undo).not.toHaveBeenCalled();
  });

  it("holds even in SKETCH mode", async () => {
    focusTextField();
    toolStore.getState().setMode("sketch");
    await runUndo();
    expect(undoSketch).not.toHaveBeenCalled();
    expect(exec).toHaveBeenCalledWith("undo");
  });
});

describe("undo router — outside a field it is mode-scoped", () => {
  it("model mode drives the document history", async () => {
    await runUndo();
    expect(ctrl.undo).toHaveBeenCalledTimes(1);
    expect(undoSketch).not.toHaveBeenCalled();
    expect(exec).not.toHaveBeenCalled();
    await runRedo();
    expect(ctrl.redo).toHaveBeenCalledTimes(1);
  });

  it("sketch mode drives the sketch-scoped stack", async () => {
    toolStore.getState().setMode("sketch");
    await runUndo();
    expect(undoSketch).toHaveBeenCalledTimes(1);
    expect(ctrl.undo).not.toHaveBeenCalled();
    await runRedo();
    expect(redoSketch).toHaveBeenCalledTimes(1);
    expect(ctrl.redo).not.toHaveBeenCalled();
  });

  it("is a no-op, not a throw, with no model controller mounted", async () => {
    setModelToolController(null);
    await expect(runUndo()).resolves.toBeUndefined();
    await expect(runRedo()).resolves.toBeUndefined();
  });
});
