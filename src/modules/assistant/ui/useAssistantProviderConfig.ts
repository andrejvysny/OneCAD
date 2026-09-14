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
import { configureAssistantProvider, providerFromSettings } from "../client/providerConfig";

/**
 * `none` — nothing configured, and the registry has been told so.
 * `configuring` — the command is in flight.
 * `ready` — the provider is installed and will serve the next request.
 * `error` — Rust refused it; `error` carries the reason.
 */
export type AssistantProviderStatus = "none" | "configuring" | "ready" | "error";

export interface AssistantProviderConfigState {
  readonly status: AssistantProviderStatus;
  /** The refusal reason, or `null`. Only ever set with `status: "error"`. */
  readonly error: string | null;
}

export function useAssistantProviderConfig(): AssistantProviderConfigState {
  const enabled = useSettingsStore((s) => s.assistantEnabled);
  const baseUrl = useSettingsStore((s) => s.assistantProviderBaseUrl);
  const model = useSettingsStore((s) => s.assistantProviderModel);
  const [state, setState] = useState<AssistantProviderConfigState>({
    status: "configuring",
    error: null,
  });

  useEffect(() => {
    const provider = providerFromSettings({ enabled, baseUrl, model });
    let live = true;
    setState({ status: provider === null ? "none" : "configuring", error: null });
    void (async () => {
      try {
        await configureAssistantProvider(provider);
        if (!live) return;
        setState({ status: provider === null ? "none" : "ready", error: null });
      } catch (caught) {
        if (!live) return;
        const message = errorMessage(caught);
        logError("assistant", "provider configuration refused", { message });
        setState({ status: "error", error: message });
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
