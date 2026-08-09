import { useEffect, useMemo, useReducer } from "react";
import { cn } from "@/ui/cn";
import { useToolStore } from "@/stores/toolStore";
import {
  usePlatform,
  useRegistryEntries,
  useActiveToolId,
  type ToolAvailability,
  type ToolDefinition,
} from "@/platform";
import { toolbarScopeToken } from "@/modules/modeling/registryToolbar";
import { ToolButton, toolbarIcon } from "./ToolButton";

const ALWAYS_ENABLED: ToolAvailability = { enabled: true };

/**
 * Centered floating tool pill (prototype 1c). Swaps its tool set with the mode
 * and tints its background in sketch mode (toolbar-sketch token).
 *
 * WHAT IT RENDERS IS THE `ToolDefinition`, not a projection of it into a modeling
 * `Tool` literal. That projection is why a registered tool used to vanish here
 * unless modeling's reverse map knew its id, which made "the toolbar is a set of
 * contributions" true of the registry and false of the screen. Presentation
 * (`title`, `icon`, `shortcutLabel`), placement (`group` boundary ⇒ separator,
 * `priority` ⇒ order), availability (`canActivate`) and activation
 * (`platform.toolHost`) now all come off the definition.
 *
 * Membership is by SCOPE: a tool declaring the active scope's token appears, and
 * a tool declaring NO scopes appears in every scope — the same "empty ⇒ always"
 * rule `CommandDefinition.scopes` states. Dropping the unscoped case (as the old
 * projection did) would have made a zero-knowledge contribution impossible.
 *
 * A tool is never gated on its own `canActivate` while it is ACTIVE: a tool like
 * Boolean changes the selection with its own arm handshake and would otherwise
 * gray itself out mid-gesture.
 *
 * The registry is read live — `useRegistryEntries` is the subscription — and the
 * filter is memoized off that snapshot, because a fresh array per render would
 * loop `useSyncExternalStore` if it were the snapshot itself.
 */
export function FloatingToolbar() {
  const mode = useToolStore((s) => s.mode);
  const platform = usePlatform();
  const tools = useRegistryEntries(platform.tools);
  const activeId = useActiveToolId();

  const token = toolbarScopeToken(mode);
  const entries = useMemo(
    () =>
      tools.filter(
        (t) => t.scopes === undefined || t.scopes.length === 0 || t.scopes.includes(token),
      ),
    // `tools` is the subscription: a registry change gives a new snapshot.
    [tools, token],
  );

  // `canActivate` is a predicate read during render, so the owner has to say when
  // its answer moved. Re-rendering is all this needs to do — the predicates are
  // re-read below. Without it the buttons would keep their mount-time verdict.
  const [, bump] = useReducer((n: number) => n + 1, 0);
  useEffect(() => {
    const subs = entries.map((t) => t.subscribe?.(bump)).filter((d) => d !== undefined);
    return () => {
      for (const s of subs) s.dispose();
    };
  }, [entries]);

  return (
    <div
      role="toolbar"
      aria-label="Tools"
      className={cn(
        "absolute left-1/2 top-3 z-30 flex -translate-x-1/2 items-center gap-0.5",
        "rounded-lg border border-border p-1 shadow-card",
        mode === "sketch" ? "bg-toolbar-sketch" : "bg-surface",
      )}
    >
      {entries.map((def, i) => (
        <ToolEntry
          key={def.id}
          def={def}
          active={def.id === activeId}
          separated={i > 0 && def.group !== entries[i - 1]?.group}
          onPick={() => void platform.toolHost.activate(def.id)}
        />
      ))}
    </div>
  );
}

function ToolEntry({
  def,
  active,
  separated,
  onPick,
}: {
  def: ToolDefinition;
  active: boolean;
  separated: boolean;
  onPick: () => void;
}) {
  const verdict = active ? ALWAYS_ENABLED : (def.canActivate?.({ selection: [], scopes: [] }) ?? ALWAYS_ENABLED);
  return (
    <>
      {separated && <span aria-hidden="true" className="mx-1 h-5 w-px bg-border" />}
      <ToolButton
        icon={toolbarIcon(def.icon)}
        label={def.title}
        shortcut={def.shortcutLabel ?? ""}
        active={active}
        onClick={onPick}
        disabled={!verdict.enabled}
        disabledReason={verdict.reason}
      />
    </>
  );
}
