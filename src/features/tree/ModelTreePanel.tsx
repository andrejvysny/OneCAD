import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { SectionLabel } from "@/ui/SectionLabel";
import { MenuItem } from "@/ui/MenuItem";
import { Popover } from "@/ui/Popover";
import { useDocumentStore } from "@/stores/documentStore";
import { selectionStore, useSelectionStore } from "@/stores/selectionStore";
import { useViewportStore } from "@/stores/viewportStore";
import { usePlatform, useRegistryEntries, type TreeNode } from "@/platform";
import type { IconName } from "@/icons/paths";
import { TreeRow } from "./TreeRow";
import { ReattachPopover } from "@/features/sketch/ReattachPopover";
import { reattachSketch } from "@/features/sketch/reattachActions";
import { Button } from "@/ui/Button";
import { Icon } from "@/icons/Icon";
import { SettingsModal } from "@/features/settings/SettingsModal";
import { deleteDatum, deleteSketch } from "./treeActions";

/**
 * Docked model tree: BODIES + SKETCHES + DATUMS sections driven by the document
 * projection. Click selects; double-clicking a sketch enters sketch mode, and
 * double-clicking a DATUM starts a new sketch hosted on it (the selection is set
 * first, then `setMode("sketch")` — SketchController's `tryEnterOnSelectedDatum`
 * picks the selection up, exactly as the face path works).
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
  const platform = usePlatform();
  const providers = useRegistryEntries(platform.tree);
  // These four subscriptions exist SO THAT the provider's `sections()` reads are
  // fresh: the provider is a projection over `getState()`, and the host owns
  // re-rendering (see TreeProvider's contract). They are load-bearing even
  // though nothing below reads their values directly.
  useDocumentStore((s) => s.bodies);
  useDocumentStore((s) => s.sketches);
  useDocumentStore((s) => s.datums);
  useSelectionStore((s) => s.selected);
  useViewportStore((s) => s.isolatedBodyIds);
  const sketches = useDocumentStore((s) => s.sketches);

  const sections = providers.flatMap((p) => p.sections());
  const nodeOf = (id: string) => sections.flatMap((s) => s.nodes).find((n) => n.id === id);

  /** Which row the shared context menu is anchored to. Re-resolved each render:
   *  node objects are rebuilt by the provider on every projection. */
  const [menu, setMenu] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // H9 reattach: the sketch whose target picker is open. A SECOND popover, opened
  // from the same row anchor after the context menu closes.
  const [reattaching, setReattaching] = useState<string | null>(null);
  const anchor = useRef<HTMLElement | null>(null);

  const menuNode = menu === null ? null : (nodeOf(menu) ?? null);

  const closeMenu = useCallback(() => {
    setMenu(null);
    setConfirmDelete(false);
  }, []);

  const openMenu = (node: TreeNode) => (e: ReactMouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    anchor.current = e.currentTarget;
    setConfirmDelete(false);
    setMenu(node.id);
    node.select();
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

  const commitRename = (node: TreeNode) => (name: string) => {
    setRenaming(null);
    node.rename?.(name);
  };

  return (
    <div className="absolute bottom-[34px] left-0 top-0 z-20 flex w-[220px] flex-col border-r border-border bg-panel">
      <div className="min-h-0 flex-1 overflow-auto pb-1.5">
        {sections.map((section) => (
          <div key={section.id}>
            <SectionLabel className="px-[14px] pb-1 pt-3">{section.title}</SectionLabel>
            <div role="listbox" aria-label={section.title}>
              {section.nodes.map((node) => (
                <TreeRow
                  key={node.id}
                  name={node.label}
                  icon={node.icon as IconName}
                  // Absent `visible` ⇒ the row has no visibility fact ⇒ no eye.
                  visible={node.visible}
                  dimmed={node.dimmed}
                  selected={node.selected}
                  onSelect={node.select}
                  onToggleVisible={node.toggleVisible}
                  onActivate={node.activate}
                  onContextMenu={openMenu(node)}
                  editing={renaming === node.id}
                  onRenameCommit={node.rename ? commitRename(node) : undefined}
                  onRenameCancel={() => setRenaming(null)}
                />
              ))}
            </div>
          </div>
        ))}
      </div>

      <div className="border-t border-border p-2">
        <Button
          variant="ghost"
          size="md"
          className="w-full justify-start"
          aria-label="Open settings"
          onClick={() => setSettingsOpen(true)}
        >
          <Icon name="settings" size={14} strokeWidth={1.8} />
          Settings
        </Button>
      </div>

      {menuNode && (
        // Keyed by the target so re-opening on a DIFFERENT row re-runs the
        // Popover's anchor-measuring effect (it keys off `open`, and the ref
        // object identity never changes).
        <Popover
          key={`${menuNode.kind}:${menuNode.id}`}
          open
          onClose={closeMenu}
          anchorRef={anchor}
          placement="bottom-start"
          width={170}
          className="p-1"
        >
          {/* Rename and Hide/Show are offered when the NODE declares them.
              DATUM W1 ships no RenameDatum command and a datum has no visibility
              fact, so its node supplies neither and the items disappear. */}
          {menuNode.rename && (
            <MenuItem
              label="Rename"
              shortcut="F2"
              data-testid="tree-menu-rename"
              onClick={() => {
                closeMenu();
                setRenaming(menuNode.id);
              }}
            />
          )}
          {menuNode.toggleVisible && (
            <MenuItem
              label={menuNode.visible ? "Hide" : "Show"}
              data-testid="tree-menu-visibility"
              onClick={() => {
                closeMenu();
                menuNode.toggleVisible?.(!menuNode.visible);
              }}
            />
          )}
          {menuNode.kind === "datum" && (
            // Two-click confirm, same idiom as the sketch delete below. The
            // backend refuses while a sketch is hosted on the datum; treeActions
            // surfaces that rejection as a sticky hint.
            <>
              {confirmDelete ? (
                <MenuItem
                  label="Confirm delete"
                  danger
                  data-testid="tree-menu-datum-delete-confirm"
                  onClick={() => {
                    closeMenu();
                    void deleteDatum(menuNode.id);
                  }}
                />
              ) : (
                <MenuItem
                  label="Delete datum"
                  danger
                  data-testid="tree-menu-datum-delete"
                  onClick={() => setConfirmDelete(true)}
                />
              )}
            </>
          )}
          {menuNode.kind === "sketch" && (
            <>
              {/* H9 REATTACH. Offered only for a sketch that is NOT face-hosted:
                  a face-hosted sketch's frame comes from the face and its
                  projected boundary is expressed in it, so moving it to a world
                  plane / datum would silently reinterpret that boundary with
                  nothing to re-project it (`reattachSketch` docs). */}
              {!sketches[menuNode.id]?.hostFace && (
                <MenuItem
                  label="Reattach…"
                  data-testid="tree-menu-reattach"
                  onClick={() => {
                    const id = menuNode.id;
                    closeMenu();
                    setReattaching(id);
                  }}
                />
              )}
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
                    void deleteSketch(menuNode.id);
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

      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}
