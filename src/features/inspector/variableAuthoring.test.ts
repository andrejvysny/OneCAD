/*
 * The routing rule behind the Variables section's one value field, and how a
 * resolved variable reads back.
 *
 * The rule matters because the two write commands are NOT interchangeable:
 * `upsertVariable` clears any stored expression, so sending a number down the
 * expression path (or vice versa) would either strand a binding or record a
 * literal as an expression.
 */
import { describe, expect, it } from "vitest";
import type { DocumentVariable } from "@/ipc/types";
import { formatResolvedVariable, readVariableInput, variableInputText } from "./variableAuthoring";

const variable = (over: Partial<DocumentVariable>): DocumentVariable => ({
  id: "v1",
  name: "w",
  value: 10,
  resolvedValue: 10,
  dimension: "scalar",
  ...over,
});

describe("readVariableInput", () => {
  it("reads a bare number as a LITERAL — the only way back from an expression", () => {
    expect(readVariableInput("25")).toEqual({ kind: "literal", value: 25 });
    expect(readVariableInput(" -3.5 ")).toEqual({ kind: "literal", value: -3.5 });
    expect(readVariableInput("1e3")).toEqual({ kind: "literal", value: 1000 });
  });

  it("strips the `=`, which is UI syntax and not part of the grammar", () => {
    expect(readVariableInput("=w*2")).toEqual({ kind: "expression", text: "w*2" });
    expect(readVariableInput("  =  w * 2 ")).toEqual({ kind: "expression", text: "w * 2" });
  });

  /** A unit suffix or arithmetic IS an expression even with no `=`: the `=` is
   *  a convenience for referencing a variable, not a mode switch. */
  it("routes a suffixed or computed value through the expression path", () => {
    expect(readVariableInput("45deg")).toEqual({ kind: "expression", text: "45deg" });
    expect(readVariableInput("2 * 3")).toEqual({ kind: "expression", text: "2 * 3" });
    // Nonsense goes there too — deciding it is nonsense is the backend's job,
    // and a second grammar here would drift from the evaluator's.
    expect(readVariableInput("wat?")).toEqual({ kind: "expression", text: "wat?" });
  });

  it("treats blank text (and a bare `=`) as nothing to commit", () => {
    for (const empty of ["", "   ", "=", " = "]) {
      expect(readVariableInput(empty)).toEqual({ kind: "empty" });
    }
  });
});

describe("variableInputText", () => {
  /** The field is seeded with what was AUTHORED, so an expression stays
   *  editable as an expression rather than collapsing to its number. */
  it("shows the expression when there is one, else the stored number", () => {
    expect(variableInputText(variable({ expr: "w*2", value: 20 }))).toBe("=w*2");
    expect(variableInputText(variable({ value: 12.5 }))).toBe("12.5");
  });
});

describe("formatResolvedVariable", () => {
  it("names the unit a variable's own dimension implies", () => {
    expect(formatResolvedVariable(variable({ dimension: "length", resolvedValue: 45 }), "mm")).toBe(
      "45 mm",
    );
    expect(formatResolvedVariable(variable({ dimension: "angle", resolvedValue: 45 }), "mm")).toBe(
      "45°",
    );
  });

  it("follows the display unit for a length", () => {
    expect(formatResolvedVariable(variable({ dimension: "length", resolvedValue: 50.8 }), "in")).toBe(
      "2 in",
    );
  });

  /** A dimensionless variable names NO unit: `depth = 10` drives a length field
   *  as 10 mm and an angle field as 10°, so claiming one would be a lie. */
  it("leaves a scalar bare, in every display unit", () => {
    expect(formatResolvedVariable(variable({ dimension: "scalar", resolvedValue: 10 }), "in")).toBe(
      "10",
    );
  });
});
