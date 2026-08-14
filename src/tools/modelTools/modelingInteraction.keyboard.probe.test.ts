/*
 * Modeling interaction contract — KEYBOARD probe (U0 red evidence).
 *
 * The frozen contract declares `enterSupport: true` for all twelve tools
 * (src/test/contracts/modelingInteractionContract.ts). Nothing read that column
 * until now, and `ModelToolController.onKeyDown` routes Enter for extrude,
 * revolve, fillet/chamfer, shell, offsetFace, hole, transform, datum and the
 * region pick ONLY — boolean, linearPattern, circularPattern and mirror have no
 * Enter path at all. These four rows are therefore RED until U2 lands the
 * table-driven Enter/Escape router.
 *
 * Each tool gets a PAIR of specs so a red can never be blamed on the harness:
 *
 *   1. a control that fires the chip's own commit callback and asserts exactly
 *      one commit reaches the client — proves the fixture really armed;
 *   2. the probe that dispatches a real capture-phase Enter on `window` and
 *      asserts the same single commit.
 *
 * Control green + probe red ⇒ the gap is the missing key routing, nothing else.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult } from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";

const okResult = (): ApplyOperationResult => ({
  revision: 7,
  features: [],
  changedBodies: [{ bodyId: "body2", meshKey: "body2#0" }],
  removedBodies: [],
  terminal: "published",
});

/** `onToolChange` tears every OTHER tool down before arming, so the fake needs
 *  every method those cancel paths touch. */
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

interface Row {
  /** The contract row this probe covers. */
  tool: string;
  /** The projected feature the re-edit entry point reads. */
  opType: string;
  /** What `get_operation_params` returns for it. */
  stored: Record<string, unknown>;
  arm: (c: ModelToolController, featureId: string) => Promise<void>;
}

const ROWS: Row[] = [
  {
    tool: "boolean",
    opType: "Boolean",
    stored: { operation: "Union", targetBodyId: "body1", toolBodyId: "body2-retired" },
    arm: (c, id) => c.editBooleanFeature(id),
  },
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
  },
  {
    tool: "mirror",
    opType: "MirrorBody",
    stored: {
      sourceBodyId: "body1",
      planeNormal: [0, 0, 1],
      planePoint: [0, 0, 0],
      fuseWithOriginal: false,
    },
    arm: (c, id) => c.editMirrorFeature(id),
  },
];

describe("modeling interaction contract — Enter commits every armed model tool", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let applyOperation: ReturnType<typeof vi.fn>;
  let applyEditCommand: ReturnType<typeof vi.fn>;

  /** Either shape is a commit reaching the backend; the contract cares that one
   *  fires exactly once, not which command carries it. */
  const commitCount = (): number => applyOperation.mock.calls.length + applyEditCommand.mock.calls.length;

  function build(row: Row): void {
    applyOperation = vi.fn(() => Promise.resolve(okResult()));
    applyEditCommand = vi.fn(() => Promise.resolve(okResult()));
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
      engine: makeEngineMock() as unknown as ViewportEngine,
      client: {
        onPreviewResult: vi.fn(() => () => {}),
        applyOperation,
        applyEditCommand,
        getOperationParams: vi.fn(() => Promise.resolve({ ...row.stored })),
      } as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  for (const row of ROWS) {
    it(`${row.tool} · control: the chip's own commit callback reaches the client once`, async () => {
      build(row);
      await row.arm(controller, "feat-1");
      await flush();

      const chip = toolChipStore.getState();
      // Whichever callback this tool currently publishes — `onApply` today,
      // `confirm` after U2 — the control only proves the fixture armed.
      const commit = chip.onConfirm;
      expect(commit, `${row.tool} must expose a commit callback once armed`).toBeTypeOf("function");
      commit?.();
      await flush();

      expect(commitCount()).toBe(1);
    });

    it(`${row.tool} · Enter commits the armed tool`, async () => {
      build(row);
      await row.arm(controller, "feat-1");
      await flush();

      // Capture-phase on `window`, exactly where the controller listens, with no
      // editable focus — the same event a canvas-focused Enter produces.
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await flush();

      expect(commitCount()).toBe(1);
    });
  }
});
