import { useEffect, useMemo, useRef } from "react";
import { Icon } from "@/icons/Icon";
import { MonoValue } from "@/ui/MonoValue";
import { cn } from "@/ui/cn";
import { usePlatform, useRegistryEntries } from "@/platform";
import { useModelingToolContext } from "@/modules/modeling/selectionContext";
import { usePaletteStore } from "@/stores/paletteStore";
import { workspaceStore } from "@/stores/workspaceStore";
import { buildPaletteItems, filterPaletteItems } from "./paletteItems";

/**
 * ⌘K command palette (prototype 2a).
 *
 * One search field over every registered command, tool and workspace. The list
 * is rebuilt from the registries on each open rather than cached, so a
 * contribution registered a moment ago is findable — see `paletteItems.ts` for
 * why there is no palette registry of its own.
 *
 * Keyboard is the whole point: ↑/↓ move, Enter runs, Esc closes. A DISABLED
 * item cannot be run and says why instead of disappearing.
 */
export function CommandPalette() {
  const open = usePaletteStore((s) => s.open);
  const query = usePaletteStore((s) => s.query);
  const index = usePaletteStore((s) => s.index);
  const setQuery = usePaletteStore((s) => s.setQuery);
  const setIndex = usePaletteStore((s) => s.setIndex);
  const hide = usePaletteStore((s) => s.hide);

  const platform = usePlatform();
  // Subscriptions, not one-shot reads: a module registering while the palette
  // is open must appear in it.
  const commands = useRegistryEntries(platform.commands);
  const tools = useRegistryEntries(platform.tools);
  const workspaces = useRegistryEntries(platform.workspaces);
  const toolContext = useModelingToolContext();

  const items = useMemo(
    () =>
      buildPaletteItems({
        platform,
        commandContext: toolContext,
        toolContext,
        activateWorkspace: (ws) => workspaceStore.getState().setActive(ws.id),
      }),
    // The registry snapshots ARE the subscription; `platform` is stable.
    [platform, toolContext, commands, tools, workspaces],
  );

  const results = useMemo(() => filterPaletteItems(items, query), [items, query]);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  if (!open) return null;

  const run = (i: number) => {
    const item = results[i];
    if (!item?.enabled) return;
    hide();
    item.run();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      hide();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setIndex(results.length === 0 ? 0 : (index + 1) % results.length);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setIndex(results.length === 0 ? 0 : (index - 1 + results.length) % results.length);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      run(index);
    }
  };

  return (
    <div className="absolute inset-0 z-[90] flex justify-center bg-scrim" onClick={hide}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        data-testid="command-palette"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
        className="mt-[76px] h-fit w-[560px] overflow-hidden rounded-lg border border-border bg-surface shadow-popover"
      >
        <div className="flex h-11 items-center gap-2.5 border-b border-border-subtle px-3.5">
          <Icon name="search" size={15} strokeWidth={1.8} className="flex-none text-ink-6" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search commands, tools, workspaces…"
            aria-label="Search commands, tools, workspaces"
            data-testid="command-palette-input"
            className="flex-1 border-none bg-transparent font-ui text-[13.5px] text-ink outline-none placeholder:text-ink-6"
          />
          <MonoValue className="rounded border border-border px-[5px] py-0.5 text-[10.5px] text-ink-6">
            esc
          </MonoValue>
        </div>

        <div className="max-h-[320px] overflow-auto p-[5px]" role="listbox">
          {results.length === 0 && (
            <div className="px-2.5 py-6 text-center text-[12.5px] text-ink-6">
              Nothing matches “{query}”.
            </div>
          )}
          {results.map((item, i) => (
            <div
              key={`${item.kind}:${item.id}`}
              role="option"
              aria-selected={i === index}
              aria-disabled={!item.enabled}
              data-testid={`palette-item-${item.id}`}
              onMouseEnter={() => setIndex(i)}
              onClick={() => run(i)}
              className={cn(
                "flex h-10 items-center gap-2.5 rounded-sm px-2.5",
                item.enabled ? "cursor-pointer" : "cursor-default opacity-55",
                i === index && "bg-well",
              )}
            >
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline gap-2">
                  <span className="truncate text-[13px] font-medium text-ink">{item.title}</span>
                  {!item.enabled && item.reason && (
                    <span className="flex-none text-[11px] text-warn">{item.reason}</span>
                  )}
                </div>
                <div className="mt-px truncate text-[11px] text-ink-6">{item.source}</div>
              </div>
              {item.shortcut && (
                <MonoValue className="flex-none rounded border border-border bg-panel px-1.5 py-0.5 text-[11px] text-ink-4">
                  {item.shortcut}
                </MonoValue>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
