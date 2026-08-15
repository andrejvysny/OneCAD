/*
 * The designation — a component's BOM identity string ("ISO 4762 M6x20",
 * "Spur 20T m2").
 *
 * ONE renderer, because the same string has to appear on a card subtitle, a
 * detail-panel header, an armed tool's panel and chip, a quick-params section
 * and (LGU-1 WP-F) a Save-to-Library template preview. It lived inside
 * `ComponentParametersSection` while the inspector was its only consumer;
 * moved here when the second one arrived rather than imported across features.
 */
import type { ComponentParameterSpec, ComponentParamValue } from "@/ipc/types";

/**
 * Substitutes every `{key}` in a designation template with `values[key]`
 * (stringified) when present; a placeholder with no known value (a
 * `role: "table"`/`"computed"` key, e.g. `{head_d}` — not resolvable
 * client-side without the dimension table) is left verbatim rather than
 * blanked, so a partially-known designation stays legible instead of reading
 * as broken. An AUTHOR looking at their own template sees the hole they
 * mistyped for the same reason.
 */
export function formatDesignation(
  template: string,
  values: Readonly<Record<string, ComponentParamValue>>,
): string {
  return template.replace(/\{(\w+)\}/g, (whole, key: string) =>
    key in values ? String(values[key]) : whole,
  );
}

/**
 * The current value for a `role: "free"` param: an override, else the package's
 * own default.
 *
 * `value` wins over `key` because the two mean different things in the two
 * package kinds a free param can come from: a generator's string-domain param
 * spells its default in `key` (`thread = { key = "M6" }`, spec §2.1) while a
 * `document` package's param puts the SOURCE VARIABLE NAME there and the number
 * in `value` (WP-F1.3). Reading `key` first would show a document param's
 * default as the literal variable name. No package declares both.
 */
export function currentParamValue(
  spec: ComponentParameterSpec,
  stored: Readonly<Record<string, ComponentParamValue>>,
  key: string,
): ComponentParamValue | undefined {
  if (key in stored) return stored[key];
  if (spec.value !== undefined) return spec.value;
  return spec.key;
}

/**
 * Every `role: "free"` entry, in table order — the set a user may configure and
 * the only set `setComponentParams` accepts (the backend enforces the same
 * rule; this is the UI half).
 */
export function freeParams(
  parameters: Readonly<Record<string, ComponentParameterSpec>>,
): [string, ComponentParameterSpec][] {
  return Object.entries(parameters).filter(([, spec]) => spec.role === "free");
}

/**
 * Package defaults for every free param, as a starting configuration.
 * Params whose default is unknowable client-side are omitted rather than
 * guessed — `formatDesignation` then leaves their hole visible.
 */
export function defaultParamValues(
  parameters: Readonly<Record<string, ComponentParameterSpec>>,
): Record<string, ComponentParamValue> {
  const out: Record<string, ComponentParamValue> = {};
  for (const [key, spec] of freeParams(parameters)) {
    const value = currentParamValue(spec, {}, key);
    if (value !== undefined) out[key] = value;
  }
  return out;
}
