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
import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon } from "@/icons/Icon";
import { TextInput } from "@/ui/TextInput";
import { Button } from "@/ui/Button";
import { createClient } from "@/ipc/client";
import type { LibraryComponent } from "@/ipc/types";
import { tasksStore } from "@/stores/tasksStore";
import { ComponentThumbnail } from "./ComponentThumbnail";
import { ComponentPreview3D } from "./ComponentPreview3D";
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

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="mb-2.5">
      <div className="mb-0.5 text-[10.5px] uppercase tracking-[0.06em] text-ink-6">{label}</div>
      <div className="text-[12.5px] text-ink-2">{children}</div>
    </div>
  );
}

export function LibraryModal({ open, onClose, canPlace, initialSelection }: LibraryModalProps) {
  const [components, setComponents] = useState<LibraryComponent[]>([]);
  const [state, setState] = useState<LoadState>("loading");
  const [query, setQuery] = useState("");
  const [reindexing, setReindexing] = useState(false);
  const [selected, setSelected] = useState<LibraryComponent | null>(null);

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
          <button
            type="button"
            aria-label="Close library"
            onClick={onClose}
            className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-6 hover:bg-well hover:text-ink-2"
          >
            <Icon name="x" size={13} strokeWidth={1.8} />
          </button>
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
              <Button
                variant="secondary"
                onClick={() => void reindex()}
                disabled={reindexing}
                aria-label="Reindex library"
              >
                {reindexing ? "Reindexing…" : "Reindex"}
              </Button>
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
                      <button
                        key={key}
                        type="button"
                        data-testid="library-modal-card"
                        data-selected={isSelected || undefined}
                        onClick={() => setSelected(isSelected ? null : c)}
                        title={c.id}
                        className={`flex flex-col gap-1 rounded-lg border p-2 text-left transition-[box-shadow,border-color] duration-150 hover:border-card-hover-border hover:shadow-card-hover ${
                          isSelected ? "border-accent ring-1 ring-accent" : "border-border bg-surface"
                        }`}
                      >
                        <ComponentThumbnail
                          componentId={c.id}
                          componentVersion={c.version}
                          size={192}
                          className="aspect-square w-full rounded bg-well object-contain"
                        />
                        <div className="truncate text-[12px] font-semibold text-ink">{c.name}</div>
                        <div className="truncate text-[10.5px] text-ink-6">{c.version}</div>
                      </button>
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
              <>
                <div className="mb-3 flex items-center gap-2">
                  <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-well text-ink-8">
                    <Icon name="cube" size={20} strokeWidth={1.5} />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13.5px] font-semibold text-ink">{selected.name}</div>
                    <div className="truncate text-[11px] text-ink-6">{selected.id}</div>
                  </div>
                </div>

                {/* The live, orbitable preview (WP-B6) — one WebGL context, mounted
                    only while a component is selected in this modal. */}
                <div className="mb-3 h-[260px]">
                  <ComponentPreview3D componentId={selected.id} componentVersion={selected.version} fill />
                </div>

                <Row label="Version">{selected.version}</Row>
                <Row label="Source">{selected.sourceKind}</Row>
                {selected.category.length > 0 && (
                  <Row label="Category">{selected.category.join(", ")}</Row>
                )}
                {selected.tags.length > 0 && (
                  <Row label="Tags">
                    <div className="flex flex-wrap gap-1">
                      {selected.tags.map((t) => (
                        <span key={t} className="rounded-sm bg-chip px-1.5 py-0.5 text-[10.5px] text-ink-3">
                          {t}
                        </span>
                      ))}
                    </div>
                  </Row>
                )}
                {Object.keys(selected.attachments).length > 0 && (
                  <Row label="Attachments">
                    <ul className="list-inside list-disc">
                      {Object.entries(selected.attachments).map(([key, a]) => (
                        <li key={key} className="truncate">
                          {key} — {a.on}
                        </li>
                      ))}
                    </ul>
                  </Row>
                )}
                {Object.entries(selected.parameters).filter(([, p]) => p.role === "free").length > 0 && (
                  <Row label="Free parameters">
                    <ul className="list-inside list-disc">
                      {Object.entries(selected.parameters)
                        .filter(([, p]) => p.role === "free")
                        .map(([key, spec]) => (
                          <li key={key}>
                            {key}
                            {spec.domain ? ` (${spec.domain.map(String).join(" / ")})` : ""}
                          </li>
                        ))}
                    </ul>
                  </Row>
                )}

                {canPlace ? (
                  <Button
                    variant="primary"
                    className="mt-1"
                    onClick={() => {
                      armPlacement(selected);
                      onClose();
                    }}
                  >
                    Place in scene
                  </Button>
                ) : (
                  <div className="mt-1 text-[11px] leading-normal text-ink-7">
                    Open a project to place this component.
                  </div>
                )}
              </>
            )}
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
