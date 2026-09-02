/**
 * Metric fastener-clearance tables for the Hole tool.
 *
 * FRONTEND-ONLY BY CONTRACT. SCHEMA §7.3 is explicit that "standard-size TABLES
 * (M-series clearance, SHCS counterbores, DIN 74 countersinks) are a FRONTEND
 * concern — params always carry raw mm". Picking `M6 / normal` therefore FILLS
 * `diameter: 6.6` into the record; it never stores "M6". A document must mean the
 * same thing to a reader who has never heard of DIN 74, and a future table
 * revision must not silently re-cut everybody's existing holes.
 *
 * PROVENANCE — every number below is transcribed from a published standard, and
 * the standard is named per column:
 *
 * * `clearance` — **ISO 273** (Fasteners — Clearance holes for bolts and screws).
 *   `close` is the FINE series (H12), `normal` is the MEDIUM series (H13). These
 *   are the two a machinist actually reaches for; the COARSE series is omitted
 *   rather than offered as a third, near-identical choice.
 * * `counterbore` — **DIN 974-1 row 1** (cylindrical counterbores for cheese-head
 *   / socket-head screws), sized to clear an **ISO 4762 / DIN 912** socket-head
 *   cap screw: `diameter` clears the head Ø `dk`, `depth` sinks the head height
 *   `k` plus the standard sink allowance, so the head finishes just below flush.
 * * `countersink` — **DIN 74 form A**, the 90° recess for ISO 2009 / ISO 7046
 *   countersunk-head screws. Its `diameter` is the recess Ø at the face (`d2`).
 *   Form A pairs with the ISO 273 MEDIUM clearance hole, which is why
 *   `countersink` and `clearance.normal` agree size-for-size below.
 * * `pitchMm` / `tapDrillMm` (WP-T1) — **ISO 261 coarse-pitch series**, a
 *   DIFFERENT provenance than the rest of this row: hand-transcribed from the
 *   public numeric standard, not BOLTS data (same split
 *   `worker/src/ops/FastenerTables.cpp:9-11` and
 *   `src-tauri/crates/onecad-library/src/tables/iso4762.rs:20-24` both
 *   document — THREE copies now, cross-referenced against each other in
 *   `holeStandards.test.ts` so a transcription error in any one is caught).
 *   `tapDrillMm = nominal − pitchMm` is Option B's drilled diameter for a
 *   threaded hole (SCHEMA §7.3 `Hole.thread`): the worker cuts a plain hole at
 *   this Ø, threading is annotation only.
 *
 * A preset only ever supplies a STARTING value — every field stays editable, and
 * a hole authored from a preset is indistinguishable from one typed by hand.
 */

import type { HoleThread, HoleType } from "@/ipc/types";

/** The two ISO 273 clearance series the picker offers. */
export type HoleFit = "close" | "normal";

/** One metric size's full clearance-hole data set. All dimensions in mm. */
export interface HoleStandardSize {
  /** Thread designation, e.g. `"M6"` — a LABEL, never persisted. */
  thread: string;
  /** Nominal thread diameter, mm (the screw the hole clears). */
  nominal: number;
  /** ISO 273 clearance-hole diameter per series. */
  clearance: Record<HoleFit, number>;
  /** DIN 974-1 row 1 counterbore for an ISO 4762 socket-head cap screw. */
  counterbore: { diameter: number; depth: number };
  /** DIN 74 form A 90° countersink. */
  countersink: { diameter: number; angleDeg: number };
  /** ISO 261 coarse-pitch series (mm) — see the module doc's provenance note. */
  pitchMm: number;
  /** `nominal − pitchMm` — the drilled diameter Option B fills for a threaded hole. */
  tapDrillMm: number;
}

/**
 * M3 … M12 — the range a hand-authored part actually uses. Deliberately not
 * exhaustive: a longer list turns a one-click preset into a scrolling dialog, and
 * anything outside it is typed directly into the diameter chip.
 */
export const HOLE_STANDARDS: readonly HoleStandardSize[] = [
  {
    thread: "M3",
    nominal: 3,
    clearance: { close: 3.2, normal: 3.4 },
    counterbore: { diameter: 6.5, depth: 3.4 },
    countersink: { diameter: 6.5, angleDeg: 90 },
    pitchMm: 0.5,
    tapDrillMm: 2.5,
  },
  {
    thread: "M4",
    nominal: 4,
    clearance: { close: 4.3, normal: 4.5 },
    counterbore: { diameter: 8.0, depth: 4.6 },
    countersink: { diameter: 8.6, angleDeg: 90 },
    pitchMm: 0.7,
    tapDrillMm: 3.3,
  },
  {
    thread: "M5",
    nominal: 5,
    clearance: { close: 5.3, normal: 5.5 },
    counterbore: { diameter: 10.0, depth: 5.7 },
    countersink: { diameter: 10.4, angleDeg: 90 },
    pitchMm: 0.8,
    tapDrillMm: 4.2,
  },
  {
    thread: "M6",
    nominal: 6,
    clearance: { close: 6.4, normal: 6.6 },
    counterbore: { diameter: 11.0, depth: 6.8 },
    countersink: { diameter: 12.4, angleDeg: 90 },
    pitchMm: 1.0,
    tapDrillMm: 5.0,
  },
  {
    thread: "M8",
    nominal: 8,
    clearance: { close: 8.4, normal: 9.0 },
    counterbore: { diameter: 15.0, depth: 9.0 },
    countersink: { diameter: 16.4, angleDeg: 90 },
    pitchMm: 1.25,
    tapDrillMm: 6.75,
  },
  {
    thread: "M10",
    nominal: 10,
    clearance: { close: 10.5, normal: 11.0 },
    counterbore: { diameter: 18.0, depth: 11.0 },
    countersink: { diameter: 20.4, angleDeg: 90 },
    pitchMm: 1.5,
    tapDrillMm: 8.5,
  },
  {
    thread: "M12",
    nominal: 12,
    clearance: { close: 13.0, normal: 13.5 },
    counterbore: { diameter: 20.0, depth: 13.0 },
    countersink: { diameter: 24.4, angleDeg: 90 },
    pitchMm: 1.75,
    tapDrillMm: 10.25,
  },
] as const;

/** The table row for a thread designation, or `undefined` for an unknown size. */
export function holeStandard(thread: string): HoleStandardSize | undefined {
  return HOLE_STANDARDS.find((s) => s.thread === thread);
}

/** The raw-mm params one preset pick fills in. Only the named fields change. */
export interface HoleStandardPatch {
  diameter: number;
  cbDiameter?: number;
  cbDepth?: number;
  csDiameter?: number;
  csAngleDeg?: number;
  /** Present only for a THREADED pick (WP-T1) — absent clears any prior thread. */
  thread?: HoleThread;
}

/** `"M6x1"` — trims a whole-number pitch's trailing `.0` (`String()`'s own rule). */
function threadDesignation(thread: string, pitchMm: number): string {
  return `${thread}x${pitchMm}`;
}

/**
 * The raw-millimetre patch a `thread` + `fit` pick applies, for the CURRENT hole
 * profile. The drill diameter always comes from the ISO 273 clearance column; the
 * conditional block is filled only for the profile that owns it, so a preset can
 * never leave a stale `cb*` on a countersink (which the Rust session rejects).
 *
 * `threaded` (WP-T1, `Simple` only) fills `diameter` from the ISO 261 TAP-DRILL
 * column instead of the ISO 273 clearance column (Option B: the drilled diameter
 * IS the tap drill, threading is annotation) and emits `thread` — `detail` is
 * fixed `"cosmetic"` in T1, the only implemented level. `fit` is ignored in this
 * branch: a threaded hole has no clearance-series choice.
 *
 * Returns `undefined` for an unknown thread rather than guessing a size.
 */
export function holeStandardPatch(
  thread: string,
  fit: HoleFit,
  holeType: HoleType,
  threaded?: boolean,
): HoleStandardPatch | undefined {
  const size = holeStandard(thread);
  if (!size) return undefined;
  if (threaded) {
    if (holeType !== "simple") return undefined;
    return {
      diameter: size.tapDrillMm,
      thread: {
        standard: "ISO261",
        designation: threadDesignation(size.thread, size.pitchMm),
        majorDiameterMm: size.nominal,
        pitchMm: size.pitchMm,
        depthMm: null,
        detail: "cosmetic",
      },
    };
  }
  const diameter = size.clearance[fit];
  if (holeType === "counterbore") {
    return {
      diameter,
      cbDiameter: size.counterbore.diameter,
      cbDepth: size.counterbore.depth,
    };
  }
  if (holeType === "countersink") {
    return {
      diameter,
      csDiameter: size.countersink.diameter,
      csAngleDeg: size.countersink.angleDeg,
    };
  }
  return { diameter };
}
