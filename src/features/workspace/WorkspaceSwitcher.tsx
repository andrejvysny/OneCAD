import { useRef } from "react";
import { Icon } from "@/icons/Icon";
import { ICONS, type IconName } from "@/icons/paths";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { cn } from "@/ui/cn";
import { useRegistryRegistrations, usePlatform, type WorkspaceDefinition } from "@/platform";
import { TitleBarButton } from "@/features/shell/TitleBarButton";
import { useWorkspaceStore } from "@/stores/workspaceStore";
import { extensionsStore } from "@/stores/extensionsStore";
import { DEFAULT_WORKSPACE_ID } from "@/modules/shell/workspaceIds";

/** Built-ins are the ones the application itself ships. */
const isBuiltIn = (owner: string) => owner === "onecad.shell" || owner.startsWith("onecad.");

const wsIcon = (icon: string | undefined): IconName =>
  icon && icon in ICONS ? (icon as IconName) : "workspace";

/**
 * Title-bar workspace selector (prototype 2a).
 *
 * The workspace is GLOBAL context, so it lives in the title bar and never
 * becomes another toolbar layer — switching is a lens over one document, not an
 * app change (ADR-0001).
 *
 * The list is the workspace REGISTRY, not a hard-coded array: a module or add-on
 * that registers a workspace appears here without this file changing, and the
 * ADD-ONS group is simply "everything whose owner is not a built-in". That is
 * also why an empty add-on group renders nothing at all rather than a
 * placeholder row — there is no catalog to advertise from.
 */
export function WorkspaceSwitcher() {
  const platform = usePlatform();
  const registrations = useRegistryRegistrations(platform.workspaces);
  const activeId = useWorkspaceStore((s) => s.activeId);
  const open = useWorkspaceStore((s) => s.menuOpen);
  const setMenuOpen = useWorkspaceStore((s) => s.setMenuOpen);
  const setActive = useWorkspaceStore((s) => s.setActive);
  const setCustomizeOpen = useWorkspaceStore((s) => s.setCustomizeOpen);
  const anchor = useRef<HTMLButtonElement | null>(null);

  const builtIn = registrations.filter((r) => isBuiltIn(r.owner));
  const addOns = registrations.filter((r) => !isBuiltIn(r.owner));

  // An active id that resolves to nothing (its owner was unloaded) must not
  // blank the button: fall back to the default workspace's title.
  const active =
    registrations.find((r) => r.entry.id === activeId)?.entry ??
    registrations.find((r) => r.entry.id === DEFAULT_WORKSPACE_ID)?.entry;

  const row = (ws: WorkspaceDefinition, owner: string) => {
    const selected = ws.id === activeId;
    return (
      <div
        key={ws.id}
        role="menuitemradio"
        aria-checked={selected}
        data-testid={`workspace-option-${ws.id}`}
        onClick={() => setActive(ws.id)}
        className="flex h-[30px] cursor-pointer items-center gap-2 rounded-[5px] px-2.5 text-[13px] text-ink hover:bg-well"
      >
        <Icon
          name="check"
          size={13}
          strokeWidth={2.4}
          className={cn("flex-none text-accent", selected ? "opacity-100" : "opacity-0")}
        />
        <span className="flex-1 truncate">{ws.title}</span>
        {!isBuiltIn(owner) && (
          <Icon name="puzzle" size={13} strokeWidth={1.6} className="flex-none text-ink-6" />
        )}
      </div>
    );
  };

  return (
    <>
      {/* Same control as every other button in the bar (see `TitleBarButton`).
          Its accent glyph is what marks it as global context; a border of its
          own would have made it a fourth kind of button in a row of three. */}
      <TitleBarButton
        ref={anchor}
        icon={wsIcon(active?.icon)}
        iconAccent
        menu
        active={open}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Workspace: ${active?.title ?? "Design"}`}
        data-testid="workspace-switcher"
        onClick={() => setMenuOpen(!open)}
      >
        {active?.title ?? "Design"}
      </TitleBarButton>

      <Popover
        open={open}
        onClose={() => setMenuOpen(false)}
        anchorRef={anchor}
        width={248}
        className="p-[5px]"
      >
        <div role="menu" aria-label="Workspaces">
          <SectionLabel className="px-2.5 pb-1 pt-[7px]">Workspaces</SectionLabel>
          {builtIn.map((r) => row(r.entry, r.owner))}

          {addOns.length > 0 && (
            <>
              <SectionLabel className="px-2.5 pb-1 pt-[9px]">Add-ons</SectionLabel>
              {addOns.map((r) => row(r.entry, r.owner))}
            </>
          )}

          <div aria-hidden="true" className="mx-1.5 my-[5px] h-px bg-border-subtle" />
          <div
            role="menuitem"
            data-testid="workspace-customize"
            onClick={() => setCustomizeOpen(true)}
            className="flex h-[30px] cursor-pointer items-center rounded-[5px] px-2.5 text-[12.5px] text-accent hover:bg-well"
          >
            Customize {active?.title ?? "Design"} workspace…
          </div>
          <div
            role="menuitem"
            data-testid="workspace-manage-extensions"
            onClick={() => {
              setMenuOpen(false);
              extensionsStore.getState().openManager();
            }}
            className="flex h-[30px] cursor-pointer items-center rounded-[5px] px-2.5 text-[12.5px] text-accent hover:bg-well"
          >
            Manage extensions…
          </div>
        </div>
      </Popover>
    </>
  );
}
