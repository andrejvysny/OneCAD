import { Fragment, useEffect, useMemo, useReducer } from "react";
import { cn } from "@/ui/cn";
import { useToolStore } from "@/stores/toolStore";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import {
  usePlatform,
  useRegistryEntries,
  useActiveToolId,
  type ToolAvailability,
  type ToolContext,
  type ToolDefinition,
} from "@/platform";
import { toolbarScopeToken } from "@/modules/modeling/registryToolbar";
import { useModelingToolContext } from "@/modules/modeling/selectionContext";
import { ToolButton, toolbarIcon } from "./ToolButton";
import { ToolFlyout } from "./ToolFlyout";

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
  // The context a contributed tool is ASKED and ACTIVATED with. Handing every
  // tool an empty one made `ToolContext` decorative: a tool that gates on the
  // selection or the scope could only ever answer "no" (Codex review, P2.5).
  const context = useModelingToolContext();

  // Tool groups the user switched off in "Customize workspace…". Filtering
  // here rather than in the registry is deliberate: the tool stays registered,
  // reachable by shortcut and by the command palette — the workspace only
  // decides what earns toolbar real estate.
  const hiddenGroups = useWorkspaceStore((s) => s.hiddenToolGroups[s.activeId]);

  const token = toolbarScopeToken(mode);
  const entries = useMemo(
    () => {
      const hidden = new Set(hiddenGroups ?? []);
      return tools.filter(
        (t) =>
          (t.scopes === undefined || t.scopes.length === 0 || t.scopes.includes(token)) &&
          !(t.group !== undefined && hidden.has(t.group)),
      );
    },
    // `tools` is the subscription: a registry change gives a new snapshot.
    [tools, token, hiddenGroups],
  );

  // Collapse family members (`ToolDefinition.flyout`) into one slot per family,
  // keeping registry order: a family sits where its first member was.
  const slots = useMemo(
    () => {
      const out: RenderSlot[] = [];
      const families = new Map<string, ToolDefinition[]>();
      for (const t of entries) {
        if (t.flyout !== undefined) {
          let members = families.get(t.flyout);
          if (members === undefined) {
            members = [];
            families.set(t.flyout, members);
            out.push({ kind: "flyout", family: t.flyout, members });
          }
          members.push(t);
        } else {
          out.push({ kind: "single", def: t });
        }
      }
      return out;
    },
    [entries],
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

  const flyoutDefault = useToolStore((s) => s.flyoutDefault);
  const setFlyoutDefault = useToolStore((s) => s.setFlyoutDefault);

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
      {slots.map((slot, i) => {
        const group = slot.kind === "single" ? slot.def.group : slot.members[0]?.group;
        const separated = i > 0 && group !== groupOf(slots[i - 1]);
        if (slot.kind === "flyout" && slot.members.length >= 2) {
          // A family whose other members were hidden collapses back to one chip.
          const stickyId = flyoutDefault[slot.family];
          const sticky = slot.members.find((m) => m.id === stickyId) ?? slot.members[0];
          return (
            <Fragment key={slot.family}>
              {separated && <Separator />}
              <ToolFlyout
                members={slot.members}
                sticky={sticky}
                activeId={activeId}
                context={context}
                onPick={(def) => {
                  setFlyoutDefault(slot.family, def.id);
                  void platform.toolHost.activate(def.id, context);
                }}
              />
            </Fragment>
          );
        }
        const def = slot.kind === "single" ? slot.def : slot.members[0];
        return (
          <Fragment key={def.id}>
            {separated && <Separator />}
            <ToolEntry
              def={def}
              active={def.id === activeId}
              context={context}
              onPick={() => void platform.toolHost.activate(def.id, context)}
            />
          </Fragment>
        );
      })}
    </div>
  );
}

function Separator() {
  return <span aria-hidden="true" className="mx-1 h-5 w-px bg-border" />;
}

function groupOf(slot: RenderSlot): string | undefined {
  return slot.kind === "single" ? slot.def.group : slot.members[0]?.group;
}

type RenderSlot =
  | { kind: "single"; def: ToolDefinition }
  | { kind: "flyout"; family: string; members: ToolDefinition[] };

function ToolEntry({
  def,
  active,
  context,
  onPick,
}: {
  def: ToolDefinition;
  active: boolean;
  context: ToolContext;
  onPick: () => void;
}) {
  const verdict = active ? ALWAYS_ENABLED : (def.canActivate?.(context) ?? ALWAYS_ENABLED);
  return (
    <ToolButton
      icon={toolbarIcon(def.icon)}
      label={def.title}
      shortcut={def.shortcutLabel ?? ""}
      active={active}
      onClick={onPick}
      disabled={!verdict.enabled}
      disabledReason={verdict.reason}
    />
  );
}
