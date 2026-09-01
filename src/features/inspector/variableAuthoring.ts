/*
 * How a typed variable value is routed to the backend, and how a resolved one
 * reads back.
 *
 * ONE text field authors all three forms — `25`, `45deg`, `=w*2` — because
 * making the user pick a mode first would be a mode they have to learn before
 * they can type a number. The routing below is the whole of that decision, kept
 * out of the component so it can be tested as the rule it is.
 */
import type { DocumentVariable } from "@/ipc/types";
import { formatLength, formatUnitless, lengthSuffix, MM_SUFFIX } from "@/units/format";
import type { LengthUnitId } from "@/units/lengthUnits";

/** A bare finite number and nothing else — no unit, no operator, no reference. */
const PLAIN_NUMBER_RE = /^[+-]?(\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/;

/** What a typed value cell means. */
export type VariableAuthoring =
  | { kind: "literal"; value: number }
  | { kind: "expression"; text: string }
  | { kind: "empty" };

/**
 * Reads a typed value cell.
 *
 * A bare number goes to `upsertVariable` as a LITERAL — which is also the only
 * way back from `=w*2` to a plain number, since that call clears any stored
 * expression. Everything else (a leading `=`, a unit suffix, arithmetic, a
 * reference) goes to `upsertVariableExpr`, where the backend is the one that
 * decides whether it resolves. The leading `=` is UI syntax and is stripped
 * before it goes on the wire.
 */
export function readVariableInput(raw: string): VariableAuthoring {
  const text = raw.trim();
  if (text === "") return { kind: "empty" };
  if (text.startsWith("=")) {
    const rest = text.slice(1).trim();
    return rest === "" ? { kind: "empty" } : { kind: "expression", text: rest };
  }
  if (PLAIN_NUMBER_RE.test(text)) {
    const value = Number.parseFloat(text);
    return Number.isFinite(value) ? { kind: "literal", value } : { kind: "empty" };
  }
  return { kind: "expression", text };
}

/** What the value cell is SEEDED with: the expression when there is one (that
 *  is what the user authored), else the stored number. */
export function variableInputText(v: DocumentVariable): string {
  return v.expr !== undefined ? `=${v.expr}` : formatUnitless(v.value);
}

/**
 * What a variable RESOLVES to, with the unit its own dimension implies.
 *
 * A `"scalar"` variable is dimensionless on purpose — `depth = 10` drives a
 * length field as 10 mm and an angle field as 10°, so naming a unit here would
 * claim one it does not have. A variable that WANTS a unit says so in its
 * expression (`10mm`), and then this shows it.
 */
export function formatResolvedVariable(v: DocumentVariable, unit: LengthUnitId): string {
  switch (v.dimension) {
    case "angle":
      return `${formatUnitless(v.resolvedValue)}°`;
    case "length":
      return unit === MM_SUFFIX
        ? `${formatUnitless(v.resolvedValue)} ${MM_SUFFIX}`
        : `${formatLength(v.resolvedValue, unit)} ${lengthSuffix(unit)}`;
    default:
      return formatUnitless(v.resolvedValue);
  }
}
