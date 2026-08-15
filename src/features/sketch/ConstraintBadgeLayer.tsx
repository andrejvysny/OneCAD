/*
 * ConstraintBadgeLayer — HTML overlay glyphs for the sketch's constraints
 * (H/V, coincident dot, dimensional value), placed at entity midpoints and
 * driven by the engine's HtmlOverlayDriver (per-frame world→screen transforms,
 * no React churn on camera move). Dimensional badges render an editable
 * DimensionInput chip. Shown only in sketch mode.
 *
 * The layer overlays EXACTLY the viewport area (top-0 → bottom-[34px]) so its
 * (0,0)…(w,h) matches the driver's projection space (canvas size).
 */
import { useEffect, useMemo, useRef } from "react";
import { Icon, ICON_MONO } from "@/icons/Icon";
import { useToolStore } from "@/stores/toolStore";
import { useSketchStore } from "@/stores/sketchStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { createClient } from "@/ipc/client";
import { editConstraintValue } from "@/tools/sketch/sketchService";
import { LENGTH_SUFFIX } from "@/units/format";
import { anchorKey, layoutBadges } from "./badgeLayout";
import { CONSTRAINT_PRESENTATION } from "./constraintCatalog";
import { DimensionInput } from "./DimensionInput";

/** Screen-px clearance from the entity axis to a badge's near edge — the
 *  driver shifts the badge the rest of the way by half its own size. */
const BADGE_OFFSET_PX = 10;

export function ConstraintBadgeLayer() {
  const mode = useToolStore((s) => s.mode);
  const showChips = useSettingsStore((s) => s.show.constraintChips);
  const showCoincident = useSettingsStore((s) => s.show.coincidentBadges);
  const session = useSketchStore((s) => s.session);
  const conflictingIds = useSketchStore((s) => s.conflictingIds);
  const engine = useViewportEngine();
  const clientRef = useRef<ReturnType<typeof createClient> | null>(null);
  if (!clientRef.current) {
    try {
      clientRef.current = createClient();
    } catch {
      clientRef.current = null;
    }
  }

  // Constraint ids the solver reports in conflict (SCHEMA §7.4) — tint their
  // badges. Computed before the Coincident filter below: a conflicting
  // Coincident is diagnostic information, not clutter, and must survive that
  // filter even when the preference is off (P1 fix — hiding a constraint the
  // solver is complaining about would silently drop the only on-canvas sign
  // something is wrong there).
  const conflicting = useMemo(() => new Set(conflictingIds), [conflictingIds]);
  // A shared vertex already communicates "these meet here" — a Coincident
  // badge on top of it adds little and is the single biggest source of
  // per-corner clutter (Sketcher UX cleanup, Track B2). Hidden by default,
  // opt back in via the snap popover's "Coincident badges" row — UNLESS the
  // solver is reporting it in conflict, which always shows through.
  const badges = useMemo(() => {
    const all = layoutBadges(session);
    if (showCoincident) return all;
    return all.filter((b) => b.kind !== "Coincident" || conflicting.has(b.id));
  }, [session, showCoincident, conflicting]);
  const plane = session?.plane ?? null;
  const refs = useRef(new Map<string, HTMLDivElement>());

  // Register each badge wrapper with the overlay driver (it owns positioning).
  useEffect(() => {
    if (!engine || !plane || mode !== "sketch" || !showChips) return;
    const overlay = engine.overlay;
    const ids: string[] = [];
    for (const b of badges) {
      const el = refs.current.get(b.id);
      if (!el) continue;
      // `axisFrom` ties the badge to its owning entity with a leader line and
      // a direction-correct perpendicular offset (a leader-anchored badge
      // reads as "belonging to that line/circle" even at a busy shared
      // vertex); `clusterId` keeps co-anchored badges (same quantized point)
      // a floor apart on top of that.
      overlay.register(b.id, el, planePointToWorld(plane, b.at), {
        axisFrom: b.axisFrom ? planePointToWorld(plane, b.axisFrom) : undefined,
        offsetPx: BADGE_OFFSET_PX,
        clusterId: anchorKey(b.at),
      });
      ids.push(b.id);
    }
    engine.invalidate();
    return () => {
      for (const id of ids) overlay.unregister(id);
    };
  }, [engine, plane, badges, mode, showChips]);

  if (mode !== "sketch" || !session || !showChips) return null;

  return (
    <div
      data-testid="constraint-badges"
      className="pointer-events-none absolute inset-x-0 bottom-[34px] top-0 z-[3] overflow-hidden"
    >
      {badges.map((b) => {
        const isConflicting = conflicting.has(b.id);
        // Co-anchored badges (same quantized anchor point, U9) stagger into a
        // row via offsetIndex instead of stacking directly on top of each
        // other — the overlay driver owns `transform` on the wrapper div
        // below (per-frame world→screen), so the stagger lives on the inner
        // span as a plain marginLeft that composes with it untouched. The
        // driver itself now supplies the direction-correct perpendicular
        // nudge (via `axisFrom`/`offsetPx` on `overlay.register`) — a fixed
        // CSS translate here would be wrong whenever the entity isn't
        // oriented the way a fixed up-right nudge assumes.
        const staggerStyle = b.offsetIndex > 0 ? { marginLeft: b.offsetIndex * 16 } : undefined;
        return (
          <div
            key={b.id}
            ref={(el) => {
              if (el) refs.current.set(b.id, el);
              else refs.current.delete(b.id);
            }}
          >
            {b.editable && b.value !== undefined ? (
              <span
                style={staggerStyle}
                className={`inline-block${isConflicting ? " rounded-sm border border-traffic-close" : ""}`}
              >
                <DimensionInput
                  value={b.value}
                  // A length badge names the LENGTH domain, not a unit —
                  // DimensionInput swaps in whatever the display unit is.
                  suffix={b.kind === "Angle" ? "" : LENGTH_SUFFIX}
                  kind={b.kind}
                  onCommit={(v) => {
                    if (clientRef.current) void editConstraintValue(clientRef.current, b.id, v);
                  }}
                />
              </span>
            ) : (
              <span
                title={b.kind}
                style={staggerStyle}
                className={`inline-flex h-3.5 w-3.5 items-center justify-center rounded-full leading-none ${
                  isConflicting ? `bg-danger-surface text-traffic-close ${ICON_MONO}` : "bg-canvas/60 text-ink-4"
                }`}
              >
                {/* Same icon set as the toolbar/context-chips/Inspector (design
                    item: one constraint icon system, not a canvas-only text
                    glyph). Fixed still reads as a padlock on the canvas
                    (Shapr3D convention) — the anchor icon stays for the
                    toolbar/inspector list, which predate this badge and don't
                    have this space pressure. */}
                <Icon
                  name={b.kind === "Fixed" ? "lock" : CONSTRAINT_PRESENTATION[b.kind].icon}
                  size={10}
                  strokeWidth={1.8}
                />
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}
