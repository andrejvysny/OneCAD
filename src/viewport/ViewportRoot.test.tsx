/*
 * refFromModelHits — arbitrates a body face/edge PickHit against a coplanar
 * sketch-fill SketchStaticHit under the same pointer (W0.4 pick-through).
 *
 * Default (no Alt): a numerically coplanar sketch fill wins the tie
 * (`secondaryHitWins`), pinned by Picker.test.ts:76 and unchanged here.
 * `pickThrough` (Alt held) suppresses that tie-break so the body face/edge
 * underneath the fill becomes selectable — the sketch is only used when
 * there is no body hit at all.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

/*
 * The promote lane is spied, not replaced: the write-back specs below drive it
 * by hand, while the PR-01 proof specs need the REAL implementation (kept here
 * so they can restore it onto the same spy).
 */
const promoteLane = vi.hoisted(() => ({
  real: null as null | typeof import("@/ipc/promote").promoteRef,
}));
vi.mock("@/ipc/promote", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/ipc/promote")>();
  promoteLane.real = actual.promoteRef;
  return { ...actual, promoteRef: vi.fn() };
});

import * as THREE from "three";
import {
  applyRendererLifecycleHint,
  promotePick,
  refFromModelHits,
  viewportGeometryChip,
  ViewportRoot,
} from "./ViewportRoot";
import { ViewportEngine } from "./engine/ViewportEngine";
import { viewportStore } from "@/stores/viewportStore";
import { documentStore } from "@/stores/documentStore";
import { resetStores } from "@/test/resetStores";
import * as registry from "@/viewport/mesh/meshRegistry";
import type { MeshEntry } from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { promoteRef, STALE_PICK_HINT } from "@/ipc/promote";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import type { PromotedElement } from "@/ipc/types";
import type { PickHit } from "./engine/Picker";
import type { SketchStaticHit } from "./engine/SketchStaticLayer";

function bodyHit(distance = 100): PickHit {
  return {
    bodyId: "body1",
    kind: "face",
    topoKey: "f:0",
    distance,
    worldPos: new THREE.Vector3(0, 0, 0),
  };
}

function sketchHit(distance = 100.0005): SketchStaticHit {
  return { kind: "sketchRegion", sketchId: "sketch1", regionId: "r0", distance };
}

describe("refFromModelHits — pick-through arbitration (W0.4)", () => {
  it("tie + no Alt: the coplanar sketch fill wins (current behavior pinned)", () => {
    const ref = refFromModelHits(bodyHit(), sketchHit(), false);
    expect(ref?.kind).toBe("sketchRegion");
  });

  it("tie + Alt (pickThrough): the body face wins outright", () => {
    const ref = refFromModelHits(bodyHit(), sketchHit(), true);
    expect(ref?.kind).toBe("face");
    expect(ref?.bodyId).toBe("body1");
  });

  it("Alt with only a sketch hit (no body under the pointer): sketch ref", () => {
    const ref = refFromModelHits(null, sketchHit(), true);
    expect(ref?.kind).toBe("sketchRegion");
  });

  it("Alt with only a body hit: body ref", () => {
    const ref = refFromModelHits(bodyHit(), null, true);
    expect(ref?.kind).toBe("face");
    expect(ref?.bodyId).toBe("body1");
  });

  it("tie + no Alt against a body EDGE: the edge wins (sketch traces the boundary)", () => {
    const edge: PickHit = { ...bodyHit(), kind: "edge", topoKey: "e:3" };
    const ref = refFromModelHits(edge, sketchHit(), false);
    expect(ref?.kind).toBe("edge");
    expect(ref?.bodyId).toBe("body1");
  });

  it("sketch clearly in FRONT of a body edge: the sketch still wins", () => {
    const edge: PickHit = { ...bodyHit(200), kind: "edge", topoKey: "e:3" };
    const ref = refFromModelHits(edge, sketchHit(100), false);
    expect(ref?.kind).toBe("sketchRegion");
  });

  it("no hits at all: null regardless of Alt", () => {
    expect(refFromModelHits(null, null, false)).toBeNull();
    expect(refFromModelHits(null, null, true)).toBeNull();
  });
});

/*
 * viewportGeometryChip — which of the two mutually exclusive geometry-status
 * chips the viewport overlays. One slot, two independent facts, so the
 * precedence is worth pinning rather than re-deriving from JSX conditions.
 */
describe("viewportGeometryChip", () => {
  it("shows nothing until the engine is ready, whatever the state says", () => {
    expect(viewportGeometryChip(false, true, false, true)).toBeNull();
    expect(viewportGeometryChip(false, false, false, true)).toBeNull();
  });

  it("pending wins over cached — meshes are mid-flight, so nothing is 'last saved'", () => {
    expect(viewportGeometryChip(true, true, false, true)).toBe("pending");
    expect(viewportGeometryChip(true, true, false, false)).toBe("pending");
  });

  it("a sticky error hint suppresses pending, and does NOT promote cached in its place", () => {
    expect(viewportGeometryChip(true, true, true, true)).toBeNull();
  });

  it("cached shows ALONGSIDE an error hint — a failed regen over stale geometry is exactly the case to label", () => {
    // Suppressing it here would leave last-saved geometry looking authoritative
    // at the very moment the rebuild that would have replaced it failed.
    expect(viewportGeometryChip(true, false, true, true)).toBe("cached");
  });

  it("live geometry shows no chip at all", () => {
    expect(viewportGeometryChip(true, false, false, false)).toBeNull();
  });
});

// ── promotePick write-back (WP-U4 / D-5, Astra break F3) ────────────────────

describe("promotePick", () => {
  const faceRef = (topoKey: string, marker?: string): EntityRef => ({
    kind: "face",
    id: `body1#${topoKey}`,
    bodyId: "body1",
    topoKey,
    ...(marker ? { anchor: { worldPoint: [1, 2, 3] as [number, number, number] } } : {}),
  });

  beforeEach(() => {
    vi.mocked(promoteRef).mockReset();
    selectionStore.getState().set([]);
  });

  const client = {} as Parameters<typeof promotePick>[0];

  it("writes the minted ElementId back and LEAVES the pick-time label alone", async () => {
    // The mesh label the user clicked is what this publication draws through;
    // the promotion answers with the backend's own key for the same element.
    // Overwriting `topoKey` with it makes the highlight depend on a key the
    // displayed table may not carry at all.
    const ref = faceRef("f:4");
    selectionStore.getState().set([ref]);
    vi.mocked(promoteRef).mockResolvedValue({
      topoKey: "f:9",
      elementId: "el_a",
      kind: "face",
      bodyId: "body1",
    });

    promotePick(client, ref);
    await new Promise((r) => setTimeout(r, 0));

    expect(selectionStore.getState().selected[0].elementId).toBe("el_a");
    expect(selectionStore.getState().selected[0].topoKey).toBe("f:4");
    expect(selectionStore.getState().selected[0].id).toBe("body1#f:4");
  });

  it("leaves a FRESH pick that reuses the promoted ref's id untouched", async () => {
    // `EntityRef.id` is reusable: deselect and pick the same label again and the
    // new ref carries the same string. The reply belongs to the OBJECT it was
    // asked about, not to whatever currently holds its id.
    const stale = faceRef("f:4");
    selectionStore.getState().set([stale]);
    let release!: (v: PromotedElement) => void;
    vi.mocked(promoteRef).mockReturnValue(
      new Promise<PromotedElement>((r) => {
        release = r;
      }),
    );

    promotePick(client, stale);
    const fresh = faceRef("f:4", "fresh"); // same id, a DIFFERENT object
    selectionStore.getState().set([fresh]);
    release({ topoKey: "f:9", elementId: "el_a", kind: "face", bodyId: "body1" });
    await new Promise((r) => setTimeout(r, 0));

    expect(selectionStore.getState().selected).toEqual([fresh]);
  });
});

/*
 * VP-HARDENING PR-01 — a click on stale geometry SELECTS but never promotes.
 *
 * A body whose replacement failed keeps its old mesh on screen, demoted to
 * `stale-inspection-only` (spec §9): the user can still orbit it, hover it and
 * click it, because taking geometry away at the moment a rebuild fails is worse
 * than labelling it. What must NOT happen is that the same click mints a
 * persistent id — the ordinal it carries addresses a publication the document
 * has moved past, and an id minted from it names whatever the head calls that
 * ordinal now.
 *
 * Driven through the REAL `promoteRef` (the module is spied, not stubbed, at the
 * top of this file) so the proof is the one `refFromModelHits` actually attached
 * at hit time — not one the test invented.
 */
describe("promotePick proof gate (PR-01, spec §9)", () => {
  const PUBLICATION = {
    documentId: "doc-1",
    runtimeSession: "runtime-1",
    snapshotId: 7,
    generation: 3,
  } as const;

  function installBody(): MeshEntry {
    const entry = registry.buildBodyObjects(
      parseMeshPayload(makeBoxMesh()), "body1", 1, undefined, undefined, PUBLICATION,
    );
    registry.swap("body1", entry);
    registry.setCurrentMeshPublication(PUBLICATION);
    documentStore.setState({
      documentId: "doc-1",
      runtimeSession: "runtime-1",
      geometrySource: "live",
      bodies: { body1: { id: "body1", name: "Body", visible: true } },
    });
    return entry;
  }

  /** The hit a real click produces: the entry is stamped by `resolvePick`. */
  function hitOn(entry: MeshEntry): PickHit {
    return {
      bodyId: "body1",
      kind: "face",
      topoKey: "f:0",
      distance: 10,
      worldPos: new THREE.Vector3(1, 2, 3),
      entry,
    };
  }

  beforeEach(() => {
    resetStores();
    registry.disposeAll();
    viewportStore.getState().setStatusHint(null);
    selectionStore.getState().set([]);
    vi.mocked(promoteRef).mockReset();
    vi.mocked(promoteRef).mockImplementation((client, ref) => promoteLane.real!(client, ref));
  });

  it("a click on a body whose replacement FAILED selects but refuses to promote", async () => {
    const entry = installBody();
    entry.displayState = "stale-inspection-only";
    const promoteSelection = vi.fn();
    const ref = refFromModelHits(hitOn(entry), null, false)!;

    // The click's own selection write — inspection is allowed on stale geometry.
    selectionStore.getState().set([ref]);
    promotePick({ promoteSelection } as unknown as Parameters<typeof promotePick>[0], ref);
    await new Promise((r) => setTimeout(r, 0));

    expect(promoteSelection).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe(STALE_PICK_HINT);
    expect(selectionStore.getState().selected).toEqual([ref]);
    expect(selectionStore.getState().selected[0].elementId).toBeUndefined();
  });

  it("a click on a CURRENT body promotes with the pick-time proof's own fence", async () => {
    const entry = installBody();
    const promoteSelection = vi.fn(async () => [
      { topoKey: "f:0", elementId: "el_a", kind: "face", bodyId: "body1" },
    ]);
    const ref = refFromModelHits(hitOn(entry), null, false)!;
    selectionStore.getState().set([ref]);

    promotePick({ promoteSelection } as unknown as Parameters<typeof promotePick>[0], ref);
    await new Promise((r) => setTimeout(r, 0));

    expect(promoteSelection).toHaveBeenCalledWith(
      "body1",
      [{ topoKey: "f:0", kind: "face", anchor: { worldPoint: [1, 2, 3] } }],
      7,
      "runtime-1",
    );
    expect(selectionStore.getState().selected[0].elementId).toBe("el_a");
    expect(viewportStore.getState().statusHint?.message).toBeUndefined();
  });

  it("refuses a ref the viewport did not just produce (no proof, no fallback)", async () => {
    installBody();
    const promoteSelection = vi.fn();
    // Hand-built: restored from persistence, or rewritten by a reconcile.
    const ref: EntityRef = {
      kind: "face",
      id: "body1#f:0",
      bodyId: "body1",
      topoKey: "f:0",
      anchor: { worldPoint: [1, 2, 3] },
    };
    selectionStore.getState().set([ref]);
    promotePick({ promoteSelection } as unknown as Parameters<typeof promotePick>[0], ref);
    await new Promise((r) => setTimeout(r, 0));

    expect(promoteSelection).not.toHaveBeenCalled();
    expect(viewportStore.getState().statusHint?.message).toBe(STALE_PICK_HINT);
  });
});

/*
 * VP-HARDENING WP02 (spec §6) — the user-facing half of the renderer lifecycle.
 *
 * The engine imports stores only as types, so ViewportRoot owns the message and
 * the way out of a failed renderer. Mounting the real component is the honest
 * test here: jsdom has no WebGL, so `createRenderer` genuinely rejects and the
 * component reaches the `error` state through its production path.
 */
describe("renderer lifecycle messaging and retry (spec §6)", () => {
  beforeEach(() => {
    viewportStore.getState().setStatusHint(null);
  });
  afterEach(() => {
    cleanup();
    viewportStore.getState().setStatusHint(null);
    vi.restoreAllMocks();
  });

  it("keeps a message on screen through lost → restoring → error", () => {
    applyRendererLifecycleHint("lost");
    expect(viewportStore.getState().statusHint?.message).toContain("Graphics context lost");
    expect(viewportStore.getState().statusHint?.severity).toBe("warn");

    applyRendererLifecycleHint("restoring");
    expect(viewportStore.getState().statusHint?.message).toContain("Graphics context lost");

    applyRendererLifecycleHint("error");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.message).toBe("Graphics renderer failed — retry from the viewport");
    expect(hint?.severity).toBe("error");
    expect(hint?.sticky).toBe(true);

    // …and only a healthy state clears it.
    applyRendererLifecycleHint("active");
    expect(viewportStore.getState().statusHint).toBeNull();
  });

  it("leaves a hint this module does not own alone", () => {
    viewportStore.getState().setStatusHint("Body failed to load — bad mesh", {
      severity: "error",
    });
    applyRendererLifecycleHint("active");
    expect(viewportStore.getState().statusHint?.message).toBe("Body failed to load — bad mesh");
  });

  it("a failed init shows the retry action AND the error hint", async () => {
    render(<ViewportRoot />);
    // jsdom cannot create a WebGL context, so init() rejects for real.
    await screen.findByTestId("renderer-retry");
    expect(screen.getByTestId("renderer-retry-action")).toBeEnabled();
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Graphics renderer failed — retry from the viewport",
    );
  });

  it("clicking the action calls retryRenderer on the live engine", async () => {
    const retry = vi
      .spyOn(ViewportEngine.prototype, "retryRenderer")
      .mockResolvedValue(false);
    render(<ViewportRoot />);
    const action = await screen.findByTestId("renderer-retry-action");

    fireEvent.click(action);
    expect(retry).toHaveBeenCalledTimes(1);
    // Still failed → the affordance and the message both stand.
    await screen.findByTestId("renderer-retry");
    expect(viewportStore.getState().statusHint?.message).toContain("Graphics renderer failed");
  });

  it("a successful retry clears the action and the hint", async () => {
    vi.spyOn(ViewportEngine.prototype, "retryRenderer").mockResolvedValue(true);
    render(<ViewportRoot />);
    const action = await screen.findByTestId("renderer-retry-action");

    fireEvent.click(action);
    await waitFor(() => expect(screen.queryByTestId("renderer-retry")).toBeNull());
  });
});
