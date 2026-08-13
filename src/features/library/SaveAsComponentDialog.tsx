/*
 * "Save as Component" (spec §7) — the authoring dialog.
 *
 * Reached from the model tree's context menu on a body (a `Slots.TreeContext`
 * contribution, WP-B1), never from a modeling surface: the library owns this
 * flow and modeling does not know it exists.
 *
 * WHAT THIS DIALOG DELIBERATELY DOES NOT DO, and why it says so on screen:
 *
 * - **No face-clicking to place attachments.** The placement solver seats a
 *   component by its local origin and +Z (`placementSolver.ts`), and nothing
 *   on the wire carries a per-attachment frame — so an attachment placed on an
 *   arbitrary face could not actually be honoured at placement time. The
 *   authoring rule is therefore the same one every built-in generator follows:
 *   the component seats at its MODEL ORIGIN with +Z out of the seating plane.
 *   The dialog states that instead of offering a picker whose result would be
 *   quietly ignored.
 * - **No parameter roles.** A `document`-kind package has geometry baked at
 *   authoring time, and `setComponentParams` already refuses to edit one (there
 *   is no re-bake lane). Declaring free params here would offer an edit that
 *   cannot be honoured.
 *
 * Both are recorded in TODO.md as the follow-up, not hidden here.
 */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/ui/Button";
import { TextInput } from "@/ui/TextInput";
import { createClient } from "@/ipc/client";
import type { NewComponentSpec } from "@/ipc/types";
import { getViewportEngine } from "@/viewport/engineBridge";
import { viewportStore } from "@/stores/viewportStore";

/** `"Bracket Plate"` → `"bracket-plate"`, for the id's trailing segment. */
export function slugify(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/** `"mine.bracket-plate"`, or `""` when the name carries nothing usable. */
export function suggestedId(name: string): string {
  const slug = slugify(name);
  return slug ? `mine.${slug}` : "";
}

/**
 * Whether `id` is shaped like a component id: namespaced, lowercase-ish
 * segments. Mirrors the backend's own rule (`validate_identity`) so the dialog
 * refuses before a round trip rather than after one.
 */
export function isValidComponentId(id: string): boolean {
  return /^[a-z0-9][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)+$/.test(id.trim());
}

/** Semver triple. The backend only requires non-empty; this asks for the real shape. */
export function isValidVersion(version: string): boolean {
  return /^\d+\.\d+\.\d+$/.test(version.trim());
}

export interface SaveAsComponentDialogProps {
  /** The body being authored, or `null` when the dialog is closed. */
  bodyId: string | null;
  /** The row's label — the default component name. */
  bodyName?: string;
  onClose: () => void;
}

export function SaveAsComponentDialog({ bodyId, bodyName, onClose }: SaveAsComponentDialogProps) {
  const [name, setName] = useState(bodyName ?? "");
  const [id, setId] = useState(suggestedId(bodyName ?? ""));
  const [version, setVersion] = useState("1.0.0");
  const [tags, setTags] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  // The id follows the name until the user edits it themselves — after that it
  // is theirs, and a later name change must not overwrite what they typed.
  const idTouched = useRef(false);

  useEffect(() => {
    if (!bodyId) return;
    setName(bodyName ?? "");
    setId(suggestedId(bodyName ?? ""));
    setVersion("1.0.0");
    setTags("");
    setError(null);
    idTouched.current = false;
    nameInput.current?.focus();
    nameInput.current?.select();
  }, [bodyId, bodyName]);

  if (!bodyId) return null;

  const trimmedName = name.trim();
  const canCommit =
    !saving && trimmedName.length > 0 && isValidComponentId(id) && isValidVersion(version);

  const commit = async () => {
    if (!canCommit) return;
    setSaving(true);
    setError(null);
    const spec: NewComponentSpec = {
      id: id.trim(),
      version: version.trim(),
      name: trimmedName,
      category: ["mine"],
      tags: tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
      // One attachment, matching what the origin convention actually offers: a
      // seating plane. `accepts: ["plane"]` is what the snap solver matches a
      // hovered planar face against.
      attachments: { seat: { on: "face:origin", accepts: ["plane"] } },
    };
    try {
      const saved = await createClient().saveAsComponent(
        bodyId,
        spec,
        getViewportEngine()?.captureThumbnail(256) ?? null,
      );
      onClose();
      viewportStore
        .getState()
        .setStatusHint(`Saved “${saved.name}” to the library — find it under Library ▸ mine.`);
    } catch (e) {
      // Stays OPEN on failure: the backend's refusals (a taken id@version, a
      // body that bakes to more than one solid) are all things the author fixes
      // right here, and closing would throw away everything they typed.
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[95] flex items-start justify-center bg-scrim pt-[110px]"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Save as component"
        data-testid="save-as-component-dialog"
        onClick={(e) => e.stopPropagation()}
        className="w-[420px] rounded-lg border border-border bg-surface p-[20px_22px] font-ui shadow-popover"
      >
        <div className="text-[14px] font-semibold text-ink">Save as component</div>
        <div className="mt-[3px] text-[12px] text-ink-5">
          Captures this body as a reusable component. It seats at the body’s model origin with
          +Z out of the seating plane — the same convention the built-in fasteners use.
        </div>

        <label className="mt-4 block text-[11.5px] text-ink-5" htmlFor="sac-name">
          Name
        </label>
        <TextInput
          ref={nameInput}
          id="sac-name"
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            if (!idTouched.current) setId(suggestedId(e.target.value));
          }}
          aria-label="Component name"
          data-testid="save-as-component-name"
          wrapperClassName="mt-1 w-full"
          onKeyDown={(e) => e.stopPropagation()}
        />

        <label className="mt-3 block text-[11.5px] text-ink-5" htmlFor="sac-id">
          Id
        </label>
        <TextInput
          id="sac-id"
          value={id}
          onChange={(e) => {
            idTouched.current = true;
            setId(e.target.value);
          }}
          aria-label="Component id"
          data-testid="save-as-component-id"
          wrapperClassName="mt-1 w-full"
          onKeyDown={(e) => e.stopPropagation()}
        />
        {id.length > 0 && !isValidComponentId(id) && (
          <div className="mt-1 text-[11.5px] text-traffic-close" data-testid="save-as-component-id-error">
            An id is namespaced and lowercase, like <code>mine.bracket-plate</code>.
          </div>
        )}

        <div className="mt-3 flex gap-2">
          <div className="w-[110px]">
            <label className="block text-[11.5px] text-ink-5" htmlFor="sac-version">
              Version
            </label>
            <TextInput
              id="sac-version"
              value={version}
              onChange={(e) => setVersion(e.target.value)}
              aria-label="Component version"
              data-testid="save-as-component-version"
              wrapperClassName="mt-1 w-full"
              onKeyDown={(e) => e.stopPropagation()}
            />
          </div>
          <div className="min-w-0 flex-1">
            <label className="block text-[11.5px] text-ink-5" htmlFor="sac-tags">
              Tags
            </label>
            <TextInput
              id="sac-tags"
              value={tags}
              onChange={(e) => setTags(e.target.value)}
              placeholder="comma, separated"
              aria-label="Component tags"
              data-testid="save-as-component-tags"
              wrapperClassName="mt-1 w-full"
              onKeyDown={(e) => e.stopPropagation()}
            />
          </div>
        </div>

        {error && (
          <div
            role="alert"
            data-testid="save-as-component-error"
            className="mt-3 text-[11.5px] text-traffic-close"
          >
            {error}
          </div>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <Button variant="secondary" data-testid="save-as-component-cancel" onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={!canCommit}
            data-testid="save-as-component-commit"
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
