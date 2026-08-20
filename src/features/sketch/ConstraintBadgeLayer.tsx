/*
 * ConstraintBadgeLayer — HTML overlay glyphs for the sketch's constraints
 * (H/V, coincident dot, dimensional value), placed at entity midpoints and
 * driven by the engine's HtmlOverlayDriver (per-frame world→screen transforms,
 * no React churn on camera move). Dimensional badges render an editable
 * DimensionInput chip. Shown only in sketch mode.
 *
 * The layer overlays EXACTLY the viewport area (top-0 → bottom-[34px]) so its
 * (0,0)…(w,h) matches the driver's projection space (canvas size).
 *
 * INTERACTIVE (audit A6 / plan item 5). The CONTAINER stays
 * `pointer-events-none` — it covers the whole viewport and must never eat a
 * click aimed at geometry — and each individual badge opts back in with
 * `pointer-events-auto`. A glyph badge is a real `<button>`: click selects the
 * constraint (`sketchSelectionStore.selectedConstraintId`, which `Delete`
 * removes), hover cross-highlights its entities, and hovering an ENTITY tints
 * the badges that reference it (the reverse direction of the same link).
 *
 * GLYPH BADGES GATED TO THE SELECT TOOL (adversarial-review M3). A drawing
 * gesture (line/rect/circle/…) needs every pointerdown/move over the
 * viewport, and a 20px glyph circle sitting ON the geometry swallowed it (a
 * rubber band froze crossing a badge; a click near one selected a constraint
 * instead of placing a point). A glyph badge opts into `pointer-events-auto`
 * — and renders as a real, hoverable `<button>` — ONLY while `sketchTool ===
 * "select"`; every other tool sees an inert `pointer-events-none` span with
 * no handlers wired.
 *
 * The DIMENSIONAL (DimensionInput) chip stays interactive in EVERY tool —
 * reverted from an earlier gated version (M3 follow-up): a value pill is an
 * unambiguous click target (Shapr3D convention: click the label anytime),
 * the click-steal risk M3 flags is specifically the glyph circles sitting on
 * top of geometry, and `e2e/dimension-edit.spec.ts` edits a committed chip
 * right after drawing, with no tool switch to select in between.
 *
 * MACHINE `Fixed` PINS on host-face `referenceLocked` geometry (SKETCH-ON-FACE
 * W2) are non-interactive in EVERY tool, select included — `Delete` on one is
 * a silent no-op (`ConstraintList`'s `visibleConstraints` already hides them
 * from the inspector for the same reason: `deleteConstraints` only accepts an
 * id that list would show). Reused here rather than re-derived so the two
 * surfaces can never disagree about which pins are machine-authored.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { Icon, ICON_MONO } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import { useToolStore } from "@/stores/toolStore";
import { useSketchStore } from "@/stores/sketchStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { sketchSelectionStore, useSketchSelectionStore } from "@/stores/sketchSelectionStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { createClient } from "@/ipc/client";
import { editConstraintValue } from "@/tools/sketch/sketchService";
import { LENGTH_SUFFIX } from "@/units/format";
import { visibleConstraints } from "@/features/inspector/ConstraintList";
import { anchorKey, layoutBadges } from "./badgeLayout";
import { CONSTRAINT_PRESENTATION } from "./constraintCatalog";
import { DimensionInput } from "./DimensionInput";

/** Screen-px clearance from the entity axis to a badge's near edge — the
 *  driver shifts the badge the rest of the way by half its own size. */
const BADGE_OFFSET_PX = 10;

export function ConstraintBadgeLayer() {
  const mode = useToolStore((s) => s.mode);
  const sketchTool = useToolStore((s) => s.sketchTool);
  const showChips = useSettingsStore((s) => s.show.constraintChips);
  const showCoincident = useSettingsStore((s) => s.show.coincidentBadges);
  const session = useSketchStore((s) => s.session);
  const conflictingIds = useSketchStore((s) => s.conflictingIds);
  const selectedConstraintId = useSketchSelectionStore((s) => s.selectedConstraintId);
  const hoveredEntityId = useSketchSelectionStore((s) => s.hover?.entityId ?? null);
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
  // Constraints referencing the entity under the cursor — the REVERSE of the
  // badge-hover → entity-highlight link, so a hovered line says which relations
  // hold it. Entity ids only: a point-pick still tints its owner's badges.
  const relatedToHover = useMemo(() => {
    if (!hoveredEntityId || !session) return new Set<string>();
    return new Set(
      session.constraints.filter((c) => c.entities.includes(hoveredEntityId)).map((c) => c.id),
    );
  }, [hoveredEntityId, session]);

  // Only the select tool gets live GLYPH badges (see the file header, M3);
  // the dimensional chip is unaffected. Machine `Fixed` pins are additionally
  // excluded from THAT — always inert, in every tool — by reusing
  // `ConstraintList`'s own `visibleConstraints` filter: the ids it drops are
  // exactly the ones `Delete` already silently refuses.
  const selectToolActive = sketchTool === "select";
  const machineFixedIds = useMemo(() => {
    if (!session) return new Set<string>();
    const visible = new Set(visibleConstraints(session.constraints, session.entities).map((c) => c.id));
    const hidden = new Set<string>();
    for (const c of session.constraints) if (!visible.has(c.id)) hidden.add(c.id);
    return hidden;
  }, [session]);

  // Ids whose badge appeared since the last pass — the apply-success signal
  // (plan item 10b): a constraint the user just authored flashes once. The
  // FIRST pass of a session only SEEDS the set, so entering a sketch that
  // already has constraints does not set the whole canvas blinking; the seed
  // resets with the session so a re-entered sketch behaves the same way.
  const [flashIds, setFlashIds] = useState<ReadonlySet<string>>(() => new Set());
  const seenIds = useRef<Set<string> | null>(null);
  useEffect(() => {
    if (!session) {
      seenIds.current = null;
      setFlashIds((prev) => (prev.size === 0 ? prev : new Set()));
      return;
    }
    const ids = new Set(badges.map((b) => b.id));
    const seen = seenIds.current;
    seenIds.current = ids;
    if (seen === null) return; // first pass of this session — seed only
    const fresh = [...ids].filter((id) => !seen.has(id));
    if (fresh.length > 0) setFlashIds(new Set(fresh));
  }, [badges, session]);

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
        const isSelected = b.id === selectedConstraintId;
        const label = CONSTRAINT_PRESENTATION[b.kind].label;
        // Machine `Fixed` pins never interact, in ANY tool; every other badge
        // needs the select tool active (see the file header, M3).
        const badgeInteractive = selectToolActive && !machineFixedIds.has(b.id);
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
            ) : badgeInteractive ? (
              <button
                type="button"
                data-testid={`constraint-badge-${b.id}`}
                data-selected={isSelected || undefined}
                aria-label={label}
                aria-pressed={isSelected}
                title={label}
                style={staggerStyle}
                onClick={() =>
                  sketchSelectionStore
                    .getState()
                    .setSelectedConstraint(isSelected ? null : b.id)
                }
                onMouseEnter={() => sketchSelectionStore.getState().setConstraintHover(b.id)}
                onMouseLeave={() => sketchSelectionStore.getState().setConstraintHover(null)}
                className={cn(
                  // 20px box / 14px glyph: the old 14px box with a 10px glyph
                  // was the "near-invisible grey dash" of audit A6, and it was
                  // also below any reasonable click target.
                  "pointer-events-auto inline-flex h-5 w-5 cursor-pointer items-center justify-center rounded-full leading-none focus-visible:shadow-focus-ring focus-visible:outline-none",
                  flashIds.has(b.id) && "badge-mount-flash",
                  // Conflict outranks selection: a red badge is the solver
                  // complaining, and tinting it accent would hide that.
                  isConflicting
                    ? `bg-danger-surface text-traffic-close ${ICON_MONO}`
                    : isSelected
                      ? "bg-sel-bg text-sel-text ring-1 ring-accent"
                      : relatedToHover.has(b.id)
                        ? "bg-sel-bg/70 text-accent"
                        : "bg-canvas/60 text-ink-2 hover:bg-hover-3",
                )}
              >
                {/* Same icon set as the toolbar/context-chips/Inspector (design
                    item: one constraint icon system, not a canvas-only text
                    glyph). Fixed still reads as a padlock on the canvas
                    (Shapr3D convention) — the anchor icon stays for the
                    toolbar/inspector list, which predate this badge and don't
                    have this space pressure. */}
                <Icon
                  name={b.kind === "Fixed" ? "lock" : CONSTRAINT_PRESENTATION[b.kind].icon}
                  size={14}
                  strokeWidth={1.8}
                />
              </button>
            ) : (
              <span
                data-testid={`constraint-badge-${b.id}`}
                title={label}
                style={staggerStyle}
                className={cn(
                  "pointer-events-none inline-flex h-5 w-5 items-center justify-center rounded-full leading-none",
                  isConflicting
                    ? `bg-danger-surface text-traffic-close ${ICON_MONO}`
                    : "bg-canvas/60 text-ink-2",
                )}
              >
                <Icon
                  name={b.kind === "Fixed" ? "lock" : CONSTRAINT_PRESENTATION[b.kind].icon}
                  size={14}
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
