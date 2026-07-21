import { describe, it, expect } from "vitest";
import { formatDimensionValue } from "./dimensionFormat";

describe("formatDimensionValue", () => {
  it("keeps a clean one-decimal value", () => {
    expect(formatDimensionValue(12.5)).toBe("12.5");
  });

  it("keeps a full 3-decimal value", () => {
    expect(formatDimensionValue(12.345)).toBe("12.345");
  });

  it("does not truncate a whole number's trailing zeros", () => {
    expect(formatDimensionValue(100)).toBe("100");
  });

  it("rounds beyond 3 decimals", () => {
    expect(formatDimensionValue(12.3456)).toBe("12.346");
  });

  it("renders zero as \"0\"", () => {
    expect(formatDimensionValue(0)).toBe("0");
  });

  it("trims a trailing zero after rounding introduces one", () => {
    expect(formatDimensionValue(100.5)).toBe("100.5");
  });

  it("handles a negative value with trimming", () => {
    expect(formatDimensionValue(-12.5)).toBe("-12.5");
  });

  it("handles a negative whole number", () => {
    expect(formatDimensionValue(-100)).toBe("-100");
  });

  it("handles negative zero as \"0\"", () => {
    expect(formatDimensionValue(-0)).toBe("0");
  });

  it("handles a negative value with full 3 decimals", () => {
    expect(formatDimensionValue(-12.345)).toBe("-12.345");
  });
});
