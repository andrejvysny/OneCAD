import {
  measurementAnnotationStore,
  useMeasurementAnnotationStore,
  type MeasurementAnnotationRecord,
  type MeasurementAnnotationSlot,
} from "@/stores/measurementAnnotationStore";

const slots: ReadonlyArray<{ id: MeasurementAnnotationSlot; label: string }> = [
  { id: "a", label: "A" },
  { id: "b", label: "B" },
  { id: "pair", label: "Pair" },
];

function annotationStatus(record: MeasurementAnnotationRecord): string | null {
  if (record.manuallyHidden) return "Hidden by you";
  if (record.placement === "no-space") return "Hidden: no clear space";
  if (record.placement === "unknown") return "Waiting for placement";
  return null;
}

function AnnotationRow({ slot, label, record }: {
  slot: MeasurementAnnotationSlot;
  label: string;
  record: MeasurementAnnotationRecord;
}) {
  const status = annotationStatus(record);
  return (
    <div className="min-w-0">
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-1">
        <span className="text-ink-5">{label}</span>
        <div className="flex shrink-0 items-center gap-1">
          <button
            aria-label={`${record.manuallyHidden ? "Show" : "Hide"} ${label} label`}
            className="rounded px-1 text-[10px] text-ink-3 focus-visible:shadow-focus-ring focus-visible:outline-none"
            type="button"
            onClick={() => measurementAnnotationStore.getState().toggleHidden(slot)}
          >
            {record.manuallyHidden ? "Show" : "Hide"}
          </button>
          <button
            type="button"
            aria-label={`${record.pinnedScreenPosition ? "Unpin" : "Pin"} ${label} label`}
            className="rounded px-1 text-[10px] text-ink-3 focus-visible:shadow-focus-ring focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50"
            disabled={!record.pinnedScreenPosition && (!record.hasLivePosition || record.manuallyHidden || record.placement !== "visible")}
            onClick={() => (record.pinnedScreenPosition
              ? measurementAnnotationStore.getState().unpin(slot)
              : measurementAnnotationStore.getState().pin(slot))}
          >
            {record.pinnedScreenPosition ? "Unpin" : "Pin"}
          </button>
        </div>
      </div>
      {status && <span className="block text-[10px] text-warn" role="status">{status}</span>}
    </div>
  );
}

/** Interaction stays with the docked reading, never over measured geometry. */
export function AnnotationControls() {
  const records = useMeasurementAnnotationStore((state) => state.slots);
  const active = slots.filter(({ id }) => records[id].identity !== null);
  if (active.length === 0) return null;
  return (
    <details
      className="pointer-events-auto mt-2 border-t border-border pt-2"
      data-testid="measurement-annotation-controls"
      data-viewport-interactive
    >
      <summary className="cursor-pointer text-ink-5">Annotation labels</summary>
      <div className="mt-1 flex flex-col gap-1">
        {active.map(({ id, label }) => <AnnotationRow key={id} slot={id} label={label} record={records[id]} />)}
      </div>
    </details>
  );
}
