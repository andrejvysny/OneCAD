/*
 * "Variables" inspector section (WP-VE.2) — the authoring surface for the
 * document variable table WP-VE.1 made drive regen.
 *
 * DOCUMENT-LEVEL, not selection-driven: the table belongs to the document, so
 * the section renders in Model mode whatever is (or is not) selected. That is
 * why it sits LAST in the modeling module's section order — a document-wide
 * table must not push the sections that are about the current selection down.
 *
 * The backend owns every rule (name grammar, duplicate = re-value, unknown
 * remove refused, undoability). This component adds no validation of its own
 * beyond the one thing it can decide locally — an obviously malformed name,
 * caught before a round trip so the message lands next to the field. Every
 * refusal the backend makes is surfaced verbatim: a second, drifting copy of the
 * rule here is how a UI starts accepting names the document rejects.
 */
import { useCallback, useEffect, useState } from "react";
import { SectionLabel } from "@/ui/SectionLabel";
import { Icon } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import { createClient } from "@/ipc/client";
import { classifyRegen, failureReason } from "@/ipc/regenOutcome";
import { viewportStore } from "@/stores/viewportStore";
import { VARIABLE_NAME_RE, type DocumentVariable, type VariableEditResult } from "@/ipc/types";

const NAME_HINT = "A name must start with a letter or _ and contain only letters, digits and _.";

/** The row a new variable is authored in. Blank name + blank value = idle. */
interface DraftRow {
  name: string;
  value: string;
}

const EMPTY_DRAFT: DraftRow = { name: "", value: "" };

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Adopt a variable write's TWO truths (W5).
 *
 * The saved table is returned for the list — a variable edit is never rolled
 * back, so the row keeps the value the document really holds — while a failed
 * downstream regen is raised on the SAME sticky status-bar hint every other
 * commit family reports through (`featureValueEdit` / `treeActions` /
 * `ComponentParametersSection`). Deliberately not the section's own
 * `variables-error` alert: that one means "the write was REFUSED", and this
 * means "the write landed and the rebuild after it failed". Collapsing the two
 * would lose exactly the distinction the user needs.
 *
 * `needsRepair` / `noop` keep the record (`keepsRecord`) and are successes here.
 */
function adopt(res: VariableEditResult): DocumentVariable[] {
  const reason = failureReason(classifyRegen(res));
  if (reason !== null) {
    viewportStore
      .getState()
      .setStatusHint(`Variable saved, but the rebuild failed: ${reason}`, {
        severity: "error",
        sticky: true,
      });
  }
  return res.variables;
}

export function VariablesSection() {
  const [vars, setVars] = useState<DocumentVariable[] | null>(null);
  const [draft, setDraft] = useState<DraftRow>(EMPTY_DRAFT);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  const load = useCallback(async (alive: () => boolean = () => true) => {
    try {
      const table = await createClient().listVariables();
      if (alive()) setVars(table);
    } catch {
      // No document open (or a backend that cannot serve the table): the section
      // has nothing to author against and renders absent, not an error.
      if (alive()) setVars(null);
    }
  }, []);

  useEffect(() => {
    let alive = true;
    void load(() => alive);
    // A variable edit dirties the timeline, so the same `document-changed` every
    // other regen fires is what tells this section to re-read — including after
    // an UNDO, which no local state of ours would ever hear about.
    const off = createClient().onDocumentChanged(() => {
      void load(() => alive);
    });
    return () => {
      alive = false;
      off();
    };
  }, [load]);

  const commit = useCallback(
    async (name: string, value: number) => {
      setPending(true);
      try {
        setVars(adopt(await createClient().upsertVariable(name, value)));
        setError(null);
        return true;
      } catch (e) {
        setError(errorText(e));
        return false;
      } finally {
        setPending(false);
      }
    },
    [],
  );

  const remove = useCallback(async (name: string) => {
    setPending(true);
    try {
      setVars(adopt(await createClient().removeVariable(name)));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPending(false);
    }
  }, []);

  const addDraft = useCallback(async () => {
    const name = draft.name.trim();
    const value = Number.parseFloat(draft.value);
    if (!VARIABLE_NAME_RE.test(name)) {
      setError(`Invalid variable name “${name}”. ${NAME_HINT}`);
      return;
    }
    if (!Number.isFinite(value)) {
      setError("A variable needs a number.");
      return;
    }
    if (await commit(name, value)) setDraft(EMPTY_DRAFT);
  }, [commit, draft]);

  if (vars === null) return null;

  return (
    <>
      <SectionLabel className="pb-1.5 pt-4">Variables</SectionLabel>

      {vars.length === 0 && (
        <div data-testid="variables-empty" className="mb-1 text-[12px] text-ink-5">
          No variables yet — add one below, then type <span className="font-mono">=name</span> into a
          feature’s value.
        </div>
      )}

      {vars.map((v) => (
        <div
          key={v.id}
          data-testid={`variable-row-${v.name}`}
          className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5"
        >
          <span className="flex-1 truncate font-mono text-[12.5px] text-ink-2" title={v.name}>
            {v.name}
          </span>
          {/* An expression-driven variable is READ-ONLY here: V1 refuses to
              resolve a chained expression at regen, so offering a number field
              would invite an edit that silently does nothing. */}
          {v.expr !== undefined ? (
            <span className="font-mono text-[12px] text-warn" title="Chained expressions are not supported">
              ={v.expr}
            </span>
          ) : (
            <input
              /*
               * KEYED ON THE VALUE. The field is uncontrolled (so typing is not
               * fought by a re-render mid-edit), which means a new `defaultValue`
               * alone would NOT reach the DOM — the row would keep showing the
               * number the user last typed even after the document moved
               * underneath it (a re-value from the draft row, or an undo). The
               * key remounts it, which is the only way an uncontrolled input
               * adopts a new backend value.
               */
              key={`${v.id}:${v.value}`}
              aria-label={`${v.name} value`}
              data-testid={`variable-value-${v.name}`}
              type="number"
              className="w-20 rounded-sm border border-border-strong bg-surface px-1 text-right font-mono text-[12px] text-ink-2 outline-none focus:border-accent"
              defaultValue={v.value}
              disabled={pending}
              onBlur={(e) => {
                const n = e.target.valueAsNumber;
                if (!Number.isFinite(n) || n === v.value) return;
                void commit(v.name, n);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
              }}
            />
          )}
          <button
            type="button"
            aria-label={`Delete ${v.name}`}
            data-testid={`variable-delete-${v.name}`}
            disabled={pending}
            className={cn(
              "cursor-pointer rounded-sm p-0.5 text-ink-5 hover:bg-hover-3 hover:text-ink-2",
              "focus-visible:shadow-focus-ring focus-visible:outline-none disabled:opacity-50",
            )}
            onClick={() => void remove(v.name)}
          >
            <Icon name="x" size={12} strokeWidth={2.2} />
          </button>
        </div>
      ))}

      <div className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5">
        <input
          aria-label="New variable name"
          data-testid="variable-new-name"
          placeholder="name"
          className="min-w-0 flex-1 rounded-sm border border-border-strong bg-surface px-1 font-mono text-[12px] text-ink-2 outline-none placeholder:text-ink-6 focus:border-accent"
          value={draft.name}
          disabled={pending}
          onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
          onKeyDown={(e) => {
            if (e.key === "Enter") void addDraft();
          }}
        />
        <input
          aria-label="New variable value"
          data-testid="variable-new-value"
          type="number"
          placeholder="0"
          className="w-20 rounded-sm border border-border-strong bg-surface px-1 text-right font-mono text-[12px] text-ink-2 outline-none placeholder:text-ink-6 focus:border-accent"
          value={draft.value}
          disabled={pending}
          onChange={(e) => setDraft((d) => ({ ...d, value: e.target.value }))}
          onKeyDown={(e) => {
            if (e.key === "Enter") void addDraft();
          }}
        />
      </div>

      {error !== null && (
        <div data-testid="variables-error" role="alert" className="mb-1 text-[12px] text-traffic-close">
          {error}
        </div>
      )}
    </>
  );
}
