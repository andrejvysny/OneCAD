import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { SectionLabel } from "@/ui/SectionLabel";
import { MenuItem } from "@/ui/MenuItem";
import { Popover } from "@/ui/Popover";
import { useDocumentStore } from "@/stores/documentStore";
import { selectionStore, useSelectionStore, type EntityKind } from "@/stores/selectionStore";
import { useToolStore } from "@/stores/toolStore";
import { TreeRow } from "./TreeRow";
import {
  deleteSketch,
  renameBody,
  renameSketch,
  setBodyVisible,
  setSketchVisible,
} from "./treeActions";

/** Which row the shared context menu is currently anchored to. */
type MenuTarget = { kind: "body" | "sketch"; id: string; name: string; visible: boolean };

/**
 * Docked model tree: BODIES + SKETCHES sections driven by the document
 * projection. Click selects; double-clicking a sketch enters sketch mode.
 *
 * The eye and the rename affordance are BACKEND-BACKED (TRUST wave): both route
 * through `treeActions`, which dispatches `SetVisibility` / `RenameBody` /
 * `RenameSketch` and reverts on rejection. A local-only store flip (what this
 * panel used to do) is silently undone by the next projection.
 *
 * Right-click opens a shared context menu anchored to the row; F2 renames the
 * selected row inline.
 */
export function ModelTreePanel() {
  const bodies = useDocumentStore((s) => s.bodies);
  const sketches = useDocumentStore((s) => s.sketches);
  const selected = useSelectionStore((s) => s.selected);
  const select = useSelectionStore((s) => s.set);
  const setMode = useToolStore((s) => s.setMode);

  const [menu, setMenu] = useState<MenuTarget | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const anchor = useRef<HTMLElement | null>(null);

  const isSelected = (kind: EntityKind, id: string) =>
    selected.some((r) => r.kind === kind && r.id === id);

  const closeMenu = useCallback(() => {
    setMenu(null);
    setConfirmDelete(false);
  }, []);

  const openMenu = (target: MenuTarget) => (e: ReactMouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    anchor.current = e.currentTarget;
    setConfirmDelete(false);
    setMenu(target);
    select([{ kind: target.kind, id: target.id }]);
  };

  // F2 renames the selected row. Scoped to a single body/sketch selection so it
  // can never guess which of a multi-select the user meant, and skipped while a
  // field already has focus (the inline rename input itself).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "F2") return;
      const el = document.activeElement;
      if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return;
      const rows = selectionStore
        .getState()
        .selected.filter((r) => r.kind === "body" || r.kind === "sketch");
      if (rows.length !== 1) return;
      e.preventDefault();
      closeMenu();
      setRenaming(rows[0].id);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [closeMenu]);

  const commitRename = (kind: "body" | "sketch", id: string) => (name: string) => {
    setRenaming(null);
    void (kind === "body" ? renameBody(id, name) : renameSketch(id, name));
  };

  return (
    <div className="absolute bottom-[34px] left-0 top-0 z-20 w-[220px] overflow-auto border-r border-border bg-panel pb-1.5">
      <SectionLabel className="px-[14px] pb-1 pt-3">Bodies</SectionLabel>
      <div role="listbox" aria-label="Bodies">
        {Object.values(bodies).map((b) => (
          <TreeRow
            key={b.id}
            name={b.name}
            icon="cube"
            visible={b.visible}
            selected={isSelected("body", b.id)}
            onSelect={() => select([{ kind: "body", id: b.id }])}
            onToggleVisible={(v) => void setBodyVisible(b.id, v)}
            onContextMenu={openMenu({ kind: "body", id: b.id, name: b.name, visible: b.visible })}
            editing={renaming === b.id}
            onRenameCommit={commitRename("body", b.id)}
            onRenameCancel={() => setRenaming(null)}
          />
        ))}
      </div>

      <SectionLabel className="px-[14px] pb-1 pt-3">Sketches</SectionLabel>
      <div role="listbox" aria-label="Sketches">
        {Object.values(sketches).map((s) => (
          <TreeRow
            key={s.id}
            name={s.name}
            icon="pen"
            visible={s.visible}
            selected={isSelected("sketch", s.id)}
            onSelect={() => select([{ kind: "sketch", id: s.id }])}
            onToggleVisible={(v) => void setSketchVisible(s.id, v)}
            onActivate={() => setMode("sketch", s.id)}
            onContextMenu={openMenu({ kind: "sketch", id: s.id, name: s.name, visible: s.visible })}
            editing={renaming === s.id}
            onRenameCommit={commitRename("sketch", s.id)}
            onRenameCancel={() => setRenaming(null)}
          />
        ))}
      </div>

      {menu && (
        // Keyed by the target so re-opening on a DIFFERENT row re-runs the
        // Popover's anchor-measuring effect (it keys off `open`, and the ref
        // object identity never changes).
        <Popover
          key={`${menu.kind}:${menu.id}`}
          open
          onClose={closeMenu}
          anchorRef={anchor}
          placement="bottom-start"
          width={170}
          className="p-1"
        >
          <MenuItem
            label="Rename"
            shortcut="F2"
            data-testid="tree-menu-rename"
            onClick={() => {
              closeMenu();
              setRenaming(menu.id);
            }}
          />
          <MenuItem
            label={menu.visible ? "Hide" : "Show"}
            data-testid="tree-menu-visibility"
            onClick={() => {
              closeMenu();
              void (menu.kind === "body"
                ? setBodyVisible(menu.id, !menu.visible)
                : setSketchVisible(menu.id, !menu.visible));
            }}
          />
          {menu.kind === "sketch" && (
            <>
              <div aria-hidden="true" className="my-1 h-px bg-border" />
              {/* Two-click confirm (the HistoryList delete idiom): destructive and
                  the tree has no other undo affordance in reach. */}
              {confirmDelete ? (
                <MenuItem
                  label="Confirm delete"
                  danger
                  data-testid="tree-menu-delete-confirm"
                  onClick={() => {
                    closeMenu();
                    void deleteSketch(menu.id);
                  }}
                />
              ) : (
                <MenuItem
                  label="Delete sketch"
                  danger
                  data-testid="tree-menu-delete"
                  onClick={() => setConfirmDelete(true)}
                />
              )}
            </>
          )}
        </Popover>
      )}
    </div>
  );
}
