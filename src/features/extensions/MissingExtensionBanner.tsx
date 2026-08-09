import { Icon } from "@/icons/Icon";
import { Button } from "@/ui/Button";
import { useExtensionsStore } from "@/stores/extensionsStore";

/**
 * Non-blocking notice: this document carries state for an extension this build
 * does not have (prototype 2a).
 *
 * NON-BLOCKING IS THE POINT. A missing module never refuses to open a document
 * and never discards its slice — the state round-trips verbatim (ADR-0005). So
 * the banner says exactly that ("Its data has been preserved"), offers details,
 * and can be dismissed; nothing about it gates the editor.
 *
 * It renders nothing when nothing is missing, which is the normal case.
 *
 * PLACEMENT: `shell.top`, in FLOW under the title bar, not `shell.notification`.
 * The notification slot's occupants are absolutely-positioned pills floating
 * over the viewport; a full-width strip that must push the work area down
 * cannot be one of them without covering the toolbar it sits above.
 */
export function MissingExtensionBanner() {
  const missing = useExtensionsStore((s) => s.missing);
  const dismissed = useExtensionsStore((s) => s.dismissed);
  const dismiss = useExtensionsStore((s) => s.dismissBanner);
  const showDetails = useExtensionsStore((s) => s.showDetails);

  if (dismissed || missing.length === 0) return null;

  const count = missing.length;
  const noun = count === 1 ? "extension that is" : "extensions that are";

  return (
    <div
      role="status"
      data-testid="missing-extension-banner"
      className="flex h-[38px] flex-none items-center gap-2.5 border-b border-warn-border bg-warn-surface px-3.5"
    >
      <Icon name="alert" size={15} strokeWidth={1.8} className="flex-none text-warn" />
      <span className="text-[12.5px] text-warn-strong">
        This project uses {count} {noun} not installed. Its data has been preserved.
      </span>
      <span className="flex-1" />
      <Button
        size="sm"
        variant="secondary"
        data-testid="missing-extension-details"
        onClick={() => showDetails(missing[0].moduleId)}
        className="border-warn-border text-warn-strong"
      >
        View details
      </Button>
      <Button
        size="sm"
        variant="ghost"
        data-testid="missing-extension-dismiss"
        onClick={dismiss}
        className="text-warn"
      >
        Dismiss
      </Button>
    </div>
  );
}
