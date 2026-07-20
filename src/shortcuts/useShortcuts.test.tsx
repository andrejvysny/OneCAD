import { describe, it, expect, beforeEach } from "vitest";
import { render, act } from "@testing-library/react";
import { resolveBinding } from "./keymap";
import { useShortcuts } from "./useShortcuts";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { sketchStore } from "@/stores/sketchStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { planeFor } from "@/ipc/mockSketch";
import type { SketchEntity } from "@/ipc/types";
import { resetStores } from "@/test/resetStores";

function press(key: string, opts: { shift?: boolean } = {}): KeyboardEvent {
  const ev = new KeyboardEvent("keydown", {
    key,
    shiftKey: opts.shift ?? false,
    bubbles: true,
    cancelable: true,
  });
  act(() => {
    window.dispatchEvent(ev);
  });
  return ev;
}

const flush = () => new Promise((r) => setTimeout(r, 0));

function Harness() {
  useShortcuts();
  return null;
}

describe("keymap resolveBinding", () => {
  it("resolves the same letter to different tools per mode", () => {
    expect(resolveBinding("r", false, "model")).toEqual({
      type: "tool",
      tool: "revolve",
    });
    expect(resolveBinding("r", false, "sketch")).toEqual({
      type: "tool",
      tool: "rect",
    });
  });

  it("routes S to enter-sketch and Enter to finish-sketch", () => {
    expect(resolveBinding("s", false, "model")).toEqual({ type: "enterSketch" });
    expect(resolveBinding("Enter", false, "sketch")).toEqual({
      type: "finishSketch",
    });
  });

  it("keeps F as the Fillet tool and moves zoom-fit to Shift+F", () => {
    expect(resolveBinding("f", false, "model")).toEqual({
      type: "tool",
      tool: "fillet",
    });
    expect(resolveBinding("f", true, "model")).toEqual({ type: "zoomFit" });
  });

  it("binds Delete/Backspace to delete-sketch-selection (sketch mode only)", () => {
    expect(resolveBinding("Delete", false, "sketch")).toEqual({
      type: "deleteSketchSelection",
    });
    expect(resolveBinding("Backspace", false, "sketch")).toEqual({
      type: "deleteSketchSelection",
    });
    // Model mode leaves them unbound (fall through to their default meaning).
    expect(resolveBinding("Delete", false, "model")).toBeNull();
    expect(resolveBinding("Backspace", false, "model")).toBeNull();
  });
});

describe("useShortcuts", () => {
  beforeEach(() => resetStores());

  it("switches tools mode-scoped (R = revolve in model, rect in sketch)", () => {
    render(<Harness />);

    press("r");
    expect(toolStore.getState().modelTool).toBe("revolve");

    act(() => toolStore.getState().setMode("sketch"));
    press("r");
    expect(toolStore.getState().sketchTool).toBe("rect");
  });

  it("enters sketch mode on S and finishes on Enter", () => {
    render(<Harness />);
    press("s");
    expect(toolStore.getState().mode).toBe("sketch");
    press("Enter");
    expect(toolStore.getState().mode).toBe("model");
  });

  it("runs the Esc ladder: cancel tool → deselect → exit sketch", () => {
    render(<Harness />);
    // Model: arm a tool, then Esc reverts to select before deselecting.
    press("e");
    expect(toolStore.getState().modelTool).toBe("extrude");
    press("Escape");
    expect(toolStore.getState().modelTool).toBe("select");
    // Selection still present (Sketch 2) — next Esc clears it.
    expect(selectionStore.getState().selected.length).toBe(1);
    press("Escape");
    expect(selectionStore.getState().selected.length).toBe(0);
  });

  it("Delete deletes the sketch selection and clears it", async () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    const entities: SketchEntity[] = [
      { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
      { id: "e2", type: "Line", p0: [40, 0], p1: [40, 40] },
    ];
    sketchStore.getState().setSession({
      sketchId: "sk-key",
      plane: planeFor("XY"),
      entities,
      constraints: [],
      dof: 8,
      status: "UnderConstrained",
    });
    // A point-pick selection deletes its OWNING entity (e1).
    sketchSelectionStore.getState().set([{ entityId: "e1", point: "Start" }]);

    let ev!: KeyboardEvent;
    await act(async () => {
      ev = press("Delete");
      await flush();
    });
    expect(ev.defaultPrevented).toBe(true); // swallowed (selection present)
    expect(sketchStore.getState().session!.entities.map((e) => e.id)).toEqual(["e2"]);
    expect(sketchSelectionStore.getState().selected).toHaveLength(0);
  });

  it("Delete falls through when the sketch selection is empty", () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    sketchSelectionStore.getState().clear();
    const ev = press("Delete");
    expect(ev.defaultPrevented).toBe(false); // not swallowed — default meaning kept
  });

  it("bails when a text input is focused", () => {
    render(
      <>
        <Harness />
        <input data-testid="field" />
      </>,
    );
    const input = document.querySelector("input")!;
    input.focus();
    act(() => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "e", bubbles: true }),
      );
    });
    expect(toolStore.getState().modelTool).toBe("select");
  });
});
