import { useState } from "react";
import { cn } from "@/ui/cn";
import { Icon } from "@/icons/Icon";
import { SettingsModal } from "@/features/settings/SettingsModal";
import { TasksChip } from "@/features/tasks/TasksChip";
import { SlotHost, Slots } from "@/platform";
import { SegmentedToggle } from "@/ui/SegmentedToggle";
import { MonoValue } from "@/ui/MonoValue";
import { useToolStore } from "@/stores/toolStore";
import { useSelectionStore, primarySelection } from "@/stores/selectionStore";
import { useDocumentStore } from "@/stores/documentStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useWorkerStore, type WorkerLifecycleState } from "@/stores/workerStore";
import {
  useViewportStore,
  formatCursor,
  formatCursorPlane,
  type Projection,
} from "@/stores/viewportStore";

/**
 * 34px status bar (prototype 1c): status text · DOF (warn when >0) · spacer ·
 * Persp/Ortho toggle · FOV (dimmed in ortho) · live mono X/Y/Z read-out.
 */
export function StatusBar() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const mode = useToolStore((s) => s.mode);
  const sel = useSelectionStore(primarySelection);
  const projection = useViewportStore((s) => s.projection);
  const setProjection = useViewportStore((s) => s.setProjection);
  const fov = useViewportStore((s) => s.fov);
  const cursor = useViewportStore((s) => s.cursor);
  const cursorPlaneUV = useViewportStore((s) => s.cursorPlaneUV);
  const statusHint = useViewportStore((s) => s.statusHint);
  const statusHintSeq = useViewportStore((s) => s.statusHintSeq);
  const workerState = useWorkerStore((s) => s.state);
  // H7b: > 0 while a regen job is out (real lane: regen-started/finished; mock:
  // the same transitions around its synchronous apply).
  const regenBusy = useDocumentStore((s) => s.regenBusy);
  const activeSketchId = useViewportStore((s) => s.activeSketchId);
  // WP-C2: the cursor read-out is mm in the store and display-unit on screen.
  // Subscribing here is what repaints the row when the unit picker moves.
  const displayUnit = useSettingsStore((s) => s.displayUnit);
  const activeSketch = useDocumentStore((s) =>
    activeSketchId ? s.sketches[activeSketchId] : undefined,
  );

  const sketching = mode === "sketch";
  /*
   * DOF belongs to a SKETCH, so the read-out appears only when one is in scope
   * and reads that sketch's own number (LGU-1 WP-A, F1's neighbour). Model
   * mode reads the SELECTED sketch's own `dof` field rather than the live
   * `dofBadge` (which is written only by an active sketch session and would
   * otherwise leak whichever sketch was last edited next to an unrelated
   * selection — selecting a FACE used to light it up too, and a face has no
   * degrees of freedom).
   */
  const selectedSketchId =
    sel?.kind === "sketchRegion" ? sel.sketchId : sel?.kind === "sketch" ? sel.id : null;
  const selectedSketch = useDocumentStore((s) =>
    selectedSketchId ? s.sketches[selectedSketchId] : undefined,
  );
  // In sketch mode the DOF number already has a home: `SketchChromeBar`'s
  // status pill. Showing it a SECOND time here was never a UX call — it was
  // kept only because ~47 e2e specs polled this exact node as a settle gate;
  // that gate now lives on `SketchChromeBar`'s own `data-testid="sketch-dof"`
  // probe (`e2e/helpers.ts`'s `dofPill`), so this row is free to drop its
  // sketch-mode copy. The model-mode case (a sketch/region SELECTED, not
  // being edited) is the only place StatusBar remains the sole DOF surface.
  const dof = sketching ? null : (selectedSketch?.dof ?? null);
  const showDof = dof !== null;
  const persp = projection === "persp";
  const statusLeft = sketching
    ? `Sketch mode — ${activeSketch?.name ?? "Sketch"}`
    : "Ready";

  return (
    <div className="absolute inset-x-0 bottom-0 z-[26] flex h-[34px] items-center gap-3 border-t border-border bg-statusbar px-3.5 text-[12px]">
      <span className="text-ink-3">{statusLeft}</span>
      {statusHint && (
        <>
          <span aria-hidden="true" className="h-[14px] w-px bg-border" />
          {/* Keyed by MESSAGE + SEQ, so a new error remounts this node and its
              one-shot CSS pulse plays again (audit A10: an error used only to
              recolor text in the far corner, which is easy to miss) — the seq
              half is what makes a REPEATED identical error re-pulse too, since
              a same-message remount would otherwise reuse the node and stay
              still. The pulse is inert under prefers-reduced-motion. */}
          <span
            key={`${statusHint.message}:${statusHintSeq}`}
            data-testid="status-hint"
            className={cn(
              statusHint.severity === "error"
                ? "hint-error-pulse text-traffic-close"
                : "text-ink-5",
            )}
          >
            {statusHint.message}
          </span>
        </>
      )}
      {regenBusy > 0 && (
        <>
          <span aria-hidden="true" className="h-[14px] w-px bg-border" />
          <span role="status" data-testid="regen-busy" className="text-ink-4">
            Rebuilding…
          </span>
        </>
      )}
      <WorkerStatusIndicator state={workerState} />
      {/* Only when a sketch/region is SELECTED in model mode — never while
          sketching (see `dof` above) and never a bare "DOF: —" placeholder
          for state that has no DOF at all (a body, a face, nothing
          selected): the whole segment, separator included, is absent then. */}
      {showDof && (
        <>
          <span aria-hidden="true" className="h-[14px] w-px bg-border" />
          <span className="font-medium text-dof-neutral">DOF: {dof}</span>
        </>
      )}
      {/* Renders nothing while nothing is running (prototype 2a). */}
      <TasksChip />
      <SlotHost slot={Slots.StatusSection} />
      <span className="flex-1" />
      {/* Persp/Ortho + FOV are dead controls while sketching — sketch mode
          force-sets ortho on entry and restores the prior projection on exit
          (SketchController), so both would just be misleading here. */}
      {!sketching && (
        <>
          <SegmentedToggle
            ariaLabel="Projection"
            size="sm"
            value={projection}
            onChange={(p: Projection) => setProjection(p)}
            options={[
              { value: "persp", label: "Persp" },
              { value: "ortho", label: "Ortho" },
            ]}
          />
          <span
            data-testid="fov"
            className="text-ink-5"
            style={{ opacity: persp ? 1 : 0.35 }}
          >
            FOV <MonoValue>{fov}°</MonoValue>
          </span>
          <span aria-hidden="true" className="h-[14px] w-px bg-border" />
        </>
      )}
      <MonoValue className="whitespace-pre text-[11.5px]">
        {/* Design item 11 / audit A8: on the sketch plane world XYZ is not
            the sketch's own axes (XY plane's u is world +Y) — while sketching
            the read-out switches to the plane-local U/V the controller itself
            draws against, same unit formatting as the world X/Y/Z it replaces. */}
        {sketching ? formatCursorPlane(cursorPlaneUV, displayUnit) : formatCursor(cursor, displayUnit)}
      </MonoValue>
      {/* APPLICATION settings, so shell chrome — it was mounted from
          `ModelTreePanel`, which made "which application-wide preferences exist"
          a model-tree responsibility and gave the tree a reason to import a
          feature it has nothing to do with. */}
      <span aria-hidden="true" className="h-[14px] w-px bg-border" />
      <button
        type="button"
        aria-label="Open settings"
        onClick={() => setSettingsOpen(true)}
        className="flex h-[22px] w-[22px] cursor-pointer items-center justify-center rounded-sm border-none bg-transparent text-ink-4 hover:bg-hover hover:text-ink-2"
      >
        <Icon name="settings" size={14} strokeWidth={1.8} />
      </button>
      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}

/**
 * Worker-status indicator (a small dot + label). Shown ONLY for the attention
 * states — the sidecar restarting (amber) or failed (red); the healthy
 * starting/ready path stays quiet. The mock never emits, so this renders nothing
 * in a plain browser / vitest.
 */
function WorkerStatusIndicator({ state }: { state: WorkerLifecycleState }) {
  if (state !== "restarting" && state !== "failed") return null;
  const failed = state === "failed";
  return (
    <>
      <span aria-hidden="true" className="h-[14px] w-px bg-border" />
      <span
        role="status"
        className={cn("flex items-center gap-1.5", failed ? "text-traffic-close" : "text-warn")}
      >
        <span
          aria-hidden="true"
          className={cn("h-[7px] w-[7px] rounded-full", failed ? "bg-traffic-close" : "bg-warn")}
        />
        {failed ? "Worker offline" : "Worker restarting…"}
      </span>
    </>
  );
}
