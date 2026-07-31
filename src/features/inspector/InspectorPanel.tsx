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
import { useToolStore } from "@/stores/toolStore";
import { useViewportStore } from "@/stores/viewportStore";
import { useSketchStore } from "@/stores/sketchStore";
import { useRepairStore } from "@/stores/repairStore";
import { useHistoryStore } from "@/stores/historyStore";
import { documentStore } from "@/stores/documentStore";
import { getModelToolController } from "@/tools/modelTools/modelToolBridge";
import { HistoryList, type HistoryRowActions } from "./HistoryList";
import { ConstraintList } from "./ConstraintList";
import { RepairPanel } from "@/features/repair/RepairPanel";
import { suppressFeature, rollToIndex, deleteFeature } from "./historyActions";
import { cn } from "@/ui/cn";
import { sketchStatusText, sketchStatusSentence } from "@/features/sketch/constraintStatus";
import { createClient } from "@/ipc/client";
import { deleteConstraints } from "@/tools/sketch/sketchService";
import type { SketchStatus } from "@/stores/documentStore";
import type { SketchConstraint } from "@/ipc/types";

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
    case "LinearPattern":
      c?.editLinearPatternFeature(item.id);
      return;
    case "CircularPattern":
      c?.editCircularPatternFeature(item.id);
      return;
    case "MirrorBody":
      c?.editMirrorFeature(item.id);
      return;
    default:
      // Sketch/Boolean/opaque rows have no parametric editor yet.
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

/** Build the per-row history affordances (suppress / roll-to-here / delete). */
function makeRowActions(suppressedMap: Record<string, boolean>): (item: FeatureMeta) => HistoryRowActions {
  return (item) => ({
    suppressed: !!suppressedMap[item.id],
    onToggleSuppress: (it) => void suppressFeature(it.id, !suppressedMap[it.id]),
    onRoll: (it) => {
      // Rollback cursor = applied op count (timeline.rs), so "roll to here" =
      // globalIndex + 1; resolve the GLOBAL index (the list may be a slice).
      const idx = documentStore.getState().features.findIndex((f) => f.id === it.id);
      if (idx >= 0) void rollToIndex(idx);
    },
    onDelete: (it) => void deleteFeature(it.id),
  });
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
  const sketchId = sel.kind === "sketchRegion" ? sel.sketchId : sel.id;
  const name = bodies[sel.id]?.name ?? sketches[sketchId]?.name ?? "";
  const statusName = isBody ? "Solid body" : sel.kind === "sketchRegion" ? "Sketch profile" : "Sketch";
  // Body → its full lineage (Sketch 1 / Extrude / Fillet); sketch → the extrude
  // that consumed it (prototype's two hardcoded HISTORY arrays).
  const history = isBody
    ? features.slice(0, 3)
    : features.filter((f) => f.kind === "extrude").slice(0, 1);
  const showDof = !isBody;
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
  const feat = features.find((f) => f.id === featureId);
  // The full-timeline view carries the per-row affordances (suppress/roll/delete).
  const suppressedMap = useHistoryStore((s) => s.suppressed);
  return (
    <>
      <div className="text-[15px] font-semibold text-ink">{feat?.label ?? "Feature"}</div>
      <div className="mt-0.5 text-[12px] text-ink-5">
        {feat?.kind ? `${cap(feat.kind)} feature` : "Feature"}
        {feat?.valueText ? ` · ${feat.valueText}` : ""}
      </div>
      {feat?.kind === "extrude" && (
        <div className="mt-1 text-[12px] text-ink-6">Double-click to edit the depth.</div>
      )}
      {feat?.kind === "revolve" && (
        <div className="mt-1 text-[12px] text-ink-6">Double-click to edit the angle.</div>
      )}
      {feat?.kind === "fillet" && (
        <div className="mt-1 text-[12px] text-ink-6">Double-click to edit the radius.</div>
      )}
      <SectionLabel className="pb-1.5 pt-4">History</SectionLabel>
      <HistoryList
        items={features}
        selectedId={featureId}
        onSelect={selectFeature}
        onEdit={editFeature}
        rowActions={makeRowActions(suppressedMap)}
      />
    </>
  );
}

const cap = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);

function SketchState({
  sketchName,
  dof,
  status,
  constraints,
  conflictingIds,
}: {
  sketchName: string;
  dof: number;
  status: SketchStatus;
  constraints: SketchConstraint[];
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
      {constraints.length > 0 ? (
        <ConstraintList constraints={constraints} onDelete={deleteConstraint} conflictingIds={conflictingIds} />
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
