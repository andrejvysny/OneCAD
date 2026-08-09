/*
 * The direction rule, as a table.
 *
 * Every row is a physical situation the user can put the tool in, so the names
 * describe geometry rather than branch numbers. The two that matter most are the
 * COPLANAR cases: a sketch lying on a body's face reads gap ≈ 0 on both sides,
 * and only `inside` can tell "grows into the solid" from "grows away from it".
 */
import { describe, it, expect } from "vitest";
import { autoModeFor, hasMaterial, CONTACT_FLIP_HOLD, NO_MATERIAL } from "./materialProbe";
import type { AutoMode, MaterialSides, SideProbe } from "./materialProbe";

const NEW: AutoMode = { mode: "NewBody", targetBodyId: null };
const probe = (bodyId: string, gap: number, inside = false): SideProbe => ({ bodyId, gap, inside });

/** A sketch ON a body's top face: the body is flush behind it. */
const ON_FACE: MaterialSides = { pos: null, neg: probe("b1", 0, true) };
/** A datum plane INSIDE a block: material both ways. */
const EMBEDDED: MaterialSides = { pos: probe("b1", 0, true), neg: probe("b1", 0, true) };
/** A datum plane 10 above a block. */
const ABOVE: MaterialSides = { pos: null, neg: probe("b1", 10) };

describe("autoModeFor", () => {
  it("holds the current mode at depth 0 — a crossing must not blink a third state", () => {
    const prev: AutoMode = { mode: "Cut", targetBodyId: "b1" };
    expect(autoModeFor(EMBEDDED, 0, prev)).toEqual(prev);
    expect(autoModeFor(EMBEDDED, Number.NaN, prev)).toEqual(prev);
  });

  it("no material either way is a NewBody, whatever the direction", () => {
    expect(autoModeFor(NO_MATERIAL, 20, NEW)).toEqual(NEW);
    expect(autoModeFor(NO_MATERIAL, -20, NEW)).toEqual(NEW);
  });

  it("sketch ON a face: pushing IN cuts, pulling OUT joins", () => {
    // Into the body it sits on.
    expect(autoModeFor(ON_FACE, -5, NEW)).toEqual({ mode: "Cut", targetBodyId: "b1" });
    // Away from it — the prism lands on that face, so it fuses.
    expect(autoModeFor(ON_FACE, 5, NEW)).toEqual({ mode: "Add", targetBodyId: "b1" });
  });

  it("a plane INSIDE a block cuts in both directions", () => {
    expect(autoModeFor(EMBEDDED, 5, NEW)).toEqual({ mode: "Cut", targetBodyId: "b1" });
    expect(autoModeFor(EMBEDDED, -5, NEW)).toEqual({ mode: "Cut", targetBodyId: "b1" });
  });

  it("a body 10 away is NOT touched until the prism reaches it", () => {
    expect(autoModeFor(ABOVE, -5, NEW)).toEqual(NEW); // still in flight
    expect(autoModeFor(ABOVE, -20, NEW)).toEqual({ mode: "Add", targetBodyId: "b1" });
    // …and never in the direction with nothing in it.
    expect(autoModeFor(ABOVE, 40, NEW)).toEqual(NEW);
  });

  it("holds through the contact threshold instead of strobing across it", () => {
    const gap = 10;
    const justShort = -(gap + CONTACT_FLIP_HOLD / 2);
    // Coming from NewBody, it takes the FULL band to flip on…
    expect(autoModeFor(ABOVE, justShort, NEW)).toEqual(NEW);
    // …and once Add, the same depth stays Add rather than falling back.
    const add: AutoMode = { mode: "Add", targetBodyId: "b1" };
    expect(autoModeFor(ABOVE, justShort, add)).toEqual(add);
    // Well clear of the band it resolves the same way from either side.
    expect(autoModeFor(ABOVE, -(gap + 5), NEW)).toEqual(add);
    expect(autoModeFor(ABOVE, -(gap - 5), add)).toEqual(NEW);
  });

  it("the hysteresis is per-BODY: a different target does not inherit the hold", () => {
    const otherAdd: AutoMode = { mode: "Add", targetBodyId: "b2" };
    const justShort = -(10 + CONTACT_FLIP_HOLD / 2);
    expect(autoModeFor(ABOVE, justShort, otherAdd)).toEqual(NEW);
  });

  it("a body BEHIND that is merely near (not touching) does not force an Add", () => {
    // Nothing ahead, a body 3 behind but not flush: this is not push/pull, and
    // guessing Add against a body the prism grows away from would be a mis-bind.
    const behindNear: MaterialSides = { pos: null, neg: probe("b1", 3) };
    expect(autoModeFor(behindNear, 20, NEW)).toEqual(NEW);
  });
});

describe("hasMaterial", () => {
  it("is false only when both probes came back empty", () => {
    expect(hasMaterial(NO_MATERIAL)).toBe(false);
    expect(hasMaterial(ON_FACE)).toBe(true);
    expect(hasMaterial({ pos: probe("b1", 4), neg: null })).toBe(true);
  });
});
