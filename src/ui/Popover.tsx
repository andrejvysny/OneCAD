import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { cn } from "./cn";

type Placement = "bottom-start" | "bottom-end" | "left-start" | "top-start";
type AutoFocus = "first" | "none";

type PopoverProps = {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  /** Element the popover is anchored to (for positioning + outside-click). */
  anchorRef?: RefObject<HTMLElement | null>;
  /** Panel width in px (prototype snap popover = 238). */
  width?: number;
  /** Show the right-pointing caret (prototype snap popover). */
  caret?: boolean;
  placement?: Placement;
  className?: string;
  /** Accessible dialog name. Existing callers retain the generic fallback. */
  ariaLabel?: string;
  /** Move keyboard-open focus into this popover; opt in for interactive dialogs. */
  autoFocus?: AutoFocus;
};

type Pos = { left: number; top: number };
type Layout = { pos: Pos; maxHeight: number; width: number };

const VIEWPORT_MARGIN = 8;
const PANEL_GAP = 6;

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max));
}

function popupWidth(width: number): number {
  return Math.min(width, Math.max(0, window.innerWidth - VIEWPORT_MARGIN * 2));
}

function preferredPosition(
  anchor: DOMRect,
  panelHeight: number,
  width: number,
  placement: Placement,
): Pos {
  if (placement === "left-start") return { left: anchor.left - width - 12, top: anchor.top };
  if (placement === "top-start") return { left: anchor.left, top: anchor.top - panelHeight - PANEL_GAP };
  if (placement === "bottom-end") return { left: anchor.right - width, top: anchor.bottom + PANEL_GAP };
  return { left: anchor.left, top: anchor.bottom + PANEL_GAP };
}

function layoutFor(
  anchor: HTMLElement | null,
  panel: HTMLDivElement | null,
  width: number,
  placement: Placement,
): Layout {
  const maxHeight = Math.max(0, window.innerHeight - VIEWPORT_MARGIN * 2);
  const nextWidth = popupWidth(width);
  if (!anchor || !panel) {
    return { pos: { left: VIEWPORT_MARGIN, top: VIEWPORT_MARGIN }, maxHeight, width: nextWidth };
  }
  const panelHeight = Math.min(panel.scrollHeight || panel.offsetHeight, maxHeight);
  const preferred = preferredPosition(anchor.getBoundingClientRect(), panelHeight, nextWidth, placement);
  return {
    pos: {
      left: clamp(preferred.left, VIEWPORT_MARGIN, window.innerWidth - nextWidth - VIEWPORT_MARGIN),
      top: clamp(preferred.top, VIEWPORT_MARGIN, window.innerHeight - panelHeight - VIEWPORT_MARGIN),
    },
    maxHeight,
    width: nextWidth,
  };
}

function focusFirst(panel: HTMLDivElement): void {
  if (panel.contains(document.activeElement)) return;
  const target = panel.querySelector<HTMLElement>("[autofocus]") ?? panel.querySelector<HTMLElement>(
    "button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex='-1'])",
  );
  (target ?? panel).focus();
}

function usePopoverLayout(
  open: boolean,
  anchorRef: PopoverProps["anchorRef"],
  panel: RefObject<HTMLDivElement | null>,
  width: number,
  placement: Placement,
  autoFocus: AutoFocus,
): Layout {
  const focusedForOpen = useRef(false);
  const [layout, setLayout] = useState<Layout>({
    pos: { left: VIEWPORT_MARGIN, top: VIEWPORT_MARGIN },
    maxHeight: 0,
    width,
  });

  useLayoutEffect(() => {
    if (!open) {
      focusedForOpen.current = false;
      return;
    }
    const anchor = anchorRef?.current ?? null;
    const update = () => {
      const next = layoutFor(anchor, panel.current, width, placement);
      setLayout((current) => (
        current.width === next.width
        && current.maxHeight === next.maxHeight
        && current.pos.left === next.pos.left
        && current.pos.top === next.pos.top
          ? current
          : next
      ));
    };
    update();
    if (autoFocus === "first" && panel.current && !focusedForOpen.current) {
      focusFirst(panel.current);
      focusedForOpen.current = true;
    }
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    if (panel.current) observer?.observe(panel.current);
    if (anchor) observer?.observe(anchor);
    window.addEventListener("resize", update);
    window.visualViewport?.addEventListener("resize", update);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", update);
      window.visualViewport?.removeEventListener("resize", update);
    };
  }, [anchorRef, autoFocus, open, placement, width]);

  return layout;
}

function usePopoverDismiss(
  open: boolean,
  onClose: () => void,
  anchorRef: PopoverProps["anchorRef"],
  panel: RefObject<HTMLDivElement | null>,
) {
  useEffect(() => {
    if (!open) return;
    const anchor = anchorRef?.current ?? null;
    const closeForEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      const target = event.target as Node;
      if (!panel.current?.contains(target) && !anchor?.contains(target)) return;
      event.preventDefault();
      event.stopPropagation();
      onClose();
      anchor?.focus();
    };
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (panel.current?.contains(target) || anchor?.contains(target)) return;
      onClose();
    };
    document.addEventListener("keydown", closeForEscape);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      document.removeEventListener("keydown", closeForEscape);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [anchorRef, onClose, open]);
}

function useAnchorKeyboardScope(
  open: boolean,
  anchorRef: PopoverProps["anchorRef"],
): void {
  useLayoutEffect(() => {
    if (!open) return;
    const anchor = anchorRef?.current;
    if (!anchor) return;
    const hadScope = anchor.hasAttribute("data-cad-keyboard-scope");
    const previousScope = anchor.getAttribute("data-cad-keyboard-scope");
    if (!hadScope) anchor.setAttribute("data-cad-keyboard-scope", "");
    return () => {
      if (!hadScope) anchor.removeAttribute("data-cad-keyboard-scope");
      else if (previousScope !== null) anchor.setAttribute("data-cad-keyboard-scope", previousScope);
    };
  }, [anchorRef, open]);
}

type PopoverPanelProps = Pick<PopoverProps, "anchorRef" | "ariaLabel" | "caret" | "children" | "className" | "onClose"> & {
  layout: Layout;
  panel: RefObject<HTMLDivElement | null>;
};

function PopoverPanel({
  anchorRef,
  ariaLabel,
  caret,
  children,
  className,
  layout,
  onClose,
  panel,
}: PopoverPanelProps) {
  const closeForEscape = () => {
    onClose();
    anchorRef?.current?.focus();
  };
  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    closeForEscape();
  };

  return createPortal(
    <div
      ref={panel}
      role="dialog"
      aria-label={ariaLabel}
      tabIndex={-1}
      data-cad-keyboard-scope
      data-viewport-interactive
      onKeyDown={onKeyDown}
      style={{
        position: "fixed",
        left: layout.pos.left,
        top: layout.pos.top,
        width: layout.width,
        maxWidth: `calc(100vw - ${VIEWPORT_MARGIN * 2}px)`,
        maxHeight: layout.maxHeight,
        overflowY: "auto",
      }}
      className={cn(
        "z-[100] rounded-md border border-border bg-surface font-ui shadow-popover",
        className,
      )}
    >
      {caret && (
        <span
          aria-hidden="true"
          className="absolute -right-[6px] top-4 h-2.5 w-2.5 rotate-45 border-r border-t border-border bg-surface"
        />
      )}
      {children}
    </div>,
    document.body,
  );
}

/** Anchored floating dialog. Portals into document.body with fixed positioning. */
export function Popover({
  open,
  onClose,
  children,
  anchorRef,
  width = 238,
  caret = false,
  placement = "bottom-start",
  className,
  ariaLabel = "Popover",
  autoFocus = "none",
}: PopoverProps) {
  const panel = useRef<HTMLDivElement>(null);
  const layout = usePopoverLayout(open, anchorRef, panel, width, placement, autoFocus);
  useAnchorKeyboardScope(open, anchorRef);
  usePopoverDismiss(open, onClose, anchorRef, panel);
  if (!open) return null;
  return (
    <PopoverPanel
      anchorRef={anchorRef}
      ariaLabel={ariaLabel}
      caret={caret}
      className={className}
      layout={layout}
      onClose={onClose}
      panel={panel}
    >
      {children}
    </PopoverPanel>
  );
}
