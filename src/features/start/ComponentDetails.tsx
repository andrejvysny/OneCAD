/*
 * The start screen's library-browser details pane (Component Library
 * WP-2.4 follow-up) — read-only identity/parameters/attachments for a
 * selected component. No commit surface: there is no open document to place
 * into from here (see `StartLibraryPanel`'s own doc comment for the boundary).
 */
import type { ReactNode } from "react";
import { Icon } from "@/icons/Icon";
import type { LibraryComponent } from "@/ipc/types";

type ComponentDetailsProps = {
  component: LibraryComponent;
  onClose: () => void;
};

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="mb-2.5">
      <div className="mb-0.5 text-[10.5px] uppercase tracking-[0.06em] text-ink-6">{label}</div>
      <div className="text-[12.5px] text-ink-2">{children}</div>
    </div>
  );
}

export function ComponentDetails({ component, onClose }: ComponentDetailsProps) {
  const freeParams = Object.entries(component.parameters).filter(([, p]) => p.role === "free");

  return (
    <div className="flex w-[280px] shrink-0 flex-col rounded-lg border border-border bg-panel p-4">
      <div className="mb-3 flex items-start gap-2">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-well text-ink-8">
          <Icon name="cube" size={20} strokeWidth={1.5} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13.5px] font-semibold text-ink">{component.name}</div>
          <div className="truncate text-[11px] text-ink-6">{component.id}</div>
        </div>
        <button
          type="button"
          aria-label="Close details"
          onClick={onClose}
          className="flex h-6 w-6 shrink-0 items-center justify-center rounded-sm text-ink-6 hover:bg-well hover:text-ink-2"
        >
          <Icon name="x" size={13} strokeWidth={1.8} />
        </button>
      </div>

      <Row label="Version">{component.version}</Row>
      <Row label="Source">{component.sourceKind}</Row>
      {component.category.length > 0 && <Row label="Category">{component.category.join(", ")}</Row>}
      {component.tags.length > 0 && (
        <Row label="Tags">
          <div className="flex flex-wrap gap-1">
            {component.tags.map((t) => (
              <span key={t} className="rounded-sm bg-chip px-1.5 py-0.5 text-[10.5px] text-ink-3">
                {t}
              </span>
            ))}
          </div>
        </Row>
      )}
      {Object.keys(component.attachments).length > 0 && (
        <Row label="Attachments">
          <ul className="list-inside list-disc">
            {Object.entries(component.attachments).map(([key, a]) => (
              <li key={key} className="truncate">
                {key} — {a.on}
              </li>
            ))}
          </ul>
        </Row>
      )}
      {freeParams.length > 0 && (
        <Row label="Free parameters">
          <ul className="list-inside list-disc">
            {freeParams.map(([key, spec]) => (
              <li key={key}>
                {key}
                {spec.domain ? ` (${spec.domain.map(String).join(" / ")})` : ""}
              </li>
            ))}
          </ul>
        </Row>
      )}
      <div className="mt-1 text-[11px] leading-normal text-ink-7">
        Open a project to place this component.
      </div>
    </div>
  );
}
