import { Icon } from "@/icons/Icon";
import type { IconName } from "@/icons/paths";
import { MonoValue } from "@/ui/MonoValue";
import { cn } from "@/ui/cn";
import type { StartNavKey } from "./StartScreen";

const NAV: { key: StartNavKey; label: string; icon: IconName }[] = [
  { key: "recent", label: "Recent", icon: "clock" },
  { key: "starred", label: "Starred", icon: "star" },
  { key: "templates", label: "Templates", icon: "template" },
  { key: "library", label: "Library", icon: "cube" },
];

type StartSidebarProps = {
  active: StartNavKey;
  onNavigate: (key: StartNavKey) => void;
  onOpenSettings: () => void;
  onOpenExtensions: () => void;
  version: string;
};

/**
 * Start-screen sidebar (prototype 1b): wordmark, Recent/Starred/Templates nav,
 * and a bottom-of-rail version stamp. Active-row styling mirrors `TreeRow`'s
 * bg-sel-bg/text-sel-text convention.
 */
export function StartSidebar({
  active,
  onNavigate,
  onOpenSettings,
  onOpenExtensions,
  version,
}: StartSidebarProps) {
  return (
    <div className="flex h-full w-[220px] shrink-0 flex-col border-r border-border bg-panel px-3 pb-3 pt-0">
      <div className="mb-4 px-2 text-[15px] font-bold tracking-[-0.01em] text-ink">OneCAD</div>

      <nav className="flex flex-col gap-0.5">
        {NAV.map((item) => {
          const isActive = item.key === active;
          return (
            <button
              key={item.key}
              type="button"
              aria-current={isActive ? "page" : undefined}
              onClick={() => onNavigate(item.key)}
              className={cn(
                "flex h-8 cursor-pointer items-center gap-2 rounded-sm px-2 text-left font-ui text-[13px]",
                isActive ? "bg-sel-bg font-medium text-sel-text" : "text-ink-3 hover:bg-hover-2",
              )}
            >
              <Icon name={item.icon} size={14} strokeWidth={1.7} />
              {item.label}
            </button>
          );
        })}
      </nav>

      <span className="flex-1" />

      {/* Extensions sits with Settings, BELOW the rule — it is application
          configuration, not project content, so grouping it with
          Recent/Starred/Templates would have said the opposite (prototype 2d). */}
      <div aria-hidden="true" className="mx-2 my-2 h-px bg-border" />
      <button
        type="button"
        data-testid="start-extensions"
        onClick={onOpenExtensions}
        className="mb-0.5 flex h-8 cursor-pointer items-center gap-2 rounded-sm px-2 text-left font-ui text-[13px] text-ink-3 hover:bg-hover-2"
      >
        <Icon name="puzzle" size={14} strokeWidth={1.6} />
        Extensions
      </button>
      <button
        type="button"
        onClick={onOpenSettings}
        className="mb-1.5 flex h-8 cursor-pointer items-center gap-2 rounded-sm px-2 text-left font-ui text-[13px] text-ink-3 hover:bg-hover-2"
      >
        <Icon name="settings" size={14} strokeWidth={1.7} />
        Settings
      </button>

      <MonoValue className="px-2 text-[11px] text-ink-7">{version}</MonoValue>
    </div>
  );
}
