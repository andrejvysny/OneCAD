import { useRef } from "react";
import { Icon } from "@/icons/Icon";
import { MonoValue } from "@/ui/MonoValue";
import { Popover } from "@/ui/Popover";
import { SectionLabel } from "@/ui/SectionLabel";
import { useTasksStore, type BackgroundTask } from "@/stores/tasksStore";

/**
 * Status-bar background-tasks chip + its popover (prototype 2a).
 *
 * Renders nothing while nothing is running, so the status bar stays quiet in
 * the normal case. The chip is the ONLY affordance that says "the app is busy
 * with something you started and can keep working through" — a modal spinner
 * would say the opposite.
 *
 * A task with no `progress` draws an indeterminate bar rather than a guess.
 * See `tasksStore` for why nothing reports a percentage yet.
 */
export function TasksChip() {
  const tasks = useTasksStore((s) => s.tasks);
  const open = useTasksStore((s) => s.open);
  const toggle = useTasksStore((s) => s.toggleOpen);
  const setOpen = useTasksStore((s) => s.setOpen);
  const anchor = useRef<HTMLButtonElement | null>(null);

  if (tasks.length === 0) return null;

  return (
    <>
      <button
        ref={anchor}
        type="button"
        aria-expanded={open}
        aria-label={`${tasks.length} background ${tasks.length === 1 ? "task" : "tasks"}`}
        data-testid="tasks-chip"
        onClick={toggle}
        className="ml-1.5 flex h-[22px] cursor-pointer items-center gap-1.5 rounded-[11px] border border-border-strong bg-surface px-2.5 font-ui text-[11.5px] font-semibold text-accent hover:bg-hover focus-visible:shadow-focus-ring focus-visible:outline-none"
      >
        <Icon name="tasks" size={11} strokeWidth={2.4} className="animate-spin" />
        {tasks.length} {tasks.length === 1 ? "task" : "tasks"}
      </button>

      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={anchor}
        placement="top-start"
        width={270}
        className="px-3.5 py-3"
      >
        <SectionLabel className="mb-2">Background tasks</SectionLabel>
        {tasks.map((task) => (
          <TaskRow key={task.id} task={task} />
        ))}
      </Popover>
    </>
  );
}

function TaskRow({ task }: { task: BackgroundTask }) {
  const pct = task.progress === undefined ? null : Math.round(task.progress * 100);
  return (
    <div className="mb-3 last:mb-0" data-testid={`task-${task.id}`}>
      <div className="flex items-center gap-2 text-[12.5px] text-ink-2">
        <span className="flex-1 truncate">{task.label}</span>
        <MonoValue className="text-[11.5px]">{pct === null ? "…" : `${pct}%`}</MonoValue>
      </div>
      <div
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct ?? undefined}
        className="mt-1.5 h-1 overflow-hidden rounded-sm bg-canvas"
      >
        <div
          className={pct === null ? "h-1 w-1/3 animate-pulse rounded-sm bg-accent" : "h-1 rounded-sm bg-accent"}
          style={pct === null ? undefined : { width: `${pct}%` }}
        />
      </div>
    </div>
  );
}
