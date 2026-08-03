/*
 * The fastener-clearance tables the Hole tool fills from (WP-C T3).
 *
 * These numbers ARE the contract: a wrong clearance column ships parts whose
 * screws do not fit, and nothing downstream can catch it (the wire carries raw
 * mm, so the backend sees a perfectly valid hole of the wrong size). The values
 * are therefore pinned here against their published standard, not merely
 * round-tripped.
 */
import { describe, it, expect } from "vitest";
import {
  HOLE_STANDARDS,
  holeStandard,
  holeStandardPatch,
  type HoleFit,
} from "./holeStandards";

describe("holeStandards — ISO 273 clearance columns", () => {
  /** ISO 273: fine (H12) / medium (H13) clearance holes, mm. */
  const ISO_273: Record<string, Record<HoleFit, number>> = {
    M3: { close: 3.2, normal: 3.4 },
    M4: { close: 4.3, normal: 4.5 },
    M5: { close: 5.3, normal: 5.5 },
    M6: { close: 6.4, normal: 6.6 },
    M8: { close: 8.4, normal: 9.0 },
    M10: { close: 10.5, normal: 11.0 },
    M12: { close: 13.0, normal: 13.5 },
  };

  it("covers M3 … M12 exactly once, in ascending order", () => {
    expect(HOLE_STANDARDS.map((s) => s.thread)).toEqual([
      "M3",
      "M4",
      "M5",
      "M6",
      "M8",
      "M10",
      "M12",
    ]);
    const nominals = HOLE_STANDARDS.map((s) => s.nominal);
    expect(nominals).toEqual([...nominals].sort((a, b) => a - b));
  });

  it.each(Object.entries(ISO_273))("%s matches ISO 273", (thread, want) => {
    const size = holeStandard(thread);
    expect(size, `${thread} must be in the table`).toBeDefined();
    expect(size!.clearance.close).toBe(want.close);
    expect(size!.clearance.normal).toBe(want.normal);
  });

  it("keeps every clearance ABOVE its nominal and close TIGHTER than normal", () => {
    for (const s of HOLE_STANDARDS) {
      expect(s.clearance.close, `${s.thread} close`).toBeGreaterThan(s.nominal);
      expect(s.clearance.normal, `${s.thread} normal`).toBeGreaterThan(s.clearance.close);
    }
  });
});

describe("holeStandards — recess columns", () => {
  it("keeps every counterbore/countersink WIDER than its own clearance hole", () => {
    // The backend refuses `cbDiameter <= diameter` / `csDiameter <= diameter`
    // outright, so a table row that violated this would be un-committable.
    for (const s of HOLE_STANDARDS) {
      expect(s.counterbore.diameter, `${s.thread} cb`).toBeGreaterThan(s.clearance.normal);
      expect(s.countersink.diameter, `${s.thread} cs`).toBeGreaterThan(s.clearance.normal);
      expect(s.counterbore.depth).toBeGreaterThan(0);
    }
  });

  it("uses the DIN 74 form A 90° recess throughout", () => {
    for (const s of HOLE_STANDARDS) expect(s.countersink.angleDeg).toBe(90);
  });

  /** DIN 74 form A recess diameters (d2), mm — paired with the ISO 273 medium hole. */
  it.each([
    ["M3", 6.5],
    ["M4", 8.6],
    ["M5", 10.4],
    ["M6", 12.4],
    ["M8", 16.4],
    ["M10", 20.4],
    ["M12", 24.4],
  ])("%s countersink is the DIN 74 form A recess", (thread, d2) => {
    expect(holeStandard(thread)!.countersink.diameter).toBe(d2);
  });
});

describe("holeStandardPatch", () => {
  it("fills ONLY the drill diameter for a simple hole", () => {
    expect(holeStandardPatch("M6", "normal", "simple")).toEqual({ diameter: 6.6 });
    expect(holeStandardPatch("M6", "close", "simple")).toEqual({ diameter: 6.4 });
  });

  it("fills the cb pair for a counterbore and NOTHING from the cs block", () => {
    const p = holeStandardPatch("M6", "normal", "counterbore");
    expect(p).toEqual({ diameter: 6.6, cbDiameter: 11, cbDepth: 6.8 });
    // A stale `cs*` would make the record un-committable (SCHEMA §7.3 forbids
    // carrying the other profile's block), so the patch must never name one.
    expect(p).not.toHaveProperty("csDiameter");
    expect(p).not.toHaveProperty("csAngleDeg");
  });

  it("fills the cs pair for a countersink and NOTHING from the cb block", () => {
    const p = holeStandardPatch("M6", "normal", "countersink");
    expect(p).toEqual({ diameter: 6.6, csDiameter: 12.4, csAngleDeg: 90 });
    expect(p).not.toHaveProperty("cbDiameter");
    expect(p).not.toHaveProperty("cbDepth");
  });

  it("refuses an unknown thread rather than guessing a size", () => {
    expect(holeStandardPatch("M7", "normal", "simple")).toBeUndefined();
    expect(holeStandard("M42")).toBeUndefined();
  });
});
