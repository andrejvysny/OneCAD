/*
 * ModelToolChips — the COMPACT PARAMETER LABEL for an armed model operation
 * (spec §4.1, review R05).
 *
 * It holds exactly one thing: the parameter's identity, its value and its unit —
 * `23.09 mm`, `Total 18 mm`, `R 2 mm`, `65°`, `Spacing 20 mm`. Operation title,
 * mode readouts, result summary, More and the single Done/Cancel pair belong to
 * `ModelOperationBar`; targets, secondary parameters and detailed validation
 * belong to `ActiveToolInspector`. It used to carry all of that plus a drag grip
 * and a Dock/Return row, which is what R05 calls "the main source of floating
 * clutter"; placement now lives behind the strip's More menu, and the grip
 * appears only during an explicit "Place label".
 *
 * Content is React; POSITIONING is imperative — an engine-owned host node is
 * registered with the HTML overlay driver so it tracks a world anchor every
 * frame with no React re-render.
 *
 * The host node is created once (never part of React's managed layout); the
 * engine appends it to the overlay and the driver transforms it. We `createPortal`
 * the label content INTO that host, so React only manages the content, never the
 * moved node — avoiding the "removeChild: not a child" reconciliation crash, and
 * letting the same input node survive being reparented into the inspector with
 * its draft, caret, validity and focus intact (spec §10.4).
 */
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/ui/cn";
import { DimensionInput } from "@/features/sketch/DimensionInput";
import { GearChipCluster } from "./GearChipCluster";
import { useToolChipStore, toolChipStore, MODEL_TOOL_CHIP_ID } from "@/stores/toolChipStore";
import { LENGTH_SUFFIX } from "@/units/format";
import { useViewportEngine } from "@/viewport/engineBridge";
import { activeToolPresentation } from "@/tools/modelTools/activeToolPresentation";
import { useToolChipPlacement, toolChipPlacementStore } from "@/stores/toolChipPlacementStore";
import { useViewportWorkArea } from "@/stores/viewportWorkAreaStore";
import { useInspectorLayoutStore } from "@/stores/inspectorLayoutStore";
import { useToolStore } from "@/stores/toolStore";
import { useToolChipDockHost } from "./toolChipDockBridge";
import { requestConfirm } from "./requestConfirm";

const CHIP_ID = MODEL_TOOL_CHIP_ID;

/** The host group's `aria-label` (WP-U10), one per `ToolChipKind`. Names the
 *  CLUSTER, not the value inside it — the value's own field carries that name. */
const CHIP_GROUP_LABEL: Record<string, string> = {
  extrudeDepth: "Extrude",
  revolveAngle: "Revolve",
  revolveAxisPick: "Revolve",
  datumOffset: "Datum plane",
  regionSelect: "Region select",
  filletRadius: "Fillet or chamfer",
  shellThickness: "Shell",
  offsetFace: "Offset face",
  hole: "Hole",
  gear: "Gear",
  sketchValue: "Sketch edit",
  dimension: "Dimension",
  linearPattern: "Linear pattern",
  circularPattern: "Circular pattern",
  transform: "Move or rotate",
  mirror: "Mirror",
  booleanOp: "Boolean operation",
};

/**
 * Is the fallback slot actually usable RIGHT NOW (§4.4)?
 *
 * `InspectorPanel` renders its content — and with it the dock host — `hidden` +
 * `inert` while the drawer is closed, so docking there would move the only
 * primary field somewhere the user can neither see nor reach. An old placement
 * choice must never make the controls disappear, so an unreachable host simply
 * is not a fallback and the label stays on its anchor.
 */
function isHostReachable(host: HTMLElement | null): boolean {
  if (!host?.isConnected) return false;
  for (let el: HTMLElement | null = host; el !== null; el = el.parentElement) {
    if (el.hasAttribute("hidden") || el.hasAttribute("inert")) return false;
  }
  return true;
}

/**
 * Escape in a primary field (spec §9.2): drop the field's raw error, then restore
 * the edit-start value through the tool's registered handler, or — until a tool
 * registers one — clear the controller's range validation and re-preview it.
 */
function revertPrimary(fieldId: string, value: number, text: string): void {
  const s = toolChipStore.getState();
  s.setRawValueValidity(fieldId, true, text);
  if (s.onRevertValue) {
    s.onRevertValue(value);
    return;
  }
  s.clearValidation();
  s.onValue?.(value);
}

export function ModelToolChips() {
  const engine = useViewportEngine();
  const kind = useToolChipStore((s) => s.kind);
  const value = useToolChipStore((s) => s.value);
  const count = useToolChipStore((s) => s.count);
  const edgeOp = useToolChipStore((s) => s.edgeOp);
  const transformMode = useToolChipStore((s) => s.transformMode);
  const distanceType = useToolChipStore((s) => s.distanceType);
  const valueError = useToolChipStore((s) => s.valueError);
  const endCondition = useToolChipStore((s) => s.endCondition);
  const symmetric = useToolChipStore((s) => s.symmetric);
  const axis = useToolChipStore((s) => s.axis);
  const suffix = useToolChipStore((s) => s.suffix);
  const label = useToolChipStore((s) => s.label);
  const worldPos = useToolChipStore((s) => s.worldPos);
  const anchorAxisFrom = useToolChipStore((s) => s.anchorAxisFrom);
  const anchorOffsetPx = useToolChipStore((s) => s.anchorOffsetPx);
  /** Type-to-enter arm (U3): remounting on `token` is what focuses the field, and
   *  `seed` is the character that replaces the formatted value. */
  const primaryEntry = useToolChipStore((s) => s.primaryEntry);
  const chipState = useToolChipStore((s) => s);
  const presentation = useMemo(() => activeToolPresentation(chipState), [chipState]);
  const placement = useToolChipPlacement((state) => state.placement);
  const autoFallback = useToolChipPlacement((state) => state.autoFallback);
  const placing = useToolChipPlacement((state) => state.placing);
  const dockHost = useToolChipDockHost();
  // The drawer's open flag is what drives the host's `hidden`/`inert`, so it is
  // also the re-render trigger that makes the DOM check below run again.
  const inspectorOpen = useInspectorLayoutStore((state) => state.open);
  const safeRect = useViewportWorkArea((state) => state.obstacleClearRect);
  const viewport = useViewportWorkArea((state) => state.viewport);
  const gestureLive = useToolStore((s) => s.gestureLive);
  const applying = presentation?.phase === "applying";
  // A plain DOM host, created once; the engine owns its DOM position.
  //
  // `role="group"` + `aria-hidden="false"` (below, alongside the tool's own
  // `aria-label`) pull this subtree back OUT of the decorative overlay's hidden
  // state: the engine's chip layer no longer carries `aria-hidden` (WP-U10,
  // `ViewportEngine.chipLayer()`) so a role/name query — or a screen reader —
  // can reach a label's field, while `overlayRef`'s canvas decoration in
  // `ViewportRoot` stays hidden. `aria-hidden="false"` on the host itself is
  // belt-and-braces: aria-hidden is a PARENT-overrides-child state (an
  // ancestor's `true` wins regardless), so the layer change is what actually
  // does the work, not this attribute alone.
  const [host] = useState(() => {
    const el = document.createElement("div");
    el.dataset.testid = "model-tool-chip";
    el.setAttribute("role", "group");
    el.setAttribute("aria-hidden", "false");
    return el;
  });

  /*
   * A gesture has ENDED, so whatever the value reads now came from the drag and
   * not from this field. Bumping the token makes the field re-display it even
   * when the echo guard would swallow the write (H7): a drag clamped back onto
   * the number a refused draft had previewed used to leave that draft on screen.
   */
  const [valueEcho, setValueEcho] = useState(0);
  const wasGestureLive = useRef(gestureLive);
  useEffect(() => {
    const was = wasGestureLive.current;
    wasGestureLive.current = gestureLive;
    // ONLY the falling edge. Bumping on mount too would re-display `value` over a
    // type-to-enter seed the instant the field was created, which is the one
    // moment the field's own text outranks the store.
    if (was && !gestureLive) setValueEcho((n) => n + 1);
  }, [gestureLive]);

  const anchorKey = worldPos ? worldPos.join(",") : "";
  const dockReachable = inspectorOpen && isHostReachable(dockHost);
  const effectiveMode = (placement.mode === "docked" || autoFallback) && dockReachable
    ? "docked"
    : placement.mode === "floating"
      ? "floating"
      : "anchored";
  const activeDockHost = effectiveMode === "docked" ? dockHost : null;
  const dragPoint = useRef<{ x: number; y: number } | null>(null);
  const dragOffset = useRef<{ x: number; y: number } | null>(null);
  const dragPointerId = useRef<number | null>(null);
  const dragOrigin = useRef<typeof placement | null>(null);
  const focusedElement = useRef<HTMLElement | null>(null);
  const [placementStatus, setPlacementStatus] = useState<{
    kind: typeof kind;
    anchorKey: string;
    width: number;
    height: number;
    fits: boolean | null;
  } | null>(null);
  const onPlacementStatus = useCallback((status: { width: number; height: number; fits: boolean | null }) => {
    setPlacementStatus((previous) => {
      const next = { kind, anchorKey, ...status };
      return previous?.kind === next.kind && previous.anchorKey === next.anchorKey
        && previous.width === next.width && previous.height === next.height && previous.fits === next.fits
        ? previous
        : next;
    });
    if (status.fits === null) engine?.invalidate();
  }, [anchorKey, engine, kind]);

  useLayoutEffect(() => {
    if (!engine || kind === "none" || !worldPos) return;
    const restoreFocus = () => {
      focusedElement.current?.focus({ preventScroll: true });
      focusedElement.current = null;
    };
    const preserveFocus = () => {
      const active = document.activeElement;
      if (active instanceof HTMLElement && host.contains(active)) focusedElement.current = active;
    };
    // `anchorKey` is the ARM's anchor, not a live one: a label whose anchor tracks
    // the gesture (extrude) is moved by the controller through `engine.moveChip`,
    // never by a store write — this effect UNMOUNTS on every change, and a label
    // detached mid-drag loses input focus. See the header of `toolChipStore`.
    if (effectiveMode === "docked" && activeDockHost) {
      engine.unmountChip(CHIP_ID, host);
      host.style.position = "";
      host.style.transform = "";
      host.style.display = "";
      activeDockHost.appendChild(host);
      restoreFocus();
      return () => {
        preserveFocus();
        engine.unmountChip(CHIP_ID, host);
        if (host.parentElement === activeDockHost) host.remove();
      };
    }
    engine.mountChip(CHIP_ID, host, worldPos, {
      axisFrom: anchorAxisFrom ?? undefined,
      offsetPx: anchorOffsetPx || undefined,
      screenPosition: placement.mode === "floating" ? { x: placement.x, y: placement.y } : undefined,
      constrainToSafeRect: true,
      onPlacementStatus,
      // The label and the value arrow share this anchor, so without this the
      // label sits ON the arrow and every press meant for the arrow hits the
      // label instead (measured: the arrow's grab pixel resolved to
      // `chip-cancel`).
      avoidValueHandle: true,
    });
    restoreFocus();
    return () => {
      preserveFocus();
      engine.unmountChip(CHIP_ID, host);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine, kind, anchorKey, host, effectiveMode, activeDockHost, placement, onPlacementStatus]);

  /*
   * The automatic fallback (N8, spec §4.4). A label with no floating footprint
   * moves to the stable slot for THIS arm only — `setAutoFallback` is transient
   * and never touches the user's own `placement`, which is what used to leave
   * every later operation docked after one bad fit. An explicit `docked`
   * preference needs no fallback; an unreachable host is not one.
   */
  useLayoutEffect(() => {
    if (kind === "none" || placement.mode === "docked" || viewport.width <= 0) return;
    const currentStatus = placementStatus?.kind === kind && placementStatus.anchorKey === anchorKey
      ? placementStatus
      : null;
    const width = currentStatus?.width ?? host.offsetWidth;
    const height = currentStatus?.height ?? host.offsetHeight;
    const hasFootprint = currentStatus?.fits !== false
      && (safeRect === null || (width <= safeRect.width && height <= safeRect.height));
    const store = toolChipPlacementStore.getState();
    if (!hasFootprint && dockReachable) {
      if (!store.autoFallback) store.setAutoFallback(true);
    } else if (store.autoFallback && hasFootprint) {
      store.setAutoFallback(false);
    }
  }, [anchorKey, dockReachable, host, kind, placement.mode, placementStatus, safeRect, viewport.width]);

  if (kind === "none" || !worldPos) return null;

  /*
   * Every armed model-operation numeric field, in one place (U3).
   *
   *   - `onPreview` fires on every parseable keystroke, so the viewport follows
   *     the typing. It routes to `onValue`, the same channel a drag uses, so the
   *     controller's existing coalescing/fencing applies unchanged.
   *   - `commitOnBlur` is OFF: an armed model tool commits on Enter or Done only.
   *     Nothing is lost, because the value already went out through `onPreview`.
   *   - `primaryEntry` seeds + focuses the field when the user typed on canvas.
   *   - `disabled` while applying: the controller ignores edits during a commit,
   *     so accepting them would be a lie (H4b follow-up).
   */
  const primaryField = (
    suffix: string,
    fieldLabel: string,
    opts?: { fieldId?: string },
  ) => (
    <DimensionInput
      key={primaryEntry ? `primary-${primaryEntry.token}` : `primary-${anchorKey}`}
      value={value}
      suffix={suffix}
      label={fieldLabel}
      variant="label"
      disabled={applying}
      redisplayToken={valueEcho}
      initialText={primaryEntry?.seed}
      autoFocus={primaryEntry !== null}
      commitOnBlur={false}
      onValidityChange={(valid, draft) =>
        toolChipStore.getState().setRawValueValidity(opts?.fieldId ?? "primary", valid, draft)
      }
      onPreview={(v) => toolChipStore.getState().onValue?.(v)}
      onCommit={(v) => toolChipStore.getState().onValue?.(v)}
      onConfirm={requestConfirm}
      onEscapeRevert={(v, text) => revertPrimary(opts?.fieldId ?? "primary", v, text)}
    />
  );

  /** The short identity that precedes the number, or nothing when the unit
   *  already says what the value is (spec §4.1's `23.09 mm` / `65°`). */
  const prefix = (testid: string, text: string) => (
    <span data-testid={testid} className="whitespace-nowrap text-[12px] font-medium text-ink-4">
      {text}
    </span>
  );

  /*
   * The label frame (spec §4.5): 30 px high, 8 px horizontal padding, one tone
   * change for an invalid value. No grip, no Dock row, no summary, no validation
   * row, no confirmation — those are the strip's and the inspector's.
   */
  const labelFrame = (children: React.ReactNode) => (
    <div
      data-testid="operation-label"
      className={cn(
        "pointer-events-auto inline-flex h-[30px] min-w-0 max-w-full items-center gap-1.5 rounded-md border bg-surface px-2 shadow-popover",
        effectiveMode === "docked" && "w-full",
        valueError ? "border-warn-border" : "border-border",
        applying && "opacity-60",
      )}
    >
      {/* The repositioning grip exists only while "Place label" is running
          (§4.4) — it was a permanent header row on every operation before. */}
      {placing && effectiveMode !== "docked" && (
        <button
          type="button"
          data-testid="chip-drag-handle"
          data-viewport-interactive
          aria-label="Move tool controls"
          title="Drag the label, then release to place it"
          className="cursor-move rounded-full bg-chip px-1.5 text-[11px] text-ink-4"
          onPointerDown={(event) => {
            if (event.button !== 0 || dragPointerId.current !== null) return;
            event.preventDefault();
            event.stopPropagation();
            event.currentTarget.setPointerCapture(event.pointerId);
            const point = engine?.clientToViewport(event.clientX, event.clientY) ?? null;
            const rect = host.getBoundingClientRect();
            const center = engine?.clientToViewport(rect.left + rect.width / 2, rect.top + rect.height / 2) ?? null;
            dragPointerId.current = event.pointerId;
            dragOrigin.current = placement;
            dragOffset.current = point && center
              ? { x: point.x - center.x, y: point.y - center.y }
              : null;
            dragPoint.current = center;
          }}
          onPointerMove={(event) => {
            if (dragPointerId.current !== event.pointerId || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
            event.preventDefault();
            event.stopPropagation();
            const point = engine?.clientToViewport(event.clientX, event.clientY) ?? null;
            const offset = dragOffset.current;
            const center = point && offset ? { x: point.x - offset.x, y: point.y - offset.y } : null;
            dragPoint.current = center;
            if (center) engine?.setChipScreenPosition(CHIP_ID, center);
          }}
          onPointerUp={(event) => {
            if (dragPointerId.current !== event.pointerId) return;
            event.preventDefault();
            event.stopPropagation();
            if (event.currentTarget.hasPointerCapture(event.pointerId)) {
              event.currentTarget.releasePointerCapture(event.pointerId);
            }
            const point = dragPoint.current;
            dragPoint.current = null;
            dragOffset.current = null;
            dragPointerId.current = null;
            dragOrigin.current = null;
            if (point) toolChipPlacementStore.getState().floatAt(point.x, point.y);
            else toolChipPlacementStore.getState().setPlacing(false);
          }}
          onPointerCancel={(event) => {
            if (dragPointerId.current !== event.pointerId) return;
            event.stopPropagation();
            if (event.currentTarget.hasPointerCapture(event.pointerId)) {
              event.currentTarget.releasePointerCapture(event.pointerId);
            }
            const origin = dragOrigin.current;
            if (origin?.mode === "floating") engine?.setChipScreenPosition(CHIP_ID, { x: origin.x, y: origin.y });
            else engine?.setChipScreenPosition(CHIP_ID, null);
            dragPoint.current = null;
            dragOffset.current = null;
            dragPointerId.current = null;
            dragOrigin.current = null;
          }}
          onLostPointerCapture={(event) => {
            if (dragPointerId.current !== event.pointerId) return;
            const origin = dragOrigin.current;
            if (origin?.mode === "floating") engine?.setChipScreenPosition(CHIP_ID, { x: origin.x, y: origin.y });
            else engine?.setChipScreenPosition(CHIP_ID, null);
            dragPoint.current = null;
            dragOffset.current = null;
            dragPointerId.current = null;
            dragOrigin.current = null;
          }}
        >
          ⋮⋮
        </button>
      )}
      {children}
    </div>
  );

  let content: React.ReactNode = null;
  if (kind === "extrudeDepth") {
    // A distance is meaningless for the non-Blind end conditions — the kernel
    // derives it — so there is no parameter to label at all, and an empty label
    // over the geometry is exactly the clutter §4.1 removes.
    content = endCondition === "Blind"
      ? labelFrame(
          <>
            {symmetric && prefix("chip-extrude-prefix", "Total")}
            {primaryField(LENGTH_SUFFIX, `Depth (${LENGTH_SUFFIX})`)}
          </>,
        )
      : null;
  } else if (kind === "revolveAngle") {
    content = labelFrame(primaryField("°", "Angle (°)"));
  } else if (kind === "revolveAxisPick") {
    // No value armed yet — the anchored prompt for the pick that is outstanding.
    content = labelFrame(
      <span data-testid="chip-revolve-axis-hint" className="whitespace-nowrap text-[12px] font-medium text-ink-2">
        Pick an axis line
      </span>,
    );
  } else if (kind === "datumOffset") {
    content = labelFrame(
      <>
        {prefix("chip-datum-base", label)}
        {primaryField(LENGTH_SUFFIX, `Offset (${LENGTH_SUFFIX})`)}
      </>,
    );
  } else if (kind === "regionSelect") {
    content = labelFrame(
      <span data-testid="chip-region-count" className="whitespace-nowrap text-[12px] font-medium text-ink-2">
        {count} region{count === 1 ? "" : "s"}
      </span>,
    );
  } else if (kind === "filletRadius" || kind === "shellThickness") {
    content = labelFrame(
      <>
        {prefix(
          "chip-edgeop-prefix",
          kind === "shellThickness" ? "Thickness" : edgeOp === "Chamfer" ? "C" : "R",
        )}
        {primaryField(
          LENGTH_SUFFIX,
          kind === "shellThickness"
            ? `Thickness (${LENGTH_SUFFIX})`
            : edgeOp === "Chamfer"
              ? `Distance (${LENGTH_SUFFIX})`
              : `Radius (${LENGTH_SUFFIX})`,
        )}
      </>,
    );
  } else if (kind === "offsetFace") {
    // SCHEMA §7.3 OffsetFace. The distance TYPE is parameter identity, not an
    // operation mode: it says which distance the number is.
    content = labelFrame(
      <>
        {prefix("chip-offset-badge", distanceType)}
        {primaryField(LENGTH_SUFFIX, `Offset (${LENGTH_SUFFIX})`)}
      </>,
    );
  } else if (kind === "hole") {
    content = labelFrame(
      <>
        {prefix("chip-hole-prefix", "⌀")}
        {primaryField(LENGTH_SUFFIX, `Hole diameter (${LENGTH_SUFFIX})`, { fieldId: "hole-diameter" })}
      </>,
    );
  } else if (kind === "gear") {
    content = labelFrame(<GearChipCluster />);
  } else if (kind === "sketchValue") {
    // WP-C T2b: an armed sketch EDIT tool's live parameter (fillet radius /
    // offset distance). It commits nothing — the geometry commits on a viewport
    // click — so the label stays open across repeated applies and is keyed on the
    // ANCHOR alone: it mounts (and auto-focuses) once per arm, so the value can
    // be typed straight away without the field re-grabbing focus after each edit.
    content = labelFrame(
      <>
        {prefix("chip-sketch-label", label)}
        <DimensionInput
          key={`sketchValue-${anchorKey}`}
          value={value}
          suffix={LENGTH_SUFFIX}
          label={`${label} (${LENGTH_SUFFIX})`}
          variant="label"
          autoFocus
          onCommit={(v) => toolChipStore.getState().onValue?.(v)}
        />
      </>,
    );
  } else if (kind === "dimension") {
    // Sketch Dimension tool: seeded + auto-focused; Enter commits, Esc cancels,
    // and a canvas click must NOT blur-commit (a 2nd line click upgrades a length
    // into an angle), so `commitOnBlur` is off. Keying by anchor+value remounts
    // (re-focuses) on each new pick — e.g. when a length upgrades to an angle —
    // but stays stable while typing (the value prop is unchanged mid-edit).
    content = (
      <DimensionInput
        key={`dim-${anchorKey}-${value}`}
        value={value}
        suffix={suffix}
        autoFocus
        commitOnBlur={false}
        onCommit={(v) => toolChipStore.getState().onValue?.(v)}
        onCancel={() => toolChipStore.getState().onCancel?.()}
      />
    );
  } else if (kind === "linearPattern") {
    content = labelFrame(
      <>
        {prefix("chip-pattern-prefix", "Spacing")}
        {primaryField(LENGTH_SUFFIX, `Spacing (${LENGTH_SUFFIX})`)}
      </>,
    );
  } else if (kind === "circularPattern") {
    content = labelFrame(primaryField("°", "Angle (°)"));
  } else if (kind === "transform") {
    // Armed placement (WP-B W1): the axis names which component the number
    // addresses — spec §4.1's `X 10 mm`.
    content = labelFrame(
      <>
        {prefix("chip-transform-prefix", axis)}
        {primaryField(
          transformMode === "rotate" ? "°" : LENGTH_SUFFIX,
          transformMode === "rotate" ? "Angle (°)" : `Distance (${LENGTH_SUFFIX})`,
        )}
      </>,
    );
  }
  // `mirror` and `booleanOp` have no parameter at the feature: their whole
  // authoring surface is the operation strip and the inspector, so they render
  // no floating label at all rather than an empty pill over the geometry.

  host.setAttribute("aria-label", `${CHIP_GROUP_LABEL[kind] ?? "Model tool"} options`);
  return createPortal(content, host);
}
