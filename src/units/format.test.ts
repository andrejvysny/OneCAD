import { describe, it, expect, afterEach } from "vitest";
import { settingsStore } from "@/stores/settingsStore";
import { DEFAULT_LENGTH_UNIT, LENGTH_UNIT_ORDER, type LengthUnitId } from "./lengthUnits";
import {
  AREA_SUFFIX,
  LENGTH_SUFFIX,
  MM_SUFFIX,
  areaSuffix,
  currentLengthUnit,
  displayToMm,
  formatArea,
  formatCursorAxis,
  formatLength,
  formatLengthWithUnit,
  formatMillimetres,
  formatUnitless,
  isLengthSuffix,
  lengthSuffix,
  mmToDisplay,
  parseLength,
} from "./format";

/** Drive the real preference — the seam reads the store, not a parameter. */
function setUnit(unit: LengthUnitId): void {
  settingsStore.getState().setDisplayUnit(unit);
}

afterEach(() => {
  setUnit(DEFAULT_LENGTH_UNIT);
  localStorage.clear();
});

describe("formatLength (millimetres — the frozen default contract)", () => {
  const cases: [number, string][] = [
    [12.5, "12.5"],
    [12.345, "12.345"],
    // The regression the capture-group anchoring exists for: a naive
    // `.replace(/\.?0+$/,"")` turns "100" into "1".
    [100, "100"],
    [1000, "1000"],
    [12.3456, "12.346"],
    [0, "0"],
    [100.5, "100.5"],
    [-12.5, "-12.5"],
    [-100, "-100"],
    [-0, "0"],
    [-12.345, "-12.345"],
    [0.0001, "0"],
    [25, "25"],
  ];
  for (const [input, expected] of cases) {
    it(`formats ${input} as "${expected}"`, () => {
      expect(formatLength(input)).toBe(expected);
    });
  }

  it("passes non-finite values through rather than inventing a number", () => {
    expect(formatLength(Number.NaN)).toBe("NaN");
    expect(formatLength(Number.POSITIVE_INFINITY)).toBe("Infinity");
  });
});

describe("formatLength (display unit)", () => {
  it("renders the SAME stored millimetres in whichever unit is selected", () => {
    expect(formatLength(25.4, "mm")).toBe("25.4");
    expect(formatLength(25.4, "cm")).toBe("2.54");
    expect(formatLength(25.4, "m")).toBe("0.0254");
    expect(formatLength(25.4, "in")).toBe("1");
  });

  it("follows the store when no unit is passed", () => {
    setUnit("in");
    expect(currentLengthUnit()).toBe("in");
    expect(formatLength(25.4)).toBe("1");
    setUnit("cm");
    expect(formatLength(25.4)).toBe("2.54");
  });

  it("composes the unit symbol on demand", () => {
    expect(formatLengthWithUnit(25.4, "in")).toBe("1 in");
    expect(formatLengthWithUnit(1000, "m")).toBe("1 m");
    expect(formatLengthWithUnit(100, "mm")).toBe("100 mm");
  });

  it("trims trailing zeros in every unit", () => {
    expect(formatLength(10, "cm")).toBe("1");
    expect(formatLength(2000, "m")).toBe("2");
    expect(formatLength(50.8, "in")).toBe("2");
  });
});

describe("formatMillimetres / formatUnitless (preference-independent)", () => {
  it("ignores the display unit entirely", () => {
    for (const unit of LENGTH_UNIT_ORDER) {
      setUnit(unit);
      expect(formatMillimetres(25.4)).toBe("25.4");
      expect(formatUnitless(45)).toBe("45");
    }
  });

  it("MM_SUFFIX is the document unit and never moves", () => {
    setUnit("in");
    expect(MM_SUFFIX).toBe("mm");
  });
});

describe("mmToDisplay / displayToMm", () => {
  it("round-trips exactly in every unit", () => {
    for (const unit of LENGTH_UNIT_ORDER) {
      for (const mm of [0, 1, 25.4, 100, 1000, -12.5]) {
        expect(displayToMm(mmToDisplay(mm, unit), unit)).toBeCloseTo(mm, 9);
      }
    }
  });

  it("the inch factor is EXACTLY 25.4 in both directions", () => {
    expect(displayToMm(1, "in")).toBe(25.4);
    expect(displayToMm(2, "in")).toBe(50.8);
    expect(mmToDisplay(25.4, "in")).toBe(1);
  });
});

describe("lengthSuffix / live suffix bindings", () => {
  it("names the current unit", () => {
    expect(lengthSuffix("mm")).toBe("mm");
    expect(lengthSuffix("in")).toBe("in");
    expect(areaSuffix("in")).toBe("in²");
  });

  it("LENGTH_SUFFIX / AREA_SUFFIX track the store (live ES bindings)", async () => {
    expect(LENGTH_SUFFIX).toBe("mm");
    setUnit("in");
    // Re-import: a live binding is only observable through a fresh read of the
    // module namespace in a test, but every real caller reads it at call time.
    const fresh = await import("./format");
    expect(fresh.LENGTH_SUFFIX).toBe("in");
    expect(fresh.AREA_SUFFIX).toBe("in²");
    setUnit("mm");
    expect(fresh.LENGTH_SUFFIX).toBe("mm");
    expect(fresh.AREA_SUFFIX).toBe("mm²");
  });

  it("isLengthSuffix recognises exactly the real units", () => {
    for (const unit of LENGTH_UNIT_ORDER) expect(isLengthSuffix(unit)).toBe(true);
    expect(isLengthSuffix("MM")).toBe(true);
    expect(isLengthSuffix("°")).toBe(false);
    expect(isLengthSuffix("")).toBe(false);
    expect(isLengthSuffix("furlongs")).toBe(false);
  });
});

describe("formatArea", () => {
  it("renders the value with the mm² unit", () => {
    expect(formatArea(800)).toBe(`800 ${AREA_SUFFIX}`);
    expect(formatArea(800)).toBe("800 mm²");
  });

  it("uses the same trimming as formatLength", () => {
    expect(formatArea(12.3456)).toBe("12.346 mm²");
    expect(formatArea(100)).toBe("100 mm²");
  });

  it("converts by the SQUARE of the factor — an area is not a length", () => {
    // 1 in² = 645.16 mm². Dividing once by 25.4 would report 25.4 in², which is
    // the exact bug the squared factor exists to prevent.
    expect(formatArea(645.16, "in")).toBe("1 in²");
    expect(formatArea(100, "cm")).toBe("1 cm²");
    expect(formatArea(1_000_000, "m")).toBe("1 m²");
  });
});

describe("formatCursorAxis", () => {
  it("pads to a stable column width and does NOT trim (no jitter)", () => {
    expect(formatCursorAxis(0, "mm")).toBe("  0.00");
    expect(formatCursorAxis(273, "mm")).toBe("273.00");
    expect(formatCursorAxis(25.4, "in")).toBe(" 1.000");
  });
});

describe("parseLength", () => {
  const ok: [string, number][] = [
    ["25", 25],
    ["25mm", 25],
    ["25 mm", 25],
    ["  25   mm  ", 25],
    ["25MM", 25],
    ["2.5 cm", 25],
    ["2.5cm", 25],
    ["0.5m", 500],
    ["0.5 M", 500],
    ["1 in", 25.4],
    ["1in", 25.4],
    ["1 IN", 25.4],
    ["-12.5", -12.5],
    ["-1 cm", -10],
    ["+3", 3],
    [".5", 0.5],
    ["0", 0],
    ["1e3", 1000],
    ["1e3 mm", 1000],
  ];
  for (const [input, expected] of ok) {
    it(`parses ${JSON.stringify(input)} to ${expected} mm`, () => {
      expect(parseLength(input)).toBeCloseTo(expected, 9);
    });
  }

  const bad = ["", "   ", "abc", "mm", "25abc", "25 furlongs", "1em", "2.5.5", "1/2", "--3", "25 mm mm"];
  for (const input of bad) {
    it(`rejects ${JSON.stringify(input)} with null`, () => {
      expect(parseLength(input)).toBeNull();
    });
  }

  it("bare input is millimetres while millimetres are displayed", () => {
    expect(parseLength("7")).toBe(parseLength(`7${MM_SUFFIX}`));
  });

  it("BARE input is read in the DISPLAY unit — typing 2 while showing inches is 2 in", () => {
    setUnit("in");
    expect(parseLength("2")).toBeCloseTo(50.8, 9);
    expect(parseLength("1")).toBe(25.4);
    setUnit("cm");
    expect(parseLength("2")).toBeCloseTo(20, 9);
    setUnit("m");
    expect(parseLength("2")).toBeCloseTo(2000, 9);
  });

  it("an explicit SUFFIX always overrides the display unit", () => {
    setUnit("in");
    expect(parseLength("2mm")).toBeCloseTo(2, 9);
    expect(parseLength("2 cm")).toBeCloseTo(20, 9);
    expect(parseLength("2 in")).toBeCloseTo(50.8, 9);
  });

  it("rejects an unknown unit whatever the display preference", () => {
    setUnit("in");
    expect(parseLength("25 furlongs")).toBeNull();
    expect(parseLength("25abc")).toBeNull();
  });

  /*
   * DISPLAY-STABLE round-trip, which is the property that actually matters:
   * re-parsing what the field shows must render back to the SAME string. That
   * is what makes DimensionInput's no-op gate hold — an untouched blur can
   * never look like a new value. Exact mm equality is NOT available at every
   * unit (0.0394 in is 1.00076 mm, four decimals of an inch being ~2.5 µm), and
   * demanding it would be demanding infinite display precision.
   */
  it("re-parsing a formatted length re-renders identically in every unit", () => {
    for (const unit of LENGTH_UNIT_ORDER) {
      setUnit(unit);
      for (const mm of [0, 1, 25.4, 100, 12.5, -12.5, 50.8, 12.345]) {
        const shown = formatLength(mm);
        expect(formatLength(parseLength(shown)!)).toBe(shown);
      }
    }
  });

  it("round-trips EXACTLY for values the unit can represent", () => {
    for (const [unit, mm] of [
      ["mm", 12.345],
      ["cm", 100],
      ["m", 1000],
      ["in", 25.4],
      ["in", 50.8],
    ] as [LengthUnitId, number][]) {
      setUnit(unit);
      expect(parseLength(formatLength(mm))).toBeCloseTo(mm, 9);
    }
  });
});

/*
 * MARSHALLING INDEPENDENCE — the invariant the whole feature rests on.
 *
 * Whatever the display preference, the number that leaves this module toward
 * the document/wire is millimetres. This is the `angleUnits.ts` bug class
 * (three marshalling sites that had to agree); pinning it here means a future
 * "helpful" conversion inside the seam fails loudly.
 */
describe("wire/document values are display-unit INDEPENDENT", () => {
  it("the same TYPED text with a suffix yields the same mm in every unit", () => {
    const seen = new Set<number | null>();
    for (const unit of LENGTH_UNIT_ORDER) {
      setUnit(unit);
      seen.add(parseLength("25 mm"));
    }
    expect([...seen]).toEqual([25]);
  });

  it("mm → display → mm is lossless enough to re-commit unchanged", () => {
    for (const unit of LENGTH_UNIT_ORDER) {
      setUnit(unit);
      const mm = 25.4;
      expect(displayToMm(mmToDisplay(mm))).toBeCloseTo(mm, 9);
    }
  });

  it("a valueText producer stays mm no matter what is displayed", () => {
    for (const unit of LENGTH_UNIT_ORDER) {
      setUnit(unit);
      expect(`${formatMillimetres(2)} ${MM_SUFFIX}`).toBe("2 mm");
    }
  });
});
