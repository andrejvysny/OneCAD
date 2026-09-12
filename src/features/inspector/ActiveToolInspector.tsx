import {
  BooleanModeSegments,
  DraftSegment,
  EndConditionSegments,
  SymmetricToggle,
} from "@/features/toolbar/ExtrudeChipControls";
import { EdgeOpInspectorControls } from "@/features/toolbar/EdgeOpChipControls";
import { HoleInspectorControls } from "@/features/toolbar/HoleChipCluster";
import {
  AlignButton,
  BOOLEAN_OPS,
  CopyToggle,
  CountStepper,
  FuseToggle,
  MIRROR_PLANES,
  OffsetFaceInspectorControls,
  PATTERN_AXES,
  SegmentToggle,
  TransformModeSegments,
} from "@/features/toolbar/ModelToolChips";
import { useCallback, type ReactNode } from "react";
import { toolChipStore, useToolChipStore } from "@/stores/toolChipStore";
import type { ToolChipState, ToolValueValidation } from "@/stores/toolChipStore";
import {
  activeToolLabel,
  activeToolPresentation,
  formatActiveToolField,
} from "@/tools/modelTools/activeToolPresentation";
import { useOperationAttemptStore } from "@/stores/operationAttemptStore";
import { useDocumentStore } from "@/stores/documentStore";
import { setToolChipDockHost } from "@/features/toolbar/toolChipDockBridge";
import { ActiveToolTargets } from "./ActiveToolTargets";
import { FitPreviewButton } from "./FitPreviewButton";

function ValidationDetails({ validation }: { validation: ToolValueValidation }) {
  if (validation.status === "valid") return null;
  return (
    <div role="alert" className="mt-2 flex items-center gap-1 text-[12px] text-warn">
      <span>{validation.message}</span>
      {validation.status === "invalid" && validation.suggestedValue !== undefined && (
        <button
          type="button"
          data-testid="inspector-validation-use-suggested"
          onClick={() => toolChipStore.getState().onUseSuggestedValue?.()}
        >
          {validation.suggestedLabel ?? "Use suggested"}
        </button>
      )}
    </div>
  );
}

function InspectorSection({
  title,
  validation,
  subtitle,
  children,
}: {
  title: string;
  validation: ToolValueValidation;
  subtitle?: string;
  children: ReactNode;
}) {
  return (
    <section data-testid="active-tool-inspector" className="mb-4 border-b border-border pb-4">
      <div className="text-[15px] font-semibold text-ink">{title}</div>
      {subtitle && <div className="mt-0.5 text-[12px] text-ink-5">{subtitle}</div>}
      <ValidationDetails validation={validation} />
      <div className="mt-3 flex flex-col items-start gap-2">{children}</div>
    </section>
  );
}

function HoleInspector({ validation }: { validation: ToolValueValidation }) {
  return (
    <InspectorSection title="Hole" validation={validation}>
      <HoleInspectorControls />
    </InspectorSection>
  );
}

function EdgeOperationInspector({ state, validation }: { state: ToolChipState; validation: ToolValueValidation }) {
  const title = state.kind === "filletRadius" ? state.edgeOp : state.kind === "shellThickness" ? "Shell" : "Offset face";
  return (
    <InspectorSection title={title} validation={validation}>
      {state.kind === "filletRadius" && state.showEdgeOpSegments && (
        <EdgeOpInspectorControls
          edgeOp={state.edgeOp}
          onEdgeOp={(edgeOp) => toolChipStore.getState().onEdgeOp?.(edgeOp)}
          distance2={state.distance2}
          onDistance2={(distance) => toolChipStore.getState().onDistance2?.(distance)}
          chamferAngleDeg={state.chamferAngleDeg}
          onChamferAngle={(angle) => toolChipStore.getState().onChamferAngle?.(angle)}
          showChamferFlip={state.showChamferFlip}
          onChamferFlip={() => toolChipStore.getState().onChamferFlip?.()}
          onConfirm={() => toolChipStore.getState().onConfirm?.()}
        />
      )}
      {state.kind === "offsetFace" && <OffsetFaceInspectorControls />}
    </InspectorSection>
  );
}

function BooleanModeControls({ state }: { state: ToolChipState }) {
  if (!state.showBooleanSegments) return null;
  return (
    <BooleanModeSegments
      active={state.booleanMode}
      canBoolean={state.canBoolean}
      onPick={(mode) => toolChipStore.getState().onBooleanMode?.(mode)}
    />
  );
}

function ExtrudeInspector({
  state,
  validation,
  subtitle,
}: {
  state: ToolChipState;
  validation: ToolValueValidation;
  subtitle: string;
}) {
  return (
    <InspectorSection
      title="Extrude"
      validation={validation}
      subtitle={subtitle}
    >
      {state.showEndConditions && (
        <EndConditionSegments
          active={state.endCondition}
          canUseBodyEnds={state.canUseBodyEnds}
          booleanMode={state.booleanMode}
          onPick={(end) => toolChipStore.getState().onEndCondition?.(end)}
        />
      )}
      {state.showDraft && (
        <DraftSegment
          deg={state.draftAngleDeg}
          onDeg={(degrees) => toolChipStore.getState().onDraftAngle?.(degrees)}
          onConfirm={() => toolChipStore.getState().onConfirm?.()}
        />
      )}
      {state.showSymmetric && state.endCondition === "Blind" && (
        <SymmetricToggle
          pressed={state.symmetric}
          onToggle={() => toolChipStore.getState().onSymmetric?.(!state.symmetric)}
        />
      )}
      <BooleanModeControls state={state} />
    </InspectorSection>
  );
}

function RevolveInspector({ state, validation }: { state: ToolChipState; validation: ToolValueValidation }) {
  return (
    <InspectorSection title="Revolve" validation={validation}>
      <button
        type="button"
        data-testid="inspector-revolve-change-axis"
        aria-label="Change axis"
        title="Pick a different revolve axis"
        onClick={() => toolChipStore.getState().onResetAxis?.()}
        className="rounded-full border border-border bg-surface px-2 py-1 text-[11.5px] font-medium text-ink-2 shadow-popover hover:bg-hover-2"
      >
        Change axis
      </button>
      <BooleanModeControls state={state} />
    </InspectorSection>
  );
}

function PatternInspector({ state, validation }: { state: ToolChipState; validation: ToolValueValidation }) {
  const title = state.kind === "linearPattern" ? "Linear pattern" : "Circular pattern";
  return (
    <InspectorSection title={title} validation={validation}>
      <SegmentToggle
        options={PATTERN_AXES}
        active={state.axis}
        label="Pattern axis"
        testid={(axis) => `chip-pattern-axis-${axis.toLowerCase()}`}
        onPick={(axis) => toolChipStore.getState().onAxis?.(axis)}
      />
      <CountStepper
        count={state.count}
        onCount={(count) => toolChipStore.getState().onCount?.(count)}
        resetKey={state.onCount}
      />
    </InspectorSection>
  );
}

function TransformInspector({ state, validation }: { state: ToolChipState; validation: ToolValueValidation }) {
  return (
    <InspectorSection title="Transform" validation={validation}>
      <SegmentToggle
        options={PATTERN_AXES}
        active={state.axis}
        label="Placement axis"
        testid={(axis) => `chip-axis-${axis.toLowerCase()}`}
        onPick={(axis) => toolChipStore.getState().onAxis?.(axis)}
      />
      <TransformModeSegments
        active={state.transformMode}
        onPick={(mode) => toolChipStore.getState().onTransformMode?.(mode)}
      />
      <CopyToggle copy={state.copy} onToggle={(copy) => toolChipStore.getState().onCopy?.(copy)} />
      <AlignButton phase={state.alignPhase} onStart={() => toolChipStore.getState().onAlign?.()} />
    </InspectorSection>
  );
}

function MirrorInspector({ state, validation }: { state: ToolChipState; validation: ToolValueValidation }) {
  return (
    <InspectorSection title="Mirror" validation={validation}>
      <SegmentToggle
        options={MIRROR_PLANES}
        active={state.plane}
        label="Mirror plane"
        testid={(plane) => `chip-mirror-plane-${plane.toLowerCase()}`}
        onPick={(plane) => toolChipStore.getState().onPlane?.(plane)}
      />
      <FuseToggle fuse={state.fuse} onToggle={(fuse) => toolChipStore.getState().onFuse?.(fuse)} />
    </InspectorSection>
  );
}

function BooleanInspector({ state, validation }: { state: ToolChipState; validation: ToolValueValidation }) {
  return (
    <InspectorSection title="Boolean" validation={validation}>
      {state.onSwap && (
        <button type="button" data-testid="chip-bool-swap" onClick={() => toolChipStore.getState().onSwap?.()}>
          Swap target and tool
        </button>
      )}
      <SegmentToggle
        options={BOOLEAN_OPS}
        active={state.op}
        label="Boolean operation"
        testid={(op) => `chip-boolean-${op.toLowerCase()}`}
        onPick={(op) => toolChipStore.getState().onOp?.(op)}
      />
    </InspectorSection>
  );
}

/** Docked secondary controls for an armed tool. Owns no parameter state. */
export function ActiveToolInspector() {
  const state = useToolChipStore((current) => current);
  const attempt = useOperationAttemptStore((current) => current.attempt);
  const documentId = useDocumentStore((current) => current.documentId);
  // activeToolPresentation reads current names from documentStore; this
  // subscription makes rename/removal facts recompute the read-only summary.
  const bodyFacts = useDocumentStore((current) => current.bodies);
  const sketchFacts = useDocumentStore((current) => current.sketches);
  void bodyFacts;
  void sketchFacts;
  const scopedAttempt = attempt?.documentId === documentId ? attempt : null;
  const presentation = activeToolPresentation(state) ?? scopedAttempt?.presentation ?? null;
  const showTargetSummary = presentation !== null && presentation.tool !== "dimension" && presentation.tool !== "sketchValue";
  const dockRef = useCallback((host: HTMLDivElement | null) => setToolChipDockHost(host), []);
  if (!presentation) return null;

  if (state.kind === "none" || scopedAttempt?.phase === "completed") {
    const terminalSubtitle = scopedAttempt?.phase === "applying"
      ? "Applying…"
      : scopedAttempt?.phase === "failed"
        ? `Failed${scopedAttempt.message ? `: ${scopedAttempt.message}` : ""}`
        : scopedAttempt?.phase === "completed"
          ? `Completed${scopedAttempt.message ? `: ${scopedAttempt.message}` : ""}`
          : "Last operation";
    return (
      <InspectorSection
        title={activeToolLabel(presentation.tool)}
        validation={presentation.validation}
        subtitle={terminalSubtitle}
      >
        {showTargetSummary && <ActiveToolTargets targets={presentation.targets} />}
        {[presentation.primary, ...presentation.secondaries].filter((field) => field !== null).map((field) => (
          <div key={field.id} className="text-[12px] text-ink-3">{field.label}: {formatActiveToolField(field)}</div>
        ))}
      </InspectorSection>
    );
  }

  const validation = presentation.validation;
  let content: ReactNode = null;
  if (state.kind === "extrudeDepth") {
    const subtitle = presentation.phase === "selecting" ? "Select target" : "Preview settings";
    content = <ExtrudeInspector state={state} validation={validation} subtitle={subtitle} />;
  }
  if (state.kind === "hole") content = <HoleInspector validation={validation} />;
  if (state.kind === "filletRadius" || state.kind === "shellThickness" || state.kind === "offsetFace") {
    content = <EdgeOperationInspector state={state} validation={validation} />;
  }
  if (state.kind === "revolveAngle") content = <RevolveInspector state={state} validation={validation} />;
  if (state.kind === "linearPattern" || state.kind === "circularPattern") {
    content = <PatternInspector state={state} validation={validation} />;
  }
  if (state.kind === "transform") content = <TransformInspector state={state} validation={validation} />;
  if (state.kind === "mirror") content = <MirrorInspector state={state} validation={validation} />;
  if (state.kind === "booleanOp") content = <BooleanInspector state={state} validation={validation} />;
  const dockTarget = (
    <div
      ref={dockRef}
      data-testid="tool-chip-dock"
      data-viewport-interactive
      className="mb-3 w-full min-w-0 max-w-full"
    />
  );
  const fitPreview = <FitPreviewButton tool={presentation.tool} phase={presentation.phase} preview={presentation.preview} validationStatus={presentation.validation.status} />;
  if (scopedAttempt?.phase === "applying") {
    return <>{dockTarget}{fitPreview}<div role="status">Applying…</div><fieldset disabled>{content}</fieldset></>;
  }
  if (scopedAttempt?.phase === "failed") {
    return <>{dockTarget}{showTargetSummary && <ActiveToolTargets targets={presentation.targets} />}<div role="status">Failed{scopedAttempt.message ? `: ${scopedAttempt.message}` : ""}</div>{content}</>;
  }
  return <>{dockTarget}{fitPreview}{showTargetSummary && <ActiveToolTargets targets={presentation.targets} />}{content}</>;
}
