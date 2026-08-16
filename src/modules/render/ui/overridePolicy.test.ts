/*
 * The one decision every assigning surface shares: what happens to a body's
 * face overrides when its body material changes.
 *
 * Driven through injected deps rather than through the dialog: the claim under
 * test is the FLOW (when is the question asked, and what is written for each
 * answer), and routing it through a modal would test React instead.
 */
import { describe, it, expect, vi } from "vitest";

import { createMaterial } from "../model/material";
import { renderStore } from "../store/renderStore";
import { renderDialogStore, type OverrideChoice } from "./dialogStore";
import {
  assignBodyWithOverridePolicy,
  defaultOverridePolicyDeps,
  type OverridePolicyDeps,
} from "./overridePolicy";

const STEEL = createMaterial("Steel");

function deps(overrides: Partial<OverridePolicyDeps> = {}) {
  const assignBody = vi.fn(async () => {});
  const clearFaceOverrides = vi.fn(async () => {});
  const prompt = vi.fn(async (): Promise<OverrideChoice> => "keep");
  const base: OverridePolicyDeps = {
    boundOverrides: () => [],
    materialName: () => "Steel",
    assignBody,
    clearFaceOverrides,
    prompt,
    ...overrides,
  };
  return { deps: base, assignBody, clearFaceOverrides, prompt };
}

describe("assignBodyWithOverridePolicy", () => {
  it("assigns straight through when the body has no bound overrides", async () => {
    const d = deps();
    const outcome = await assignBodyWithOverridePolicy("body1", STEEL.id, d.deps);

    expect(outcome).toBe("assigned");
    expect(d.assignBody).toHaveBeenCalledWith("body1", STEEL.id);
    // The whole point: no modal for the ordinary case.
    expect(d.prompt).not.toHaveBeenCalled();
    expect(d.clearFaceOverrides).not.toHaveBeenCalled();
  });

  it("unassigning (None) is the same path", async () => {
    const d = deps();
    await assignBodyWithOverridePolicy("body1", null, d.deps);
    expect(d.assignBody).toHaveBeenCalledWith("body1", null);
  });

  it("KEEP assigns the body and leaves every override standing", async () => {
    const prompt = vi.fn(async (): Promise<OverrideChoice> => "keep");
    const d = deps({ boundOverrides: () => ["el_a", "el_b"], prompt });
    const outcome = await assignBodyWithOverridePolicy("body1", STEEL.id, d.deps);

    expect(outcome).toBe("kept-overrides");
    expect(d.deps.assignBody).toHaveBeenCalledWith("body1", STEEL.id);
    expect(d.clearFaceOverrides).not.toHaveBeenCalled();
  });

  it("REPLACE assigns the body and clears exactly the bound ids", async () => {
    const prompt = vi.fn(async (): Promise<OverrideChoice> => "replace");
    const d = deps({ boundOverrides: () => ["el_a", "el_b"], prompt });
    const outcome = await assignBodyWithOverridePolicy("body1", STEEL.id, d.deps);

    expect(outcome).toBe("replaced-overrides");
    expect(d.clearFaceOverrides).toHaveBeenCalledWith(["el_a", "el_b"]);
  });

  it("CANCEL writes nothing at all", async () => {
    const prompt = vi.fn(async (): Promise<OverrideChoice> => "cancel");
    const d = deps({ boundOverrides: () => ["el_a"], prompt });
    const outcome = await assignBodyWithOverridePolicy("body1", STEEL.id, d.deps);

    expect(outcome).toBe("cancelled");
    expect(d.assignBody).not.toHaveBeenCalled();
    expect(d.clearFaceOverrides).not.toHaveBeenCalled();
  });

  it("asks about the BOUND overrides only — the prompt names what is on screen", async () => {
    const prompt = vi.fn(async (): Promise<OverrideChoice> => "keep");
    const d = deps({ boundOverrides: () => ["el_a"], prompt, materialName: () => "Brass" });
    await assignBodyWithOverridePolicy("body1", STEEL.id, d.deps, "Bracket");

    expect(prompt).toHaveBeenCalledWith({
      bodyId: "body1",
      bodyLabel: "Bracket",
      materialId: STEEL.id,
      materialName: "Brass",
      overrideElementIds: ["el_a"],
    });
  });
});

describe("the live wiring", () => {
  it("reads bound overrides off the store and raises the real dialog", async () => {
    renderStore.getState().reset();
    renderDialogStore.getState().reset();
    renderStore.getState().reportBoundFaceOverrides("body1", ["el_a"]);

    const live = defaultOverridePolicyDeps();
    expect(live.boundOverrides("body1")).toEqual(["el_a"]);
    expect(live.materialName(null)).toBe("None");
    // An id naming nothing in the library resolves to "None" rather than to a
    // fabricated name — same rule `materialQuery` follows for a dangling id.
    expect(live.materialName("mat_missing")).toBe("None");

    const answered = live.prompt({
      bodyId: "body1",
      materialId: null,
      materialName: "None",
      overrideElementIds: ["el_a"],
    });
    expect(renderDialogStore.getState().override?.request.bodyId).toBe("body1");
    renderDialogStore.getState().answerOverride("cancel");
    await expect(answered).resolves.toBe("cancel");

    renderStore.getState().reset();
  });

  it("a superseding request settles the one it replaced, so no caller hangs", async () => {
    renderDialogStore.getState().reset();
    const first = renderDialogStore.getState().requestOverrideChoice({
      bodyId: "body1",
      materialId: null,
      materialName: "None",
      overrideElementIds: ["el_a"],
    });
    const second = renderDialogStore.getState().requestOverrideChoice({
      bodyId: "body2",
      materialId: null,
      materialName: "None",
      overrideElementIds: ["el_b"],
    });

    await expect(first).resolves.toBe("cancel");
    renderDialogStore.getState().answerOverride("replace");
    await expect(second).resolves.toBe("replace");
  });
});
