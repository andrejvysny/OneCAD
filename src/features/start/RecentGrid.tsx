import type { RecentProject } from "@/ipc/types";
import { ProjectCard } from "./ProjectCard";

type RecentGridProps = {
  projects: RecentProject[];
  onOpen: (path: string) => void;
  onRename: (path: string, newName: string) => Promise<boolean>;
  onDelete: (path: string) => Promise<boolean>;
  onReveal: (path: string) => void;
};

/** Responsive card grid, ~4-up at the default window width (prototype 1b). */
export function RecentGrid({ projects, onOpen, onRename, onDelete, onReveal }: RecentGridProps) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-4">
      {projects.map((p) => (
        <ProjectCard
          key={p.id}
          project={p}
          onOpen={onOpen}
          onRename={onRename}
          onDelete={onDelete}
          onReveal={onReveal}
        />
      ))}
    </div>
  );
}
