/*
 * ExtrudeChipControls — the individual controls of the extrude cluster, plus the
 * `⋯` overflow that now holds all but the dimension.
 *
 * WHY THE OVERFLOW EXISTS. A fresh extrude arm used to render a dimension, four
 * end-condition segments, a draft toggle, a symmetric toggle and three boolean
 * segments in one pill — a dozen decisions demanded BEFORE the user has seen a
 * result, most of which the drag itself answers. The chip now carries the one
 * thing a drag cannot state exactly (the number) and hides the rest behind `⋯`.
 *
 * The overflow button is also a READOUT: it shows the boolean mode the direction
 * resolved to, because a mode that changes on its own must never change out of
 * sight. Everything else that is non-default raises a dot on the same button.
 *
 * `BooleanModeSegments` is shared with the REVOLVE cluster, which still renders
 * it inline — collapsing happens in the extrude branch, not in these controls.
 */
import { cn } from "@/ui/cn";
import { Tooltip } from "@/ui/Tooltip";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import { ChipOverflow } from "@/features/toolbar/ChipOverflow";
import { toolChipStore } from "@/stores/toolChipStore";
import type { BooleanMode, ExtrudeEndCondition } from "@/tools/modelTools/modelToolMachine";

/** Intersect stays hidden until its full real-worker/browser evidence gate passes. */
const BOOLEAN_MODES: { mode: BooleanMode; label: string; testid: string }[] = [
  { mode: "NewBody", label: "New Body", testid: "chip-bool-newbody" },
  { mode: "Add", label: "Add", testid: "chip-bool-add" },
  { mode: "Cut", label: "Cut", testid: "chip-bool-cut" },
];



/**
 * The armed-extrude end-condition segments (MODEL-OPS W1). `Symmetric` is NOT
 * here — it stays the ⇔ toggle, so there is one control per concept.
 * `ToNext`/`ToFace` need an existing body to reach, so they disable at zero
 * bodies rather than being offered and failing at commit.
 */
const END_CONDITIONS: {
  end: ExtrudeEndCondition;
  label: string;
  testid: string;
  needsBody: boolean;
}[] = [
  { end: "Blind", label: "Blind", testid: "chip-end-blind", needsBody: false },
  { end: "ThroughAll", label: "Through all", testid: "chip-end-throughall", needsBody: true },
  { end: "ToNext", label: "To next", testid: "chip-end-tonext", needsBody: true },
  { end: "ToFace", label: "To face", testid: "chip-end-toface", needsBody: true },
];

export function EndConditionSegments({
  active,
  canUseBodyEnds,
  booleanMode,
  onPick,
}: {
  active: ExtrudeEndCondition;
  canUseBodyEnds: boolean;
  /**
   * `ThroughAll` is a TOOL extent: with `NewBody` there is no body to reach
   * through, and the worker refuses the pair by name
   * (`EXTRUDE_THROUGH_ALL_NO_TARGET`, SCHEMA §7.3) — so the segment is disabled
   * rather than offered and refused at commit.
   */
  booleanMode: BooleanMode;
  onPick: (end: ExtrudeEndCondition) => void;
}) {
  const throughAllNeedsTarget = booleanMode === "NewBody";
  return (
    <div
      className="flex overflow-hidden rounded-full"
      role="group"
      aria-label="End condition"
      title={
        canUseBodyEnds
          ? throughAllNeedsTarget
            ? "Through all needs Add or Cut (a body to reach through)"
            : undefined
          : "Through all / To next / To face need an existing body"
      }
    >
      {END_CONDITIONS.map((c) => {
        const disabled =
          (c.needsBody && !canUseBodyEnds) || (c.end === "ThroughAll" && throughAllNeedsTarget);
        return (
          <button
            key={c.end}
            type="button"
            data-testid={c.testid}
            aria-pressed={c.end === active}
            disabled={disabled}
            onClick={() => onPick(c.end)}
            className={cn(
              "px-2 py-1 text-[11.5px] font-medium",
              c.end === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
              disabled && "cursor-not-allowed opacity-40 hover:bg-chip",
            )}
          >
            {c.label}
          </button>
        );
      })}
    </div>
  );
}

/**
 * The [Draft] segment on the armed extrude cluster (WP-C3). `ExtrudeParams`
 * has carried `draftAngleDeg` end-to-end since W-WP6 (the worker applies it with
 * `BRepOffsetAPI_DraftAngle`), but no UI ever authored one — this is that
 * surface.
 *
 * It stays visible in the inspector: collapsing an invalid field would hide the
 * exact draft text that is blocking confirmation.
 */
export function DraftSegment({
  deg,
  onDeg,
  onConfirm,
}: {
  deg: number;
  onDeg: (deg: number) => void;
  onConfirm: () => void;
}) {
  return (
    <span data-testid="chip-draft-input">
      <DimensionInput
        value={deg}
        suffix="°"
        label="Draft angle (°)"
        onValidityChange={(valid, draft) =>
          toolChipStore.getState().setRawValueValidity("extrude-draft", valid, draft)
        }
        onCommit={onDeg}
        onConfirm={onConfirm}
      />
    </span>
  );
}



/**
 * The ⇔ symmetric toggle on the armed extrude cluster (Alt-drag syncs it).
 *
 * T2 (2026-09-14 review): the glyph alone was read as a direction flip by
 * everyone, and neither the `aria-label` nor the native `title` reached a sighted
 * user — the WebView renders no native tooltip. The word is now on the control,
 * and the hint goes through the app's own {@link Tooltip}. The real direction
 * flip is the separate {@link DirectionFlipButton} (T3).
 */
export function SymmetricToggle({ pressed, onToggle }: { pressed: boolean; onToggle: () => void }) {
  return (
    <Tooltip label="Symmetric (hold Alt while dragging)">
      <button
        type="button"
        data-testid="chip-symmetric"
        aria-label="Symmetric"
        aria-pressed={pressed}
        onClick={onToggle}
        className={cn(
          "flex items-center gap-1 rounded-full px-2 py-1 text-[11.5px] font-medium",
          pressed ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
        )}
      >
        <span aria-hidden="true">⇔</span>
        Symmetric
      </button>
    </Tooltip>
  );
}

/**
 * Reverse the extrude direction (T3).
 *
 * Dragging the depth handle back through zero already reversed the prism, but
 * nothing said so — the discoverable-looking control was the symmetric toggle,
 * and the working one was undocumented. The controller consumes this ONCE into
 * the signed depth (`resolveDepth(raw, {flip:true})`), so the arrow and the
 * number stay in agreement and a later drag through zero is not negated twice.
 *
 * Named "Flip", never "cancel"/"✕": the frozen chip probe in
 * `modelingInteraction.golden.test.tsx` resolves the single cancel control by
 * accessible name.
 */
export function DirectionFlipButton({ onFlip }: { onFlip: () => void }) {
  return (
    <Tooltip label="Reverse the extrude direction">
      <button
        type="button"
        data-testid="chip-flip"
        aria-label="Flip direction"
        onClick={onFlip}
        className="flex items-center gap-1 rounded-full bg-chip px-2 py-1 text-[11.5px] font-medium text-ink-3 hover:bg-hover-2"
      >
        <span aria-hidden="true">⇅</span>
        Flip
      </button>
    </Tooltip>
  );
}

/**
 * The New Body / Add / Cut segment group on the armed extrude/revolve cluster
 * (Wave 2). Disabled (all three) when no existing body can be a boolean target;
 * the disabled group carries the "Needs an existing body" title.
 */
export function BooleanModeSegments({
  active,
  canBoolean,
  onPick,
}: {
  active: BooleanMode;
  canBoolean: boolean;
  onPick: (mode: BooleanMode) => void;
}) {
  return (
    <div
      className="flex overflow-hidden rounded-full"
      role="group"
      aria-label="Boolean mode"
      title={canBoolean ? undefined : "Needs an existing body"}
    >
      {BOOLEAN_MODES.map((b) => (
        <button
          key={b.mode}
          type="button"
          data-testid={b.testid}
          aria-pressed={b.mode === active}
          disabled={!canBoolean && b.mode !== "NewBody"}
          onClick={() => onPick(b.mode)}
          className={cn(
            "px-2 py-1 text-[11.5px] font-medium",
            b.mode === active ? "bg-sel-bg text-sel-text" : "bg-chip text-ink-3 hover:bg-hover-2",
            !canBoolean && b.mode !== "NewBody" && "cursor-not-allowed opacity-40 hover:bg-chip",
          )}
        >
          {b.label}
        </button>
      ))}
    </div>
  );
}


/** Short label for the resolved boolean mode — the collapsed chip's readout. */
const MODE_LABEL: Record<BooleanMode, string> = {
  NewBody: "New",
  Add: "Add",
  Cut: "Cut",
  Intersect: "Intersect",
};

export interface ExtrudeOverflowProps {
  endCondition: ExtrudeEndCondition;
  canUseBodyEnds: boolean;
  showEndConditions: boolean;
  onEndCondition?: (end: ExtrudeEndCondition) => void;
  draftAngleDeg: number;
  showDraft: boolean;
  onDraftAngle?: (deg: number) => void;
  symmetric: boolean;
  showSymmetric: boolean;
  onSymmetric?: (symmetric: boolean) => void;
  onFlip?: () => void;
  booleanMode: BooleanMode;
  canBoolean: boolean;
  showBooleanSegments: boolean;
  onBooleanMode?: (mode: BooleanMode) => void;
  onConfirm?: () => void;
}

/**
 * The `⋯` button plus its popover.
 *
 * DISMISSAL is an outside press or a second click on the button. Esc deliberately
 * still cancels the TOOL: the controller owns Escape from a window listener
 * registered in capture phase at construction time, so nothing mounted later can
 * preempt it, and faking a first-Esc-closes-the-popover rule by racing listener
 * order would be worse than the consistency of Esc always meaning "cancel".
 */
export function ExtrudeOverflow(props: ExtrudeOverflowProps): React.ReactElement {
  // A setting the user cannot see is a setting they cannot undo. The dot says
  // "something in here is not the default" without spelling out what.
  const marked =
    (props.showEndConditions && props.endCondition !== "Blind") ||
    (props.showDraft && props.draftAngleDeg !== 0) ||
    (props.showSymmetric && props.symmetric);

  return (
    <ChipOverflow
      ariaLabel="Extrude options"
      title="End condition, draft, symmetric, boolean mode"
      readoutTestId="chip-mode-readout"
      readout={props.showBooleanSegments ? MODE_LABEL[props.booleanMode] : "⋯"}
      marked={marked}
      // The same token `palette.destructive()` tints the prism with, so the
      // chip and the 3D preview say "Cut" in one colour.
      buttonClassName={props.booleanMode === "Cut" ? "text-traffic-close" : undefined}
    >
      {props.showEndConditions && (
        <EndConditionSegments
          active={props.endCondition}
          canUseBodyEnds={props.canUseBodyEnds}
          booleanMode={props.booleanMode}
          onPick={(end) => props.onEndCondition?.(end)}
        />
      )}
      {props.showBooleanSegments && (
        <BooleanModeSegments
          active={props.booleanMode}
          canBoolean={props.canBoolean}
          onPick={(mode) => props.onBooleanMode?.(mode)}
        />
      )}
      <div className="flex items-center gap-1">
        {props.showDraft && (
          <DraftSegment
            deg={props.draftAngleDeg}
            onDeg={(deg) => props.onDraftAngle?.(deg)}
            onConfirm={() => props.onConfirm?.()}
          />
        )}
        {props.showSymmetric && props.endCondition === "Blind" && (
          <SymmetricToggle
            pressed={props.symmetric}
            onToggle={() => props.onSymmetric?.(!props.symmetric)}
          />
        )}
        {props.onFlip && <DirectionFlipButton onFlip={props.onFlip} />}
      </div>
    </ChipOverflow>
  );
}
