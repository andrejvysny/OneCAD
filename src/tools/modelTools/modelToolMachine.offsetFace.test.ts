/*
 * OffsetFace reducer (SCHEMA §7.3 `op.offsetFace`).
 *
 * The two rules worth a test file of their own:
 *
 *  1. THE DRAG BELONGS TO `Offset`. `Total`/`Radius`/`Diameter` are absolute
 *     values with no zero to drag from, so a `drag` there must be INERT — not
 *     re-scaled, not partially applied. A drag silently reinterpreted as a
 *     diameter would author a number the user never typed.
 *  2. NOTHING IS CLAMPED (SCHEMA §7.3 is explicit). An out-of-domain entry is
 *     REJECTED — the state keeps its last valid value and `valueError` goes true —
 *     because a clamped value desynchronizes the stored param, the preview the
 *     user approved and the geometry the next regen builds.
 */
import { describe, it, expect } from "vitest";
import {
  DEFAULT_OFFSET_DISTANCE,
  offsetCanConfirm,
  offsetFaceInit,
  offsetFaceStep,
  offsetValueInDomain,
  type OffsetFaceFsm,
} from "./modelToolMachine";

/** An armed offset over `faceCount` faces, at `Offset` unless told otherwise. */
function armed(over: Partial<Parameters<typeof offsetFaceStep>[1] & { faceCount: number }> = {}): OffsetFaceFsm {
  const step = offsetFaceStep(offsetFaceInit(), {
    kind: "arm",
    faceCount: 1,
    ...over,
  } as Parameters<typeof offsetFaceStep>[1]);
  return step.state;
}

describe("offsetFaceStep — arm", () => {
  it("arms at the default distance and emits `begin`", () => {
    const step = offsetFaceStep(offsetFaceInit(), { kind: "arm", faceCount: 3 });
    expect(step.effect).toBe("begin");
    expect(step.state).toMatchObject({
      phase: "armed",
      distance: DEFAULT_OFFSET_DISTANCE,
      distanceType: "Offset",
      chainTangentFaces: true,
      faceCount: 3,
      touched: false,
      valueError: false,
    });
  });

  it("bails on an empty closure", () => {
    const step = offsetFaceStep(offsetFaceInit(), { kind: "arm", faceCount: 0 });
    expect(step.effect).toBe("none");
    expect(step.state.phase).toBe("idle");
  });

  it("REFUSES a non-Offset type over a multi-face closure (never downgrades it)", () => {
    // SCHEMA §7.3: only `Offset` admits a multi-face set. Silently arming as an
    // Offset would leave the caller believing it armed a Radius over four faces.
    const step = offsetFaceStep(offsetFaceInit(), {
      kind: "arm",
      faceCount: 4,
      distanceType: "Radius",
    });
    expect(step.effect).toBe("none");
    expect(step.state.phase).toBe("idle");
  });

  it("forces the tangent chain OFF for Total, whatever the caller asked", () => {
    const s = armed({ distanceType: "Total", chainTangentFaces: true });
    expect(s.distanceType).toBe("Total");
    expect(s.chainTangentFaces).toBe(false);
  });
});

describe("offsetFaceStep — drag is Offset-only", () => {
  it("grabs and drags a free Offset", () => {
    const grabbed = offsetFaceStep(armed(), { kind: "grab" });
    expect(grabbed.state.phase).toBe("dragging");
    const dragged = offsetFaceStep(grabbed.state, { kind: "drag", distance: -4.5 });
    expect(dragged.effect).toBe("update");
    expect(dragged.state.distance).toBe(-4.5); // signed: an Offset may shrink
    expect(dragged.state.touched).toBe(true);
  });

  it("refuses a grab for every ABSOLUTE type", () => {
    for (const distanceType of ["Total", "Radius", "Diameter"] as const) {
      const s = armed({ distanceType, chainTangentFaces: false, distance: 10 });
      expect(offsetFaceStep(s, { kind: "grab" }).state.phase).toBe("armed");
    }
  });

  it("IGNORES a drag frame that reaches an absolute type", () => {
    // Not re-scaled, not partially applied — inert.
    const s: OffsetFaceFsm = { ...armed({ distanceType: "Radius", distance: 10 }), phase: "dragging" };
    const step = offsetFaceStep(s, { kind: "drag", distance: 99 });
    expect(step.effect).toBe("none");
    expect(step.state.distance).toBe(10);
  });

  it("ignores a non-finite drag frame", () => {
    const dragging = offsetFaceStep(armed(), { kind: "grab" }).state;
    expect(offsetFaceStep(dragging, { kind: "drag", distance: Number.NaN }).state.distance).toBe(
      DEFAULT_OFFSET_DISTANCE,
    );
  });

  it("release keeps the tool ARMED at the dragged value (no implicit commit)", () => {
    const dragging = offsetFaceStep(armed(), { kind: "grab" }).state;
    const dragged = offsetFaceStep(dragging, { kind: "drag", distance: 7 }).state;
    const released = offsetFaceStep(dragged, { kind: "release" });
    expect(released.effect).toBe("update");
    expect(released.state).toMatchObject({ phase: "armed", distance: 7 });
  });
});

describe("offsetFaceStep — setDistance never clamps", () => {
  it("accepts any finite value for a free Offset, sign included", () => {
    const step = offsetFaceStep(armed(), { kind: "setDistance", distance: -0.5 });
    expect(step.effect).toBe("update");
    expect(step.state.distance).toBe(-0.5);
    expect(step.state.valueError).toBe(false);
  });

  it("REJECTS a non-positive absolute value, keeping the last valid one", () => {
    const s = armed({ distanceType: "Diameter", distance: 8 });
    const step = offsetFaceStep(s, { kind: "setDistance", distance: -3 });
    expect(step.effect).toBe("none");
    expect(step.state.distance).toBe(8); // untouched — NOT clamped to +3 or to 0
    expect(step.state.valueError).toBe(true);
  });

  it("rejects a non-finite value for every type", () => {
    const step = offsetFaceStep(armed(), { kind: "setDistance", distance: Number.POSITIVE_INFINITY });
    expect(step.state.distance).toBe(DEFAULT_OFFSET_DISTANCE);
    expect(step.state.valueError).toBe(true);
  });

  it("clears the error flag on the next value that IS in domain", () => {
    const bad = offsetFaceStep(armed({ distanceType: "Radius", distance: 5 }), {
      kind: "setDistance",
      distance: 0,
    }).state;
    expect(bad.valueError).toBe(true);
    expect(offsetFaceStep(bad, { kind: "setDistance", distance: 6 }).state).toMatchObject({
      distance: 6,
      valueError: false,
    });
  });

  it("is inert while idle", () => {
    expect(offsetFaceStep(offsetFaceInit(), { kind: "setDistance", distance: 4 }).effect).toBe("none");
  });
});

describe("offsetFaceStep — setDistanceType", () => {
  it("REFUSES a non-Offset type when the closure is not a single face", () => {
    const s = armed({ faceCount: 3 });
    expect(offsetFaceStep(s, { kind: "setDistanceType", distanceType: "Radius" }).effect).toBe("none");
    expect(offsetFaceStep(s, { kind: "setDistanceType", distanceType: "Radius" }).state.distanceType).toBe(
      "Offset",
    );
  });

  it("emits `begin` — the CLOSURE depends on the type, so it is a re-arm", () => {
    const step = offsetFaceStep(armed(), { kind: "setDistanceType", distanceType: "Diameter" });
    expect(step.effect).toBe("begin");
    expect(step.state.distanceType).toBe("Diameter");
  });

  it("reseeds an UNTOUCHED value and preserves a touched one", () => {
    const pristine = armed({ distance: 4 });
    expect(
      offsetFaceStep(pristine, { kind: "setDistanceType", distanceType: "Radius" }).state.distance,
    ).toBe(DEFAULT_OFFSET_DISTANCE);
    const touched = offsetFaceStep(pristine, { kind: "setDistance", distance: 9 }).state;
    expect(
      offsetFaceStep(touched, { kind: "setDistanceType", distanceType: "Radius" }).state.distance,
    ).toBe(9);
  });

  it("turns the tangent chain off when switching TO Total", () => {
    const step = offsetFaceStep(armed(), { kind: "setDistanceType", distanceType: "Total" });
    expect(step.state.chainTangentFaces).toBe(false);
  });

  it("is a no-op when the type has not changed", () => {
    expect(offsetFaceStep(armed(), { kind: "setDistanceType", distanceType: "Offset" }).effect).toBe(
      "none",
    );
  });
});

describe("offsetFaceStep — setChainTangent", () => {
  it("emits `begin`: toggling changes the closure the record freezes", () => {
    const step = offsetFaceStep(armed(), { kind: "setChainTangent", chainTangentFaces: false });
    expect(step.effect).toBe("begin");
    expect(step.state.chainTangentFaces).toBe(false);
  });

  it("is inert for Total (single face, chain off by definition)", () => {
    const s = armed({ distanceType: "Total", chainTangentFaces: false, distance: 10 });
    const step = offsetFaceStep(s, { kind: "setChainTangent", chainTangentFaces: true });
    expect(step.effect).toBe("none");
    expect(step.state.chainTangentFaces).toBe(false);
  });

  it("is a no-op when the flag has not changed", () => {
    expect(offsetFaceStep(armed(), { kind: "setChainTangent", chainTangentFaces: true }).effect).toBe(
      "none",
    );
  });
});

describe("offsetFaceStep — seed (PrepareOffsetFace.currentDims)", () => {
  it("seeds an untouched absolute value", () => {
    const s = armed({ distanceType: "Radius", distance: DEFAULT_OFFSET_DISTANCE });
    const step = offsetFaceStep(s, { kind: "seed", distance: 10 });
    expect(step.effect).toBe("update");
    expect(step.state.distance).toBe(10);
    // A seed is the KERNEL's number, not the user's — it must not mark the value
    // as authored, or the next type switch would refuse to reseed.
    expect(step.state.touched).toBe(false);
  });

  it("never overwrites a value the user authored", () => {
    const touched = offsetFaceStep(armed({ distanceType: "Radius", distance: 5 }), {
      kind: "setDistance",
      distance: 9,
    }).state;
    expect(offsetFaceStep(touched, { kind: "seed", distance: 10 }).state.distance).toBe(9);
  });

  it("rejects an out-of-domain seed", () => {
    const s = armed({ distanceType: "Radius", distance: 5 });
    expect(offsetFaceStep(s, { kind: "seed", distance: 0 }).state.distance).toBe(5);
  });
});

describe("offsetFaceStep — confirm", () => {
  it("commits an armed offset above the degenerate magnitude", () => {
    const s = offsetFaceStep(armed(), { kind: "setDistance", distance: 2 }).state;
    const step = offsetFaceStep(s, { kind: "confirm" });
    expect(step.effect).toBe("commit");
    expect(step.state.phase).toBe("committing");
  });

  it("REFUSES a degenerate |Offset| — an identity no-op is not a feature", () => {
    const s = offsetFaceStep(armed(), { kind: "setDistance", distance: 0.0004 }).state;
    const step = offsetFaceStep(s, { kind: "confirm" });
    expect(step.effect).toBe("none");
    expect(step.state.phase).toBe("armed");
    expect(step.state.valueError).toBe(true);
    expect(step.state.distance).toBe(0.0004); // still NOT clamped
  });

  it("commits a negative Offset (shrinking is a legal direction)", () => {
    const s = offsetFaceStep(armed(), { kind: "setDistance", distance: -2 }).state;
    expect(offsetFaceStep(s, { kind: "confirm" }).effect).toBe("commit");
  });

  it("only fires from `armed`", () => {
    const dragging = offsetFaceStep(armed(), { kind: "grab" }).state;
    expect(offsetFaceStep(dragging, { kind: "confirm" }).effect).toBe("none");
  });

  it("commitFailed returns to armed so the closure + distance survive", () => {
    const committing = offsetFaceStep(
      offsetFaceStep(armed(), { kind: "setDistance", distance: 3 }).state,
      { kind: "confirm" },
    ).state;
    const back = offsetFaceStep(committing, { kind: "commitFailed" });
    expect(back.state).toMatchObject({ phase: "armed", distance: 3, faceCount: 1 });
  });
});

describe("offsetFaceStep — cancel / settle", () => {
  it("cancel from a live phase emits `cancel` and resets", () => {
    const step = offsetFaceStep(armed(), { kind: "cancel" });
    expect(step.effect).toBe("cancel");
    expect(step.state).toEqual(offsetFaceInit());
  });

  it("cancel while idle is inert (the sweep is idempotent)", () => {
    expect(offsetFaceStep(offsetFaceInit(), { kind: "cancel" }).effect).toBe("none");
  });

  it("settle always resets", () => {
    expect(offsetFaceStep(armed(), { kind: "settle" }).state).toEqual(offsetFaceInit());
  });
});

describe("offsetValueInDomain / offsetCanConfirm", () => {
  it("Offset admits any finite value; the absolute types demand a positive one", () => {
    expect(offsetValueInDomain(-5, "Offset")).toBe(true);
    expect(offsetValueInDomain(0, "Offset")).toBe(true);
    expect(offsetValueInDomain(Number.NaN, "Offset")).toBe(false);
    for (const t of ["Total", "Radius", "Diameter"] as const) {
      expect(offsetValueInDomain(1, t)).toBe(true);
      expect(offsetValueInDomain(0, t)).toBe(false);
      expect(offsetValueInDomain(-1, t)).toBe(false);
    }
  });

  it("confirm is stricter than the domain: it also refuses a degenerate magnitude", () => {
    expect(offsetValueInDomain(0.0004, "Offset")).toBe(true); // legal to HOLD…
    expect(offsetCanConfirm({ ...armed(), distance: 0.0004 })).toBe(false); // …not to WRITE
    expect(offsetCanConfirm({ ...armed(), distance: 0.002 })).toBe(true);
    expect(offsetCanConfirm({ ...armed(), faceCount: 0, distance: 5 })).toBe(false);
  });
});
