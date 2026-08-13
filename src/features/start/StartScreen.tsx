import { useEffect, useMemo, useState } from "react";
import { Button } from "@/ui/Button";
import { TextInput } from "@/ui/TextInput";
import { Icon } from "@/icons/Icon";
import { useAppStore } from "@/stores/appStore";
import type { ProjectTemplate, RecentProject } from "@/ipc/types";
import { RecentGrid } from "./RecentGrid";
import { RecentGridSkeleton } from "./RecentGridSkeleton";
import { RecoveryCard } from "./RecoveryCard";
import { SortMenu, type SortKey } from "./SortMenu";
import { StartSidebar } from "./StartSidebar";
import { StartLibraryPanel } from "./StartLibraryPanel";
import { TemplateGrid } from "./TemplateGrid";
import { createClient } from "@/ipc/client";
import { SettingsModal } from "@/features/settings/SettingsModal";
import { ExtensionsManager } from "@/features/extensions/ExtensionsManager";
import { extensionsStore } from "@/stores/extensionsStore";

const APP_VERSION = "v0.1.0";

/**
 * Sidebar nav sections (prototype 1b + Component Library WP-2.4's addition).
 * "recent" and "library" have real backing data; "starred"/"templates" stay
 * placeholders (`NAV_EMPTY_HINT`) — no starring or template feature exists yet.
 */
export type StartNavKey = "recent" | "starred" | "templates" | "library";

const NAV_TITLE: Record<StartNavKey, string> = {
  recent: "Recent",
  starred: "Starred",
  templates: "Templates",
  library: "Library",
};

const NAV_EMPTY_HINT: Record<"starred" | "templates", string> = {
  starred: "No starred projects yet.",
  templates: "No templates yet. Open a project and run “Save as Template…” from the command palette.",
};

/** Filter (case-insensitive substring on name) + sort — mirrors prototype 1a. */
function filterSort(
  recents: RecentProject[],
  query: string,
  sort: SortKey,
): RecentProject[] {
  const q = query.toLowerCase();
  const list = recents.filter((p) => p.name.toLowerCase().includes(q));
  return [...list].sort(
    sort === "name"
      ? (a, b) => a.name.localeCompare(b.name)
      : (a, b) => Date.parse(b.modifiedAt) - Date.parse(a.modifiedAt),
  );
}

/**
 * Start screen — variant 1b "sidebar + grid": a full-window layout with a
 * fixed nav rail (wordmark, Recent/Starred/Templates, Import) on the left
 * and a searchable + sortable recent-projects grid filling the rest.
 */
export function StartScreen() {
  const recents = useAppStore((s) => s.recents);
  const recentsStatus = useAppStore((s) => s.recentsStatus);
  const loadRecents = useAppStore((s) => s.loadRecents);
  const renameRecent = useAppStore((s) => s.renameRecent);
  const deleteRecent = useAppStore((s) => s.deleteRecent);
  const revealRecent = useAppStore((s) => s.revealRecent);
  const newProject = useAppStore((s) => s.newProject);
  const newFromTemplate = useAppStore((s) => s.newFromTemplate);
  const openProject = useAppStore((s) => s.openProject);
  const importFromDialog = useAppStore((s) => s.importFromDialog);
  const importError = useAppStore((s) => s.importError);
  const recovery = useAppStore((s) => s.recovery);
  const recoveryStatus = useAppStore((s) => s.recoveryStatus);
  const checkRecovery = useAppStore((s) => s.checkRecovery);
  const recoverDocument = useAppStore((s) => s.recoverDocument);
  const discardRecovery = useAppStore((s) => s.discardRecovery);

  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SortKey>("date");
  const [nav, setNav] = useState<StartNavKey>("recent");
  // Templates load on first visit to their row rather than at mount: the
  // recents grid is what the screen opens on, and a template read touches the
  // library root (WP-B3).
  const [templates, setTemplates] = useState<ProjectTemplate[] | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    if (recentsStatus === "idle") void loadRecents();
  }, [recentsStatus, loadRecents]);

  useEffect(() => {
    if (nav !== "templates" || templates !== null) return;
    let alive = true;
    // An unreadable library reads as "no templates", never as an error banner:
    // the start screen's job is to let the user get INTO a project, and a
    // template row that cannot load must not stand in the way of that.
    void createClient()
      .listTemplates()
      .then((list) => alive && setTemplates(list))
      .catch(() => alive && setTemplates([]));
    return () => {
      alive = false;
    };
  }, [nav, templates]);

  useEffect(() => {
    if (recoveryStatus === "idle") void checkRecovery();
  }, [recoveryStatus, checkRecovery]);

  // A project deleted/renamed outside the app (Finder, another process) while
  // the start screen sits idle only clears on the next `loadRecents()` call —
  // refresh on refocus so that happens without an explicit reload. Mirrors
  // `ViewportRoot.tsx`'s wake listener verbatim.
  useEffect(() => {
    const onWake = () => {
      if (recentsStatus === "ready") void loadRecents();
    };
    document.addEventListener("visibilitychange", onWake);
    window.addEventListener("focus", onWake);
    return () => {
      document.removeEventListener("visibilitychange", onWake);
      window.removeEventListener("focus", onWake);
    };
  }, [recentsStatus, loadRecents]);

  // Idle-prefetch the editor chunk (App.tsx's lazy() import, EXACT same
  // specifier) once recents have loaded, so New-project rarely blocks on a
  // cold fetch of the ~130-module editor tree. Deliberately NO setTimeout
  // fallback: jsdom has no requestIdleCallback, which keeps that whole tree
  // out of every StartScreen vitest run; an older Safari merely loses the
  // prefetch and falls back to the ordinary lazy-load-on-navigate path.
  useEffect(() => {
    if (recentsStatus !== "ready") return;
    const id = window.requestIdleCallback?.(() => {
      void import("@/features/shell/EditorScreen");
    });
    return () => {
      if (id !== undefined) window.cancelIdleCallback?.(id);
    };
  }, [recentsStatus]);

  const list = useMemo(
    () => filterSort(recents, query, sort),
    [recents, query, sort],
  );
  const ready = recentsStatus === "ready";
  const bootError = recentsStatus === "error" || recoveryStatus === "error";

  return (
    // `relative`: the extensions manager is an `absolute inset-0` overlay and
    // needs this element as its containing block (the editor gives it one too).
    <div className="relative flex h-full w-full flex-col font-ui text-ink">
      {/* Invisible full-width titlebar: the window uses titleBarStyle "Overlay"
          (macOS draws the traffic lights over the webview, no native bar), and
          this screen has no TitleBar.tsx of its own — without a drag region the
          whole top edge is dead. Spans sidebar + main so dragging works anywhere
          along the top, not just over the sidebar. Split into two panes colored
          to match what sits directly beneath each — a single flat tint would
          show as a seam against the sidebar/main boundary below it. */}
      <div className="flex h-9 w-full flex-none" aria-hidden="true">
        <div data-tauri-drag-region className="w-[220px] shrink-0 border-r border-border bg-panel" />
        <div data-tauri-drag-region className="flex-1 bg-surface" />
      </div>

      <div className="flex flex-1 overflow-hidden">
        <StartSidebar
          active={nav}
          onNavigate={setNav}
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenExtensions={() => extensionsStore.getState().openManager()}
          version={APP_VERSION}
        />

        <main className="flex-1 overflow-auto bg-surface px-8 pb-7 pt-0">
          {/* Header: section title · search · sort · Import · New project */}
          <div className="mb-5 flex items-center gap-2.5">
            <h1 className="text-[17px] font-bold tracking-[-0.01em] text-ink">
              {NAV_TITLE[nav]}
            </h1>
            <span className="flex-1" />
            {nav === "recent" && (
              <>
                <TextInput
                  leadingIcon="search"
                  placeholder="Search projects…"
                  aria-label="Search projects"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  wrapperClassName="w-[220px]"
                />
                <SortMenu value={sort} onChange={setSort} />
              </>
            )}
            <Button variant="secondary" size="lg" onClick={() => void importFromDialog()}>
              <Icon name="import" size={15} strokeWidth={1.7} />
              Import…
            </Button>
            <Button variant="primary" size="lg" onClick={() => void newProject()}>
              <Icon name="plus" size={15} strokeWidth={2} />
              New project
            </Button>
          </div>

          {/* Import failure — the StatusBar this would normally use is editor-only
              chrome, and a failed import leaves the user right here. */}
          {importError && (
            <div role="alert" className="mb-5 text-[12.5px] text-traffic-close">
              Import failed: {importError}
            </div>
          )}

          {/* Crash-recovery offer (a crashed session left an autosave) */}
          {recovery && (
            <RecoveryCard
              recovery={recovery}
              onRestore={recoverDocument}
              onDiscard={discardRecovery}
            />
          )}

          {nav === "library" ? (
            <StartLibraryPanel />
          ) : nav === "templates" && templates !== null && templates.length > 0 ? (
            <TemplateGrid templates={templates} onUse={(id) => void newFromTemplate(id)} />
          ) : nav !== "recent" ? (
            <div className="pb-5 pt-9 text-center text-[12.5px] text-ink-6">
              {NAV_EMPTY_HINT[nav]}
            </div>
          ) : (
            <div aria-busy={!ready}>
              {!ready && !bootError && <RecentGridSkeleton />}
              {bootError && (
                <div className="flex flex-col items-center gap-3 pb-5 pt-9 text-center">
                  <div className="text-[12.5px] text-traffic-close">
                    Could not load the start screen.
                  </div>
                  <div className="flex gap-2">
                    {recentsStatus === "error" && (
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => void loadRecents()}
                      >
                        Retry recents
                      </Button>
                    )}
                    {recoveryStatus === "error" && (
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => void checkRecovery()}
                      >
                        Retry recovery
                      </Button>
                    )}
                  </div>
                </div>
              )}
              {ready && list.length > 0 && (
                <RecentGrid
                  projects={list}
                  onOpen={openProject}
                  onRename={renameRecent}
                  onDelete={deleteRecent}
                  onReveal={revealRecent}
                />
              )}
              {ready && list.length === 0 && (
                <EmptyState searching={query.length > 0} />
              )}
            </div>
          )}
        </main>
      </div>

      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      {/* Mounted directly rather than through a slot: the start screen has no
          slot hosts (its whole point is not to pull the editor chunk in), and
          the manager reaches only the platform registries. */}
      <ExtensionsManager />
    </div>
  );
}

/**
 * Empty grid state. When a search is active the prototype's exact copy is used;
 * with no recents at all we add a short hint pointing at the actions above.
 */
function EmptyState({ searching }: { searching: boolean }) {
  if (searching) {
    return (
      <div className="pb-5 pt-9 text-center text-[12.5px] text-ink-6">
        No projects match your search.
      </div>
    );
  }
  return (
    <div className="pb-5 pt-9 text-center">
      <div className="text-[12.5px] text-ink-6">No recent projects yet.</div>
      <div className="mt-1.5 text-[11.5px] text-ink-7">
        Start a new project or open an existing file.
      </div>
    </div>
  );
}
