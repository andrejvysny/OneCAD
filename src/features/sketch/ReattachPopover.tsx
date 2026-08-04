/*
 * ReattachPopover (H9) — the target picker behind "Reattach…".
 *
 * Lists the three named world planes and every RESOLVED datum. An unresolved
 * datum is shown disabled rather than hidden: it exists in the tree, so silently
 * omitting it reads as a bug, while offering it would send a command the backend
 * refuses (`stamp_datum_plane`).
 *
 * There is deliberately no host-FACE target. A face-hosted sketch's frame is
 * derived from the face by the worker and its projected boundary is expressed in
 * that frame; the frontend can neither author one (SKETCH-ON-FACE W2) nor safely
 * take one away. V1 is world ⇄ datum.
 */
import { Popover } from "@/ui/Popover";
import { MenuItem } from "@/ui/MenuItem";
import { SectionLabel } from "@/ui/SectionLabel";
import { useDocumentStore } from "@/stores/documentStore";
import type { SketchAttachTarget } from "@/ipc/types";
import type { RefObject } from "react";

const WORLD_PLANES = ["XY", "XZ", "YZ"] as const;

export function ReattachPopover({
  open,
  anchorRef,
  onClose,
  onPick,
}: {
  open: boolean;
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  onPick: (target: SketchAttachTarget) => void;
}) {
  const datums = useDocumentStore((s) => s.datums);
  const datumList = Object.values(datums);

  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      placement="bottom-start"
      width={190}
      className="p-1"
    >
      <SectionLabel className="px-2.5 pb-1 pt-1">Reattach to</SectionLabel>
      {WORLD_PLANES.map((plane) => (
        <MenuItem
          key={plane}
          label={`${plane} plane`}
          data-testid={`reattach-world-${plane}`}
          onClick={() => {
            onClose();
            onPick({ kind: "world", plane });
          }}
        />
      ))}
      {datumList.length > 0 && (
        <>
          <div aria-hidden="true" className="my-1 h-px bg-border" />
          {datumList.map((d) => (
            <MenuItem
              key={d.id}
              label={d.resolvedValid ? d.name : `${d.name} (unresolved)`}
              data-testid={`reattach-datum-${d.id}`}
              className={d.resolvedValid ? undefined : "cursor-default text-ink-6 hover:bg-transparent"}
              onClick={() => {
                if (!d.resolvedValid) return;
                onClose();
                onPick({ kind: "datum", datumId: d.id });
              }}
            />
          ))}
        </>
      )}
    </Popover>
  );
}
