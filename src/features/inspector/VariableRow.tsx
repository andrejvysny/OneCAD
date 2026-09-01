/*
 * One row of the Variables section: the name (renameable in place), the value
 * (a text cell that takes `25`, `45deg` or `=w*2`), what it resolves to, and
 * why it does not when it does not.
 *
 * Split out of `VariablesSection` because a row now carries three independent
 * edit affordances plus a diagnostic, and the section is about the LIST.
 *
 * Both fields are uncontrolled and KEYED on the backend value they show: a
 * controlled field would be fought by a re-render mid-edit, and a new
 * `defaultValue` alone would never reach the DOM — the row would keep showing
 * what the user last typed even after the document moved underneath it (an
 * undo, or a rename that rewrote this row's expression). The key remounts it,
 * which is the only way an uncontrolled input adopts a new backend value.
 */
import { Icon } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import type { DocumentVariable } from "@/ipc/types";
import { useSettingsStore } from "@/stores/settingsStore";
import { formatResolvedVariable, variableInputText } from "./variableAuthoring";

export interface VariableRowProps {
  variable: DocumentVariable;
  /** No edit may start while another is in flight. */
  pending: boolean;
  /** The typed value cell, verbatim — the section decides literal vs expression. */
  onCommitValue(name: string, raw: string): void;
  onRename(name: string, newName: string): void;
  onDelete(name: string): void;
}

const FIELD = cn(
  "min-w-0 rounded-sm border border-border-strong bg-surface px-1 font-mono text-[12px]",
  "text-ink-2 outline-none focus:border-accent disabled:opacity-50",
);

export function VariableRow({
  variable: v,
  pending,
  onCommitValue,
  onRename,
  onDelete,
}: VariableRowProps) {
  // Every length row re-reads on a unit switch, exactly as the dimension chips
  // do — the resolved read-out is a measurement, not document text.
  const unit = useSettingsStore((s) => s.displayUnit);
  const authored = variableInputText(v);

  return (
    <div
      data-testid={`variable-row-${v.name}`}
      className="mb-1 flex flex-col gap-0.5 rounded-sm bg-chip px-2.5 py-1"
    >
      <div className="flex items-center gap-2">
        <input
          key={`${v.id}:${v.name}`}
          aria-label={`${v.name} name`}
          data-testid={`variable-name-${v.name}`}
          className={cn(FIELD, "w-0 flex-1 truncate")}
          defaultValue={v.name}
          disabled={pending}
          title={v.name}
          onBlur={(e) => {
            const next = e.target.value.trim();
            if (next === "" || next === v.name) {
              // Nothing to do — and an EMPTY field is an abandoned edit, not a
              // request to name the variable "".
              e.target.value = v.name;
              return;
            }
            onRename(v.name, next);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") {
              e.currentTarget.value = v.name;
              e.currentTarget.blur();
            }
          }}
        />
        <input
          key={`${v.id}:${authored}`}
          aria-label={`${v.name} value`}
          data-testid={`variable-value-${v.name}`}
          /* TEXT, not `number`: `=w*2` and `45deg` are values here, and a
             numeric input would silently swallow both. */
          className={cn(FIELD, "w-20 text-right")}
          defaultValue={authored}
          disabled={pending}
          onBlur={(e) => {
            if (e.target.value.trim() === authored) return;
            onCommitValue(v.name, e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") {
              e.currentTarget.value = authored;
              e.currentTarget.blur();
            }
          }}
        />
        <button
          type="button"
          aria-label={`Delete ${v.name}`}
          data-testid={`variable-delete-${v.name}`}
          disabled={pending}
          className={cn(
            "cursor-pointer rounded-sm p-0.5 text-ink-5 hover:bg-hover-3 hover:text-ink-2",
            "focus-visible:shadow-focus-ring focus-visible:outline-none disabled:opacity-50",
          )}
          onClick={() => onDelete(v.name)}
        >
          <Icon name="x" size={12} strokeWidth={2.2} />
        </button>
      </div>

      {/* An expression-driven row shows what it currently RESOLVES to: the
          field above holds the authored text, so without this the number the
          document is actually building with would be nowhere on screen. */}
      {v.expr !== undefined && v.error === undefined && (
        <div
          data-testid={`variable-resolved-${v.name}`}
          className="pl-1 text-right font-mono text-[11px] text-ink-5"
        >
          = {formatResolvedVariable(v, unit)}
        </div>
      )}
      {/* Broken: the reason, where it was authored. `resolvedValue` is the last
          number anybody could justify and is deliberately NOT shown next to it. */}
      {v.error !== undefined && (
        <div
          data-testid={`variable-error-${v.name}`}
          role="alert"
          className="text-[11px] text-traffic-close"
        >
          {v.error}
        </div>
      )}
    </div>
  );
}
