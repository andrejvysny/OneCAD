/*
 * Start-screen Templates row (spec §8; Component Library WP-B3) — the nav key
 * that has been stubbed since the start screen shipped.
 *
 * Card anatomy mirrors `ProjectCard`'s (preview well over a name + meta line)
 * without reusing the component: a template card has no path, no date, and no
 * rename/reveal/delete menu — every prop `ProjectCard` requires describes a
 * file on disk that the user owns, and a template is neither. Sharing the shape
 * without sharing the affordances is the honest half to share.
 */
import type { ProjectTemplate } from "@/ipc/types";
import { Icon } from "@/icons/Icon";

// The same faint 45° hatch `ProjectCard` fills an empty preview well with.
const PREVIEW_HATCH =
  "repeating-linear-gradient(45deg, var(--color-hatch) 0 10px, transparent 10px 20px)";

export interface TemplateGridProps {
  templates: readonly ProjectTemplate[];
  /** Starts a NEW untitled document from the template — never opens the template itself. */
  onUse: (id: string) => void;
}

export function TemplateGrid({ templates, onUse }: TemplateGridProps) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-4">
      {templates.map((template) => (
        <button
          key={template.id}
          type="button"
          data-testid="template-card"
          onClick={() => onUse(template.id)}
          title={template.description ?? template.name}
          className="group flex flex-col overflow-hidden rounded-lg border border-border bg-surface text-left transition hover:border-accent hover:shadow-card"
        >
          <div
            className="flex h-[120px] items-center justify-center border-b border-border bg-well text-ink-7"
            style={template.previewDataUrl ? undefined : { backgroundImage: PREVIEW_HATCH }}
          >
            {template.previewDataUrl ? (
              <img
                src={template.previewDataUrl}
                alt=""
                className="h-full w-full object-cover"
              />
            ) : (
              <Icon name="cube" size={26} strokeWidth={1.4} />
            )}
          </div>
          <div className="min-w-0 px-3 py-2.5">
            <div className="truncate text-[12.5px] font-medium text-ink-2">{template.name}</div>
            <div className="truncate text-[11.5px] text-ink-6">
              {template.description ?? "Starts a new untitled project"}
            </div>
          </div>
        </button>
      ))}
    </div>
  );
}
