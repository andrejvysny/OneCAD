/*
 * ConstraintContextChips (UI b) — a small floating chip row anchored near the
 * selection centroid, offering the SAME applicable set as the toolbar (shared
 * `useApplicableConstraints`) as one-click chips. Positioned like the constraint
 * badges: a single host node registered with the engine's HtmlOverlayDriver (the
 * driver projects the centroid world→screen each frame, no React churn on camera
 * move), floated a fixed offset above via CSS. Hidden when the selection or the
 * applicable set is empty, so Esc / selection-change dismisses it naturally.
 */
import { Icon } from "@/icons/Icon";
import { useEffect, useRef } from "react";
import { useToolStore } from "@/stores/toolStore";
import { useSketchStore } from "@/stores/sketchStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { createClient } from "@/ipc/client";
import { applyConstraint, deleteConstraints } from "@/tools/sketch/sketchService";
import { constraintShortcutLabel } from "@/modules/modeling/bindings";
import { useApplicableConstraints } from "./useApplicableConstraints";
import { CONSTRAINT_PRESENTATION } from "./constraintCatalog";

const OVERLAY_ID = "__constraint_context_chips";

export function ConstraintContextChips() {
  const mode = useToolStore((s) => s.mode);
  const session = useSketchStore((s) => s.session);
  const engine = useViewportEngine();
  const { applicables, centroid, existingFixed } = useApplicableConstraints();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const clientRef = useRef<ReturnType<typeof createClient> | null>(null);
  if (!clientRef.current) {
    try {
      clientRef.current = createClient();
    } catch {
      clientRef.current = null;
    }
  }

  const plane = session?.plane ?? null;
  const active =
    mode === "sketch" && !!session && !!centroid && (applicables.length > 0 || !!existingFixed);

  // Register the host with the overlay driver at the selection centroid.
  useEffect(() => {
    const el = hostRef.current;
    if (!engine || !plane || !centroid || !active || !el) return;
    const overlay = engine.overlay;
    overlay.register(OVERLAY_ID, el, planePointToWorld(plane, centroid));
    engine.invalidate();
    return () => overlay.unregister(OVERLAY_ID);
  }, [engine, plane, centroid, active]);

  if (!active) return null;

  return (
    <div
      data-testid="constraint-context-chips"
      className="pointer-events-none absolute inset-x-0 bottom-[34px] top-0 z-[4] overflow-hidden"
    >
      <div ref={hostRef}>
        <div className="pointer-events-auto inline-flex -translate-x-1/2 -translate-y-9 items-center gap-0.5 rounded-md border border-border bg-surface p-0.5 shadow-panel">
          {existingFixed ? (
            // Already pinned (e.g. drawn onto the origin) — offer removal
            // instead of a redundant second "Fixed", the anchor icon read as
            // a lock the same way a Fixed badge does on the canvas.
            <button
              type="button"
              aria-label="Disconnect"
              title="Disconnect"
              onClick={() => {
                if (clientRef.current) void deleteConstraints(clientRef.current, [existingFixed.id]);
              }}
              className="flex h-6 items-center gap-1 rounded-sm px-1.5 leading-none text-ink-4 hover:bg-hover-3 hover:text-traffic-close focus-visible:shadow-focus-ring focus-visible:outline-none"
            >
              <Icon name="unlock" size={15} strokeWidth={1.7} />
              <span className="text-[11px]">Disconnect</span>
            </button>
          ) : null}
          {applicables
            .filter((a) => !(existingFixed && a.type === "Fixed"))
            .map((a, i) => {
              const { icon, label } = CONSTRAINT_PRESENTATION[a.type];
              // The chip is icon-only, so its tooltip is the only place the
              // chord can be discovered from the canvas (plan item 6).
              const chord = constraintShortcutLabel(a.type);
              return (
                <button
                  key={`${a.type}-${i}`}
                  type="button"
                  aria-label={label}
                  title={chord ? `${label} (${chord})` : label}
                  onClick={() => {
                    if (clientRef.current) void applyConstraint(clientRef.current, a);
                  }}
                  className="flex h-6 min-w-6 cursor-pointer items-center justify-center rounded-sm px-1 leading-none text-ink-4 hover:bg-hover-3 hover:text-accent focus-visible:shadow-focus-ring focus-visible:outline-none"
                >
                  <Icon name={icon} size={15} strokeWidth={1.7} />
                </button>
              );
            })}
        </div>
      </div>
    </div>
  );
}
