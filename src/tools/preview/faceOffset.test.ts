import { describe, it, expect } from "vitest";
import {
  DEFAULT_OFFSET_DISTANCE,
  cylindricalOffsetPath,
  distanceFromValueText,
  ghostOffsets,
  offsetAnchorFor,
  offsetAxisFor,
  offsetFaceValue,
  offsetTargetWitness,
  orientTowardReference,
  planarOffsetPath,
  radialDimensionPath,
  radialDimensionWitness,
  radialFrameAt,
  totalThicknessPath,
  totalThicknessWitnesses,
  OFFSET_AXIS_MAX_DIVERGENCE_RAD,
  type FaceFrame,
} from "./faceOffset";
import {
  classifyMapping,
  handlePointAt,
  sampleMapping,
  type AnchorScale,
  type LinearHandlePath,
  type ProjectionContext,
} from "./handleProjection";

/**
 * SCHEMA §7.3 `op.offsetFace`: the signed offset the WORKER derives from an
 * absolute dimension. Restated here — never in `faceOffset.ts`, which must not
 * duplicate a kernel computation — so the drag-direction specs can show that
 * growing an inner Radius outward is a NEGATIVE signed offset.
 */
const kernelSignedOffset = (
  value: number,
  currentRadius: number,
  kind: "Radius" | "Diameter",
  sidedness: "pin" | "hole",
): number => (sidedness === "pin" ? 1 : -1) * ((kind === "Diameter" ? value / 2 : value) - currentRadius);

const frame = (center: [number, number, number], normal: [number, number, number]): FaceFrame => ({
  center,
  normal,
});

describe("offsetAxisFor", () => {
  it("returns the unit mean normal for one face", () => {
    expect(offsetAxisFor([frame([0, 0, 10], [0, 0, 3])])).toEqual([0, 0, 1]);
  });

  it("averages agreeing normals and re-normalizes", () => {
    // Two faces 60° apart — inside the 45°-from-the-MEAN tolerance (each deviates
    // 30°), so the pair still has one honest arrow between them.
    const axis = offsetAxisFor([
      frame([0, 0, 0], [1, 0, 0]),
      frame([0, 0, 0], [Math.cos(Math.PI / 3), Math.sin(Math.PI / 3), 0]),
    ]);
    expect(axis).not.toBeNull();
    expect(Math.hypot(...(axis as [number, number, number]))).toBeCloseTo(1, 12);
    // The bisector of the two: 30° off each.
    expect(axis?.[0]).toBeCloseTo(Math.cos(Math.PI / 6), 10);
    expect(axis?.[1]).toBeCloseTo(Math.sin(Math.PI / 6), 10);
  });

  it("REFUSES opposing normals rather than averaging them to nothing", () => {
    // The pair that motivates the whole refusal: their sum is the zero vector, so
    // a naive mean would normalize noise into a fabricated direction.
    expect(offsetAxisFor([frame([0, 0, 0], [0, 0, 1]), frame([0, 0, 0], [0, 0, -1])])).toBeNull();
  });

  it("REFUSES a set whose worst normal exceeds the divergence tolerance", () => {
    // Perpendicular faces: each is 45° from the mean, which is exactly AT the
    // tolerance — nudge one past it and the axis is refused.
    const past = OFFSET_AXIS_MAX_DIVERGENCE_RAD + 0.05;
    expect(
      offsetAxisFor([frame([0, 0, 0], [1, 0, 0]), frame([0, 0, 0], [Math.cos(2 * past), Math.sin(2 * past), 0])]),
    ).toBeNull();
  });

  it("refuses an empty set and a degenerate normal", () => {
    expect(offsetAxisFor([])).toBeNull();
    expect(offsetAxisFor([frame([0, 0, 0], [0, 0, 0])])).toBeNull();
  });
});

describe("offsetAnchorFor", () => {
  it("is the plain mean of the face centres", () => {
    expect(
      offsetAnchorFor([frame([0, 0, 0], [0, 0, 1]), frame([2, 4, 6], [0, 0, 1])]),
    ).toEqual([1, 2, 3]);
  });

  it("refuses an empty set and a non-finite centre (never substitutes the origin)", () => {
    expect(offsetAnchorFor([])).toBeNull();
    expect(offsetAnchorFor([frame([0, Number.NaN, 0], [0, 0, 1])])).toBeNull();
  });
});

describe("ghostOffsets", () => {
  it("moves EACH face along its OWN normal, not along the mean", () => {
    const offsets = ghostOffsets(
      [frame([0, 0, 0], [1, 0, 0]), frame([0, 0, 0], [0, 1, 0])],
      2.5,
    );
    expect(offsets).toEqual([
      [2.5, 0, 0],
      [0, 2.5, 0],
    ]);
  });

  it("normalizes the frame normal before scaling", () => {
    expect(ghostOffsets([frame([0, 0, 0], [0, 0, 4])], 3)).toEqual([[0, 0, 3]]);
  });

  it("carries the SIGN — a negative offset pulls the face back", () => {
    const [offset] = ghostOffsets([frame([0, 0, 0], [0, 0, 1])], -2);
    expect(offset[2]).toBe(-2);
    expect(offset[0]).toBeCloseTo(0, 12); // ±0 either way; the magnitude is what matters
    expect(offset[1]).toBeCloseTo(0, 12);
  });

  it("drops a degenerate frame rather than translating it by zero", () => {
    expect(ghostOffsets([frame([0, 0, 0], [0, 0, 0]), frame([0, 0, 0], [0, 0, 1])], 1)).toEqual([
      [0, 0, 1],
    ]);
  });

  it("yields nothing for a non-finite distance", () => {
    expect(ghostOffsets([frame([0, 0, 0], [0, 0, 1])], Number.NaN)).toEqual([]);
  });
});

describe("offsetFaceValue (mirrors dto.rs feature_value)", () => {
  it("prefixes the absolute cylindrical forms and keeps mm for the rest", () => {
    expect(offsetFaceValue(2.5, "Offset")).toEqual({
      valueText: "2.5 mm",
      primaryValue: 2.5,
      primaryValueKind: "length",
    });
    expect(offsetFaceValue(12, "Total")).toEqual({
      valueText: "12.0 mm",
      primaryValue: 12,
      primaryValueKind: "length",
    });
    expect(offsetFaceValue(6, "Radius")).toEqual({
      valueText: "R6.0",
      primaryValue: 6,
      primaryValueKind: "length",
    });
    expect(offsetFaceValue(8, "Diameter")).toEqual({
      valueText: "Ø8.0",
      primaryValue: 8,
      primaryValueKind: "diameter",
    });
  });

  it("keeps a NEGATIVE offset's sign in the row text", () => {
    expect(offsetFaceValue(-2.5, "Offset").valueText).toBe("-2.5 mm");
  });
});

describe("distanceFromValueText", () => {
  it("round-trips every shape offsetFaceValue emits", () => {
    for (const [d, t] of [
      [2.5, "Offset"],
      [12, "Total"],
      [6, "Radius"],
      [8, "Diameter"],
    ] as const) {
      expect(distanceFromValueText(offsetFaceValue(d, t).valueText)).toBe(d);
    }
  });

  it("keeps a negative offset NEGATIVE (the sign is meaningful here)", () => {
    // The fillet/shell parsers fall back on a non-positive value; doing that here
    // would silently turn a shrinking offset into a growing one.
    expect(distanceFromValueText("-2.5 mm")).toBe(-2.5);
  });

  it("falls back only on text with no number at all", () => {
    expect(distanceFromValueText("—")).toBe(DEFAULT_OFFSET_DISTANCE);
    expect(distanceFromValueText("", 7)).toBe(7);
  });
});

/*
 * H10 — the OFFSET ATTACHMENT PATHS (docs/design/astra/modeling-handle-attachment.md
 * §5 "Offset: …", §6 test vectors, §7 "Inner cylinder sign inversion").
 *
 * Camera O below is the derivation's §6 orthographic camera: eye (0,0,100) down
 * −Z, 1000 × 1000 CSS px, 100 mm view height, so `screen(P) = (500 + 10·Px,
 * 500 − 10·Py)` and `s = 0.1 mm/px`. Every expected number is the derivation's,
 * quoted in the assertion it pins.
 */

/** Column-major orthographic matrix for a symmetric frustum (three's order). */
function orthographic(halfW: number, halfH: number, near: number, far: number): number[] {
  const m = new Array<number>(16).fill(0);
  m[0] = 1 / halfW;
  m[5] = 1 / halfH;
  m[10] = -2 / (far - near);
  m[14] = -(far + near) / (far - near);
  m[15] = 1;
  return m;
}

/** `proj · translate(-eye)` — a camera at `eye` looking down world −Z. */
function viewProjAt(proj: number[], eye: [number, number, number]): number[] {
  const m = proj.slice();
  m[12] = proj[0] * -eye[0] + proj[4] * -eye[1] + proj[8] * -eye[2] + proj[12];
  m[13] = proj[1] * -eye[0] + proj[5] * -eye[1] + proj[9] * -eye[2] + proj[13];
  m[14] = proj[2] * -eye[0] + proj[6] * -eye[1] + proj[10] * -eye[2] + proj[14];
  m[15] = proj[3] * -eye[0] + proj[7] * -eye[1] + proj[11] * -eye[2] + proj[15];
  return m;
}

const VP = 1000;
const cameraO = (): ProjectionContext => ({
  viewProj: viewProjAt(orthographic(50, 50, 0.1, 1000), [0, 0, 100]),
  viewportWidth: VP,
  viewportHeight: VP,
});
const SCALE_O: AnchorScale = { worldPerPx: 0.1 };

/** The value a `+dxPx` horizontal drag lands on, through the frozen mapping. */
function dragTo(path: LinearHandlePath, startMm: number, dxPx: number): number {
  const mapping = classifyMapping(
    { q0Mm: startMm, point0Mm: handlePointAt(path, startMm), dPointDValue: path.dPointDValue },
    cameraO(),
    SCALE_O,
  );
  expect(mapping.kind).toBe("world");
  return sampleMapping(mapping, [dxPx, 0]).valueMm;
}

/** `Cq` for a path seated at `valueMm` under camera O — unclamped, as logged. */
function conditioningAt(path: LinearHandlePath, valueMm: number): number {
  const mapping = classifyMapping(
    { q0Mm: valueMm, point0Mm: handlePointAt(path, valueMm), dPointDValue: path.dPointDValue },
    cameraO(),
    SCALE_O,
  );
  return mapping.kind === "world" ? mapping.conditioning : 0;
}

describe("radialFrameAt — the cylindrical attachment's own frame", () => {
  it("stations the centre at the sample and normalizes the radial direction", () => {
    const frame = radialFrameAt([0, 0, 0], [0, 0, 2], [10, 0, 40])!;
    expect(frame.centreMm).toEqual([0, 0, 40]);
    expect(frame.radial).toEqual([1, 0, 0]);
  });

  it("REFUSES a sample on the axis — the radial anchor is unresolved (§4)", () => {
    // §6 "Zero/unresolved radius": P = C. There is no radial direction, so the
    // caller must try another real surface sample rather than normalize noise.
    expect(radialFrameAt([0, 0, 0], [0, 0, 1], [0, 0, 40])).toBeNull();
    expect(radialFrameAt([0, 0, 0], [0, 0, 0], [10, 0, 40])).toBeNull();
    expect(radialFrameAt([Number.NaN, 0, 0], [0, 0, 1], [10, 0, 40])).toBeNull();
  });
});

describe("radialDimensionPath — H(R) = C + R·r̂, H(D) = C + (D/2)·r̂", () => {
  it("OUTER Radius: R0 = 10 mm, r̂ = +X, +20 px ⇒ 12 mm (§6 'Outer Radius')", () => {
    const path = radialDimensionPath([0, 0, 0], [0, 0, 1], [10, 0, 0], "Radius")!;
    expect(path.q0Mm).toBe(0);
    expect(path.point0Mm).toEqual([0, 0, 0]);
    expect(path.dPointDValue).toEqual([1, 0, 0]);
    expect(handlePointAt(path, 10)).toEqual([10, 0, 0]);
    expect(dragTo(path, 10, 20)).toBeCloseTo(12, 12);
  });

  it("INNER Radius grows OUTWARD too: 6 → 8 mm on +20 px (§7 sign inversion)", () => {
    // The counterexample. Material-outward for a hole is −r̂, so using it as the
    // increasing-Radius direction would run this drag backwards. σ maps only to
    // the kernel's signed offset `d = σ(R − R0) = −1·2 = −2 mm`.
    const path = radialDimensionPath([0, 0, 0], [0, 0, 1], [6, 0, 0], "Radius")!;
    expect(path.dPointDValue).toEqual([1, 0, 0]);
    expect(dragTo(path, 6, 20)).toBeCloseTo(8, 12);
    expect(kernelSignedOffset(8, 6, "Radius", "hole")).toBeCloseTo(-2, 12);
    expect(kernelSignedOffset(12, 10, "Radius", "pin")).toBeCloseTo(2, 12);
  });

  it("INNER Diameter: 12 → 16 mm on the SAME +20 px, because dH/dD = r̂/2", () => {
    const path = radialDimensionPath([0, 0, 0], [0, 0, 1], [6, 0, 0], "Diameter")!;
    expect(path.dPointDValue).toEqual([0.5, 0, 0]);
    expect(handlePointAt(path, 12)).toEqual([6, 0, 0]);
    expect(dragTo(path, 12, 20)).toBeCloseTo(16, 12);
    expect(kernelSignedOffset(16, 6, "Diameter", "hole")).toBeCloseTo(-2, 12);
  });

  it("judges sensitivity on Cq = g·s INCLUDING Diameter's ½ (§6 'Diameter threshold')", () => {
    // Radial conditioning 0.1598 / 0.1602 ⇒ scalar 0.0799 / 0.0801 ⇒ proxy / world.
    for (const [radial, expectedWorld] of [[0.1598, false], [0.1602, true]] as const) {
      const c = radial; // Cq of the unit radial under camera O is 10·c·0.1 = c
      const tilted: [number, number, number] = [c, 0, Math.sqrt(1 - c * c)];
      const path = radialDimensionPath([0, 0, 0], [0, 1, 0], [tilted[0] * 6, 0, tilted[2] * 6], "Diameter")!;
      const mapping = classifyMapping(
        { q0Mm: 12, point0Mm: handlePointAt(path, 12), dPointDValue: path.dPointDValue },
        cameraO(),
        SCALE_O,
      );
      expect(conditioningAt(radialDimensionPath([0, 0, 0], [0, 1, 0], [tilted[0] * 6, 0, tilted[2] * 6], "Radius")!, 6))
        .toBeCloseTo(radial, 6);
      expect(mapping.kind).toBe(expectedWorld ? "world" : "proxy");
    }
  });

  it("an END-ON cylinder AXIS is irrelevant: Radius Cq = 1, Diameter Cq = 0.5", () => {
    // §6 "Cylinder axis end-on". The axis projects to nothing; what is tested is
    // the RADIAL manipulation derivative, and both stay world mappings.
    const radius = radialDimensionPath([0, 0, 0], [0, 0, 1], [10, 0, 0], "Radius")!;
    const diameter = radialDimensionPath([0, 0, 0], [0, 0, 1], [10, 0, 0], "Diameter")!;
    expect(conditioningAt(radius, 10)).toBeCloseTo(1, 12);
    expect(conditioningAt(diameter, 20)).toBeCloseTo(0.5, 12);
  });

  it("an END-ON RADIAL direction is a proxy, despite a visible cylinder axis", () => {
    // §6 "Radial direction end-on": axis +X (fully across the screen), r̂ = +Z.
    const path = radialDimensionPath([0, 0, 0], [1, 0, 0], [0, 0, 10], "Radius")!;
    expect(path.dPointDValue).toEqual([0, 0, 1]);
    const mapping = classifyMapping(
      { q0Mm: 10, point0Mm: handlePointAt(path, 10), dPointDValue: path.dPointDValue },
      cameraO(),
      SCALE_O,
    );
    expect(mapping.kind).toBe("proxy");
  });

  it("refuses an unresolvable radial anchor rather than producing NaN", () => {
    expect(radialDimensionPath([0, 0, 0], [0, 0, 1], [0, 0, 40], "Radius")).toBeNull();
  });
});

describe("cylindricalOffsetPath — H(q) = C + (R0 + σq)·r̂", () => {
  it("INNER tube R = 6, σ = −1, q = +2 mm ⇒ radial coordinate 4 mm (§6)", () => {
    const path = cylindricalOffsetPath([0, 0, 0], [0, 0, 1], [6, 0, 0], 6, "hole")!;
    expect(path.point0Mm).toEqual([6, 0, 0]);
    expect(path.dPointDValue).toEqual([-1, 0, 0]);
    expect(handlePointAt(path, 2)).toEqual([4, 0, 0]);
    // 2 mm at 10 px/mm = 20 px, and the screen direction runs toward the axis.
    const mapping = classifyMapping(path, cameraO(), SCALE_O);
    expect(mapping.kind).toBe("world");
    if (mapping.kind === "world") {
      expect(mapping.g0PxPerMm).toBeCloseTo(10, 12);
      expect(mapping.direction[0]).toBeCloseTo(-1, 12);
    }
  });

  it("a PIN grows outward: σ = +1 puts the derivative along +r̂", () => {
    const path = cylindricalOffsetPath([0, 0, 0], [0, 0, 1], [10, 0, 0], 10, "pin")!;
    expect(path.dPointDValue).toEqual([1, 0, 0]);
    expect(handlePointAt(path, 2)).toEqual([12, 0, 0]);
  });

  it("seats point0 at the CLASSIFIED radius, not at the faceted sample", () => {
    // A polyline vertex sits inside the true cylinder; using it would shorten the
    // construction (the `cylindricalWallPath` rule, same reason).
    const path = cylindricalOffsetPath([0, 0, 0], [0, 0, 1], [9.9, 0, 40], 10, "pin")!;
    expect(path.point0Mm).toEqual([10, 0, 40]);
  });

  it("refuses a degenerate frame or a non-positive radius", () => {
    expect(cylindricalOffsetPath([0, 0, 0], [0, 0, 1], [0, 0, 0], 10, "pin")).toBeNull();
    expect(cylindricalOffsetPath([0, 0, 0], [0, 0, 1], [10, 0, 0], 0, "pin")).toBeNull();
  });
});

describe("planarOffsetPath — H(q) = P + q·n", () => {
  it("rides the face's own outward normal, normalized", () => {
    const path = planarOffsetPath([0, 0, 10], [0, 0, 3])!;
    expect(path.q0Mm).toBe(0);
    expect(path.point0Mm).toEqual([0, 0, 10]);
    expect(path.dPointDValue).toEqual([0, 0, 1]);
    expect(handlePointAt(path, -2)).toEqual([0, 0, 8]);
  });

  it("refuses a degenerate or non-finite frame", () => {
    expect(planarOffsetPath([0, 0, 0], [0, 0, 0])).toBeNull();
    expect(planarOffsetPath([Number.NaN, 0, 0], [0, 0, 1])).toBeNull();
  });
});

describe("orientTowardReference — certifying the Total normal's sign", () => {
  it("flips the opposite plane's normal to point at the selected face", () => {
    expect(orientTowardReference([-1, 0, 0], [0, 0, 0], [4, 0, 20])).toEqual([1, 0, 0]);
    expect(orientTowardReference([1, 0, 0], [0, 0, 0], [4, 0, 20])).toEqual([1, 0, 0]);
  });

  it("REFUSES a sign it cannot certify (reference in the opposite plane)", () => {
    expect(orientTowardReference([1, 0, 0], [0, 0, 0], [0, 5, 20])).toBeNull();
    expect(orientTowardReference([1, 0, 0], [0, 0, 0], [0, 0, 0])).toBeNull();
  });
});

describe("totalThicknessPath — B = P − t0·n, H(T) = B + T·n", () => {
  // §6 "Total plate": selected plane x = 4 mm, opposite x = 0, P = (4,0,20), T = 6.
  const path = (): LinearHandlePath => totalThicknessPath([4, 0, 20], [1, 0, 0], 4)!;

  it("puts B on the opposite plane and H(6) at (6,0,20)", () => {
    expect(path().point0Mm).toEqual([0, 0, 20]);
    expect(handlePointAt(path(), 4)).toEqual([4, 0, 20]);
    expect(handlePointAt(path(), 6)).toEqual([6, 0, 20]);
  });

  it("+20 px from the prepared 4 mm lands on 6 mm", () => {
    expect(dragTo(path(), 4, 20)).toBeCloseTo(6, 12);
  });

  it("refuses without a positive reference thickness", () => {
    expect(totalThicknessPath([4, 0, 20], [1, 0, 0], 0)).toBeNull();
    expect(totalThicknessPath([4, 0, 20], [0, 0, 0], 4)).toBeNull();
  });
});

describe("offset witnesses — what each segment is allowed to claim", () => {
  it("a single face's offset is a TARGET against its frozen reference surface", () => {
    const path = planarOffsetPath([0, 0, 10], [0, 0, 1])!;
    const w = offsetTargetWitness(path, 2, 1);
    expect(w.meaning).toBe("targetConstruction");
    expect(w.fromMm).toEqual([0, 0, 10]);
    expect(w.toMm).toEqual([0, 0, 12]);
    expect(w.label).toContain("construction");
  });

  it("a MULTI-face offset is a shared PARAMETER, never a moving centroid", () => {
    // §5: V3 closures can contain rebuilt blends and fixed supports whose motion
    // roles the frontend does not have, so no per-face claim may be made.
    const w = offsetTargetWitness(planarOffsetPath([0, 0, 10], [0, 0, 1])!, 2, 3);
    expect(w.meaning).toBe("parameterConstruction");
    expect(w.label).toContain("shared by 3 faces");
  });

  it("Radius spans the centreline to the radial target; Ø spans the diameter", () => {
    const radius = radialDimensionPath([0, 0, 0], [0, 0, 1], [10, 0, 0], "Radius")!;
    const rw = radialDimensionWitness(radius, 12, "Radius");
    expect(rw.meaning).toBe("targetConstruction");
    expect(rw.fromMm).toEqual([0, 0, 0]);
    expect(rw.toMm).toEqual([12, 0, 0]);

    const diameter = radialDimensionPath([0, 0, 0], [0, 0, 1], [10, 0, 0], "Diameter")!;
    const dw = radialDimensionWitness(diameter, 16, "Diameter");
    expect(dw.fromMm).toEqual([-8, 0, 0]);
    expect(dw.toMm).toEqual([8, 0, 0]);
    expect(dw.meaning).toBe("targetConstruction");
    expect(dw.label).toContain("construction");
  });

  it("Total draws BOTH the prepared reference t0 and the target T", () => {
    const path = totalThicknessPath([4, 0, 20], [1, 0, 0], 4)!;
    const [reference, target] = totalThicknessWitnesses(path, 6, 4, true);
    // P ↔ B: the 4 mm the prepare MEASURED.
    expect(reference.meaning).toBe("measuredReference");
    expect(reference.fromMm).toEqual([4, 0, 20]);
    expect(reference.toMm).toEqual([0, 0, 20]);
    // B ↔ H(T): the 6 mm the user is asking for, not yet built.
    expect(target.meaning).toBe("targetConstruction");
    expect(target.fromMm).toEqual([0, 0, 20]);
    expect(target.toMm).toEqual([6, 0, 20]);
  });

  it("downgrades the reference segment when the prepare established no t0", () => {
    const path = totalThicknessPath([4, 0, 20], [1, 0, 0], 4)!;
    expect(totalThicknessWitnesses(path, 6, 4, false)[0].meaning).toBe("targetConstruction");
  });
});

describe("offsetAxisFor — the π/4 divergence boundary (§6 'Divergent normals')", () => {
  /** Two normals symmetric about +X at ±`deg` from it. */
  const pair = (deg: number): FaceFrame[] => {
    const r = (deg * Math.PI) / 180;
    return [
      frame([0, 0, 0], [Math.cos(r), Math.sin(r), 0]),
      frame([0, 0, 0], [Math.cos(r), -Math.sin(r), 0]),
    ];
  };

  it("accepts ±44.99° and refuses ±45.01°", () => {
    expect(offsetAxisFor(pair(44.99))).not.toBeNull();
    expect(offsetAxisFor(pair(45.01))).toBeNull();
  });
});
