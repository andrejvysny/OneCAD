/*
 * "Variables" inspector section (WP-VE.2) — the authoring surface for the
 * document variable table WP-VE.1 made drive regen.
 *
 * DOCUMENT-LEVEL, not selection-driven: the table belongs to the document, so
 * the section renders in Model mode whatever is (or is not) selected. That is
 * why it sits LAST in the modeling module's section order — a document-wide
 * table must not push the sections that are about the current selection down.
 *
 * The backend owns every rule (name grammar, expression validity, duplicate =
 * re-value, unknown remove refused, undoability). This component adds no
 * validation of its own beyond the one thing it can decide locally — an
 * obviously malformed NAME, caught before a round trip so the message lands next
 * to the field. Every refusal the backend makes is surfaced verbatim: a second,
 * drifting copy of the rule here is how a UI starts accepting what the document
 * rejects. In particular nothing here judges an EXPRESSION; `readVariableInput`
 * only decides which of the two write commands a typed cell goes to.
 */
import { useCallback, useEffect, useState } from "react";
import { SectionLabel } from "@/ui/SectionLabel";
import { cn } from "@/ui/cn";
import { createClient } from "@/ipc/client";
import { classifyRegen, failureReason, keepsRecord } from "@/ipc/regenOutcome";
import { applyEditResult } from "./historyActions";
import { viewportStore } from "@/stores/viewportStore";
import { VARIABLE_NAME_RE, type DocumentVariable, type VariableEditResult } from "@/ipc/types";
import { VariableRow } from "./VariableRow";
import { readVariableInput } from "./variableAuthoring";

const NAME_HINT = "A name must start with a letter or _ and contain only letters, digits and _.";

/** The row a new variable is authored in. Blank name + blank value = idle. */
interface DraftRow {
  name: string;
  value: string;
}

const EMPTY_DRAFT: DraftRow = { name: "", value: "" };

const DRAFT_FIELD = cn(
  "rounded-sm border border-border-strong bg-surface px-1 font-mono text-[12px] text-ink-2",
  "outline-none placeholder:text-ink-6 focus:border-accent",
);

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
 *
 * On the success path the TIMELINE is hydrated too, through the same
 * `applyEditResult` every other commit family uses. A variable write can move
 * feature rows — a RENAME rewrites every binding, so a bound row's `=name` is
 * stale the moment it lands — and the row is rendered from
 * `documentStore.features`, not from anything this section holds. Verdict
 * first, hydrate second: a FAILED rebuild is not hydrated, because the store
 * then still holds what the backend actually kept.
 */
function adopt(res: VariableEditResult): DocumentVariable[] {
  const outcome = classifyRegen(res);
  if (keepsRecord(outcome)) applyEditResult(res);
  const reason = failureReason(outcome);
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

  /**
   * Run one write. Errors-as-values: a REFUSAL becomes the section's inline
   * alert and `false`, never a rethrow — the caller decides what to do with a
   * refused edit (the draft row keeps its text so it can be corrected).
   */
  const write = useCallback(async (run: () => Promise<VariableEditResult>) => {
    setPending(true);
    try {
      setVars(adopt(await run()));
      setError(null);
      return true;
    } catch (e) {
      setError(errorText(e));
      return false;
    } finally {
      setPending(false);
    }
  }, []);

  /**
   * Commit a typed value cell. ONE field authors all three forms; which of the
   * two write commands it lowers to is `readVariableInput`'s only decision, and
   * whether the expression itself is any good is the backend's.
   */
  const commitValue = useCallback(
    (name: string, raw: string) => {
      const authored = readVariableInput(raw);
      if (authored.kind === "empty") {
        setError("A variable needs a number or an expression.");
        return Promise.resolve(false);
      }
      const client = createClient();
      return write(() =>
        authored.kind === "literal"
          ? client.upsertVariable(name, authored.value)
          : client.upsertVariableExpr(name, authored.text),
      );
    },
    [write],
  );

  const rename = useCallback(
    (name: string, newName: string) => {
      if (!VARIABLE_NAME_RE.test(newName)) {
        setError(`Invalid variable name “${newName}”. ${NAME_HINT}`);
        return;
      }
      void write(() => createClient().renameVariable(name, newName));
    },
    [write],
  );

  const remove = useCallback(
    (name: string) => {
      void write(() => createClient().removeVariable(name));
    },
    [write],
  );

  const addDraft = useCallback(async () => {
    const name = draft.name.trim();
    if (!VARIABLE_NAME_RE.test(name)) {
      setError(`Invalid variable name “${name}”. ${NAME_HINT}`);
      return;
    }
    if (await commitValue(name, draft.value)) setDraft(EMPTY_DRAFT);
  }, [commitValue, draft]);

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
        <VariableRow
          key={v.id}
          variable={v}
          pending={pending}
          onCommitValue={(name, raw) => void commitValue(name, raw)}
          onRename={rename}
          onDelete={remove}
        />
      ))}

      <div className="mb-1 flex h-[30px] items-center gap-2 rounded-sm bg-chip px-2.5">
        <input
          aria-label="New variable name"
          data-testid="variable-new-name"
          placeholder="name"
          className={cn(DRAFT_FIELD, "min-w-0 flex-1")}
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
          /* TEXT: a new variable may be authored as `25`, `45deg` or `=w*2`,
             and a numeric input would swallow the last two. */
          placeholder="0"
          className={cn(DRAFT_FIELD, "w-20 text-right")}
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
