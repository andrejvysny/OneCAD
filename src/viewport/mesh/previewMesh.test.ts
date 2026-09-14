/*
 * VP-HARDENING WP04 — the preview lane's validate-or-skip decision.
 *
 * `buildBodyObjects` refuses an unvalidated mesh by throwing, and the two
 * preview callers (`ModelToolController.applyPreviewBodies`,
 * `library/placementController.onPreviewResult`) run inside a listener that
 * `ipc/localSolver.ts` fires from a `setTimeout`, where a throw is an unhandled
 * exception rather than a caught failure. `buildPreviewEntry` is the seam that
 * turns that into a skipped body — and, since PR-03A, the one that also prices
 * the preview and admits it against the document budget.
 *
 * The degenerate payload below is not hypothetical: `prismPreview.addWall`
 * computes its radial normal as `(u−hubU)/(Math.hypot(nu, nv) || 1)`, so a ring
 * whose points coincide with its own area centroid — coincident points, or any
 * zero-area (collinear) ring — emits a ZERO-length normal for every wall vertex.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { buildPreviewEntry, __resetPreviewMeshReportsForTests } from "./previewMesh";
import * as preview from "./previewMesh";
import * as admission from "./meshAdmission";
import { __resetRegistryForTests, disposeAll, flushDisposals, remove, swap } from "./meshRegistry";
import { makeBoxMesh, makeExtrudeBodyMesh } from "@/ipc/mockMeshes";
import type { PrismProfile } from "@/tools/preview/prismPreview";
import type { SketchPlane } from "@/ipc/types";
import { __resetLogForTests, logSnapshot } from "@/debug/log";

const XY_PLANE = {
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
} as unknown as SketchPlane;

function prism(ring: [number, number][]): PrismProfile {
  return { ring, holes: [], cap: { positions: ring.flat(), indices: [0, 1, 2] } };
}

/** The exact mock preview payload for an extrude of `ring`, MESH1 bytes and all. */
function previewBlob(ring: [number, number][]): ArrayBuffer {
  return makeExtrudeBodyMesh(prism(ring), XY_PLANE, 10);
}

const HEALTHY: [number, number][] = [[0, 0], [10, 0], [0, 10]];
const COLLINEAR: [number, number][] = [[0, 0], [1, 0], [2, 0]];
const COINCIDENT: [number, number][] = [[5, 5], [5, 5], [5, 5]];

beforeEach(() => {
  disposeAll();
  __resetRegistryForTests();
  __resetPreviewMeshReportsForTests();
  __resetLogForTests();
});
afterEach(() => {
  __resetLogForTests({ enabled: false });
  disposeAll();
  __resetRegistryForTests();
});

const warnings = () => logSnapshot().filter((e) => e.level === "warn" && e.tag === "preview");

describe("buildPreviewEntry validate-or-skip", () => {
  it("builds a healthy preview", () => {
    const entry = buildPreviewEntry(previewBlob(HEALTHY), "preview:0", 1)!;
    expect(entry).not.toBeNull();
    expect(entry.displayState).toBe("current");
    expect(entry.accounting.triangleCount).toBeGreaterThan(0);
    expect(warnings()).toHaveLength(0);
  });

  it("skips a zero-area preview ring, whose wall normals are zero-length", () => {
    for (const ring of [COLLINEAR, COINCIDENT]) {
      __resetPreviewMeshReportsForTests();
      __resetLogForTests();
      expect(buildPreviewEntry(previewBlob(ring), "preview:0", 1)).toBeNull();
      const logged = warnings();
      expect(logged).toHaveLength(1);
      expect(logged[0].ctx).toMatchObject({ bodyId: "preview:0", code: "normal-length" });
    }
  });

  it("skips a structurally malformed preview payload with the parser's kind", () => {
    const torn = makeBoxMesh();
    new DataView(torn).setUint32(0x00, 0x4d455349, true); // bad magic
    expect(buildPreviewEntry(torn, "preview:0", 1)).toBeNull();
    expect(warnings()[0].ctx).toMatchObject({ bodyId: "preview:0", code: "bad-magic" });
  });

  it("logs one line per (body, code) so a drag cannot flood the log", () => {
    const blob = previewBlob(COLLINEAR);
    for (let frame = 0; frame < 50; frame++) {
      expect(buildPreviewEntry(blob, "preview:0", frame)).toBeNull();
    }
    expect(warnings()).toHaveLength(1);

    // A different body, or a different defect, is still worth saying once.
    expect(buildPreviewEntry(blob, "preview:1", 1)).toBeNull();
    expect(warnings()).toHaveLength(2);
    const torn = makeBoxMesh();
    new DataView(torn).setUint16(0x04, 9, true); // unsupported version
    expect(buildPreviewEntry(torn, "preview:0", 1)).toBeNull();
    expect(warnings()).toHaveLength(3);
  });

  it("never throws, and never publishes a status hint — a preview is transient", () => {
    const torn = new ArrayBuffer(8);
    expect(() => buildPreviewEntry(torn, "preview:0", 1)).not.toThrow();
    expect(buildPreviewEntry(torn, "preview:0", 1)).toBeNull();
    expect(logSnapshot().some((e) => e.level === "error")).toBe(false);
  });
});

/*
 * VP-HARDENING PR-03B — the preview lanes spend the SAME budget as the document.
 *
 * An exact preview or a placement ghost allocates exactly the kind of resource
 * §9 prices, so it must be admitted against the same authority a committed body
 * is — otherwise a drag can put arbitrarily many unpriced meshes on the GPU
 * while admission reports the document's bodies only.
 */
describe("buildPreviewEntry", () => {
  afterEach(() => {
    admission.__resetDocumentAdmissionForTests();
  });

  it("admits a healthy preview, charges it, and releases on the retirement flush", () => {
    admission.__resetDocumentAdmissionForTests();
    const budget = admission.getDocumentAdmission();
    const entry = preview.buildPreviewEntry(previewBlob(HEALTHY), "preview:0", 1);
    expect(entry).not.toBeNull();
    // Priced as the PLAN, and the plan of a preview body — no document colour,
    // no authored overrides — is indexed unless its own payload says otherwise.
    expect(entry!.plan.layout).toBe("indexed");
    expect(entry!.geometry.getIndex()).not.toBeNull();
    expect(budget.snapshot()).toEqual({
      preparedCpuBytes: entry!.plan.cpuBytes,
      estimatedGpuBytes: entry!.plan.gpuBytes,
      holdings: 1,
    });
    expect(warnings()).toHaveLength(0);

    // The preview lane publishes and drops through the registry exactly as a
    // document body does, so the same ordered retirement frees it.
    swap("preview:0", entry!);
    remove("preview:0");
    expect(budget.snapshot().holdings).toBe(1); // still allocated this frame
    flushDisposals();
    expect(entry!.resourceState).toBe("disposed");
    expect(budget.snapshot().holdings).toBe(0);
  });

  it("refuses a preview that does not fit the remaining budget, and builds nothing", () => {
    admission.__resetDocumentAdmissionForTests({ estimatedGpuBytes: 1 });
    const budget = admission.getDocumentAdmission();

    expect(preview.buildPreviewEntry(previewBlob(HEALTHY), "preview:0", 1)).toBeNull();

    expect(budget.snapshot().holdings).toBe(0);
    const logged = warnings();
    expect(logged).toHaveLength(1);
    expect(logged[0].ctx).toMatchObject({ bodyId: "preview:0", code: "gpu-budget" });
  });

  it("skips an invalid preview before it can reserve anything", () => {
    admission.__resetDocumentAdmissionForTests();
    const budget = admission.getDocumentAdmission();
    expect(preview.buildPreviewEntry(previewBlob(COLLINEAR), "preview:0", 1)).toBeNull();
    expect(budget.snapshot().holdings).toBe(0);
    expect(warnings()[0].ctx).toMatchObject({ code: "normal-length" });
  });
});
