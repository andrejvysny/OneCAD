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

  it("lists the four seeded packages with the flag on, SHCS first", async () => {
    setLibraryFlag(true);
    const list = await mockClient.listLibraryComponents();
    expect(list.map((c) => c.id)).toEqual([
      "onecad.std.iso4762",
      "onecad.std.iso15",
      "onecad.std.nema17",
      "onecad.std.nema23",
    ]);
    expect(await mockClient.reindexLibrary()).toEqual({ total: 4, indexed: 4, skipped: [] });
  });

  it("the bearing is keyed by its CODE domain, with locked boundary dimensions", async () => {
    setLibraryFlag(true);
    const list = await mockClient.listLibraryComponents();
    const bearing = list.find((c) => c.id === "onecad.std.iso15")!;
    expect(bearing.parameters.code).toMatchObject({ role: "free", key: "608" });
    expect(bearing.parameters.code.domain).toContain("6202");
    // d/D/B are read off the table, never chosen per instance.
    for (const key of ["bore", "od", "width"]) {
      expect(bearing.parameters[key].role).toBe("table");
    }
    expect(bearing.designation).toBe("Bearing {code}");
    expect(bearing.attachments.bore_axis.accepts).toContain("hole");
  });

  it("the steppers offer body length only, and seat on the faceplate or the pilot", async () => {
    setLibraryFlag(true);
    const list = await mockClient.listLibraryComponents();
    for (const id of ["onecad.std.nema17", "onecad.std.nema23"]) {
      const motor = list.find((c) => c.id === id)!;
      const free = Object.entries(motor.parameters).filter(([, s]) => s.role === "free");
      expect(free.map(([k]) => k)).toEqual(["length"]);
      expect(motor.parameters.length).toMatchObject({ value: 40, min: 20 });
      expect(motor.attachments.faceplate.accepts).toEqual(["plane"]);
      expect(motor.attachments.pilot_axis.accepts).toContain("cylinder");
      expect(motor.generatorId).toBe(id.split(".").pop());
    }
  });

  it("resolves a seeded component's source for placement", async () => {
    setLibraryFlag(true);
    const source = await mockClient.resolveComponentSource("onecad.std.nema17", "1.0.0");
    expect(source).toMatchObject({ kind: "generator", generatorId: "nema17" });
  });
});

describe("mockClient project templates — the built-in starters", () => {
  it("offers nothing without the flag (there is no seeded root on this lane)", async () => {
    setLibraryFlag(false);
    expect(await mockClient.listTemplates()).toEqual([]);
  });

  it("lists the three starters with the flag on", async () => {
    setLibraryFlag(true);
    const templates = await mockClient.listTemplates();
    expect(templates.map((t) => t.id)).toEqual([
      "onecad.std.template.blank",
      "onecad.std.template.printed-part",
      "onecad.std.template.nema17-mount",
    ]);
    expect(templates.map((t) => t.name)).toEqual(["Blank", "3D-Printed Part", "NEMA 17 Motor Mount"]);
    // Every starter says what it actually contains — the card renders it.
    expect(templates.every((t) => (t.description ?? "").length > 0)).toBe(true);
  });

  it("starts a new document from a starter, and refuses an id nothing carries", async () => {
    setLibraryFlag(true);
    const doc = await mockClient.newFromTemplate("onecad.std.template.nema17-mount");
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
      "onecad.std.template.nema17-mount",
      "me.mine",
    ]);
  });
});
