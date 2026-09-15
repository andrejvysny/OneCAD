/*
 * The left-sidebar tab strip — "Model" / "Variables" / "Assistant". Rendered by
 * EVERY panel that shares the region (`ModelTreePanel`, `VariablesPanel`,
 * `AssistantPanel`) at its own top: each panel owns its full `Slots.ShellLeft`
 * footprint and decides whether to render its body based on `sidebarTabStore`;
 * see `modules/modeling/ui.ts` for why this lives beside the panels rather than
 * as a contribution of its own.
 *
 * The Assistant tab is GATED: `settingsStore.assistantEnabled` is off by
 * default, and while it is off the button is not rendered at all — the same
 * gate `AssistantPanel` reads before rendering anything.
 */
import { useSettingsStore } from "@/stores/settingsStore";
import { useSidebarTabStore, type SidebarTab } from "@/stores/sidebarTabStore";
import { cn } from "@/ui/cn";

const TABS: { id: SidebarTab; label: string }[] = [
  { id: "model", label: "Model" },
  { id: "variables", label: "Variables" },
];

const ASSISTANT_TAB: { id: SidebarTab; label: string } = { id: "assistant", label: "Assistant" };

export function SidebarTabHeader() {
  const activeTab = useSidebarTabStore((s) => s.activeTab);
  const setActiveTab = useSidebarTabStore((s) => s.setActiveTab);
  const assistantEnabled = useSettingsStore((s) => s.assistantEnabled);
  const tabs = assistantEnabled ? [...TABS, ASSISTANT_TAB] : TABS;
  return (
    <div className="flex flex-none border-b border-border" role="tablist" aria-label="Sidebar">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={activeTab === tab.id}
          data-testid={`sidebar-tab-${tab.id}`}
          onClick={() => setActiveTab(tab.id)}
          className={cn(
            "flex-1 border-b-2 px-2.5 py-1.5 text-[11.5px] font-medium transition-colors",
            activeTab === tab.id
              ? "border-accent text-ink-2"
              : "border-transparent text-ink-6 hover:text-ink-4",
          )}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
