import { useEffect, useReducer, useRef, useState } from "react";
import { Icon } from "@/icons/Icon";
import { toolbarIcon } from "@/features/toolbar/ToolButton";
import { Popover } from "@/ui/Popover";
import { MonoValue } from "@/ui/MonoValue";
import { cn } from "@/ui/cn";
import { usePlatform, useRegistryEntries, type ToolAvailability } from "@/platform";
import { useModelingToolContext } from "@/modules/modeling/selectionContext";
import { TitleBarButton } from "./TitleBarButton";

const ALWAYS_ENABLED: ToolAvailability = { enabled: true };

/**
 * Title-bar "Generators ▾" menu: parametric SHAPE generators (gears today,
 * more later — springs, threads, whatever else mints a body from a math core
 * rather than a sketch) that don't belong in the general modeling toolbar
 * alongside Extrude/Fillet/Combine.
 *
 * Registry-driven, mirroring `WorkspaceSwitcher`'s relationship to
 * `platform.workspaces`: this file reads `platform.tools` filtered to
 * `ToolDefinition.generatorsMenu === true` and knows nothing about gears
 * specifically. A future generator (a different recipe, or a whole different
 * module) appears here the moment its own `ModelToolDescriptor` sets that
 * flag — no edit to this file, same as a new workspace needs no edit to
 * `WorkspaceSwitcher`.
 *
 * NOT scoped to the current mode/scope the way `FloatingToolbar` is: the
 * title bar is stable chrome, not per-mode chrome, and every generator here
 * mints a body (model-mode-only), so activating one auto-switches mode via
 * `activateTool`'s own AUTO-MODE handling (`platform.toolHost.activate`) —
 * the row does not need to hide itself first.
 */
export function GeneratorsMenu() {
  const platform = usePlatform();
  const tools = useRegistryEntries(platform.tools);
  const context = useModelingToolContext();
  const [open, setOpen] = useState(false);
  const btn = useRef<HTMLButtonElement | null>(null);

  const entries = tools.filter((t) => t.generatorsMenu === true);

  // Mirrors `FloatingToolbar`'s own re-render-on-availability-change wiring —
  // `canActivate` is a plain predicate read at render time, so an owner whose
  // answer can move on its own must announce it via `subscribe`.
  const [, bump] = useReducer((n: number) => n + 1, 0);
  useEffect(() => {
    const subs = entries.map((t) => t.subscribe?.(bump)).filter((d) => d !== undefined);
    return () => {
      for (const s of subs) s.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entries.map((t) => t.id).join(",")]);

  // No catalog ⇒ no menu — same "an empty add-on group renders nothing"
  // philosophy `WorkspaceSwitcher` already applies, one level up: with zero
  // generators registered there is nothing here to open at all.
  if (entries.length === 0) return null;

  return (
    <div data-tauri-drag-region className="relative">
      <TitleBarButton
        ref={btn}
        menu
        active={open}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        data-testid="generators-menu-trigger"
      >
        Generators
      </TitleBarButton>

      <Popover open={open} onClose={() => setOpen(false)} anchorRef={btn} placement="bottom-start" width={200} className="p-1">
        <div role="menu" aria-label="Generators">
          {entries.map((def) => {
            const verdict = def.canActivate?.(context) ?? ALWAYS_ENABLED;
            const disabled = !verdict.enabled;
            return (
              <div
                key={def.id}
                role="menuitem"
                aria-disabled={disabled}
                data-testid={`generators-menu-${def.id}`}
                title={disabled ? verdict.reason : undefined}
                onClick={() => {
                  if (disabled) return;
                  setOpen(false);
                  void platform.toolHost.activate(def.id, context);
                }}
                className={cn(
                  "flex h-[30px] items-center gap-2 rounded-[5px] px-2.5 text-[12.5px]",
                  disabled ? "cursor-not-allowed text-ink-5 opacity-40" : "cursor-pointer text-ink-2 hover:bg-well",
                )}
              >
                <Icon name={toolbarIcon(def.icon)} size={15} strokeWidth={1.7} className="flex-none" />
                <span className="flex-1 truncate">{def.title}</span>
                {def.shortcutLabel && (
                  <MonoValue className="text-[11px] text-ink-6">{def.shortcutLabel}</MonoValue>
                )}
              </div>
            );
          })}
        </div>
      </Popover>
    </div>
  );
}
