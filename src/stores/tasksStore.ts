/*
 * Background tasks — the status-bar chip and its popover (prototype 2a).
 *
 * The chip exists so long work stops being invisible: today a rebuild shows one
 * word ("Rebuilding…") and an export shows nothing at all, which is how a user
 * ends up wondering whether the app is stuck.
 *
 * PROGRESS IS OPTIONAL, AND THAT IS THE HONEST PART. Nothing in the stack
 * reports a percentage yet — regen is a request/response over OCW1 with no
 * intermediate frames, and the worker does not stream progress. So a task
 * whose `progress` is undefined renders an indeterminate bar rather than a
 * fabricated number. When the protocol grows progress frames, the producer
 * starts passing `progress` and the UI needs no change.
 *
 * The store is a plain registry keyed by an opaque id: whoever starts long work
 * calls `begin`, and `end` is idempotent so a failure path can call it blindly.
 */
import { createStore, useStore } from "zustand";

export interface BackgroundTask {
  readonly id: string;
  readonly label: string;
  /** 0..1, or undefined when the producer cannot say. */
  readonly progress?: number;
}

export interface TasksState {
  tasks: readonly BackgroundTask[];
  /** Popover over the chip. */
  open: boolean;

  begin(id: string, label: string, progress?: number): void;
  setProgress(id: string, progress: number | undefined): void;
  end(id: string): void;
  setOpen(open: boolean): void;
  toggleOpen(): void;
}

export const tasksStore = createStore<TasksState>((set) => ({
  tasks: [],
  open: false,

  begin: (id, label, progress) =>
    set((s) => ({
      // Re-beginning an id REPLACES it rather than duplicating: a producer that
      // retries must not leave a ghost row behind.
      tasks: [...s.tasks.filter((t) => t.id !== id), { id, label, progress }],
    })),

  setProgress: (id, progress) =>
    set((s) => ({
      tasks: s.tasks.map((t) => (t.id === id ? { ...t, progress } : t)),
    })),

  end: (id) =>
    set((s) => {
      const tasks = s.tasks.filter((t) => t.id !== id);
      // Closing the popover as the last task drains avoids leaving an empty
      // panel anchored to a chip that is no longer there.
      return tasks.length === 0 ? { tasks, open: false } : { tasks };
    }),

  setOpen: (open) => set({ open }),
  toggleOpen: () => set((s) => ({ open: !s.open })),
}));

export function useTasksStore<T>(selector: (s: TasksState) => T): T {
  return useStore(tasksStore, selector);
}
