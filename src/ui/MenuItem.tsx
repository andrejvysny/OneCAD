import { cn } from "./cn";
import { MonoValue } from "./MonoValue";

type MenuItemProps = {
  label: string;
  /** Right-aligned mono accelerator hint (e.g. "⌘S"). */
  shortcut?: string;
  onClick: () => void;
  /** Destructive styling (delete / confirm-delete rows). */
  danger?: boolean;
  className?: string;
  "data-testid"?: string;
};

/**
 * 30px row inside an anchored Popover menu. Extracted verbatim from `FileMenu`
 * (which was its only consumer) so the model tree's context menu renders the same
 * chrome — a second hand-rolled copy is how two menus drift apart.
 */
export function MenuItem({
  label,
  shortcut,
  onClick,
  danger,
  className,
  "data-testid": testid,
}: MenuItemProps) {
  return (
    <div
      role="menuitem"
      data-testid={testid}
      onClick={onClick}
      className={cn(
        "flex h-[30px] cursor-pointer items-center gap-2 rounded-[5px] px-2.5 text-[12.5px] hover:bg-well",
        danger ? "text-traffic-close" : "text-ink-2",
        className,
      )}
    >
      <span className="flex-1">{label}</span>
      {shortcut && <MonoValue className="text-[11px] text-ink-6">{shortcut}</MonoValue>}
    </div>
  );
}
