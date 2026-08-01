/*
 * Keymap resolution — the mode/chord rules, and the W2-B cross-mode OPT-OUT.
 */
import { describe, it, expect } from "vitest";
import { MODEL_KEYS, SKETCH_KEYS, resolveBinding } from "./keymap";

describe("resolveBinding — mode scoping", () => {
  it("a shared letter resolves per mode (R = revolve / rect)", () => {
    expect(resolveBinding("r", false, "model")).toEqual({ type: "tool", tool: "revolve" });
    expect(resolveBinding("r", false, "sketch")).toEqual({ type: "tool", tool: "rect" });
  });

  it("an exact shift chord does not answer the plain key", () => {
    expect(resolveBinding("r", true, "sketch")).toEqual({ type: "tool", tool: "centerRect" });
    expect(resolveBinding("r", false, "sketch")).toEqual({ type: "tool", tool: "rect" });
  });

  it("cross-mode fallback still carries authoring tools both ways", () => {
    // E is model-only; pressing it while drawing finishes the sketch + arms extrude.
    expect(resolveBinding("e", false, "sketch")).toEqual({ type: "tool", tool: "extrude" });
    // L is sketch-only; pressing it in model mode starts a sketch with Line.
    expect(resolveBinding("l", false, "model")).toEqual({ type: "tool", tool: "line" });
  });

  it("cross-mode fallback never carries a non-tool action", () => {
    // Delete is sketch-scoped `deleteSketchSelection` — inert in model mode.
    expect(resolveBinding("Delete", false, "model")).toBeNull();
  });
});

describe("resolveBinding — Measure (?) is NOT cross-mode (W2-B)", () => {
  it("arms Measure in model mode", () => {
    expect(resolveBinding("?", true, "model")).toEqual({ type: "tool", tool: "measure" });
  });

  it("is INERT in sketch mode — the keystroke must never finish the sketch", () => {
    // Without the NO_CROSS_MODE opt-out this resolves to {tool:"measure"}, and
    // `activateTool` would run `finishSketchToTool` — squashing and ENDING the
    // user's live sketch session as a side effect of a read-only shortcut.
    expect(resolveBinding("?", true, "sketch")).toBeNull();
  });

  it("plain `/` (no shift) is not Measure in either mode", () => {
    expect(resolveBinding("/", false, "model")).toBeNull();
    expect(resolveBinding("/", false, "sketch")).toBeNull();
  });

  it("`?` is claimed by exactly one table, so the exclusion has one owner", () => {
    expect(MODEL_KEYS.filter((b) => b.key === "?")).toHaveLength(1);
    expect(SKETCH_KEYS.filter((b) => b.key === "?")).toHaveLength(0);
  });
});
