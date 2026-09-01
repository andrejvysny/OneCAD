/*
 * "Import components…" (Component Library WP-C2) — the Library modal's bulk
 * vendor-stock import flow. Distinct from `SaveAsComponentDialog`: that one
 * captures a body already IN the open document; this one reads STEP files off
 * disk with no document open required (reachable from the Start screen's
 * Library too), landing zero or more catalog entries in one round trip.
 *
 * FILE SELECTION. The real (Tauri) lane calls `client.pickComponentFiles()` —
 * Rust's native multi-select STEP dialog (`pick_component_files`) — and uses
 * the returned absolute paths directly, no browser `File` involved. The MOCK
 * lane cannot open a native dialog: `pickComponentFiles` always resolves `[]`
 * there, so `handleChoose` falls back to a HIDDEN `<input type="file"
 * multiple>`, the one selection mechanism Playwright can drive with no Tauri
 * capability at all. A browser `File` carries no filesystem path, which is
 * exactly why that fallback is mock-lane only — see `client.ts`'s doc comment
 * on `pickComponentFiles`.
 *
 * `ingestComponents` never throws PER PART — a whole-call rejection (e.g. no
 * worker reachable yet) is caught here and turned into one `failed` row per
 * requested path, matching the errors-as-values convention every other store
 * action follows: never rethrow, report the outcome.
 */
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/ui/cn";
import { Button } from "@/ui/Button";
import { TextInput } from "@/ui/TextInput";
import { createClient } from "@/ipc/client";
import type { IngestComponentsReport, IngestPartResult } from "@/ipc/types";

export interface IngestComponentsDialogProps {
  open: boolean;
  onClose: () => void;
  /** Called once, after a batch lands at least one `ok` part — the caller re-lists the catalog. */
  onImported: () => void;
}

const STATUS_LABEL: Record<IngestPartResult["status"], string> = {
  ok: "OK",
  refused: "Refused",
  failed: "Failed",
};

const STATUS_TEXT: Record<IngestPartResult["status"], string> = {
  ok: "text-dof-ok",
  refused: "text-warn-strong",
  failed: "text-traffic-close",
};

/** `"/vendor/rp4040.step"` → `"rp4040.step"`, for a compact row label. */
function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** True inside a Tauri webview — same inline check `documentStore`/`client.ts`
 *  use, since `pickComponentFiles` alone cannot tell a real cancel (tauri
 *  lane, `[]`) apart from "there is no dialog to open" (mock lane, `[]`). */
function isTauriLane(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** A chosen file: `path` is set from `pickComponentFiles` (real lane) and
 *  carries the absolute path `ingestComponents` needs; a mock-lane hidden-
 *  input pick has no filesystem path, so `name` alone stands in for it (the
 *  mock refuses by name anyway). */
interface ChosenFile {
  name: string;
  path?: string;
}

export function IngestComponentsDialog({ open, onClose, onImported }: IngestComponentsDialogProps) {
  const [files, setFiles] = useState<ChosenFile[]>([]);
  const [vendor, setVendor] = useState("vendor");
  const [category, setCategory] = useState("imported");
  const [tags, setTags] = useState("");
  const [importing, setImporting] = useState(false);
  const [report, setReport] = useState<IngestComponentsReport | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  // Fresh state on every open — a re-open must not carry over a previous
  // batch's result or file selection.
  useEffect(() => {
    if (!open) return;
    setFiles([]);
    setVendor("vendor");
    setCategory("imported");
    setTags("");
    setReport(null);
    if (fileInput.current) fileInput.current.value = "";
  }, [open]);

  if (!open) return null;

  const canImport = !importing && files.length > 0;

  /** "Choose files…": try the real native multi-select dialog first, and only
   *  fall back to the hidden input when there is no dialog to fall back FROM
   *  (the mock lane — see the header note). A real-lane empty result is an
   *  ordinary cancel, not a fallback trigger. */
  const handleChoose = async () => {
    const paths = await createClient().pickComponentFiles();
    if (paths.length > 0) {
      setFiles(paths.map((p) => ({ name: basename(p), path: p })));
      return;
    }
    if (!isTauriLane()) fileInput.current?.click();
  };

  const runImport = async () => {
    if (!canImport) return;
    setImporting(true);
    setReport(null);
    // Real-lane picks carry an absolute `path`; a mock-lane hidden-input pick
    // has none, so `.name` is what the mock refuses by (see the header note).
    const paths = files.map((f) => f.path ?? f.name);
    const categoryList = category
      .split(",")
      .map((c) => c.trim())
      .filter(Boolean);
    const tagList = tags
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean);
    try {
      const result = await createClient().ingestComponents({
        paths,
        defaults: {
          vendor: vendor.trim() || "vendor",
          category: categoryList.length > 0 ? categoryList : ["imported"],
          tags: tagList.length > 0 ? tagList : undefined,
        },
      });
      setReport(result);
      if (result.parts.some((p) => p.status === "ok")) onImported();
    } catch (e) {
      // Errors-as-values: a rejected round trip becomes a failed row per
      // requested path, never a thrown error left for the caller to catch.
      const message = e instanceof Error ? e.message : String(e);
      setReport({
        libraryRoot: "",
        parts: paths.map((path) => ({ path, status: "failed", message })),
      });
    } finally {
      setImporting(false);
    }
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[120] flex items-start justify-center bg-scrim pt-[110px]"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Import components"
        data-testid="library-ingest-dialog"
        onClick={(e) => e.stopPropagation()}
        className="w-[440px] rounded-lg border border-border bg-surface p-[20px_22px] font-ui shadow-popover"
      >
        <div className="text-[14px] font-semibold text-ink">Import components…</div>
        <div className="mt-[3px] text-[12px] text-ink-5">
          Bulk-imports STEP files as catalog components — one row per file, so a bad file among
          good ones never blocks the rest.
        </div>

        <input
          ref={fileInput}
          type="file"
          multiple
          accept=".step,.stp"
          data-testid="library-ingest-file-input"
          className="hidden"
          onChange={(e) =>
            setFiles(Array.from(e.target.files ?? []).map((f) => ({ name: f.name })))
          }
        />
        <div className="mt-4 flex items-center justify-between">
          <div className="text-[11.5px] text-ink-5">
            {files.length === 0
              ? "No files chosen"
              : `${files.length} file${files.length === 1 ? "" : "s"} chosen`}
          </div>
          <Button
            variant="secondary"
            data-testid="library-ingest-choose"
            onClick={() => void handleChoose()}
          >
            Choose files…
          </Button>
        </div>
        {files.length > 0 && (
          <ul className="mt-1.5 flex max-h-[90px] flex-col gap-0.5 overflow-auto">
            {files.map((f, i) => (
              <li key={`${f.name}-${i}`} className="truncate text-[11px] text-ink-6">
                {f.name}
              </li>
            ))}
          </ul>
        )}

        <div className="mt-4 flex gap-2">
          <div className="w-[120px]">
            <label className="block text-[11.5px] text-ink-5" htmlFor="ic-vendor">
              Vendor
            </label>
            <TextInput
              id="ic-vendor"
              value={vendor}
              onChange={(e) => setVendor(e.target.value)}
              aria-label="Vendor"
              data-testid="library-ingest-vendor"
              wrapperClassName="mt-1 w-full"
            />
          </div>
          <div className="min-w-0 flex-1">
            <label className="block text-[11.5px] text-ink-5" htmlFor="ic-category">
              Category
            </label>
            <TextInput
              id="ic-category"
              value={category}
              onChange={(e) => setCategory(e.target.value)}
              placeholder="comma, separated"
              aria-label="Category"
              data-testid="library-ingest-category"
              wrapperClassName="mt-1 w-full"
            />
          </div>
        </div>
        <label className="mt-3 block text-[11.5px] text-ink-5" htmlFor="ic-tags">
          Tags
        </label>
        <TextInput
          id="ic-tags"
          value={tags}
          onChange={(e) => setTags(e.target.value)}
          placeholder="comma, separated (optional)"
          aria-label="Tags"
          data-testid="library-ingest-tags"
          wrapperClassName="mt-1 w-full"
        />

        {report && (
          <ul className="mt-4 flex flex-col gap-1" data-testid="library-ingest-result">
            {report.parts.map((part, i) => (
              <li
                key={`${part.path}-${i}`}
                data-testid="library-ingest-result-row"
                data-status={part.status}
                className="flex items-center gap-2 rounded border border-border-subtle bg-well px-2 py-1"
              >
                <span
                  data-testid="library-ingest-result-status"
                  className={cn(
                    "shrink-0 rounded-[4px] border border-border-subtle px-1.5 py-[1.5px] text-[9px] font-[750] uppercase leading-none tracking-[0.07em]",
                    STATUS_TEXT[part.status],
                  )}
                >
                  {STATUS_LABEL[part.status]}
                </span>
                <span className="min-w-0 flex-1 truncate text-[11.5px] text-ink-2">
                  {basename(part.path)}
                </span>
                {part.message && (
                  <span className="min-w-0 max-w-[220px] truncate text-[11px] text-ink-5">
                    {part.message}
                  </span>
                )}
              </li>
            ))}
            {report.parts.length === 0 && (
              <li className="text-[11.5px] text-ink-5">Nothing to import.</li>
            )}
          </ul>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <Button variant="secondary" data-testid="library-ingest-cancel" onClick={onClose}>
            {report ? "Close" : "Cancel"}
          </Button>
          <Button
            disabled={!canImport}
            data-testid="library-ingest-submit"
            onClick={() => void runImport()}
          >
            {importing ? "Importing…" : "Import"}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
