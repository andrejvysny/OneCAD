import { createStore, useStore } from "zustand";
import type { ActiveToolPresentation } from "@/tools/modelTools/activeToolPresentation";

export type OperationAttemptPhase = "applying" | "completed" | "failed";

export interface OperationAttempt {
  token: number;
  documentId?: string;
  phase: OperationAttemptPhase;
  message?: string;
  presentation: ActiveToolPresentation | null;
}

interface OperationAttemptState {
  attempt: OperationAttempt | null;
  begin(documentId: string | undefined, presentation: ActiveToolPresentation | null): number | null;
  settle(token: number, phase: Exclude<OperationAttemptPhase, "applying">, message?: string): void;
  clear(): void;
}

let nextToken = 1;

export const operationAttemptStore = createStore<OperationAttemptState>()((set, get) => ({
  attempt: null,
  begin(documentId, presentation) {
    const active = get().attempt;
    if (active?.phase === "applying" && active.documentId === documentId) return null;
    const token = nextToken++;
    set({ attempt: { token, documentId, phase: "applying", presentation } });
    return token;
  },
  settle(token, phase, message) {
    if (get().attempt?.token !== token) return;
    set({ attempt: { ...get().attempt!, phase, ...(message ? { message } : {}) } });
  },
  clear() {
    set({ attempt: null });
  },
}));

export function useOperationAttemptStore<T>(selector: (state: OperationAttemptState) => T): T {
  return useStore(operationAttemptStore, selector);
}
