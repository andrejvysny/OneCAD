/*
 * The mock lane's COMPONENT-LIBRARY surface (WP-F3): the classify answer the
 * placement gesture runs on, the opt-in seed catalog, and the built-in starter
 * templates.
 *
 * WHY THESE THREE TOGETHER: they are the mock lane's parity with what the tauri
 * lane actually serves. A mock that classifies no cylinder makes the concentric
 * snap and auto-size unreachable in Playwright; a mock catalog holding one
 * screw makes the bearing/motor packages the app ships untested outside Rust;
 * a template list that starts empty makes the start screen's starter grid a
 * frontend feature nothing exercises. Each gap hid a whole UI path behind "the
 * mock cannot do that".
 *
 * The parity that is NOT claimed: geometry. The mock has no kernel, so every
 * catalog entry places as the same synthetic solid — identity, parameters and
 * attachments are mirrored, shapes are not.
 */
import { describe, it, expect, afterEach, beforeEach } from "vitest";
import {
  MOCK_DEMO_BORE_BODY_ID,
  mockClient,
  resetMockDocument,
  seedMockDemoCylinder,
} from "./mockClient";

/** Point the mock's URL-flag reader at `?mocklibrary=1` (or clear it). */
function setLibraryFlag(on: boolean): void {
  window.history.replaceState({}, "", on ? "/?mocklibrary=1" : "/");
}

afterEach(() => {
  setLibraryFlag(false);
  resetMockDocument();
});

describe("mockClient.classifyElement — measured cylinder answers", () => {
  beforeEach(() => {
    seedMockDemoCylinder();
  });

  it("reports the demo bore as a cylinder with ITS radius and axis", async () => {
    const bore = await mockClient.classifyElement(MOCK_DEMO_BORE_BODY_ID, "", "f:1");
    expect(bore).not.toBeNull();
    expect(bore!.kind).toBe("face");
    expect(bore!.surfaceType).toBe("cylinder");
    // Ø8.5 — the M8 clearance hole the demo body is drilled to.
    expect(bore!.frame!.radius).toBeCloseTo(4.25, 4);
    expect(bore!.frame!.axis![2]).toBeCloseTo(1, 6);
    // A cylinder frame carries an axis and NO normal (SCHEMA's ClassifyFrame).
    expect(bore!.frame!.normal).toBeNull();
    // The bushing sits 80 mm along +X, and the frame says so.
    expect(bore!.frame!.origin[0]).toBeCloseTo(80, 4);
  });

  it("reports the outer wall separately, at the outer radius", async () => {
    const wall = await mockClient.classifyElement(MOCK_DEMO_BORE_BODY_ID, "", "f:0");
    expect(wall!.surfaceType).toBe("cylinder");
    expect(wall!.frame!.radius).toBeCloseTo(20, 4);
  });

  it("still answers plane for a flat face — the existing lane is untouched", async () => {
    // The bushing's own top annulus is planar, and so is every seed-box face.
    const cap = await mockClient.classifyElement(MOCK_DEMO_BORE_BODY_ID, "", "f:2");
    expect(cap!.surfaceType).toBe("plane");
    expect(cap!.frame!.axis).toBeNull();
    expect(cap!.frame!.normal![2]).toBeCloseTo(1, 6);

    const boxFace = await mockClient.classifyElement("body1", "", "f:4");
    expect(boxFace!.surfaceType).toBe("plane");
    expect(boxFace!.frame!.radius).toBeNull();
  });

  it("is null for a key the body does not carry", async () => {
    expect(await mockClient.classifyElement(MOCK_DEMO_BORE_BODY_ID, "", "f:999")).toBeNull();
  });
});

describe("mockClient library catalog — mirrors the shipped seed packages", () => {
  it("is empty without the flag (a dead catalog teaches a UI bug to pass)", async () => {
    setLibraryFlag(false);
    expect(await mockClient.listLibraryComponents()).toEqual([]);
    expect(await mockClient.reindexLibrary()).toEqual({ total: 0, indexed: 0, skipped: [] });
  });

  it("lists the one seeded package with the flag on", async () => {
    setLibraryFlag(true);
    const list = await mockClient.listLibraryComponents();
    expect(list.map((c) => c.id)).toEqual(["onecad.std.iso4762"]);
    expect(await mockClient.reindexLibrary()).toEqual({ total: 1, indexed: 1, skipped: [] });
  });

  it("the screw is keyed by its THREAD domain, and seats on the head or the shank", async () => {
    setLibraryFlag(true);
    const list = await mockClient.listLibraryComponents();
    const screw = list.find((c) => c.id === "onecad.std.iso4762")!;
    expect(screw.parameters.thread).toMatchObject({ role: "free", key: "M6" });
    expect(screw.parameters.thread.domain).toContain("M8");
    // dk is read off the table, never chosen per instance.
    expect(screw.parameters.head_d.role).toBe("table");
    expect(screw.designation).toBe("ISO 4762 {thread}x{length}");
    expect(screw.attachments.headSeat.accepts).toEqual(["plane"]);
    expect(screw.attachments.shankAxis.accepts).toContain("hole");
  });

  it("resolves a seeded component's source for placement", async () => {
    setLibraryFlag(true);
    const source = await mockClient.resolveComponentSource("onecad.std.iso4762", "1.0.0");
    expect(source).toMatchObject({ kind: "generator", generatorId: "iso4762" });
  });
});

describe("mockClient.ingestComponents — MOCK LIMIT refusal (WP-C2)", () => {
  it("refuses every requested path by name, never fabricating a catalog entry", async () => {
    const report = await mockClient.ingestComponents({
      paths: ["/vendor/a.step", "/vendor/b.step"],
      defaults: { vendor: "vendor", category: ["imported"] },
    });
    expect(report.libraryRoot).toBe("<mock>");
    expect(report.parts).toEqual([
      {
        path: "/vendor/a.step",
        status: "refused",
        message: "MOCK LIMIT: component ingestion needs the OCCT worker",
      },
      {
        path: "/vendor/b.step",
        status: "refused",
        message: "MOCK LIMIT: component ingestion needs the OCCT worker",
      },
    ]);
  });

  it("an empty batch reports an empty batch, not an error", async () => {
    const report = await mockClient.ingestComponents({
      paths: [],
      defaults: { vendor: "vendor", category: ["imported"] },
    });
    expect(report.parts).toEqual([]);
  });
});

describe("mockClient.pickComponentFiles — no native dialog on this lane (WP-C2)", () => {
  it("always resolves [] — the mock cannot open a native picker", async () => {
    expect(await mockClient.pickComponentFiles()).toEqual([]);
  });
});

describe("mockClient.repairMateAxis — the SCHEMA §9 mateAxisReversed repair (WP-I)", () => {
  /** Places an M6 SHCS with a mate of `kind`, returning its record id. */
  async function placeMated(
    kind: "concentric" | "coincident",
  ): Promise<string> {
    setLibraryFlag(true);
    const res = await mockClient.placeComponent(
      "onecad.std.iso4762",
      "1.0.0",
      [0, 0, 0],
      undefined,
      undefined,
      {
        selfAttachment: "shankAxis",
        targetBodyId: MOCK_DEMO_BORE_BODY_ID,
        targetTopoKey: "f:1",
        targetKind: "face",
        kind,
        flipped: false,
        anchorWorldPoint: [80, 0, 0],
      },
    );
    // The NEWEST PlaceComponent row of the commit's own feature list — never a
    // diff against `getProjection()`, whose zustand store is not reset between
    // tests and so still carries the previous test's (identically numbered) row.
    const rows = (res.features ?? []).filter((f) => f.opType === "PlaceComponent");
    const placed = rows[rows.length - 1];
    expect(placed).toBeTruthy();
    return placed.id;
  }

  it("toggles `flipped` on keep-direction and re-freezes the axis and sidedness", async () => {
    const record = await placeMated("concentric");
    await mockClient.repairMateAxis(record, true, [0, 0, -1], "pin");
    const mate = (await mockClient.getOperationParams(record)).mate as Record<string, unknown>;
    expect(mate.flipped).toBe(true);
    expect(mate.targetAxis).toEqual([0, 0, -1]);
    expect(mate.targetSidedness).toBe("pin");
  });

  it("leaves `flipped` alone when following the axis, and keeps an unmeasured sidedness", async () => {
    const record = await placeMated("concentric");
    await mockClient.repairMateAxis(record, false, [0, 0, -1], "hole");
    await mockClient.repairMateAxis(record, false, [1, 0, 0]);
    const mate = (await mockClient.getOperationParams(record)).mate as Record<string, unknown>;
    expect(mate.flipped).toBe(false);
    expect(mate.targetAxis).toEqual([1, 0, 0]);
    // Absent evidence NEVER clears the stored sidedness — the pin/hole check
    // must not be silently disarmed by a repair that could not measure it.
    expect(mate.targetSidedness).toBe("hole");
  });

  it("mirrors the backend's refusals rather than staying green on a rejected call", async () => {
    const coincident = await placeMated("coincident");
    await expect(mockClient.repairMateAxis(coincident, true, [0, 0, -1])).rejects.toThrow(
      /concentric/,
    );
    await expect(mockClient.repairMateAxis("op_nope", true, [0, 0, -1])).rejects.toThrow(
      /no params/,
    );
    const concentric = await placeMated("concentric");
    await expect(
      mockClient.repairMateAxis(concentric, true, [0, Number.NaN, 0]),
    ).rejects.toThrow(/finite/);
  });
});

describe("mockClient project templates — the built-in starters", () => {
  it("offers nothing without the flag (there is no seeded root on this lane)", async () => {
    setLibraryFlag(false);
    expect(await mockClient.listTemplates()).toEqual([]);
  });

  it("lists the two starters with the flag on", async () => {
    setLibraryFlag(true);
    const templates = await mockClient.listTemplates();
    expect(templates.map((t) => t.id)).toEqual([
      "onecad.std.template.blank",
      "onecad.std.template.printed-part",
    ]);
    expect(templates.map((t) => t.name)).toEqual(["Blank", "3D-Printed Part"]);
    // Every starter says what it actually contains — the card renders it.
    expect(templates.every((t) => (t.description ?? "").length > 0)).toBe(true);
  });

  it("starts a new document from a starter, and refuses an id nothing carries", async () => {
    setLibraryFlag(true);
    const doc = await mockClient.newFromTemplate("onecad.std.template.printed-part");
    expect(doc.documentId).toBeTruthy();
    await expect(mockClient.newFromTemplate("onecad.std.template.nope")).rejects.toThrow(
      /unknown template/,
    );
  });

  it("will not let a saved template shadow a starter's id", async () => {
    setLibraryFlag(true);
    await expect(
      mockClient.saveAsTemplate("onecad.std.template.blank", "Mine"),
    ).rejects.toThrow(/already exists/);
    // A fresh id still saves, and lands AFTER the starters.
    await mockClient.saveAsTemplate("me.mine", "Mine");
    expect((await mockClient.listTemplates()).map((t) => t.id)).toEqual([
      "onecad.std.template.blank",
      "onecad.std.template.printed-part",
      "me.mine",
    ]);
  });
});
