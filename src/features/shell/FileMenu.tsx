import { useRef, useState } from "react";
import { Popover } from "@/ui/Popover";
import { TitleBarButton } from "./TitleBarButton";
import { RenameDocumentDialog } from "./RenameDocumentDialog";
import { MenuItem } from "@/ui/MenuItem";
import {
  closeProject,
  export3mf,
  exportObj,
  exportStep,
  exportStl,
  importProject,
  insertStep,
  openDocumentDialog,
  saveDocument,
  saveDocumentAs,
} from "./fileActions";

/**
 * Compact File menu in the title bar: Open… / Save / Save As… / Import STEP… /
 * Export STEP… / Export STL… / Export OBJ… / Export 3MF…, each routed through `fileActions`
 * (same path the ⌘O/⌘S/⇧⌘S shortcuts use). Mirrors the start-screen SortMenu pattern
 * (a hairline trigger + anchored Popover) so it reuses the existing primitives +
 * design tokens.
 */
export function FileMenu() {
  const [open, setOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const btn = useRef<HTMLButtonElement | null>(null);

  const run = (action: () => void | Promise<unknown>) => {
    setOpen(false);
    void action();
  };

  return (
    <div data-tauri-drag-region className="relative">
      {/* No `data-tauri-drag-region` here: it makes the OS swallow the press
          into a window drag, and this button would stop opening its menu. */}
      <TitleBarButton
        ref={btn}
        menu
        active={open}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        File
      </TitleBarButton>

      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={btn}
        placement="bottom-start"
        width={190}
        className="p-1"
      >
        <MenuItem label="Open…" shortcut="⌘O" onClick={() => run(openDocumentDialog)} />
        <MenuItem label="Save" shortcut="⌘S" onClick={() => run(saveDocument)} />
        <MenuItem label="Save As…" shortcut="⇧⌘S" onClick={() => run(saveDocumentAs)} />
        {/* Renaming belongs with the other document-identity actions, next to
            Save As… — which is what you reach for when the name has to reach
            the disk (see `RenameDocumentDialog`). */}
        <MenuItem
          label="Rename…"
          data-testid="file-menu-rename"
          onClick={() => {
            setOpen(false);
            setRenaming(true);
          }}
        />
        {/* Import lands in the Open/Save group, ABOVE the Export separator: it
            mutates the open document, so it belongs with the input actions. */}
        <MenuItem label="Import STEP…" onClick={() => run(insertStep)} />
        <MenuItem label="Import Project…" onClick={() => run(importProject)} />
        <div aria-hidden="true" className="my-1 h-px bg-border" />
        <MenuItem label="Close Project" shortcut="⌘W" onClick={() => run(closeProject)} />
        <div aria-hidden="true" className="my-1 h-px bg-border" />
        <MenuItem label="Export STEP…" onClick={() => run(exportStep)} />
        <MenuItem label="Export STL…" onClick={() => run(exportStl)} />
        <MenuItem label="Export OBJ…" onClick={() => run(exportObj)} />
        <MenuItem label="Export 3MF…" onClick={() => run(export3mf)} />
      </Popover>

      <RenameDocumentDialog open={renaming} onClose={() => setRenaming(false)} />
    </div>
  );
}
