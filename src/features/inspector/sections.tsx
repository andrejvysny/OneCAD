/*
 * The inspector's SECTIONS, as standalone components.
 *
 * Each one was a block inside `InspectorPanel`'s branch tree; they are platform
 * inspector contributions now (registered in `@/modules/modeling/ui`), which is
 * what lets a non-modeling module put a section here at all.
 *
 * Two consequences of the contribution shape, both deliberate:
 *  - A contribution is mounted with NO PROPS (`ContributionComponent`), so every
 *    section reads its own stores instead of receiving them. Their local state was
 *    already per-section, so nothing was shared to lose.
 *  - Each section renders its OWN `SectionLabel`. The host cannot render titles
 *    for them: the label is what the frozen order contract counts, and a section
 *    that decides at runtime it has nothing to show (no dimensions, no lineage)
 *    must render NOTHING — label included.
 */
import { useCallback, useEffect, useState } from "react";
import { Icon } from "@/icons/Icon";
import { SectionLabel } from "@/ui/SectionLabel";
import {
  useDocumentStore,
  documentStore,
  type FeatureMeta,
} from "@/stores/documentStore";
import { useSelectionStore, primarySelection, selectionStore } from "@/stores/selectionStore";
import { useToolStore, type ModelTool } from "@/stores/toolStore";
import { useSketchStore } from "@/stores/sketchStore";
import { viewportStore } from "@/stores/viewportStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { getModelToolController } from "@/tools/modelTools/modelToolBridge";
import { HistoryList, type HistoryRowActions, type HistoryValueEdit } from "./HistoryList";
import { canBindFeatureValue, canEditFeatureValue, commitFeatureValue } from "./featureValueEdit";
import { ConstraintList, isDimensionalConstraint, visibleConstraints } from "./ConstraintList";
import { suppressFeature, rollToIndex, deleteFeature } from "./historyActions";
import { setBodyColor, setFaceColor } from "@/features/tree/treeActions";
import { CONSTRAINT_PRESENTATION } from "@/features/sketch/constraintCatalog";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import { lengthSuffix } from "@/units/format";
import { createClient } from "@/ipc/client";
import { deleteConstraints } from "@/tools/sketch/sketchService";
import { IMPORT_STEP_OP_TYPE, type Rgba, type SketchConstraint } from "@/ipc/types";
import { palette } from "@/viewport/engine/palette";

// ── shared row behavior ──────────────────────────────────────────────────────

/** Delete one constraint (row × button) — fire-and-forget, mirrors useShortcuts'
 * deleteEntities call: the service re-solves + writes the session back, so the
 * list re-renders off the live sketchStore subscription. */
function deleteConstraint(id: string): void {
  void deleteConstraints(createClient(), [id]);
}

/** Click a history chip → select that feature; double-click → parametric re-edit. */
export function selectFeature(id: string): void {
  selectionStore.getState().set([{ kind: "feature", id }]);
}

/**
 * Route a re-edit on the feature's exact `opType`, falling back to the coarse
 * `kind` for a projection emitted before `opType` existed. The backend folds
 * Chamfer+Shell into kind `fillet` and the pattern/mirror ops into `boolean`
 * (`dto.rs feature_kind`), so routing on `kind` alone sent a Chamfer into the
 * fillet editor and left Shell/patterns/Mirror unreachable on the real lane.
 */
export function editFeature(item: FeatureMeta): void {
  const c = getModelToolController();
  const what = item.opType ?? kindFallback(item.kind);
  switch (what) {
    case "Extrude":
      c?.editExtrudeFeature(item.id);
      return;
    case "Revolve":
      c?.editRevolveFeature(item.id);
      return;
    case "Fillet":
    case "Chamfer":
      void c?.editEdgeOpFeature(item.id, what);
      return;
    case "Shell":
      void c?.editShellFeature(item.id);
      return;
    case "OffsetFace":
      // Distance-only re-edit: the frozen operative closure (and the Total
      // opposite) live in the stored params and are never re-picked here.
      void c?.editOffsetFaceFeature(item.id);
      return;
    case "LinearPattern":
      void c?.editLinearPatternFeature(item.id);
      return;
    case "CircularPattern":
      void c?.editCircularPatternFeature(item.id);
      return;
    case "MirrorBody":
      void c?.editMirrorFeature(item.id);
      return;
    case "TransformBody":
      void c?.editTransformFeature(item.id);
      return;
    case "Hole":
      void c?.editHoleFeature(item.id);
      return;
    case "Boolean":
      // Operation swap only — a Boolean's tool body is CONSUMED, so there is
      // nothing to re-pick (see `editBooleanFeature`).
      void c?.editBooleanFeature(item.id);
      return;
    case IMPORT_STEP_OP_TYPE:
      // An import has NO parametric inputs — its geometry comes from a file, so
      // there is nothing an editor could change. Opening one anyway (the coarse
      // `kind: "boolean"` bucket would have routed it into the Boolean editor)
      // would arm a tool against inputs that do not exist. Say so instead: an
      // unroutable row is otherwise silent, and silence reads as a broken
      // double-click on a row that looks exactly like its editable neighbours.
      viewportStore.getState().setStatusHint("Imported feature — re-import to update");
      return;
    default:
      // Sketch/opaque rows have no parametric editor yet.
      return;
  }
}

/** Best-effort opType for a legacy projection that carries only `kind`. */
function kindFallback(kind: FeatureMeta["kind"]): string {
  switch (kind) {
    case "extrude":
      return "Extrude";
    case "revolve":
      return "Revolve";
    case "fillet":
      return "Fillet";
    case "shell":
      return "Shell";
    case "linearPattern":
      return "LinearPattern";
    case "circularPattern":
      return "CircularPattern";
    case "mirror":
      return "MirrorBody";
    default:
      return "";
  }
}

/**
 * Builds the inline value editor for a history row (H3).
 *
 * Both gate inputs are passed in rather than read off the stores here, so the
 * caller SUBSCRIBES to them: arming a model tool has to re-render the rows into
 * their read-only state, and a `getState()` read would leave the field live.
 *
 * `index` is positional within the FULL timeline, which is what the rollback-cursor
 * gate needs — the feature section renders `documentStore.features` whole, so the
 * row index IS the global index.
 */
function makeValueEdit(
  appliedOps: number,
  modelTool: ModelTool,
): (item: FeatureMeta, index: number) => HistoryValueEdit {
  return (item, index) => ({
    editable: canEditFeatureValue(item, index, appliedOps, modelTool),
    bindable: canBindFeatureValue(item.opType),
    onCommit: (it, value) => {
      void commitFeatureValue(it.id, it.opType ?? "", value);
    },
    // WP-VE.2 — `expr` null CLEARS the binding; a name sets it. Same one-scalar
    // `UpdateOperationParams` patch either way.
    onCommitExpr: (it, expr, value) => {
      void commitFeatureValue(it.id, it.opType ?? "", value, expr);
    },
  });
}

/**
 * Per-row history affordances (edit / suppress / roll-to-here / delete).
 *
 * `suppressed` reads the PROJECTION (`FeatureMeta.suppressed`, sourced from the
 * backend record flag), never a frontend overlay: the retired optimistic overlay
 * started empty on every document open, so a persisted suppression rendered
 * un-dimmed and its toggle computed `!undefined === true` — re-suppressing a
 * feature that was already suppressed, forever. The toggle therefore negates the
 * feature's OWN authoritative flag.
 */
function rowActions(item: FeatureMeta): HistoryRowActions {
  const suppressed = item.suppressed ?? false;
  return {
    suppressed,
    onToggleSuppress: (it) => void suppressFeature(it.id, !(it.suppressed ?? false)),
    onRoll: (it) => {
      // Rollback cursor = applied op count (timeline.rs), so "roll to here" =
      // globalIndex + 1; resolve the GLOBAL index (the list may be a slice).
      const idx = documentStore.getState().features.findIndex((f) => f.id === it.id);
      if (idx >= 0) void rollToIndex(idx);
    },
    onDelete: (it) => void deleteFeature(it.id),
    // H10: the dependent count behind the suppress/delete affordances.
    getDependents: (it) => createClient().featureDependencies(it.id),
  };
}

// ── Appearance ───────────────────────────────────────────────────────────────

export function BodyAppearanceSection() {
  const sel = useSelectionStore(primarySelection);
  const bodyId = sel?.kind === "body" ? sel.id : null;
  const color = useDocumentStore((s) => (bodyId ? s.bodies[bodyId]?.color : undefined));
  const [hex, setHex] = useState(() => rgbaToHex(color));
  useEffect(() => setHex(rgbaToHex(color)), [color]);
  if (!bodyId) return null;

  const alpha = color?.[3] ?? 255;
  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Appearance</SectionLabel>
      <ColorControls
        hex={hex}
        alpha={alpha}
        color={color}
        labelPrefix="Body"
        onColor={(next) => void setBodyColor(bodyId, next)}
        onReset={() => void setBodyColor(bodyId, null)}
      />
    </>
  );
}

export function FaceAppearanceSection() {
  const sel = useSelectionStore(primarySelection);
  const face =
    sel?.kind === "face" && sel.bodyId && sel.elementId
      ? { bodyId: sel.bodyId, elementId: sel.elementId }
      : null;
  const color = useDocumentStore((s) =>
    face ? s.bodies[face.bodyId]?.faceColors?.[face.elementId] : undefined,
  );
  const [hex, setHex] = useState(() => rgbaToHex(color));
  useEffect(() => setHex(rgbaToHex(color)), [color]);
  if (!face) return null;

  const alpha = color?.[3] ?? 255;
  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Appearance</SectionLabel>
      <ColorControls
        hex={hex}
        alpha={alpha}
        color={color}
        labelPrefix="Face"
        onColor={(next) => void setFaceColor(face.bodyId, face.elementId, next)}
        onReset={() => void setFaceColor(face.bodyId, face.elementId, null)}
      />
    </>
  );
}

/** The swatch + opacity pair, identical for a body and a face. */
function ColorControls({
  hex,
  alpha,
  color,
  labelPrefix,
  onColor,
  onReset,
}: {
  hex: string;
  alpha: number;
  color: Rgba | undefined;
  labelPrefix: string;
  onColor: (next: Rgba) => void;
  onReset: () => void;
}) {
  const alphaPct = Math.round((alpha / 255) * 100);
  return (
    <div>
      <div className="flex items-center gap-2">
        <input
          type="color"
          value={hex}
          onChange={(e) => onColor(hexToRgba(e.target.value, alpha / 255))}
          className="h-8 w-8 cursor-pointer rounded border-0 bg-transparent p-0"
          aria-label={`${labelPrefix} color`}
        />
        <input
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={alpha / 255}
          onChange={(e) => {
            const a = Math.round(Number(e.target.value) * 255);
            const base = color ? color.slice(0, 3) : hexToRgb(hex);
            onColor([base[0], base[1], base[2], a]);
          }}
          className="flex-1"
          aria-label={`${labelPrefix} opacity`}
        />
        <span className="w-8 text-right text-[11px] text-ink-5">{alphaPct}%</span>
      </div>
      {color && (
        <button
          type="button"
          onClick={onReset}
          className="mt-2 text-[11px] text-ink-5 hover:text-ink"
        >
          Reset to default
        </button>
      )}
    </div>
  );
}

/**
 * The swatch value for a body/face with NO authored color. `<input type="color">`
 * only speaks `#rrggbb`, so the neutral body-fill TOKEN is resolved through the
 * same palette the viewport uses — a raw hex literal here would both break the
 * hex gate (tokens.css is the sole source of design colors) and freeze the
 * swatch at the light theme's value while the viewport followed the dark one.
 */
function neutralSwatchHex(): string {
  return `#${palette.bodyNeutral().getHexString()}`;
}

function rgbaToHex(c: Rgba | undefined): string {
  if (!c) return neutralSwatchHex();
  const toHex = (n: number) => Math.max(0, Math.min(255, n)).toString(16).padStart(2, "0");
  return `#${toHex(c[0])}${toHex(c[1])}${toHex(c[2])}`;
}

function hexToRgb(hex: string): [number, number, number] {
  const normalized = hex.replace("#", "");
  const bigint = Number.parseInt(normalized.length === 3 ? normalized.split("").map((c) => c + c).join("") : normalized, 16);
  return [(bigint >> 16) & 255, (bigint >> 8) & 255, bigint & 255];
}

function hexToRgba(hex: string, alpha01: number): Rgba {
  const [r, g, b] = hexToRgb(hex);
  return [r, g, b, Math.round(Math.max(0, Math.min(1, alpha01)) * 255)];
}

// ── History ──────────────────────────────────────────────────────────────────

/**
 * Lineage for a SELECTED body/sketch — a slice, not the timeline. A body shows
 * its build-up (Sketch 1 / Extrude / Fillet); anything else shows the extrude
 * that consumed it. Deliberately never passes `appliedOps`: a row index inside a
 * slice is not the global timeline index the rollback cursor is expressed in.
 */
export function HistorySelectionSection() {
  const sel = useSelectionStore(primarySelection);
  const features = useDocumentStore((s) => s.features);
  if (!sel || sel.kind === "feature") return null;

  const items =
    sel.kind === "body"
      ? features.slice(0, 3)
      : features.filter((f) => f.kind === "extrude").slice(0, 1);
  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">History</SectionLabel>
      <HistoryList items={items} onSelect={selectFeature} onEdit={editFeature} />
    </>
  );
}

/**
 * The whole timeline, for a selected feature. The ONLY view that may pass
 * `appliedOps`: it renders `features` WHOLE, so a row index IS the global index
 * the cursor is expressed in.
 *
 * The inline-edit hint ships INSIDE this section rather than as its own: it must
 * stay BELOW the list, and a selection-dependent block above the rows changes the
 * panel height as the selection moves, shifting rows out from under a
 * double-click's second press (it broke the hole re-edit spec exactly so).
 */
export function HistoryFeatureSection() {
  const sel = useSelectionStore(primarySelection);
  const features = useDocumentStore((s) => s.features);
  // Subscriptions, not `getState()` reads: arming a model tool or moving the
  // rollback bar must re-render the rows into their read-only state (H3 gates).
  const modelTool = useToolStore((s) => s.modelTool);
  const appliedOps = useDocumentStore((s) => s.appliedOps);
  if (sel?.kind !== "feature") return null;

  const feat = features.find((f) => f.id === sel.id);
  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">History</SectionLabel>
      <HistoryList
        items={features}
        selectedId={sel.id}
        onSelect={selectFeature}
        onEdit={editFeature}
        rowActions={rowActions}
        valueEdit={makeValueEdit(appliedOps, modelTool)}
        appliedOps={appliedOps}
        onRollToEnd={() => void rollToIndex(features.length - 1)}
      />
      {feat?.primaryValue !== undefined ? (
        <div className="mt-2 text-[12px] leading-normal text-ink-6">
          Click a row's value to change it, or double-click the row for the full editor.
        </div>
      ) : null}
    </>
  );
}

// ── Dependencies ─────────────────────────────────────────────────────────────

/**
 * H10 dependency view — "Depends on" (upstream) / "Used by" (downstream), read
 * straight off `client.featureDependencies`. Ids are mapped to their live
 * `FeatureMeta.label` (falling back to the bare id for a stale/legacy reference
 * `features` no longer carries); each row selects that feature. Either
 * subsection — and the whole block — hides when empty, so a leaf feature with no
 * lineage renders nothing here.
 */
export function FeatureDependenciesSection() {
  const sel = useSelectionStore(primarySelection);
  const features = useDocumentStore((s) => s.features);
  const featureId = sel?.kind === "feature" ? sel.id : null;
  const [deps, setDeps] = useState<{ upstream: string[]; downstream: string[] } | null>(null);

  useEffect(() => {
    if (!featureId) return;
    let alive = true;
    setDeps(null);
    createClient()
      .featureDependencies(featureId)
      .then((d) => {
        if (alive) setDeps(d);
      })
      .catch(() => {
        // An unknown/opaque record (or a mock lane with no tracked lineage for it)
        // has nothing honest to show — an empty section, not an error banner.
        if (alive) setDeps({ upstream: [], downstream: [] });
      });
    return () => {
      alive = false;
    };
  }, [featureId]);

  if (!featureId) return null;
  if (!deps || (deps.upstream.length === 0 && deps.downstream.length === 0)) return null;

  const labelOf = (id: string): string => features.find((f) => f.id === id)?.label ?? id;
  const list = (title: string, ids: string[], testPrefix: string) =>
    ids.length === 0 ? null : (
      <>
        <SectionLabel className="pb-1.5 pt-4">{title}</SectionLabel>
        {ids.map((id) => (
          <div
            key={id}
            role="button"
            tabIndex={0}
            data-testid={`${testPrefix}-${id}`}
            onClick={() => selectFeature(id)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") selectFeature(id);
            }}
            className="mb-1 flex h-7 cursor-pointer items-center rounded-sm bg-chip px-2.5 text-[12.5px] text-ink-2 hover:bg-hover-2"
          >
            {labelOf(id)}
          </div>
        ))}
      </>
    );

  return (
    <>
      {list("Depends on", deps.upstream, "feature-dep-upstream")}
      {list("Used by", deps.downstream, "feature-dep-downstream")}
    </>
  );
}

// ── Dimensions ───────────────────────────────────────────────────────────────

/**
 * H3b — the dimensional constraints of a SELECTED past sketch feature, editable in
 * place. The counterpart to a solid feature's inline value: a sketch's "primary
 * dimension" is not one number, so it gets a section instead of a row chip.
 *
 * Reads through the client on selection (`getOperationParams` → the record's
 * `sketchId`, then `getSketch`) rather than off a store, because a sketch the user
 * has only SELECTED was never entered — nothing has hydrated its geometry. A commit
 * goes straight to `setSketchDimension`; the BACKEND dirties the consuming features,
 * so there is no timeline replay here.
 */
export function SketchDimensionsSection() {
  const sel = useSelectionStore(primarySelection);
  const featureId = sel?.kind === "feature" ? sel.id : null;
  const unit = useSettingsStore((s) => s.displayUnit);
  const [sketchId, setSketchId] = useState<string | null>(null);
  const [dims, setDims] = useState<SketchConstraint[]>([]);

  /** `alive` guards against a selection change landing mid-fetch: two round-trips
   *  in, a resolved older request must not paint another feature's dimensions. */
  const load = useCallback(
    async (alive: () => boolean = () => true) => {
      if (!featureId) return;
      const client = createClient();
      try {
        const params = await client.getOperationParams(featureId);
        // The record's key is `sketchId` (core `SketchOpParams`, serde-renamed).
        const id = typeof params.sketchId === "string" ? params.sketchId : null;
        if (!id) return;
        const session = await client.getSketch(id);
        if (!alive()) return;
        setSketchId(id);
        setDims(
          session.constraints.filter(
            (c) => isDimensionalConstraint(c.type) && typeof c.value === "number",
          ),
        );
      } catch {
        // A sketch record the backend cannot serve params/geometry for (an older
        // document, or the mock's seeded rows) simply has no dimensions to offer —
        // an empty section is the honest answer, not an error banner.
        if (!alive()) return;
        setSketchId(null);
        setDims([]);
      }
    },
    [featureId],
  );

  useEffect(() => {
    let alive = true;
    setSketchId(null);
    setDims([]);
    void load(() => alive);
    return () => {
      alive = false;
    };
  }, [load]);

  if (sketchId === null || dims.length === 0) return null;
  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Dimensions</SectionLabel>
      {dims.map((c) => (
        <div
          key={c.id}
          data-testid={`sketch-dimension-${c.id}`}
          className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5"
        >
          <span className="flex w-4 shrink-0 justify-center text-ink-5">
            <Icon name={CONSTRAINT_PRESENTATION[c.type].icon} size={14} strokeWidth={1.7} />
          </span>
          <span className="flex-1 truncate text-[12.5px] text-ink-2">{c.type}</span>
          <DimensionInput
            value={c.value ?? 0}
            kind={c.type}
            suffix={c.type === "Angle" ? "" : lengthSuffix(unit)}
            onCommit={(v) => {
              void createClient()
                .setSketchDimension(sketchId, c.id, c.type, v)
                // Re-read: the solver may have moved OTHER dimensions to satisfy this one.
                .then(() => load())
                .catch((e: unknown) => {
                  viewportStore.getState().setStatusHint(
                    `Dimension edit failed: ${e instanceof Error ? e.message : String(e)}`,
                    { severity: "error", sticky: true },
                  );
                });
            }}
          />
        </div>
      ))}
    </>
  );
}

// ── Constraints ──────────────────────────────────────────────────────────────

/** Model mode, sketch selected: the prompt. There is no live session to list. */
export function ConstraintsHintSection() {
  return (
    <>
      <SectionLabel className="pb-1.5 pt-3.5">Constraints</SectionLabel>
      <div className="text-[12px] leading-normal text-ink-6">
        Select geometry to constrain.
      </div>
    </>
  );
}

/**
 * Sketch mode: the live list. UNCONDITIONAL — an empty sketch shows the label
 * over "No constraints yet.", so the panel does not reflow when the first
 * constraint lands (frozen in `inspectorContract.ts`).
 */
export function ConstraintsListSection() {
  const constraints = useSketchStore((s) => s.session?.constraints);
  // Only for the locked-row filter — the panel renders no geometry.
  const entities = useSketchStore((s) => s.session?.entities);
  const conflictingIds = useSketchStore((s) => s.conflictingIds);
  const list = constraints ?? [];
  const geometry = entities ?? [];
  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Constraints</SectionLabel>
      {visibleConstraints(list, geometry).length > 0 ? (
        <ConstraintList
          constraints={list}
          entities={geometry}
          onDelete={deleteConstraint}
          conflictingIds={conflictingIds}
        />
      ) : (
        <div className="text-[12px] leading-normal text-ink-6">
          No constraints yet.
        </div>
      )}
    </>
  );
}
