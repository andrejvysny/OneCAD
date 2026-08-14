/*
 * "Save as Component" (spec §7) — the authoring dialog.
 *
 * Reached from the model tree's context menu on a body (a `Slots.TreeContext`
 * contribution, WP-B1), never from a modeling surface: the library owns this
 * flow and modeling does not know it exists.
 *
 * ATTACHMENTS ARE PICKED IN THE VIEWPORT (WP-F1.2). "Add attachment" arms
 * `attachmentPicker`, and each click on a planar face, a cylindrical face or a
 * circular edge authors a named attachment WITH ITS OWN LOCAL FRAME — the wire
 * support for which landed as WP-F1.1, which is what made this honest to offer.
 * Before that, an attachment on an arbitrary face could not have been honoured
 * at placement time, so the dialog only offered the model-origin convention.
 * That convention is still the fallback: a component saved with no picked
 * attachment gets the same origin seat it always did.
 *
 * A BODY THAT BAKES TO SEVERAL SOLIDS gets an offer, not just a refusal: spec
 * §9 requires one solid, and the dialog can re-submit the same save with
 * `unionSolids`, which fuses inside the BAKE (the open document keeps its
 * bodies). The other two answers — pick one solid, split into several
 * components — need a solid-picking UI that does not exist yet, and the offer
 * says so rather than pretending they are unavailable for a deeper reason.
 *
 * PARAMETERS ARE THE DOCUMENT'S VARIABLES (WP-F1.3). Each checked variable
 * becomes a `role: "free"` parameter bound to that variable by name; editing it
 * on a placed instance replays this document with the new value and re-bakes.
 * Only variables can be offered: a re-bake sets variables, so a parameter with
 * nothing to set would be an edit that cannot be honoured. A document with no
 * variables therefore offers nothing, and a component with nothing checked
 * behaves exactly as every pre-WP-F1.3 authored package.
 *
 * WHAT THIS DIALOG STILL DELIBERATELY DOES NOT DO:
 *
 * - **No `role: "table"`/`"computed"` parameters, no domains, no min/snap.**
 *   Those describe a catalog part driven by a dimension table, which authoring
 *   from a document does not produce.
 *
 * Recorded in TODO.md as the follow-up, not hidden here.
 */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/ui/Button";
import { TextInput } from "@/ui/TextInput";
import { createClient } from "@/ipc/client";
import type {
  DocumentVariable,
  LibraryAttachment,
  LibraryAttachmentFrame,
  NewComponentParameter,
  NewComponentSpec,
} from "@/ipc/types";
import { getViewportEngine } from "@/viewport/engineBridge";
import { viewportStore } from "@/stores/viewportStore";
import {
  beginAttachmentPick,
  cancelAttachmentPick,
  DEFAULT_NAME,
  ON_PREFIX,
  PICK_HINT,
  type AttachmentKind,
} from "@/modules/library/attachmentPicker";

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

/**
 * An attachment name is a manifest TABLE KEY (`[attachments].<key>`) and a mate
 * is recorded BY that name, so it stays to the character set every seeded
 * package uses. Uniqueness is checked separately — a duplicate would silently
 * overwrite the earlier row when the record is built.
 */
/*
 * The dialog opens pre-filled with the BODY's name, which for most bodies is
 * whatever the tree auto-generated — "Body 2". That default then becomes a
 * permanent library identity, and `suggestedId` mints `mine.body-2` from it
 * (LGU-1 WP-A, defect F10).
 *
 * A WARNING, not a refusal. "Body 2" is a legal name and the backend's id rules
 * stay the authority on what is acceptable; this only makes sure the user
 * chose it rather than inherited it.
 */
export function isPlaceholderName(name: string): boolean {
  return /^(body|sketch|feature)\s*\d*$/i.test(name.trim());
}

export function isValidAttachmentName(name: string): boolean {
  return /^[a-z0-9_-]+$/.test(name);
}

/** The marker `library.rs` puts in the multi-solid refusal. Kept in sync there. */
export const MULTI_SOLID_REFUSAL = "MULTI_SOLID_BODY";

/** One authored attachment row. `id` is React identity only — never sent. */
interface AttachmentRow {
  id: number;
  name: string;
  kind: AttachmentKind;
  accepts: string[];
  frame: LibraryAttachmentFrame;
}

/** `"seat"`, then `"seat2"`, `"seat3"`… — the first name not already taken. */
function uniqueName(rows: readonly AttachmentRow[], base: string): string {
  const taken = new Set(rows.map((r) => r.name));
  if (!taken.has(base)) return base;
  for (let n = 2; ; n++) {
    const candidate = `${base}${n}`;
    if (!taken.has(candidate)) return candidate;
  }
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
  const [rows, setRows] = useState<AttachmentRow[]>([]);
  const [variables, setVariables] = useState<DocumentVariable[]>([]);
  const [exposed, setExposed] = useState<ReadonlySet<string>>(new Set());
  const [picking, setPicking] = useState(false);
  const [pickHint, setPickHint] = useState<string | null>(null);
  // The multi-solid refusal is the ONE backend failure with a repair the dialog
  // can offer; everything else is just reported.
  const [offerUnion, setOfferUnion] = useState(false);
  const nameInput = useRef<HTMLInputElement>(null);
  const nextRowId = useRef(1);
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
    setRows([]);
    setExposed(new Set());
    setOfferUnion(false);
    idTouched.current = false;
    nameInput.current?.focus();
    nameInput.current?.select();
  }, [bodyId, bodyName]);

  // The document's variables are the ONLY things a saved component can expose
  // as free parameters (a re-bake sets variables). Read once per open; a
  // failure leaves the list empty, which degrades to the pre-WP-F1.3 dialog
  // rather than blocking the save.
  useEffect(() => {
    if (!bodyId) return;
    let alive = true;
    void createClient()
      .listVariables()
      .then((vars) => {
        if (alive) setVariables(vars);
      })
      .catch(() => {
        if (alive) setVariables([]);
      });
    return () => {
      alive = false;
    };
  }, [bodyId]);

  // Pick mode must not outlive the dialog — an armed picker with nowhere to
  // deliver would keep swallowing viewport clicks.
  useEffect(() => {
    if (bodyId) return;
    cancelAttachmentPick();
    setPicking(false);
    setPickHint(null);
  }, [bodyId]);
  useEffect(() => () => cancelAttachmentPick(), []);

  if (!bodyId) return null;

  const trimmedName = name.trim();
  const duplicateNames = rows.some((r, i) => rows.findIndex((o) => o.name === r.name) !== i);
  const attachmentsValid = rows.every((r) => isValidAttachmentName(r.name)) && !duplicateNames;
  const canCommit =
    !saving &&
    trimmedName.length > 0 &&
    isValidComponentId(id) &&
    isValidVersion(version) &&
    attachmentsValid;

  const stopPicking = () => {
    cancelAttachmentPick();
    setPicking(false);
    setPickHint(null);
  };

  const startPicking = () => {
    const armed = beginAttachmentPick({
      onPick: (picked) => {
        setRows((prev) => [
          ...prev,
          {
            id: nextRowId.current++,
            name: uniqueName(prev, DEFAULT_NAME[picked.kind]),
            kind: picked.kind,
            accepts: picked.accepts,
            frame: picked.frame,
          },
        ]);
        setPickHint(PICK_HINT);
      },
      onReject: (message) => setPickHint(message),
      onExit: () => {
        setPicking(false);
        setPickHint(null);
      },
    });
    if (!armed) {
      setPickHint("No viewport to pick in — open the model first.");
      return;
    }
    setPicking(true);
    setPickHint(PICK_HINT);
  };

  /**
   * The `[attachments]` table this save writes. A picked row's `on` follows its
   * NAME so the two can never drift apart, and rides with the frame the pick
   * captured. No rows ⇒ the pre-WP-F1.2 origin seat, which is still a real
   * (frameless) attachment: the component then seats at its model origin, the
   * convention every built-in generator follows.
   */
  const attachmentTable = (): Record<string, LibraryAttachment> => {
    if (rows.length === 0) return { seat: { on: "face:origin", accepts: ["plane"] } };
    const table: Record<string, LibraryAttachment> = {};
    for (const r of rows) {
      table[r.name] = { on: `${ON_PREFIX[r.kind]}:${r.name}`, accepts: r.accepts, frame: r.frame };
    }
    return table;
  };

  /**
   * The `[parameters]` table this save writes: one `role: "free"` entry per
   * checked variable, keyed BY THE VARIABLE NAME so a designation template
   * (`"{depth} deep"`) and the binding read the same word. `value` freezes the
   * variable's current number as the package default — which is what the frozen
   * document itself builds, so an instance that overrides nothing resolves to
   * exactly the baked solid.
   */
  const parameterTable = (): Record<string, NewComponentParameter> => {
    const table: Record<string, NewComponentParameter> = {};
    for (const v of variables) {
      if (exposed.has(v.name)) table[v.name] = { key: v.name, value: v.value };
    }
    return table;
  };

  const toggleExposed = (name: string) =>
    setExposed((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });

  const commit = async (unionSolids: boolean) => {
    if (!canCommit) return;
    stopPicking();
    setSaving(true);
    setError(null);
    setOfferUnion(false);
    const spec: NewComponentSpec = {
      id: id.trim(),
      version: version.trim(),
      name: trimmedName,
      category: ["mine"],
      tags: tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
      attachments: attachmentTable(),
      parameters: parameterTable(),
    };
    try {
      const saved = await createClient().saveAsComponent(
        bodyId,
        spec,
        getViewportEngine()?.captureThumbnail(256) ?? null,
        unionSolids,
      );
      onClose();
      viewportStore
        .getState()
        .setStatusHint(`Saved “${saved.name}” to the library — find it under Library ▸ mine.`);
    } catch (e) {
      // Stays OPEN on failure: the backend's refusals (a taken id@version, a
      // body that bakes to more than one solid) are all things the author fixes
      // right here, and closing would throw away everything they typed.
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      // Matched on the MARKER, never the sentence — the offer must not vanish
      // because the backend reworded its refusal. A union that itself failed
      // (disjoint bodies) reports plainly instead of offering the same thing twice.
      setOfferUnion(!unionSolids && message.includes(MULTI_SOLID_REFUSAL));
    } finally {
      setSaving(false);
    }
  };

  return createPortal(
    <div
      // While picking, the scrim must let pointer events through to the canvas
      // AND stop being a click-to-close target — the author is clicking the
      // model on purpose, and every one of those clicks would otherwise close
      // the dialog they are filling in.
      className={`fixed inset-0 z-[95] flex items-start justify-center pt-[110px] ${
        picking ? "pointer-events-none" : "bg-scrim"
      }`}
      onClick={picking ? undefined : onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Save as component"
        data-testid="save-as-component-dialog"
        data-authoring-dialog=""
        onClick={(e) => e.stopPropagation()}
        className="pointer-events-auto w-[420px] rounded-lg border border-border bg-surface p-[20px_22px] font-ui shadow-popover"
      >
        <div className="text-[14px] font-semibold text-ink">Save as component</div>
        <div className="mt-[3px] text-[12px] text-ink-5">
          Captures this body as a reusable component. With no attachment picked it seats at the
          body’s model origin with +Z out of the seating plane — the same convention the built-in
          fasteners use.
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
        {isPlaceholderName(name) && (
          <div
            className="mt-1 text-[11.5px] text-warn-strong"
            data-testid="save-as-component-name-warning"
          >
            This will be this part&rsquo;s permanent library name — give it a real one.
          </div>
        )}

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

        <div className="mt-4 flex items-center justify-between">
          <div className="text-[11.5px] text-ink-5">Attachments</div>
          <Button
            variant="secondary"
            data-testid="save-as-component-add-attachment"
            onClick={picking ? stopPicking : startPicking}
          >
            {picking ? "Stop picking" : "Add attachment"}
          </Button>
        </div>

        {pickHint && (
          <div className="mt-1 text-[11.5px] text-ink-4" data-testid="save-as-component-pick-hint">
            {pickHint}
          </div>
        )}

        {rows.length === 0 ? (
          <div className="mt-1 text-[11.5px] text-ink-5" data-testid="save-as-component-no-attachments">
            None picked — the component will seat at its model origin.
          </div>
        ) : (
          <ul className="mt-1 flex flex-col gap-1" data-testid="save-as-component-attachments">
            {rows.map((row) => (
              <li
                key={row.id}
                className="flex items-center gap-2 rounded border border-border-subtle bg-well px-2 py-1"
                data-testid="save-as-component-attachment-row"
              >
                <TextInput
                  value={row.name}
                  onChange={(e) =>
                    setRows((prev) =>
                      prev.map((r) => (r.id === row.id ? { ...r, name: e.target.value } : r)),
                    )
                  }
                  aria-label={`Attachment name for ${ON_PREFIX[row.kind]}`}
                  data-testid="save-as-component-attachment-name"
                  wrapperClassName="w-[110px]"
                  onKeyDown={(e) => e.stopPropagation()}
                />
                <span className="min-w-0 flex-1 truncate text-[11px] text-ink-5">
                  on {ON_PREFIX[row.kind]} · accepts {row.accepts.join(", ")}
                </span>
                <span className="rounded bg-chip px-1 text-[10px] text-ink-4">frame</span>
                <button
                  type="button"
                  aria-label={`Remove attachment ${row.name}`}
                  data-testid="save-as-component-attachment-remove"
                  className="text-[11px] text-ink-5 hover:text-ink"
                  onClick={() => setRows((prev) => prev.filter((r) => r.id !== row.id))}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}

        {!attachmentsValid && (
          <div
            className="mt-1 text-[11.5px] text-traffic-close"
            data-testid="save-as-component-attachment-error"
          >
            {duplicateNames
              ? "Attachment names must be unique — a mate is recorded by name."
              : "An attachment name uses lowercase letters, digits, - and _."}
          </div>
        )}

        <div className="mt-4 text-[11.5px] text-ink-5">Parameters</div>
        {variables.length === 0 ? (
          <div className="mt-1 text-[11.5px] text-ink-5" data-testid="save-as-component-no-variables">
            This document has no variables — nothing to expose. Add one in the Variables section to
            make a size editable on placed copies.
          </div>
        ) : (
          <ul className="mt-1 flex flex-col gap-1" data-testid="save-as-component-parameters">
            {variables.map((v) => (
              <li
                key={v.name}
                className="flex items-center gap-2 rounded border border-border-subtle bg-well px-2 py-1"
                data-testid="save-as-component-parameter-row"
              >
                <input
                  type="checkbox"
                  id={`sac-param-${v.name}`}
                  aria-label={`Expose ${v.name}`}
                  checked={exposed.has(v.name)}
                  onChange={() => toggleExposed(v.name)}
                />
                <label
                  htmlFor={`sac-param-${v.name}`}
                  className="min-w-0 flex-1 truncate text-[11.5px] text-ink-2"
                >
                  {v.name}
                </label>
                <span className="text-[11px] text-ink-5">{v.value}</span>
              </li>
            ))}
          </ul>
        )}

        {error && (
          <div
            role="alert"
            data-testid="save-as-component-error"
            className="mt-3 text-[11.5px] text-traffic-close"
          >
            {error}
          </div>
        )}

        {offerUnion && (
          <div
            className="mt-2 rounded border border-warn-border bg-warn-surface p-2 text-[11.5px] text-warn-strong"
            data-testid="save-as-component-union-offer"
          >
            The bake can fuse those solids into one and save that. Only the component is fused —
            this document keeps its bodies. Picking one solid, or splitting them into separate
            components, is not offered in this version; disjoint bodies cannot be fused at all.
            <div className="mt-2 flex gap-2">
              <Button
                data-testid="save-as-component-union-commit"
                onClick={() => void commit(true)}
              >
                Union into one solid and save
              </Button>
              <Button
                variant="secondary"
                data-testid="save-as-component-union-cancel"
                onClick={() => setOfferUnion(false)}
              >
                Cancel
              </Button>
            </div>
          </div>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <Button variant="secondary" data-testid="save-as-component-cancel" onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={!canCommit}
            data-testid="save-as-component-commit"
            onClick={() => void commit(false)}
          >
            {saving ? "Saving…" : "Save"}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
