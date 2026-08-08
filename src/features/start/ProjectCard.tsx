import { useEffect, useRef, useState } from "react";
import type { RecentProject } from "@/ipc/types";
import { Icon } from "@/icons/Icon";
import { MenuItem } from "@/ui/MenuItem";
import { Popover } from "@/ui/Popover";
import { TextInput } from "@/ui/TextInput";
import { cn } from "@/ui/cn";

type ProjectCardProps = {
  project: RecentProject;
  onOpen: (path: string) => void;
  /** Rename on disk (card menu). Resolves `false` on failure — the input reverts. */
  onRename: (path: string, newName: string) => Promise<boolean>;
  /** Move to the OS trash (card menu, two-click confirm). */
  onDelete: (path: string) => Promise<boolean>;
  /** Reveal the file in the OS file manager (card menu). */
  onReveal: (path: string) => void;
};

// Faint 45° hatch that fills the empty preview well (prototype 1a, line 76).
// Token-tinted so it inverts with the theme (see VIEWPORT_HATCH).
const PREVIEW_HATCH =
  "repeating-linear-gradient(45deg, var(--color-hatch) 0 10px, transparent 10px 20px)";

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/**
 * One recent-project tile: a preview well over a name + path/date meta line,
 * plus an overflow menu (rename / reveal / delete). Hover lifts the card with an
 * accent-tinted border + soft shadow.
 *
 * The overflow trigger + its `Popover` are SIBLINGS of the open-on-click area,
 * not descendants of it: `Popover` portals into `document.body`, and React
 * bubbles synthetic events through the COMPONENT tree even across a portal —
 * nesting the menu inside the clickable card would fire `onOpen` on every menu
 * click. Kept as siblings, a menu click never reaches the card's handler at all.
 */
export function ProjectCard({ project, onOpen, onRename, onDelete, onReveal }: ProjectCardProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [editing, setEditing] = useState(false);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const input = useRef<HTMLInputElement | null>(null);
  // Guards the blur handler: Enter/Escape both settle the edit and then blur, and
  // a second commit off that blur would either double-write or resurrect a
  // cancelled name (same guard as TreeRow's inline rename).
  const settled = useRef(false);

  useEffect(() => {
    if (!editing) {
      settled.current = false;
      return;
    }
    const el = input.current;
    el?.focus();
    el?.select();
  }, [editing]);

  const closeMenu = () => {
    setMenuOpen(false);
    setConfirmDelete(false);
  };

  const commitRename = () => {
    if (settled.current) return;
    settled.current = true;
    const value = (input.current?.value ?? project.name).trim();
    setEditing(false);
    if (value && value !== project.name) void onRename(project.path, value);
  };
  const cancelRename = () => {
    if (settled.current) return;
    settled.current = true;
    setEditing(false);
  };

  return (
    <div className="group relative">
      <div
        role="button"
        tabIndex={0}
        title={project.path}
        onClick={editing ? undefined : () => onOpen(project.path)}
        onKeyDown={
          editing
            ? undefined
            : (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onOpen(project.path);
                }
              }
        }
        className="block w-full cursor-pointer overflow-hidden rounded-lg border border-border bg-surface text-left transition-[box-shadow,border-color] duration-150 hover:border-card-hover-border hover:shadow-card-hover focus-visible:shadow-focus-ring focus-visible:outline-none"
      >
        <div
          className="flex h-24 items-center justify-center border-b border-border-subtle bg-well"
          style={project.thumbnail ? undefined : { backgroundImage: PREVIEW_HATCH }}
        >
          {project.thumbnail ? (
            <img
              src={project.thumbnail}
              alt=""
              className="h-full w-full object-cover"
            />
          ) : (
            <span className="font-mono text-[10px] text-ink-7">model preview</span>
          )}
        </div>
        <div className="px-[11px] pb-[10px] pt-[9px]">
          {editing ? (
            <TextInput
              ref={input}
              defaultValue={project.name}
              aria-label={`Rename ${project.name}`}
              wrapperClassName="-my-px"
              className="h-[22px] px-1.5 text-[13px] font-semibold"
              onClick={(e) => e.stopPropagation()}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") commitRename();
                else if (e.key === "Escape") cancelRename();
              }}
              onBlur={commitRename}
            />
          ) : (
            <div className="truncate text-[13px] font-semibold text-ink">{project.name}</div>
          )}
          <div className="mt-[3px] flex gap-1.5 text-[11px] text-ink-5">
            <span className="flex-1 truncate">{project.path}</span>
            <span className="shrink-0">{formatDate(project.modifiedAt)}</span>
          </div>
        </div>
      </div>

      <button
        ref={trigger}
        type="button"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label={`${project.name} options`}
        onClick={(e) => {
          e.stopPropagation();
          setMenuOpen((v) => !v);
        }}
        className={cn(
          "absolute right-1.5 top-1.5 flex h-6 w-6 cursor-pointer items-center justify-center rounded-sm text-ink-6 hover:bg-well hover:text-ink-2 focus-visible:shadow-focus-ring focus-visible:outline-none",
          menuOpen ? "bg-well text-ink-2 opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100",
        )}
      >
        <Icon name="overflow" size={14} strokeWidth={1.8} />
      </button>

      <Popover
        open={menuOpen}
        onClose={closeMenu}
        anchorRef={trigger}
        placement="bottom-end"
        width={170}
        className="p-1"
      >
        <MenuItem
          label="Rename"
          data-testid="project-card-rename"
          onClick={() => {
            setMenuOpen(false);
            setEditing(true);
          }}
        />
        <MenuItem
          label="Reveal in Finder"
          data-testid="project-card-reveal"
          onClick={() => {
            closeMenu();
            onReveal(project.path);
          }}
        />
        <div aria-hidden="true" className="my-1 h-px bg-border" />
        {confirmDelete ? (
          <MenuItem
            label="Confirm delete"
            danger
            data-testid="project-card-delete-confirm"
            onClick={() => {
              closeMenu();
              void onDelete(project.path);
            }}
          />
        ) : (
          <MenuItem
            label="Delete"
            danger
            data-testid="project-card-delete"
            onClick={() => setConfirmDelete(true)}
          />
        )}
      </Popover>
    </div>
  );
}
