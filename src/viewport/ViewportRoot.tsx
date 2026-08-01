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
import { useEffect, useRef, useState } from "react";
import { cn } from "@/ui/cn";
import { ViewportEngine } from "./engine/ViewportEngine";
import { secondaryHitWins, type PickHit } from "./engine/Picker";
import { MeshIngest } from "./mesh/meshSync";
import { SketchStaticSync } from "./sketchStaticSync";
import { DatumSync } from "./datumSync";
import { setViewportEngine } from "./engineBridge";
import { viewLabelForDirection } from "@/features/viewcube/ViewCube";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import { toolStore } from "@/stores/toolStore";
import { installInputProbe } from "./debug/inputProbe";
import {
  selectionStore,
  sketchRegionRef,
  topoRefId,
  type EntityRef,
} from "@/stores/selectionStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { sketchStore } from "@/stores/sketchStore";
import { createClient } from "@/ipc/client";
import {
  emitMockDocumentChanged,
  mockMeshKey,
  resetMockSketches,
  resetMockDocument,
  setMockLatency,
} from "@/ipc/mockClient";
import type { SketchEntity } from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
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
 * Promote a face/edge pick to a persistent Rust-minted ElementId (SCHEMA §7.5)
 * and write it back onto the still-selected ref. Fire-and-forget; a failed / stale
 * promotion leaves the transient topoKey ref intact (the tool falls back to it).
 */
function promotePick(client: ReturnType<typeof createClient>, ref: EntityRef): void {
  if ((ref.kind !== "face" && ref.kind !== "edge") || !ref.bodyId || !ref.topoKey) return;
  const pick = { topoKey: ref.topoKey, anchor: ref.anchor ? { worldPoint: ref.anchor.worldPoint } : undefined };
  void client
    .promoteSelection(ref.bodyId, [pick])
    .then((promoted) => {
      const elementId = promoted.find((p) => p.topoKey === ref.topoKey)?.elementId;
      if (!elementId) return;
      const sel = selectionStore.getState();
      // Only if the ref is still selected (selection may have moved on).
      if (!sel.selected.some((r) => r.id === ref.id)) return;
      sel.set(sel.selected.map((r) => (r.id === ref.id ? { ...r, elementId } : r)));
    })
    .catch(() => {
      // Promotion failed (no snapshot / worker error) — keep the topoKey ref.
    });
}

// Faint 45° hatch behind the placeholder (prototype 1c) — fallback only.
const VIEWPORT_HATCH =
  "repeating-linear-gradient(45deg, rgba(0,0,0,0.02) 0 12px, transparent 12px 24px)";

function hasFlag(name: string): boolean {
  return new URLSearchParams(window.location.search).has(name);
}

export function ViewportRoot({ className }: { className?: string }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const overlayRef = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [sketching, setSketching] = useState(
    () => toolStore.getState().mode === "sketch",
  );

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

        // ── Mesh ingestion (pull model) ──
        const client = createClient();
        const meshIngest = new MeshIngest();
        meshIngest.attach(engine, client);
        // ?inputprobe — TEMPORARY wheel/gesture diagnostic. Remove with the probe.
        if (hasFlag("inputprobe")) {
          cleanups.push(installInputProbe(container));
        }

        cleanups.push(() => meshIngest.detach());

        // ── Always-visible document sketches in MODEL mode (Fusion-style) ──
        const sketchStaticSync = new SketchStaticSync();
        sketchStaticSync.attach(engine, client);
        cleanups.push(() => sketchStaticSync.detach());

        // ── Always-visible datum (reference) planes (DATUM W1) ──
        const datumSync = new DatumSync();
        datumSync.attach(engine);
        cleanups.push(() => datumSync.detach());

        // Missed-event race: projection-updated may fire before these listeners
        // attach (open/new/recover populate the doc before the viewport wires up) —
        // one authoritative pull recovers it; applyProjectionToStore reconciles by
        // revision, so a stale response here can never clobber newer state.
        void client.getProjection().catch(() => {});

        // Auto-fit the camera once, the first time a body finishes loading.
        let fitted = false;
        cleanups.push(
          meshIngest.onBodyLoaded(() => {
            if (!fitted) {
              fitted = true;
              engine.fitView();
            }
          }),
        );

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
          const id = engine.datumHitTest(x, y);
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
        // (those run while a model tool is active, not "select"). ──
        const onDblClick = (e: MouseEvent) => {
          const s = toolStore.getState();
          if (s.mode !== "model" || s.modelTool !== "select") return;
          const hit = engine.sketchStaticHitTest(e.clientX, e.clientY);
          if (hit) s.setMode("sketch", hit.sketchId);
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

        // ?vpdemo — drive the mock box through the FULL onDocumentChanged path.
        if (hasFlag("vpdemo")) {
          emitMockDocumentChanged({
            revision: 1,
            changedBodies: [{ bodyId: "body1", meshKey: mockMeshKey("body1", "coarse") }],
            removedBodies: [],
          });
        }

        // ?sketchdemo — enter sketch mode on the seeded XY sketch (no backend;
        // the mock enterSketch does it). Pass an explicit id (skip the plane
        // picker) so the demo is deterministic. Proves the sketch UX end to end.
        if (hasFlag("sketchdemo")) {
          resetMockSketches();
          toolStore.getState().setMode("sketch", "sketch2");
        }

        // ?toolsdemo — seed a finished rectangle sketch + a window harness so the
        // 60fps gate can arm/drag/commit extrude and read frame timing.
        if (hasFlag("toolsdemo")) {
          resetMockSketches();
          resetMockDocument();
          const sid = "toolsketch";
          let regionRef: EntityRef | null = null;
          const rect: SketchEntity[] = [
            { id: "e1", type: "Line", p0: [-30, -20], p1: [30, -20] },
            { id: "e2", type: "Line", p0: [30, -20], p1: [30, 20] },
            { id: "e3", type: "Line", p0: [30, 20], p1: [-30, 20] },
            { id: "e4", type: "Line", p0: [-30, 20], p1: [-30, -20] },
          ];
          void (async () => {
            await client.enterSketch({ newOnPlane: "XY", sketchId: sid });
            await client.sketchUpsert(sid, rect, []);
            const finish = await client.finishSketch(sid);
            if (finish.regions[0]) {
              regionRef = sketchRegionRef(sid, finish.regions[0].regionId);
            }
            documentStore.getState().addSketch({
              id: sid,
              name: "Sketch T",
              visible: true,
              dof: 0,
              status: "ok",
              geometryToken: `mock:${sid}:v1`,
            });
          })();
          (window as unknown as { __toolsGate?: unknown }).__toolsGate = {
            setLatency: (ms: number) => setMockLatency(ms),
            engine,
            controller: modelToolController,
            container,
            client,
            stores: { selectionStore, toolStore, documentStore },
            arm: () => {
              if (!regionRef) return;
              selectionStore.getState().set([regionRef]);
              toolStore.getState().setTool("extrude");
            },
          };
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
      .catch((err) => {
        // jsdom / no-WebGL: stay in placeholder mode. Not fatal.
        console.warn("[viewport] engine init failed; showing placeholder:", err);
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
      className={cn("overflow-hidden bg-canvas", className)}
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
    </div>
  );
}
