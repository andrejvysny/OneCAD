import { useCallback, useState } from "react";
import { cn } from "@/ui/cn";
import { Icon } from "@/icons/Icon";
import { SectionLabel } from "@/ui/SectionLabel";
import { useRepairStore, repairStore } from "@/stores/repairStore";
import { useDocumentStore } from "@/stores/documentStore";
import { createClient } from "@/ipc/client";
import {
  rebindCandidate,
  repairInputPath,
  repairMateAxisChoice,
  REPAIR_NOT_REBINDABLE,
} from "@/features/inspector/historyActions";
import type { NeedsRepairItem, ResolveCandidate } from "@/ipc/types";
import { OperationDiagnosticDetails } from "@/features/inspector/OperationDiagnosticDetails";
import { MATE_AXIS_KEEP_LABEL } from "@/ipc/operationDiagnostics";

type CandidateLoad =
  | { status: "loading" }
  | { status: "error"; message: string }
  // H9: the slot this refId names has no `EditOperationInput` path (a whole-body
  // input, or an op with no addressable slots). Showing clickable candidates here
  // would send a command the backend can only reject — say so instead.
  | { status: "not-rebindable" }
  | {
      status: "ready";
      candidates: ResolveCandidate[];
      snapshotId: number;
      revision: number;
      bodyId?: string;
      /** SCHEMA §9 `uiLabel` (WP-I) — the informational text for `mateSeatOffFace`. */
      uiLabel?: string;
    };

function loadKey(revision: number, snapshotId: number, refId: string): string {
  return `${revision}:${snapshotId}:${refId}`;
}

function isCurrentRepair(refId: string, revision: number, snapshotId: number): boolean {
  const state = repairStore.getState();
  return (
    state.revision === revision &&
    state.snapshotId === snapshotId &&
    state.expandedRefId === refId &&
    state.items.some((item) => item.refId === refId)
  );
}

/** Humanize a repair reason token for display. */
function reasonText(reason: string): string {
  switch (reason) {
    case "ambiguous":
      return "Ambiguous — several candidates match";
    case "no-candidates":
      return "No candidate matched";
    case "low-confidence":
      return "Low confidence — no clear match";
    case "ordinal-permutation":
      // VF-B6: the split children of an N-body op changed geometric order, so
      // `:<k>` names a different solid than this reference was authored against.
      return "Split pieces swapped order — this may point at a different body";
    case "legacyReferenceFace":
      // WP-F (SCHEMA §7.3/§9): this chamfer's two legs used to land on whichever
      // adjacent face had the smaller ordinal, and an upstream edit has just
      // reordered them — so which face carries the first distance is no longer
      // trustworthy. The two candidates ARE the edge's two adjacent faces, and
      // picking one persists the choice for good.
      return "Chamfer reference face was never recorded — choose which face the first distance is measured on";
    case "mateAxisReversed":
      // WP-I (SCHEMA §9): a cylindrical mate's target has no intrinsic axial
      // sign, so a re-authored target can flip it with nothing on the face to
      // tell that apart from a rigid 180° turn — the two op-built candidates
      // (`candidate.label`) are the two ways to answer that, not a rebind.
      return "This mate's axis may have reversed — choose which direction the component should follow";
    case "mateSeatOffFace":
      // WP-I: informational only — the worker could not re-seat the mate on the
      // target face at all, so there is nothing to pick, only to read.
      return "This mate's seat moved off the target face";
    default:
      // Unknown tokens (a newer build's reason) fall through verbatim rather than
      // being swallowed — the panel is deliberately generic over `reason`.
      return reason;
  }
}

/** The trailing `input<k>` slot of a refId (or the whole id if it has no dot). */
function refTail(refId: string): string {
  const dot = refId.lastIndexOf(".");
  return dot >= 0 ? refId.slice(dot + 1) : refId;
}

/**
 * The inspector REPAIR state (M4b): one card per NeedsRepair ref. Expanding a card
 * fetches the ranked candidates via `resolveRefs` and renders each as a score
 * meter + summary; clicking a candidate rebinds the ref (promote → EditOperationInput).
 */
export function RepairPanel() {
  const items = useRepairStore((s) => s.items);
  const revision = useRepairStore((s) => s.revision);
  const snapshotId = useRepairStore((s) => s.snapshotId);
  const expandedRefId = useRepairStore((s) => s.expandedRefId);
  const features = useDocumentStore((s) => s.features);
  const [loads, setLoads] = useState<Record<string, CandidateLoad>>({});
  const [busyRefId, setBusyRefId] = useState<string | null>(null);

  const labelFor = useCallback(
    (item: NeedsRepairItem): string => {
      const feat = features.find((f) => f.id === item.opId);
      // Fall back to a short opId prefix when the feature is not in the projection.
      return feat?.label ?? `Feature ${item.opId.slice(0, 6)}`;
    },
    [features],
  );

  const expand = useCallback((item: NeedsRepairItem) => {
    const state = repairStore.getState();
    const eventRevision = state.revision;
    const eventSnapshotId = state.snapshotId;
    const next = state.expandedRefId === item.refId ? null : item.refId;
    repairStore.getState().setExpanded(next);
    // A cached `not-rebindable` verdict is NOT sticky: it is derived from the
    // item's op TYPE, which comes from the projection — so a verdict reached
    // before the projection hydrated must be re-taken on the next expand rather
    // than telling the user forever that a rebindable slot cannot be rebound.
    const key = loadKey(eventRevision, eventSnapshotId, item.refId);
    const cached = loads[key];
    if (next && (!cached || cached.status === "not-rebindable")) {
      setLoads((s) => ({ ...s, [key]: { status: "loading" } }));
      // WP-I: `mateAxisReversed`/`mateSeatOffFace` items name a `PlaceComponent`
      // record, which has no `InputPath` slot table (it is not an
      // `EditOperationInput` rebind at all — `mateAxisReversed`'s action is
      // `repairMateAxis`, and `mateSeatOffFace` has no action). Skip the
      // InputPath gate that would otherwise call these "not rebindable".
      const isMateItem = item.reason === "mateAxisReversed" || item.reason === "mateSeatOffFace";
      (isMateItem ? Promise.resolve(true) : repairInputPath(item))
        .then(async (path) => {
          if (!isCurrentRepair(item.refId, eventRevision, eventSnapshotId)) return;
          if (!path) {
            setLoads((s) => ({ ...s, [key]: { status: "not-rebindable" } }));
            return;
          }
          const results = await createClient().resolveRefs([
            { refId: item.refId, snapshotId: eventSnapshotId, revision: eventRevision },
          ]);
          if (!isCurrentRepair(item.refId, eventRevision, eventSnapshotId)) return;
          const r = results.find((x) => x.refId === item.refId) ?? results[0];
          if (
            !r ||
            r.refId !== item.refId ||
            r.snapshotId !== eventSnapshotId ||
            r.revision !== eventRevision
          ) {
            throw new Error("Repair candidates expired — reopen this reference");
          }
          // Denormalize the ref-level authoritative body onto each candidate — the
          // wire only carries it once, on `r.bodyId` (see `ResolveCandidate.bodyId`).
          const candidates = [...(r?.candidates ?? [])]
            .map((c) => ({ ...c, bodyId: r.bodyId }))
            .sort((a, b) => b.score - a.score);
          setLoads((s) => ({
            ...s,
            [key]: {
              status: "ready",
              candidates,
              snapshotId: r.snapshotId,
              revision: r.revision,
              bodyId: r.bodyId,
              uiLabel: r.uiLabel,
            },
          }));
        })
        .catch((e: unknown) => {
          if (!isCurrentRepair(item.refId, eventRevision, eventSnapshotId)) return;
          setLoads((s) => ({
            ...s,
            [key]: { status: "error", message: e instanceof Error ? e.message : String(e) },
          }));
        });
    }
  }, [loads]);

  const choose = useCallback(async (
    item: NeedsRepairItem,
    candidate: ResolveCandidate,
    load: Extract<CandidateLoad, { status: "ready" }>,
  ) => {
    if (!isCurrentRepair(item.refId, load.revision, load.snapshotId)) return;
    setBusyRefId(item.refId);
    try {
      // WP-I: `mateAxisReversed` is not a rebind — the two candidates are the two
      // ANSWERS to one question, told apart only by `candidate.label` (both rows
      // share a TopoKey and score, so a stable sort over equal scores is not a
      // promise of row order — route on the label, never the click's row index).
      if (item.reason === "mateAxisReversed") {
        await repairMateAxisChoice(item, candidate.label === MATE_AXIS_KEEP_LABEL);
      } else {
        await rebindCandidate(item, candidate, load.snapshotId);
      }
    } finally {
      setBusyRefId(null);
      repairStore.getState().setHoveredWorldPos(null);
    }
  }, []);

  return (
    <>
      <div className="flex items-center justify-between">
        <div className="text-[15px] font-semibold text-ink">Repair references</div>
        <button
          type="button"
          aria-label="Close repair panel"
          data-testid="repair-close"
          onClick={() => repairStore.getState().closePanel()}
          className="flex h-6 w-6 items-center justify-center rounded-sm text-ink-5 hover:bg-hover-2"
        >
          <Icon name="x" size={14} strokeWidth={1.8} />
        </button>
      </div>
      <div className="mt-0.5 text-[12px] text-warn">
        {items.length === 1 ? "1 reference" : `${items.length} references`} could not be re-bound after
        the last edit.
      </div>

      <SectionLabel className="pb-1.5 pt-4">References</SectionLabel>
      <div>
        {items.map((item) => {
          const open = expandedRefId === item.refId;
          const load = loads[loadKey(revision, snapshotId, item.refId)];
          const busy = busyRefId === item.refId;
          const diagnostics = features.find((feature) => feature.id === item.opId)?.diagnostics;
          return (
            <div
              key={item.refId}
              data-testid={`repair-item-${item.refId}`}
              className={cn(
                "mb-1.5 rounded-sm border",
                open ? "border-warn-border bg-warn-surface" : "border-border bg-chip",
              )}
            >
              <button
                type="button"
                data-testid={`repair-item-head-${item.refId}`}
                onClick={() => expand(item)}
                disabled={busy}
                className="flex w-full items-center gap-2 px-2.5 py-2 text-left"
              >
                <Icon
                  name="chevronDown"
                  size={13}
                  strokeWidth={1.8}
                  className={cn("text-ink-5 transition-transform", open ? "" : "-rotate-90")}
                />
                <span className="flex-1">
                  <span className="block text-[12.5px] font-medium text-ink-2">{labelFor(item)}</span>
                  {/*
                    * The count comes from the SAME `ResolveRefs` response that
                    * renders the selectable rows (U7).
                    *
                    * It used to come from the `needs-repair` EVENT
                    * (`item.candidateCount`) while the rows came from a separate
                    * `resolveRefs` call, so the panel could say "5 candidates"
                    * directly above "No candidates to choose from" — the
                    * contradiction the UX audit flagged (finding 06). The event's
                    * count is a scoring hint, not a promise that any of them is
                    * an eligible rebind target.
                    *
                    * Until that response lands the row shows the REASON only.
                    * Saying nothing is honest; saying a number we have not
                    * verified is not.
                    */}
                  <span className="block text-[11.5px] text-ink-5">
                    {reasonText(item.reason)} · {refTail(item.refId)}
                    {load?.status === "ready" &&
                      ` · ${load.candidates.length} ${load.candidates.length === 1 ? "candidate" : "candidates"}`}
                  </span>
                </span>
              </button>

              <OperationDiagnosticDetails diagnostics={diagnostics} />

              {open && (
                <div className="border-t border-warn-border px-2.5 py-2">
                  {(!load || load.status === "loading") && (
                    <div className="text-[11.5px] text-ink-5">Resolving candidates…</div>
                  )}
                  {load?.status === "error" && (
                    <div className="text-[11.5px] text-warn-strong">{load.message}</div>
                  )}
                  {load?.status === "not-rebindable" && (
                    <div
                      data-testid={`repair-not-rebindable-${item.refId}`}
                      className="text-[11.5px] text-ink-5"
                    >
                      {REPAIR_NOT_REBINDABLE}
                    </div>
                  )}
                  {/* WP-I: `mateSeatOffFace` is informational only — the worker
                    * could not re-seat the mate at all, so there is one candidate
                    * and no action, just the `uiLabel` (or the candidate's own
                    * summary as a fallback) telling the user what happened. */}
                  {load?.status === "ready" && item.reason === "mateSeatOffFace" && (
                    <div
                      data-testid={`repair-info-${item.refId}`}
                      className="text-[11.5px] text-ink-5"
                    >
                      {load.uiLabel ?? load.candidates[0]?.summary ?? reasonText(item.reason)}
                    </div>
                  )}
                  {load?.status === "ready" &&
                    item.reason !== "mateSeatOffFace" &&
                    load.candidates.length === 0 && (
                      <div className="text-[11.5px] text-ink-5">No candidates to choose from.</div>
                    )}
                  {load?.status === "ready" &&
                    item.reason !== "mateSeatOffFace" &&
                    load.candidates.map((c, i) => (
                      <CandidateRow
                        key={`${c.topoKey}-${i}`}
                        refId={item.refId}
                        candidate={c}
                        disabled={busy}
                        onChoose={() => void choose(item, c, load)}
                      />
                    ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </>
  );
}

/** One candidate row: a score meter + summary + topoKey; click rebinds. */
function CandidateRow({
  refId,
  candidate,
  disabled,
  onChoose,
}: {
  refId: string;
  candidate: ResolveCandidate;
  disabled: boolean;
  onChoose: () => void;
}) {
  const pct = Math.max(0, Math.min(100, Math.round(candidate.score * 100)));
  return (
    <button
      type="button"
      data-testid={`repair-candidate-${refId}-${candidate.topoKey}`}
      disabled={disabled}
      onClick={onChoose}
      // Hover publishes the candidate world position to the repair store — a clean
      // DATA seam a future engine marker can subscribe to (no engine coupling here).
      onMouseEnter={() => repairStore.getState().setHoveredWorldPos(candidate.worldPos)}
      onMouseLeave={() => repairStore.getState().setHoveredWorldPos(null)}
      className={cn(
        "mb-1 flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left",
        disabled ? "opacity-50" : "bg-surface hover:bg-hover-2",
      )}
    >
      <span className="flex-1">
        {/* WP-I: an op-built item (e.g. `mateAxisReversed`) carries two
          * candidates that are geometrically identical — `label` is the only
          * thing that tells them apart, and takes priority over `summary`. */}
        <span className="block text-[12px] text-ink-2">{candidate.label ?? candidate.summary}</span>
        <span className="mt-1 block h-[4px] w-full overflow-hidden rounded-full bg-well">
          <span className="block h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
        </span>
      </span>
      <span className="w-9 text-right text-[11px] font-medium tabular-nums text-ink-4">{pct}%</span>
    </button>
  );
}
