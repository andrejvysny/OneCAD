/*
 * Start-screen library browser (Component Library WP-2.4 follow-up) — lets
 * a user explore the component library WITHOUT opening a project first, per
 * the user's own framing: "not only through opened project."
 *
 * DELIBERATELY READ-ONLY. `list_library_components`/`reindex_library` need
 * no open document (`library_root(app)` only reads `app_data_dir()` — no
 * `AppState.runtime` lock), so browsing here is real, not a fake preview.
 * PLACING a component, though, mints a `PlaceComponent` record on a live
 * document (`CommandApiService.placeComponent` → `DocumentRuntime`), which
 * genuinely does not exist yet on this screen. Rather than fake an arm/drag
 * gesture with nowhere to land it, a selected card shows `ComponentDetails`
 * and says plainly that placing needs an open project — the same "say it is
 * not there" discipline the Extensions ▸ Browse empty state and the P1
 * embedded-source test gap already follow in this codebase.
 */
import { useEffect, useState } from "react";
import { createClient } from "@/ipc/client";
import type { LibraryComponent } from "@/ipc/types";
import { TextInput } from "@/ui/TextInput";
import { Button } from "@/ui/Button";
import { Icon } from "@/icons/Icon";
import { ComponentDetails } from "./ComponentDetails";

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

export function StartLibraryPanel() {
  const [components, setComponents] = useState<LibraryComponent[]>([]);
  const [state, setState] = useState<LoadState>("loading");
  const [query, setQuery] = useState("");
  const [reindexing, setReindexing] = useState(false);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);

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

  const reindex = async () => {
    setReindexing(true);
    try {
      await createClient().reindexLibrary();
      load();
    } finally {
      setReindexing(false);
    }
  };

  const filtered = components.filter((c) => matches(c, query));
  const selected = filtered.find((c) => `${c.id}@${c.version}` === selectedKey) ?? null;

  return (
    <div className="flex gap-4">
      <div className="min-w-0 flex-1">
        <div className="mb-4 flex items-center gap-1.5">
          <TextInput
            leadingIcon="search"
            placeholder="Search components…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            wrapperClassName="w-[220px]"
            aria-label="Search library components"
          />
          <Button
            variant="secondary"
            onClick={() => void reindex()}
            disabled={reindexing}
            aria-label="Reindex library"
          >
            {reindexing ? "Reindexing…" : "Reindex"}
          </Button>
        </div>

        {state === "loading" && (
          <div className="pb-5 pt-9 text-center text-[12.5px] text-ink-6">Loading…</div>
        )}
        {state === "error" && (
          <div className="pb-5 pt-9 text-center text-[12.5px] text-ink-6">
            Could not read the component library.
          </div>
        )}
        {state === "ready" && components.length === 0 && (
          <div className="pb-5 pt-9 text-center">
            <div className="text-[12.5px] text-ink-6">No components indexed yet.</div>
            <div className="mt-1.5 text-[11.5px] text-ink-7">
              Drop a package into the library folder, then reindex.
            </div>
          </div>
        )}
        {state === "ready" && components.length > 0 && filtered.length === 0 && (
          <div className="pb-5 pt-9 text-center text-[12.5px] text-ink-6">
            No matches for &ldquo;{query}&rdquo;.
          </div>
        )}
        {state === "ready" && filtered.length > 0 && (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3">
            {filtered.map((c) => {
              const key = `${c.id}@${c.version}`;
              const isSelected = selectedKey === key;
              return (
                <button
                  key={key}
                  type="button"
                  data-testid="start-library-card"
                  data-selected={isSelected || undefined}
                  onClick={() => setSelectedKey(isSelected ? null : key)}
                  title={c.id}
                  className={`flex flex-col gap-1 rounded-lg border p-2 text-left transition-[box-shadow,border-color] duration-150 hover:border-card-hover-border hover:shadow-card-hover ${
                    isSelected ? "border-accent ring-1 ring-accent" : "border-border bg-surface"
                  }`}
                >
                  <div className="flex aspect-square items-center justify-center rounded bg-well text-ink-8">
                    <Icon name="cube" size={26} strokeWidth={1.5} />
                  </div>
                  <div className="truncate text-[12px] font-semibold text-ink">{c.name}</div>
                  <div className="truncate text-[10.5px] text-ink-6">{c.version}</div>
                </button>
              );
            })}
          </div>
        )}
      </div>

      {selected && <ComponentDetails component={selected} onClose={() => setSelectedKey(null)} />}
    </div>
  );
}
