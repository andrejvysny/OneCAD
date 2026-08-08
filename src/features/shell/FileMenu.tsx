import { useRef, useState } from "react";
import { Icon } from "@/icons/Icon";
import { Popover } from "@/ui/Popover";
import { MenuItem } from "@/ui/MenuItem";
import {
  closeProject,
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
 * Export STEP… / Export STL… / Export OBJ…, each routed through `fileActions`
 * (same path the ⌘O/⌘S/⇧⌘S shortcuts use). Mirrors the start-screen SortMenu pattern
 * (a hairline trigger + anchored Popover) so it reuses the existing primitives +
 * design tokens.
 */
export function FileMenu() {
  const [open, setOpen] = useState(false);
  const btn = useRef<HTMLButtonElement | null>(null);

  const run = (action: () => void | Promise<unknown>) => {
    setOpen(false);
    void action();
  };

  return (
    <div data-tauri-drag-region className="relative">
      <button
        ref={btn}
        type="button"
        data-tauri-drag-region
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        className="flex h-[26px] cursor-pointer items-center gap-1 rounded-sm px-2 font-ui text-[12.5px] font-medium text-ink-3 hover:bg-hover"
      >
        File
        <Icon name="chevronDown" size={11} strokeWidth={2} className="text-ink-5" />
      </button>

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
      </Popover>
    </div>
  );
}
