import type { RefObject } from "react";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";

export function NavigationHelp({
  open,
  onClose,
  anchorRef,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
}) {
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      placement="top-start"
      width={236}
      ariaLabel="Navigation help"
      autoFocus="first"
      className="py-1.5"
    >
      <div aria-label="Navigation help" data-testid="navigation-help">
        <SectionLabel className="px-3.5 pb-1 pt-0.5">Navigation</SectionLabel>
        <div className="space-y-2 px-3.5 pb-1 text-[12px] text-ink-3">
          <section aria-label="Mouse controls">
            <div className="font-medium text-ink-2">Mouse</div>
            <ul className="mt-0.5 space-y-0.5">
              <li>Left drag: select or use the active tool</li>
              <li>Wheel: zoom</li>
              <li>Middle or right drag: pan</li>
              <li>Shift + right drag: orbit</li>
            </ul>
          </section>
          <section aria-label="Trackpad controls">
            <div className="font-medium text-ink-2">Trackpad</div>
            <ul className="mt-0.5 space-y-0.5">
              <li>Two-finger scroll: pan</li>
              <li>Shift + two-finger scroll: orbit</li>
              <li>Pinch: zoom</li>
            </ul>
          </section>
        </div>
      </div>
    </Popover>
  );
}
