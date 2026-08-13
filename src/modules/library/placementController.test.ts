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
 * The Playwright mock lane cannot cover it: `mockClient.classifyElement`
 * derives its answer from mesh metrics and only ever reports `plane` or
 * `other` — it has no cylinder/circle case and therefore no radius, which is
 * the input auto-size runs on.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import type { ClassifyResult, LibraryComponent, PreviewSession } from "@/ipc/types";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { setViewportEngine } from "@/viewport/engineBridge";
import {
  armPlacement,
  cancelPlacement,
  configurePlacementController,
} from "./placementController";

// Hoisted by vitest regardless of placement; kept at top level so the file
// reads in the order it actually executes. The controller resolves a
// component's real geometry source at arm time (WP-3.2) — here it is a plain
// generator, which is what auto-size applies to.
vi.mock("@/ipc/client", () => ({
  createClient: () => ({
    resolveComponentSource: async () => ({
      kind: "generator",
      generatorId: "iso4762",
      generatorVersion: 1,
    }),
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
}

/** Installs a stub engine + services whose hover always classifies a hole of `2 * radius`. */
function install(radius: number): Harness {
  const engine = {
    setOrbitSuppressed: vi.fn(),
    probePick: vi.fn(() => ({
      bodyId: "body_1",
      kind: "face",
      elementId: "el_1",
      topoKey: "f:1",
      worldPos: { x: 0, y: 0, z: 0 },
    })),
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

  configurePlacementController({
    geometryQuery: {
      classifyElement: vi.fn(async () => cylinderClassify(radius)),
    },
    commandApi: {
      placeComponent,
      detachComponent: vi.fn(),
      beginPreview: vi.fn(async () => session),
      updatePreview,
      endPreview: vi.fn(async () => null),
      onPreviewResult: vi.fn(() => () => undefined),
    },
  } as unknown as Parameters<typeof configurePlacementController>[0]);

  return { updatePreview, placeComponent };
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
