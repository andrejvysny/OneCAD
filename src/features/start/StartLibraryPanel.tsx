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
 * gesture with nowhere to land it, a selected card opens the shared
 * `LibraryModal` without `canPlace`, which says plainly that placing needs
 * an open project — the same "say it is not there" discipline the
 * Extensions ▸ Browse empty state and the P1 embedded-source test gap
 * already follow in this codebase.
 */
import { useEffect, useState } from "react";
import { createClient } from "@/ipc/client";
import type { LibraryComponent } from "@/ipc/types";
import { TextInput } from "@/ui/TextInput";
import { LibraryModal } from "@/features/library/LibraryModal";
import { ComponentCard } from "@/features/library/ComponentCard";

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

  const filtered = components.filter((c) => matches(c, query));
  const selected = filtered.find((c) => `${c.id}@${c.version}` === selectedKey) ?? null;

  return (
    <div className="flex gap-4">
      <div className="min-w-0 flex-1">
        {/*
          No Reindex button here (LGU-1 WP-A, defect F9). The grid re-reads the
          catalog on mount, and the manual rebuild now lives in the library
          modal's overflow menu — one home for a maintenance verb, not two.
        */}
        <div className="mb-4 flex items-center gap-1.5">
          <TextInput
            leadingIcon="search"
            placeholder="Search components…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            wrapperClassName="w-[220px]"
            aria-label="Search library components"
          />
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
              return (
                <ComponentCard
                  key={key}
                  component={c}
                  selected={selectedKey === key}
                  onClick={() => setSelectedKey(key)}
                  testId="start-library-card"
                />
              );
            })}
          </div>
        )}
      </div>

      <LibraryModal
        open={selectedKey !== null}
        onClose={() => setSelectedKey(null)}
        initialSelection={selected}
      />
    </div>
  );
}
