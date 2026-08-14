/**
 * Gear wire-marshalling contract (SCHEMA §7.3 `Gear`; Gear Generator G1).
 *
 * Two things are pinned here, and both are correctness seams rather than
 * formatting details:
 *
 *  1. **Key-for-key wire shape.** The Rust `GearParams` serde is the other half
 *     of this contract and cannot see this file, so a rename on either side is
 *     only caught by asserting the actual keys. A round-trip test would pass
 *     happily through a mutual rename.
 *
 *  2. **PREVIEW == COMMIT.** SCHEMA §7.6 forbids a second ad-hoc mapper, and the
 *     preview lane is where a divergence would be invisible: the ghost would be
 *     built from different numbers than the committed record. `previewOps.gearOp`
 *     and `gearMachine.gearParamsOf` are therefore asserted to agree, not merely
 *     to each be individually plausible.
 */

import { describe, expect, it } from "vitest";

import { buildPreviewOp } from "./previewOps";
import { wireOperation } from "./tauriCommandMap";
import type { GearParams, SemanticRef } from "./types";
import {
  gearInit,
  gearParamsOf,
  gearStep,
  type GearEvent,
  type GearFsm,
} from "@/tools/modelTools/gearMachine";

const FACE: SemanticRef = {
  primary: { bodyId: "body_1", elementId: "el_top", kind: "face" },
  anchor: { worldPoint: [10, 10, 25] },
} as unknown as SemanticRef;

function run(events: GearEvent[], from: GearFsm = gearInit()): GearFsm {
  return events.reduce((s, e) => gearStep(s, e).state, from);
}

function armedOnFace(extra: GearEvent[] = []): GearFsm {
  return run([
    { kind: "start" },
    { kind: "pickFace", face: FACE, point: [10, 10, 25] },
    ...extra,
  ]);
}

describe("gear wire shape", () => {
  it("marshals the SCHEMA §7.3 keys exactly", () => {
    const wire = wireOperation({ opType: "Gear", opId: "op1", params: gearParamsOf(armedOnFace()) });
    expect(wire.opType).toBe("Gear");
    const p = wire.params as unknown as Record<string, unknown>;
    expect(Object.keys(p).sort()).toEqual(["involuteExternal", "placement", "recipe"]);
    expect(p.recipe).toBe("involuteExternal");

    const placement = p.placement as Record<string, unknown>;
    expect(Object.keys(placement).sort()).toEqual(["face", "frame", "point"]);
    // The INACTIVE placement mode is spelled null, never omitted.
    expect(placement.frame).toBeNull();
    expect(placement.face).not.toBeNull();

    const inv = p.involuteExternal as Record<string, unknown>;
    // Dimensions marshal as wire Scalars (`{value}`); coefficients stay bare
    // numbers, because they are multiples of module and not drivable lengths.
    expect(inv.module).toEqual({ value: 2 });
    expect(inv.height).toEqual({ value: 5 });
    expect(inv.pressureAngleDeg).toEqual({ value: 20 });
    expect(inv.teeth).toBe(20);
    expect(inv.clearance).toBe(0.25);
    expect(inv.shift).toBe(0);
    expect(inv.sampleCount).toBe(20);
  });

  it("spells every inactive conditional null rather than omitting it", () => {
    const wire = wireOperation({ opType: "Gear", params: gearParamsOf(armedOnFace()) });
    const inv = (wire.params as unknown as Record<string, unknown>).involuteExternal as Record<string, unknown>;
    // A `null` is what the Rust `Option<Scalar>` reads back as; an omitted key
    // would make "this gear has no bore" an omission instead of a fact.
    expect(inv).toHaveProperty("axleHoleDiameter", null);
    expect(inv).toHaveProperty("offsetHoleDiameter", null);
    expect(inv).toHaveProperty("offsetHoleOffset", null);
  });

  it("drops a bore dimension left behind after its flag was switched off", () => {
    const s = run(
      [
        { kind: "setAxleHole", on: true },
        { kind: "setAxleHoleDiameter", value: 12 },
        { kind: "setAxleHole", on: false },
      ],
      armedOnFace(),
    );
    const wire = wireOperation({ opType: "Gear", params: gearParamsOf(s) });
    const inv = (wire.params as unknown as Record<string, unknown>).involuteExternal as Record<string, unknown>;
    expect(inv.axleHole).toBe(false);
    expect(inv.axleHoleDiameter).toBeNull();
  });

  it("marshals a frame placement with a null face", () => {
    const framed = run([
      { kind: "start" },
      { kind: "pickFrame", origin: [1, 2, 3], axis: [0, 0, 1], xDir: [1, 0, 0] },
    ]);
    const wire = wireOperation({ opType: "Gear", params: gearParamsOf(framed) });
    const placement = (wire.params as unknown as Record<string, unknown>).placement as Record<string, unknown>;
    expect(placement.face).toBeNull();
    expect(placement.frame).toEqual({ origin: [1, 2, 3], axis: [0, 0, 1], xDir: [1, 0, 0] });
    expect(placement.point).toEqual([1, 2, 3]);
  });
});

describe("gear preview == commit (SCHEMA §7.6)", () => {
  it("builds the identical wire op through both lanes", () => {
    const params: GearParams = gearParamsOf(
      armedOnFace([
        { kind: "setTeeth", teeth: 31 },
        { kind: "setModule", module: 1.5 },
        { kind: "setAxleHole", on: true },
        { kind: "setAxleHoleDiameter", value: 8 },
      ]),
    );

    const commitWire = wireOperation({ opType: "Gear", opId: "op1", params });

    const previewOp = buildPreviewOp({
      opId: "op1",
      opType: "Gear",
      latestParams: params as unknown as Record<string, unknown>,
    });
    const previewWire = wireOperation(previewOp);

    // Not "both look reasonable" — byte-identical, which is the only form of
    // this assertion that a divergence cannot slip through.
    expect(previewWire).toEqual(commitWire);
  });

  it("agrees for a frame placement too", () => {
    const params = gearParamsOf(
      run([
        { kind: "start" },
        { kind: "pickFrame", origin: [4, 0, 0], axis: [0, 1, 0], xDir: [0, 0, 1] },
      ]),
    );
    const commitWire = wireOperation({ opType: "Gear", opId: "op2", params });
    const previewWire = wireOperation(
      buildPreviewOp({
        opId: "op2",
        opType: "Gear",
        latestParams: params as unknown as Record<string, unknown>,
      }),
    );
    expect(previewWire).toEqual(commitWire);
  });

  it("refuses to preview a payload with no placement rather than guessing one", () => {
    expect(() =>
      buildPreviewOp({
        opId: "op3",
        opType: "Gear",
        latestParams: { recipe: "involuteExternal" } as unknown as Record<string, unknown>,
      }),
    ).toThrow();
  });

  it("refuses a payload carrying BOTH placement modes", () => {
    expect(() =>
      buildPreviewOp({
        opId: "op4",
        opType: "Gear",
        latestParams: {
          recipe: "involuteExternal",
          placement: {
            face: FACE,
            frame: { origin: [0, 0, 0], axis: [0, 0, 1], xDir: [1, 0, 0] },
            point: [0, 0, 0],
          },
          involuteExternal: { teeth: 20, module: 2, height: 5, pressureAngleDeg: 20 },
        } as unknown as Record<string, unknown>,
      }),
    ).toThrow();
  });
});
