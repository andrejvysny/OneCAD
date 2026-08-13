/*
 * Modeling interaction GOLDEN probe.
 *
 * Compares production behavior against the frozen TARGET contract in
 * src/test/contracts/modelingInteractionContract.ts. RED until the UX program
 * closes each gap.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { MODELING_INTERACTION_CONTRACT } from "@/test/contracts/modelingInteractionContract";
import { ModelToolChips } from "@/features/toolbar/ModelToolChips";
import { toolChipStore } from "@/stores/toolChipStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";

const WORLD: [number, number, number] = [0, 0, 0];

function fakeEngine(): ViewportEngine {
  return {
    mountChip: (_id: string, el: HTMLElement) => document.body.appendChild(el),
    unmountChip: (_id: string, el: HTMLElement) => el.remove(),
  } as unknown as ViewportEngine;
}

describe("modeling interaction contract", () => {
  it("every row has a unique tool id", () => {
    const ids = MODELING_INTERACTION_CONTRACT.map((r) => r.tool);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("every row declares click-away policy as 'cancel'", () => {
    for (const row of MODELING_INTERACTION_CONTRACT) {
      expect(row.clickAwayPolicy).toBe("cancel");
    }
  });

  it("every row declares visible cancel = true", () => {
    for (const row of MODELING_INTERACTION_CONTRACT) {
      expect(row.visibleCancel).toBe(true);
    }
  });
});

describe("modeling interaction contract — chip probe", () => {
  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
  });

  it.each([
    { tool: "booleanOp", setup: () => toolChipStore.getState().showBoolean("Union", WORLD, vi.fn(), vi.fn()) },
    {
      tool: "linearPattern",
      setup: () => toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onSpacing: vi.fn(), onApply: vi.fn() }),
    },
    {
      tool: "circularPattern",
      setup: () => toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onAngle: vi.fn(), onApply: vi.fn() }),
    },
    { tool: "mirror", setup: () => toolChipStore.getState().showMirror("XY", WORLD, { onPlane: vi.fn(), onApply: vi.fn() }) },
  ])("$tool chip has a visible cancel control", ({ setup }) => {
    render(<ModelToolChips />);
    act(() => setup());
    expect(screen.getByRole("button", { name: /cancel|✕/i })).toBeInTheDocument();
  });

  it.each([
    { tool: "extrude", setup: () => toolChipStore.getState().showExtrude(10, WORLD, { onValue: vi.fn(), onSymmetric: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
    { tool: "filletRadius", setup: () => toolChipStore.getState().showFillet(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }) },
    { tool: "shellThickness", setup: () => toolChipStore.getState().showShell(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }) },
  ])("$tool chip has a visible cancel control", ({ setup }) => {
    render(<ModelToolChips />);
    act(() => setup());
    expect(screen.getByRole("button", { name: /cancel|✕/i })).toBeInTheDocument();
  });
});
