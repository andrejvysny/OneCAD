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
import { Button } from "@/ui/Button";
import { ComponentPreview3D } from "./ComponentPreview3D";
import { useDocumentStore } from "@/stores/documentStore";
import { useSelectionStore, primarySelection } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { createClient } from "@/ipc/client";
import { classifyRegen, failureReason } from "@/ipc/regenOutcome";
import { currentParamValue, formatDesignation } from "@/modules/library/designation";
import type {
  ComponentParamValue,
  ComponentUpgrade,
  LibraryComponent,
} from "@/ipc/types";

/** Sticky error hint — mirrors `featureValueEdit.ts::errorHint`. */
function errorHint(text: string): void {
  viewportStore.getState().setStatusHint(text, { severity: "error", sticky: true });
}

/*
 * `formatDesignation` and `currentValue` MOVED to
 * `@/modules/library/designation.ts` (LGU-1 WP-A): the designation is now
 * rendered by the card grids, the detail panel and the armed-tool panel as
 * well, and this section is no longer its only consumer. Re-exported here so
 * the name stays importable from where it has always lived.
 */
export { formatDesignation } from "@/modules/library/designation";

const currentValue = currentParamValue;

/** `"<id>@<version>"` — the catalog key the replace picker selects by. */
function componentKey(id: string, version: string): string {
  return `${id}@${version}`;
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
  const [catalog, setCatalog] = useState<LibraryComponent[]>([]);
  const [upgrade, setUpgrade] = useState<ComponentUpgrade | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [replaceTo, setReplaceTo] = useState<string>("");

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
        // Read-only, and deliberately never acted on automatically: a document
        // must not open differently because a shared library moved underneath
        // it (spec §0 invariant 4 — the Toolbox failure mode).
        const offered = await client.componentUpgradeAvailable(featureId);
        if (!alive()) return;
        const match =
          list.find((c) => c.id === componentId && c.version === componentVersion) ?? null;
        setRecord({ componentId, componentVersion, params: stored });
        setComponent(match);
        setCatalog(list);
        setUpgrade(offered);
      } catch {
        // A record `getOperationParams` cannot serve (stale/legacy) simply has
        // nothing to configure here — the section renders absent, not an error.
        if (!alive()) return;
        setRecord(null);
        setComponent(null);
        setUpgrade(null);
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

  // NOT gated on free params any more (WP-B4): a `document`-source component
  // has none by construction, and replace/upgrade are exactly what its
  // instances need.
  const freeParams = Object.entries(component.parameters).filter(([, s]) => s.role === "free");

  const values: Record<string, ComponentParamValue> = {};
  for (const [key, spec] of freeParams) {
    const v = currentValue(spec, record.params, key);
    if (v !== undefined) values[key] = v;
  }
  const designation = component.designation ? formatDesignation(component.designation, values) : null;

  const commit = async (key: string, value: ComponentParamValue) => {
    setPending(key);
    try {
      const res = await createClient().setComponentParams(featureId as string, { [key]: value });
      // The command applies the edit and then RE-BAKES the component's geometry
      // (WP-F1.3), so resolving is not the same as succeeding: a re-bake that
      // fails comes back as `terminal: "failed"` with no thrown error.
      const reason = failureReason(classifyRegen(res));
      if (reason !== null) {
        errorHint(`Parameter edit failed: ${reason}`);
        return;
      }
      viewportStore.getState().setStatusHint("Component updated");
      await load();
    } catch (e) {
      errorHint(`Parameter edit failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setPending(null);
    }
  };

  /**
   * Swaps this instance to `id@version` IN PLACE (spec §3.3). A recorded mate
   * that the new component cannot carry is DROPPED and named in the hint —
   * the backend never re-binds it to a different attachment, and a silent drop
   * would leave the user to discover later that their part stopped following
   * its target.
   */
  const replaceWith = async (id: string, version: string, what: string) => {
    setPending(what);
    try {
      const report = await createClient().replaceComponent(featureId as string, id, version);
      viewportStore
        .getState()
        .setStatusHint(
          report.droppedMateAttachment
            ? `Replaced — the “${report.droppedMateAttachment}” mate was dropped (the new component has no such attachment).`
            : "Component replaced",
        );
      await load();
    } catch (e) {
      errorHint(`Replace failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setPending(null);
    }
  };

  const alternatives = catalog.filter(
    (c) => componentKey(c.id, c.version) !== componentKey(record.componentId, record.componentVersion),
  );

  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Component Parameters</SectionLabel>
      {designation !== null ? (
        <div className="mb-2 truncate text-[12.5px] font-medium text-ink-2" title={designation}>
          {designation}
        </div>
      ) : null}

      {/* The instance AS CONFIGURED (WP-B6): the preview is keyed by the live
          free params, so changing a size re-renders it — the same round trip
          the placement ghost takes, at rest. */}
      <div className="mb-2">
        <ComponentPreview3D
          componentId={record.componentId}
          componentVersion={record.componentVersion}
          params={values}
          size={196}
        />
      </div>
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
                className="rounded-sm border border-border bg-surface px-1 text-[12px] text-ink-2 outline-none"
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
                className="w-16 rounded-sm border border-border bg-surface px-1 text-right text-[12px] text-ink-2 outline-none"
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

      {upgrade && (
        <div
          data-testid="component-upgrade"
          className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5"
        >
          <span className="flex-1 truncate text-[12.5px] text-ink-2">
            Version {upgrade.latestVersion} available
          </span>
          <Button
            variant="secondary"
            data-testid="component-upgrade-apply"
            disabled={pending !== null}
            onClick={() =>
              void replaceWith(upgrade.componentId, upgrade.latestVersion, "upgrade")
            }
          >
            Upgrade
          </Button>
        </div>
      )}

      {alternatives.length > 0 && (
        <div className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5">
          <select
            aria-label="Replace with"
            data-testid="component-replace-pick"
            className="min-w-0 flex-1 rounded-sm border border-border bg-surface px-1 text-[12px] text-ink-2 outline-none"
            value={replaceTo}
            disabled={pending !== null}
            onChange={(e) => setReplaceTo(e.target.value)}
          >
            <option value="">Replace with…</option>
            {alternatives.map((c) => (
              <option key={componentKey(c.id, c.version)} value={componentKey(c.id, c.version)}>
                {c.name}
              </option>
            ))}
          </select>
          <Button
            variant="secondary"
            data-testid="component-replace-apply"
            disabled={pending !== null || replaceTo === ""}
            onClick={() => {
              const picked = alternatives.find(
                (c) => componentKey(c.id, c.version) === replaceTo,
              );
              if (picked) void replaceWith(picked.id, picked.version, "replace");
            }}
          >
            Replace
          </Button>
        </div>
      )}
    </>
  );
}
