/*
 * Modeling interaction GOLDEN probe.
 *
 * Compares production behavior against the frozen TARGET contract in
 * src/test/contracts/modelingInteractionContract.ts. RED until the UX program
 * closes each gap.
 *
 * The CONTRACT rows are unchanged by H7 — `visibleCancel: true` is still true,
 * because Cancel is still visible; it moved from the floating label to the
 * stable operation strip. Only this PROBE changed, from rendering
 * `ModelToolChips` alone to rendering the production shell region, which is what
 * lets it see review R05's "two confirmation surfaces" defect at all.
 * (TODO.md SESSION 37 H7, spec §4.1/§4.2.)
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act, within } from "@testing-library/react";
import { MODELING_INTERACTION_CONTRACT } from "@/test/contracts/modelingInteractionContract";
import { ModelToolChips } from "@/features/toolbar/ModelToolChips";
import { ModelOperationBar } from "@/features/toolbar/ModelOperationBar";
import { ActiveToolInspector } from "@/features/inspector/ActiveToolInspector";
import { toolChipStore } from "@/stores/toolChipStore";
import { toolChipPlacementStore } from "@/stores/toolChipPlacementStore";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";

const WORLD: [number, number, number] = [0, 0, 0];

function fakeEngine(): ViewportEngine {
  return {
    mountChip: (_id: string, el: HTMLElement) => document.body.appendChild(el),
    unmountChip: (_id: string, el: HTMLElement) => el.remove(),
    invalidate: () => undefined,
  } as unknown as ViewportEngine;
}

/**
 * The PRODUCTION shell region for an armed model operation (TODO.md SESSION 37
 * H7, spec §4.1/§4.2): the anchored parameter label, the stable operation strip
 * and the inspector, exactly the three contributions `contributeModelingUi`
 * mounts. Rendering `ModelToolChips` alone is what let review R05's defect hide
 * — a probe on one surface cannot see a second confirmation surface beside it.
 */
const renderShellRegion = () =>
  render(
    <>
      <ModelToolChips />
      <ModelOperationBar />
      <ActiveToolInspector />
    </>,
  );

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

  it("every row declares Enter support", () => {
    for (const row of MODELING_INTERACTION_CONTRACT) {
      expect(row.enterSupport, `${row.tool} must commit on Enter`).toBe(true);
    }
  });

  // The two columns added 2026-08-14 are only meaningful if they track
  // `primaryParameter`: a tool with no primary numeric value has nothing to type
  // into and nothing to preview per keystroke.
  it("primaryEntry and livePreviewOnEdit follow primaryParameter", () => {
    for (const row of MODELING_INTERACTION_CONTRACT) {
      if (row.primaryParameter === null) {
        expect(row.primaryEntry, `${row.tool} has no primary value`).toBe("none");
        expect(row.livePreviewOnEdit, `${row.tool} has nothing to preview live`).toBe(false);
      } else {
        expect(row.primaryEntry, `${row.tool} must accept type-to-enter`).toBe("typeToEnter");
        expect(row.livePreviewOnEdit, `${row.tool} must preview every edit`).toBe(true);
      }
    }
  });
});

describe("modeling interaction contract — shell-region probe", () => {
  beforeEach(() => {
    setViewportEngine(fakeEngine());
    toolChipStore.getState().clear();
    toolChipPlacementStore.getState().reset();
  });
  afterEach(() => {
    setViewportEngine(null);
    toolChipStore.getState().clear();
    toolChipPlacementStore.getState().reset();
  });

  it.each([
    { tool: "booleanOp", setup: () => toolChipStore.getState().showBoolean("Union", WORLD, { onOp: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
    {
      tool: "linearPattern",
      setup: () => toolChipStore.getState().showLinearPattern("X", 3, 20, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onSpacing: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }),
    },
    {
      tool: "circularPattern",
      setup: () => toolChipStore.getState().showCircularPattern("Z", 4, 360, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onAngle: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }),
    },
    { tool: "mirror", setup: () => toolChipStore.getState().showMirror("XY", WORLD, { onPlane: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
  ])("$tool has a visible cancel control", ({ setup }) => {
    renderShellRegion();
    act(() => setup());
    expect(screen.getByRole("button", { name: /cancel|✕/i })).toBeInTheDocument();
  });

  it.each([
    { tool: "extrude", setup: () => toolChipStore.getState().showExtrude(10, WORLD, { onValue: vi.fn(), onSymmetric: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
    { tool: "filletRadius", setup: () => toolChipStore.getState().showFillet(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }) },
    { tool: "shellThickness", setup: () => toolChipStore.getState().showShell(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }) },
  ])("$tool has a visible cancel control", ({ setup }) => {
    renderShellRegion();
    act(() => setup());
    expect(screen.getByRole("button", { name: /cancel|✕/i })).toBeInTheDocument();
  });

  /*
   * Review R05's regression gate. Before H7 the floating panel kept its own
   * ✓/✕ while `ModelOperationBar` added Done/Cancel beside it, so an armed
   * operation offered TWO confirmation surfaces. The contract rows are
   * unchanged — `visibleCancel: true` is still true, in the strip.
   *
   * `gear` is deliberately outside this set: `GearPropertiesPanel` keeps its own
   * Apply/Cancel in the inspector (it predates H7 and was left alone), so the
   * generator is the one operation with a second pair. Its Apply now routes
   * through the same `requestConfirm` gate.
   * (TODO.md SESSION 37 H7, spec §4.1/§4.2.)
   */
  it.each([
    { tool: "extrude", setup: () => toolChipStore.getState().showExtrude(10, WORLD, { onValue: vi.fn(), onSymmetric: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
    { tool: "filletRadius", setup: () => toolChipStore.getState().showFillet(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }) },
    { tool: "mirror", setup: () => toolChipStore.getState().showMirror("XY", WORLD, { onPlane: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
  ])("$tool offers exactly one Done/Cancel pair in the whole shell region", ({ setup }) => {
    renderShellRegion();
    act(() => setup());
    const strip = screen.getByTestId("model-operation-bar");
    expect(screen.getAllByRole("button", { name: /^(done|confirm|✓|apply)$/i })).toEqual([
      within(strip).getByTestId("model-operation-done"),
    ]);
    expect(screen.getAllByRole("button", { name: /^(cancel|✕)$/i })).toEqual([
      within(strip).getByTestId("model-operation-cancel"),
    ]);
  });

  it("the floating label carries the parameter only — no grip, dock, summary, validation or confirm", () => {
    renderShellRegion();
    act(() =>
      toolChipStore.getState().showFillet(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }),
    );
    act(() => toolChipStore.getState().setResultSummary("Fillet · 1 body"));
    act(() =>
      toolChipStore.getState().setRangeValidation({ status: "invalid", draft: 99, message: "Too large" }),
    );

    const label = screen.getByTestId("model-tool-chip");
    for (const testid of [
      "chip-drag-handle",
      "chip-dock",
      "chip-return",
      "chip-result-summary",
      "tool-validation",
      "chip-confirm",
      "chip-cancel",
    ]) {
      expect(within(label).queryByTestId(testid), testid).toBeNull();
    }
    expect(within(label).queryAllByRole("button")).toEqual([]);
    // …and the parameter itself is still there, at label readability (§4.5).
    expect(within(label).getByLabelText("Radius (mm)")).toHaveValue("2");
  });

  /*
   * U0 red evidence. `Apply`, ✓ and Enter must call ONE `confirm` callback;
   * `Cancel`, ✕ and Escape must call ONE `cancel`. Boolean, both patterns and
   * mirror used to publish a separate `onApply` protocol instead, which is also
   * why they had no Enter path (see modelingInteraction.keyboard.probe). U2
   * deleted `onApply`: there is now one confirm slot and one cancel slot.
   */
  it.each([
    { tool: "booleanOp", setup: () => toolChipStore.getState().showBoolean("Union", WORLD, { onOp: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
    {
      tool: "linearPattern",
      setup: () =>
        toolChipStore
          .getState()
          .showLinearPattern("X", 3, 20, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onSpacing: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }),
    },
    {
      tool: "circularPattern",
      setup: () =>
        toolChipStore
          .getState()
          .showCircularPattern("Z", 4, 360, WORLD, { onAxis: vi.fn(), onCount: vi.fn(), onAngle: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }),
    },
    { tool: "mirror", setup: () => toolChipStore.getState().showMirror("XY", WORLD, { onPlane: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
    { tool: "extrude", setup: () => toolChipStore.getState().showExtrude(10, WORLD, { onValue: vi.fn(), onSymmetric: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }) },
    { tool: "filletRadius", setup: () => toolChipStore.getState().showFillet(2, WORLD, vi.fn(), { onConfirm: vi.fn(), onCancel: vi.fn() }) },
  ])("$tool chip confirms through the shared `onConfirm` protocol", ({ setup }) => {
    act(() => setup());
    const chip = toolChipStore.getState();
    expect(chip.onConfirm).toBeTypeOf("function");
    expect(chip.onCancel).toBeTypeOf("function");
  });
});
