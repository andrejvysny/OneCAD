import { Button } from "@/ui/Button";
import type { RecoveryInfo } from "@/ipc/types";

type RecoveryCardProps = {
  recovery: RecoveryInfo[];
  /** `documentId` of the offer whose decision is in flight, if any. */
  pendingId: string | null;
  onRestore: (documentId: string) => void;
  onDiscard: (documentId: string) => void;
};

/**
 * What to call an offer.
 *
 * Ladder: the title the marker recorded → the saved file's stem → a generic label.
 * The last rung is reached by a never-saved document AND by an ORPHAN (a container
 * whose marker did not survive), and it must stay generic: the autosave file is
 * named `<documentId>.onecad`, so deriving a name from it would show the user a raw
 * UUID and tell them nothing about which document they are being offered.
 */
function documentName(offer: RecoveryInfo): string {
  if (offer.title) return offer.title;
  if (!offer.originalPath) return "Untitled document";
  const file = offer.originalPath.split(/[\\/]/).pop() ?? offer.originalPath;
  return file.replace(/\.[^.]+$/, "");
}

/**
 * Human date AND time for the autosave mtime.
 *
 * The time is the point: "how much work is in this?" is the only question the user
 * has, and a date alone renders a crash ninety seconds ago as indistinguishable
 * from one last Tuesday. `0` means the filesystem could not answer — show nothing
 * rather than "Jan 1, 1970".
 */
function formatModified(modifiedMs: number): string | null {
  if (!modifiedMs) return null;
  return new Date(modifiedMs).toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * Crash-recovery attention card (start screen). Shown above the recent-projects
 * list when crashed sessions left autosaves behind; offers to Restore or Discard
 * each one. Warn-token styling mirrors RepairBanner / RepairPanel so it reads as
 * the same "needs attention" affordance.
 *
 * Renders EVERY outstanding offer, newest first. Several sessions can have crashed,
 * and a single-offer card left the rest with no way to be seen or resolved.
 */
export function RecoveryCard({ recovery, pendingId, onRestore, onDiscard }: RecoveryCardProps) {
  if (recovery.length === 0) return null;
  return (
    <div className="mb-[22px] rounded-md border border-warn-border bg-warn-surface px-[15px] py-3">
      <div className="text-[13px] font-semibold text-warn-strong">
        {recovery.length === 1
          ? "Unsaved changes recovered"
          : `${recovery.length} documents with unsaved changes`}
      </div>
      <ul className="mt-2 flex flex-col gap-2">
        {recovery.map((offer) => {
          const modified = formatModified(offer.modifiedMs);
          const busy = pendingId === offer.documentId;
          return (
            <li key={offer.documentId} className="flex items-center gap-2.5">
              <span aria-hidden="true" className="h-[7px] w-[7px] shrink-0 rounded-full bg-warn" />
              <div className="flex-1 text-[11.5px] text-warn">
                <span className="font-medium">{documentName(offer)}</span>
                {modified && ` · autosaved ${modified}`}
              </div>
              <div className="flex shrink-0 gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={pendingId !== null}
                  onClick={() => onDiscard(offer.documentId)}
                >
                  Discard
                </Button>
                <Button
                  variant="primary"
                  size="sm"
                  disabled={pendingId !== null}
                  onClick={() => onRestore(offer.documentId)}
                >
                  {busy ? "Restoring…" : "Restore"}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
