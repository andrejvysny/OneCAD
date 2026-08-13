/*
 * "Save as Template" (spec §8) — freezes the OPEN document as a starter.
 *
 * Reached from the command palette (`onecad.library.command.saveAsTemplate`)
 * rather than the File menu: the File menu lives in `features/shell`, and the
 * shell must not import the library to offer a library action. A registered
 * command is the module-owned way in, and the palette already lists every one.
 *
 * Nothing about the open document changes — it keeps its path, title and dirty
 * flag. Saving a template is not saving your work, and a dialog that quietly
 * marked the document clean would be lying about both.
 */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/ui/Button";
import { TextInput } from "@/ui/TextInput";
import { createClient } from "@/ipc/client";
import { getViewportEngine } from "@/viewport/engineBridge";
import { viewportStore } from "@/stores/viewportStore";
import { slugify } from "./SaveAsComponentDialog";

export function SaveAsTemplateDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    setName("");
    setDescription("");
    setError(null);
    input.current?.focus();
  }, [open]);

  if (!open) return null;

  // The id is derived, not asked for: a template is never referenced from a
  // document (unlike a component's recorded identity), so there is nothing for
  // the user to keep stable by hand.
  const id = slugify(name);
  const canCommit = !saving && id.length > 0;

  const commit = async () => {
    if (!canCommit) return;
    setSaving(true);
    setError(null);
    try {
      const saved = await createClient().saveAsTemplate(
        id,
        name.trim(),
        description.trim() || undefined,
        getViewportEngine()?.captureThumbnail(256) ?? null,
      );
      onClose();
      viewportStore
        .getState()
        .setStatusHint(`Saved “${saved.name}” as a template — it appears on the start screen.`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[95] flex items-start justify-center bg-scrim pt-[120px]"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Save as template"
        data-testid="save-as-template-dialog"
        onClick={(e) => e.stopPropagation()}
        className="w-[400px] rounded-lg border border-border bg-surface p-[20px_22px] font-ui shadow-popover"
      >
        <div className="text-[14px] font-semibold text-ink">Save as template</div>
        <div className="mt-[3px] text-[12px] text-ink-5">
          Freezes this document as a starter on the start screen. Your open project is untouched —
          starting from a template always creates a new untitled copy.
        </div>

        <label className="mt-4 block text-[11.5px] text-ink-5" htmlFor="sat-name">
          Name
        </label>
        <TextInput
          ref={input}
          id="sat-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          aria-label="Template name"
          data-testid="save-as-template-name"
          wrapperClassName="mt-1 w-full"
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") void commit();
            else if (e.key === "Escape") onClose();
          }}
        />

        <label className="mt-3 block text-[11.5px] text-ink-5" htmlFor="sat-description">
          Description
        </label>
        <TextInput
          id="sat-description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="optional"
          aria-label="Template description"
          data-testid="save-as-template-description"
          wrapperClassName="mt-1 w-full"
          onKeyDown={(e) => e.stopPropagation()}
        />

        {error && (
          <div
            role="alert"
            data-testid="save-as-template-error"
            className="mt-3 text-[11.5px] text-traffic-close"
          >
            {error}
          </div>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <Button variant="secondary" data-testid="save-as-template-cancel" onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={!canCommit}
            data-testid="save-as-template-commit"
            onClick={() => void commit()}
          >
            {saving ? "Saving…" : "Save"}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
