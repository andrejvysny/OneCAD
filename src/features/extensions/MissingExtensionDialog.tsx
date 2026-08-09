import { Icon } from "@/icons/Icon";
import { Button } from "@/ui/Button";
import { MonoValue } from "@/ui/MonoValue";
import { useExtensionsStore } from "@/stores/extensionsStore";

/**
 * Details for one missing extension (prototype 2a).
 *
 * The copy is the whole feature: a user seeing an unfamiliar id needs to know
 * that nothing was lost and that continuing is safe. Both buttons are
 * non-destructive — "Find extension" opens the manager rather than fetching
 * anything, because there is no registry to fetch from yet and a button that
 * silently does nothing is worse than one that shows you where it would look.
 *
 * Only the module ID is shown, not a friendly name: the name lives in a
 * manifest this build does not have, and inventing one from the id would put
 * words in an absent author's mouth.
 */
export function MissingExtensionDialog() {
  const detailsFor = useExtensionsStore((s) => s.detailsFor);
  const missing = useExtensionsStore((s) => s.missing);
  const close = useExtensionsStore((s) => s.closeDetails);
  const openManager = useExtensionsStore((s) => s.openManager);

  if (detailsFor === null) return null;
  const entry = missing.find((m) => m.moduleId === detailsFor);

  return (
    <div
      className="absolute inset-0 z-[80] flex items-center justify-center bg-scrim"
      onClick={close}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Missing extension"
        data-testid="missing-extension-dialog"
        onClick={(e) => e.stopPropagation()}
        className="w-[400px] rounded-lg bg-surface p-[22px_24px] shadow-popover"
      >
        <div className="flex items-center gap-2.5">
          <div className="flex h-9 w-9 flex-none items-center justify-center rounded-md bg-warn-surface">
            <Icon name="alert" size={18} strokeWidth={1.8} className="text-warn" />
          </div>
          <div>
            <div className="text-[14px] font-semibold text-ink">Missing extension</div>
            <div className="text-[12px] text-ink-5">Not installed in this build</div>
          </div>
        </div>

        <MonoValue className="mb-2 mt-3 block text-[11px] text-ink-6">{detailsFor}</MonoValue>
        {entry && (
          <MonoValue className="mb-2 block text-[11px] text-ink-6">
            schema v{entry.schemaVersion}
          </MonoValue>
        )}

        <p className="m-0 text-[12.5px] leading-[1.6] text-titlebar-text">
          This project contains data created by this extension. The data is preserved and will be
          written back unchanged when you save — it simply cannot be displayed or edited until the
          extension is installed.
        </p>

        <div className="mt-5 flex justify-end gap-2">
          <Button variant="secondary" data-testid="missing-extension-continue" onClick={close}>
            Continue without it
          </Button>
          <Button
            data-testid="missing-extension-find"
            onClick={() => {
              close();
              openManager("browse");
            }}
          >
            Find extension
          </Button>
        </div>
      </div>
    </div>
  );
}
