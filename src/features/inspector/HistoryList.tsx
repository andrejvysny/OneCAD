import { useEffect, useRef, useState } from "react";
import { cn } from "@/ui/cn";
import { Icon } from "@/icons/Icon";
import { MonoValue } from "@/ui/MonoValue";
import type { IconName } from "@/icons/paths";
import { IMPORT_STEP_OP_TYPE } from "@/ipc/types";
import type { FeatureKind, FeatureMeta } from "@/stores/documentStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import { formatLength, formatUnitless, lengthSuffix, MM_SUFFIX } from "@/units/format";
import type { LengthUnitId } from "@/units/lengthUnits";

const FEATURE_ICON: Record<FeatureKind, IconName> = {
  sketch: "pen",
  extrude: "extrude",
  revolve: "revolve",
  fillet: "fillet",
  boolean: "boolean",
  shell: "shell",
  linearPattern: "linearPattern",
  circularPattern: "circularPattern",
  mirror: "mirrorBody",
};

/**
 * Icon keyed by the exact authored `opType` (dto.rs `feature_kind` — see
 * op_type()/feature_kind() there for the source strings), not the coarse
 * `FeatureKind` bucket. `FeatureKind` folds Chamfer+Shell into "fillet" and the
 * pattern/mirror ops into "boolean", so a row keyed only on `kind` shows the
 * wrong icon for those (a Chamfer row would show the Fillet icon). Identity
 * entries kept for the un-folded kinds for symmetry / a future direct lookup.
 */
const OPTYPE_ICON: Record<string, IconName> = {
  Sketch: "pen",
  Extrude: "extrude",
  Revolve: "revolve",
  Fillet: "fillet",
  Chamfer: "chamfer",
  Shell: "shell",
  Boolean: "boolean",
  LinearPattern: "linearPattern",
  CircularPattern: "circularPattern",
  MirrorBody: "mirrorBody",
  // A placement also buckets under `kind: "boolean"` (dto.rs feature_kind), so
  // without this entry every Move row would show the boolean glyph.
  TransformBody: "move",
  // A hole buckets under `kind: "boolean"` too (it is a body MODIFIER, not a
  // dress-up op), so it needs its own entry to avoid the boolean glyph.
  Hole: "hole",
  // An import buckets under `kind: "boolean"` in the projection (interim), so
  // without this entry every imported body's row would show the boolean glyph.
  [IMPORT_STEP_OP_TYPE]: "import",
};

/**
 * Inline editing of a row's primary dimension (H3). Supplied per row by the panel,
 * which owns the guards (`featureValueEdit.canEditFeatureValue`) — this component
 * stays presentational and never reaches into the tool/document stores for them.
 */
export interface HistoryValueEdit {
  /** Whether the value may be edited in place right now (all guards passed). */
  editable: boolean;
  /** Commit a new value in the DOCUMENT domain (mm / degrees). */
  onCommit(item: FeatureMeta, value: number): void;
}

/**
 * The row's value, rendered for READING: the primary dimension through the display
 * unit when the projection carries one, else the backend's millimetre-fixed
 * `valueText` verbatim (a Boolean's operation, a placement's Δ, a pattern count —
 * none of which are lengths).
 */
function displayValue(item: FeatureMeta, unit: LengthUnitId): string {
  const v = item.primaryValue;
  if (v === undefined) return item.valueText;
  switch (item.primaryValueKind) {
    case "angle":
      return `${formatUnitless(v)}°`;
    // A hole's row is Ø-PREFIXED rather than unit-suffixed (dto.rs), so it cannot
    // be misread as a length; the number itself is still unit-aware.
    case "diameter":
      return `Ø${formatLength(v, unit)}`;
    default:
      return `${formatLength(v, unit)} ${lengthSuffix(unit)}`;
  }
}

/**
 * How long a value click waits before it opens the editor.
 *
 * A row's DOUBLE-click is the established "full re-edit" gesture, and the browser
 * only fires `dblclick` when both clicks share a target — so swapping the value chip
 * for an input on the first click silently kills it (the hole/step-import re-edit
 * specs caught exactly that). Deferring the swap keeps both clicks on the chip; the
 * second one cancels the pending open and the row re-edits instead. Roughly the
 * platform double-click threshold, which is the shortest delay that is still safe.
 */
const VALUE_EDIT_OPEN_MS = 220;

/** Per-row history affordances (M4b): suppress toggle · roll-to-here · delete. */
export interface HistoryRowActions {
  /** Whether this feature is suppressed — dims the row + icon. Sourced from the
   *  PROJECTION (`FeatureMeta.suppressed`), never a frontend overlay. */
  suppressed: boolean;
  onToggleSuppress: (item: FeatureMeta) => void;
  onRoll: (item: FeatureMeta) => void;
  onDelete: (item: FeatureMeta) => void;
}

/** 32px history chip (prototype 1c). Selected feature = sel-bg + sel-text. */
function FeatureRow({
  item,
  selected,
  onSelect,
  onEdit,
  actions,
  valueEdit,
}: {
  item: FeatureMeta;
  selected: boolean;
  onSelect?: (id: string) => void;
  onEdit?: (item: FeatureMeta) => void;
  actions?: HistoryRowActions;
  valueEdit?: HistoryValueEdit;
}) {
  const interactive = Boolean(onSelect || onEdit);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [editingValue, setEditingValue] = useState(false);
  // The ONE subscription that re-renders every history value on a unit switch.
  const unit = useSettingsStore((s) => s.displayUnit);
  const suppressed = actions?.suppressed ?? false;
  const editable = valueEdit?.editable ?? false;
  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cancelPendingOpen = () => {
    if (openTimer.current === null) return;
    clearTimeout(openTimer.current);
    openTimer.current = null;
  };
  // An OPEN editor closes the moment its row stops being editable — arming a model
  // tool mid-edit must not leave a live field that could commit underneath the
  // tool's own gesture. Checked in the render below too, so the field never paints
  // one extra frame before this lands.
  useEffect(() => {
    if (!editable) setEditingValue(false);
  }, [editable]);
  useEffect(() => () => cancelPendingOpen(), []);
  // An errored feature (regen failure) tints red + tooltips the worker reason
  // (MODEL-HARDEN W0.5). Selection styling still wins so a selected row stays legible.
  const isError = item.status === "error";

  return (
    <div
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      data-testid={`history-row-${item.id}`}
      title={isError ? item.statusMessage : undefined}
      onClick={() => onSelect?.(item.id)}
      onDoubleClick={() => onEdit?.(item)}
      className={cn(
        "group relative mb-1 flex h-8 items-center gap-2 rounded-sm px-2.5",
        interactive && "cursor-pointer",
        selected ? "bg-sel-bg" : "bg-chip hover:bg-hover-2",
        suppressed && "opacity-60",
      )}
    >
      <Icon
        name={OPTYPE_ICON[item.opType ?? ""] ?? FEATURE_ICON[item.kind]}
        size={14}
        strokeWidth={1.7}
        className={selected ? "text-sel-text" : isError ? "text-traffic-close" : "text-ink-4"}
      />
      <span
        className={cn(
          "flex-1 text-[12.5px]",
          selected ? "text-sel-text" : isError ? "text-traffic-close" : "text-ink-2",
          suppressed && "line-through",
        )}
      >
        {item.label}
      </span>

      {/* The value is ALWAYS rendered — it used to disappear the moment a row grew
          its affordance cluster, which hid the one number the row is about on
          exactly the (full-timeline) view where a user goes to change it. The
          cluster now sits BESIDE it. */}
      {editingValue && editable && item.primaryValue !== undefined ? (
        /* Clicks inside the field must not select the row — but a DOUBLE-click is
           deliberately left to bubble: the row's dblclick re-edit is an established
           gesture, and swallowing it here would make it die wherever the value chip
           happens to sit under the pointer. The second click lands in the field,
           the row arms the full editor, and the tool gate then closes this field. */
        <span onClick={(e) => e.stopPropagation()}>
          <DimensionInput
            value={item.primaryValue}
            /* An angle marks its domain with "°"; a length passes the mm literal,
               which DimensionInput swaps for the live display unit (and which is
               what makes it parse bare input in that unit). */
            suffix={item.primaryValueKind === "angle" ? "°" : MM_SUFFIX}
            autoFocus
            onCommit={(v) => {
              setEditingValue(false);
              valueEdit?.onCommit(item, v);
            }}
            onCancel={() => setEditingValue(false)}
          />
        </span>
      ) : editable ? (
        <MonoValue
          /* A click opens the editor AND still selects the row — the click is NOT
             swallowed. Stopping it here carved a dead zone out of the middle of the
             row (the chip sits near its centre once the affordance cluster reserves
             its width), so a click that happened to land on the value silently
             failed to select: the regression two committed specs caught. */
          role="button"
          tabIndex={0}
          data-testid={`history-value-${item.id}`}
          title="Edit value"
          onClick={(e) => {
            // The SECOND click of a double-click cancels the pending open and lets
            // the row's re-edit win — see VALUE_EDIT_OPEN_MS.
            cancelPendingOpen();
            if (e.detail > 1) return;
            openTimer.current = setTimeout(() => {
              openTimer.current = null;
              setEditingValue(true);
            }, VALUE_EDIT_OPEN_MS);
          }}
          onKeyDown={(e) => {
            // Keyboard activation carries no double-click ambiguity — open at once.
            if (e.key !== "Enter" && e.key !== " ") return;
            e.stopPropagation();
            e.preventDefault();
            cancelPendingOpen();
            setEditingValue(true);
          }}
          className={cn(
            "cursor-text rounded-sm px-1 text-[11.5px] hover:bg-hover-3",
            selected ? "text-sel-text" : "text-ink-4",
          )}
        >
          {displayValue(item, unit)}
        </MonoValue>
      ) : (
        displayValue(item, unit) && (
          <MonoValue
            data-testid={`history-value-${item.id}`}
            className={cn("text-[11.5px]", selected ? "text-sel-text" : "text-ink-4")}
          >
            {displayValue(item, unit)}
          </MonoValue>
        )
      )}

      {actions && (
        <div
          // Revealed on row hover; the suppress toggle stays visible when active so
          // a suppressed row keeps its "dimmed + icon" signal (design §5.1).
          className={cn(
            "flex items-center gap-0.5",
            suppressed ? "opacity-100" : "opacity-0 group-hover:opacity-100",
          )}
          onClick={(e) => e.stopPropagation()}
        >
          {/* Discoverable twin of the row double-click: the full re-edit (tool
              session + viewport gesture), as opposed to the value chip's
              one-number commit. Hidden for a row with no parametric editor. */}
          {onEdit && (
            <RowIconButton
              testid={`history-edit-${item.id}`}
              icon="penEdit"
              title="Edit feature"
              onClick={() => {
                setConfirmingDelete(false);
                onEdit(item);
              }}
            />
          )}
          <RowIconButton
            testid={`history-suppress-${item.id}`}
            icon="eye"
            title={suppressed ? "Unsuppress" : "Suppress"}
            active={suppressed}
            onClick={() => {
              setConfirmingDelete(false);
              actions.onToggleSuppress(item);
            }}
          />
          <RowIconButton
            testid={`history-roll-${item.id}`}
            icon="clock"
            title="Roll to here"
            onClick={() => {
              setConfirmingDelete(false);
              actions.onRoll(item);
            }}
          />
          {confirmingDelete ? (
            <RowIconButton
              testid={`history-delete-confirm-${item.id}`}
              icon="check"
              title="Confirm delete"
              danger
              onClick={() => {
                setConfirmingDelete(false);
                actions.onDelete(item);
              }}
            />
          ) : (
            <RowIconButton
              testid={`history-delete-${item.id}`}
              icon="x"
              title="Delete"
              onClick={() => setConfirmingDelete(true)}
            />
          )}
        </div>
      )}
    </div>
  );
}

/** A tiny 20px icon button used in the history-row affordance cluster. */
function RowIconButton({
  testid,
  icon,
  title,
  onClick,
  active,
  danger,
}: {
  testid: string;
  icon: IconName;
  title: string;
  onClick: () => void;
  active?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      data-testid={testid}
      title={title}
      aria-label={title}
      onClick={onClick}
      className={cn(
        "flex h-5 w-5 items-center justify-center rounded-sm hover:bg-hover-3",
        danger ? "text-traffic-close" : active ? "text-warn" : "text-ink-5",
      )}
    >
      <Icon name={icon} size={13} strokeWidth={1.8} />
    </button>
  );
}

/**
 * Feature-timeline chips for the inspector SELECTION state. Now LIVE: fed from
 * documentStore.features. Click selects a feature; double-clicking an editable
 * feature re-enters its drag edit (parametric-edit seed). When `rowActions` is
 * provided (full-timeline view) each row grows hover affordances: edit, suppress,
 * roll-to-here, delete (M4b) — and, with `valueEdit`, an inline editor on the
 * row's primary dimension (H3).
 */
export function HistoryList({
  items,
  selectedId,
  onSelect,
  onEdit,
  rowActions,
  valueEdit,
}: {
  items: FeatureMeta[];
  selectedId?: string;
  onSelect?: (id: string) => void;
  onEdit?: (item: FeatureMeta) => void;
  /** Builds the per-row affordances for a feature (omit to hide the menu). */
  rowActions?: (item: FeatureMeta) => HistoryRowActions;
  /**
   * Builds the inline value editor for a feature at `index` **within `items`**.
   * The index is passed through because the rollback-cursor gate is positional;
   * callers that render a SLICE must resolve the global index themselves.
   */
  valueEdit?: (item: FeatureMeta, index: number) => HistoryValueEdit;
}) {
  return (
    <div>
      {items.map((f, i) => (
        <FeatureRow
          key={f.id}
          item={f}
          selected={f.id === selectedId}
          onSelect={onSelect}
          onEdit={onEdit}
          actions={rowActions?.(f)}
          valueEdit={valueEdit?.(f, i)}
        />
      ))}
    </div>
  );
}
