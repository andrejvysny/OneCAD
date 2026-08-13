/*
 * placementController — the interactive placement gesture (spec §5.1-§5.4;
 * Component Library WP-1.5). A module-level singleton, like
 * `ModelToolController` but library-owned and independent of it: modeling's
 * tool FSM/pick-handler seat stays untouched, so this gesture cannot fight
 * orbit or selection for the canvas.
 *
 * WHY NOT A `ViewportContribution`: that contract (`ctx.onFrame`/
 * `raycastFromClient`/`registerSecondaryHover`) has no raw pointer-event or
 * arbitrary-engine-method hook — by design, contributions only draw into
 * their own scene-graph slot. This gesture instead follows the SAME pattern
 * `ModelToolController` already uses: a direct `ViewportEngine` reference
 * (via `engineBridge`, the existing seam for code outside `<ViewportRoot>`)
 * and the SAME `setPreviewBody`/`clearPreviewBody` methods that engine
 * already exposes generically for any L2 preview body — no platform surface
 * had to change (see HANDOFF.md's resolved "gesture routing" question).
 *
 * PLAIN `pointerdown`/`pointermove`/`keydown` LISTENERS on `window`, CAPTURE
 * PHASE, only while armed. Capture-phase + `stopPropagation` shuts out the
 * canvas's own bubble-phase orbit/pick listeners without touching them or
 * `ViewportRoot.tsx` — the shell still does not know this module exists.
 * `engine.setOrbitSuppressed(true)` is the belt to that suspenders.
 *
 * AUTO-SIZE (spec §5.3's hole row, §5.4 step 3, WP-A3): hovering a cylinder or
 * a circular edge measures the hole and picks the largest declared size that
 * still fits, feeding it to the ghost AND the commit through the same
 * `source.params`. Nothing is substituted when nothing fits — the armed size
 * simply stands, the same refusal the generator applies to an unknown size.
 *
 * SCOPE CUT vs the full spec, recorded here (also in TODO.md's gate entry):
 *   - No free-space follow. The ghost appears only once hovering a target
 *     whose classification matches one of the component's attachments — spec
 *     step 1's "ghost follows the cursor" is honored from the FIRST valid
 *     hover, not from arm-time. Free-space placement (step 6, the Move-tool
 *     fallback) is not wired; there is no Move-tool integration point yet.
 *
 * MATE AUTHORING (WP-H2, spec §5.4 step 5): a snapped commit records the
 * attachment pair — the matched attachment key, the target pick's identity
 * evidence (bodyId + topoKey + elementId when the pick carried one), the snap
 * kind, and the flip state. The backend promotes the topoKey to a Rust-minted
 * ElementId at the head and authors `PlaceComponentParams.mate`, which is what
 * regen's re-seat lane (WP-3.1) resolves. Fails closed: an unpromotable target
 * refuses the placement rather than silently degrading to free-space.
 */
import { useSyncExternalStore } from "react";
import { getViewportEngine } from "@/viewport/engineBridge";
import type { PickHit } from "@/viewport/engine/Picker";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { buildBodyObjects, remove as removeMesh, swap as swapMesh } from "@/viewport/mesh/meshRegistry";
import { viewportStore } from "@/stores/viewportStore";
import { createClient } from "@/ipc/client";
import type {
  ComponentParamValue,
  LibraryComponent,
  PlaceComponentSource,
  PreviewResult,
  PreviewSession,
} from "@/ipc/types";
import type { CommandApiService, GeometryQueryService } from "@/modules/modeling/manifest";
import {
  attachmentAccepts,
  classifySnapKind,
  nearestSmallerThread,
  solveCandidatePlacement,
  type CandidatePlacement,
  type MateSnapKind,
} from "./placementSolver";

interface Services {
  geometryQuery: GeometryQueryService;
  commandApi: CommandApiService;
}

let services: Services | null = null;

/**
 * Injects the modeling services this gesture needs (ADR-0002: the kernel
 * touch routes through modeling's published services, never a direct
 * `CadClient` call from library code). Called once from
 * `contributeLibraryUi` at editor mount; `null` on unmount so a stale
 * reference cannot outlive the scope that resolved it.
 */
export function configurePlacementController(next: Services | null): void {
  if (!next) cancelPlacement();
  services = next;
}

interface LastMatch {
  snapKind: MateSnapKind;
  attachmentKey: string;
  frame: NonNullable<import("@/ipc/types").ClassifyResult["frame"]>;
  pickWorldPos: [number, number, number];
  /** The hovered pick's identity evidence — becomes the committed mate's target. */
  target: { bodyId: string; topoKey: string; elementId?: string; kind: "face" | "edge" };
}

let armedComponent: LibraryComponent | null = null;
/** The armed component's resolved geometry source (WP-3.2) — see `armPlacement`. */
let armedSource: PlaceComponentSource | null = null;
let flipped = false;
let previewSession: PreviewSession | null = null;
let previewBodyIds: string[] = [];
let unsubscribePreviewResult: (() => void) | null = null;
let epoch = 0;
let hoverSeq = 0;
let lastMatch: LastMatch | null = null;
let lastCandidate: CandidatePlacement | null = null;
/**
 * Free-param overrides the gesture itself chose — auto-size only, today. Sent
 * with BOTH the ghost (`source.params`) and the commit, so the two can never
 * describe different hardware.
 */
let gestureParams: Record<string, ComponentParamValue> = {};

export function isPlacementArmed(): boolean {
  return armedComponent !== null;
}

// Armed-state pub/sub, for `LibraryPanel`'s card highlight — mirrors
// `engineBridge.ts`'s module-singleton-+-`useSyncExternalStore` shape (a
// React context would not reach this file's non-React callers).
const armedListeners = new Set<() => void>();
function publishArmed(): void {
  for (const l of [...armedListeners]) l();
}
/** `"<id>@<version>"` of the armed component, or `null`. */
export function armedComponentKey(): string | null {
  return armedComponent ? `${armedComponent.id}@${armedComponent.version}` : null;
}
export function useArmedComponentKey(): string | null {
  return useSyncExternalStore(
    (cb) => {
      armedListeners.add(cb);
      return () => armedListeners.delete(cb);
    },
    armedComponentKey,
    () => null,
  );
}

/** Arms the gesture for `component`. Cancels any prior armed placement first. */
export function armPlacement(component: LibraryComponent): void {
  cancelPlacement();
  if (!services) return; // no editor mounted yet — nothing to arm against
  const engine = getViewportEngine();
  if (!engine) return;

  armedComponent = component;
  flipped = false;
  lastMatch = null;
  lastCandidate = null;
  gestureParams = {};
  epoch = 0;
  publishArmed();

  engine.setOrbitSuppressed(true);
  window.addEventListener("pointermove", onPointerMove, true);
  window.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("keydown", onKeyDown, true);
  unsubscribePreviewResult = services.commandApi.onPreviewResult(onPreviewResult);

  // Resolve the component's geometry source BEFORE the first preview (WP-3.2).
  // For a blob-backed component this is also what materializes its baked bytes
  // for the worker, so the ghost cannot lower an empty path; for a generator it
  // is a cheap lookup. Called on `createClient()` rather than through
  // `services` on purpose: resolving a LIBRARY identity is the library's own
  // surface (the same call shape `LibraryPanel` uses for list/reindex), while
  // everything that touches the kernel below still routes through modeling's
  // published services per ADR-0002.
  void createClient()
    .resolveComponentSource(component.id, component.version)
    .then((source) => {
      if (armedComponent !== component) return; // cancelled mid-flight
      armedSource = source;
      return services?.commandApi
        .beginPreview({
          opType: "PlaceComponent",
          params: placementDraftParams(component, source, [0, 0, 0], identityRotate()),
        })
        .then((session) => {
          // Armed state may have been cancelled while the round trip was in flight.
          if (armedComponent !== component) {
            void services?.commandApi.endPreview(session.sessionId, false);
            return;
          }
          previewSession = session;
        });
    })
    .catch((e: unknown) => {
      if (armedComponent !== component) return;
      // No usable geometry ⇒ no ghost, and no silent half-armed state that
      // would commit a placement the preview never showed.
      cancelPlacement();
      viewportStore
        .getState()
        .setStatusHint(
          `Cannot place ${component.name}: ${e instanceof Error ? e.message : String(e)}`,
        );
    });

  viewportStore
    .getState()
    .setStatusHint(`Place ${component.name} — hover a target, A flips, Esc cancels`, {
      sticky: true,
    });
}

/** Tears the gesture down: hides the ghost, cancels the preview session, restores input. */
export function cancelPlacement(): void {
  window.removeEventListener("pointermove", onPointerMove, true);
  window.removeEventListener("pointerdown", onPointerDown, true);
  window.removeEventListener("keydown", onKeyDown, true);
  unsubscribePreviewResult?.();
  unsubscribePreviewResult = null;

  const engine = getViewportEngine();
  engine?.setOrbitSuppressed(false);
  if (previewBodyIds.length > 0) {
    engine?.clearPreviewBody(previewBodyIds);
    for (const id of previewBodyIds) removeMesh(id);
    previewBodyIds = [];
  }
  if (previewSession) {
    void services?.commandApi.endPreview(previewSession.sessionId, false);
    previewSession = null;
  }
  if (armedComponent) viewportStore.getState().setStatusHint(null);

  armedComponent = null;
  armedSource = null;
  lastMatch = null;
  lastCandidate = null;
  gestureParams = {};
  publishArmed();
}

function identityRotate(): { center: [number, number, number]; axis: [number, number, number]; angleDeg: number } {
  return { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: 0 };
}

function placementDraftParams(
  component: LibraryComponent,
  source: PlaceComponentSource,
  translate: [number, number, number],
  rotate: { center: [number, number, number]; axis: [number, number, number]; angleDeg: number },
): Record<string, unknown> {
  return {
    componentId: component.id,
    componentVersion: component.version,
    componentRevision: component.revision,
    // The backend's own resolution, verbatim — see `armPlacement`. This used to
    // be a locally-assembled generator source, which previewed the generator
    // stub's screw for ANY component whose real source was a blob. The
    // gesture's own free params ride INSIDE it, because `source.params` is
    // what the worker's table lookup reads.
    source: withGestureParams(source),
    translate,
    rotate,
  };
}

/**
 * `source` with the gesture's chosen free params folded in. Only a generator
 * source takes them: a blob-backed component's geometry was baked at authoring
 * time and has no parameters to re-derive from (the backend refuses the same
 * combination, loudly, rather than recording an inert override).
 */
function withGestureParams(source: PlaceComponentSource): PlaceComponentSource {
  if (source.kind !== "generator" || Object.keys(gestureParams).length === 0) return source;
  return { ...source, params: { ...gestureParams } };
}

/**
 * The free-param key this component sizes by, or `null` when it declares no
 * `thread` domain to choose from — a component with a free-text size, or none
 * at all, simply never auto-sizes.
 */
function sizeDomain(component: LibraryComponent): readonly string[] | null {
  const spec = component.parameters?.thread;
  if (!spec || spec.role !== "free") return null;
  const domain = spec.domain?.filter((v): v is string => typeof v === "string") ?? [];
  return domain.length > 0 ? domain : null;
}

/**
 * Auto-size (spec §5.3's hole row, §5.4 step 3): a cylinder or circular-edge
 * frame carries the hole's radius, so the largest size that still fits is
 * decidable right here. Returns whether the chosen size CHANGED, so the caller
 * only re-hints when there is something new to say.
 *
 * Nothing fitting is not an error and not a substitution: the armed size
 * stands and the user still places a screw, it just is not the one the hole
 * wanted. Deciding otherwise would be the Toolbox failure mode in miniature.
 */
function applyAutoSize(component: LibraryComponent, frame: LastMatch["frame"]): boolean {
  if (frame.radius === null || frame.radius === undefined) return false;
  const domain = sizeDomain(component);
  if (!domain) return false;
  const picked = nearestSmallerThread(frame.radius * 2, domain);
  if (!picked || gestureParams.thread === picked) return false;
  gestureParams = { ...gestureParams, thread: picked };
  return true;
}

/** The component's attachments whose `accepts` admits `snapKind`, in table order. */
function matchingAttachments(component: LibraryComponent, snapKind: MateSnapKind): string[] {
  return Object.keys(component.attachments).filter((key) =>
    attachmentAccepts(component.attachments[key].accepts, snapKind),
  );
}

function pushCandidate(translate: [number, number, number], rotate: CandidatePlacement["rotate"]): void {
  if (!armedComponent || !previewSession || !services || !armedSource) return;
  epoch += 1;
  services.commandApi.updatePreview(
    previewSession.sessionId,
    placementDraftParams(armedComponent, armedSource, translate, rotate),
    epoch,
  );
}

function clearGhostOnly(): void {
  const engine = getViewportEngine();
  if (previewBodyIds.length === 0) return;
  engine?.clearPreviewBody(previewBodyIds);
  for (const id of previewBodyIds) removeMesh(id);
  previewBodyIds = [];
}

function recomputeFromLastMatch(): void {
  if (!lastMatch) return;
  const candidate = solveCandidatePlacement(
    lastMatch.snapKind,
    lastMatch.frame,
    lastMatch.pickWorldPos,
    flipped,
  );
  lastCandidate = candidate;
  pushCandidate(candidate.translate, candidate.rotate);
}

function onPointerMove(e: PointerEvent): void {
  if (!armedComponent || !services) return;
  const engine = getViewportEngine();
  const hit: PickHit | null = engine?.probePick(e.clientX, e.clientY) ?? null;
  if (!hit) {
    lastMatch = null;
    lastCandidate = null;
    clearGhostOnly();
    return;
  }
  const seq = ++hoverSeq;
  const component = armedComponent;
  void services.geometryQuery
    .classifyElement(hit.bodyId, hit.elementId ?? "", hit.topoKey)
    .then((classify) => {
      if (seq !== hoverSeq || armedComponent !== component) return; // stale or cancelled
      const snapKind = classify ? classifySnapKind(classify) : null;
      if (!snapKind || !classify?.frame) {
        lastMatch = null;
        lastCandidate = null;
        clearGhostOnly();
        return;
      }
      const candidates = matchingAttachments(component, snapKind);
      if (candidates.length === 0) {
        lastMatch = null;
        lastCandidate = null;
        clearGhostOnly();
        return;
      }
      // Preserve the user's Tab-chosen attachment across hovers when it is
      // still a valid match for the newly hovered target; otherwise fall
      // back to the first match.
      const attachmentKey =
        lastMatch && candidates.includes(lastMatch.attachmentKey)
          ? lastMatch.attachmentKey
          : candidates[0];
      const pickWorldPos: [number, number, number] = [hit.worldPos.x, hit.worldPos.y, hit.worldPos.z];
      lastMatch = {
        snapKind,
        attachmentKey,
        frame: classify.frame,
        pickWorldPos,
        target: { bodyId: hit.bodyId, topoKey: hit.topoKey, elementId: hit.elementId, kind: hit.kind },
      };
      // Auto-size BEFORE the candidate push, so the very first ghost frame for
      // this hover already shows the size the commit will place.
      if (applyAutoSize(component, classify.frame)) {
        viewportStore
          .getState()
          .setStatusHint(
            `Place ${component.name} ${String(gestureParams.thread)} — A flips, Esc cancels`,
            { sticky: true },
          );
      }
      recomputeFromLastMatch();
    });
}

function onPointerDown(e: PointerEvent): void {
  if (!armedComponent) return;
  e.preventDefault();
  e.stopPropagation();
  if (!lastCandidate || !armedComponent || !lastMatch) return;
  const component = armedComponent;
  const candidate = lastCandidate;
  // The recorded snap (WP-H2): what makes the placement re-seat on regen when
  // the target moves, and surface NeedsRepair when it vanishes (spec §5.5).
  const mate: import("@/ipc/types").PlaceComponentMate = {
    selfAttachment: lastMatch.attachmentKey,
    targetBodyId: lastMatch.target.bodyId,
    targetTopoKey: lastMatch.target.topoKey,
    targetElementId: lastMatch.target.elementId,
    targetKind: lastMatch.target.kind,
    kind: lastMatch.snapKind,
    flipped,
    anchorWorldPoint: lastMatch.pickWorldPos,
  };
  void services?.commandApi
    .placeComponent(
      component.id,
      component.version,
      candidate.translate,
      candidate.rotate,
      Object.keys(gestureParams).length > 0 ? { ...gestureParams } : undefined,
      mate,
    )
    .catch((e: unknown) => {
      // Fail closed, visibly: the backend refuses a mate it cannot promote
      // (stale pick) instead of authoring a silently-unmated record.
      viewportStore
        .getState()
        .setStatusHint(
          `Place failed: ${e instanceof Error ? e.message : String(e)}`,
          { severity: "error", sticky: true },
        );
    });
  cancelPlacement();
}

function onKeyDown(e: KeyboardEvent): void {
  if (!armedComponent) return;
  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    cancelPlacement();
    return;
  }
  if (e.key === "a" || e.key === "A") {
    e.preventDefault();
    e.stopPropagation();
    flipped = !flipped;
    recomputeFromLastMatch();
    return;
  }
  if (e.key === "Tab" && lastMatch && armedComponent) {
    const candidates = matchingAttachments(armedComponent, lastMatch.snapKind);
    if (candidates.length > 1) {
      e.preventDefault();
      e.stopPropagation();
      const idx = candidates.indexOf(lastMatch.attachmentKey);
      const next = candidates[(idx + 1) % candidates.length];
      lastMatch = { ...lastMatch, attachmentKey: next };
      recomputeFromLastMatch();
    }
  }
}

function onPreviewResult(r: PreviewResult): void {
  if (!previewSession || r.sessionId !== previewSession.sessionId) return;
  if (r.committed || r.error) return;
  const bodies = r.bodies ?? (r.mesh ? [{ bodyId: r.bodyId, mesh: r.mesh }] : []);
  const engine = getViewportEngine();
  clearGhostOnly();
  if (!engine || bodies.length === 0) return;
  let rev = 0;
  for (const b of bodies) {
    const ghostId = `${previewSession.previewBodyId}:${rev}`;
    const view = parseMeshPayload(b.mesh);
    const entry = buildBodyObjects(view, ghostId, ++rev);
    swapMesh(ghostId, entry);
    engine.setPreviewBody(entry);
    previewBodyIds.push(ghostId);
  }
}
