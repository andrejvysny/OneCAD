import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, act } from "@testing-library/react";
import { resolveBinding } from "./keymap";
import { useShortcuts } from "./useShortcuts";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { sketchStore } from "@/stores/sketchStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { planeFor } from "@/ipc/mockSketch";
import type { SketchEntity } from "@/ipc/types";
import { resetStores } from "@/test/resetStores";
import { flushSketchMutations, redoSketch, undoSketch } from "@/tools/sketch/sketchService";
import { setModelToolController } from "@/tools/modelTools/modelToolBridge";
import type { ModelToolController } from "@/tools/modelTools/ModelToolController";

// Spy the sketch undo/redo verbs (keep the rest of the module real: deleteEntities +
// flushSketchMutations are used elsewhere in this file).
vi.mock("@/tools/sketch/sketchService", async (importActual) => {
  const actual = await importActual<typeof import("@/tools/sketch/sketchService")>();
  return { ...actual, undoSketch: vi.fn(() => Promise.resolve()), redoSketch: vi.fn(() => Promise.resolve()) };
});

function press(key: string, opts: { shift?: boolean; meta?: boolean } = {}): KeyboardEvent {
  const ev = new KeyboardEvent("keydown", {
    key,
    shiftKey: opts.shift ?? false,
    metaKey: opts.meta ?? false,
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

  it("W2-B/C: binds the four new sketch draw tools without breaking the model table", () => {
    // ⇧R is an EXACT shift match — plain R still resolves to rect.
    expect(resolveBinding("r", true, "sketch")).toEqual({ type: "tool", tool: "centerRect" });
    expect(resolveBinding("r", false, "sketch")).toEqual({ type: "tool", tool: "rect" });
    // Model mode has no ⇧R, so it reaches centerRect via the cross-mode fallback.
    expect(resolveBinding("r", true, "model")).toEqual({ type: "tool", tool: "centerRect" });

    expect(resolveBinding("g", false, "sketch")).toEqual({ type: "tool", tool: "polygon" });
    expect(resolveBinding("g", false, "model")).toEqual({ type: "tool", tool: "polygon" });
  });

  it("O is the sketch Ellipse in BOTH modes (free in model → cross-mode fallback)", () => {
    // Deliberately NOT `E`: that stays the model Extrude binding, i.e. the
    // cross-mode "finish the sketch and extrude" handoff (keymap.ts header).
    expect(resolveBinding("o", false, "sketch")).toEqual({ type: "tool", tool: "ellipse" });
    expect(resolveBinding("o", false, "model")).toEqual({ type: "tool", tool: "ellipse" });
    expect(resolveBinding("e", false, "sketch")).toEqual({ type: "tool", tool: "extrude" });
    expect(resolveBinding("e", false, "model")).toEqual({ type: "tool", tool: "extrude" });

    // Sketch S = slot; model S stays the new-sketch intent (mode bindings win).
    expect(resolveBinding("s", false, "sketch")).toEqual({ type: "tool", tool: "slot" });
    expect(resolveBinding("s", false, "model")).toEqual({ type: "enterSketch" });

    // Sketch P = point, deliberately SHADOWING the cross-mode linearPattern.
    expect(resolveBinding("p", false, "sketch")).toEqual({ type: "tool", tool: "point" });
    expect(resolveBinding("p", false, "model")).toEqual({ type: "tool", tool: "linearPattern" });

    // DATUM W1: D is mode-resolved, exactly like R. Sketch mode's Dimension tool
    // claims `d` first, so the datum tool can never fire while drawing.
    expect(resolveBinding("d", false, "model")).toEqual({ type: "tool", tool: "datum" });
    expect(resolveBinding("d", false, "sketch")).toEqual({ type: "tool", tool: "dimension" });
    // …and the chord form is unclaimed in both tables.
    expect(resolveBinding("d", true, "model")).toBeNull();
    expect(resolveBinding("d", true, "sketch")).toBeNull();
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

  it("AUTO-MODE: a key bound only in the other mode resolves cross-mode (tool actions)", () => {
    // L is sketch-only, E is model-only — both cross the boundary.
    expect(resolveBinding("l", false, "model")).toEqual({ type: "tool", tool: "line" });
    expect(resolveBinding("e", false, "sketch")).toEqual({ type: "tool", tool: "extrude" });
    // Shared letters NEVER cross: the current mode's claim wins.
    expect(resolveBinding("r", false, "sketch")).toEqual({ type: "tool", tool: "rect" });
    expect(resolveBinding("r", false, "model")).toEqual({ type: "tool", tool: "revolve" });
    expect(resolveBinding("m", false, "sketch")).toEqual({ type: "tool", tool: "mirror" });
    expect(resolveBinding("m", false, "model")).toEqual({ type: "tool", tool: "mirror" });
    // Non-tool sketch-only actions (Delete) still cannot cross into model mode.
    expect(resolveBinding("Delete", false, "model")).toBeNull();
  });

  it("W3: ⇧I is isolate in model mode and INERT everywhere else", () => {
    expect(resolveBinding("i", true, "model")).toEqual({ type: "isolate" });
    // Exact shift chord — plain `i` stays free in both tables.
    expect(resolveBinding("i", false, "model")).toBeNull();
    expect(resolveBinding("i", false, "sketch")).toBeNull();
    // Not a `tool` action, so the cross-mode fallback cannot leak it into sketch
    // mode (isolating bodies would hide what the sketch is drawn against).
    expect(resolveBinding("i", true, "sketch")).toBeNull();
  });

  it("W3: ⇧I collides with nothing — the pinned neighbours keep their meaning", () => {
    expect(resolveBinding("f", true, "model")).toEqual({ type: "zoomFit" });
    expect(resolveBinding("r", true, "sketch")).toEqual({ type: "tool", tool: "centerRect" });
    expect(resolveBinding("?", true, "model")).toEqual({ type: "tool", tool: "measure" });
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

  it("enters sketch mode on S and finishes on Enter", async () => {
    render(<Harness />);
    press("s");
    expect(toolStore.getState().mode).toBe("sketch");
    // finishSketch now DRAINS the sketch mutation queue before flipping mode, so the
    // flip lands on a microtask — await the queue before asserting.
    await act(async () => {
      press("Enter");
      await flush();
    });
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

  it("⇧I isolates the selected bodies, and Esc leaves isolation BEFORE deselecting", () => {
    render(<Harness />);
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);

    press("i", { shift: true });
    expect(viewportStore.getState().isolatedBodyIds).toEqual(["body1"]);

    // Rung order matters: clearing the selection first would strand the user
    // inside an isolate set with nothing selected.
    press("Escape");
    expect(viewportStore.getState().isolatedBodyIds).toBeNull();
    expect(selectionStore.getState().selected).toHaveLength(1);

    press("Escape");
    expect(selectionStore.getState().selected).toHaveLength(0);
  });

  it("⇧I toggles isolation back off", () => {
    render(<Harness />);
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    press("i", { shift: true });
    press("i", { shift: true });
    expect(viewportStore.getState().isolatedBodyIds).toBeNull();
  });

  it("plain `i` does nothing (the chord is exact-shift)", () => {
    render(<Harness />);
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    const ev = press("i");
    expect(ev.defaultPrevented).toBe(false);
    expect(viewportStore.getState().isolatedBodyIds).toBeNull();
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
      // Drain the ACTUAL mutation queue rather than racing a fixed macrotask
      // (flaky under parallel worker-pool load — deleteEntities's sketchUpsert
      // await can settle after an arbitrary single setTimeout(0)).
      await flushSketchMutations();
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

describe("useShortcuts — undo/redo routing (mode-gated)", () => {
  let ctrl: { undo: ReturnType<typeof vi.fn>; redo: ReturnType<typeof vi.fn> };

  beforeEach(() => {
    resetStores();
    vi.mocked(undoSketch).mockClear();
    vi.mocked(redoSketch).mockClear();
    ctrl = { undo: vi.fn(() => Promise.resolve()), redo: vi.fn(() => Promise.resolve()) };
    setModelToolController(ctrl as unknown as ModelToolController);
  });

  it("⌘Z in SKETCH mode drives the sketch undo (not the model history)", () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    press("z", { meta: true });
    expect(undoSketch).toHaveBeenCalledTimes(1);
    expect(ctrl.undo).not.toHaveBeenCalled();
  });

  it("⇧⌘Z in SKETCH mode drives the sketch redo", () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    press("z", { meta: true, shift: true });
    expect(redoSketch).toHaveBeenCalledTimes(1);
    expect(ctrl.redo).not.toHaveBeenCalled();
  });

  it("⌘Z in MODEL mode drives the model history (not the sketch undo)", () => {
    render(<Harness />);
    press("z", { meta: true });
    expect(ctrl.undo).toHaveBeenCalledTimes(1);
    expect(undoSketch).not.toHaveBeenCalled();
  });

  it("Ctrl+Y in SKETCH mode drives the sketch redo", () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    press("y", { meta: true });
    expect(redoSketch).toHaveBeenCalledTimes(1);
    expect(ctrl.redo).not.toHaveBeenCalled();
  });
});

// ── W1-B: X — construction geometry ───────────────────────────────────────────

describe("X — construction geometry", () => {
  beforeEach(() => resetStores());

  it("resolves in sketch mode only, and never crosses into model mode", () => {
    expect(resolveBinding("x", false, "sketch")).toEqual({ type: "toggleConstruction" });
    // Not a `tool` action ⇒ the AUTO-MODE cross-mode fallback must NOT leak it.
    expect(resolveBinding("x", false, "model")).toBeNull();
  });

  function seedSketch(entities: SketchEntity[]): void {
    sketchStore.getState().setSession({
      sketchId: "sk-x",
      plane: planeFor("XY"),
      entities,
      constraints: [],
      dof: 8,
      status: "UnderConstrained",
    });
  }

  const real = (id: string): SketchEntity => ({ id, type: "Line", p0: [0, 0], p1: [40, 0] });
  const cons = (id: string): SketchEntity => ({ ...real(id), construction: true });
  const flags = (): boolean[] =>
    sketchStore.getState().session!.entities.map((e) => !!e.construction);

  it("with an EMPTY selection toggles the sticky draw mode", () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    sketchSelectionStore.getState().clear();

    expect(sketchStore.getState().constructionMode).toBe(false);
    const ev = press("x");
    expect(ev.defaultPrevented).toBe(true);
    expect(sketchStore.getState().constructionMode).toBe(true);
    press("x");
    expect(sketchStore.getState().constructionMode).toBe(false);
  });

  it("with a selection flips those entities and leaves the draw mode alone", async () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    seedSketch([real("e1"), real("e2")]);
    // A point-pick flips its OWNING entity (same V1 rule as delete).
    sketchSelectionStore.getState().set([{ entityId: "e1", point: "Start" }]);

    await act(async () => {
      press("x");
      await flushSketchMutations();
    });
    expect(flags()).toEqual([true, false]);
    expect(sketchStore.getState().constructionMode).toBe(false); // mode untouched
  });

  it("MIXED selection: everything becomes construction unless ALL already are", async () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    seedSketch([cons("e1"), real("e2")]);
    sketchSelectionStore.getState().set([{ entityId: "e1" }, { entityId: "e2" }]);

    // Mixed ⇒ target true: the real one joins the construction one.
    await act(async () => {
      press("x");
      await flushSketchMutations();
    });
    expect(flags()).toEqual([true, true]);

    // Now ALL are construction ⇒ the next press flips the whole pick back to real.
    await act(async () => {
      press("x");
      await flushSketchMutations();
    });
    expect(flags()).toEqual([false, false]);
  });

  it("is a no-op when the selection names no live entity", async () => {
    render(<Harness />);
    act(() => toolStore.getState().setMode("sketch"));
    seedSketch([real("e1")]);
    sketchSelectionStore.getState().set([{ entityId: "gone" }]);

    await act(async () => {
      press("x");
      await flushSketchMutations();
    });
    expect(flags()).toEqual([false]);
    expect(sketchStore.getState().constructionMode).toBe(false); // never fell back to the mode
  });
});
