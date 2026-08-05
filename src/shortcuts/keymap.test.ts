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

describe("resolveBinding — chamfer tool id + H binding died (FILLET-CHAMFER-UNIFY W2)", () => {
  it("model `h` resolves to the GLOBAL home action, un-shadowed", () => {
    expect(resolveBinding("h", false, "model")).toEqual({ type: "home" });
  });

  it("`f` still resolves to the 3D fillet in MODEL mode", () => {
    expect(resolveBinding("f", false, "model")).toEqual({ type: "tool", tool: "fillet" });
  });

  // WP-C T2b: sketch mode now claims `f` for the 2D corner fillet, which RETIRES
  // the old cross-mode fallback (`f` while drawing used to finish the sketch and
  // arm the 3D fillet). One letter, one meaning, mode-resolved like `t`/`d`/`r`.
  it("`f` resolves to the 2D sketch fillet in SKETCH mode (no cross-mode leak)", () => {
    expect(resolveBinding("f", false, "sketch")).toEqual({ type: "tool", tool: "sketchFillet" });
  });

  it("nothing resolves to tool \"chamfer\" — no key, in either mode, plain or shifted", () => {
    for (const mode of ["model", "sketch"] as const) {
      for (const shift of [false, true]) {
        for (const key of [...new Set([...MODEL_KEYS, ...SKETCH_KEYS].map((b) => b.key))]) {
          const action = resolveBinding(key, shift, mode);
          if (action && action.type === "tool") {
            expect(action.tool).not.toBe("chamfer");
          }
        }
      }
    }
  });
});

describe("resolveBinding — ⇧A is the 3-point arc (SP-4)", () => {
  it("plain `a` still resolves to the centre-start-end arc", () => {
    expect(resolveBinding("a", false, "sketch")).toEqual({ type: "tool", tool: "arc" });
  });

  it("⇧A resolves to arc3p in sketch mode", () => {
    expect(resolveBinding("a", true, "sketch")).toEqual({ type: "tool", tool: "arc3p" });
  });

  it("model mode reaches BOTH arcs cross-mode (a draw tool starts a sketch)", () => {
    expect(resolveBinding("a", false, "model")).toEqual({ type: "tool", tool: "arc" });
    expect(resolveBinding("a", true, "model")).toEqual({ type: "tool", tool: "arc3p" });
  });

  it("`a` is claimed only by the sketch table, plain and shifted", () => {
    expect(MODEL_KEYS.filter((b) => b.key === "a")).toHaveLength(0);
    expect(SKETCH_KEYS.filter((b) => b.key === "a")).toHaveLength(2);
  });
});

describe("resolveBinding — ⇧T is Extend (SP-4)", () => {
  it("plain `t` is untouched in BOTH modes (sketch Trim / model Transform)", () => {
    expect(resolveBinding("t", false, "sketch")).toEqual({ type: "tool", tool: "trim" });
    expect(resolveBinding("t", false, "model")).toEqual({ type: "tool", tool: "transform" });
  });

  it("⇧T resolves to extend in sketch mode", () => {
    expect(resolveBinding("t", true, "sketch")).toEqual({ type: "tool", tool: "extend" });
  });

  it("model mode reaches Extend cross-mode (a sketch edit tool starts a sketch)", () => {
    expect(resolveBinding("t", true, "model")).toEqual({ type: "tool", tool: "extend" });
  });

  it("`t` is claimed twice by sketch (plain + chord) and once by model (plain)", () => {
    expect(SKETCH_KEYS.filter((b) => b.key === "t")).toHaveLength(2);
    expect(MODEL_KEYS.filter((b) => b.key === "t")).toHaveLength(1);
    expect(MODEL_KEYS.filter((b) => b.key === "t" && b.shift)).toHaveLength(0);
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
