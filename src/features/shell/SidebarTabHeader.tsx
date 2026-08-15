/*
 * The left-sidebar tab strip — "Model" / "Variables". Rendered by BOTH
 * `ModelTreePanel` and `VariablesPanel` at their own top (each panel owns its
 * full `Slots.ShellLeft` footprint and decides whether to render its body
 * based on `sidebarTabStore`; see `modules/modeling/ui.ts` for why this lives
 * beside the panels rather than as a third contribution).
 */
import { useSidebarTabStore, type SidebarTab } from "@/stores/sidebarTabStore";
import { cn } from "@/ui/cn";

const TABS: { id: SidebarTab; label: string }[] = [
  { id: "model", label: "Model" },
  { id: "variables", label: "Variables" },
];

export function SidebarTabHeader() {
  const activeTab = useSidebarTabStore((s) => s.activeTab);
  const setActiveTab = useSidebarTabStore((s) => s.setActiveTab);
  return (
    <div className="flex flex-none border-b border-border" role="tablist" aria-label="Sidebar">
      {TABS.map((tab) => (
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
