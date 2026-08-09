import { useEffect, useRef, type MouseEvent as ReactMouseEvent } from "react";
import { cn } from "@/ui/cn";
import { Icon } from "@/icons/Icon";
import { EyeToggle } from "@/ui/EyeToggle";
import { TextInput } from "@/ui/TextInput";
import { MonoValue } from "@/ui/MonoValue";
import type { IconName } from "@/icons/paths";

type TreeRowProps = {
  name: string;
  icon: IconName;
  /** Eye state. Only meaningful alongside `onToggleVisible`. */
  visible?: boolean;
  selected: boolean;
  onSelect: () => void;
  /**
   * Show/hide handler. OMITTED ⇒ the row renders no eye at all — a datum plane
   * has no visibility fact in the document (DATUM W1), and a dead toggle that
   * silently does nothing is worse than no toggle.
   */
  onToggleVisible?: (visible: boolean) => void;
  /**
   * Dim the row without touching its eye: the body is currently isolated AWAY
   * (W3). Transient viewport state, NOT a document fact — the eye keeps showing
   * the persisted `visible` value so the two never look like one thing.
   */
  dimmed?: boolean;
  /**
   * Short trailing value ("500 N", "not installed"). Right-aligned mono, and it
   * survives truncation ahead of the label: a row whose meaning lives in the
   * value must not lose the value first.
   */
  meta?: string;
  /**
   * The row names something wrong (a missing owner, an unresolved reference).
   * Caution treatment, and deliberately NOT the same as `dimmed`: dimmed means
   * "not in play right now", this means "look at me".
   */
  problem?: boolean;
  /** Double-click activator (sketch rows enter sketch mode). */
  onActivate?: () => void;
  /** Right-click → the tree's shared context menu (anchored to this row). */
  onContextMenu?: (e: ReactMouseEvent<HTMLDivElement>) => void;
  /** Inline rename in progress — the label swaps for a focused text field. */
  editing?: boolean;
  /** Enter / blur with a name. The caller validates + commits (empty is refused). */
  onRenameCommit?: (name: string) => void;
  /** Escape — abandon the edit with no write. */
  onRenameCancel?: () => void;
};

/**
 * 32px model-tree row (prototype 1c). Selected = sel-bg + sel-text; hover
 * surface otherwise. The eye is a nested toggle whose click must not bubble
 * into the row's select (prototype stops propagation).
 *
 * `role="option"` + the eye's `role="switch"` are load-bearing: the e2e helpers
 * (`bodyOptions` / `sketchOptions`) address tree rows through them.
 */
export function TreeRow({
  name,
  icon,
  visible = true,
  selected,
  onSelect,
  onToggleVisible,
  dimmed = false,
  meta,
  problem = false,
  onActivate,
  onContextMenu,
  editing = false,
  onRenameCommit,
  onRenameCancel,
}: TreeRowProps) {
  const input = useRef<HTMLInputElement>(null);
  // Guards the blur handler: Enter/Escape both settle the edit and then blur, and a
  // second commit off that blur would either double-write or resurrect a cancelled
  // name. Whichever path settles first wins.
  const settled = useRef(false);

  useEffect(() => {
    if (!editing) {
      settled.current = false;
      return;
    }
    const el = input.current;
    el?.focus();
    el?.select();
  }, [editing]);

  const commit = (): void => {
    if (settled.current) return;
    settled.current = true;
    onRenameCommit?.(input.current?.value ?? name);
  };
  const cancel = (): void => {
    if (settled.current) return;
    settled.current = true;
    onRenameCancel?.();
  };

  return (
    <div
      role="option"
      aria-selected={selected}
      // While editing, the row must not select/activate underneath the field —
      // a stray click inside the input would otherwise enter sketch mode.
      onClick={editing ? undefined : onSelect}
      onDoubleClick={editing ? undefined : onActivate}
      onContextMenu={onContextMenu}
      className={cn(
        "mx-2 my-px flex h-8 cursor-default items-center gap-2 rounded-sm px-2",
        selected
          ? "bg-sel-bg text-sel-text"
          : problem
            ? "text-warn hover:bg-hover-2"
            : "text-tree-label hover:bg-hover-2",
        dimmed && "opacity-50",
      )}
    >
      <Icon name={icon} size={15} strokeWidth={1.6} className="flex-none" />
      {editing ? (
        <TextInput
          ref={input}
          defaultValue={name}
          aria-label={`Rename ${name}`}
          wrapperClassName="flex-1"
          className="h-[24px] text-[13px]"
          onClick={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
          onBlur={commit}
          onKeyDown={(e) => {
            // The tree sits under global shortcuts (Escape cancels the active tool,
            // letters arm tools) — keep every keystroke inside the field.
            e.stopPropagation();
            if (e.key === "Enter") commit();
            else if (e.key === "Escape") cancel();
          }}
        />
      ) : (
        <span className="min-w-0 flex-1 truncate text-[13px]">{name}</span>
      )}
      {!editing && meta && (
        <MonoValue className="flex-none text-[11px] text-ink-6">{meta}</MonoValue>
      )}
      {onToggleVisible && (
        <span
          className="flex"
          onClick={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
        >
          <EyeToggle
            on={visible}
            onChange={onToggleVisible}
            ariaLabel={`Toggle ${name} visibility`}
          />
        </span>
      )}
    </div>
  );
}
