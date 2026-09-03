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
import { useSketchStore } from "@/stores/sketchStore";
import { useRepairStore } from "@/stores/repairStore";
import { RepairPanel } from "@/features/repair/RepairPanel";
import { OperationDiagnosticDetails } from "@/features/inspector/OperationDiagnosticDetails";
import { editFeature } from "@/features/inspector/sections";
import { GearPropertiesPanel, GearSelectedSummary } from "@/features/inspector/GearPropertiesPanel";
import { useToolChipStore } from "@/stores/toolChipStore";
import { InspectorSectionHost } from "@/modules/modeling/InspectorSectionHost";
import { ConstraintMenu } from "@/features/sketch/ConstraintMenu";
import { cn } from "@/ui/cn";
import {
  sketchStatusText,
  sketchStatusSentence,
  sketchStatusToneClass,
  emptySketchCard,
} from "@/features/sketch/constraintStatus";
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
 * REPAIR deliberately does NOT host sections: it replaces the panel body
 * outright, and the frozen contract records none for it.
 *
 * EMPTY DOES host them, as of WP-VE.2. It used to be a bare "nothing selected"
 * placard for the same reason REPAIR is — every section then was about the
 * SELECTION, so with none there was nothing to show. A document-level section
 * (Variables) breaks that assumption: gating it on a selection would hide the
 * document's own parameters behind picking some unrelated body. The placard
 * stays; sections render under it.
 */
export function InspectorPanel() {
  const mode = useToolStore((s) => s.mode);
  const sel = useSelectionStore(primarySelection);
  const bodies = useDocumentStore((s) => s.bodies);
  const sketches = useDocumentStore((s) => s.sketches);
  const features = useDocumentStore((s) => s.features);
  const activeSketchId = useViewportStore((s) => s.activeSketchId);
  // Design item 12 / audit A11a: the DOF card's "empty sketch" branch needs
  // the LIVE session's entity count, not the document registry's dof/status
  // — a fresh sketch reports dof 0/"ok" from the registry same as a fully
  // constrained one, which is exactly the false-completeness copy being fixed.
  const sketchSession = useSketchStore((s) => s.session);
  const repairPanelOpen = useRepairStore((s) => s.panelOpen);
  const repairItemCount = useRepairStore((s) => s.items.length);
  // Gear Generator G1-h: an armed gear (fresh placement OR `editGearFeature`
  // re-edit) owns the panel outright — its properties form has too many fields
  // for the floating chip (see `GearPropertiesPanel`'s header comment).
  const gearArmed = useToolChipStore((s) => s.kind === "gear");

  const sketching = mode === "sketch";
  // Enter the REPAIR state from the banner (panel open) OR by selecting a feature
  // that is itself in NeedsRepair — but only while there is something to repair.
  const selFeatureNeedsRepair =
    sel?.kind === "feature" && features.find((f) => f.id === sel.id)?.status === "needsRepair";
  const showRepair = !gearArmed && !sketching && repairItemCount > 0 && (repairPanelOpen || selFeatureNeedsRepair);
  // A FEATURE-ROW selection that resolves to an already-committed Gear op
  // (FreeCAD's "select an object, see its properties"). Deliberately NOT
  // extended to a BODY-tree selection: nothing in the projection correlates a
  // bodyId back to the feature that minted it (`body_<opId>`'s `opId` is the
  // WORKER's plan-step id, not the record's own — confirmed different on the
  // mock lane), so a body click stays on the ordinary `SelectionState` below
  // until that mapping is real. A history-row click already reaches this.
  const gearFeatureId = !gearArmed ? resolveGearFeatureId(sel, features) : null;

  return (
    <div className="absolute bottom-[34px] right-0 top-0 z-20 box-border w-[260px] overflow-auto border-l border-border bg-panel p-4">
      {gearArmed ? (
        <GearPropertiesPanel />
      ) : sketching && activeSketchId && sketches[activeSketchId] ? (
        <SketchState
          sketchName={sketches[activeSketchId].name}
          dof={sketches[activeSketchId].dof}
          status={sketches[activeSketchId].status}
          entityCount={sketchSession?.entities.length ?? 0}
        />
      ) : sketching ? (
        // Plane-pick phase (no activeSketchId yet) or the sketch registry
        // hasn't caught up: no solve state exists, so claim nothing about DOF
        // (mirrors SelectionState's "absent solve state renders no placard").
        <div className="text-[12px] text-ink-6">Select a sketch plane to begin.</div>
      ) : showRepair ? (
        <RepairPanel />
      ) : gearFeatureId ? (
        <GearSelectedSummary featureId={gearFeatureId} />
      ) : sel && sel.kind === "feature" ? (
        <FeatureState featureId={sel.id} features={features} />
      ) : sel ? (
        <SelectionState sel={sel} bodies={bodies} sketches={sketches} />
      ) : (
        <>
          <EmptyState />
          <InspectorSectionHost />
        </>
      )}
    </div>
  );
}

/** `null` unless `sel` is a feature row whose `opType` is Gear. */
function resolveGearFeatureId(sel: EntityRef | null, features: FeatureMeta[]): string | null {
  if (!sel || sel.kind !== "feature") return null;
  return features.find((f) => f.id === sel.id)?.opType === "Gear" ? sel.id : null;
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
  /*
   * ONE constraint-status authority (U7).
   *
   * This branch used to hardcode `Under-constrained · DOF {dof}` with no status
   * check and a `?? 0` default, so selecting a FULLY CONSTRAINED sketch in the
   * tree rendered the impossible "Under-constrained · DOF 0" the UX audit caught
   * — and an unknown sketch rendered it too. D14 fixed `sketchStatusText`, which
   * the sketch chrome bar and the sketch-card branch below already use; this was
   * the second, silent authority.
   *
   * ABSENT solve state renders NO placard at all (LGU-1 WP-A, defect F1).
   * Defaulting it to `dof: 0, status: "under"` made `sketchStatusText` report
   * "Fully constrained · DOF 0" for a sketch the registry has never heard of —
   * the strongest possible claim, made on no evidence, which the screenshot
   * audit caught on screen in model mode. The `dof === 0` guard INSIDE
   * `sketchStatusText` is a different thing and stays: there the zero is a real
   * solver result whose status label is merely lagging. Here there is no solve
   * at all, and the honest render is silence.
   */
  const solve = sketches[sketchId];
  const status = solve ? sketchStatusText(solve.status, solve.dof) : null;

  return (
    <>
      <div className="text-[15px] font-semibold text-ink">{name}</div>
      <div className="mt-0.5 text-[12px] text-ink-5">{statusName}</div>
      {isSketch && status !== null && (
        <div className={cn("mt-1 text-[12px] font-medium", sketchStatusToneClass(status.tone))}>
          {status.label}
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
      {feat?.status === "error" ? (
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
      ) : (
        feat?.diagnostics &&
        feat.diagnostics.length > 0 && (
          // Non-error terminal (e.g. REGION_REBOUND_BY_ANCHOR, SKETCH_ENTITY_DEGENERATE):
          // same detail rendering, framed with the warn tokens instead of the failure well.
          <div
            data-testid="feature-diagnostic-section"
            className="mt-3 rounded-sm border border-[color:var(--banner-warn-line)] bg-[var(--banner-warn-bg)] px-2.5 py-2"
          >
            <div className="text-[12px] font-medium text-warn">Feature warning</div>
            <OperationDiagnosticDetails diagnostics={feat.diagnostics} />
          </div>
        )
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
  entityCount,
}: {
  sketchName: string;
  dof: number;
  status: SketchStatus;
  entityCount: number;
}) {
  // A blank sketch has nothing to be "fully defined" about — its registry
  // dof/status (typically 0/"ok") reads through the ordinary path as the
  // false completeness claim the audit caught (design item 12 / A11a).
  const empty = entityCount === 0;
  const { label, tone, sentence } = empty
    ? emptySketchCard()
    : { ...sketchStatusText(status, dof), sentence: sketchStatusSentence(status, dof) };
  // under/ok share the plain neutral card — under-constrained mid-sketch is
  // normal, not a warning. over/error each get their own severity tint so a
  // redundant constraint doesn't read as visually identical to a conflicting
  // one.
  const cardClass =
    tone === "over"
      ? "border-warn-border bg-warn-surface"
      : tone === "error"
        ? "border-danger-border bg-danger-surface"
        : "border-border bg-well";
  const bodyClass = tone === "over" ? "text-warn-strong" : tone === "error" ? "text-danger-strong" : "text-ink-5";
  return (
    <>
      <div className="text-[15px] font-semibold text-ink">{sketchName}</div>

      {/* DOF state card (1e treatment folded into 1c per WP spec). */}
      <div className={cn("mt-3 rounded-md border px-3 py-2.5", cardClass)}>
        <div className={cn("text-[12px] font-medium", sketchStatusToneClass(tone))}>{label}</div>
        <div className={cn("mt-1 text-[12px] leading-normal", bodyClass)}>{sentence}</div>
      </div>

      {/* Moved here from the top chrome bar (Sketcher UX cleanup): constraint
          discovery lives with the DOF card it feeds, not the toolbar. */}
      <div className="mt-3">
        <ConstraintMenu />
      </div>

      <InspectorSectionHost />

      <div className="mt-2 text-[11.5px] leading-normal text-ink-6">
        Drag geometry or add constraints until DOF reaches 0.
      </div>
    </>
  );
}
