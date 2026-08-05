/*
 * Snap-radius registry (SP-5.7) — the on-screen reach of every point-like pick,
 * as a user preference.
 *
 * A radius is DATA, not a magic number, for the same reason render modes, length
 * units and themes are (see `viewport/engine/renderModes.ts`): the picker
 * control, the persisted setting, the snap engine and the controller's hit
 * tolerances all read this table, so they cannot disagree about which ids exist,
 * what they are called, or how many pixels one of them is worth.
 *
 * PURE and geometry-free ON PURPOSE. `settingsStore` imports this module, and
 * `snapEngine` pulls in entity types + ellipse math — putting the radius table
 * there would couple the persisted-prefs module to the whole geometry layer just
 * to read a number.
 */

/** The offered snap reaches. `m` is the historical (and default) 8px. */
export type SnapRadiusId = "s" | "m" | "l";

/**
 * Reach in SCREEN pixels — the pixel-space analogue of the C++ 2mm sketch-coord
 * radius (see `snapEngine`'s ladder docstring). `m` is exactly `SNAP_PX`, so a
 * persisted blob migrated onto this setting snaps exactly as it did before.
 */
export const SNAP_RADIUS_PX: Record<SnapRadiusId, number> = { s: 5, m: 8, l: 12 };

/** Control order for the radius toggle (also the canonical id enumeration). */
export const SNAP_RADIUS_ORDER: readonly SnapRadiusId[] = ["s", "m", "l"];

/** Segment labels. Here rather than in the popover so the control and the
 *  setting can never name the same radius two different things. */
export const SNAP_RADIUS_LABEL: Record<SnapRadiusId, string> = { s: "S", m: "M", l: "L" };

/**
 * 8px. The default is the value every build before this setting existed used, so
 * turning the knob into a preference changed nobody's snapping until they move it.
 */
export const DEFAULT_SNAP_RADIUS: SnapRadiusId = "m";

/**
 * Narrow an untrusted value (persisted setting, hand-edited localStorage, a
 * rolled-back build that wrote an id this registry no longer has) to a known
 * radius, falling back to the default rather than throwing — an unknown id must
 * never leave the sketch unable to snap at all.
 */
export function coerceSnapRadius(v: unknown): SnapRadiusId {
  if (typeof v === "string" && Object.prototype.hasOwnProperty.call(SNAP_RADIUS_PX, v)) {
    return v as SnapRadiusId;
  }
  return DEFAULT_SNAP_RADIUS;
}
