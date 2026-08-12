/*
 * "Component Parameters" inspector section (Component Library WP-2.4) — the
 * configurator half of spec §294: a selected `PlaceComponent` feature's
 * `role: "free"` params as editable controls, plus the live designation
 * string (spec §2.1's `"ISO 4762 M{thread}x{length}"` template).
 *
 * DATA-DRIVEN, same honesty rule `SketchDimensionsSection`/`HistoryFeatureSection`
 * follow: `canRender` (the library module's own inspector contribution) only
 * knows "a feature is selected" — it cannot see `opType` (platform selection
 * refs don't carry it, ADR-0002's boundary). This component reads
 * `documentStore`'s `FeatureMeta.opType` itself and renders NOTHING, label
 * included, for anything but `PlaceComponent`.
 *
 * Commits go through `client.setComponentParams`, never the generic
 * `applyEditCommand`/`updateScalarParamsCommand` path `featureValueEdit.ts`
 * uses for other ops' inline dimension edits — that generic path would
 * bypass the role=free enforcement `SetComponentParams` (WP-2.3) exists
 * specifically to apply.
 */
import { useCallback, useEffect, useState } from "react";
import { SectionLabel } from "@/ui/SectionLabel";
import { useDocumentStore } from "@/stores/documentStore";
import { useSelectionStore, primarySelection } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { createClient } from "@/ipc/client";
import type { ComponentParamValue, ComponentParameterSpec, LibraryComponent } from "@/ipc/types";

/** Sticky error hint — mirrors `featureValueEdit.ts::errorHint`. */
function errorHint(text: string): void {
  viewportStore.getState().setStatusHint(text, { severity: "error", sticky: true });
}

/**
 * Substitutes every `{key}` in a designation template with `values[key]`
 * (stringified) when present; a placeholder with no known value (a
 * `role: "table"`/`"computed"` key, e.g. `{head_d}` — not resolvable
 * client-side without the dimension table) is left verbatim rather than
 * blanked, so a partially-known designation stays legible instead of reading
 * as broken.
 */
export function formatDesignation(
  template: string,
  values: Readonly<Record<string, ComponentParamValue>>,
): string {
  return template.replace(/\{(\w+)\}/g, (whole, key: string) =>
    key in values ? String(values[key]) : whole,
  );
}

/** The current value for a `role: "free"` param: an override, else the package's own default. */
function currentValue(
  spec: ComponentParameterSpec,
  stored: Record<string, ComponentParamValue>,
  key: string,
): ComponentParamValue | undefined {
  if (key in stored) return stored[key];
  if (spec.key !== undefined) return spec.key;
  return spec.value;
}

interface LoadedRecord {
  componentId: string;
  componentVersion: string;
  params: Record<string, ComponentParamValue>;
}

export function ComponentParametersSection() {
  const sel = useSelectionStore(primarySelection);
  const features = useDocumentStore((s) => s.features);
  const featureId = sel?.kind === "feature" ? sel.id : null;
  const isPlaceComponent =
    featureId !== null && features.find((f) => f.id === featureId)?.opType === "PlaceComponent";

  const [record, setRecord] = useState<LoadedRecord | null>(null);
  const [component, setComponent] = useState<LibraryComponent | null>(null);
  const [pending, setPending] = useState<string | null>(null);

  const load = useCallback(
    async (alive: () => boolean = () => true) => {
      if (!featureId || !isPlaceComponent) return;
      const client = createClient();
      try {
        const params = await client.getOperationParams(featureId);
        const componentId = typeof params.componentId === "string" ? params.componentId : null;
        const componentVersion =
          typeof params.componentVersion === "string" ? params.componentVersion : null;
        const stored =
          params.params && typeof params.params === "object"
            ? (params.params as Record<string, ComponentParamValue>)
            : {};
        if (!componentId || !componentVersion) return;
        const list = await client.listLibraryComponents();
        if (!alive()) return;
        const match =
          list.find((c) => c.id === componentId && c.version === componentVersion) ?? null;
        setRecord({ componentId, componentVersion, params: stored });
        setComponent(match);
      } catch {
        // A record `getOperationParams` cannot serve (stale/legacy) simply has
        // nothing to configure here — the section renders absent, not an error.
        if (!alive()) return;
        setRecord(null);
        setComponent(null);
      }
    },
    [featureId, isPlaceComponent],
  );

  useEffect(() => {
    let alive = true;
    setRecord(null);
    setComponent(null);
    void load(() => alive);
    return () => {
      alive = false;
    };
  }, [load]);

  if (!isPlaceComponent || !record || !component) return null;

  const freeParams = Object.entries(component.parameters).filter(([, s]) => s.role === "free");
  if (freeParams.length === 0) return null;

  const values: Record<string, ComponentParamValue> = {};
  for (const [key, spec] of freeParams) {
    const v = currentValue(spec, record.params, key);
    if (v !== undefined) values[key] = v;
  }
  const designation = component.designation ? formatDesignation(component.designation, values) : null;

  const commit = async (key: string, value: ComponentParamValue) => {
    setPending(key);
    try {
      await createClient().setComponentParams(featureId as string, { [key]: value });
      viewportStore.getState().setStatusHint("Component updated");
      await load();
    } catch (e) {
      errorHint(`Parameter edit failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setPending(null);
    }
  };

  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Component Parameters</SectionLabel>
      {designation !== null ? (
        <div className="mb-2 truncate text-[12.5px] font-medium text-ink-2" title={designation}>
          {designation}
        </div>
      ) : null}
      {freeParams.map(([key, spec]) => {
        const value = values[key];
        const disabled = pending === key;
        return (
          <div
            key={key}
            className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5"
          >
            <span className="flex-1 truncate text-[12.5px] text-ink-2">{key}</span>
            {spec.domain && spec.domain.length > 0 ? (
              <select
                aria-label={`${key} value`}
                className="rounded-sm border border-line bg-surface px-1 text-[12px] text-ink-2 outline-none"
                value={String(value ?? "")}
                disabled={disabled}
                onChange={(e) => void commit(key, e.target.value)}
              >
                {spec.domain.map((d) => (
                  <option key={String(d)} value={String(d)}>
                    {String(d)}
                  </option>
                ))}
              </select>
            ) : (
              <input
                aria-label={`${key} value`}
                type="number"
                className="w-16 rounded-sm border border-line bg-surface px-1 text-right text-[12px] text-ink-2 outline-none"
                defaultValue={typeof value === "number" ? value : ""}
                min={spec.min}
                disabled={disabled}
                onBlur={(e) => {
                  const n = e.target.valueAsNumber;
                  if (!Number.isFinite(n)) return;
                  if (spec.min !== undefined && n < spec.min) {
                    errorHint(`${key} must be at least ${spec.min}`);
                    return;
                  }
                  if (n !== value) void commit(key, n);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") e.currentTarget.blur();
                }}
              />
            )}
          </div>
        );
      })}
    </>
  );
}
