/*
 * "Which model is answering?" — the assistant panel's one piece of status.
 *
 * Reads the assistant host's registered providers through the bridge. It is the
 * cheapest honest probe available: a provider list that comes back means the
 * sidecar is up and the transport works, and an enabled provider's label plus
 * its default model is exactly what a user needs to know before typing.
 *
 * It is ALSO where a refused provider configuration is reported. Rust validates
 * the endpoint the user typed (ADR-0017: canonical literal loopback only) and
 * refuses anything else; this strip is the one place they learn that, so a
 * refusal outranks whatever the probe found. Left unsaid, a rejected endpoint
 * would resurface as "unknown provider" on every answer instead.
 *
 * Errors are values: a failed probe becomes an "unavailable" state on screen,
 * logged once. Nothing rethrows, because there is no caller above a panel.
 */
import { useEffect, useState } from "react";
import { logError } from "@/debug/log";
import { cn } from "@/ui/cn";
import { getAssistantClient } from "../client/assistantClient";
import type { AssistantProviderConfigState } from "./useAssistantProviderConfig";

type IndicatorStatus = "loading" | "ready" | "offline" | "error";

interface IndicatorState {
  readonly status: IndicatorStatus;
  readonly label: string;
}

const DOT: Record<IndicatorStatus, string> = {
  loading: "bg-ink-6",
  ready: "bg-dof-ok",
  offline: "bg-danger-strong",
  error: "bg-danger-strong",
};

export interface LocalModelIndicatorProps {
  /** The live result of syncing settings into the Rust-side registry. */
  readonly config: AssistantProviderConfigState;
}

export function LocalModelIndicator({ config }: LocalModelIndicatorProps) {
  const [state, setState] = useState<IndicatorState>({
    status: "loading",
    label: "Connecting…",
  });

  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const providers = await getAssistantClient().listProviders();
        if (!live) return;
        const active = providers.find((p) => p.enabled);
        setState(
          active === undefined
            ? { status: "offline", label: "No model configured" }
            : { status: "ready", label: `${active.label} · ${active.defaultModel}` },
        );
      } catch (error) {
        if (!live) return;
        logError("assistant", "provider probe failed", { message: String(error) });
        setState({ status: "offline", label: "Assistant host unavailable" });
      }
    })();
    return () => {
      live = false;
    };
  }, []);

  // A refused configuration outranks the probe: it names a fault the user can
  // actually fix, and it is the reason the model would not answer anyway.
  const view: IndicatorState =
    config.status === "error"
      ? { status: "error", label: `Model endpoint refused: ${config.error ?? "unknown reason"}` }
      : state;

  return (
    <div
      className={cn(
        "flex flex-none items-center gap-1.5 border-b border-border px-3 py-1.5 text-[11px]",
        view.status === "error" ? "bg-danger-surface text-danger-strong" : "text-ink-4",
      )}
      data-testid="assistant-model-indicator"
      data-status={view.status}
      {...(view.status === "error" ? { role: "alert" as const } : {})}
    >
      <span className={cn("h-1.5 w-1.5 flex-none rounded-full", DOT[view.status])} aria-hidden="true" />
      <span
        className={view.status === "error" ? "min-w-0 break-words" : "truncate"}
        {...(view.status === "error" ? { "data-testid": "assistant-provider-error" } : {})}
      >
        {view.label}
      </span>
    </div>
  );
}
