/*
 * Keeps the Rust-side provider registry in step with the user's settings.
 *
 * WHERE THIS LIVES AND WHY: the hook is called from `AssistantPanel`, which the
 * module mounts whether or not the feature is enabled (it renders `null` when it
 * is not the active tab or the gate is off — see `../ui.ts`). So the sync runs on
 * panel mount, on every edit to the endpoint or model, AND on the transition that
 * turns the assistant off, which is the one that has to CLEAR the registry. A
 * hook inside a component that only mounts while enabled would miss exactly that.
 *
 * Errors are values: a refused endpoint becomes an `error` status with the reason
 * Rust gave, logged once, and `LocalModelIndicator` puts it on screen. Nothing
 * rethrows — there is no caller above a panel.
 */
import { useEffect, useState } from "react";
import { logError } from "@/debug/log";
import { errorMessage } from "@/ipc/apiError";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  configureAssistantProvider,
  providerFromSettings,
  type AssistantProviderReadiness,
} from "../client/providerConfig";

/**
 * How far the configuration got, as one word.
 *
 * The four middle values are a LADDER and each one is a separate claim:
 * **Rust accepting a URL is not a working model**, which is exactly what the
 * single `ready` this replaced used to say.
 *
 * `none` — nothing configured, and the registry has been told so.
 * `configuring` — the command is in flight.
 * `configured` — Rust validated the endpoint and installed the gateway. Nothing
 *   has run a turn against it yet, and the sidecar may not even know about it.
 * `synchronized` — the running sidecar acknowledged the configuration. This is
 *   the first state in which a message can actually be answered.
 * `reachable` — something answered at the endpoint.
 * `eligible` — and it serves the configured model. The only "green" state.
 * `error` — Rust refused it; `error` carries the reason.
 */
export type AssistantProviderStatus =
  | "none"
  | "configuring"
  | "configured"
  | "synchronized"
  | "reachable"
  | "eligible"
  | "error";

export interface AssistantProviderConfigState {
  readonly status: AssistantProviderStatus;
  /** The refusal reason, or `null`. Only ever set with `status: "error"`. */
  readonly error: string | null;
  /**
   * The four states as Rust reported them, with the reason the ladder stopped.
   * `null` before the first answer, and for a cleared configuration.
   */
  readonly readiness: AssistantProviderReadiness | null;
}

/** The highest rung the readiness reached. */
export function statusOf(readiness: AssistantProviderReadiness): AssistantProviderStatus {
  if (!readiness.configured) return "none";
  if (!readiness.synchronized) return "configured";
  if (!readiness.reachable) return "synchronized";
  if (!readiness.eligible) return "reachable";
  return "eligible";
}

export function useAssistantProviderConfig(): AssistantProviderConfigState {
  const enabled = useSettingsStore((s) => s.assistantEnabled);
  const baseUrl = useSettingsStore((s) => s.assistantProviderBaseUrl);
  const model = useSettingsStore((s) => s.assistantProviderModel);
  const [state, setState] = useState<AssistantProviderConfigState>({
    status: "configuring",
    error: null,
    readiness: null,
  });

  useEffect(() => {
    const provider = providerFromSettings({ enabled, baseUrl, model });
    let live = true;
    setState({
      status: provider === null ? "none" : "configuring",
      error: null,
      readiness: null,
    });
    void (async () => {
      try {
        const readiness = await configureAssistantProvider(provider);
        if (!live) return;
        // Reported as Rust measured it, not as "it was accepted, so it works".
        setState({ status: statusOf(readiness), error: null, readiness });
      } catch (caught) {
        if (!live) return;
        const message = errorMessage(caught);
        logError("assistant", "provider configuration refused", { message });
        setState({ status: "error", error: message, readiness: null });
      }
    })();
    return () => {
      // A later edit supersedes this one: its answer must not overwrite the
      // newer request's status.
      live = false;
    };
  }, [enabled, baseUrl, model]);

  return state;
}
