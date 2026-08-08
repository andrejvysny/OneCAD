/*
 * ViewportRoot — the React bridge to ViewportEngine (imperative Three.js).
 *
 * Owns the <canvas> + an absolutely-positioned overlay layer (pointer-events
 * none; overlay children opt back in). Manages the engine lifecycle
 * (StrictMode-safe) and wires stores both directions:
 *   store → engine : projection, gridVisible
 *   engine → store : cameraViewLabel (on camera change), cursor (pointer raycast
 *                    onto Z=0)
 * A hatched placeholder is shown until the engine is ready (and stays if WebGL
 * is unavailable, e.g. jsdom), so the shell degrades gracefully.
 */
import { useOptionalPlatform } from "@/platform";
import { getDatumVisuals } from "@/modules/modeling/datumViewport";
import { useEffect, useRef, useState } from "react";
import { cn } from "@/ui/cn";
import { logWarn } from "@/debug/log";
import { ViewportEngine } from "./engine/ViewportEngine";
import { resetPaletteCache } from "./engine/palette";
import { getResolvedTheme, subscribeResolvedTheme } from "@/theme/themeController";
import { secondaryHitWins, type PickHit } from "./engine/Picker";
import { MeshIngest } from "./mesh/meshSync";
import { SketchStaticSync } from "./sketchStaticSync";
import { DatumSync } from "./datumSync";
import { setViewportEngine } from "./engineBridge";
import { viewLabelForDirection } from "@/features/viewcube/ViewCube";
import { useViewportStore, viewportStore } from "@/stores/viewportStore";
import { selectGeometryCached, useDocumentStore } from "@/stores/documentStore";
import { settingsStore } from "@/stores/settingsStore";
import { toolStore } from "@/stores/toolStore";
import {
  selectionStore,
  sketchRegionRef,
  topoRefId,
  type EntityRef,
} from "@/stores/selectionStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { sketchStore } from "@/stores/sketchStore";
import { createClient } from "@/ipc/client";
import { promoteOne } from "@/ipc/promote";
import { SketchController } from "@/tools/sketch/SketchController";
import { ModelToolController } from "@/tools/modelTools/ModelToolController";
import { setModelToolController } from "@/tools/modelTools/modelToolBridge";
import type { SketchStaticHit } from "./engine/SketchStaticLayer";

/** A face/edge PickHit → a selection ref (carries the anchor for AcquireElementIds). */
function refFromHit(hit: PickHit): EntityRef {
  return {
    kind: hit.kind,
    id: topoRefId(hit.bodyId, hit.topoKey),
    bodyId: hit.bodyId,
    topoKey: hit.topoKey,
    elementId: hit.elementId,
    anchor: { worldPoint: [hit.worldPos.x, hit.worldPos.y, hit.worldPos.z] },
  };
}

function refFromSketchStaticHit(hit: SketchStaticHit): EntityRef {
  return hit.kind === "sketchRegion"
    ? sketchRegionRef(hit.sketchId, hit.regionId)
    : { kind: "sketch", id: hit.sketchId };
}

/**
 * Arbitrate a body-face/edge hit against a coplanar sketch fill under the same
 * pointer. Default: the sketch wins a numeric tie (`secondaryHitWins`) so a
 * sketch lying flush on a body face stays the easier target. `pickThrough`
 * (Alt held) suppresses that tie-break — the body wins outright, and the
 * sketch is only used when there is no body hit at all (W0.4).
 */
export function refFromModelHits(
  bodyHit: PickHit | null,
  sketchHit: SketchStaticHit | null,
  pickThrough: boolean,
): EntityRef | null {
  if (
    !pickThrough &&
    sketchHit &&
    (!bodyHit || secondaryHitWins(bodyHit.distance, sketchHit.distance))
  ) {
    return refFromSketchStaticHit(sketchHit);
  }
  if (bodyHit) return refFromHit(bodyHit);
  return sketchHit ? refFromSketchStaticHit(sketchHit) : null;
}

/**
 * Which geometry-status chip the viewport overlays, if any.
 *
 * Two facts, one slot, so the precedence has to be explicit:
 *  - PENDING WINS. Meshes are actively landing, so labelling the frame "last
 *    saved" would be stale before it finished rendering. It still defers to a
 *    sticky error hint (a body-load failure is the more actionable signal), and
 *    that suppression does NOT promote `cached` — the geometry is mid-swap
 *    either way.
 *  - CACHED does NOT defer to the error hint. Cached geometry plus a regen
 *    failure is precisely the state the user must be told about: the hint says
 *    the rebuild failed, the chip says what is on screen instead. Hiding one
 *    would leave stale geometry looking live.
 */
export function viewportGeometryChip(
  ready: boolean,
  geometryPending: boolean,
  errorHintActive: boolean,
  geometryCached: boolean,
): "pending" | "cached" | null {
  if (!ready) return null;
  if (geometryPending) return errorHintActive ? null : "pending";
  return geometryCached ? "cached" : null;
}

/**
 * Promote a face/edge pick to a persistent Rust-minted ElementId (SCHEMA §7.5)
 * and write it back onto the still-selected ref. Fire-and-forget; a failed / stale
 * promotion leaves the transient topoKey ref intact (the tool falls back to it)
 * and `promoteOne` hints that the pick is out of date.
 */
function promotePick(client: ReturnType<typeof createClient>, ref: EntityRef): void {
  if ((ref.kind !== "face" && ref.kind !== "edge") || !ref.bodyId || !ref.topoKey) return;
  const pick = { topoKey: ref.topoKey, anchor: ref.anchor ? { worldPoint: ref.anchor.worldPoint } : undefined };
  void promoteOne(client, ref.bodyId, pick).then((promoted) => {
    if (!promoted) return;
    const sel = selectionStore.getState();
    // Only if the ref is still selected (selection may have moved on).
    if (!sel.selected.some((r) => r.id === ref.id)) return;
    // Both ids, never `id`: that stays the pick-time `${bodyId}#${topoKey}`
    // identity `sameRef`/toggle compare on. The backend RESOLVED the pick, so its
    // topoKey is the authority on which element the mesh names right now.
    sel.set(
      sel.selected.map((r) =>
        r.id === ref.id
          ? { ...r, elementId: promoted.elementId, topoKey: promoted.topoKey || r.topoKey }
          : r,
      ),
    );
  });
}

// Faint 45° hatch behind the placeholder (prototype 1c) — fallback only.
// Tinted from a token so it inverts with the theme; a fixed black wash is
// invisible on a dark canvas.
const VIEWPORT_HATCH =
  "repeating-linear-gradient(45deg, var(--color-hatch) 0 12px, transparent 12px 24px)";

function hasFlag(name: string): boolean {
  return new URLSearchParams(window.location.search).has(name);
}

export function ViewportRoot({ className }: { className?: string }) {
  // Optional, not required: `ViewportRoot` is rendered bare by its own unit test
  // and by dev harnesses. No platform ⇒ no contributed layers, which is the
  // honest degradation — the built-in scene still renders.
  const platform = useOptionalPlatform();
  const containerRef = useRef<HTMLDivElement>(null);
  const overlayRef = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [sketching, setSketching] = useState(
    () => toolStore.getState().mode === "sketch",
  );
  const geometryPending = useViewportStore((s) => s.geometryPending);
  // Defer to the sticky failure hint (StatusBar) rather than showing both at
  // once — a body load failure is the more specific, more actionable signal.
  const errorHintActive =
    useViewportStore((s) => s.statusHint?.severity) === "error";
  const geometryCached = useDocumentStore(selectGeometryCached);
  const chip = viewportGeometryChip(ready, geometryPending, errorHintActive, geometryCached);

  // Mirror sketch mode for the placeholder styling (before the engine is ready).
  useEffect(() => {
    const unsub = toolStore.subscribe((s) =>
      setSketching(s.mode === "sketch"),
    );
    return unsub;
  }, []);

  useEffect(() => {
    const container = containerRef.current;
    const overlay = overlayRef.current;
    if (!container || !overlay) return;

    // The demo shows the grid so ?vpdemo proves grid + box + lighting together.
    if (hasFlag("vpdemo") && !viewportStore.getState().gridVisible) {
      viewportStore.setState({ gridVisible: true });
    }

    const engine = new ViewportEngine();
    let cancelled = false;
    const cleanups: Array<() => void> = [];
    // The theme the engine is about to build its materials against. `init()`
    // awaits renderer construction, and the theme subscription below can only
    // be installed after that resolves — so a flip landing inside that window
    // would be missed entirely and the scene would stay on the old theme until
    // the NEXT flip. Compared once, after subscribing.
    const themeAtInit = getResolvedTheme();

    engine
      .init(container, overlay, {
        experimentalWebGpu: settingsStore.getState().experimentalWebGpu,
        debug: hasFlag("vpdebug"),
        gridVisible: viewportStore.getState().gridVisible,
        // Read live so a preference change takes effect without a remount.
        getDevicePref: () => settingsStore.getState().navigation.inputDevice,
        // Model-tool drags (extrude depth, fillet radius, revolve angle) project
        // the pointer against the current camera, so a wheel-orbit mid-drag
        // would make the dragged value jump. Sketch drags re-raycast the plane
        // each move and are unaffected, so they are deliberately not gated.
        isDragActive: () => toolStore.getState().phase === "dragging",
        onDeviceChange: (device) =>
          viewportStore.getState().setDetectedInputDevice(device),
      })
      .then(() => {
        if (cancelled) {
          engine.dispose();
          return;
        }
        setViewportEngine(engine);
        setReady(true);

        // ── Viewport contributions (in-scene layers) ──
        // Handed the REGISTRY, not a snapshot: modeling's layers register when
        // the editor mounts while this init() is still awaiting the renderer, so
        // whichever side lost that race would be dropped by a one-shot attach.
        // The engine stays subscribed and disposes what it attached.
        engine.setContributionRegistry(platform?.viewport ?? null);
        cleanups.push(() => engine.setContributionRegistry(null));

        // ── Mesh ingestion (pull model) ──
        const client = createClient();
        const meshIngest = new MeshIngest();
        meshIngest.attach(engine, client);
        cleanups.push(() => meshIngest.detach());

        // ── Live re-theme ──
        // ONE orchestrator, because the ordering is not optional: the palette
        // cache must be dropped before ANY layer re-reads it, and the two
        // material-library owners (engine previews, MeshIngest commits) have no
        // common parent. Independent per-owner subscriptions would race.
        const applyTheme = () => {
          resetPaletteCache();
          engine.applyTheme();
          meshIngest.refreshColors();
        };
        cleanups.push(subscribeResolvedTheme(applyTheme));
        // Close the init-window race described above. Guarded, not
        // unconditional: applyTheme() rebuilds the PMREM environment, which is
        // real GPU work and must not run on every boot.
        if (getResolvedTheme() !== themeAtInit) applyTheme();

        // ── Always-visible document sketches in MODEL mode (Fusion-style) ──
        const sketchStaticSync = new SketchStaticSync();
        sketchStaticSync.attach(engine, client);
        cleanups.push(() => sketchStaticSync.detach());

        // ── Always-visible datum (reference) planes (DATUM W1) ──
        const datumSync = new DatumSync();
        datumSync.attach();
        cleanups.push(() => datumSync.detach());

        // Missed-event race: projection-updated may fire before these listeners
        // attach (open/new/recover populate the doc before the viewport wires up) —
        // one authoritative pull recovers it; applyProjectionToStore reconciles by
        // revision, so a stale response here can never clobber newer state.
        void client.getProjection().catch(() => {});

        // Auto-fit the camera once loading settles. Bodies stream in one at a
        // time, so fitting on the first only frames whichever body loads
        // first — wrong for multi-body assemblies. The engine owns the debounce
        // (`requestAutoFit`) so that a fit which is scheduled but not yet running
        // is visible as `engine.autoFitPending`: a bridge-local timer moved the
        // camera 250 ms after everything else already believed it had settled.
        cleanups.push(meshIngest.onBodyLoaded(() => engine.requestAutoFit()));
        cleanups.push(() => engine.cancelAutoFit());

        // Wake-up repaint: on-demand rendering means a tab/window that was hidden
        // or a compositor that dropped the last frame shows stale/blank content
        // until something invalidates — repaint on visibility/focus return.
        const onWake = () => {
          if (!document.hidden) engine.invalidate();
        };
        document.addEventListener("visibilitychange", onWake);
        window.addEventListener("focus", onWake);
        cleanups.push(() => {
          document.removeEventListener("visibilitychange", onWake);
          window.removeEventListener("focus", onWake);
        });

        // ── Sketch mode drawing tools + snapping (F-WP6) ──
        const sketchController = new SketchController({ engine, client, container });
        cleanups.push(() => sketchController.dispose());

        // ── Model tools + two-level preview (F-WP7) ──
        const modelToolController = new ModelToolController({
          engine,
          client,
          container,
          onBodyLoaded: (cb) => meshIngest.onBodyLoaded(cb),
          debug: hasFlag("vpdebug") || hasFlag("toolsdemo"),
        });
        setModelToolController(modelToolController);
        cleanups.push(() => {
          setModelToolController(null);
          modelToolController.dispose();
        });

        // ── Picking → selection store (engine stays store-agnostic) ──
        // Measure (W2-B) reuses the SAME face/edge raycast as `select` rather
        // than owning a second one: a duplicate hit-test would be a second
        // source of truth for "what is under the cursor", and the two would
        // drift. It differs only in what it does with the hit (below).
        const isPickingActive = () => {
          const s = toolStore.getState();
          return s.mode === "model" && (s.modelTool === "select" || s.modelTool === "measure");
        };
        const isMeasuring = () =>
          toolStore.getState().mode === "model" &&
          toolStore.getState().modelTool === "measure";
        // A datum plane is the LAST resort under the pointer (DATUM W1): bodies
        // and sketches are the things you are actually modelling, a datum is
        // reference scaffolding, so it only wins where nothing else is hit.
        const datumRefAt = (x: number, y: number): EntityRef | null => {
          const id = getDatumVisuals()?.hitTest(x, y) ?? null;
          return id ? { kind: "datum", id } : null;
        };
        engine.configurePicking({
          isActive: isPickingActive,
          onHover: (hit, x, y, alt) => {
            const sel = selectionStore.getState();
            const sketchHit = engine.sketchStaticHitTest(x, y);
            sel.setHover(refFromModelHits(hit, sketchHit, alt) ?? datumRefAt(x, y));
          },
          onPick: (hit, mods, x, y) => {
            const sel = selectionStore.getState();
            const sketchHit = engine.sketchStaticHitTest(x, y);
            const ref = refFromModelHits(hit, sketchHit, mods.alt) ?? datumRefAt(x, y);
            if (!ref) {
              sel.clear();
              return;
            }
            // Measure: the pick is a READ, so it always REPLACES the selection
            // (a modifier-extended multi-select would not mean anything to a
            // two-element measurement) and routes to the controller, which owns
            // the promote → elementInfo → overlay sequence. Everything else in
            // this handler is unchanged, including the promotion below — the
            // controller re-promotes only if this fire-and-forget one has not
            // landed by the time it needs an id.
            if (isMeasuring()) {
              sel.set([ref]);
              void modelToolController.measurePick(ref);
              return;
            }
            if (mods.shift || mods.meta) sel.toggle(ref);
            else sel.set([ref]);
            // Promote face/edge picks to a persistent ElementId (mock mints ids;
            // the real client routes to AcquireElementIds). Promoted id flows back
            // onto the selected ref, exactly as the mock does.
            promotePick(client, ref);
          },
        });
        cleanups.push(() => engine.configurePicking(null));

        // ── Double-click a static sketch → edit it (AUTO-MODE; mirrors the
        // tree row double-click). Model mode + select tool only, so it never
        // fights armed model tools or the region-pick double-click accelerator
        // (those run while a model tool is active, not "select").
        //
        // A miss falls through to a body FACE → select the whole connected body
        // (sketch-from-face lives on `S` with a face selected, not here). ──
        const onDblClick = (e: MouseEvent) => {
          const s = toolStore.getState();
          if (s.mode !== "model" || s.modelTool !== "select") return;
          const hit = engine.sketchStaticHitTest(e.clientX, e.clientY);
          if (hit) {
            s.setMode("sketch", hit.sketchId);
            return;
          }
          const face = engine.probePick(e.clientX, e.clientY);
          if (!face || face.kind !== "face") return;
          const ref: EntityRef = { kind: "body", id: face.bodyId };
          const sel = selectionStore.getState();
          if (e.shiftKey || e.metaKey) sel.toggle(ref);
          else sel.set([ref]);
        };
        container.addEventListener("dblclick", onDblClick);
        cleanups.push(() => container.removeEventListener("dblclick", onDblClick));

        // ── Selection store → highlight layer (tree ↔ viewport sync) ──
        const applyHighlight = () => {
          const s = selectionStore.getState();
          engine.setHighlightState(s.hover, s.selected);
        };
        applyHighlight();
        cleanups.push(selectionStore.subscribe(applyHighlight));

        // ── Sketch selection store → live SketchObject (edit-mode recolor) ──
        // No-op off sketch mode (setSketch* are guarded on the sketch presence).
        // Hover union rule: entity hover (`s.hover`, set by pointer-hover over
        // canvas geometry) and constraint-row hover (`s.constraintHover`, set by
        // ConstraintList mouseenter) can both be live at once — deterministically,
        // entity hover WINS when both are set (it is the more specific, pointer-
        // driven signal); constraint hover only drives the highlight when there is
        // no entity hover. The constraint's entity ids are resolved from the LIVE
        // sketchStore session (not a stale snapshot) each time either store fires.
        const applySketchSelection = () => {
          const s = sketchSelectionStore.getState();
          engine.setSketchSelection(s.selected.map((sel) => sel.entityId));
          if (s.hover) {
            engine.setSketchHover([s.hover.entityId]);
          } else if (s.constraintHover) {
            const session = sketchStore.getState().session;
            const constraint = session?.constraints.find((c) => c.id === s.constraintHover);
            engine.setSketchHover(constraint?.entities ?? []);
          } else {
            engine.setSketchHover([]);
          }
        };
        applySketchSelection();
        cleanups.push(sketchSelectionStore.subscribe(applySketchSelection));

        // ?vpdemo / ?sketchdemo / ?toolsdemo — demo-only flags, handled by a
        // dynamically-imported dev module (devDemos.ts) so the mock-kernel
        // import it needs never rides along into a production bundle:
        // `import.meta.env.DEV` is statically false in a prod build, so
        // Rollup dead-code-eliminates the whole `import("./devDemos")` call
        // — the demo flags are simply dead in production. In dev/e2e this
        // costs one microtask of delay versus the old synchronous block;
        // every e2e assertion that depends on these flags already polls
        // (`toPass`/`expect.poll`) rather than asserting immediately after
        // navigation, so that tick is invisible there.
        const vpdemo = hasFlag("vpdemo");
        const sketchdemo = hasFlag("sketchdemo");
        const toolsdemo = hasFlag("toolsdemo");
        if (import.meta.env.DEV && (vpdemo || sketchdemo || toolsdemo)) {
          void import("./devDemos").then((m) => {
            // Re-checked AFTER the await: StrictMode double-mount may have
            // torn this effect down while the dynamic import was in flight.
            if (cancelled) return;
            m.runDemoFlags({ engine, client, container, modelToolController, vpdemo, sketchdemo, toolsdemo });
          });
        }

        // store → engine (projection, grid)
        let lastProj = viewportStore.getState().projection;
        let lastGrid = viewportStore.getState().gridVisible;
        cleanups.push(
          viewportStore.subscribe((s) => {
            if (s.projection !== lastProj) {
              lastProj = s.projection;
              engine.setProjection(s.projection);
            }
            if (s.gridVisible !== lastGrid) {
              lastGrid = s.gridVisible;
              engine.setGridVisible(s.gridVisible);
            }
          }),
        );

        // engine → store (camera view label)
        const syncLabel = () => {
          const next = viewLabelForDirection(engine.getViewDirection());
          const st = viewportStore.getState();
          if (st.cameraViewLabel !== next) st.setCameraViewLabel(next);
        };
        syncLabel();
        cleanups.push(engine.onCameraChanged(syncLabel));

        // engine → store (cursor on Z=0), rAF-coalesced
        let pendingEvent: PointerEvent | null = null;
        let scheduled = false;
        const onMove = (e: PointerEvent) => {
          pendingEvent = e;
          if (scheduled) return;
          scheduled = true;
          requestAnimationFrame(() => {
            scheduled = false;
            if (!pendingEvent) return;
            const hit = engine.screenToWorldOnZ0(
              pendingEvent.clientX,
              pendingEvent.clientY,
            );
            if (hit) {
              viewportStore.getState().setCursor({ x: hit.x, y: hit.y, z: 0 });
            }
          });
        };
        container.addEventListener("pointermove", onMove);
        cleanups.push(() =>
          container.removeEventListener("pointermove", onMove),
        );
      })
      .catch((err: unknown) => {
        // jsdom / no-WebGL: stay in placeholder mode. Not fatal.
        logWarn("vp", "engine init failed; showing placeholder", { error: err });
      });

    return () => {
      cancelled = true;
      for (const c of cleanups) c();
      setViewportEngine(null);
      engine.dispose();
      setReady(false);
    };
  }, []);

  return (
    <div
      ref={containerRef}
      data-testid="viewport-canvas"
      // Selection (pick a face/edge on hit, deselect on an empty click) is owned
      // by the engine Picker on the canvas — no React onClick clear here, which
      // would otherwise wipe a fresh pick as the click bubbles up.
      // NOTE: `className` supplies the positioning (absolute inset…). It already
      // establishes a containing block for the engine's absolute canvas, so do
      // NOT add `relative` here — it would conflict and collapse the height.
      // No `overflow-hidden`: the engine's chip layer (`chipLayer()` in
      // ViewportEngine, `chipLayerEl`) is appended here as a sibling of the
      // overlay, and edge-anchored chips must be able to spill past this box's
      // edge rather than get clipped. Safe to leave unclipped because every
      // other child — canvas, overlay, placeholder — is `absolute inset-0`, so
      // nothing else can escape this container's bounds.
      className={cn("bg-canvas", className)}
    >
      {/* The engine appends its own <canvas> here (absolute, inset-0). */}
      <div
        ref={overlayRef}
        className="pointer-events-none absolute inset-0 z-[1]"
        aria-hidden="true"
      />
      {!ready && (
        <div
          className={cn(
            "absolute inset-0 z-[2] flex items-center justify-center",
            sketching ? "bg-canvas-sketch" : "bg-canvas-model",
          )}
          style={{ backgroundImage: VIEWPORT_HATCH }}
        >
          <span className="font-mono text-[12px] text-ink-7">
            {sketching
              ? "[ 2D sketch grid — canvas placeholder ]"
              : "[ 3D viewport — loading engine… ]"}
          </span>
        </div>
      )}
      {chip === "pending" && (
        <div
          data-testid="geometry-pending"
          // Non-interactive: e2e clicks the canvas constantly, and this chip
          // must never intercept a pick.
          className="pointer-events-none absolute inset-0 z-[2] flex items-center justify-center"
        >
          <span className="flex items-center gap-2 rounded-full border border-border bg-surface px-3 py-1.5 text-[12.5px] font-medium text-ink-5 shadow-panel">
            Rebuilding geometry…
          </span>
        </div>
      )}
      {chip === "cached" && (
        <div
          data-testid="geometry-cached"
          // Same non-interactive rule as the pending chip — never intercept a pick.
          // Anchored TOP-centre, not dead-centre like its sibling: "Rebuilding
          // geometry…" is transient, but this one persists for as long as nothing
          // has regenerated (potentially a whole read-only session), and a pill
          // parked over the middle of the model would obscure the very geometry it
          // is describing.
          className="pointer-events-none absolute inset-x-0 top-3 z-[2] flex justify-center"
        >
          <span className="flex items-center gap-2 rounded-full border border-border bg-surface px-3 py-1.5 text-[12.5px] font-medium text-ink-5 shadow-panel">
            Last saved geometry
          </span>
        </div>
      )}
    </div>
  );
}
