/*
 * The trusted provider-configuration seam (ADR-0017).
 *
 * The assistant's local model lives behind a Rust-owned registry: the sidecar
 * names a provider ID in `provider.fetch` and Rust turns that ID into a URL. The
 * registry is populated from OneCAD settings and from nowhere else, which is what
 * this file is — the one place the webview proposes an endpoint, and the one
 * command that installs it.
 *
 * TWO RULES THIS FILE EXISTS TO ENFORCE:
 *
 *   1. The frontend PROPOSES; Rust DECIDES. `baseUrl` is sent verbatim and is
 *      validated by `validate_provider_base` on the Rust side — canonical literal
 *      loopback only. Nothing here may pre-approve, rewrite or widen a URL, and
 *      no request ever leaves the webview for that endpoint.
 *   2. NO CREDENTIAL. There is no `apiKey` field, here or in the Rust DTO: a
 *      loopback runtime does not need one, and `localStorage` is not where a
 *      secret belongs. The Rust DTO refuses one by name rather than dropping it.
 *
 * A refusal is a VALUE, not an exception to swallow: `configureAssistantProvider`
 * rejects with the reason Rust gave, and the caller shows it. A rejected endpoint
 * that nobody reported would resurface later as "unknown provider" on every
 * answer, which names the wrong fault entirely.
 */
import { invoke } from "@tauri-apps/api/core";

/** The `#[tauri::command]` that installs or clears the provider. */
export const ASSISTANT_CONFIGURE_PROVIDER_COMMAND = "assistant_configure_provider";

/**
 * The id of the one provider OneCAD settings describe.
 *
 * A single local runtime, so a constant rather than a per-entry setting — and it
 * is the id the sidecar names in `provider.fetch`, so it is not the user's to
 * choose.
 */
export const ASSISTANT_PROVIDER_ID = "local";

/** What the command is invoked with. camelCase, matching `ProviderSettingsDto`. */
export interface AssistantProviderSettings {
  readonly id: string;
  readonly baseUrl: string;
  readonly model: string;
}

/** The three settings fields the provider is built from. */
export interface AssistantProviderInputs {
  readonly enabled: boolean;
  readonly baseUrl: string;
  readonly model: string;
}

/**
 * The provider the current settings describe, or `null` when there is none to
 * install.
 *
 * `null` for a disabled assistant and for either field left empty: a half-filled
 * endpoint is not a provider, and installing one would only produce a refusal the
 * user did not ask for while they were still typing.
 */
export function providerFromSettings(
  inputs: AssistantProviderInputs,
): AssistantProviderSettings | null {
  const baseUrl = inputs.baseUrl.trim();
  const model = inputs.model.trim();
  if (!inputs.enabled || baseUrl === "" || model === "") return null;
  return { id: ASSISTANT_PROVIDER_ID, baseUrl, model };
}

/**
 * How far a configured provider actually got — four states, not one "ready".
 *
 * Mirrors Rust's `ProviderReadinessDto`. They are a LADDER: `configured` is only
 * "Rust accepted the endpoint and will resolve this id", which is not a working
 * model and must never be shown as one.
 */
export interface AssistantProviderReadiness {
  /** The Rust-minted generation carrying this configuration; 0 when cleared. */
  readonly generation: number;
  /** Rust validated the base and installed the gateway. */
  readonly configured: boolean;
  /** The running sidecar acknowledged the generation — until then, no turn runs. */
  readonly synchronized: boolean;
  /** The endpoint answered a probe. */
  readonly reachable: boolean;
  /** The endpoint's own catalogue lists the configured model. */
  readonly eligible: boolean;
  /** Why the ladder stopped where it did; `null` when every rung is true. */
  readonly detail: string | null;
}

/**
 * Installs `provider`, or clears the registry with `null`, and reports how far
 * it got.
 *
 * The gateway takes effect on the next `provider.fetch` — Rust reads the
 * registry per request — and the sidecar is told over the bridge, so an edit
 * reaches a sidecar that is already running without a restart.
 */
export async function configureAssistantProvider(
  provider: AssistantProviderSettings | null,
): Promise<AssistantProviderReadiness> {
  return await invoke<AssistantProviderReadiness>(ASSISTANT_CONFIGURE_PROVIDER_COMMAND, {
    provider,
  });
}
