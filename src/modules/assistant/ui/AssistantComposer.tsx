/*
 * The assistant's input row.
 *
 * Presentational and controlled from above: it owns the draft text and nothing
 * else, so the panel keeps the one place that knows how a turn is submitted.
 * Enter sends, Shift+Enter breaks a line — the convention every chat surface
 * uses, and the reason this is a textarea rather than an input.
 */
import { useState, type ChangeEvent, type KeyboardEvent } from "react";
import { Button } from "@/ui/Button";

export interface AssistantComposerProps {
  /** Called with a non-empty, trimmed draft. The panel decides what that means. */
  onSubmit(text: string): void;
  /** A turn is in flight: the control stays readable but refuses a second send. */
  busy: boolean;
}

export function AssistantComposer({ onSubmit, busy }: AssistantComposerProps) {
  const [draft, setDraft] = useState("");
  const text = draft.trim();
  const canSend = text.length > 0 && !busy;

  const send = (): void => {
    if (!canSend) return;
    setDraft("");
    onSubmit(text);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (event.key !== "Enter" || event.shiftKey) return;
    event.preventDefault();
    send();
  };

  return (
    <div className="flex flex-none flex-col gap-1.5 border-t border-border p-2">
      <textarea
        value={draft}
        onChange={(event: ChangeEvent<HTMLTextAreaElement>) => setDraft(event.target.value)}
        onKeyDown={onKeyDown}
        rows={3}
        placeholder="Ask about this model…"
        aria-label="Message the assistant"
        data-testid="assistant-composer-input"
        className="resize-none rounded-sm border border-border bg-surface px-2 py-1.5 text-[12px] text-ink-2 placeholder:text-ink-6 focus-visible:shadow-focus-ring focus-visible:outline-none"
      />
      <Button
        size="sm"
        disabled={!canSend}
        onClick={send}
        data-testid="assistant-composer-send"
        className="self-end"
      >
        {busy ? "Sending…" : "Send"}
      </Button>
    </div>
  );
}
