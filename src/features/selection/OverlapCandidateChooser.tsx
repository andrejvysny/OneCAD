import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type RefObject,
} from "react";
import { createClient } from "@/ipc/client";
import {
  installedEntryIsCurrent,
  installedProofIsCurrent,
  promoteViewportPick,
  stalePickHint,
  type InstalledPickProof,
} from "@/ipc/promote";
import { Popover } from "@/ui/Popover";
import { isInteractiveBoundary } from "@/ui/interactiveBoundary";
import { topoRefId, selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { viewportStore } from "@/stores/viewportStore";
import type { ProbeCandidate, ProbeCandidateKind } from "@/viewport/engine/Picker";
import { attachPickProof } from "@/viewport/mesh/pickProof";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";

const DRAG_PX = 4;
const CANDIDATE_KINDS: readonly ProbeCandidateKind[] = ["body", "face", "edge"];

type Filters = Record<ProbeCandidateKind, boolean>;
type PointerStart = { pointerId: number; x: number; y: number; moved: boolean };
type Session = {
  id: number;
  candidates: readonly ProbeCandidate[];
  filters: Filters;
  active: number;
  x: number;
  y: number;
  committing: boolean;
};

type Props = {
  containerRef: RefObject<HTMLDivElement | null>;
  engine: ViewportEngine | null;
  meshEpoch: number;
};

function unmodified(event: PointerEvent): boolean {
  return !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
}

function canOpen(): boolean {
  const tool = toolStore.getState();
  return tool.mode === "model" && tool.modelTool === "select";
}

/**
 * The candidate's installed-entry proof — face/edge only.
 *
 * A BODY candidate names a body id, not a topological element: it is selected
 * without ever reaching `AcquireElementIds`, so it has nothing to promote and no
 * element-level proof to carry. Its currentness is the ENTRY's alone
 * ({@link candidateIsCurrent}).
 */
function proofFor(candidate: ProbeCandidate): InstalledPickProof | null {
  if (!candidate.entry || candidate.kind === "body") return null;
  return { entry: candidate.entry, kind: candidate.kind, topoKey: candidate.topoKey };
}

function candidateIsCurrent(candidate: ProbeCandidate): boolean {
  if (candidate.kind === "body") {
    return candidate.entry !== undefined
      && candidate.topoKey === candidate.bodyId
      && installedEntryIsCurrent(candidate.bodyId, candidate.entry);
  }
  const proof = proofFor(candidate);
  return proof !== null && installedProofIsCurrent(candidate.bodyId, proof);
}

function candidateRef(candidate: ProbeCandidate, elementId?: string): EntityRef {
  if (candidate.kind === "body") return { kind: "body", id: candidate.bodyId };
  const ref: EntityRef = {
    kind: candidate.kind,
    id: topoRefId(candidate.bodyId, candidate.topoKey),
    bodyId: candidate.bodyId,
    topoKey: candidate.topoKey,
    elementId,
    anchor: { worldPoint: [candidate.worldPos.x, candidate.worldPos.y, candidate.worldPos.z] },
  };
  // This ref IS a viewport pick — probed, not typed — so it carries the same
  // pick-time proof `ViewportRoot.refFromHit` attaches. Without it a chooser
  // selection would be a second-class one that no tool could ever promote.
  const proof = proofFor(candidate);
  if (proof) attachPickProof(ref, proof);
  return ref;
}

function shownCandidates(session: Session): readonly ProbeCandidate[] {
  return session.candidates.filter((candidate) => session.filters[candidate.kind]);
}

function ordinalFor(candidate: ProbeCandidate): string | null {
  if (candidate.kind === "body" || !candidate.entry) return null;
  const index = candidate.kind === "face" ? candidate.entry.faceIndex : candidate.entry.edgeIndex;
  const ordinal = index?.ordinalForId(candidate.topoKey) ?? -1;
  return ordinal >= 0 ? String(ordinal + 1) : null;
}

function candidateLabel(candidate: ProbeCandidate): string {
  const bodyName = documentStore.getState().bodies[candidate.bodyId]?.name ?? candidate.bodyId;
  const type = candidate.kind.charAt(0).toUpperCase() + candidate.kind.slice(1);
  const ordinal = ordinalFor(candidate);
  return ordinal ? `${type} · ${bodyName} · ${ordinal}` : `${type} · ${bodyName}`;
}

function firstActive(candidates: readonly ProbeCandidate[], filters: Filters): number {
  return candidates.findIndex((candidate) => filters[candidate.kind]);
}

export function OverlapCandidateChooser({ containerRef, engine, meshEpoch }: Props) {
  const [session, setSession] = useState<Session | null>(null);
  const sessionRef = useRef<Session | null>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);
  const pointerRef = useRef<PointerStart | null>(null);
  const hoverRef = useRef<EntityRef | null>(null);
  const candidateButtons = useRef(new Map<number, HTMLButtonElement>());
  const nextId = useRef(0);

  const setCurrentSession = useCallback((next: Session | null) => {
    sessionRef.current = next;
    setSession(next);
  }, []);

  const clearChooserHover = useCallback(() => {
    const hover = hoverRef.current;
    if (hover && selectionStore.getState().hover === hover) selectionStore.getState().setHover(null);
    hoverRef.current = null;
  }, []);

  const dismiss = useCallback((restoreViewportFocus = false) => {
    setCurrentSession(null);
    clearChooserHover();
    if (restoreViewportFocus) queueMicrotask(() => containerRef.current?.focus());
  }, [clearChooserHover, containerRef, setCurrentSession]);

  const dismissSession = useCallback((id: number, restoreViewportFocus = false) => {
    if (sessionRef.current?.id === id) dismiss(restoreViewportFocus);
  }, [dismiss]);

  const preview = useCallback((id: number, candidate: ProbeCandidate) => {
    if (!candidateIsCurrent(candidate)) {
      stalePickHint();
      dismissSession(id);
      return;
    }
    const ref = candidateRef(candidate);
    hoverRef.current = ref;
    selectionStore.getState().setHover(ref);
  }, [dismissSession]);

  const openAt = useCallback((x: number, y: number) => {
    if (!engine || !canOpen()) return;
    const candidates = engine.probeCandidates(x, y, { kinds: CANDIDATE_KINDS }).filter(candidateIsCurrent);
    if (candidates.length === 0) return;
    const filters: Filters = { body: true, face: true, edge: true };
    const next: Session = {
      id: ++nextId.current,
      candidates,
      filters,
      active: firstActive(candidates, filters),
      x,
      y,
      committing: false,
    };
    setCurrentSession(next);
  }, [engine, setCurrentSession]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !engine) return;
    const onDown = (event: PointerEvent) => {
      if (isInteractiveBoundary(event)) {
        pointerRef.current = null;
        return;
      }
      pointerRef.current = event.button === 2 && unmodified(event) && canOpen()
        ? { pointerId: event.pointerId, x: event.clientX, y: event.clientY, moved: false }
        : null;
    };
    const onMove = (event: PointerEvent) => {
      if (isInteractiveBoundary(event)) return;
      const start = pointerRef.current;
      if (!start || start.pointerId !== event.pointerId) return;
      if (Math.max(Math.abs(event.clientX - start.x), Math.abs(event.clientY - start.y)) > DRAG_PX) start.moved = true;
    };
    const onUp = (event: PointerEvent) => {
      if (isInteractiveBoundary(event)) {
        pointerRef.current = null;
        return;
      }
      const start = pointerRef.current;
      pointerRef.current = null;
      if (!start || start.pointerId !== event.pointerId || event.button !== 2 || start.moved || !unmodified(event)) return;
      openAt(event.clientX, event.clientY);
    };
    const clearPointer = () => { pointerRef.current = null; };
    container.addEventListener("pointerdown", onDown);
    container.addEventListener("pointermove", onMove);
    container.addEventListener("pointerup", onUp);
    container.addEventListener("pointercancel", clearPointer);
    return () => {
      container.removeEventListener("pointerdown", onDown);
      container.removeEventListener("pointermove", onMove);
      container.removeEventListener("pointerup", onUp);
      container.removeEventListener("pointercancel", clearPointer);
    };
  }, [containerRef, engine, openAt]);

  useEffect(() => clearChooserHover, [clearChooserHover]);
  useEffect(() => {
    const validate = () => {
      const current = sessionRef.current;
      if (!current) return;
      if (!canOpen() || current.candidates.some((candidate) => !candidateIsCurrent(candidate))) {
        dismissSession(current.id);
      }
    };
    validate();
    return documentStore.subscribe(validate);
  }, [dismissSession]);
  useEffect(() => {
    const current = sessionRef.current;
    if (current && current.candidates.some((candidate) => !candidateIsCurrent(candidate))) {
      dismissSession(current.id);
    }
  }, [dismissSession, meshEpoch]);
  useEffect(() => toolStore.subscribe(() => {
    const current = sessionRef.current;
    if (current && !canOpen()) dismissSession(current.id);
  }), [dismissSession]);
  useEffect(() => viewportStore.subscribe(() => {
    const current = sessionRef.current;
    if (current && current.candidates.some((candidate) => !candidateIsCurrent(candidate))) {
      dismissSession(current.id);
    }
  }), [dismissSession]);

  const candidates = useMemo(() => session ? shownCandidates(session) : [], [session]);
  const active = session && session.active >= 0 ? session.candidates[session.active] : null;

  useEffect(() => {
    if (session && !session.committing && active) preview(session.id, active);
  }, [active, preview, session]);

  const updateSession = (update: (current: Session) => Session) => {
    const current = sessionRef.current;
    if (!current) return;
    setCurrentSession(update(current));
  };

  const setActive = (candidate: ProbeCandidate) => {
    updateSession((current) => ({ ...current, active: current.candidates.indexOf(candidate) }));
  };

  const toggleFilter = (kind: ProbeCandidateKind) => {
    updateSession((current) => {
      const filters = { ...current.filters, [kind]: !current.filters[kind] };
      const activeCandidate = current.candidates[current.active];
      const active = activeCandidate && filters[activeCandidate.kind]
        ? current.active
        : firstActive(current.candidates, filters);
      return { ...current, filters, active };
    });
  };

  const cycle = (direction: 1 | -1) => {
    const current = sessionRef.current;
    const visible = current ? shownCandidates(current) : [];
    if (!current || visible.length === 0) return;
    const index = Math.max(0, visible.indexOf(current.candidates[current.active]));
    const next = visible[(index + direction + visible.length) % visible.length];
    const nextIndex = current.candidates.indexOf(next);
    setActive(next);
    queueMicrotask(() => candidateButtons.current.get(nextIndex)?.focus());
  };

  const commit = async (requested?: ProbeCandidate) => {
    const current = sessionRef.current;
    const candidate = requested ?? (current ? current.candidates[current.active] ?? null : null);
    if (!current || !candidate || current.committing) return;
    if (!candidateIsCurrent(candidate) || !canOpen()) {
      stalePickHint();
      dismissSession(current.id);
      return;
    }
    if (candidate.kind === "body") {
      updateSession((value) => ({ ...value, committing: true }));
      if (sessionRef.current?.id === current.id && candidateIsCurrent(candidate) && canOpen()) {
        selectionStore.getState().set([candidateRef(candidate)]);
      } else {
        stalePickHint();
      }
      dismissSession(current.id);
      return;
    }
    // `candidateIsCurrent` already proved this above; the read is what narrows
    // it for the promotion, which owns the snapshot fence from here (the proof
    // carries the publication the candidate was probed against).
    const proof = proofFor(candidate);
    if (!proof) {
      stalePickHint();
      dismissSession(current.id);
      return;
    }
    updateSession((value) => ({ ...value, committing: true }));
    const promoted = await promoteViewportPick(
      createClient(),
      proof,
      { topoKey: candidate.topoKey, kind: candidate.kind, anchor: { worldPoint: [candidate.worldPos.x, candidate.worldPos.y, candidate.worldPos.z] } },
    );
    if (
      promoted &&
      sessionRef.current?.id === current.id &&
      candidateIsCurrent(candidate) &&
      canOpen()
    ) {
      selectionStore.getState().set([candidateRef(candidate, promoted.elementId)]);
    }
    dismissSession(current.id);
  };

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      dismiss(true);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      cycle(event.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      void commit();
    }
  };

  if (!session) return null;
  return (
    <>
      <span
        ref={anchorRef}
        aria-hidden="true"
        className="pointer-events-none fixed h-px w-px"
        style={{ left: session.x, top: session.y }}
      />
      <Popover
        open
        onClose={() => dismiss()}
        anchorRef={anchorRef}
        ariaLabel="Select overlapping geometry"
        autoFocus="first"
        width={280}
      >
        <div className="p-2" onKeyDownCapture={onKeyDown}>
          <div className="mb-2 text-[12px] font-medium text-ink">Select overlapping</div>
          <div className="mb-2 flex gap-1" aria-label="Candidate filters">
            {CANDIDATE_KINDS.map((kind) => (
              <button
                key={kind}
                type="button"
                aria-pressed={session.filters[kind]}
                onClick={(event) => {
                  toggleFilter(kind);
                  event.currentTarget.focus({ preventScroll: true });
                }}
                className="rounded bg-chip px-2 py-1 text-[11px] text-ink-3 hover:bg-hover-2"
              >
                {kind.charAt(0).toUpperCase() + kind.slice(1)}
              </button>
            ))}
          </div>
          <div role="list" aria-label="Overlapping geometry candidates" className="space-y-1">
            {candidates.map((candidate) => {
              const selected = candidate === active;
              const index = session.candidates.indexOf(candidate);
              return (
                <button
                  key={`${candidate.bodyId}/${candidate.kind}/${candidate.topoKey}`}
                  ref={(node) => {
                    if (node) candidateButtons.current.set(index, node);
                    else candidateButtons.current.delete(index);
                  }}
                  type="button"
                  autoFocus={selected}
                  disabled={session.committing}
                  aria-current={selected ? "true" : undefined}
                  onFocus={() => setActive(candidate)}
                  onMouseEnter={() => setActive(candidate)}
                  onPointerEnter={(event) => event.currentTarget.focus({ preventScroll: true })}
                  onClick={() => { setActive(candidate); void commit(candidate); }}
                  className="block w-full rounded px-2 py-1.5 text-left text-[12px] text-ink-3 hover:bg-hover-2 disabled:opacity-60"
                >
                  {candidateLabel(candidate)}
                </button>
              );
            })}
            {candidates.length === 0 && <div className="px-2 py-1 text-[12px] text-ink-6">No candidates in this filter.</div>}
          </div>
        </div>
      </Popover>
    </>
  );
}
