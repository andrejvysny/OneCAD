/*
 * Left-sidebar "Variables" tab — the document variable table's home now that
 * it no longer shares the right inspector with the selection-driven sections
 * (WP-VE.2's own reasoning still applies: it is document-level, not
 * selection-driven, so a sidebar tab of its own fits better than living
 * beside sections about whatever is currently picked). Shares
 * `Slots.ShellLeft` with `ModelTreePanel` via `sidebarTabStore`, same
 * "one region, several tab-like occupants" pattern that panel documents.
 */
import { useSidebarTabStore } from "@/stores/sidebarTabStore";
import { SidebarTabHeader } from "@/features/shell/SidebarTabHeader";
import { VariablesSection } from "./VariablesSection";

export function VariablesPanel() {
  // Read unconditionally (rules of hooks) — only the returned JSX is gated,
  // matching `ModelTreePanel`'s own comment on the same mechanism.
  const activeSidebarTab = useSidebarTabStore((s) => s.activeTab);
  if (activeSidebarTab !== "variables") return null;

  return (
    <div className="absolute bottom-[34px] left-0 top-0 z-20 flex w-[220px] flex-col border-r border-border bg-panel">
      <SidebarTabHeader />
      <div className="min-h-0 flex-1 overflow-auto p-3">
        <VariablesSection />
      </div>
    </div>
  );
}
