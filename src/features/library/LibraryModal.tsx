/*
 * LibraryModal — full-size library browser + component detail (Component
 * Library WP-2.4 follow-up). Shared by the Start screen (`canPlace` unset —
 * no open document to place into) and the editor's "Library" toolbar tool
 * (`canPlace` set). Grid/search/detail/preview used to be split and
 * duplicated across `StartLibraryPanel`+`ComponentDetails` and the sidebar
 * `LibraryPanel`; merging them here means the 3D-preview overflow fix in
 * `ComponentPreview3D` only has to exist once, and a full-size modal replaces
 * both the cramped sidebar card and the cramped sidebar tab.
 *
 * PLACEMENT stays a two-step gesture, unchanged: "Place" arms
 * `placementController` (same call `LibraryPanel`'s card click already made)
 * and closes the modal so the viewport is reachable — the click that actually
 * commits happens later, in the 3D view, per that controller's own doc
 * comment.
 */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "@/icons/Icon";
import { TextInput } from "@/ui/TextInput";
import { Popover } from "@/ui/Popover";
import { MenuItem } from "@/ui/MenuItem";
import { createClient } from "@/ipc/client";
import type { LibraryComponent } from "@/ipc/types";
import { tasksStore } from "@/stores/tasksStore";
import { ComponentCard } from "./ComponentCard";
import { ComponentDetailRail } from "./ComponentDetailRail";
import { armPlacement } from "@/modules/library/placementController";

export interface LibraryModalProps {
  open: boolean;
  onClose: () => void;
  /** Absent on the Start screen — there is no open document to place into. */
  canPlace?: boolean;
  /**
   * Pre-selects a component when the modal opens (the Start screen already
   * has the full object from its own grid fetch — handing it over directly
   * avoids waiting on this modal's own independent fetch to resolve before
   * showing detail for the card the user just clicked).
   */
  initialSelection?: LibraryComponent | null;
}

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

export function LibraryModal({ open, onClose, canPlace, initialSelection }: LibraryModalProps) {
  const [components, setComponents] = useState<LibraryComponent[]>([]);
  const [state, setState] = useState<LoadState>("loading");
  const [query, setQuery] = useState("");
  const [reindexing, setReindexing] = useState(false);
  const [selected, setSelected] = useState<LibraryComponent | null>(null);
  const [overflowOpen, setOverflowOpen] = useState(false);
  const overflowRef = useRef<HTMLButtonElement>(null);

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

  // Loaded on OPEN, not on mount: this component is mounted unconditionally
  // (a `Slots.ShellOverlay` host, or always present next to the Start-screen
  // grid) so it can react to Escape/backdrop while closed — fetching then
  // would be a wasted catalog read on every Start-screen/editor mount that
  // never opens it. The catalog can also change under a still-mounted host
  // (an authored component, a reindex elsewhere), so re-fetch on every open
  // rather than once. Also resets the selection to whatever the caller
  // pre-selected — re-opening onto a different card must not carry over the
  // previous session's pick.
  useEffect(() => {
    if (!open) return;
    load();
    setSelected(initialSelection ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const reindex = async () => {
    setReindexing(true);
    // WP-1.6: the tasks chip's producer — same call shape the old sidebar
    // `LibraryPanel` used, moved here with it.
    tasksStore.getState().begin("library.reindex", "Reindexing library…");
    try {
      await createClient().reindexLibrary();
      load();
    } finally {
      setReindexing(false);
      tasksStore.getState().end("library.reindex");
    }
  };

  const filtered = components.filter((c) => matches(c, query));
  const selectedKey = selected ? `${selected.id}@${selected.version}` : null;

  return createPortal(
    <div className="fixed inset-0 z-[110] flex items-center justify-center bg-scrim p-5">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Library"
        className="flex h-[min(760px,calc(100vh-40px))] w-[min(1200px,calc(100vw-40px))] flex-col overflow-hidden rounded-md border border-border bg-surface shadow-popover"
      >
        <div className="flex flex-none items-center justify-between border-b border-border px-4 py-2.5">
          <div className="text-[13px] font-semibold text-ink-3">Library</div>
          <div className="flex items-center gap-1">
            {/*
              "Rebuild index" lives in an overflow, not on the toolbar (LGU-1
              WP-A, defect F9). Reindexing is maintenance for a catalog that
              already reindexes itself — this modal re-reads it on every open —
              so a primary button spent the most prominent slot beside Search on
              a developer verb ("Reindex") that most users would either ignore
              or click hoping it did something for them.
            */}
            <button
              type="button"
              ref={overflowRef}
              aria-label="Library options"
              aria-haspopup="menu"
              onClick={() => setOverflowOpen((v) => !v)}
              className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-6 hover:bg-well hover:text-ink-2"
            >
              <Icon name="overflow" size={13} strokeWidth={1.8} />
            </button>
            <button
              type="button"
              aria-label="Close library"
              onClick={onClose}
              className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-6 hover:bg-well hover:text-ink-2"
            >
              <Icon name="x" size={13} strokeWidth={1.8} />
            </button>
          </div>
          <Popover
            open={overflowOpen}
            onClose={() => setOverflowOpen(false)}
            anchorRef={overflowRef}
            placement="bottom-end"
            width={190}
          >
            <MenuItem
              label={reindexing ? "Rebuilding index…" : "Rebuild index"}
              data-testid="library-rebuild-index"
              onClick={() => {
                setOverflowOpen(false);
                void reindex();
              }}
            />
          </Popover>
        </div>

        <div className="flex min-h-0 flex-1">
          <div className="flex min-w-0 flex-1 flex-col border-r border-border">
            <div className="flex flex-none items-center gap-1.5 border-b border-border px-3 py-2">
              <TextInput
                leadingIcon="search"
                placeholder="Search components…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                wrapperClassName="min-w-0 flex-1"
                aria-label="Search library components"
              />
            </div>

            <div className="min-h-0 flex-1 overflow-auto p-3">
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
                      <ComponentCard
                        key={key}
                        component={c}
                        selected={isSelected}
                        onClick={() => setSelected(isSelected ? null : c)}
                        testId="library-modal-card"
                      />
                    );
                  })}
                </div>
              )}
            </div>
          </div>

          <div className="flex w-[340px] shrink-0 flex-col overflow-auto p-4">
            {!selected && (
              <div className="flex h-full items-center justify-center text-center text-[12px] text-ink-7">
                Select a component to see details.
              </div>
            )}
            {selected && (
              <ComponentDetailRail
                key={`${selected.id}@${selected.version}`}
                component={selected}
                canPlace={canPlace}
                onPlace={(component) => {
                  armPlacement(component);
                  onClose();
                }}
              />
            )}
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
