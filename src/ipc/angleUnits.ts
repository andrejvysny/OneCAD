/*
 * angleUnits — the ONE canonical seam between the sketch UI angle domain and the
 * wire angle domain (BUG-2).
 *
 *   UI domain   = DEGREES  (dimensionTool authors °, badges display °).
 *   WIRE domain = RADIANS  (Rust `Constraint::Angle.value` + the worker's PlaneGCS
 *                           `AngleConstraint` are radians; SCHEMA §7.3/§7.4).
 *
 * Every deg↔rad conversion for a sketch dimension MUST go through here so all
 * three marshalling sites (toWireConstraint, the SetDimension diff, and hydration)
 * agree. Only the `Angle` dimension needs converting — every other dimensional
 * kind (Distance/Radius/Diameter/H-/V-Distance) is millimetres in both domains and
 * passes through untouched.
 */
import type { SketchConstraintType } from "./types";

/** Radians→degrees conversion factor — the same canonical `rad` unit factor
 * `onecad-core`'s expr engine uses to fold a `1rad` literal to canonical
 * degrees (`src-tauri/crates/onecad-core/src/expr/dimension.rs`). Exported
 * so `src/ipc/expr/dimension.ts` never re-derives it. */
export const DEGREES_PER_RADIAN = 180 / Math.PI;

export const degToRad = (deg: number): number => (deg * Math.PI) / 180;
export const radToDeg = (rad: number): number => (rad * 180) / Math.PI;

/** A dimensional kind whose value is an angle (only `Angle` today). */
export function isAngular(type: SketchConstraintType): boolean {
  return type === "Angle";
}

/** UI value (degrees for Angle) → wire value (radians for Angle); pass-through else. */
export function toWireDimensionValue(type: SketchConstraintType, uiValue: number): number {
  return isAngular(type) ? degToRad(uiValue) : uiValue;
}

/** Wire value (radians for Angle) → UI value (degrees for Angle); pass-through else. */
export function fromWireDimensionValue(type: SketchConstraintType, wireValue: number): number {
  return isAngular(type) ? radToDeg(wireValue) : wireValue;
}
