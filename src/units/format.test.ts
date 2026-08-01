import { describe, it, expect } from "vitest";
import {
  AREA_SUFFIX,
  LENGTH_SUFFIX,
  formatArea,
  formatLength,
  parseLength,
} from "./format";

describe("formatLength", () => {
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

describe("formatArea", () => {
  it("renders the value with the mm² unit", () => {
    expect(formatArea(800)).toBe(`800 ${AREA_SUFFIX}`);
    expect(formatArea(800)).toBe("800 mm²");
  });

  it("uses the same trimming as formatLength", () => {
    expect(formatArea(12.3456)).toBe("12.346 mm²");
    expect(formatArea(100)).toBe("100 mm²");
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

  it("bare input is millimetres (the document unit)", () => {
    expect(parseLength("7")).toBe(parseLength(`7${LENGTH_SUFFIX}`));
  });

  it("round-trips a formatted length", () => {
    for (const v of [0, 1, 25, 100, 12.5, 12.345, -12.5]) {
      expect(parseLength(formatLength(v))).toBeCloseTo(v, 9);
    }
  });
});
