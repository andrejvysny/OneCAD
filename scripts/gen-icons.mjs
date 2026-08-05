#!/usr/bin/env node
/*
 * Regenerate src/icons/cppIcons.generated.ts from the OneCAD-CPP icon masters.
 *
 *   node scripts/gen-icons.mjs
 *   ONECAD_CPP_DIR=/path/to/OneCAD-CPP node scripts/gen-icons.mjs
 *
 * OneCAD-CPP is a read-only oracle: this script never writes there. Its output
 * is committed, so neither a build nor CI ever needs that checkout present.
 *
 * The masters are deliberately minimal (see resources/icons/DESIGN.md §7: "no
 * <?xml?> prolog, no editor cruft, no inline style= blocks"), which is why a
 * regex reader is sufficient and no XML parser dependency is pulled in. The
 * asserts below are what keep that assumption honest — if the source set ever
 * grows an element or attribute this does not model, generation fails loudly
 * rather than silently dropping part of a glyph.
 */

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CPP = process.env.ONECAD_CPP_DIR ?? resolve(REPO, "..", "OneCAD-CPP");
const SRC_DIR = join(CPP, "resources", "icons");
const OUT = join(REPO, "src", "icons", "cppIcons.generated.ts");

/** The two-tone ACCENT sentinel every master is authored against. */
const ACCENT = "#2d7ff9";
/**
 * The family's PRIMARY structural weight (DESIGN.md §2). Every emitted `sw` is a
 * ratio against this, NOT against the icon's own root width.
 *
 * That distinction matters: 17 masters (the solid-body group, the boolean ops,
 * the view cubes) are deliberately authored at a lighter root of 1.75 because
 * they carry more internal detail and mush at full weight. Normalising each icon
 * against its own root would flatten all of them back to the caller's weight and
 * silently discard that judgement; normalising against the shared base keeps a
 * `patternLinear` lighter than an `extrude` while still letting each call site's
 * `strokeWidth` scale the whole set.
 */
const BASE_WIDTH = 2;
/** Legacy white fill; DESIGN.md §3 collapses it into the primary tone. */
const LEGACY_WHITE = "#ffffff";

const KNOWN_ELEMENTS = new Set(["svg", "g", "path"]);
const PATH_ATTRS = new Set([
  "d",
  "fill",
  "stroke",
  "stroke-width",
  "stroke-dasharray",
  "stroke-linecap",
  "stroke-linejoin",
  "transform",
]);

function fail(msg) {
  console.error(`gen-icons: ${msg}`);
  process.exit(1);
}

/** `ic_constraint_point_on_curve.svg` -> `constraintPointOnCurve`. */
function toKey(file) {
  return basename(file, ".svg")
    .replace(/^ic_/, "")
    .replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());
}

function parseAttrs(tag) {
  const attrs = {};
  for (const m of tag.matchAll(/([a-zA-Z-]+)\s*=\s*"([^"]*)"/g)) {
    attrs[m[1]] = m[2];
  }
  return attrs;
}

/**
 * Normalize a color attribute to one of: "primary" | "accent" | "none".
 * Anything else is an unmodelled color and must not be silently emitted.
 */
function tone(value, where) {
  if (value === undefined) return undefined;
  const v = value.trim().toLowerCase();
  if (v === "none") return "none";
  if (v === "currentcolor" || v === LEGACY_WHITE) return "primary";
  if (v === ACCENT) return "accent";
  fail(`${where}: unexpected color ${JSON.stringify(value)}`);
}

function parseIcon(file) {
  const text = readFileSync(join(SRC_DIR, file), "utf8");
  const where = file;

  for (const m of text.matchAll(/<([a-zA-Z]+)/g)) {
    if (!KNOWN_ELEMENTS.has(m[1])) fail(`${where}: unsupported <${m[1]}>`);
  }

  const rootTag = text.match(/<svg\b[^>]*>/);
  if (!rootTag) fail(`${where}: no <svg> root`);
  const root = parseAttrs(rootTag[0]);

  if (root.viewBox !== "0 0 24 24") {
    fail(`${where}: viewBox ${root.viewBox}, expected "0 0 24 24"`);
  }
  // Absent is legal for an all-filled glyph (ic_overflow), which strokes nothing.
  const rootWidth =
    root["stroke-width"] === undefined ? null : Number(root["stroke-width"]);
  if (rootWidth !== null && (!Number.isFinite(rootWidth) || rootWidth <= 0)) {
    fail(`${where}: root stroke-width ${root["stroke-width"]}`);
  }

  // Paint attributes cascade: <svg> root -> <g> -> <path>. Neither wrapper
  // carries geometry, so fold both down onto each path rather than modelling a
  // tree. <g> only ever appears in ic_orbit / ic_pan.
  const inherited = {};
  for (const key of ["fill", "stroke", "stroke-width"]) {
    if (root[key] !== undefined) inherited[key] = root[key];
  }

  const elements = [];
  let group = null;

  for (const m of text.matchAll(/<(g|path)\b([^>]*?)\/?>|<\/g>/g)) {
    if (m[0] === "</g>") {
      group = null;
      continue;
    }
    const attrs = parseAttrs(m[2] ?? "");
    if (m[1] === "g") {
      if (attrs.d) fail(`${where}: <g> carries geometry`);
      group = attrs;
      continue;
    }

    const own = { ...inherited, ...(group ?? {}), ...attrs };
    for (const key of Object.keys(own)) {
      if (!PATH_ATTRS.has(key)) fail(`${where}: unsupported attribute ${key}`);
    }
    if (!own.d) fail(`${where}: <path> without d`);

    const fill = tone(own.fill, where) ?? "none";
    const stroke = tone(own.stroke, where) ?? "primary";

    // The masters express "this element is filled" as fill=<color> stroke=none.
    // Anything outside that pairing is a shape this renderer cannot reproduce.
    const filled = stroke === "none";
    if (filled && fill === "none") fail(`${where}: element paints nothing`);
    if (!filled && fill !== "none") {
      fail(`${where}: element both stroked and filled — not modelled`);
    }

    const el = { d: own.d.replace(/\s+/g, " ").trim() };
    const paintTone = filled ? fill : stroke;
    if (paintTone === "accent") el.tone = "accent";
    if (filled) el.filled = true;

    if (!filled && own["stroke-width"] !== undefined) {
      const w = Number(own["stroke-width"]);
      if (!Number.isFinite(w)) fail(`${where}: stroke-width ${own["stroke-width"]}`);
      if (rootWidth === null) fail(`${where}: stroked element with no root stroke-width`);
      // Round to kill float noise (1.6/2 = 0.8000000000000001) so reruns of the
      // generator produce a byte-identical file.
      const ratio = Number((w / BASE_WIDTH).toFixed(4));
      if (ratio !== 1) el.sw = ratio;
    }
    if (own["stroke-dasharray"]) el.dash = own["stroke-dasharray"];
    if (own.transform) el.transform = own.transform;

    elements.push(el);
  }

  if (group !== null) fail(`${where}: unclosed <g>`);
  if (elements.length === 0) fail(`${where}: no drawable elements`);
  return elements;
}

function render(el) {
  const parts = [`d: ${JSON.stringify(el.d)}`];
  if (el.tone) parts.push(`tone: "accent"`);
  if (el.filled) parts.push("filled: true");
  if (el.sw !== undefined) parts.push(`sw: ${el.sw}`);
  if (el.dash) parts.push(`dash: ${JSON.stringify(el.dash)}`);
  if (el.transform) parts.push(`transform: ${JSON.stringify(el.transform)}`);
  return `{ ${parts.join(", ")} }`;
}

const files = readdirSync(SRC_DIR)
  .filter((f) => f.endsWith(".svg"))
  .sort();
if (files.length === 0) fail(`no SVGs under ${SRC_DIR}`);

const icons = new Map();
for (const file of files) {
  const key = toKey(file);
  if (icons.has(key)) fail(`duplicate key ${key} from ${file}`);
  icons.set(key, parseIcon(file));
}

const body = [...icons]
  .map(([key, els]) => {
    const inner = els.map((e) => `    ${render(e)},`).join("\n");
    return `  ${key}: [\n${inner}\n  ],`;
  })
  .join("\n");

writeFileSync(
  OUT,
  `/*
 * GENERATED FILE — DO NOT EDIT.
 *
 * Source of truth: OneCAD-CPP \`resources/icons/*.svg\` (${files.length} masters) and
 * the spec they are drawn against, mirrored here as \`src/icons/DESIGN.md\`.
 * Regenerate with:
 *
 *   node scripts/gen-icons.mjs
 *
 * Transformations applied by the generator:
 *   - \`ic_foo_bar.svg\` -> key \`fooBar\`
 *   - the masters' literal accent color -> \`tone: "accent"\`, which the renderer
 *     paints with \`var(--color-icon-accent)\`. No color literal survives into
 *     this file, which is what keeps the hex gate clean.
 *   - \`fill=<color> stroke="none"\` -> \`filled: true\`
 *   - \`<g>\` attribute wrappers flattened onto their children
 *   - per-element \`stroke-width\` -> \`sw\`, a RATIO of the master's root width,
 *     so each call site's own \`strokeWidth\` stays the master knob
 */

import type { IconDef } from "./types";

export const CPP_ICONS = {
${body}
} as const satisfies Record<string, IconDef>;

export type CppIconName = keyof typeof CPP_ICONS;
`,
  "utf8",
);

const paths = [...icons.values()].reduce((n, els) => n + els.length, 0);
console.log(`gen-icons: ${icons.size} icons, ${paths} elements -> ${OUT}`);
