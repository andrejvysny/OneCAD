/*
 * Roadmap WP0.3 — the table-driven terminal test, tool side.
 *
 * Companion to `src/features/inspector/regenTerminals.test.ts`: the same five
 * terminals, asserted against the modeling-tool consumer family instead of the
 * history one, so the two families cannot drift apart on what a terminal means.
 *
 * The defect this pins (R-04) is specific: `commitPattern` inspected only
 * `errorMessage`, so a regen that resolved with a `needsRepair` or `timeout`
 * terminal published a SUCCESS hint and moved the selection onto bodies the op
 * had not produced.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult, RegenTerminal } from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { ModelToolController } from "./ModelToolController";

interface Row {
  terminal: RegenTerminal;
  errorMessage?: string;
  /** Only a publish may move the selection onto the produced bodies. */
  selects: boolean;
  /** A failure is sticky and names a reason; a repair prompt is sticky too. */
  sticky: boolean;
  contains: string;
}

const TABLE: Row[] = [
  { terminal: "published", selects: true, sticky: false, contains: "Linear pattern" },
  { terminal: "noop", selects: false, sticky: false, contains: "Linear pattern" },
  { terminal: "needsRepair", selects: false, sticky: true, contains: "needs repair" },
  {
    terminal: "failed",
    errorMessage: "kernel refused",
    selects: false,
    sticky: true,
    contains: "kernel refused",
  },
  { terminal: "failed", selects: false, sticky: true, contains: "without a reason" },
  { terminal: "timeout", selects: false, sticky: true, contains: "did not complete in time" },
];

function result(row: Row): ApplyOperationResult {
  return {
    revision: 2,
    features: [],
    // A pattern's children: the source is body1, so body2 is what a publish
    // would select. Every other terminal must leave the selection alone.
    changedBodies: [{ bodyId: "body2", meshKey: "body2#0" }],
    removedBodies: [],
    terminal: row.terminal,
    errorMessage: row.errorMessage,
  };
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
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("the modeling tools report every regen terminal honestly", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let applyOperation: ReturnType<typeof vi.fn>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  beforeEach(() => {
    resetStores();
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
      features: [
        {
          id: "feat-lp",
          kind: "boolean",
          opType: "LinearPattern",
          label: "Linear Pattern",
          valueText: "×7",
          status: "ok",
        },
      ],
    });
    engineMock = makeEngineMock();
    applyOperation = vi.fn();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: {
        onPreviewResult: vi.fn(() => () => {}),
        applyOperation,
        getOperationParams: vi.fn(() =>
          Promise.resolve({
            sourceBodyId: "body1",
            direction: [0, 1, 0],
            spacing: { value: 33 },
            count: 7,
          }),
        ),
      } as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
  });

  async function commitPattern(row: Row): Promise<void> {
    applyOperation.mockResolvedValue(result(row));
    await controller.editLinearPatternFeature("feat-lp");
    await flush();
    toolChipStore.getState().onConfirm?.();
    await flush();
  }

  for (const row of TABLE) {
    const label = `${row.terminal}${row.errorMessage ? " with a message" : ""}`;
    it(`pattern commit · ${label}`, async () => {
      selectionStore.setState({ selected: [{ kind: "body", id: "body1" }] });

      await commitPattern(row);

      const hint = viewportStore.getState().statusHint;
      expect(hint?.message, "every terminal must say something").toContain(row.contains);
      expect(hint?.sticky ?? false).toBe(row.sticky);
      if (row.terminal === "failed" || row.terminal === "timeout") {
        expect(hint?.severity).toBe("error");
        expect(hint?.message).toContain("Pattern failed");
      }

      // The selection moves to the produced children ONLY on a publish. Every
      // other terminal leaves the user pointed where they were.
      const selected = selectionStore.getState().selected.map((r) => r.id);
      expect(selected).toEqual(row.selects ? ["body2"] : ["body1"]);

      // The tool always returns to select; a failure must not strand the chip.
      expect(toolStore.getState().modelTool).toBe("select");
      expect(toolChipStore.getState().kind).toBe("none");
    });
  }

  it("a failed commit leaves the authored values recoverable and stacks no duplicate", async () => {
    await commitPattern(TABLE[3]);
    expect(applyOperation).toHaveBeenCalledTimes(1);
    // The stored feature is untouched: a retry re-reads the same params.
    expect(documentStore.getState().features).toHaveLength(1);
    expect(documentStore.getState().features[0]?.valueText).toBe("×7");
  });
});
