import { createStore, useStore } from "zustand";

export type MeasurementAnnotationSlot = "a" | "b" | "pair";
export type AnnotationPlacementStatus = "visible" | "no-space" | "unknown";

export interface MeasurementAnnotationRecord {
  identity: string | null;
  manuallyHidden: boolean;
  pinnedScreenPosition: { x: number; y: number } | null;
  placement: AnnotationPlacementStatus;
  hasLivePosition: boolean;
}

type AnnotationSlots = Record<MeasurementAnnotationSlot, MeasurementAnnotationRecord>;

interface MeasurementAnnotationState {
  slots: AnnotationSlots;
  reconcile(identities: Partial<Record<MeasurementAnnotationSlot, string | null>>): void;
  toggleHidden(slot: MeasurementAnnotationSlot): void;
  pin(slot: MeasurementAnnotationSlot): void;
  unpin(slot: MeasurementAnnotationSlot): void;
  setPlacement(slot: MeasurementAnnotationSlot, placement: AnnotationPlacementStatus): void;
  setLivePosition(slot: MeasurementAnnotationSlot, position: { x: number; y: number } | null): void;
  reset(): void;
}

const slots = ["a", "b", "pair"] as const;
const livePositions = new Map<MeasurementAnnotationSlot, { x: number; y: number }>();

function emptyRecord(identity: string | null = null): MeasurementAnnotationRecord {
  return { identity, manuallyHidden: false, pinnedScreenPosition: null, placement: "unknown", hasLivePosition: false };
}

function emptySlots(): AnnotationSlots {
  return { a: emptyRecord(), b: emptyRecord(), pair: emptyRecord() };
}

function changedRecord(a: MeasurementAnnotationRecord, b: MeasurementAnnotationRecord): boolean {
  return a.identity !== b.identity || a.manuallyHidden !== b.manuallyHidden
    || a.pinnedScreenPosition?.x !== b.pinnedScreenPosition?.x
    || a.pinnedScreenPosition?.y !== b.pinnedScreenPosition?.y || a.placement !== b.placement
    || a.hasLivePosition !== b.hasLivePosition;
}

export const measurementAnnotationStore = createStore<MeasurementAnnotationState>()((set, get) => ({
  slots: emptySlots(),
  reconcile(identities) {
    const previous = get().slots;
    const next = {} as AnnotationSlots;
    let changed = false;
    for (const slot of slots) {
      const identity = identities[slot] ?? null;
      next[slot] = previous[slot].identity === identity ? previous[slot] : emptyRecord(identity);
      if (previous[slot].identity !== identity) livePositions.delete(slot);
      changed ||= changedRecord(previous[slot], next[slot]);
    }
    if (changed) set({ slots: next });
  },
  toggleHidden(slot) {
    const manuallyHidden = !get().slots[slot].manuallyHidden;
    if (manuallyHidden) livePositions.delete(slot);
    set((state) => ({
      slots: {
        ...state.slots,
        [slot]: { ...state.slots[slot], manuallyHidden, hasLivePosition: manuallyHidden ? false : state.slots[slot].hasLivePosition },
      },
    }));
  },
  pin(slot) {
    const position = livePositions.get(slot);
    if (!position) return;
    set((state) => ({ slots: { ...state.slots, [slot]: { ...state.slots[slot], pinnedScreenPosition: position } } }));
  },
  unpin(slot) {
    set((state) => ({ slots: { ...state.slots, [slot]: { ...state.slots[slot], pinnedScreenPosition: null } } }));
  },
  setPlacement(slot, placement) {
    if (get().slots[slot].placement === placement) return;
    set((state) => ({ slots: { ...state.slots, [slot]: { ...state.slots[slot], placement } } }));
  },
  setLivePosition(slot, position) {
    if (position) livePositions.set(slot, position);
    else livePositions.delete(slot);
    if (get().slots[slot].hasLivePosition === (position !== null)) return;
    set((state) => ({ slots: { ...state.slots, [slot]: { ...state.slots[slot], hasLivePosition: position !== null } } }));
  },
  reset() {
    livePositions.clear();
    set({ slots: emptySlots() });
  },
}));

export function useMeasurementAnnotationStore<T>(selector: (state: MeasurementAnnotationState) => T): T {
  return useStore(measurementAnnotationStore, selector);
}
