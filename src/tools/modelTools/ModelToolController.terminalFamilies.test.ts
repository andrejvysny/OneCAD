/*
 * Regen-terminal honesty across EVERY modeling commit family (U0 red evidence).
 *
 * `ModelToolController.terminals.test.ts` pins the six terminals for the pattern
 * family, and `regenTerminals.test.ts` does the same for the three history
 * families. Those two were WP0.3's whole deliverable — but nine other commit
 * paths never adopted `classifyRegen` and still infer success from a resolved
 * promise (or, for extrude/revolve, from a non-empty body count):
 *
 *   commitTransform (:5789) · boolean re-edit (:7443) · hole re-edit (:4487)
 *   commitShellEdit (:3518) · commitOffsetFaceEdit (:4136) · commitEdgeOpEdit (:7193)
 *
 * A regen that RESOLVES carrying `terminal:"failed"` + `errorMessage`, or
 * `terminal:"needsRepair"`, therefore reads as success: the hint is a
 * non-sticky success line and the document is announced as changed.
 *
 * These specs pin the minimum honest behaviour — a failure is STICKY and names
 * its reason — for every remaining family. RED until U1 routes them through
 * `classifyRegen`. Each family carries a `published` control so a red can never
 * be blamed on the fixture.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  RegenTerminal,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";

/** The minimum sketch a profile-bound arm (extrude/revolve re-edit) needs to read. */
const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};
const REGION: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};
/** A vertical line clear of the profile — a legal revolve axis. */
const AXIS_LINE = {
  id: "e1",
  type: "Line" as const,
  p0: [-5, 0] as [number, number],
  p1: [-5, 20] as [number, number],
};
const SESSION: SketchSession = {
  sketchId: "sk",
  plane: PLANE,
  entities: [AXIS_LINE],
  constraints: [],
  dof: 0,
  status: "FullyConstrained",
};

function makeEngineMock() {
  return {
    // Extrude/revolve preview surface — only reached by the two profile-bound rows.
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => null),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    screenToPlaneOn: vi.fn((_plane: SketchPlane, x: number, y: number) => ({ x, y })),
    planePixelWorld: vi.fn(() => 1),
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
    showTransformGizmo: vi.fn(),
    hideTransformGizmo: vi.fn(),
    setTransformGizmoHover: vi.fn(),
    setTransformGizmoActive: vi.fn(),
    hitTransformGizmo: vi.fn(() => null),
    getBoundsForBodies: vi.fn(() => null),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 5));

interface Family {
  name: string;
  /** The projected feature row the re-edit entry point reads. */
  feature: { kind: string; opType: string };
  stored: Record<string, unknown>;
  arm: (c: ModelToolController, featureId: string) => Promise<void> | void;
}

const FAMILIES: Family[] = [
  {
    name: "transform re-edit",
    feature: { kind: "boolean", opType: "TransformBody" },
    stored: {
      targets: ["body1"],
      translate: [{ value: 30 }, { value: 0 }, { value: 0 }],
      rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: { value: 0 } },
      copy: false,
    },
    arm: (c, id) => c.editTransformFeature(id),
  },
  {
    name: "boolean re-edit",
    feature: { kind: "boolean", opType: "Boolean" },
    stored: { operation: "Union", targetBodyId: "body1", toolBodyId: "body2-retired" },
    arm: (c, id) => c.editBooleanFeature(id),
  },
  {
    name: "shell re-edit",
    feature: { kind: "fillet", opType: "Shell" },
    stored: { thickness: { value: 2 }, openFaces: ["f:2"], targetBodyId: "body1" },
    arm: (c, id) => c.editShellFeature(id),
  },
  {
    name: "offsetFace re-edit",
    feature: { kind: "fillet", opType: "OffsetFace" },
    stored: {
      faceIds: ["el-f2"],
      faces: [{ primary: { bodyId: "body1", elementId: "el-f2", kind: "face" } }],
      distance: { value: 2.5 },
      distanceType: "Offset",
      chainTangentFaces: true,
      targetBodyId: "body1",
    },
    arm: (c, id) => c.editOffsetFaceFeature(id),
  },
  {
    name: "fillet re-edit",
    feature: { kind: "fillet", opType: "Fillet" },
    stored: {
      radius: { value: 2 },
      edgeIds: ["el-edge-5"],
      edges: [{ primary: { bodyId: "body1", elementId: "el-edge-5", kind: "edge" } }],
    },
    arm: (c, id) => c.editEdgeOpFeature(id, "Fillet"),
  },
  {
    name: "extrude re-edit",
    feature: { kind: "extrude", opType: "Extrude" },
    stored: {
      profile: { sketchId: "sk", regionId: "r0" },
      distance: { value: 20 },
      extrudeMode: "Blind",
      booleanMode: "NewBody",
    },
    arm: (c, id) => {
      c.editExtrudeFeature(id);
    },
  },
  {
    name: "revolve re-edit",
    feature: { kind: "revolve", opType: "Revolve" },
    stored: {
      profile: { sketchId: "sk", regionId: "r0" },
      angleDeg: { value: 90 },
      axis: { kind: "sketchLine", sketchId: "sk", entityId: "e1" },
      booleanMode: "NewBody",
    },
    arm: (c, id) => {
      c.editRevolveFeature(id);
    },
  },
  {
    name: "hole re-edit",
    feature: { kind: "boolean", opType: "Hole" },
    stored: {
      targetBodyId: "body1",
      face: { primary: { bodyId: "body1", elementId: "el-face-2", kind: "face" } },
      point: [3, 4, 25],
      holeType: "simple",
      diameter: { value: 8.4 },
      depth: { value: 16 },
    },
    arm: (c, id) => c.editHoleFeature(id),
  },
];

interface TerminalRow {
  label: string;
  terminal: RegenTerminal;
  errorMessage?: string;
  /** A failure must be sticky; a success must not be. */
  sticky: boolean;
}

const TERMINALS: TerminalRow[] = [
  { label: "control · published", terminal: "published", sticky: false },
  { label: "failed with a reason", terminal: "failed", errorMessage: "kernel refused", sticky: true },
  { label: "needsRepair", terminal: "needsRepair", sticky: true },
];

describe("every modeling commit family reports a regen terminal honestly", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;
  /** The live mesh-ingest listener. Extrude/revolve hold their completion until
   *  every committed body has entered the scene, so the fixture must deliver it. */
  let bodyLoaded: ((id: string) => void) | null = null;

  function build(family: Family, row: TerminalRow): void {
    const result: ApplyOperationResult = {
      revision: 9,
      features: [],
      // A body the operation did NOT produce: a family that treats this as a
      // success will happily hydrate and select it.
      changedBodies: [{ bodyId: "body9", meshKey: "body9#0" }],
      removedBodies: [],
      terminal: row.terminal,
      errorMessage: row.errorMessage,
    };
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
      features: [
        {
          id: "feat-1",
          kind: family.feature.kind as never,
          opType: family.feature.opType,
          label: family.name,
          valueText: "2.0 mm",
          status: "ok",
        },
      ],
    });
    controller = new ModelToolController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client: {
        onPreviewResult: vi.fn(() => () => {}),
        applyOperation: vi.fn(() => Promise.resolve(result)),
        applyEditCommand: vi.fn(() => Promise.resolve(result)),
        getOperationParams: vi.fn(() => Promise.resolve({ ...family.stored })),
        getSketchRegions: vi.fn(() => Promise.resolve({ regions: [REGION] })),
        getSketch: vi.fn(() => Promise.resolve(SESSION)),
        finishSketch: vi.fn(() => Promise.resolve({ regions: [REGION] })),
        canFoldTransform: vi.fn(() => Promise.resolve(null)),
        beginPreview: vi.fn(() => Promise.resolve({ sessionId: "pv-1", previewBodyId: "pb-1" })),
        updatePreview: vi.fn(),
        endPreview: vi.fn(() => Promise.resolve(result)),
      } as unknown as CadClient,
      container,
      onBodyLoaded: (cb) => {
        bodyLoaded = cb;
        return () => {
          bodyLoaded = null;
        };
      },
    });
  }

  beforeEach(() => {
    resetStores();
    bodyLoaded = null;
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  for (const family of FAMILIES) {
    for (const row of TERMINALS) {
      it(`${family.name} · ${row.label}`, async () => {
        build(family, row);
        await family.arm(controller, "feat-1");
        await flush();

        const chip = toolChipStore.getState();
        const commit = chip.onConfirm;
        expect(commit, `${family.name} must arm and expose a commit`).toBeTypeOf("function");
        commit?.();
        // The profile-bound families (extrude/revolve) commit through the preview
        // lane: the exact barrier resolves on a macrotask, then the apply, then a
        // wait for the committed body's mesh to enter the scene. Settle past all
        // three, delivering the ingest the real engine would.
        for (let i = 0; i < 8; i++) {
          await settle();
          bodyLoaded?.("body9");
        }

        const hint = viewportStore.getState().statusHint;
        expect(hint, "every terminal must say something").toBeTruthy();
        expect(hint?.sticky ?? false, `${row.terminal} stickiness (${hint?.message})`).toBe(row.sticky);
        if (row.errorMessage) {
          expect(hint?.message).toContain(row.errorMessage);
          expect(hint?.severity).toBe("error");
        }
      });
    }
  }
});
