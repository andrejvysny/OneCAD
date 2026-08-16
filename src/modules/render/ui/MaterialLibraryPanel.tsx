/*
 * The left-sidebar "Materials" tab — the document's material library.
 *
 * Registered into `Slots.ShellLeft` exactly the way `VariablesPanel` is, and
 * shares that region the same way: every occupant owns the full footprint,
 * renders its own `SidebarTabHeader`, and returns `null` unless
 * `sidebarTabStore` names it. That store is shell chrome (which sidebar tab is
 * showing), not a modeling store — render reads no document state it does not
 * own.
 *
 * TWO GROUPS, and the difference between them is real:
 *   "In this design" — the document's own materials. Editable, deletable,
 *                      countable (how many bodies and faces wear each), and
 *                      DRAGGABLE onto the model.
 *   "Starter materials" — templates (`starterMaterials.ts`), not library
 *                      entries. Clicking one ADDS a fresh copy with a newly
 *                      minted id; the template itself is never assigned.
 *
 * STARTER ROWS ARE NOT DRAGGABLE — click-to-add only. A drag would have to mint
 * the material on `dragstart`, i.e. write it into the document (an undoable
 * transaction) before knowing whether the gesture would even end over the
 * canvas; a drag released over the sidebar would leave an orphan material the
 * user never asked for. Adding first and then dragging the real row is one more
 * click and no surprises.
 *
 * NO "APPLY TO SELECTION" HERE. It would need the current selection, and there
 * is no platform selection service — `InspectorContext.selection` is the only
 * sanctioned read, and it reaches inspector SECTIONS only. Importing modeling's
 * `selectionStore` would be exactly the cross-module reach the architecture
 * forbids, so selection-driven assignment lives in `MaterialInspectorSection`
 * (which is handed the selection) and on the tree row's own context menu, and
 * this panel offers drag-and-drop instead. Recorded as an open seam: a platform
 * selection service would let this row come back.
 *
 * READ-ONLY is a first-class state, not an error. A slice this build cannot
 * parse is preserved verbatim and every write stands down (ADR-0005), so the
 * panel SAYS so and disables its own mutations rather than letting a click
 * silently do nothing.
 */
import { useState } from "react";

import { Icon } from "@/icons/Icon";
import { SectionLabel } from "@/ui/SectionLabel";
import { cn } from "@/ui/cn";
import { SidebarTabHeader } from "@/features/shell/SidebarTabHeader";
import { useSidebarTabStore } from "@/stores/sidebarTabStore";

import { createMaterial, type Color3, type MaterialDef } from "../model/material";
import { renderStore } from "../store/renderStore";
import { renderDialogStore } from "./dialogStore";
import { encodeMaterialDrag } from "./assignDragDrop";
import { colorToCss } from "./materialColor";
import { STARTER_MATERIALS, type MaterialTemplate } from "./starterMaterials";
import { materialQuery, useRenderStore } from "./state";

function Swatch({ color }: { color: Color3 | undefined }) {
  return (
    <span
      aria-hidden="true"
      data-testid="material-swatch"
      className="h-3.5 w-3.5 flex-none rounded-[3px] border border-border-strong"
      style={{ background: colorToCss(color) }}
    />
  );
}

function IconButton({
  label,
  icon,
  onClick,
  disabled,
  testId,
  danger,
}: {
  label: string;
  icon: "penEdit" | "x";
  onClick: () => void;
  disabled?: boolean;
  testId: string;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "cursor-pointer rounded-sm p-0.5 text-ink-5 hover:bg-hover-3",
        danger ? "hover:text-danger-strong" : "hover:text-ink-2",
        "focus-visible:shadow-focus-ring focus-visible:outline-none disabled:opacity-50",
      )}
    >
      <Icon name={icon} size={12} strokeWidth={2.2} />
    </button>
  );
}

function DocumentRow({
  def,
  usage,
  readOnly,
}: {
  def: MaterialDef;
  usage: number;
  readOnly: boolean;
}) {
  const [renaming, setRenaming] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  const commitRename = (next: string) => {
    setRenaming(false);
    const name = next.trim();
    if (name === "" || name === def.name) return;
    void renderStore.getState().upsertMaterial({ ...def, name });
  };

  return (
    <div
      data-testid={`material-row-${def.id}`}
      // The row itself is the drag source (not a handle): the whole row is what
      // the user reads as "this material", and a handle would be a second thing
      // to discover. `draggable` is off in read-only, where an assignment could
      // not be written anyway.
      draggable={!readOnly}
      onDragStart={(e) => {
        if (readOnly) return;
        encodeMaterialDrag(e.dataTransfer, def.id);
      }}
      className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2"
    >
      <Swatch color={def.base.base_color} />
      {renaming ? (
        <input
          autoFocus
          aria-label={`Rename ${def.name}`}
          data-testid={`material-rename-${def.id}`}
          defaultValue={def.name}
          className="min-w-0 flex-1 rounded-sm border border-accent bg-surface px-1 text-[12.5px] text-ink outline-none"
          onBlur={(e) => commitRename(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") setRenaming(false);
          }}
        />
      ) : (
        <button
          type="button"
          title={readOnly ? def.name : `${def.name} — click to rename`}
          disabled={readOnly}
          onClick={() => setRenaming(true)}
          className="min-w-0 flex-1 cursor-text truncate text-left text-[12.5px] text-ink-2 disabled:cursor-default"
        >
          {def.name}
        </button>
      )}
      <span
        data-testid={`material-usage-${def.id}`}
        title={`${usage} assignment${usage === 1 ? "" : "s"}`}
        className="flex-none font-mono text-[11px] text-ink-5"
      >
        {usage}
      </span>
      <IconButton
        label={`Edit ${def.name}`}
        icon="penEdit"
        testId={`material-edit-${def.id}`}
        disabled={readOnly}
        onClick={() => renderDialogStore.getState().openEditor(def.id)}
      />
      {confirmingDelete ? (
        <button
          type="button"
          data-testid={`material-delete-confirm-${def.id}`}
          onClick={() => {
            setConfirmingDelete(false);
            void renderStore.getState().removeMaterial(def.id);
          }}
          onBlur={() => setConfirmingDelete(false)}
          autoFocus
          className="flex-none cursor-pointer rounded-sm px-1 text-[11px] font-medium text-danger-strong"
        >
          Delete?
        </button>
      ) : (
        <IconButton
          label={`Delete ${def.name}`}
          icon="x"
          danger
          testId={`material-delete-${def.id}`}
          disabled={readOnly}
          onClick={() => setConfirmingDelete(true)}
        />
      )}
    </div>
  );
}

function StarterRow({ template, disabled }: { template: MaterialTemplate; disabled: boolean }) {
  const { name, ...layers } = template;
  return (
    <button
      type="button"
      data-testid={`starter-material-${name}`}
      disabled={disabled}
      title={`Add ${name} to this design`}
      onClick={() => void renderStore.getState().upsertMaterial(createMaterial(name, layers))}
      className={cn(
        "mb-1 flex h-[28px] w-full cursor-pointer items-center gap-2 rounded-sm px-2 text-left",
        "hover:bg-hover disabled:cursor-default disabled:opacity-50",
      )}
    >
      <Swatch color={template.base.base_color} />
      <span className="min-w-0 flex-1 truncate text-[12.5px] text-ink-3">{name}</span>
      <span aria-hidden="true" className="flex-none text-[13px] leading-none text-ink-5">
        +
      </span>
    </button>
  );
}

/**
 * "N assignments no longer resolve" — INFORMATIONAL, never blocking.
 *
 * A dangling body assignment and an unbound face override are both reported,
 * never deleted: a body missing from this projection may be back after the next
 * regen, and a face the viewport could not bind in one tessellation is not
 * evidence that the assignment is stale (H5-B — a deterministic "this does not
 * resolve" beats a silent removal).
 */
function UnresolvedNote({
  dangling,
  unbound,
}: {
  dangling: readonly string[];
  unbound: Record<string, readonly string[]>;
}) {
  const unboundCount = Object.values(unbound).reduce((n, ids) => n + ids.length, 0);
  const total = dangling.length + unboundCount;
  if (total === 0) return null;
  return (
    <div
      data-testid="material-unresolved"
      className="mb-2 rounded-sm border border-warn-border bg-warn-surface px-2 py-1.5 text-[11.5px] text-warn"
    >
      {total} assignment{total === 1 ? "" : "s"} no longer resolve
      {total === 1 ? "s" : ""}. They are kept, not removed.
    </div>
  );
}

export function MaterialLibraryPanel() {
  // Read unconditionally (rules of hooks) — only the returned JSX is gated,
  // matching `VariablesPanel`'s own comment on the same mechanism.
  const activeSidebarTab = useSidebarTabStore((s) => s.activeTab);
  const library = useRenderStore((s) => s.state.library);
  const readOnly = useRenderStore((s) => s.readOnly);
  const dangling = useRenderStore((s) => s.danglingBodies);
  const unbound = useRenderStore((s) => s.unboundFaceOverrides);
  // Recomputed on every store change, which is exactly when a count can move.
  const usage = materialQuery.usageCounts();

  if (activeSidebarTab !== "materials") return null;
  const materials = Object.values(library);

  return (
    <div className="absolute bottom-[34px] left-0 top-0 z-20 flex w-[220px] flex-col border-r border-border bg-panel">
      <SidebarTabHeader />
      <div className="min-h-0 flex-1 overflow-auto p-3">
        {readOnly && (
          <div
            data-testid="material-read-only"
            className="mb-2 rounded-sm border border-warn-border bg-warn-surface px-2 py-1.5 text-[11.5px] text-warn"
          >
            This document&rsquo;s materials were written by another version and are preserved
            unread. Nothing here can be changed.
          </div>
        )}

        <UnresolvedNote dangling={dangling} unbound={unbound} />

        <SectionLabel className="pb-1.5">In this design</SectionLabel>
        {materials.length === 0 && (
          <div data-testid="material-library-empty" className="mb-1 text-[12px] text-ink-5">
            No materials yet — add a starter below, then drag it onto a body.
          </div>
        )}
        {materials.map((m) => (
          <DocumentRow key={m.id} def={m} usage={usage[m.id] ?? 0} readOnly={readOnly} />
        ))}

        <SectionLabel className="pb-1.5 pt-4">Starter materials</SectionLabel>
        {STARTER_MATERIALS.map((t) => (
          <StarterRow key={t.name} template={t} disabled={readOnly} />
        ))}
      </div>
    </div>
  );
}
