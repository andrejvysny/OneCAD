/*
 * placementController — auto-size on a hole rim (spec §5.3's hole row, §5.4
 * step 3; WP-A3), driven through the real gesture rather than by calling the
 * solver directly.
 *
 * WHY THIS TEST EXISTS AT ALL: the one thing auto-size must never do is choose
 * a size for the GHOST that the COMMIT does not carry. That agreement lives in
 * two different call paths (`updatePreview`'s `source.params` and
 * `placeComponent`'s `params`), so it cannot be proven by a unit test of the
 * picker — only by watching both calls from one gesture. This exact class of
 * bug has already shipped twice in this module (a dropped `rotate`, then a
 * hardcoded `source`), which is why it is pinned here.
 *
 * The Playwright lane now reaches this too (WP-F3 gave `mockClient.
 * classifyElement` a measured cylinder answer and `?vpdemo=cyl` a bore to
 * hover) — but it can only watch the OUTCOME, a committed record carrying the
 * auto-sized thread. Watching both CALLS of one gesture, and proving the ghost
 * and the commit agree even when nothing fits, still only happens here.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import type { ClassifyResult, LibraryComponent, PlaceComponentSource, PreviewSession } from "@/ipc/types";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { setViewportEngine } from "@/viewport/engineBridge";
import {
  armPlacement,
  cancelPlacement,
  configurePlacementController,
} from "./placementController";

/** The default resolution; a `mock`-prefixed name is what vitest hoists `vi.mock` against. */
let mockResolvedSource: PlaceComponentSource = {
  kind: "generator",
  generatorId: "iso4762",
  generatorVersion: 1,
};

// Hoisted by vitest regardless of placement; kept at top level so the file
// reads in the order it actually executes. The controller resolves a
// component's real geometry source at arm time (WP-3.2) — mutable per-test via
// `mockResolvedSource` so the WP-C `profile`-kind test below can arm against a
// different resolved source without a second mock module.
vi.mock("@/ipc/client", () => ({
  createClient: () => ({
    resolveComponentSource: async () => mockResolvedSource,
  }),
}));

const CLEARANCE_HOLE_M6_RADIUS = 3.3; // Ø6.6 — the standard M6 clearance hole.

function componentFixture(): LibraryComponent {
  return {
    id: "onecad.std.iso4762",
    version: "1.0.0",
    name: "Socket Head Cap Screw",
    category: ["fasteners"],
    tags: ["metric"],
    sourceKind: "generator",
    revision: `sha256:${"0".repeat(64)}`,
    generatorId: "iso4762",
    generatorVersion: 1,
    attachments: {
      shank_axis: { on: "cylinder:shank", accepts: ["cylinder", "hole", "circularEdge"] },
    },
    parameters: {
      thread: { role: "free", key: "M6", domain: ["M3", "M4", "M5", "M6", "M8", "M10"] },
      length: { role: "free", value: 20 },
    },
    designation: "ISO 4762 {thread}x{length}",
  } as LibraryComponent;
}

/** A cylindrical face whose radius names a hole of `2 * radius` mm. */
function cylinderClassify(radius: number): ClassifyResult {
  return {
    kind: "face",
    surfaceType: "cylinder",
    curveType: "",
    frame: { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius },
  };
}

interface Harness {
  updatePreview: ReturnType<typeof vi.fn>;
  placeComponent: ReturnType<typeof vi.fn>;
  beginPreview: ReturnType<typeof vi.fn>;
}

/**
 * Installs a stub engine + services whose hover always classifies a hole of
 * `2 * radius`. `hit: false` makes every probe MISS instead — the free-space
 * lane — with a camera ray straight down from 100 mm above (0,0), so the
 * ground-plane intersection is the origin and any drift is visible.
 */
function install(radius: number, opts?: { hit?: boolean }): Harness {
  const engine = {
    setOrbitSuppressed: vi.fn(),
    probePick: vi.fn(() =>
      opts?.hit === false
        ? null
        : {
            bodyId: "body_1",
            kind: "face",
            elementId: "el_1",
            topoKey: "f:1",
            worldPos: { x: 0, y: 0, z: 0 },
          },
    ),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100], dir: [0, 0, -1] })),
    setPreviewBody: vi.fn(),
    clearPreviewBody: vi.fn(),
  } as unknown as ViewportEngine;
  setViewportEngine(engine);

  const updatePreview = vi.fn();
  const placeComponent = vi.fn(async () => undefined);
  const session: PreviewSession = {
    sessionId: "s1",
    previewBodyId: "preview_1",
  } as PreviewSession;

  const beginPreview = vi.fn(async () => session);

  configurePlacementController({
    geometryQuery: {
      classifyElement: vi.fn(async () => cylinderClassify(radius)),
    },
    commandApi: {
      placeComponent,
      detachComponent: vi.fn(),
      beginPreview,
      updatePreview,
      endPreview: vi.fn(async () => null),
      onPreviewResult: vi.fn(() => () => undefined),
    },
  } as unknown as Parameters<typeof configurePlacementController>[0]);

  return { updatePreview, placeComponent, beginPreview };
}

/** Lets every awaited microtask in the hover/arm chains settle. */
async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

describe("placementController auto-size", () => {
  afterEach(() => {
    cancelPlacement();
    configurePlacementController(null);
    setViewportEngine(null);
  });

  it("sizes the GHOST and the COMMIT to the same thread", async () => {
    const harness = install(CLEARANCE_HOLE_M6_RADIUS);
    armPlacement(componentFixture());
    await settle();

    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 10, clientY: 10 }));
    await settle();

    expect(harness.updatePreview).toHaveBeenCalled();
    const calls = harness.updatePreview.mock.calls;
    const previewParams = calls[calls.length - 1][1] as {
      source: { params?: Record<string, unknown> };
    };
    expect(previewParams.source.params).toMatchObject({ thread: "M6" });

    window.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, clientY: 10 }));
    await settle();

    expect(harness.placeComponent).toHaveBeenCalledTimes(1);
    // Argument 5 is the free-param map; the ghost above chose the same one.
    expect(harness.placeComponent.mock.calls[0][4]).toMatchObject({ thread: "M6" });
  });

  it("records the snapped mate on commit (WP-H2, spec §5.4 step 5)", async () => {
    const harness = install(CLEARANCE_HOLE_M6_RADIUS);
    armPlacement(componentFixture());
    await settle();

    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 10, clientY: 10 }));
    await settle();
    // Flip once so the recorded mate carries the gesture's real flip state.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
    window.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, clientY: 10 }));
    await settle();

    expect(harness.placeComponent).toHaveBeenCalledTimes(1);
    // Argument 6 is the recorded snap — identity evidence, kind, and flip.
    expect(harness.placeComponent.mock.calls[0][5]).toEqual({
      selfAttachment: "shank_axis",
      targetBodyId: "body_1",
      targetTopoKey: "f:1",
      targetElementId: "el_1",
      targetKind: "face",
      kind: "concentric",
      flipped: true,
      anchorWorldPoint: [0, 0, 0],
    });
  });

  it("leaves the size alone when no declared size fits the hole", async () => {
    // A 1 mm hole takes no fastener in this domain. Substituting the smallest
    // would be the Toolbox failure mode in miniature, so the armed default
    // stands and the ghost carries no override at all.
    const harness = install(0.5);
    armPlacement(componentFixture());
    await settle();
    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 10, clientY: 10 }));
    await settle();

    const calls = harness.updatePreview.mock.calls;
    const previewParams = calls[calls.length - 1][1] as {
      source: { params?: Record<string, unknown> };
    };
    expect(previewParams.source.params).toBeUndefined();
  });
});

/*
 * PROFILE SOURCE (Component Library WP-C). `withGestureParams` used to return
 * a NON-generator source unchanged, so a profile component's gesture override
 * never reached the ghost at all. Broadening it to `profile` had to MERGE
 * rather than replace: `source.params.length` is the resolved source's own
 * regen input (not an override), and a naive copy of the generator branch's
 * replace-with-gestureParams behaviour would have silently dropped it the
 * moment an unrelated gesture key (auto-size's `thread`) got folded in.
 */
describe("placementController profile source (WP-C)", () => {
  afterEach(() => {
    cancelPlacement();
    configurePlacementController(null);
    setViewportEngine(null);
    mockResolvedSource = { kind: "generator", generatorId: "iso4762", generatorVersion: 1 };
  });

  function profileComponentFixture(): LibraryComponent {
    return {
      id: "vendor.extrusion-2020",
      version: "1.0.0",
      name: "2020 Extrusion",
      category: ["structural"],
      tags: [],
      sourceKind: "profile",
      revision: `sha256:${"0".repeat(64)}`,
      attachments: {
        shank_axis: { on: "cylinder:shank", accepts: ["cylinder", "hole", "circularEdge"] },
      },
      parameters: {
        thread: { role: "free", key: "M6", domain: ["M3", "M4", "M5", "M6", "M8", "M10"] },
      },
    } as LibraryComponent;
  }

  it("carries a gesture override into the ghost WITHOUT dropping the resolved length", async () => {
    mockResolvedSource = {
      kind: "profile",
      sha256: "a".repeat(64),
      codec: "brep",
      brepFormat: 4,
      params: { length: 120 },
    };
    const harness = install(CLEARANCE_HOLE_M6_RADIUS);
    armPlacement(profileComponentFixture());
    await settle();

    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 10, clientY: 10 }));
    await settle();

    const calls = harness.updatePreview.mock.calls;
    const previewParams = calls[calls.length - 1][1] as {
      source: { params?: Record<string, unknown> };
    };
    // Both survive: the gesture's own `thread` override rides ALONGSIDE the
    // resolved source's `length` — a replace-based merge would have dropped it.
    expect(previewParams.source.params).toMatchObject({ length: 120, thread: "M6" });
  });

  it("previews the resolved length verbatim when the gesture chose no override", async () => {
    mockResolvedSource = {
      kind: "profile",
      sha256: "a".repeat(64),
      codec: "brep",
      brepFormat: 4,
      params: { length: 250 },
    };
    // No hover at all — this is the FIRST preview `armPlacement` opens
    // (`beginPreview`, not `updatePreview`), with gestureParams still empty.
    const harness = install(0.5);
    armPlacement(profileComponentFixture());
    await settle();

    expect(harness.beginPreview).toHaveBeenCalledTimes(1);
    const draft = harness.beginPreview.mock.calls[0][0] as {
      params: { source: { params?: Record<string, unknown> } };
    };
    expect(draft.params.source.params).toEqual({ length: 250 });
  });
});

/*
 * FREE SPACE (spec §5.4 steps 1/6; WP-F3). Two things have to hold together and
 * neither is provable from the solver alone: the ghost must FOLLOW with nothing
 * under the cursor (the WP-1.5 scope cut hid it instead), and the commit must
 * carry NO mate — a free-space drop has no target, and recording one anyway is
 * how a placement would come back `NeedsRepair` against something it never
 * touched.
 */
describe("placementController free space", () => {
  afterEach(() => {
    cancelPlacement();
    configurePlacementController(null);
    setViewportEngine(null);
  });

  it("follows the cursor on the ground plane when nothing is hovered", async () => {
    const harness = install(CLEARANCE_HOLE_M6_RADIUS, { hit: false });
    armPlacement(componentFixture());
    await settle();

    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 10, clientY: 10 }));
    await settle();

    expect(harness.updatePreview).toHaveBeenCalled();
    const calls = harness.updatePreview.mock.calls;
    const params = calls[calls.length - 1][1] as {
      translate: [number, number, number];
      rotate: { axis: [number, number, number]; angleDeg: number };
      source: { params?: Record<string, unknown> };
    };
    // The stub ray drops from (0,0,100) straight down, so z = 0 is the origin.
    expect(params.translate).toEqual([0, 0, 0]);
    // Identity rotation: there is no target frame to orient against.
    expect(params.rotate.angleDeg).toBe(0);
    // …and no auto-size either — the input for it is a hole's radius.
    expect(params.source.params).toBeUndefined();
  });

  it("commits at the ghost's transform with NO mate", async () => {
    const harness = install(CLEARANCE_HOLE_M6_RADIUS, { hit: false });
    armPlacement(componentFixture());
    await settle();

    window.dispatchEvent(new PointerEvent("pointermove", { clientX: 10, clientY: 10 }));
    await settle();
    window.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, clientY: 10 }));
    await settle();

    expect(harness.placeComponent).toHaveBeenCalledTimes(1);
    const call = harness.placeComponent.mock.calls[0];
    expect(call[2]).toEqual([0, 0, 0]); // translate — what the ghost showed
    expect(call[3]).toMatchObject({ angleDeg: 0 });
    expect(call[5]).toBeUndefined(); // the mate argument
  });

  it("does not commit before the first move (no ghost, nothing agreed to)", async () => {
    const harness = install(CLEARANCE_HOLE_M6_RADIUS, { hit: false });
    armPlacement(componentFixture());
    await settle();

    window.dispatchEvent(new PointerEvent("pointerdown", { clientX: 10, clientY: 10 }));
    await settle();

    expect(harness.placeComponent).not.toHaveBeenCalled();
  });
});
