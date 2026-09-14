/*
 * Left-sidebar "Assistant" tab.
 *
 * Shares its `Slots.ShellLeft` footprint with `ModelTreePanel` and
 * `VariablesPanel` through `sidebarTabStore` — one region, several tab-like
 * occupants, each rendering `null` when it is not the active tab (see
 * `SidebarTabHeader`'s doc comment). It is additionally gated on
 * `settingsStore.assistantEnabled`, which is OFF by default: nothing about the
 * assistant reaches a user who has not turned it on.
 *
 * It is also where the user's provider settings are pushed to Rust: the panel is
 * mounted even while the gate is off (it renders `null`), so the sync hook it
 * calls sees the turn-it-off transition and clears the registry.
 *
 * Transcript state is LOCAL, deliberately. The conversation's home is the
 * assistant host's own durable store, reached through the AgentKit client; a
 * zustand store here would be a second, stale copy of it. What this component
 * keeps is the optimistic echo of what the user just typed plus the request
 * status — errors-as-values, logged and shown, never rethrown.
 */
import { useCallback, useRef, useState } from "react";
import { logError } from "@/debug/log";
import { SidebarTabHeader } from "@/features/shell/SidebarTabHeader";
import { useSettingsStore } from "@/stores/settingsStore";
import { useSidebarTabStore } from "@/stores/sidebarTabStore";
import { getAssistantClient } from "../client/assistantClient";
import { AssistantComposer } from "./AssistantComposer";
import { LocalModelIndicator } from "./LocalModelIndicator";
import { LocalModelSettings } from "./LocalModelSettings";
import { useAssistantProviderConfig } from "./useAssistantProviderConfig";

type SubmitStatus = "idle" | "sending" | "error";

interface LocalTurn {
  readonly key: number;
  readonly text: string;
}

export function AssistantPanel() {
  // Read unconditionally (rules of hooks) — only the returned JSX is gated,
  // the same shape `ModelTreePanel` and `VariablesPanel` use.
  const activeSidebarTab = useSidebarTabStore((s) => s.activeTab);
  const enabled = useSettingsStore((s) => s.assistantEnabled);
  // Unconditional, and deliberately ABOVE the gate: this component is mounted
  // whether or not the assistant is enabled, so it is the one place that can tell
  // the Rust-side registry to CLEAR when the user turns the assistant off
  // (ADR-0017 — configuration changes only through a trusted command).
  const providerConfig = useAssistantProviderConfig();
  const [turns, setTurns] = useState<readonly LocalTurn[]>([]);
  const [status, setStatus] = useState<SubmitStatus>("idle");
  const [error, setError] = useState<string | null>(null);
  const chatIdRef = useRef<string | null>(null);
  const nextKeyRef = useRef(0);

  const submit = useCallback((text: string) => {
    nextKeyRef.current += 1;
    setTurns((prev) => [...prev, { key: nextKeyRef.current, text }]);
    setStatus("sending");
    setError(null);
    void (async () => {
      try {
        const client = getAssistantClient();
        // One chat per panel lifetime, created on the first turn: a chat with
        // no message in it is a row the host would have to garbage-collect.
        chatIdRef.current ??= (await client.createChat()).id;
        await client.submitMessage({ chatId: chatIdRef.current }, { content: text });
        setStatus("idle");
      } catch (caught) {
        const message = caught instanceof Error ? caught.message : String(caught);
        logError("assistant", "submit failed", { message });
        setError(message);
        setStatus("error");
      }
    })();
  }, []);

  if (!enabled || activeSidebarTab !== "assistant") return null;

  return (
    <div
      className="absolute bottom-[34px] left-0 top-0 z-20 flex w-[220px] flex-col border-r border-border bg-panel"
      data-testid="assistant-panel"
    >
      <SidebarTabHeader />
      <LocalModelIndicator config={providerConfig} />
      <LocalModelSettings config={providerConfig} />
      <div className="min-h-0 flex-1 overflow-auto p-2">
        {turns.length === 0 ? (
          <p className="px-1 py-2 text-[11.5px] leading-relaxed text-ink-5">
            Ask the assistant about the open document. It can read the model and propose
            edits; every proposal is reviewed before it is applied.
          </p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {turns.map((turn) => (
              <li
                key={turn.key}
                className="rounded-sm bg-chip px-2 py-1.5 text-[12px] text-ink-2"
                data-testid="assistant-turn"
              >
                {turn.text}
              </li>
            ))}
          </ul>
        )}
        {status === "error" && error !== null && (
          <p
            className="mt-1.5 rounded-sm bg-danger-surface px-2 py-1.5 text-[11.5px] text-danger-strong"
            role="alert"
            data-testid="assistant-error"
          >
            {error}
          </p>
        )}
      </div>
      <AssistantComposer onSubmit={submit} busy={status === "sending"} />
    </div>
  );
}
