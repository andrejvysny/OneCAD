/*
 * The Component Library panel (WP-1.4/1.5; spec §7 "The Library panel") —
 * search + card list, docked at `Slots.ShellLeft` beside `ModelTreePanel`
 * (see `SidebarTabHeader`'s doc comment for the shared-slot tab mechanism).
 *
 * A card click ARMS the placement gesture (`placementController.ts`) —
 * clicking the already-armed card again cancels it (a toggle). The gesture
 * itself lives entirely in the controller: this panel only starts/stops it
 * and reflects the armed state back onto the card (a ring), it never touches
 * the viewport directly. Not a real HTML5 drag: see the controller's own
 * doc comment for why (pointer capture on `window`, not drag events).
 */
import { useEffect, useState } from "react";
import { createClient } from "@/ipc/client";
import type { LibraryComponent } from "@/ipc/types";
import { useSidebarTabStore } from "@/stores/sidebarTabStore";
import { tasksStore } from "@/stores/tasksStore";
import { SidebarTabHeader } from "@/features/shell/SidebarTabHeader";
import { TextInput } from "@/ui/TextInput";
import { Button } from "@/ui/Button";
import { Icon } from "@/icons/Icon";
import { armPlacement, cancelPlacement, useArmedComponentKey } from "@/modules/library/placementController";

type LoadState = "loading" | "ready" | "error";

function matches(component: LibraryComponent, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    component.name.toLowerCase().includes(q) ||
    component.id.toLowerCase().includes(q) ||
    component.category.some((c) => c.toLowerCase().includes(q)) ||
    component.tags.some((t) => t.toLowerCase().includes(q))
  );
}

export function LibraryPanel() {
  // Component Library WP-1.4: read unconditionally (rules of hooks) — only
  // the returned JSX is gated. See `ModelTreePanel`'s matching comment.
  const activeSidebarTab = useSidebarTabStore((s) => s.activeTab);
  const armedKey = useArmedComponentKey();

  const [components, setComponents] = useState<LibraryComponent[]>([]);
  const [state, setState] = useState<LoadState>("loading");
  const [query, setQuery] = useState("");
  const [reindexing, setReindexing] = useState(false);

  const load = () => {
    setState("loading");
    createClient()
      .listLibraryComponents()
      .then((list) => {
        setComponents(list);
        setState("ready");
      })
      .catch(() => setState("error"));
  };

  useEffect(load, []);

  // WP-1.6: the tasks chip's first real producer. `reindexLibrary()` is a
  // single atomic response (`ReindexReport` has no incremental count), so
  // begin→end with no `setProgress` in between is the honest shape — same
  // "progress is optional" rule `tasksStore`'s own doc comment states.
  const reindex = async () => {
    setReindexing(true);
    tasksStore.getState().begin("library.reindex", "Reindexing library…");
    try {
      await createClient().reindexLibrary();
      load();
    } finally {
      setReindexing(false);
      tasksStore.getState().end("library.reindex");
    }
  };

  if (activeSidebarTab !== "library") return null;

  const filtered = components.filter((c) => matches(c, query));

  return (
    <div className="absolute bottom-[34px] left-0 top-0 z-20 flex w-[220px] flex-col border-r border-border bg-panel">
      <SidebarTabHeader />

      <div className="flex flex-none items-center gap-1.5 border-b border-border px-2 py-1.5">
        <TextInput
          leadingIcon="search"
          placeholder="Search components…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          wrapperClassName="min-w-0 flex-1"
          aria-label="Search library components"
        />
        <Button
          variant="secondary"
          onClick={() => void reindex()}
          disabled={reindexing}
          aria-label="Reindex library"
          title="Reindex library"
        >
          {reindexing ? "…" : "Reindex"}
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-1.5">
        {state === "loading" && (
          <div className="px-1 py-3 text-[11.5px] text-ink-7">Loading…</div>
        )}
        {state === "error" && (
          <div className="px-1 py-3 text-[11.5px] text-ink-7">
            Could not read the component library.
          </div>
        )}
        {state === "ready" && components.length === 0 && (
          <div className="px-1 py-3 text-[11.5px] text-ink-7">
            No components indexed yet. Drop a package into the library folder,
            then reindex.
          </div>
        )}
        {state === "ready" && components.length > 0 && filtered.length === 0 && (
          <div className="px-1 py-3 text-[11.5px] text-ink-7">No matches for “{query}”.</div>
        )}
        <div className="grid grid-cols-2 gap-1.5">
          {filtered.map((c) => {
            const key = `${c.id}@${c.version}`;
            const armed = armedKey === key;
            return (
              <button
                key={key}
                type="button"
                data-testid="library-card"
                data-armed={armed || undefined}
                onClick={() => (armed ? cancelPlacement() : armPlacement(c))}
                className={`flex flex-col gap-0.5 rounded-md border p-1.5 text-left ${
                  armed ? "border-accent bg-surface ring-1 ring-accent" : "border-border bg-surface"
                }`}
                title={armed ? `Placing ${c.id} — click again to cancel` : c.id}
              >
                <div className="flex aspect-square items-center justify-center rounded bg-panel text-ink-8">
                  <Icon name="cube" size={22} strokeWidth={1.5} />
                </div>
                <div className="truncate text-[11px] font-medium text-ink-3">{c.name}</div>
                <div className="truncate text-[10px] text-ink-7">{c.version}</div>
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
