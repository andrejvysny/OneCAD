/*
 * The OneCAD icon registry.
 *
 * PROVENANCE. The bulk of this set is the purpose-built two-tone CAD icon
 * family from `OneCAD-CPP/resources/icons/` — 88 masters drawn against a single
 * spec (mirrored here as `./DESIGN.md`: 24x24 grid, 20x20 live area, one shared
 * dimetric projection, "exactly one idea gets the accent"). They arrive via
 * `./cppIcons.generated.ts`; regenerate with `node scripts/gen-icons.mjs`.
 *
 * This SUPERSEDES the earlier contract, under which every path was extracted
 * verbatim from the HTML design prototype. That set was single-path and
 * monochrome, covered none of the 18 constraints, and explicitly refused to
 * supply an `eye-off`. The CPP family is now the app's icon language; glyphs
 * survive outside it only where CPP has no counterpart — see `./authored.ts`,
 * which keeps their original per-entry provenance.
 *
 * ALIASES. Call sites keep the names they already use rather than churning 18
 * files to rename `rect` -> `rectangle`. An alias is one icon under two names,
 * a pattern this registry already had (`pen` === `sketch`, `display` === `cube`).
 */

import { AUTHORED_ICONS } from "./authored";
import { CPP_ICONS } from "./cppIcons.generated";
import type { IconDef } from "./types";

/**
 * Legacy call-site names -> their master in the CPP family.
 *
 * `mirror` is deliberately NOT aliased: it maps 1:1 by name already, and the
 * solid-body mirror is a separate authored glyph (`mirrorBody`) because CPP
 * ships only the flat sketch form. Collapsing the two would hand two distinct
 * toolbar entries the same icon.
 */
const ALIASES = {
  rect: CPP_ICONS.rectangle,
  boolean: CPP_ICONS.booleanUnion,
  cube: CPP_ICONS.body,
  display: CPP_ICONS.body,
  eye: CPP_ICONS.eyeOn,
  x: CPP_ICONS.close,
  datum: CPP_ICONS.plane,
  layers: CPP_ICONS.stack,
  pen: CPP_ICONS.sketch,
  penEdit: CPP_ICONS.edit,
  linearPattern: CPP_ICONS.patternLinear,
  circularPattern: CPP_ICONS.patternCircular,
} as const satisfies Record<string, IconDef>;

/*
 * `as const` is what makes `IconName` the exact union of registered names, but
 * it also narrows every element to its literal shape — so an element authored
 * without `tone` gets a type on which `tone` does not EXIST, and the renderer
 * cannot read the field at all. Infer the key union from the literal, then
 * publish the registry through the widened element type.
 */
const REGISTRY = {
  ...CPP_ICONS,
  ...AUTHORED_ICONS,
  ...ALIASES,
} as const satisfies Record<string, IconDef>;

export type IconName = keyof typeof REGISTRY;

export const ICONS: Record<IconName, IconDef> = REGISTRY;
