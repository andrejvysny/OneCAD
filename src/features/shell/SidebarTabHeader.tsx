/*
 * The left-sidebar tab strip (Component Library WP-1.4) — "Model" / "Library".
 * Rendered by BOTH `ModelTreePanel` and `LibraryPanel` at their own top (each
 * panel owns its full `Slots.ShellLeft` footprint and decides whether to
 * render its body based on `sidebarTabStore`; see `modules/library/register.ts`
 * for why this lives beside the panels rather than as a third contribution).
 */
import { useSidebarTabStore, type SidebarTab } from "@/stores/sidebarTabStore";
import { cn } from "@/ui/cn";

const TABS: { id: SidebarTab; label: string }[] = [
  { id: "model", label: "Model" },
  { id: "library", label: "Library" },
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
