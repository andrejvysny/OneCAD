/*
 * Toolbar GOLDEN probe (Platform refactor W0).
 *
 * Pins the shipped toolbar arrangement against a frozen contract so that later
 * waves — which make these tables registry-derived — must reproduce it exactly.
 * The contract is a literal copy in `src/test/contracts/`, never an import of
 * the value under test.
 */
import { describe, it, expect } from "vitest";
import { MODEL_TOOLS, SKETCH_TOOLS, toolsForMode, isSeparator } from "./toolbarConfig";
import { MODEL_TOOLS_CONTRACT, SKETCH_TOOLS_CONTRACT } from "@/test/contracts/toolbarContract";

describe("toolbar contract", () => {
  it("the model toolbar matches the frozen arrangement", () => {
    expect(MODEL_TOOLS).toEqual(MODEL_TOOLS_CONTRACT);
  });

  it("the sketch toolbar matches the frozen arrangement", () => {
    expect(SKETCH_TOOLS).toEqual(SKETCH_TOOLS_CONTRACT);
  });

  it("toolsForMode routes each mode to its own set", () => {
    expect(toolsForMode("model")).toEqual(MODEL_TOOLS_CONTRACT);
    expect(toolsForMode("sketch")).toEqual(SKETCH_TOOLS_CONTRACT);
  });

  it("every non-separator entry carries a complete presentation", () => {
    for (const entry of [...MODEL_TOOLS, ...SKETCH_TOOLS]) {
      if (isSeparator(entry)) continue;
      expect(entry.id, "tool id").toBeTruthy();
      expect(entry.icon, `icon for ${entry.id}`).toBeTruthy();
      expect(entry.label, `label for ${entry.id}`).toBeTruthy();
      expect(entry.shortcut, `shortcut for ${entry.id}`).toBeTruthy();
    }
  });

  it("no tool id repeats inside a mode's set", () => {
    for (const set of [MODEL_TOOLS, SKETCH_TOOLS]) {
      const ids = set.filter((e) => !isSeparator(e)).map((e) => (isSeparator(e) ? "" : e.id));
      expect(new Set(ids).size).toBe(ids.length);
    }
  });
});
