import type { ComponentPropsWithoutRef, ReactNode, Ref } from "react";
import { cn } from "./cn";

/**
 * Any other span attribute (`title`, `data-testid`, `role`, the pointer handlers)
 * passes straight through — the history row's inline value editor makes this chip
 * clickable, and re-declaring the mono/tabular classes on a bare span next to it
 * would let the two drift.
 */
type MonoValueProps = Omit<ComponentPropsWithoutRef<"span">, "children"> & {
  children: ReactNode;
  className?: string;
  ref?: Ref<HTMLSpanElement>;
};

/** Monospace numeric/value text (prototype "83.3 mm", "X 273.00", ...). */
export function MonoValue({ children, className, ref, ...rest }: MonoValueProps) {
  return (
    <span
      ref={ref}
      className={cn("font-mono tabular-nums text-ink-4", className)}
      {...rest}
    >
      {children}
    </span>
  );
}
