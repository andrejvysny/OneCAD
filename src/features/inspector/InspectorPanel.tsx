import { Icon } from "@/icons/Icon";
import {
  useDocumentStore,
  type BodyMeta,
  type FeatureMeta,
  type SketchMeta,
} from "@/stores/documentStore";
import { useSelectionStore, primarySelection, type EntityRef } from "@/stores/selectionStore";
import { useToolStore } from "@/stores/toolStore";
import { useViewportStore } from "@/stores/viewportStore";
import { useRepairStore } from "@/stores/repairStore";
import { RepairPanel } from "@/features/repair/RepairPanel";
import { OperationDiagnosticDetails } from "@/features/inspector/OperationDiagnosticDetails";
import { editFeature } from "@/features/inspector/sections";
import { InspectorSectionHost } from "@/modules/modeling/InspectorSectionHost";
import { cn } from "@/ui/cn";
import { sketchStatusText, sketchStatusSentence } from "@/features/sketch/constraintStatus";
import type { SketchStatus } from "@/stores/documentStore";

/**
 * Context-aware inspector (prototype 1c), three states:
 *  - EMPTY     — nothing selected in model mode
 *  - SELECTION — a body/sketch selected in model mode (status + sections)
 *  - SKETCH    — sketch mode (DOF warn card + sections)
 *
 * The panel owns the FRAME and the per-state chrome — headings, the DOF card,
 * the trailing hints. The labelled SECTIONS are platform contributions rendered
 * by `InspectorSectionHost` in registry order (see
 * `@/modules/modeling/inspectorSections`), which is what lets a module other
 * than modeling put something here.
 *
 * EMPTY and REPAIR deliberately do NOT host sections: they replace the panel
 * body outright, and the frozen contract records no sections for either.
 */
export function InspectorPanel() {
  const mode = useToolStore((s) => s.mode);
  const sel = useSelectionStore(primarySelection);
  const bodies = useDocumentStore((s) => s.bodies);
  const sketches = useDocumentStore((s) => s.sketches);
  const features = useDocumentStore((s) => s.features);
  const activeSketchId = useViewportStore((s) => s.activeSketchId);
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
        />
      ) : showRepair ? (
        <RepairPanel />
      ) : sel && sel.kind === "feature" ? (
        <FeatureState featureId={sel.id} features={features} />
      ) : sel ? (
        <SelectionState sel={sel} bodies={bodies} sketches={sketches} />
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
}: {
  sel: EntityRef;
  bodies: Record<string, BodyMeta>;
  sketches: Record<string, SketchMeta>;
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
  const dof = sketches[sketchId]?.dof ?? 0;

  return (
    <>
      <div className="text-[15px] font-semibold text-ink">{name}</div>
      <div className="mt-0.5 text-[12px] text-ink-5">{statusName}</div>
      {isSketch && (
        <div className="mt-1 text-[12px] font-medium text-warn">
          Under-constrained · DOF {dof}
        </div>
      )}
      <InspectorSectionHost />
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
  return (
    <>
      <div className="text-[15px] font-semibold text-ink">{feat?.label ?? "Feature"}</div>
      <div className="mt-0.5 text-[12px] text-ink-5">
        {feat?.kind ? `${cap(feat.kind)} feature` : "Feature"}
        {feat?.valueText ? ` · ${feat.valueText}` : ""}
      </div>
      {feat?.status === "error" && (
        <>
          <div className="mt-3 rounded-sm border border-border bg-well px-2.5 py-2">
            <div className="text-[12px] font-medium text-traffic-close">Feature failed</div>
            {feat.statusMessage && <div className="mt-1 text-[12px] leading-normal text-ink-2">{feat.statusMessage}</div>}
          </div>
          <OperationDiagnosticDetails diagnostics={feat.diagnostics} />
          <button
            type="button"
            data-testid="feature-edit-retry"
            onClick={() => editFeature(feat)}
            className="mt-3 rounded-sm bg-accent px-2.5 py-1.5 text-[12px] font-medium text-on-accent hover:bg-accent-hover"
          >
            Edit and retry
          </button>
        </>
      )}
      <InspectorSectionHost />
    </>
  );
}

const cap = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);

function SketchState({
  sketchName,
  dof,
  status,
}: {
  sketchName: string;
  dof: number;
  status: SketchStatus;
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

      <InspectorSectionHost />

      <div className="mt-2 text-[11.5px] leading-normal text-ink-6">
        Drag geometry or add constraints until DOF reaches 0.
      </div>
    </>
  );
}
