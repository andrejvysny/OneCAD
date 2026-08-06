import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { cn } from "@/ui/cn";
import { ICON_MONO, Icon } from "@/icons/Icon";
import { MenuItem } from "@/ui/MenuItem";
import { Popover } from "@/ui/Popover";
import { MonoValue } from "@/ui/MonoValue";
import type { IconName } from "@/icons/paths";
import { IMPORT_STEP_OP_TYPE } from "@/ipc/types";
import type { FeatureDependencies } from "@/ipc/types";
import type { FeatureKind, FeatureMeta } from "@/stores/documentStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import { ReattachPopover } from "@/features/sketch/ReattachPopover";
import { reattachSketch, sketchIdOfFeature } from "@/features/sketch/reattachActions";
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
  // OffsetFace folds into the Fillet/Chamfer/Shell dress-up bucket (`dto.rs
  // feature_kind`), so without this entry every offset row would show the fillet
  // glyph. `pushpull` is the toolbar's own glyph for the tool.
  OffsetFace: "pushpull",
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
  /**
   * Read-only dependency counts (H10) behind suppress/delete — fetched ONCE per
   * row/menu-open, never per render (see the row's `ensureDeps` and the
   * Popover's `openMenu`). Omit to skip the dependent-count hint entirely.
   */
  getDependents?: (item: FeatureMeta) => Promise<FeatureDependencies>;
}

/** " — N dependent(s)" for a non-empty downstream set, else "". */
function depSuffix(downstream: string[] | undefined): string {
  const n = downstream?.length ?? 0;
  return n > 0 ? ` — ${n} dependent${n === 1 ? "" : "s"}` : "";
}

/**
 * How a row reads, derived from the timeline's FAILURE state (H7b).
 *
 *  - `error`  — the op itself failed in the worker (red, as before);
 *  - `repair` — the op needs a reference repair (warn + a "⚠" glyph);
 *  - `stale`  — a DIRTY row sitting after the halt point: it was never reached,
 *               so it is not "about to rebuild", it is blocked. A dirty row
 *               BEFORE the halt is an ordinary mid-regen row and stays normal;
 *  - `normal` — everything else.
 */
export type HistoryRowTone = "normal" | "error" | "repair" | "stale";

/**
 * The index the timeline STOPPED at — the first `error`/`needsRepair` row — or
 * `-1` when nothing halted it. The whole failure-visibility layer (row tone here,
 * the `TimelineStoppedBanner` sentence) derives from this ONE rule so the row a
 * user is sent to is always the row that is tinted.
 */
export function haltIndexOf(items: FeatureMeta[]): number {
  return items.findIndex((f) => f.status === "error" || f.status === "needsRepair");
}

/** The tone of one row given the timeline's halt point ({@link haltIndexOf}). */
export function rowTone(item: FeatureMeta, index: number, haltIndex: number): HistoryRowTone {
  if (item.status === "error") return "error";
  if (item.status === "needsRepair") return "repair";
  if (item.status === "dirty" && haltIndex >= 0 && index > haltIndex) return "stale";
  return "normal";
}

/** Row label/icon color for a tone (selection styling wins over all of them). */
const TONE_TEXT: Record<HistoryRowTone, string> = {
  normal: "text-ink-2",
  error: "text-traffic-close",
  repair: "text-warn",
  stale: "text-ink-6",
};
/*
 * `normal` and `stale` are neutral inks, so they keep the icon two-tone. The
 * other two paint it a state hue and must collapse the accent with it — see
 * ICON_MONO.
 */
const TONE_ICON: Record<HistoryRowTone, string> = {
  normal: "text-ink-4",
  error: `text-traffic-close ${ICON_MONO}`,
  repair: `text-warn ${ICON_MONO}`,
  stale: "text-ink-6",
};

/** The row's tooltip: the failure reason if any, else the rollback explanation. */
function rowTitle(item: FeatureMeta, tone: HistoryRowTone, applied: boolean): string | undefined {
  if (tone === "error") return item.statusMessage;
  if (tone === "repair") return item.statusMessage ?? "Needs repair — a reference could not be resolved";
  if (!applied) return "Not applied — beyond the rollback bar";
  if (tone === "stale") return "Not rebuilt — the timeline stopped earlier";
  return undefined;
}

/** 32px history chip (prototype 1c). Selected feature = sel-bg + sel-text. */
function FeatureRow({
  item,
  selected,
  applied,
  tone,
  onSelect,
  onEdit,
  onContextMenu,
  actions,
  valueEdit,
}: {
  item: FeatureMeta;
  selected: boolean;
  /** `false` ⇒ the row sits beyond the rollback bar (grayed + italic). */
  applied: boolean;
  tone: HistoryRowTone;
  onSelect?: (id: string) => void;
  onEdit?: (item: FeatureMeta) => void;
  onContextMenu?: (e: ReactMouseEvent<HTMLDivElement>) => void;
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
  // H10 dependency counts behind the suppress/delete affordances — fetched ONCE
  // (the ref guards it), on the first hover of the cluster (or the × click as a
  // fallback for a pointer that never hovers), never per render.
  const [deps, setDeps] = useState<FeatureDependencies | null>(null);
  const depsRequested = useRef(false);
  const ensureDeps = () => {
    if (depsRequested.current || !actions?.getDependents) return;
    depsRequested.current = true;
    void actions.getDependents(item).then(setDeps, () => {});
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
  const isError = tone === "error";

  return (
    <div
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      data-testid={`history-row-${item.id}`}
      data-applied={applied ? "true" : "false"}
      data-tone={tone}
      title={rowTitle(item, tone, applied)}
      onClick={() => onSelect?.(item.id)}
      onDoubleClick={() => onEdit?.(item)}
      onContextMenu={onContextMenu}
      className={cn(
        "group relative mb-1 flex h-8 items-center gap-2 rounded-sm px-2.5",
        interactive && "cursor-pointer",
        selected ? "bg-sel-bg" : "bg-chip hover:bg-hover-2",
        suppressed && "opacity-60",
        // Beyond the rollback bar: the row describes an op the document is NOT
        // currently built from, so it reads as a draft rather than as history.
        !applied && "italic opacity-50",
      )}
    >
      <Icon
        name={OPTYPE_ICON[item.opType ?? ""] ?? FEATURE_ICON[item.kind]}
        size={14}
        strokeWidth={1.7}
        className={selected ? `text-sel-text ${ICON_MONO}` : TONE_ICON[tone]}
      />
      <span
        className={cn(
          "flex-1 text-[12.5px]",
          selected ? "text-sel-text" : TONE_TEXT[tone],
          suppressed && "line-through",
        )}
      >
        {item.label}
      </span>
      {tone === "repair" && (
        <span
          data-testid={`history-repair-glyph-${item.id}`}
          title="Needs repair"
          aria-label="Needs repair"
          className={cn("text-[12px] leading-none", selected ? "text-sel-text" : "text-warn")}
        >
          ⚠
        </span>
      )}

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
          // Revealed on row hover; PINNED visible for the three rows a user needs to
          // act on without hunting: a suppressed row (keeps its "dimmed + icon"
          // signal, design §5.1), the SELECTED row (the one the inspector is about —
          // hover-only affordances on it are undiscoverable on a trackpad), and an
          // ERRORED row, whose suppress button is the one-click way to unblock the
          // rest of the rebuild.
          className={cn(
            "flex items-center gap-0.5",
            suppressed || selected || isError ? "opacity-100" : "opacity-0 group-hover:opacity-100",
          )}
          onClick={(e) => e.stopPropagation()}
          // H10: the cluster becoming interactive is the "menu open" moment for the
          // dependent-count hint — fetched once, not on every render.
          onMouseEnter={ensureDeps}
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
            // On the halting row this button IS the recovery action, so it says what
            // it will accomplish rather than just naming the state change; either way
            // a non-zero downstream count rides along as a hint (H10).
            title={
              suppressed
                ? "Unsuppress"
                : isError
                  ? "Suppress to continue rebuild"
                  : `Suppress${depSuffix(deps?.downstream)}`
            }
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
              title={`Confirm delete${depSuffix(deps?.downstream)}`}
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
              onClick={() => {
                // A pointer that clicks straight through without hovering first (a
                // fast double-click, a touch tap) still gets the count once it
                // resolves — `ensureDeps` is idempotent.
                ensureDeps();
                setConfirmingDelete(true);
              }}
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
        danger
          ? `text-traffic-close ${ICON_MONO}`
          : active
            ? `text-warn ${ICON_MONO}`
            : "text-ink-5",
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
 * roll-to-here, delete (M4b) — a RIGHT-CLICK on the row opens the same set as a
 * menu (H7b) — and, with `valueEdit`, an inline editor on the row's primary
 * dimension (H3).
 *
 * `appliedOps` turns on the ROLLBACK CURSOR rendering (grayed rows + marker +
 * banner). It is POSITIONAL over `items`, so only a caller rendering the FULL
 * lineage may pass it; a slice view (`SelectionState`) must leave it undefined or
 * the bar would land at an index that means nothing.
 */
export function HistoryList({
  items,
  selectedId,
  onSelect,
  onEdit,
  rowActions,
  valueEdit,
  appliedOps,
  onRollToEnd,
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
  /** The timeline cursor, in `items` indices. Omit on a slice view (see above). */
  appliedOps?: number;
  /** Restore the whole timeline (`rollToIndex(total - 1)`); powers the banner +
   *  the menu's "Roll to end". Omit ⇒ neither is offered. */
  onRollToEnd?: () => void;
}) {
  const [menu, setMenu] = useState<FeatureMeta | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  // H10: the dependent counts behind the menu's Suppress/Delete items — fetched
  // ONCE per menu OPEN, not per render.
  const [menuDeps, setMenuDeps] = useState<FeatureDependencies | null>(null);
  // H9 reattach: the SKETCH id behind the Sketch row whose target picker is open
  // (a feature id is a record id — `sketchIdOfFeature` resolves the link).
  const [reattaching, setReattaching] = useState<string | null>(null);
  const anchor = useRef<HTMLElement | null>(null);
  const closeMenu = useCallback(() => {
    setMenu(null);
    setConfirmDelete(false);
    setMenuDeps(null);
  }, []);

  const rolledBack = appliedOps !== undefined && appliedOps < items.length;
  const haltIndex = haltIndexOf(items);
  // Same shape as ModelTreePanel's tree menu: right-click anchors to the row, and
  // SELECTS it first so the menu can never act on a row other than the visible one.
  const openMenu = (item: FeatureMeta) => (e: ReactMouseEvent<HTMLDivElement>) => {
    if (!rowActions) return;
    e.preventDefault();
    anchor.current = e.currentTarget;
    setConfirmDelete(false);
    setMenu(item);
    setMenuDeps(null);
    const getDependents = rowActions(item).getDependents;
    if (getDependents) void getDependents(item).then(setMenuDeps, () => {});
    onSelect?.(item.id);
  };
  const menuActions = menu ? rowActions?.(menu) : undefined;

  return (
    <div>
      {rolledBack && onRollToEnd && (
        <div
          data-testid="history-rollback-banner"
          className="mb-2 flex items-center gap-2 rounded-md border border-warn-border bg-warn-surface px-2.5 py-1.5"
        >
          <span className="flex-1 text-[11.5px] leading-tight text-warn">
            {rolledBackLabel(items.length - (appliedOps ?? 0))}
          </span>
          <button
            type="button"
            data-testid="history-roll-to-end"
            onClick={onRollToEnd}
            className="rounded-sm px-1.5 py-0.5 text-[11.5px] font-medium text-warn hover:bg-hover-3"
          >
            Roll to end
          </button>
        </div>
      )}
      {items.map((f, i) => (
        <div key={f.id}>
          {/* The bar itself: drawn BEFORE the first unapplied row, i.e. between
              `appliedOps - 1` and `appliedOps`. */}
          {rolledBack && i === appliedOps && (
            <div data-testid="history-rollback-marker" className="mb-1 flex items-center gap-1.5">
              <span aria-hidden="true" className="h-px flex-1 bg-warn" />
              <span className="text-[10px] uppercase tracking-wide text-warn">rolled back</span>
              <span aria-hidden="true" className="h-px flex-1 bg-warn" />
            </div>
          )}
          <FeatureRow
            item={f}
            selected={f.id === selectedId}
            applied={appliedOps === undefined || i < appliedOps}
            tone={rowTone(f, i, haltIndex)}
            onSelect={onSelect}
            onEdit={onEdit}
            onContextMenu={openMenu(f)}
            actions={rowActions?.(f)}
            valueEdit={valueEdit?.(f, i)}
          />
        </div>
      ))}

      {menu && menuActions && (
        // Keyed by the row so re-opening on a DIFFERENT one re-runs the Popover's
        // anchor-measuring effect (it keys off `open`; the ref identity is stable).
        <Popover
          key={menu.id}
          open
          onClose={closeMenu}
          anchorRef={anchor}
          placement="bottom-end"
          width={170}
          className="p-1"
        >
          {onEdit && (
            <MenuItem
              label="Edit…"
              data-testid="history-menu-edit"
              onClick={() => {
                closeMenu();
                onEdit(menu);
              }}
            />
          )}
          {/* H9 REATTACH — Sketch rows only. Host-face sketches are excluded by
              the same rule the tree uses; the check lives in the ACTION path
              (the projection's feature row carries no attachment), so a
              face-hosted sketch simply resolves no reattachable id. */}
          {menu.kind === "sketch" && (
            <MenuItem
              label="Reattach…"
              data-testid="history-menu-reattach"
              onClick={() => {
                const id = menu.id;
                closeMenu();
                void sketchIdOfFeature(id).then((sketchId) => {
                  if (sketchId) setReattaching(sketchId);
                });
              }}
            />
          )}
          <MenuItem
            label="Roll to here"
            data-testid="history-menu-roll-here"
            onClick={() => {
              closeMenu();
              menuActions.onRoll(menu);
            }}
          />
          {rolledBack && onRollToEnd && (
            <MenuItem
              label="Roll to end"
              data-testid="history-menu-roll-end"
              onClick={() => {
                closeMenu();
                onRollToEnd();
              }}
            />
          )}
          <MenuItem
            label={
              menuActions.suppressed
                ? "Unsuppress"
                : `Suppress${depSuffix(menuDeps?.downstream)}`
            }
            data-testid="history-menu-suppress"
            onClick={() => {
              closeMenu();
              menuActions.onToggleSuppress(menu);
            }}
          />
          <div aria-hidden="true" className="my-1 h-px bg-border" />
          {/* Two-click confirm — the house idiom for a destructive menu row
              (ModelTreePanel's sketch/datum delete, the row's own × button). */}
          {confirmDelete ? (
            <MenuItem
              label={`Confirm delete${depSuffix(menuDeps?.downstream)}`}
              danger
              data-testid="history-menu-delete-confirm"
              onClick={() => {
                closeMenu();
                menuActions.onDelete(menu);
              }}
            />
          ) : (
            <MenuItem
              label={`Delete${depSuffix(menuDeps?.downstream)}`}
              danger
              data-testid="history-menu-delete"
              onClick={() => setConfirmDelete(true)}
            />
          )}
        </Popover>
      )}

      {reattaching && (
        <ReattachPopover
          open
          anchorRef={anchor}
          onClose={() => setReattaching(null)}
          onPick={(target) => {
            const id = reattaching;
            setReattaching(null);
            void reattachSketch(id, target);
          }}
        />
      )}
    </div>
  );
}

/** "N operations rolled back" (singular-aware). */
function rolledBackLabel(n: number): string {
  return `${n} operation${n === 1 ? "" : "s"} rolled back`;
}
