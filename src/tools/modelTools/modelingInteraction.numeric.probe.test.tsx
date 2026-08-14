/*
 * Modeling interaction contract — NUMERIC probe (U0 red evidence, closed by U3).
 *
 * Covers the two columns added to the frozen contract on 2026-08-14:
 *
 *   primaryEntry: "typeToEnter"  — a printable character while the tool is armed
 *                                  routes to the primary numeric value with no
 *                                  prior click (spec D4).
 *   livePreviewOnEdit: true      — every parseable change updates the preview;
 *                                  blur must never be the only mechanism (D3).
 *
 * The probe drives BOTH halves of the real path, because that is where the
 * contract lives: the controller owns the canvas keystroke (the chip must not
 * grow its own keyboard behaviour — that is exactly the fragmentation this
 * program removes), and the rendered chip owns everything typed after it, since
 * the field then has focus. Simulating the second character on `window` would be
 * testing something no browser does.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";

import type { CadClient } from "@/ipc/client";
import { ModelToolChips } from "@/features/toolbar/ModelToolChips";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { setViewportEngine } from "@/viewport/engineBridge";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import {
  buildBodyObjects,
  swap as swapMesh,
  __resetRegistryForTests,
} from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { ModelToolController } from "./ModelToolController";

/** A pattern ghost is built from the SOURCE body's mesh, so the registry has to
 *  hold one or `rebuildLinearGhost` returns before it reaches the engine. */
function registerBody(bodyId: string, blob: ArrayBuffer): void {
  swapMesh(bodyId, buildBodyObjects(parseMeshPayload(blob), bodyId, 1));
}

function makeEngineMock() {
  return {
    showGhostPreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    probePick: vi.fn(() => null),
    hideExtrudePreview: vi.fn(),
    clearPreviewBody: vi.fn(),
    setPreviewTint: vi.fn(),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    hideRevolvePreview: vi.fn(),
    hideRegionPick: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    // The chip host the React bridge mounts into.
    mountChip: (_id: string, el: HTMLElement) => document.body.appendChild(el),
    unmountChip: (_id: string, el: HTMLElement) => el.remove(),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/** One printable character, delivered exactly as a canvas-focused keypress. */
function typeOnCanvas(key: string): void {
  window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
}

interface Row {
  tool: string;
  opType: string;
  stored: Record<string, unknown>;
  arm: (c: ModelToolController, featureId: string) => Promise<void>;
  /** What the primary value reads before typing. */
  seeded: number;
}

const ROWS: Row[] = [
  {
    tool: "linearPattern",
    opType: "LinearPattern",
    stored: {
      sourceBodyId: "body1",
      direction: [1, 0, 0],
      spacing: { value: 20 },
      count: 3,
      resultPolicyVersion: 2,
      fuseResult: false,
    },
    arm: (c, id) => c.editLinearPatternFeature(id),
    seeded: 20,
  },
  {
    tool: "circularPattern",
    opType: "CircularPattern",
    stored: {
      sourceBodyId: "body1",
      axisDirection: [0, 0, 1],
      axisOrigin: [0, 0, 0],
      angleDeg: { value: 360 },
      count: 4,
      resultPolicyVersion: 2,
      fuseResult: false,
    },
    arm: (c, id) => c.editCircularPatternFeature(id),
    seeded: 360,
  },
];

describe("modeling interaction contract — type-to-enter and live preview", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let engineMock: ReturnType<typeof makeEngineMock>;

  function build(row: Row): void {
    engineMock = makeEngineMock();
    setViewportEngine(engineMock as unknown as ViewportEngine);
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
      features: [
        {
          id: "feat-1",
          kind: "boolean",
          opType: row.opType,
          label: row.tool,
          valueText: "",
          status: "ok",
        },
      ],
    });
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: {
        onPreviewResult: vi.fn(() => () => {}),
        applyOperation: vi.fn(),
        applyEditCommand: vi.fn(),
        getOperationParams: vi.fn(() => Promise.resolve({ ...row.stored })),
      } as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  beforeEach(() => {
    resetStores();
    __resetRegistryForTests();
    registerBody("body1", makeBoxMesh());
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    setViewportEngine(null);
    container.remove();
  });

  for (const row of ROWS) {
    it(`${row.tool} · control: the chip publishes the seeded primary value`, async () => {
      build(row);
      render(<ModelToolChips />);
      await act(async () => {
        await row.arm(controller, "feat-1");
        await flush();
      });

      // Proves the fixture armed and that `value` IS the primary parameter slot
      // the router must write to. Without this, a red below could be a bad arm.
      expect(toolChipStore.getState().value).toBe(row.seeded);
      expect(screen.getByLabelText("Dimension value")).toHaveValue(String(row.seeded));
    });

    it(`${row.tool} · typing routes to the primary value without a prior click`, async () => {
      build(row);
      render(<ModelToolChips />);
      await act(async () => {
        await row.arm(controller, "feat-1");
        await flush();
      });

      // Nothing was clicked: the canvas has focus, the chip field does not.
      act(() => typeOnCanvas("2"));

      const field = screen.getByLabelText("Dimension value");
      // The typed character REPLACED the formatted value rather than appending,
      // and the field took focus so the next character edits normally.
      expect(field).toHaveValue("2");
      expect(document.activeElement).toBe(field);
      expect(toolChipStore.getState().value).toBe(2);

      // …and it does, in the field, exactly as in a browser.
      act(() => {
        fireEvent.change(field, { target: { value: "25" } });
      });
      expect(toolChipStore.getState().value).toBe(25);
    });

    it(`${row.tool} · every parseable keystroke updates the preview`, async () => {
      build(row);
      render(<ModelToolChips />);
      await act(async () => {
        await row.arm(controller, "feat-1");
        await flush();
      });

      // The arm itself draws one ghost; only keystrokes after that count.
      const before = engineMock.showGhostPreview.mock.calls.length;
      act(() => typeOnCanvas("2"));
      const field = screen.getByLabelText("Dimension value");
      act(() => {
        fireEvent.change(field, { target: { value: "25" } });
      });

      // Two parseable values passed through ("2", then "25"), so the preview must
      // have been rebuilt — the exact count is a scheduling detail, but zero means
      // blur is still the only update mechanism.
      expect(engineMock.showGhostPreview.mock.calls.length).toBeGreaterThan(before);
    });

    it(`${row.tool} · partial text previews nothing and never clamps`, async () => {
      build(row);
      render(<ModelToolChips />);
      await act(async () => {
        await row.arm(controller, "feat-1");
        await flush();
      });

      const field = screen.getByLabelText("Dimension value");
      act(() => {
        fireEvent.change(field, { target: { value: "12abc" } });
      });

      // The text stays exactly as typed — editable, not rewritten, not clamped —
      // and the FSM still holds the last valid value.
      expect(field).toHaveValue("12abc");
      expect(toolChipStore.getState().value).toBe(row.seeded);
    });
  }
});
