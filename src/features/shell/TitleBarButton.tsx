import type { ButtonHTMLAttributes, ReactNode, Ref } from "react";
import { cn } from "@/ui/cn";
import { Icon } from "@/icons/Icon";
import type { IconName } from "@/icons/paths";

type TitleBarButtonProps = {
  /** Leading glyph. */
  icon?: IconName;
  /** Paint the leading glyph with the accent (the workspace lozenge). */
  iconAccent?: boolean;
  /** Trailing chevron — this button opens a menu. */
  menu?: boolean;
  /** Pressed/expanded treatment (its menu is open, or its mode is on). */
  active?: boolean;
  children?: ReactNode;
  className?: string;
  ref?: Ref<HTMLButtonElement>;
} & Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children" | "className">;

/**
 * THE title-bar control. Every button in the bar is one of these.
 *
 * They used to be three hand-rolled buttons with three geometries — a 26px
 * borderless icon, a 26px borderless text button, a 28px bordered one — which
 * read as three different kinds of control sitting in a row. The design report's
 * first rule is one control geometry everywhere, and the way to keep that true
 * is one component, not three that currently agree.
 *
 * Geometry: 28px tall, 6px radius, transparent until hovered. Borderless is the
 * right default for a title bar: a row of outlined controls competes with the
 * document title, which is the only thing in the bar that should carry weight.
 * A menu button gets a chevron, which is the affordance a border was doing the
 * job of.
 *
 * DRAG REGION. The bar is `data-tauri-drag-region`, and these deliberately are
 * NOT: the attribute makes the OS swallow the press into a window drag, so a
 * button carrying it never fires its own click.
 */
export function TitleBarButton({
  icon,
  iconAccent = false,
  menu = false,
  active = false,
  children,
  className,
  ref,
  ...rest
}: TitleBarButtonProps) {
  const iconOnly = children === undefined;
  return (
    <button
      ref={ref}
      type="button"
      className={cn(
        "flex h-[28px] flex-none cursor-pointer items-center gap-[6px] rounded-sm border-none",
        "font-ui text-[12.5px] font-medium transition-colors",
        "focus-visible:shadow-focus-ring focus-visible:outline-none",
        iconOnly ? "w-[28px] justify-center px-0" : "px-2.5",
        active ? "bg-sel-bg text-sel-text" : "bg-transparent text-ink-3 hover:bg-hover",
        className,
      )}
      {...rest}
    >
      {icon && (
        <Icon
          name={icon}
          size={iconOnly ? 15 : 13}
          strokeWidth={1.8}
          className={cn("flex-none", iconAccent && !active && "text-accent")}
        />
      )}
      {children}
      {menu && (
        <Icon name="chevronDown" size={11} strokeWidth={2} className="flex-none text-ink-5" />
      )}
    </button>
  );
}
