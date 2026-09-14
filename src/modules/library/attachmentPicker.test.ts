/*
 * `deriveAttachment` (WP-F1.2) — the pure half of attachment picking: what a
 * classified pick becomes. The frame is the load-bearing part, because the
 * placement solver seats a component BY it; an attachment whose `accepts` did
 * not match the snap kind its own geometry produces would simply never snap.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import type { ClassifyResult } from "@/ipc/types";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { setViewportEngine } from "@/viewport/engineBridge";
import { classifySnapKind, attachmentAccepts } from "./placementSolver";
import {
  beginAttachmentPick,
  cancelAttachmentPick,
  deriveAttachment,
  perpendicularTo,
  PICK_REJECTED,
} from "./attachmentPicker";
import { configureAuthoringController } from "./authoringController";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { documentStore } from "@/stores/documentStore";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import {
  buildBodyObjects,
  disposeAll,
  setCurrentMeshPublication,
  swap,
  __resetRegistryForTests,
  type MeshEntry,
} from "@/viewport/mesh/meshRegistry";

const CIRCLE_EDGE: ClassifyResult = {
  kind: "edge",
  surfaceType: "",
  curveType: "circle",
  frame: { origin: [2, 2, 8], normal: null, axis: [0, 0, 1], radius: 3 },
};

describe("deriveAttachment", () => {
  it("authors a rim attachment at a circular edge's own centre", () => {
    // A circle has ONE meaningful centre, so the click position is not used —
    // unlike a face, where the clicked point picks out where the seat sits.
    const picked = deriveAttachment(CIRCLE_EDGE, [5, 5, 8]);
    expect(picked).toEqual({
      kind: "circularEdge",
      accepts: ["cylinder", "hole", "circularEdge"],
      frame: { origin: [2, 2, 8], z: [0, 0, 1], x: [1, 0, 0] },
    });
  });

  it("refuses geometry with no seatable frame rather than inventing one", () => {
    expect(
      deriveAttachment({ kind: "face", surfaceType: "plane", curveType: "", frame: null }, [0, 0, 0]),
    ).toBeNull();
    expect(
      deriveAttachment(
        {
          kind: "edge",
          surfaceType: "",
          curveType: "line",
          frame: { origin: [0, 0, 0], normal: null, axis: [1, 0, 0], radius: null },
        },
        [0, 0, 0],
      ),
    ).toBeNull();
  });

  it("falls back to world Y for the roll reference when z IS world X", () => {
    expect(perpendicularTo([1, 0, 0])).toEqual([0, 1, 0]);
    const picked = deriveAttachment(
      {
        kind: "face",
        surfaceType: "cylinder",
        curveType: "",
        frame: { origin: [0, 0, 0], normal: null, axis: [1, 0, 0], radius: 2 },
      },
      [5, 1, 1],
    );
    expect(picked?.frame).toEqual({ origin: [5, 0, 0], z: [1, 0, 0], x: [0, 1, 0] });
  });

  it("produces `accepts` the snap solver actually admits, for every picked kind", () => {
    const cases: ClassifyResult[] = [
      { kind: "face", surfaceType: "plane", curveType: "", frame: { origin: [0, 0, 0], normal: [0, 0, 1], axis: null, radius: null } },
      { kind: "face", surfaceType: "cylinder", curveType: "", frame: { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius: 3 } },
      CIRCLE_EDGE,
    ];
    for (const classify of cases) {
      const picked = deriveAttachment(classify, [0, 0, 0]);
      const snapKind = classifySnapKind(classify);
      expect(picked).not.toBeNull();
      expect(snapKind).not.toBeNull();
      expect(attachmentAccepts(picked!.accepts, snapKind!)).toBe(true);
    }
  });
});

/*
 * R1(a) MAJOR 5 — authoring an attachment off a stale pick.
 *
 * `classifyElement` resolves a snapshot-scoped `f:N` against the HEAD, so a
 * click on a body whose replacement failed — still drawn, demoted to
 * `stale-inspection-only` — would author an attachment FRAME measured on
 * whatever the head calls that ordinal now. That frame is then baked into a
 * saved component, which makes it the most durable kind of wrong bind there is.
 */
describe("attachmentPicker pick currency (PR-01)", () => {
  const PUBLICATION = {
    documentId: "doc-1",
    runtimeSession: "runtime-1",
    snapshotId: 7,
    generation: 3,
  } as const;

  function installBody(): MeshEntry {
    const entry = buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body_1", 1, undefined, undefined, PUBLICATION,
    );
    swap("body_1", entry);
    setCurrentMeshPublication(PUBLICATION);
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: { body_1: { id: "body_1", name: "Body", visible: true } },
    });
    return entry;
  }

  /** Arm the picker over one hit and return everything a case asserts on. */
  function arm(entry: MeshEntry) {
    const classifyElement = vi.fn(async (): Promise<ClassifyResult> => ({
      kind: "edge",
      surfaceType: "",
      curveType: "circle",
      frame: { origin: [2, 2, 8], normal: null, axis: [0, 0, 1], radius: 3 },
    }));
    setViewportEngine({
      setOrbitSuppressed: vi.fn(),
      probePick: vi.fn(() => ({
        bodyId: "body_1",
        kind: "edge",
        elementId: undefined,
        topoKey: "e:1",
        worldPos: { x: 1, y: 2, z: 3 },
        entry,
      })),
    } as unknown as ViewportEngine);
    configureAuthoringController({ geometryQuery: { classifyElement } });
    const onPick = vi.fn();
    const onReject = vi.fn();
    expect(beginAttachmentPick({ onPick, onReject, onExit: vi.fn() })).toBe(true);
    return { classifyElement, onPick, onReject };
  }

  const settle = async (): Promise<void> => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
  };

  afterEach(() => {
    cancelAttachmentPick();
    configureAuthoringController(null);
    setViewportEngine(null);
    setCurrentMeshPublication(null);
    disposeAll();
    __resetRegistryForTests();
  });

  it("classifies a click on a CURRENT body", async () => {
    const { classifyElement, onPick, onReject } = arm(installBody());

    window.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, clientY: 10 }));
    await settle();

    expect(classifyElement).toHaveBeenCalledWith("body_1", "", "e:1");
    expect(onPick).toHaveBeenCalledTimes(1);
    expect(onReject).not.toHaveBeenCalled();
  });

  it("refuses a click on a stale-inspection-only body without classifying it", async () => {
    const entry = installBody();
    entry.displayState = "stale-inspection-only";
    const { classifyElement, onPick, onReject } = arm(entry);

    window.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, clientY: 10 }));
    await settle();

    expect(classifyElement).not.toHaveBeenCalled();
    expect(onPick).not.toHaveBeenCalled();
    expect(onReject).toHaveBeenCalledWith(PICK_REJECTED);
  });
});
