import type { ToolChipState, ToolValueValidation } from "@/stores/toolChipStore";
import { documentStore } from "@/stores/documentStore";
import { formatLengthWithUnit } from "@/units/format";
import type {
  ActiveToolContext,
  ActiveToolContextFor,
  AuthoredBodyRef,
  AuthoredElementRef,
  ContextToolKind,
} from "./activeToolContext";

export type ActiveToolPreviewState = "none" | "pending" | "valid" | "invalid" | "applying";

export interface ActiveToolField {
  id: string;
  label: string;
  value: number | string | boolean | null;
  unit?: "length" | "angle";
}

/** Read-only projection consumed by both the compact chip and docked inspector. */
export type ActiveToolBodyReference = { kind: "body"; bodyId: string; label: string; resolution: "current" | "missing" };
export type ActiveToolSketchReference = { kind: "sketch"; sketchId: string; label: string; resolution: "current" | "missing" };
export type ActiveToolElementReference = {
      kind: "face" | "edge";
      bodyId: string;
      elementId: string | null;
      label: string;
      resolution: "identity-only" | "unresolved" | "missing";
      /** Measured area/arc length carried by the publisher; absent ⇒ unmeasured. */
      area?: number;
      /** Surface normal carried by the publisher; absent ⇒ unknown. */
      normal?: readonly [number, number, number];
    };
export type ActiveToolReference = ActiveToolBodyReference | ActiveToolSketchReference | ActiveToolElementReference;

/**
 * Which way the tool is currently going, from the LIVE signed value plus the
 * symmetric flag — the arm-time vector is only the basis (UX review 2026-09-14,
 * T5: `Direction: Normal [0, 0, 1]` never moved, through a symmetric toggle or a
 * drag through zero).
 */
export type ActiveToolDirectionSense = "positive" | "negative" | "both";

export type ActiveToolDirectionReference =
  | { kind: "normal"; vector: readonly [number, number, number]; sense: ActiveToolDirectionSense; label: string }
  | { kind: "sketchLine"; lineId: string; sense: ActiveToolDirectionSense; label: string };

type PresentedContext<C extends ActiveToolContext> =
  C extends { kind: "profile" } ? Omit<C, "sketch" | "hostBodies" | "direction"> & {
    sketch: ActiveToolSketchReference;
    hostBodies: ActiveToolBodyReference[];
    direction: ActiveToolDirectionReference | null;
  } :
  C extends { kind: "regions" } ? Omit<C, "sketch"> & { sketch: ActiveToolSketchReference } :
  C extends { kind: "edgeOperation" } ? Omit<C, "affectedBodies" | "edges" | "referenceFaces"> & {
    affectedBodies: ActiveToolBodyReference[];
    edges: ActiveToolElementReference[];
    referenceFaces: { a: ActiveToolElementReference | null; b: ActiveToolElementReference | null }[];
  } :
  C extends { kind: "faces" } ? Omit<C, "affectedBodies" | "faces" | "oppositeFace"> & {
    affectedBodies: ActiveToolBodyReference[];
    faces: ActiveToolElementReference[];
    oppositeFace?: ActiveToolElementReference;
  } :
  C extends { kind: "bodies" } ? Omit<C, "bodies"> & { bodies: ActiveToolBodyReference[] } :
  C extends { kind: "boolean" } ? Omit<C, "target" | "toolBody"> & { target: ActiveToolBodyReference; toolBody: ActiveToolBodyReference } :
  C extends { kind: "gear" } ? Omit<C, "support"> & { support: ActiveToolElementReference | null } :
  C;

export type ActiveToolPresentedContextFor<K extends ContextToolKind> = PresentedContext<ActiveToolContextFor<K>>;

export type ActiveToolIntent =
  | { tool: "extrudeDepth"; kind: "extrude"; operation: ToolChipState["booleanMode"]; extent: ToolChipState["endCondition"]; symmetric: boolean }
  | { tool: "revolveAngle" | "revolveAxisPick"; kind: "revolve"; operation: ToolChipState["booleanMode"] }
  | { tool: "filletRadius"; kind: "edgeOperation"; operation: ToolChipState["edgeOp"] }
  | { tool: "shellThickness"; kind: "shell" }
  | { tool: "offsetFace"; kind: "offsetFace"; distanceType: ToolChipState["distanceType"]; tangent: boolean }
  | { tool: "hole"; kind: "hole"; holeType: ToolChipState["holeType"] }
  | { tool: "linearPattern" | "circularPattern"; kind: "pattern"; axis: ToolChipState["axis"] }
  | { tool: "transform"; kind: "transform"; mode: ToolChipState["transformMode"]; axis: ToolChipState["axis"]; copy: boolean }
  | { tool: "mirror"; kind: "mirror"; plane: ToolChipState["plane"]; fuse: boolean }
  | { tool: "booleanOp"; kind: "boolean"; operation: ToolChipState["op"] }
  | { tool: "datumOffset"; kind: "datum" }
  | { tool: "gear"; kind: "gear" }
  | { tool: "regionSelect"; kind: "regionSelect" }
  | { tool: "dimension" | "sketchValue"; kind: "sketchParameter" };

interface ActiveToolPresentationBase {
  phase: "idle" | "selecting" | "armed" | "applying";
  primary: ActiveToolField | null;
  modes: ActiveToolField[];
  secondaries: ActiveToolField[];
  validation: ToolValueValidation;
  preview: ActiveToolPreviewState;
  canConfirm: boolean;
  canCancel: boolean;
}

type ActiveToolIntentFor<K extends ToolChipState["kind"]> = ActiveToolIntent extends infer I
  ? I extends { tool: infer T }
    ? K extends T ? I : never
    : never
  : never;

type ActiveToolPresentationFor<K extends ToolChipState["kind"]> = ActiveToolPresentationBase & {
  tool: K;
  targets: K extends ContextToolKind ? ActiveToolPresentedContextFor<K> | null : null;
  intent: ActiveToolIntentFor<K>;
};

export type ActiveToolPresentation = {
  [K in ToolChipState["kind"]]: ActiveToolPresentationFor<K>
}[ToolChipState["kind"]];

function pairPresentation<K extends ToolChipState["kind"]>(
  tool: K,
  targets: ActiveToolPresentationFor<K>["targets"],
  intent: ActiveToolPresentationFor<K>["intent"],
  base: ActiveToolPresentationBase,
): ActiveToolPresentationFor<K> {
  return { ...base, tool, targets, intent };
}

const TOOL_LABELS: Partial<Record<ToolChipState["kind"], string>> = {
  extrudeDepth: "Extrude",
  revolveAngle: "Revolve",
  filletRadius: "Edge operation",
  shellThickness: "Shell",
  offsetFace: "Offset face",
  linearPattern: "Linear pattern",
  circularPattern: "Circular pattern",
  booleanOp: "Boolean",
  datumOffset: "Datum plane",
  regionSelect: "Region selection",
  revolveAxisPick: "Revolve axis",
};

export function activeToolLabel(tool: ToolChipState["kind"]): string {
  return TOOL_LABELS[tool] ?? tool.charAt(0).toUpperCase() + tool.slice(1);
}

/**
 * The heading for one presentation — the operation as the user knows it.
 *
 * `activeToolLabel` maps the CHIP KIND, and one chip kind serves two operations:
 * `filletRadius` arms both Fillet and Chamfer, which is why the inspector titled
 * a committed Chamfer "Edge operation" while its history row said "Chamfer" (UX
 * review 2026-09-14, N3). The read-only recap renders after the chip is cleared,
 * so the live `state.edgeOp` is gone by then; the operation survives on the
 * presentation's own intent, which is what this reads.
 */
export function activeToolPresentationTitle(presentation: ActiveToolPresentation): string {
  if (presentation.tool === "filletRadius") return presentation.intent.operation;
  return activeToolLabel(presentation.tool);
}

export function formatActiveToolField(field: ActiveToolField): string {
  if (field.value === null) return "—";
  if (field.unit === "length" && typeof field.value === "number") return formatLengthWithUnit(field.value);
  if (field.unit === "angle" && typeof field.value === "number") return `${field.value}°`;
  if (typeof field.value === "boolean") return field.value ? "Yes" : "No";
  return String(field.value);
}

export function canConfirmActiveTool(s: ToolChipState): boolean {
  return activeToolPresentation(s)?.canConfirm === true;
}

function presentBody(ref: AuthoredBodyRef): ActiveToolBodyReference {
  const body = documentStore.getState().bodies[ref.bodyId];
  return {
    kind: "body",
    bodyId: ref.bodyId,
    label: body?.name ?? `Missing body (${ref.bodyId})`,
    resolution: body ? "current" : "missing",
  };
}

function presentElement(ref: AuthoredElementRef): ActiveToolElementReference {
  // Carried through verbatim, never fetched: the inspector reports the evidence
  // the publisher already had and stays silent otherwise (N11).
  const measured = {
    ...(ref.area !== undefined ? { area: ref.area } : {}),
    ...(ref.normal !== undefined ? { normal: ref.normal } : {}),
  };
  const body = documentStore.getState().bodies[ref.bodyId];
  if (!body) return { kind: ref.kind, bodyId: ref.bodyId, elementId: ref.elementId ?? null, label: `Missing ${ref.kind} body (${ref.bodyId})`, resolution: "missing", ...measured };
  if (!ref.elementId) return { kind: ref.kind, bodyId: ref.bodyId, elementId: null, label: `${body.name} · unresolved ${ref.kind}`, resolution: "unresolved", ...measured };
  return { kind: ref.kind, bodyId: ref.bodyId, elementId: ref.elementId, label: `${body.name} · ${ref.kind} ${ref.elementId}`, resolution: "identity-only", ...measured };
}

/** The word for each sense, as a label prefix. */
const SENSE_PREFIX: Readonly<Record<ActiveToolDirectionSense, string>> = {
  positive: "+",
  negative: "−",
  both: "Both ± ",
};

function presentDirection(
  direction: ActiveToolContextFor<"extrudeDepth">["direction"],
  sense: ActiveToolDirectionSense,
): ActiveToolDirectionReference | null {
  if (!direction) return null;
  const prefix = SENSE_PREFIX[sense];
  // "Normal" leads the label when the sense is signed and follows the "Both ± "
  // phrase when it is not, which is the only reason the noun changes case here.
  const noun = direction.kind === "normal"
    ? `${sense === "both" ? "normal" : "Normal"} [${direction.vector.join(", ")}]`
    : `${sense === "both" ? "sketch line" : "Sketch line"} ${direction.lineId}`;
  return { ...direction, sense, label: `${prefix}${noun}` };
}

function presentSketch(sketchId: string): ActiveToolSketchReference {
  const current = documentStore.getState().sketches[sketchId];
  return {
    kind: "sketch",
    sketchId,
    label: current?.name ?? `Missing sketch (${sketchId})`,
    resolution: current ? "current" : "missing",
  };
}

function presentContext<C extends ActiveToolContext>(context: C, sense: ActiveToolDirectionSense): PresentedContext<C>;
function presentContext(context: ActiveToolContext, sense: ActiveToolDirectionSense): PresentedContext<ActiveToolContext> {
  switch (context.kind) {
    case "profile": return {
      ...context,
      sketch: presentSketch(context.sketch.sketchId),
      hostBodies: context.hostBodies.map(presentBody),
      direction: presentDirection(context.direction, sense),
    };
    case "regions": return { ...context, sketch: presentSketch(context.sketch.sketchId) };
    case "edgeOperation": return {
      ...context,
      affectedBodies: context.affectedBodies.map(presentBody),
      edges: context.edges.map(presentElement),
      referenceFaces: context.referenceFaces.map((pair) => ({
        a: pair.a ? presentElement(pair.a) : null,
        b: pair.b ? presentElement(pair.b) : null,
      })),
    };
    case "faces": {
      const { affectedBodies, faces, oppositeFace, ...identity } = context;
      return {
        ...identity,
        affectedBodies: affectedBodies.map(presentBody),
        faces: faces.map(presentElement),
        ...(oppositeFace ? { oppositeFace: presentElement(oppositeFace) } : {}),
      };
    }
    case "bodies": return { ...context, bodies: context.bodies.map(presentBody) };
    case "boolean": return { ...context, target: presentBody(context.target), toolBody: presentBody(context.toolBody) };
    case "datum": return context;
    case "gear": return { ...context, support: context.support ? presentElement(context.support) : null };
  }
}

function missingRequiredTargetMessage(s: ToolChipState): string | null {
  const context = s.context;
  if (!context || context.tool !== s.kind) {
    return s.kind === "dimension" || s.kind === "sketchValue"
      ? null
      : "Tool targets are unavailable — cancel and reopen the tool";
  }
  const doc = documentStore.getState();
  const bodyMissing = (ref: AuthoredBodyRef): boolean => !doc.bodies[ref.bodyId];
  switch (context.kind) {
    case "profile": {
      if (!doc.sketches[context.sketch.sketchId]) return "Profile sketch is no longer available";
      if (s.booleanMode !== "NewBody" && (context.hostBodies.length === 0 || context.hostBodies.some(bodyMissing))) {
        return "Target body is no longer available";
      }
      return null;
    }
    case "regions": return doc.sketches[context.sketch.sketchId] ? null : "Profile sketch is no longer available";
    case "edgeOperation": return context.affectedBodies.length === 0 || context.affectedBodies.some(bodyMissing) ? "Affected body is no longer available" : null;
    case "faces": return context.affectedBodies.length === 0 || context.affectedBodies.some(bodyMissing) ? "Affected body is no longer available" : null;
    case "bodies": return context.bodies.length === 0 || context.bodies.some(bodyMissing) ? "Source body is no longer available" : null;
    case "boolean": return bodyMissing(context.target) || bodyMissing(context.toolBody) ? "Boolean operand is no longer available" : null;
    case "gear": return context.support && !doc.bodies[context.support.bodyId] ? "Gear support face is no longer available" : null;
    case "datum": return null;
  }
}

function primaryLabel(s: ToolChipState): string {
  if (s.kind === "filletRadius") return s.edgeOp === "Chamfer" ? "Distance" : "Radius";
  if (s.kind === "shellThickness") return "Thickness";
  if (s.kind === "revolveAngle" || s.kind === "circularPattern") return "Angle";
  if (s.kind === "linearPattern") return "Spacing";
  if (s.kind === "hole") return "Hole diameter";
  return s.label || "Value";
}

function primaryUnit(s: ToolChipState): ActiveToolField["unit"] {
  return s.kind === "revolveAngle" || s.kind === "circularPattern" ||
    (s.kind === "transform" && s.transformMode === "rotate") || s.suffix === "°"
    ? "angle"
    : "length";
}

export function activeToolPresentation(s: ToolChipState): ActiveToolPresentation | null {
  if (s.kind === "none") return null;
  const hasPrimary = s.onValue !== null;
  const missingTargetMessage = missingRequiredTargetMessage(s);
  const blocked = s.validation.status !== "valid" || s.retainedCommitFailure !== null || missingTargetMessage !== null;
  const secondaries: ActiveToolField[] = [];
  if (s.kind === "hole") {
    secondaries.push({ id: "depth", label: "Depth", value: s.holeDepth, unit: "length" });
    if (s.holeType === "counterbore") {
      secondaries.push(
        { id: "cbDiameter", label: "Counterbore diameter", value: s.cbDiameter, unit: "length" },
        { id: "cbDepth", label: "Counterbore depth", value: s.cbDepth, unit: "length" },
      );
    }
    if (s.holeType === "countersink") {
      secondaries.push(
        { id: "csDiameter", label: "Countersink diameter", value: s.csDiameter, unit: "length" },
        { id: "csAngle", label: "Countersink angle", value: s.csAngleDeg, unit: "angle" },
      );
    }
  }
  if (s.kind === "filletRadius" && s.edgeOp === "Chamfer") {
    secondaries.push(
      { id: "distance2", label: "Second distance", value: s.distance2, unit: "length" },
      { id: "angle", label: "Chamfer angle", value: s.chamferAngleDeg, unit: "angle" },
    );
  }
  if (s.kind === "extrudeDepth") {
    secondaries.push(
      { id: "draft", label: "Draft angle", value: s.draftAngleDeg, unit: "angle" },
      { id: "symmetric", label: "Symmetric", value: s.symmetric },
      { id: "regions", label: "Regions", value: s.regionCount },
    );
  }
  if (s.kind === "regionSelect") {
    secondaries.push({ id: "regions", label: "Selected regions", value: s.count });
  }
  if (s.kind === "linearPattern" || s.kind === "circularPattern") {
    secondaries.push({ id: "count", label: "Instances", value: s.count });
  }
  if (s.kind === "transform") {
    secondaries.push(
      { id: "copy", label: "Copy", value: s.copy },
      { id: "alignPhase", label: "Align", value: s.alignPhase ?? "off" },
    );
  }
  if (s.kind === "mirror") secondaries.push({ id: "fuse", label: "Fuse", value: s.fuse });
  if (s.kind === "gear") {
    secondaries.push(
      { id: "teeth", label: "Teeth", value: s.count },
      { id: "height", label: "Height", value: s.gearHeight, unit: "length" },
      { id: "pressureAngle", label: "Pressure angle", value: s.gearPressureAngleDeg, unit: "angle" },
      { id: "shift", label: "Profile shift", value: s.gearShift },
      { id: "clearance", label: "Clearance", value: s.gearClearance },
      { id: "backlash", label: "Backlash", value: s.gearBacklash, unit: "length" },
      { id: "undercut", label: "Undercut", value: s.gearUndercut },
    );
  }
  if (s.kind === "offsetFace") {
    secondaries.push({ id: "tangent", label: "Follow tangent faces", value: s.chainTangentFaces });
  }
  const modes: ActiveToolField[] = [];
  if (s.kind === "extrudeDepth" || s.kind === "revolveAngle") {
    modes.push({ id: "booleanMode", label: "Operation", value: s.booleanMode });
    if (s.kind === "extrudeDepth") modes.push({ id: "endCondition", label: "Extent", value: s.endCondition });
  } else if (s.kind === "filletRadius") {
    modes.push({ id: "edgeOp", label: "Operation", value: s.edgeOp });
  } else if (s.kind === "offsetFace") {
    modes.push({ id: "distanceType", label: "Distance type", value: s.distanceType });
  } else if (s.kind === "hole") {
    modes.push({ id: "holeType", label: "Hole type", value: s.holeType });
  } else if (s.kind === "linearPattern" || s.kind === "circularPattern" || s.kind === "transform") {
    modes.push({ id: "axis", label: "Axis", value: s.axis });
    if (s.kind === "transform") modes.push({ id: "transformMode", label: "Transform", value: s.transformMode });
  } else if (s.kind === "mirror") {
    modes.push({ id: "plane", label: "Plane", value: s.plane });
  } else if (s.kind === "booleanOp") {
    modes.push({ id: "operation", label: "Operation", value: s.op });
  }
  const base: ActiveToolPresentationBase = {
    phase: s.previewLifecycle.status === "applying"
      ? "applying"
      : s.kind === "regionSelect" || s.kind === "revolveAxisPick" || s.alignPhase !== null
        ? "selecting"
        : "armed",
    primary: hasPrimary
      ? { id: "primary", label: primaryLabel(s), value: s.value, unit: primaryUnit(s) }
      : null,
    modes,
    secondaries,
    validation: s.retainedCommitFailure
      ? { status: "invalid", draft: s.value, message: s.retainedCommitFailure.message }
      : s.validation.status !== "valid"
        ? s.validation
        : missingTargetMessage
          ? { status: "invalid", draft: s.value, message: missingTargetMessage }
          : s.validation,
    preview: s.previewLifecycle.status,
    canConfirm:
      s.onConfirm !== null &&
      !blocked &&
      s.previewLifecycle.status !== "invalid" &&
      s.previewLifecycle.status !== "applying" &&
      s.kind !== "revolveAxisPick" &&
      (s.kind !== "regionSelect" || s.count > 0),
    canCancel: s.onCancel !== null && s.previewLifecycle.status !== "applying",
  };
  // T5: the live direction sense. The arm-time vector in the context is only the
  // BASIS — a symmetric toggle or a drag through zero changes the direction
  // without ever touching it.
  const sense: ActiveToolDirectionSense = s.symmetric ? "both" : s.value < 0 ? "negative" : "positive";
  switch (s.kind) {
    case "extrudeDepth": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "extrude", operation: s.booleanMode, extent: s.endCondition, symmetric: s.symmetric }, base);
    case "revolveAngle": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "revolve", operation: s.booleanMode }, base);
    case "revolveAxisPick": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "revolve", operation: s.booleanMode }, base);
    case "filletRadius": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "edgeOperation", operation: s.edgeOp }, base);
    case "shellThickness": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "shell" }, base);
    case "offsetFace": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "offsetFace", distanceType: s.distanceType, tangent: s.chainTangentFaces }, base);
    case "linearPattern": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "pattern", axis: s.axis }, base);
    case "circularPattern": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "pattern", axis: s.axis }, base);
    case "booleanOp": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "boolean", operation: s.op }, base);
    case "datumOffset": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "datum" }, base);
    case "regionSelect": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "regionSelect" }, base);
    case "mirror": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "mirror", plane: s.plane, fuse: s.fuse }, base);
    case "transform": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "transform", mode: s.transformMode, axis: s.axis, copy: s.copy }, base);
    case "hole": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "hole", holeType: s.holeType }, base);
    case "gear": return pairPresentation(s.kind, s.context?.tool === s.kind ? presentContext(s.context, sense) : null, { tool: s.kind, kind: "gear" }, base);
    case "dimension": return pairPresentation(s.kind, null, { tool: s.kind, kind: "sketchParameter" }, base);
    case "sketchValue": return pairPresentation(s.kind, null, { tool: s.kind, kind: "sketchParameter" }, base);
  }
}
