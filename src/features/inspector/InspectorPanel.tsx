import { useCallback, useEffect, useState } from "react";
import { Icon } from "@/icons/Icon";
import { SectionLabel } from "@/ui/SectionLabel";
import {
  useDocumentStore,
  type BodyMeta,
  type FeatureMeta,
  type SketchMeta,
} from "@/stores/documentStore";
import {
  useSelectionStore,
  primarySelection,
  selectionStore,
  type EntityRef,
} from "@/stores/selectionStore";
import { useToolStore, type ModelTool } from "@/stores/toolStore";
import { useViewportStore, viewportStore } from "@/stores/viewportStore";
import { useSketchStore } from "@/stores/sketchStore";
import { useRepairStore } from "@/stores/repairStore";
import { documentStore } from "@/stores/documentStore";
import { getModelToolController } from "@/tools/modelTools/modelToolBridge";
import { HistoryList, type HistoryRowActions, type HistoryValueEdit } from "./HistoryList";
import { canEditFeatureValue, commitFeatureValue } from "./featureValueEdit";
import { ConstraintList, isDimensionalConstraint, visibleConstraints } from "./ConstraintList";
import { RepairPanel } from "@/features/repair/RepairPanel";
import { suppressFeature, rollToIndex, deleteFeature } from "./historyActions";
import { setBodyColor, setFaceColor } from "@/features/tree/treeActions";
import { cn } from "@/ui/cn";
import type { Rgba } from "@/ipc/types";
import { sketchStatusText, sketchStatusSentence } from "@/features/sketch/constraintStatus";
import { CONSTRAINT_PRESENTATION } from "@/features/sketch/constraintCatalog";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import { useSettingsStore } from "@/stores/settingsStore";
import { lengthSuffix } from "@/units/format";
import { createClient } from "@/ipc/client";
import { deleteConstraints } from "@/tools/sketch/sketchService";
import type { SketchStatus } from "@/stores/documentStore";
import { IMPORT_STEP_OP_TYPE } from "@/ipc/types";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";

/** Delete one constraint (row × button) — fire-and-forget, mirrors useShortcuts'
 * deleteEntities call: the service re-solves + writes the session back, so the
 * list re-renders off the live sketchStore subscription. */
function deleteConstraint(id: string): void {
  void deleteConstraints(createClient(), [id]);
}

/** Click a history chip → select that feature; double-click → parametric re-edit. */
function selectFeature(id: string): void {
  selectionStore.getState().set([{ kind: "feature", id }]);
}
/**
 * Route a re-edit on the feature's exact `opType`, falling back to the coarse
 * `kind` for a projection emitted before `opType` existed. The backend folds
 * Chamfer+Shell into kind `fillet` and the pattern/mirror ops into `boolean`
 * (`dto.rs feature_kind`), so routing on `kind` alone sent a Chamfer into the
 * fillet editor and left Shell/patterns/Mirror unreachable on the real lane.
 */
function editFeature(item: FeatureMeta): void {
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
 * gate needs — `FeatureState` renders `documentStore.features` whole, so the row
 * index IS the global index.
 */
function makeValueEdit(
  appliedOps: number,
  modelTool: ModelTool,
): (item: FeatureMeta, index: number) => HistoryValueEdit {
  return (item, index) => ({
    editable: canEditFeatureValue(item, index, appliedOps, modelTool),
    onCommit: (it, value) => {
      void commitFeatureValue(it.id, it.opType ?? "", value);
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

/**
 * Context-aware inspector (prototype 1c), three states:
 *  - EMPTY     — nothing selected in model mode
 *  - SELECTION — a body/sketch selected in model mode (status + HISTORY)
 *  - SKETCH    — sketch mode (DOF warn card + CONSTRAINTS)
 */
export function InspectorPanel() {
  const mode = useToolStore((s) => s.mode);
  const sel = useSelectionStore(primarySelection);
  const bodies = useDocumentStore((s) => s.bodies);
  const sketches = useDocumentStore((s) => s.sketches);
  const features = useDocumentStore((s) => s.features);
  const activeSketchId = useViewportStore((s) => s.activeSketchId);
  const constraints = useSketchStore((s) => s.session?.constraints);
  // Only for the ConstraintList's locked-row filter — the panel renders no geometry.
  const sketchEntities = useSketchStore((s) => s.session?.entities);
  const conflictingIds = useSketchStore((s) => s.conflictingIds);
  const repairPanelOpen = useRepairStore((s) => s.panelOpen);
  const repairItemCount = useRepairStore((s) => s.items.length);

  const sketching = mode === "sketch";
  // Enter the REPAIR state from the banner (panel open) OR by selecting a feature
  // that is itself in NeedsRepair — but only while there is something to repair.
  const selFeatureNeedsRepair =
    sel?.kind === "feature" && features.find((f) => f.id === sel.id)?.status === "needsRepair";
  const showRepair = !sketching && repairItemCount > 0 && (repairPanelOpen || selFeatureNeedsRepair);

  return (
    <div className="absolute bottom-[34px] right-0 top-0 z-20 box-border w-[260px] overflow-auto border-l border-border bg-panel p-4">
      {sketching ? (
        <SketchState
          sketchName={sketches[activeSketchId ?? ""]?.name ?? "Sketch"}
          dof={sketches[activeSketchId ?? ""]?.dof ?? 0}
          status={sketches[activeSketchId ?? ""]?.status ?? "under"}
          constraints={constraints ?? []}
          entities={sketchEntities ?? []}
          conflictingIds={conflictingIds}
        />
      ) : showRepair ? (
        <RepairPanel />
      ) : sel && sel.kind === "feature" ? (
        <FeatureState featureId={sel.id} features={features} />
      ) : sel ? (
        <SelectionState sel={sel} bodies={bodies} sketches={sketches} features={features} />
      ) : (
        <EmptyState />
      )}
    </div>
  );
}

function EmptyState() {
  return (
    <div className="px-2 py-[26px] text-center">
      <div className="mx-auto mb-2.5 flex h-10 w-10 items-center justify-center rounded-full bg-well">
        <Icon name="select" size={18} strokeWidth={1.7} className="text-ink-6" />
      </div>
      <div className="text-[13px] font-semibold text-ink-3">Nothing selected</div>
      <div className="mt-1 text-[12px] leading-normal text-ink-6">
        Select a body, sketch, face or edge to see its parameters and history.
      </div>
    </div>
  );
}

function SelectionState({
  sel,
  bodies,
  sketches,
  features,
}: {
  sel: EntityRef;
  bodies: Record<string, BodyMeta>;
  sketches: Record<string, SketchMeta>;
  features: FeatureMeta[];
}) {
  const isBody = sel.kind === "body";
  const isFace = sel.kind === "face";
  const isEdge = sel.kind === "edge";
  const isSketch = sel.kind === "sketch" || sel.kind === "sketchRegion";
  const sketchId = sel.kind === "sketchRegion" ? sel.sketchId : sel.id;
  const name = isFace || isEdge
    ? bodies[sel.bodyId ?? ""]?.name ?? ""
    : bodies[sel.id]?.name ?? sketches[sketchId]?.name ?? "";
  const statusName = isBody
    ? "Solid body"
    : isFace
      ? "Face"
      : isEdge
        ? "Edge"
        : sel.kind === "sketchRegion"
          ? "Sketch profile"
          : "Sketch";
  // Body → its full lineage (Sketch 1 / Extrude / Fillet); sketch → the extrude
  // that consumed it (prototype's two hardcoded HISTORY arrays).
  const history = isBody
    ? features.slice(0, 3)
    : features.filter((f) => f.kind === "extrude").slice(0, 1);
  const showDof = isSketch;
  const dof = sketches[sketchId]?.dof ?? 0;

  return (
    <>
      <div className="text-[15px] font-semibold text-ink">{name}</div>
      <div className="mt-0.5 text-[12px] text-ink-5">{statusName}</div>
      {showDof && (
        <div className="mt-1 text-[12px] font-medium text-warn">
          Under-constrained · DOF {dof}
        </div>
      )}

      {isBody && (
        <>
          <SectionLabel className="pb-1.5 pt-4">Appearance</SectionLabel>
          <BodyAppearanceSection bodyId={sel.id} />
        </>
      )}

      {isFace && sel.bodyId && sel.elementId && (
        <>
          <SectionLabel className="pb-1.5 pt-4">Appearance</SectionLabel>
          <FaceAppearanceSection bodyId={sel.bodyId} elementId={sel.elementId} />
        </>
      )}

      <SectionLabel className="pb-1.5 pt-4">History</SectionLabel>
      <HistoryList items={history} onSelect={selectFeature} onEdit={editFeature} />

      {showDof && (
        <>
          <SectionLabel className="pb-1.5 pt-3.5">Constraints</SectionLabel>
          <div className="text-[12px] leading-normal text-ink-6">
            Select geometry to constrain.
          </div>
        </>
      )}
    </>
  );
}

function FeatureState({
  featureId,
  features,
}: {
  featureId: string;
  features: FeatureMeta[];
}) {
  // Subscriptions, not `getState()` reads: arming a model tool or moving the
  // rollback bar must re-render the rows into their read-only state (H3 gates).
  const modelTool = useToolStore((s) => s.modelTool);
  const appliedOps = useDocumentStore((s) => s.appliedOps);
  const feat = features.find((f) => f.id === featureId);
  return (
    <>
      <div className="text-[15px] font-semibold text-ink">{feat?.label ?? "Feature"}</div>
      <div className="mt-0.5 text-[12px] text-ink-5">
        {feat?.kind ? `${cap(feat.kind)} feature` : "Feature"}
        {feat?.valueText ? ` · ${feat.valueText}` : ""}
      </div>
      {feat?.kind === "sketch" && <SketchDimensions featureId={featureId} />}
      <SectionLabel className="pb-1.5 pt-4">History</SectionLabel>
      {/* The ONLY view that may pass `appliedOps`: it renders `features` WHOLE, so
          a row index is the global timeline index the cursor is expressed in. */}
      <HistoryList
        items={features}
        selectedId={featureId}
        onSelect={selectFeature}
        onEdit={editFeature}
        rowActions={rowActions}
        valueEdit={makeValueEdit(appliedOps, modelTool)}
        appliedOps={appliedOps}
        onRollToEnd={() => void rollToIndex(features.length - 1)}
      />
      {/* BELOW the list on purpose: a selection-dependent block ABOVE it changes the
          panel height as the selection moves, which shifts the rows out from under a
          double-click's second press (it broke the hole re-edit spec exactly so). */}
      {feat?.primaryValue !== undefined ? (
        <div className="mt-2 text-[12px] leading-normal text-ink-6">
          Click a row's value to change it, or double-click the row for the full editor.
        </div>
      ) : null}
      {/* Also below the list, same reason. */}
      <FeatureDependenciesSection featureId={featureId} features={features} />
    </>
  );
}

const cap = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);

/**
 * H10 dependency view — "Depends on" (upstream) / "Used by" (downstream), read
 * straight off `client.featureDependencies`. Ids are mapped to their live
 * `FeatureMeta.label` (falling back to the bare id for a stale/legacy reference
 * `features` no longer carries); each row selects that feature. Either
 * subsection — and the whole block — hides when empty, so a leaf feature with no
 * lineage renders nothing here.
 */
function FeatureDependenciesSection({
  featureId,
  features,
}: {
  featureId: string;
  features: FeatureMeta[];
}) {
  const [deps, setDeps] = useState<{ upstream: string[]; downstream: string[] } | null>(null);

  useEffect(() => {
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
function SketchDimensions({ featureId }: { featureId: string }) {
  const unit = useSettingsStore((s) => s.displayUnit);
  const [sketchId, setSketchId] = useState<string | null>(null);
  const [dims, setDims] = useState<SketchConstraint[]>([]);

  /** `alive` guards against a selection change landing mid-fetch: two round-trips
   *  in, a resolved older request must not paint another feature's dimensions. */
  const load = useCallback(
    async (alive: () => boolean = () => true) => {
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

function FaceAppearanceSection({ bodyId, elementId }: { bodyId: string; elementId: string }) {
  const color = useDocumentStore((s) => s.bodies[bodyId]?.faceColors?.[elementId]);
  const [hex, setHex] = useState(() => rgbaToHex(color));
  useEffect(() => setHex(rgbaToHex(color)), [color]);
  const alpha = color?.[3] ?? 255;
  const alphaPct = Math.round((alpha / 255) * 100);

  return (
    <div>
      <div className="flex items-center gap-2">
        <input
          type="color"
          value={hex}
          onChange={(e) =>
            void setFaceColor(bodyId, elementId, hexToRgba(e.target.value, alpha / 255))
          }
          className="h-8 w-8 cursor-pointer rounded border-0 bg-transparent p-0"
          aria-label="Face color"
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
            void setFaceColor(bodyId, elementId, [base[0], base[1], base[2], a]);
          }}
          className="flex-1"
          aria-label="Face opacity"
        />
        <span className="w-8 text-right text-[11px] text-ink-5">{alphaPct}%</span>
      </div>
      {color && (
        <button
          type="button"
          onClick={() => void setFaceColor(bodyId, elementId, null)}
          className="mt-2 text-[11px] text-ink-5 hover:text-ink"
        >
          Reset to default
        </button>
      )}
    </div>
  );
}

function BodyAppearanceSection({ bodyId }: { bodyId: string }) {
  const color = useDocumentStore((s) => s.bodies[bodyId]?.color);
  const [hex, setHex] = useState(() => rgbaToHex(color));
  useEffect(() => setHex(rgbaToHex(color)), [color]);
  const alpha = color?.[3] ?? 255;
  const alphaPct = Math.round((alpha / 255) * 100);

  return (
    <div>
      <div className="flex items-center gap-2">
        <input
          type="color"
          value={hex}
          onChange={(e) => void setBodyColor(bodyId, hexToRgba(e.target.value, alpha / 255))}
          className="h-8 w-8 cursor-pointer rounded border-0 bg-transparent p-0"
          aria-label="Body color"
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
            void setBodyColor(bodyId, [base[0], base[1], base[2], a]);
          }}
          className="flex-1"
          aria-label="Body opacity"
        />
        <span className="w-8 text-right text-[11px] text-ink-5">{alphaPct}%</span>
      </div>
      {color && (
        <button
          type="button"
          onClick={() => void setBodyColor(bodyId, null)}
          className="mt-2 text-[11px] text-ink-5 hover:text-ink"
        >
          Reset to default
        </button>
      )}
    </div>
  );
}

function rgbaToHex(c: Rgba | undefined): string {
  if (!c) return "#a9aeb6";
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

function SketchState({
  sketchName,
  dof,
  status,
  constraints,
  entities,
  conflictingIds,
}: {
  sketchName: string;
  dof: number;
  status: SketchStatus;
  constraints: SketchConstraint[];
  entities: SketchEntity[];
  conflictingIds: string[];
}) {
  const { label, tone } = sketchStatusText(status, dof);
  const solved = status === "ok";
  return (
    <>
      <div className="text-[15px] font-semibold text-ink">{sketchName}</div>

      {/* DOF state card (1e treatment folded into 1c per WP spec). */}
      <div
        className={cn(
          "mt-3 rounded-md border px-3 py-2.5",
          solved ? "border-border bg-well" : "border-warn-border bg-warn-surface",
        )}
      >
        <div className={cn("text-[12px] font-medium", tone === "ok" ? "text-ink-4" : "text-warn")}>
          {label}
        </div>
        <div className={cn("mt-1 text-[12px] leading-normal", solved ? "text-ink-5" : "text-warn-strong")}>
          {sketchStatusSentence(status, dof)}
        </div>
      </div>

      <SectionLabel className="pb-1.5 pt-4">Constraints</SectionLabel>
      {visibleConstraints(constraints, entities).length > 0 ? (
        <ConstraintList
          constraints={constraints}
          entities={entities}
          onDelete={deleteConstraint}
          conflictingIds={conflictingIds}
        />
      ) : (
        <div className="text-[12px] leading-normal text-ink-6">
          No constraints yet.
        </div>
      )}
      <div className="mt-2 text-[11.5px] leading-normal text-ink-6">
        Drag geometry or add constraints until DOF reaches 0.
      </div>
    </>
  );
}
